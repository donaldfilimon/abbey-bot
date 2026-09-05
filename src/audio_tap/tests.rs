use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn fixture(fixture_wire: Vec<u8>) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 4096];
        let _received = socket.read(&mut request).await.unwrap();
        socket.write_all(&fixture_wire).await.unwrap();
        String::from_utf8_lossy(&request[.._received]).into_owned()
    });
    (format!("http://{address}"), task)
}
fn headers() -> Vec<u8> {
    include_bytes!(
        "../../tools/abbey-audio-tap/Tests/AudioTapCoreTests/Fixtures/stream-header.http"
    )
    .to_vec()
}

#[test]
fn endpoints_are_numeric_loopback_only() {
    for endpoint in ["http://127.0.0.1:8182", "http://[::1]:8182"] {
        assert!(AudioTapClient::new(endpoint).is_ok(), "{endpoint}");
    }
    for endpoint in [
        "http://localhost:8182",
        "https://127.0.0.1:8182",
        "http://127.0.0.1:8182/path",
        "http://user:secret@127.0.0.1",
        "http://example.com",
        "http://192.168.1.1",
        "http://127.0.0.1/?x=1",
        "http://127.0.0.1/#x",
    ] {
        assert!(AudioTapClient::new(endpoint).is_err(), "{endpoint}");
    }
}
#[tokio::test]
async fn fake_pcm_ramp_reaches_raw_adapter_losslessly() {
    let pcm = (0..1920)
        .flat_map(|i| ((i - 960) as i16).to_le_bytes())
        .collect::<Vec<_>>();
    let mut response = headers();
    // HTTP transport splits are unrelated to stereo-frame boundaries.
    for part in [&pcm[..3], &pcm[3..1919], &pcm[1919..]] {
        response.extend_from_slice(format!("{:x}\r\n", part.len()).as_bytes());
        response.extend_from_slice(part);
        response.extend_from_slice(b"\r\n");
    }
    response.extend_from_slice(b"0\r\n\r\n");
    let (endpoint, server) = fixture(response).await;
    let client = AudioTapClient::new(&endpoint).unwrap();
    let mut stream = client.connect().await.unwrap();
    let frame = stream.next().await.unwrap();
    assert_eq!(frame, pcm);
    let buffer = PcmBuffer::new();
    buffer.push(&frame).unwrap();
    let mut raw = songbird::input::RawAdapter::new(buffer.reader(), 48000, 2);
    let mut bytes = vec![0; 16 + 1920 * 4];
    raw.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes[..8], b"SbirdRaw");
    assert_eq!(&bytes[8..12], &48000u32.to_le_bytes());
    assert_eq!(&bytes[12..16], &2u32.to_le_bytes());
    for (i, sample) in bytes[16..].as_chunks::<4>().0.iter().enumerate() {
        let f = f32::from_le_bytes(*sample);
        assert_eq!((f * 32768.0) as i16, i as i16 - 960);
    }
    assert_eq!(stream.next().await.unwrap_err(), TapError::Ended);
    server.await.unwrap();
}
#[tokio::test]
async fn protocol_failure_never_becomes_silence() {
    for response in [
        b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec(),
        b"HTTP/1.1 302 Found\r\nLocation: http://192.0.2.1/stream\r\nContent-Length: 0\r\n\r\n"
            .to_vec(),
    ] {
        let (endpoint, server) = fixture(response).await;
        assert!(
            AudioTapClient::new(&endpoint)
                .unwrap()
                .connect()
                .await
                .is_err()
        );
        server.await.unwrap();
    }
    let mut response = headers();
    response.extend_from_slice(b"4\r\nab");
    let (endpoint, server) = fixture(response).await;
    let mut stream = AudioTapClient::new(&endpoint)
        .unwrap()
        .connect()
        .await
        .unwrap();
    assert!(stream.next().await.is_err());
    server.await.unwrap();
}
#[tokio::test]
async fn health_validates_service_without_stream_request() {
    let body = include_bytes!(
        "../../tools/abbey-audio-tap/Tests/AudioTapCoreTests/Fixtures/health-idle.json"
    );
    let response = [
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes(),
        body.to_vec(),
    ]
    .concat();
    let (endpoint, server) = fixture(response).await;
    AudioTapClient::new(&endpoint)
        .unwrap()
        .health()
        .await
        .unwrap();
    let request = server.await.unwrap();
    assert!(request.starts_with("GET /health HTTP/1.1\r\n"));
}
#[test]
fn close_and_overflow_discard_buffers() {
    let buffer = PcmBuffer::new();
    for _ in 0..5 {
        buffer.push(&vec![0; FRAME_BYTES]).unwrap();
    }
    assert_eq!(
        buffer.push(&vec![0; FRAME_BYTES]),
        Err(TapError::Backpressure)
    );
    buffer.close();
    assert!(buffer.reader().read(&mut [0; 4]).is_err());
}
#[test]
fn expired_buffer_never_replays() {
    let buffer = PcmBuffer::new();
    buffer.push(&vec![0; FRAME_BYTES]).unwrap();
    buffer.0.queue.lock().unwrap().bytes.front_mut().unwrap().0 =
        Instant::now() - Duration::from_secs(1);
    assert_eq!(
        buffer.reader().read(&mut [0; 4]).unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
}

#[tokio::test]
async fn stalled_stream_fails_without_padding_or_retry() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 1024];
        let _received = socket.read(&mut request).await.unwrap();
        socket.write_all(&headers()).await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    });
    let mut stream = AudioTapClient::new(&endpoint)
        .unwrap()
        .connect()
        .await
        .unwrap();
    assert_eq!(stream.next().await.unwrap_err(), TapError::Stalled);
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn reconnect_has_no_audio_from_previous_connection() {
    for sample in [123i16, -321i16] {
        let pcm = sample.to_le_bytes().repeat(1920);
        let response = [
            headers(),
            format!("{:x}\r\n", pcm.len()).into_bytes(),
            pcm.clone(),
            b"\r\n0\r\n\r\n".to_vec(),
        ]
        .concat();
        let (endpoint, server) = fixture(response).await;
        let mut stream = AudioTapClient::new(&endpoint)
            .unwrap()
            .connect()
            .await
            .unwrap();
        assert_eq!(stream.next().await.unwrap(), pcm);
        server.await.unwrap();
    }
}

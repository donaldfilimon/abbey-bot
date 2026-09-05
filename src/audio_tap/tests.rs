use super::*;

fn frame(sample: i16) -> Vec<u8> {
    sample.to_le_bytes().repeat(FRAME_BYTES / 2)
}

#[test]
fn only_a_bare_numeric_loopback_http_endpoint_is_accepted() {
    // The tap carries whatever the Mac is playing, so the endpoint check is a
    // privacy boundary rather than a formatting nicety: every rejection here is
    // a way audio could otherwise leave the machine or reach a proxy.
    assert!(AudioTapClient::new("http://127.0.0.1:8181/").is_ok());
    assert!(AudioTapClient::new("http://[::1]:8181/").is_ok());

    for endpoint in [
        "https://127.0.0.1:8181/",     // TLS to loopback is still not this protocol
        "http://localhost:8181/",      // a name can be re-pointed; only literals are pinned
        "http://192.168.1.10:8181/",   // routable, so off-machine
        "http://user@127.0.0.1:8181/", // credentials belong nowhere in this URL
        "http://127.0.0.1:8181/?x=1",
        "http://127.0.0.1:8181/#x",
        "http://127.0.0.1:8181/stream", // paths are joined, never supplied
        "not a url",
    ] {
        // `AudioTapClient` holds a reqwest client and deliberately has no
        // `Debug`, so match rather than unwrap the error.
        assert!(
            matches!(AudioTapClient::new(endpoint), Err(TapError::Endpoint)),
            "{endpoint}"
        );
    }
}

#[test]
fn a_push_must_be_exactly_one_frame() {
    // Songbird is handed fixed-size frames; a short or long push would desync
    // every later frame rather than fail once.
    let buffer = PcmBuffer::new();
    assert_eq!(buffer.push(&[]).unwrap_err(), TapError::Protocol);
    assert_eq!(
        buffer.push(&vec![0; FRAME_BYTES - 2]).unwrap_err(),
        TapError::Protocol
    );
    assert_eq!(
        buffer.push(&vec![0; FRAME_BYTES + 2]).unwrap_err(),
        TapError::Protocol
    );
    assert!(buffer.push(&frame(0)).is_ok());
}

#[test]
fn the_s16_to_f32_conversion_is_exact_at_both_rails() {
    // The comment in `push` claims this conversion is exact for every i16.
    // Silence must stay silence and full scale must not wrap sign.
    let buffer = PcmBuffer::new();
    buffer.push(&frame(i16::MIN)).unwrap();
    let mut reader = buffer.reader();
    let mut out = [0u8; 4];
    reader.read_exact(&mut out).unwrap();
    assert!((f32::from_le_bytes(out) - -1.0).abs() < f32::EPSILON);

    let buffer = PcmBuffer::new();
    buffer.push(&frame(0)).unwrap();
    let mut reader = buffer.reader();
    let mut out = [0u8; 4];
    reader.read_exact(&mut out).unwrap();
    assert_eq!(f32::from_le_bytes(out), 0.0);
}

#[test]
fn the_queue_is_bounded_and_refuses_rather_than_growing() {
    // Five frames is the whole budget. An unbounded queue would trade a stall
    // for latency that never recovers.
    let buffer = PcmBuffer::new();
    for index in 0..5 {
        assert!(buffer.push(&frame(0)).is_ok(), "frame {index}");
    }
    assert_eq!(buffer.push(&frame(0)).unwrap_err(), TapError::Backpressure);
}

#[test]
fn closing_drops_queued_audio_and_refuses_later_pushes() {
    // `close` is the stop path. Audio queued before it must not play after it,
    // which is what makes "music stopped" truthful.
    let buffer = PcmBuffer::new();
    buffer.push(&frame(1000)).unwrap();
    buffer.close();
    assert_eq!(
        buffer.push(&frame(1000)).unwrap_err(),
        TapError::Backpressure
    );

    let mut reader = buffer.reader();
    let mut out = [0u8; 4];
    assert_eq!(
        reader.read(&mut out).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[test]
fn a_zero_length_read_never_blocks() {
    let buffer = PcmBuffer::new();
    let mut reader = buffer.reader();
    assert_eq!(reader.read(&mut []).unwrap(), 0);
}

#[test]
fn audio_older_than_the_freshness_window_is_dropped_rather_than_played_late() {
    // Playing a stale frame is worse than stopping: it would emit audio from a
    // moment the listener has already left behind.
    let buffer = PcmBuffer::new();
    buffer.push(&frame(1000)).unwrap();
    std::thread::sleep(FRESHNESS + Duration::from_millis(50));
    let mut reader = buffer.reader();
    let mut out = [0u8; 4];
    assert_eq!(
        reader.read(&mut out).unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
}

#[test]
fn a_partial_read_leaves_the_rest_of_the_frame_queued() {
    let buffer = PcmBuffer::new();
    buffer.push(&frame(0)).unwrap();
    let mut reader = buffer.reader();
    let mut first = [0u8; 4];
    assert_eq!(reader.read(&mut first).unwrap(), 4);
    let mut rest = [0u8; 8];
    assert_eq!(reader.read(&mut rest).unwrap(), 8);
}

#[test]
fn the_reader_is_an_unseekable_stream_of_unknown_length() {
    // Symphonia probes these before decoding; a wrong answer here makes it try
    // to seek a live capture.
    let buffer = PcmBuffer::new();
    let mut reader = buffer.reader();
    assert!(!reader.is_seekable());
    assert_eq!(reader.byte_len(), None);
    assert_eq!(
        reader.seek(SeekFrom::Start(0)).unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
}

#[test]
fn every_failure_says_plainly_that_music_stopped() {
    // These strings reach a Discord channel. Each names a distinct cause, and
    // none of them leaks an endpoint or a byte of audio.
    for error in [
        TapError::Endpoint,
        TapError::Unavailable,
        TapError::Protocol,
        TapError::Ended,
        TapError::Stalled,
        TapError::Backpressure,
    ] {
        let text = error.to_string();
        assert!(!text.is_empty(), "{error:?}");
        assert!(!text.contains("127.0.0.1"), "{error:?}");
    }
    assert_eq!(
        [
            TapError::Unavailable,
            TapError::Protocol,
            TapError::Ended,
            TapError::Stalled,
            TapError::Backpressure,
        ]
        .iter()
        .filter(|error| error.to_string().contains("music stopped"))
        .count(),
        5
    );
}

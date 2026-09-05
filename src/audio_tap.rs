//! Loopback-only sidecar protocol and bounded, expiring PCM transport.
//! No Discord types; no capture starts until `connect` is explicitly called.
use std::{
    collections::VecDeque,
    io::{self, Read, Seek, SeekFrom},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
use symphonia::core::io::MediaSource;

pub const FRAME_BYTES: usize = 960 * 2 * 2;
const MAX_BUFFER: usize = FRAME_BYTES * 5;
pub const FRESHNESS: Duration = Duration::from_millis(100);
const STALL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapError {
    Endpoint,
    Busy,
    Unavailable,
    Protocol,
    Ended,
    Stalled,
    Backpressure,
}
impl std::fmt::Display for TapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
        Self::Busy => "Audio tap is busy or still finishing capture teardown.",
        Self::Endpoint => "Audio tap requires a plain numeric loopback HTTP endpoint without credentials, query or fragment.",
        Self::Unavailable => "Audio tap unavailable; music stopped. Check the sidecar and capture permission.",
        Self::Protocol => "Audio tap returned an incompatible or malformed PCM stream; music stopped.",
        Self::Ended => "Audio tap ended; music stopped.", Self::Stalled => "Audio tap stalled; music stopped.",
        Self::Backpressure => "Audio tap fell behind; queued audio was discarded and music stopped.",
    })
    }
}
impl std::error::Error for TapError {}

#[derive(Clone)]
pub struct AudioTapClient {
    http: reqwest::Client,
    endpoint: reqwest::Url,
}
impl AudioTapClient {
    pub fn new(endpoint: &str) -> Result<Self, TapError> {
        let url = reqwest::Url::parse(endpoint).map_err(|_| TapError::Endpoint)?;
        if url.scheme() != "http"
            || !crate::llm::url_is_loopback(&url)
            || url
                .host_str()
                .unwrap_or("")
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_err()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err(TapError::Endpoint);
        }
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .build()
            .map_err(|_| TapError::Unavailable)?;
        Ok(Self {
            http,
            endpoint: url,
        })
    }
    pub async fn health(&self) -> Result<(), TapError> {
        let mut response = tokio::time::timeout(
            Duration::from_secs(2),
            self.http.get(self.endpoint.join("health").unwrap()).send(),
        )
        .await
        .map_err(|_| TapError::Unavailable)?
        .map_err(|_| TapError::Unavailable)?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(TapError::Unavailable);
        }
        let body = tokio::time::timeout(Duration::from_secs(2), async {
            let mut body = Vec::new();
            while let Some(bytes) = response.chunk().await.map_err(|_| TapError::Protocol)? {
                if body.len() + bytes.len() > 4096 {
                    return Err(TapError::Protocol);
                }
                body.extend_from_slice(&bytes);
            }
            Ok(body)
        })
        .await
        .map_err(|_| TapError::Unavailable)??;
        let value: serde_json::Value =
            serde_json::from_slice(&body).map_err(|_| TapError::Protocol)?;
        if value["service"] != "abbey-audio-tap"
            || value["protocol_version"] != 1
            || value["audio"]["format"] != "s16le"
            || value["audio"]["sample_rate"] != 48000
            || value["audio"]["channels"] != 2
            || value["stream_path"] != "/stream"
        {
            return Err(TapError::Protocol);
        }
        Ok(())
    }
    pub async fn connect(&self) -> Result<TapStream, TapError> {
        let response = tokio::time::timeout(
            Duration::from_secs(6),
            self.http.get(self.endpoint.join("stream").unwrap()).send(),
        )
        .await
        .map_err(|_| TapError::Unavailable)?
        .map_err(|_| TapError::Unavailable)?;
        if response.status() == reqwest::StatusCode::CONFLICT {
            return Err(TapError::Busy);
        }
        if response.status() != reqwest::StatusCode::OK {
            return Err(TapError::Unavailable);
        }
        for (name, expected) in [
            ("content-type", "application/octet-stream"),
            ("x-audio-format", "s16le"),
            ("x-audio-sample-rate", "48000"),
            ("x-audio-channels", "2"),
        ] {
            if response.headers().get(name).and_then(|v| v.to_str().ok()) != Some(expected) {
                return Err(TapError::Protocol);
            }
        }
        Ok(TapStream {
            response,
            pending: Vec::new(),
            pending_since: Instant::now(),
        })
    }
}

pub struct TapStream {
    response: reqwest::Response,
    pending: Vec<u8>,
    pending_since: Instant,
}
impl TapStream {
    pub async fn next(&mut self) -> Result<Vec<u8>, TapError> {
        loop {
            if self.pending.len() >= FRAME_BYTES {
                if self.pending_since.elapsed() > FRESHNESS {
                    return Err(TapError::Backpressure);
                }
                return Ok(self.pending.drain(..FRAME_BYTES).collect());
            }
            let chunk = tokio::time::timeout(STALL, self.response.chunk())
                .await
                .map_err(|_| TapError::Stalled)?
                .map_err(|_| TapError::Ended)?
                .ok_or(if self.pending.is_empty() {
                    TapError::Ended
                } else {
                    TapError::Protocol
                })?;
            if self.pending.len() + chunk.len() > MAX_BUFFER {
                return Err(TapError::Backpressure);
            }
            if self.pending.is_empty() {
                self.pending_since = Instant::now();
            }
            self.pending.extend_from_slice(&chunk);
        }
    }
}

struct Queue {
    bytes: VecDeque<(Instant, Vec<u8>)>,
    size: usize,
    closed: bool,
}
struct BufferInner {
    queue: Mutex<Queue>,
    wake: Condvar,
}
#[derive(Clone)]
pub struct PcmBuffer(Arc<BufferInner>);
impl PcmBuffer {
    pub fn new() -> Self {
        Self(Arc::new(BufferInner {
            queue: Mutex::new(Queue {
                bytes: VecDeque::new(),
                size: 0,
                closed: false,
            }),
            wake: Condvar::new(),
        }))
    }
    pub fn push(&self, pcm: &[u8]) -> Result<(), TapError> {
        if pcm.len() != FRAME_BYTES {
            return Err(TapError::Protocol);
        }
        let mut q = self
            .0
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if q.closed || q.size + pcm.len() * 2 > MAX_BUFFER * 2 {
            return Err(TapError::Backpressure);
        }
        // RawAdapter requires f32, not s16. This conversion is exact for every i16.
        let converted = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|v| (f32::from(i16::from_le_bytes(*v)) / 32768.0).to_le_bytes())
            .collect::<Vec<_>>();
        q.size += converted.len();
        q.bytes.push_back((Instant::now(), converted));
        self.0.wake.notify_all();
        Ok(())
    }
    pub fn close(&self) {
        let mut q = self
            .0
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        q.closed = true;
        q.size = 0;
        q.bytes.clear();
        self.0.wake.notify_all();
    }
    pub fn reader(&self) -> PcmReader {
        PcmReader {
            buffer: self.clone(),
        }
    }
}
pub struct PcmReader {
    buffer: PcmBuffer,
}
impl Read for PcmReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let inner = &self.buffer.0;
        let q = inner
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut q, timeout) = inner
            .wake
            .wait_timeout_while(q, STALL, |q| !q.closed && q.bytes.is_empty())
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if q.closed || timeout.timed_out() {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let Some((time, bytes)) = q.bytes.front_mut() else {
            return Err(io::ErrorKind::BrokenPipe.into());
        };
        if time.elapsed() > FRESHNESS {
            q.closed = true;
            q.bytes.clear();
            q.size = 0;
            return Err(io::ErrorKind::TimedOut.into());
        }
        let n = out.len().min(bytes.len());
        out[..n].copy_from_slice(&bytes[..n]);
        bytes.drain(..n);
        if bytes.is_empty() {
            q.bytes.pop_front();
        }
        q.size -= n;
        Ok(n)
    }
}
impl Seek for PcmReader {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        Err(io::ErrorKind::Unsupported.into())
    }
}
impl MediaSource for PcmReader {
    fn is_seekable(&self) -> bool {
        false
    }
    fn byte_len(&self) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests;

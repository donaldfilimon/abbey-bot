use super::*;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The real HTTP adapter dispatches a memory tool, then loses its continuation.
/// Neither provider retry nor a second memory proposal may hide that failure.
fn provider() -> (String, std::thread::JoinHandle<usize>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let worker = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut count = 0;
        while count < 2 && Instant::now() < deadline {
            let (mut stream, _) = match listener.accept() {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("synthetic listener: {error}"),
            };
            // Windows hands back an accepted socket in the listener's non-blocking
            // mode, so the blocking reads below would fail with WouldBlock and
            // `set_read_timeout` would not apply. Unix does not inherit it, where
            // this is a no-op.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0; 4096];
                let n = stream.read(&mut chunk).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&chunk[..n]);
                assert!(request.len() < 1024 * 1024);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let size: usize = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if request.len() >= end + 4 + size {
                        break;
                    }
                }
            }
            let (status, body) = if count == 0 {
                let delta = serde_json::json!({"choices":[{"delta":{"tool_calls":[{
                    "index":0,"id":"memory-call","type":"function","function":{
                        "name":"remember_fact","arguments":"{\"fact\":\"synthetic pipeline fact\"}"
                    }}]},"finish_reason":null}]});
                (
                    200,
                    format!(
                        "data: {delta}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}]}}\n\ndata: [DONE]\n\n"
                    ),
                )
            } else {
                (500, "synthetic continuation failure".into())
            };
            write!(stream, "HTTP/1.1 {status} Synthetic\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            count += 1;
        }
        count
    });
    (endpoint, worker)
}

fn state(endpoint: String) -> Arc<AppState> {
    let mut state = AppState::in_memory();
    let inner = Arc::get_mut(&mut state).unwrap();
    inner
        .providers
        .set_primary(Some(crate::llm::Backend::OpenAiCompatible {
            endpoint,
            model: "synthetic".into(),
        }));
    let config = serde_json::json!({
        "abi_cli": std::env::temp_dir().join("absent-abbey-pipeline-gate"),
        "endpoint": "http://127.0.0.1:50051",
        "token_file": std::env::temp_dir().join("absent-abbey-pipeline-token"),
        "policy_version": "policy_v1", "contract_revision": 2,
        "contract_digest": "01".repeat(32), "timeout_secs": 5
    });
    inner.episode_gate = Some(Arc::new(crate::episode_gate::EpisodeGate::new(
        crate::episode_gate::EpisodeGateConfig::from_json(&config.to_string()).unwrap(),
    )));
    state
}

struct Out {
    state: Arc<AppState>,
    fail_at: Option<usize>,
    attempts: Mutex<Vec<String>>,
}
impl Outbound for Out {
    async fn send(&self, _: &str, message: &OutboundMessage) -> Result<String, String> {
        let mut attempts = self.attempts.lock().unwrap();
        if attempts.is_empty() {
            assert_eq!(
                AppState::lock(&self.state.memory_queue).len(),
                0,
                "the model tool must refuse personal writes before continuation fails: {}",
                message.text
            );
        }
        attempts.push(message.text.clone());
        if self.fail_at.is_some_and(|index| attempts.len() >= index) {
            Err("synthetic delivery failure".into())
        } else {
            Ok(format!("sent-{}", attempts.len()))
        }
    }
    async fn typing(&self, _: &str) {}
    async fn react(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn fetch(&self, _: &str, _: usize) -> Result<Vec<u8>, String> {
        Err("unused".into())
    }
    async fn edit(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
}

#[tokio::test]
async fn continuation_failure_does_not_create_a_personal_memory_notice_or_reproposal() {
    for fail_at in [None, Some(2)] {
        let (endpoint, provider) = provider();
        let state = state(endpoint);
        let out = Out {
            state: state.clone(),
            fail_at,
            attempts: Mutex::new(Vec::new()),
        };
        let result = handle(
            &state,
            &out,
            message("remember this", Some("g"), "u"),
            true,
            None,
        )
        .await;
        assert!(matches!(result, Outcome::ReplyFailed(_)));
        assert_eq!(
            provider.join().unwrap(),
            2,
            "one tool turn and one failed continuation"
        );
        let attempts = out.attempts.lock().unwrap();
        assert_eq!(
            attempts.len(),
            1,
            "refused model writes create no queued admission or memory notice"
        );
        assert!(AppState::lock(&state.memory_queue).is_empty());
        assert!(
            state
                .memory_service()
                .facts("discord:g", "discord:u")
                .is_empty()
        );
        assert_eq!(
            state.episode_gate.as_ref().unwrap().counters().unavailable,
            0,
            "a model write refusal never invokes the admission gate"
        );
    }
}

#[tokio::test]
async fn failed_original_delivery_leaves_no_model_memory_to_submit() {
    let (endpoint, provider) = provider();
    let state = state(endpoint);
    let out = Out {
        state: state.clone(),
        fail_at: Some(1),
        attempts: Mutex::new(Vec::new()),
    };
    assert!(matches!(
        handle(
            &state,
            &out,
            message("remember this", Some("g"), "u"),
            true,
            None
        )
        .await,
        Outcome::ReplyFailed(_)
    ));
    assert_eq!(provider.join().unwrap(), 2);
    assert_eq!(out.attempts.lock().unwrap().len(), 1);
    assert!(AppState::lock(&state.memory_queue).is_empty());
    assert_eq!(
        crate::memory_gate::drain(&state).await,
        crate::memory_gate::Drained::default()
    );
    assert_eq!(
        state.episode_gate.as_ref().unwrap().counters().unavailable,
        0,
        "a later periodic drain cannot submit the cancelled request"
    );
    assert!(
        state
            .memory_service()
            .facts("discord:g", "discord:u")
            .is_empty()
    );
}

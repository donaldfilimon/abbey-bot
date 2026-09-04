use super::*;
use std::collections::HashSet;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

struct Fixture {
    runtime: Arc<VoiceRuntime>,
    input: mpsc::Sender<crate::offline_voice::VoiceFrame>,
    events: mpsc::UnboundedReceiver<&'static str>,
    release: Option<oneshot::Sender<()>>,
    cancel: watch::Sender<bool>,
    actor: tokio::task::JoinHandle<()>,
    server: tokio::task::JoinHandle<()>,
    sequence: u64,
    playback: SharedPlayback,
    // Retain the channels for the full session, just as the Discord shell does.
    _driver: watch::Sender<bool>,
    _lifecycle: mpsc::UnboundedSender<SessionEvent>,
}

impl Fixture {
    async fn new(second_transcript: &'static str) -> Self {
        Self::with_gate(second_transcript, "generation").await
    }

    async fn with_gate(second_transcript: &'static str, gate_stage: &'static str) -> Self {
        Self::with_transcripts("Abby, say hello.", second_transcript, gate_stage, None).await
    }

    async fn with_transcripts(
        first_transcript: &'static str,
        second_transcript: &'static str,
        gate_stage: &'static str,
        transcription_permits: Option<Arc<tokio::sync::Semaphore>>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (event_tx, events) = mpsc::unbounded_channel();
        let (release, wait) = oneshot::channel();
        let wait = Arc::new(Mutex::new(Some(wait)));
        let server = tokio::spawn(async move {
            let mut requests = JoinSet::new();
            let mut transcriptions = 0;
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut data = Vec::new();
                let (header_end, length) = loop {
                    let mut chunk = [0; 4096];
                    let count = socket.read(&mut chunk).await.unwrap();
                    assert_ne!(count, 0);
                    data.extend_from_slice(&chunk[..count]);
                    if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&data[..end]);
                        let length = header
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        break (end + 4, length);
                    }
                };
                while data.len() < header_end + length {
                    let mut chunk = [0; 4096];
                    let count = socket.read(&mut chunk).await.unwrap();
                    assert_ne!(count, 0);
                    data.extend_from_slice(&chunk[..count]);
                }
                let header = String::from_utf8_lossy(&data[..header_end]);
                let route = header.split_whitespace().nth(1).unwrap().to_owned();
                let transcript = if transcriptions == 0 {
                    first_transcript
                } else {
                    second_transcript
                };
                if route == "/v1/audio/transcriptions" {
                    transcriptions += 1;
                }
                let event_tx = event_tx.clone();
                let wait = Arc::clone(&wait);
                let transcription_permits = transcription_permits.clone();
                requests.spawn(async move {
                    let stage = match route.as_str() {
                        "/v1/audio/transcriptions" => "transcription",
                        "/v1/chat/completions" => "generation",
                        "/v1/audio/speech" => "synthesis",
                        other => panic!("unexpected test route: {other}"),
                    };
                    let gate = if stage == gate_stage { wait.lock().await.take() } else { None };
                    let _ = event_tx.send(stage);
                    if stage == "transcription" && let Some(permits) = transcription_permits {
                        permits.acquire().await.unwrap().forget();
                    }
                    if let Some(gate) = gate { let _ = gate.await; }
                    // Fixed synthetic responses only; this listener never
                    // handles production speech or credentials.
                    let (content_type, canned_reply) = match route.as_str() {
                        "/v1/audio/transcriptions" => {
                            ("application/json", serde_json::json!({"text": transcript}).to_string().into_bytes())
                        }
                        "/v1/chat/completions" => {
                            ("application/json", br#"{"choices":[{"message":{"role":"assistant","content":"Hello there."},"finish_reason":"stop"}]}"#.to_vec())
                        }
                        "/v1/audio/speech" => {
                            ("audio/wav", crate::offline_voice::encode_mono_pcm16_wav(&vec![1200; 24_000], 24_000).unwrap())
                        }
                        other => panic!("unexpected test route: {other}"),
                    };
                    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", canned_reply.len());
                    // A deliberately superseded turn may close its HTTP request.
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.write_all(&canned_reply).await;
                });
            }
        });
        let config = crate::offline_voice::OfflineVoiceConfig::from_values(
            Some(endpoint.clone()),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let runtime = Arc::new(VoiceRuntime::new(crate::voice::VoiceConfig {
            guild_id: 1,
            channel_id: 2,
            backend: crate::voice::VoiceBackendConfig::Local(config.clone()),
            wake_word_required: true,
            wake_words: crate::voice::VoiceConfig::default_wake_words(),
        }));
        let start = runtime.reserve_start();
        let epoch = runtime.begin(HashSet::from([1, 2])).await;
        assert!(runtime.activate(epoch, start, "test session active").await);
        let (input, input_rx) = mpsc::channel(64);
        let (lifecycle_tx, lifecycle) = mpsc::unbounded_channel();
        let (driver, driver_disconnect) = watch::channel(false);
        let (cancel, cancelled) = watch::channel(false);
        let playback = Arc::new(Mutex::new(None));
        let actor = tokio::spawn(run(LocalSession {
            runtime: Arc::clone(&runtime),
            state: AppState::in_memory(),
            call: Arc::new(Mutex::new(songbird::Call::standalone(
                std::num::NonZeroU64::new(1).unwrap(),
                std::num::NonZeroU64::new(99).unwrap(),
            ))),
            client: MlxAudioClient::new(config).unwrap(),
            epoch,
            input: input_rx,
            lifecycle,
            events: lifecycle_tx.clone(),
            driver_disconnect,
            cancel: cancelled,
            playback: Arc::clone(&playback),
            backend: crate::llm::Backend::from_values(None, Some(endpoint), Some("test".into()))
                .unwrap(),
        }));
        Self {
            runtime,
            input,
            events,
            release: Some(release),
            cancel,
            actor,
            server,
            sequence: 0,
            playback,
            _driver: driver,
            _lifecycle: lifecycle_tx,
        }
    }

    async fn utterance(&mut self, speaker: u64) {
        self.frames(speaker, 20, true).await;
        self.frames(speaker, 25, false).await;
    }

    async fn frames(&mut self, speaker: u64, count: usize, voiced: bool) {
        for _ in 0..count {
            self.sequence += 1;
            self.input
                .send(crate::offline_voice::VoiceFrame {
                    sequence: self.sequence,
                    speaker_id: Some(speaker),
                    samples: vec![if voiced { 2000 } else { 0 }; 480],
                    overlap: false,
                })
                .await
                .unwrap();
        }
    }

    async fn expect(&mut self, expected: &'static str) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while self.events.recv().await.unwrap() != expected {}
        })
        .await
        .unwrap_or_else(|_| panic!("voice never reached {expected}"));
    }

    async fn expect_playback(&self, playing: bool) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while self.playback.lock().await.is_some() != playing {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("playback did not reach the expected state");
    }

    async fn expect_media_closed(&self) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while self.runtime.media_enabled(self.runtime.current_epoch()) {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("voice media gate stayed open");
    }

    async fn stop(self) {
        let _ = self.cancel.send(true);
        self.actor.await.unwrap();
        self.server.abort();
    }
}

#[tokio::test]
async fn unrelated_speech_preserves_pending_reply() {
    let mut fixture = Fixture::new("This is an ordinary aside.").await;
    fixture.utterance(1).await;
    fixture.expect("generation").await;
    fixture.utterance(2).await;
    fixture.expect("transcription").await;
    fixture.release.take().unwrap().send(()).unwrap();
    fixture.expect("synthesis").await;
    assert_eq!(fixture.runtime.snapshot().await.barge_ins, 0);
    fixture.stop().await;
}

#[tokio::test]
async fn same_speaker_aside_does_not_replace_an_answer_before_playback() {
    let mut fixture = Fixture::new("An ordinary aside while waiting.").await;
    fixture.utterance(1).await;
    fixture.expect("generation").await;
    fixture.utterance(1).await;
    fixture.expect("transcription").await;
    fixture.release.take().unwrap().send(()).unwrap();
    fixture.expect("synthesis").await;
    assert_eq!(fixture.runtime.snapshot().await.barge_ins, 0);
    fixture.stop().await;
}

#[tokio::test]
async fn new_speech_does_not_discard_inflight_wake_recognition() {
    let mut fixture = Fixture::with_gate("An unrelated aside.", "transcription").await;
    fixture.utterance(1).await;
    fixture.expect("transcription").await;
    fixture.utterance(2).await;
    fixture.release.take().unwrap().send(()).unwrap();
    fixture.expect("generation").await;
    fixture.expect("synthesis").await;
    assert_eq!(fixture.runtime.snapshot().await.barge_ins, 0);
    fixture.stop().await;
}

#[tokio::test]
async fn prepared_reply_waits_for_a_short_noise_to_end() {
    let mut fixture = Fixture::new("unused").await;
    fixture.utterance(1).await;
    fixture.expect("generation").await;
    fixture.frames(2, 2, true).await;
    fixture.release.take().unwrap().send(()).unwrap();
    fixture.expect("synthesis").await;
    // No complete utterance will be emitted for this 40 ms noise.
    assert!(fixture.playback.lock().await.is_none());
    fixture.frames(2, 25, false).await;
    fixture.expect_playback(true).await;
    assert_eq!(fixture.runtime.snapshot().await.barge_ins, 0);
    fixture.stop().await;
}

#[tokio::test]
async fn audible_playback_still_stops_on_speech() {
    let mut fixture = Fixture::new("unused").await;
    fixture.utterance(1).await;
    fixture.expect("generation").await;
    fixture.release.take().unwrap().send(()).unwrap();
    fixture.expect("synthesis").await;
    fixture.expect_playback(true).await;
    fixture.frames(2, 2, true).await;
    fixture.expect_playback(false).await;
    // Taking the handle and recording the successful stop are separate
    // actor steps; synchronize on the counter before asserting it.
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.runtime.snapshot().await.barge_ins == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("playback stop was not counted");
    assert_eq!(fixture.runtime.snapshot().await.barge_ins, 1);
    assert_eq!(fixture.runtime.snapshot().await.completed_turns, 0);
    fixture.stop().await;
}

#[tokio::test]
async fn spoken_withdrawal_closes_media_while_a_reply_is_pending() {
    let mut fixture = Fixture::new("I do not consent.").await;
    fixture.utterance(1).await;
    fixture.expect("generation").await;
    fixture.utterance(2).await;
    fixture.expect("transcription").await;
    fixture.expect_media_closed().await;
    fixture.release.take().unwrap().send(()).unwrap();
    assert_eq!(
        fixture.runtime.snapshot().await.phase,
        VoicePhase::AwaitingConsent
    );
    assert!(fixture.playback.lock().await.is_none());
    fixture.stop().await;
}

#[tokio::test]
async fn a_new_addressed_question_supersedes_the_old_answer() {
    let mut fixture = Fixture::new("Abby, answer this instead.").await;
    fixture.utterance(1).await;
    fixture.expect("generation").await;
    fixture.utterance(2).await;
    fixture.expect("generation").await;
    fixture.expect("synthesis").await;
    fixture.expect_playback(true).await;
    assert_eq!(fixture.runtime.snapshot().await.barge_ins, 1);
    fixture.release.take().unwrap().send(()).unwrap();
    fixture.stop().await;
}

#[tokio::test]
async fn recognition_backlog_fails_closed_instead_of_dropping_withdrawals() {
    let mut fixture = Fixture::with_gate("An aside.", "transcription").await;
    fixture.utterance(1).await;
    fixture.expect("transcription").await;
    for _ in 0..MAX_PENDING_UTTERANCES {
        fixture.utterance(2).await;
    }
    fixture.expect_media_closed().await;
    assert_eq!(fixture.runtime.snapshot().await.phase, VoicePhase::Failed);
    assert!(fixture.playback.lock().await.is_none());
    fixture.stop().await;
}

#[tokio::test]
async fn queued_recognition_keeps_transcribing_status_until_drained() {
    let permits = Arc::new(tokio::sync::Semaphore::new(0));
    let mut fixture = Fixture::with_transcripts(
        "An ordinary aside.",
        "Another ordinary aside.",
        "none",
        Some(Arc::clone(&permits)),
    )
    .await;
    fixture.utterance(1).await;
    fixture.expect("transcription").await;
    fixture.utterance(2).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.input.capacity() != 64 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    permits.add_permits(1);
    fixture.expect("transcription").await;
    let snapshot = fixture.runtime.snapshot().await;
    assert_eq!(snapshot.phase, VoicePhase::Thinking);
    assert_eq!(snapshot.status, "transcribing locally");
    permits.add_permits(1);
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.runtime.snapshot().await.phase != VoicePhase::Listening {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    fixture.stop().await;
}

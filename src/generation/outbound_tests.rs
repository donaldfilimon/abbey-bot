use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct TextAdapter {
    id: ProviderId,
    calls: AtomicUsize,
    end: TextEnd,
    emit_delta: bool,
}
#[derive(Clone, Copy)]
enum TextEnd {
    Complete,
    BackendFailure,
    MixedTools,
}
impl crate::provider::TurnAdapter for TextAdapter {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }
    fn turn<'a>(
        &'a self,
        _: &'a str,
        _: &'a [llm::ChatTurn],
        _: &'a [crate::tools::ToolSpec],
        _: &'a str,
    ) -> crate::provider::TurnFuture<'a> {
        Box::pin(async move {
            if matches!(self.end, TextEnd::BackendFailure) {
                return Err(llm::LlmError::classified(
                    "synthetic provider timeout",
                    crate::provider::ProviderFailureKind::Timeout,
                ));
            }
            Ok(llm::ModelTurn {
                text: "A complete synthetic answer for a text request.".into(),
                calls: if matches!(self.end, TextEnd::MixedTools) {
                    vec![crate::tools::ToolCall {
                        id: "read-only-call".into(),
                        name: "list_facts".into(),
                        arguments: serde_json::json!({}),
                    }]
                } else {
                    vec![]
                },
            })
        })
    }
    fn execute<'a>(
        &'a self,
        request: crate::provider::AdapterRequest<'a>,
    ) -> crate::provider::TurnFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.emit_delta
                && let Some(sender) = request.deltas
            {
                sender.send("A synthetic reply that is long enough to trigger its first progressive send.".into()).unwrap();
                tokio::task::yield_now().await;
            }
            self.turn(
                request.system,
                request.turns,
                request.tools,
                request.call_id,
            )
            .await
        })
    }
}

#[derive(Default)]
struct HeldLostAck {
    recorded: crate::pipeline::testing::FakeOut,
    accepted: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl Outbound for HeldLostAck {
    async fn send(
        &self,
        channel: &str,
        message: &OutboundMessage,
    ) -> Result<String, OutboundFailure> {
        self.recorded.send(channel, message).await?;
        self.accepted.notify_one();
        self.release.notified().await;
        Err(OutboundFailure::new(
            OutboundFailureCategory::Transport,
            DeliveryCertainty::PossiblySent,
            None,
        ))
    }
    async fn edit(&self, channel: &str, id: &str, text: &str) -> Result<(), OutboundFailure> {
        self.recorded.edit(channel, id, text).await
    }
    async fn typing(&self, _: &str) {}
    async fn react(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
        Ok(())
    }
    async fn fetch(&self, _: &str, _: usize) -> Result<Vec<u8>, String> {
        unreachable!()
    }
}

#[tokio::test]
async fn delivery_faults_lost_ack_with_consent_withdrawal_preserves_certainty_and_never_replays() {
    for pipeline in [false, true] {
        let mut state = AppState::in_memory();
        let primary = Arc::new(TextAdapter {
            id: ProviderId::parse("primary").unwrap(),
            calls: AtomicUsize::new(0),
            end: TextEnd::Complete,
            emit_delta: true,
        });
        let secondary = Arc::new(TextAdapter {
            id: ProviderId::parse("secondary").unwrap(),
            calls: AtomicUsize::new(0),
            end: TextEnd::Complete,
            emit_delta: true,
        });
        let providers = &mut Arc::get_mut(&mut state).unwrap().providers;
        providers.register_test_streaming_adapter(primary.clone());
        providers.register_test_streaming_adapter(secondary.clone());
        let context = super::consent::tests::authorized_context(&state);
        #[cfg(unix)]
        let observed = pipeline.then(|| observe_events(&state));
        let ask = Ask {
            subject: Some(("discord:g", "discord:u")),
            session_mode: SessionMode::SourceOnly,
            scope: "discord:c",
            context: &context,
            user_input: "question",
            now: 1,
        };
        let event = crate::platform::SocialEvent {
            network: crate::platform::SocialNetwork::Discord,
            kind: crate::platform::EventKind::Message {
                text: "question".into(),
                attachments: vec![],
            },
            native_message_id: "m".into(),
            native_channel_id: "c".into(),
            native_guild_id: Some("g".into()),
            native_user_id: "u".into(),
            user_display_name: "member".into(),
            is_bot: false,
            timestamp: 0,
        };
        let out = HeldLostAck::default();
        let work = async {
            if pipeline {
                let outcome = crate::pipeline::handle(&state, &out, event, true, None).await;
                assert_eq!(
                    outcome,
                    crate::pipeline::Outcome::ReplyFailed(
                        "outbound transport failure (possibly sent)".into()
                    )
                );
            } else {
                let error = generate_read_only(
                    &state,
                    Persona::Abbey,
                    &ask,
                    Some(Delivery {
                        out: &out,
                        native_channel_id: "c",
                        reply_to: None,
                    }),
                )
                .await
                .unwrap_err();
                assert_eq!(error.kind(), llm::LlmErrorKind::Delivery);
                assert_eq!(
                    error.outbound_failure().unwrap().certainty(),
                    DeliveryCertainty::PossiblySent
                );
            }
        };
        tokio::pin!(work);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::select! {
                _ = &mut work => panic!("send must remain held"),
                _ = out.accepted.notified() => {},
            }
        })
        .await
        .unwrap();
        {
            let mut stores = AppState::lock(&state.stores);
            let subject = stores
                .personal_memory
                .get_mut(&crate::personal_memory::subject_key(
                    "discord:g",
                    "discord:u",
                ))
                .unwrap();
            subject.choice = crate::personal_memory::UseChoice::Off;
            subject.advance().unwrap();
            stores.personal_memory_exposure.epoch += 1;
        }
        out.release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(2), work)
            .await
            .unwrap();
        #[cfg(unix)]
        if let Some((home, writer)) = observed {
            let events = finish_events(home, writer).await;
            assert!(
                events
                    .iter()
                    .any(|event| event["code"] == "response_delivery"
                        && event["outcome"] == "failed")
            );
        }
        assert_eq!(out.recorded.sent.lock().unwrap().len(), 1);
        assert!(out.recorded.edited.lock().unwrap().is_empty());
        assert_eq!(primary.calls.load(Ordering::Relaxed), 1);
        assert_eq!(secondary.calls.load(Ordering::Relaxed), 0);
    }
}

struct HeldReplacement {
    recorded: crate::pipeline::testing::FakeOut,
    accepted: tokio::sync::Notify,
    release: tokio::sync::Notify,
    failure: OutboundFailure,
}
impl Outbound for HeldReplacement {
    async fn send(
        &self,
        channel: &str,
        message: &OutboundMessage,
    ) -> Result<String, OutboundFailure> {
        self.recorded.send(channel, message).await
    }
    async fn edit(&self, channel: &str, id: &str, text: &str) -> Result<(), OutboundFailure> {
        self.recorded.edit(channel, id, text).await?;
        self.accepted.notify_one();
        self.release.notified().await;
        Err(self.failure)
    }
    async fn typing(&self, _: &str) {}
    async fn react(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
        unreachable!()
    }
    async fn fetch(&self, _: &str, _: usize) -> Result<Vec<u8>, String> {
        unreachable!()
    }
}

fn request_event() -> crate::platform::SocialEvent {
    crate::platform::SocialEvent {
        network: crate::platform::SocialNetwork::Discord,
        kind: crate::platform::EventKind::Message {
            text: "question".into(),
            attachments: vec![],
        },
        native_message_id: "m".into(),
        native_channel_id: "c".into(),
        native_guild_id: Some("g".into()),
        native_user_id: "u".into(),
        user_display_name: "member".into(),
        is_bot: false,
        timestamp: 0,
    }
}

#[cfg(unix)]
fn observe_events(
    state: &AppState,
) -> (
    std::path::PathBuf,
    crate::service::telemetry::TelemetryWriter,
) {
    let home = crate::readiness::tests::temporary_home();
    let environment =
        crate::readiness::private::PrivateDirectory::open(&home, &[".config", "abbey-bot"])
            .unwrap();
    environment
        .publish("env", b"DISCORD_TOKEN=fixture-only\n")
        .unwrap();
    let managed =
        crate::managed_service::begin(&home).unwrap_or_else(|_| panic!("fixture preflight failed"));
    let identity = managed.publisher.identity().clone();
    let writer = crate::service::telemetry::TelemetryWriter::start(
        managed.log,
        managed.publisher,
        managed.fatal,
    );
    let status = Arc::new(crate::service::status::ManagedStatus::new(
        identity,
        writer.requests(),
        managed.privacy_report,
        false,
        false,
    ));
    state.attach_observability(writer.requests(), status);
    (home, writer)
}

#[cfg(unix)]
async fn finish_events(
    home: std::path::PathBuf,
    mut writer: crate::service::telemetry::TelemetryWriter,
) -> Vec<serde_json::Value> {
    writer.stop();
    writer.joined().await.unwrap();
    let text = std::fs::read_to_string(home.join("Library/Logs/abbey-bot/abbey-bot.events.jsonl"))
        .unwrap();
    std::fs::remove_dir_all(home).unwrap();
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn delivery_faults_failed_failure_replacement_preserves_certainty_without_replay() {
    for end in [TextEnd::BackendFailure, TextEnd::MixedTools] {
        for failure in [
            OutboundFailure::http(None, None),
            OutboundFailure::http(Some(403), None),
        ] {
            for withdraw in [false, true] {
                for pipeline in [false, true] {
                    let mut state = AppState::in_memory();
                    let primary = Arc::new(TextAdapter {
                        id: ProviderId::parse("primary").unwrap(),
                        calls: AtomicUsize::new(0),
                        end,
                        emit_delta: true,
                    });
                    let secondary = Arc::new(TextAdapter {
                        id: ProviderId::parse("secondary").unwrap(),
                        calls: AtomicUsize::new(0),
                        end: TextEnd::Complete,
                        emit_delta: true,
                    });
                    let providers = &mut Arc::get_mut(&mut state).unwrap().providers;
                    providers.register_test_streaming_adapter(primary.clone());
                    providers.register_test_streaming_adapter(secondary.clone());
                    let context = super::consent::tests::authorized_context(&state);
                    let ask = Ask {
                        subject: Some(("discord:g", "discord:u")),
                        session_mode: SessionMode::SourceOnly,
                        scope: "discord:c",
                        context: &context,
                        user_input: "question",
                        now: 1,
                    };
                    let out = HeldReplacement {
                        recorded: Default::default(),
                        accepted: Default::default(),
                        release: Default::default(),
                        failure,
                    };
                    let work = async {
                        if pipeline {
                            let outcome =
                                crate::pipeline::handle(&state, &out, request_event(), true, None)
                                    .await;
                            assert_eq!(
                                outcome,
                                crate::pipeline::Outcome::ReplyFailed(failure.to_string())
                            );
                        } else {
                            let mut host = crate::runtime::ToolScope {
                                memory_turn: None,
                                state: &state,
                                network: crate::platform::SocialNetwork::Discord,
                                scoped_guild: "discord:g".into(),
                                scoped_user: "discord:u".into(),
                                scoped_channel: "discord:c".into(),
                                now: 1,
                                persona: Persona::Abbey,
                            };
                            let error = generate_with_tools(
                                &state,
                                &mut host,
                                &ask,
                                Some(Delivery {
                                    out: &out,
                                    native_channel_id: "c",
                                    reply_to: None,
                                }),
                            )
                            .await
                            .unwrap_err();
                            assert_eq!(error.kind(), llm::LlmErrorKind::Delivery);
                            assert_eq!(error.outbound_failure(), Some(failure));
                        }
                    };
                    tokio::pin!(work);
                    tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    tokio::select! {
                        outcome = &mut work => panic!("replacement should still be held: {outcome:?}"),
                        _ = out.accepted.notified() => {},
                    }
                }).await.unwrap();
                    if withdraw {
                        let mut stores = AppState::lock(&state.stores);
                        let subject = stores
                            .personal_memory
                            .get_mut(&crate::personal_memory::subject_key(
                                "discord:g",
                                "discord:u",
                            ))
                            .unwrap();
                        subject.choice = crate::personal_memory::UseChoice::Off;
                        subject.advance().unwrap();
                        stores.personal_memory_exposure.epoch += 1;
                    }
                    out.release.notify_one();
                    tokio::time::timeout(std::time::Duration::from_secs(2), work)
                        .await
                        .unwrap();
                    assert_eq!(out.recorded.sent.lock().unwrap().len(), 1);
                    assert_eq!(out.recorded.edited.lock().unwrap().len(), 1);
                    assert_eq!(primary.calls.load(Ordering::Relaxed), 1);
                    assert_eq!(secondary.calls.load(Ordering::Relaxed), 0);
                    assert_eq!(AppState::lock(&state.engine).session_len("discord:c"), 0);
                    assert!(AppState::lock(&state.rewards).export_pending().is_empty());
                }
            }
        }
    }
}

struct EmptyReceipt {
    recorded: crate::pipeline::testing::FakeOut,
    receipt: &'static str,
    completed: AtomicUsize,
}
impl Outbound for EmptyReceipt {
    async fn send(
        &self,
        channel: &str,
        message: &OutboundMessage,
    ) -> Result<String, OutboundFailure> {
        self.recorded.send(channel, message).await?;
        Ok(self.receipt.into())
    }
    async fn completed_exchange(&self, _: &AppState, _: &crate::platform::SocialEvent, _: &str) {
        self.completed.fetch_add(1, Ordering::Relaxed);
    }
    async fn edit(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
        unreachable!()
    }
    async fn typing(&self, _: &str) {}
    async fn react(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
        unreachable!()
    }
    async fn fetch(&self, _: &str, _: usize) -> Result<Vec<u8>, String> {
        unreachable!()
    }
}

#[tokio::test]
async fn delivery_faults_empty_final_receipt_never_confirms_or_commits_an_exchange() {
    for streaming in [false, true] {
        for receipt in ["", " \t\n"] {
            let mut state = AppState::in_memory();
            let primary = Arc::new(TextAdapter {
                id: ProviderId::parse("primary").unwrap(),
                calls: AtomicUsize::new(0),
                end: TextEnd::Complete,
                emit_delta: false,
            });
            let providers = &mut Arc::get_mut(&mut state).unwrap().providers;
            if streaming {
                providers.register_test_streaming_adapter(primary.clone());
            } else {
                providers.register_test_adapter(primary.clone());
            }
            super::consent::tests::authorized_context(&state);
            let facts_before = AppState::lock(&state.stores)
                .memory
                .facts("discord:g", "discord:u")
                .to_vec();
            let out = EmptyReceipt {
                recorded: Default::default(),
                receipt,
                completed: AtomicUsize::new(0),
            };
            #[cfg(unix)]
            let (home, writer) = observe_events(&state);
            let outcome = crate::pipeline::handle(&state, &out, request_event(), true, None).await;
            #[cfg(unix)]
            let events = finish_events(home, writer).await;
            assert_eq!(
                outcome,
                crate::pipeline::Outcome::ReplyFailed(
                    "outbound internal failure (possibly sent)".into()
                )
            );
            assert_eq!(out.recorded.sent.lock().unwrap().len(), 1);
            assert_eq!(out.completed.load(Ordering::Relaxed), 0);
            assert_eq!(primary.calls.load(Ordering::Relaxed), 1);
            assert_eq!(AppState::lock(&state.engine).session_len("discord:c"), 0);
            assert!(AppState::lock(&state.rewards).export_pending().is_empty());
            assert!(AppState::lock(&state.memory_queue).is_empty());
            assert_eq!(
                AppState::lock(&state.stores)
                    .memory
                    .facts("discord:g", "discord:u"),
                facts_before
            );
            #[cfg(unix)]
            {
                assert!(
                    !events
                        .iter()
                        .any(|event| event["code"] == "discord_first_post"
                            && event["outcome"] == "succeeded")
                );
                assert!(
                    events
                        .iter()
                        .any(|event| event["code"] == "discord_post_failure"
                            && event["error_category"] == "internal")
                );
                assert!(
                    events
                        .iter()
                        .any(|event| event["code"] == "response_delivery"
                            && event["outcome"] == "failed")
                );
            }
        }
    }
}

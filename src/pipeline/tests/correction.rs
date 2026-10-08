use super::*;
use std::sync::Arc;

#[tokio::test]
async fn correction_repair_without_evidence_admits_uncertainty() {
    let (state, adapter) = fixture(Mode::Tool, false, "discord:g");
    let out = FakeOut::default();
    let result = handle(
        &state,
        &out,
        message("that's wrong", Some("g"), "u1"),
        true,
        Some("abbey-msg"),
    )
    .await;
    assert_eq!(result, Outcome::Replied);
    assert!(
        adapter.seen.lock().unwrap().is_empty(),
        "no provider call without current evidence"
    );
    let sent = out.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].1.text,
        "I can’t verify my earlier answer from the evidence I can currently access. Please share a current source or restate the question so I can check it."
    );
    println!("Insufficient evidence: {}", sent[0].1.text);
    assert!(AppState::lock(&state.memory_queue).is_empty());
}

use crate::personal_memory::*;
use std::sync::{Mutex, Weak};

#[derive(Clone, Copy)]
enum Mode {
    Text,
    Tool,
    Revoke,
    RemoveSource,
    DisableLearning,
    Fail,
    Held,
}
type Seen = (String, Vec<crate::llm::ChatTurn>, Vec<String>);
struct Adapter {
    id: crate::provider::ProviderId,
    mode: Mode,
    state: Mutex<Weak<AppState>>,
    seen: Arc<Mutex<Vec<Seen>>>,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl crate::provider::TurnAdapter for Adapter {
    fn provider_id(&self) -> &crate::provider::ProviderId {
        &self.id
    }
    fn turn<'a>(
        &'a self,
        system: &'a str,
        turns: &'a [crate::llm::ChatTurn],
        tools: &'a [crate::tools::ToolSpec],
        _: &'a str,
    ) -> crate::provider::TurnFuture<'a> {
        Box::pin(async move {
            self.seen.lock().unwrap().push((
                system.into(),
                turns.to_vec(),
                tools.iter().map(|t| t.name.to_string()).collect(),
            ));
            if matches!(self.mode, Mode::Fail) {
                return Err(crate::llm::LlmError::classified(
                    "synthetic transport failure",
                    crate::provider::ProviderFailureKind::TransportUnavailable,
                ));
            }
            if matches!(self.mode, Mode::Held) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            let state = self.state.lock().unwrap().upgrade().unwrap();
            match self.mode {
                Mode::Revoke => {
                    let mut stores = AppState::lock(&state.stores);
                    let subject = stores
                        .personal_memory
                        .get_mut(&subject_key("discord:g", "discord:u1"))
                        .unwrap();
                    subject.choice = UseChoice::Off;
                    subject.advance().unwrap();
                    stores.personal_memory_exposure.epoch += 1;
                }
                Mode::RemoveSource => {
                    AppState::lock(&state.rewards).settle_expired(runtime::now() + 151);
                }
                Mode::DisableLearning => {
                    let mut stores = AppState::lock(&state.stores);
                    AppState::lock(&state.guilds)
                        .update("discord:g", &mut *stores, |s| s.learning_enabled = false);
                }
                _ => {}
            }
            tokio::task::yield_now().await;
            Ok(crate::llm::ModelTurn {
                text: if matches!(self.mode, Mode::Tool) {
                    String::new()
                } else {
                    "The current port is 8181.".into()
                },
                calls: if matches!(self.mode, Mode::Tool) {
                    vec![crate::tools::ToolCall {
                        id: "attempt".into(),
                        name: "remember_fact".into(),
                        arguments: serde_json::json!({"fact":"invented port 9999"}),
                    }]
                } else {
                    vec![]
                },
            })
        })
    }
}
fn fixture(mode: Mode, evidence: bool, guild: &str) -> (Arc<AppState>, Arc<Adapter>) {
    fixture_for(mode, evidence, guild, "u1")
}
fn fixture_for(
    mode: Mode,
    evidence: bool,
    guild: &str,
    actor: &str,
) -> (Arc<AppState>, Arc<Adapter>) {
    let user = format!("discord:{actor}");
    let adapter = Arc::new(Adapter {
        id: crate::provider::ProviderId::parse("correction-fixture").unwrap(),
        mode,
        state: Mutex::new(Weak::new()),
        seen: Arc::new(Mutex::new(vec![])),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let mut state = AppState::in_memory();
    Arc::get_mut(&mut state)
        .unwrap()
        .providers
        .register_test_adapter(adapter.clone());
    let secondary = matches!(mode, Mode::Fail).then(|| {
        Arc::new(Adapter {
            id: crate::provider::ProviderId::parse("correction-secondary").unwrap(),
            mode: Mode::Text,
            state: Mutex::new(Weak::new()),
            seen: adapter.seen.clone(),
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        })
    });
    if let Some(secondary) = &secondary {
        Arc::get_mut(&mut state)
            .unwrap()
            .providers
            .register_test_adapter(secondary.clone());
        *secondary.state.lock().unwrap() = Arc::downgrade(&state);
    }
    *adapter.state.lock().unwrap() = Arc::downgrade(&state);
    {
        let mut stores = AppState::lock(&state.stores);
        AppState::lock(&state.guilds).update(guild, &mut *stores, |s| s.learning_enabled = true);
        if evidence {
            let fact = "The current port is 8181.";
            stores.memory.remember(guild, &user, fact, 1);
            let key = fact_key(guild, &user, fact);
            let mut subject = PersonalMemorySubject {
                schema: 1,
                policy_version: PERSONAL_MEMORY_POLICY_VERSION,
                choice: UseChoice::On,
                revision: 2,
                consent_epoch: 2,
                ..Default::default()
            };
            subject.proofs.insert(
                key.clone(),
                FactProof {
                    authority: FactAuthority::SelfAuthored,
                    member: MemberProof {
                        actor: user.clone(),
                        subject: user.clone(),
                        guild: guild.into(),
                        interaction_id: "fixture".into(),
                        platform: "discord".into(),
                        at: 1,
                        policy_version: PERSONAL_MEMORY_POLICY_VERSION,
                    },
                    fact_key: key,
                    previous_revision: 1,
                },
            );
            stores
                .personal_memory
                .insert(subject_key(guild, &user), subject);
        }
    }
    AppState::lock(&state.rewards).register_turn(ReplyTurn {
        state: vec![0.0; 18],
        action: BotAction::Reply.index(),
        sent_native_message_id: "abbey-msg".into(),
        scope: "discord:c1".into(),
        scoped_guild_id: guild.into(),
        ask: "what is the current port?".into(),
        asker: user.clone(),
        now: runtime::now(),
    });
    (state, adapter)
}

#[tokio::test]
async fn correction_exact_unique_and_dm_use_read_only_current_sources() {
    for (guild, pointer) in [
        (Some("g"), Some("abbey-msg")),
        (Some("g"), None),
        (None, Some("abbey-msg")),
    ] {
        let scope = if guild.is_some() {
            "discord:g"
        } else {
            "discord:dm:u1"
        };
        let (state, adapter) = fixture(Mode::Text, true, scope);
        AppState::lock(&state.engine).commit(
            "discord:c1",
            "LEGACY_USER_SECRET",
            "OLD_ASSISTANT_SECRET",
            1,
        );
        let out = FakeOut::default();
        assert_eq!(
            handle(
                &state,
                &out,
                message("that's wrong about the port", guild, "u1"),
                true,
                pointer
            )
            .await,
            Outcome::Replied
        );
        let seen = adapter.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].2.is_empty(), "corrections must never offer tools");
        assert!(
            seen[0]
                .0
                .contains(crate::brain::correction::RECOVERY_INSTRUCTIONS)
        );
        assert!(seen[0].0.contains("8181"));
        assert!(!format!("{seen:?}").contains("OLD_ASSISTANT_SECRET"));
        assert!(!format!("{seen:?}").contains("LEGACY_USER_SECRET"));
        let sent = out.sent.lock().unwrap();
        assert!(sent[0].1.text.contains("8181"));
        println!("Current-source repair: {}", sent[0].1.text);
    }
}

#[tokio::test]
async fn correction_model_mutating_tool_attempt_is_refused_without_memory_edges() {
    let (state, adapter) = fixture(Mode::Tool, true, "discord:g");
    let before = AppState::lock(&state.stores).personal_memory.clone();
    let out = FakeOut::default();
    assert!(matches!(
        handle(
            &state,
            &out,
            message("that's wrong about the port", Some("g"), "u1"),
            true,
            Some("abbey-msg")
        )
        .await,
        Outcome::ReplyFailed(_)
    ));
    let seen = adapter.seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "no mutating continuation or fallback replay");
    assert!(seen[0].2.is_empty());
    assert!(AppState::lock(&state.memory_queue).is_empty());
    assert_eq!(AppState::lock(&state.stores).personal_memory, before);
    assert_eq!(
        state.memory_service().facts("discord:g", "discord:u1"),
        vec!["The current port is 8181.".to_string()]
    );
}

#[tokio::test]
async fn correction_revocation_during_actual_generation_blocks_evidence_delivery() {
    for mode in [Mode::Revoke, Mode::RemoveSource, Mode::DisableLearning] {
        let (state, adapter) = fixture(mode, true, "discord:g");
        let mut supervisor = crate::service::ServiceSupervisor::new();
        supervisor.finish_startup();
        let _writer = state.attach_service(supervisor.operations());
        let out = FakeOut::default();
        assert!(matches!(
            handle(
                &state,
                &out,
                message("that's wrong about the port", Some("g"), "u1"),
                true,
                Some("abbey-msg")
            )
            .await,
            Outcome::ReplyFailed(_)
        ));
        {
            let seen = adapter.seen.lock().unwrap();
            assert_eq!(seen.len(), 1);
            assert!(seen[0].2.is_empty());
        }
        assert!(
            out.sent
                .lock()
                .unwrap()
                .iter()
                .all(|(_, m)| !m.text.contains("8181"))
        );
        {
            let sent = out.sent.lock().unwrap();
            assert_eq!(sent.len(), 1);
            assert!(
                sent[0].1.text.contains(
                    "The context for this answer changed while I was checking it. Please ask again."
                ),
                "{}",
                sent[0].1.text
            );
            assert!(!sent[0].1.text.contains("backend"));
            assert!(!sent[0].1.text.contains("call failed"));
            println!("Context withdrawal: {}", sent[0].1.text);
        }
        assert!(AppState::lock(&state.memory_queue).is_empty());
        let start = supervisor.begin_draining(
            crate::service::ShutdownReason::Signal,
            tokio::time::Instant::now(),
        );
        let report = supervisor
            .cancel_and_reap(start.budget.stage(tokio::time::Instant::now()))
            .await;
        assert_eq!(report.outcome, crate::service::ReapOutcome::Joined);
        assert!(supervisor.outstanding().is_empty());
        assert!(supervisor.try_freeze(true).is_ok());
    }
}

#[tokio::test]
async fn correction_quoted_bare_no_ambiguous_expired_cross_scope_and_disabled_are_not_repairs() {
    for case in [
        "quoted",
        "code",
        "indented-code",
        "pasted",
        "bare",
        "ambiguous",
        "expired",
        "cross-scope",
        "disabled",
        "unknown",
        "reaction",
        "thanks",
    ] {
        let (state, adapter) = fixture(Mode::Text, false, "discord:g");
        let mut event = message("that's wrong", Some("g"), "u1");
        let mut pointer = Some("abbey-msg");
        match case {
            "quoted" => {
                event.kind = EventKind::Message {
                    text: "\"that's wrong\"".into(),
                    attachments: vec![],
                }
            }
            "code" => {
                event.kind = EventKind::Message {
                    text: "`wrong`".into(),
                    attachments: vec![],
                }
            }
            "indented-code" => {
                event.kind = EventKind::Message {
                    text: "    wrong".into(),
                    attachments: vec![],
                };
            }
            "pasted" => {
                event.kind = EventKind::Message {
                    text: "Example: that's wrong".into(),
                    attachments: vec![],
                }
            }
            "bare" => {
                event.kind = EventKind::Message {
                    text: "No!".into(),
                    attachments: vec![],
                }
            }
            "thanks" => {
                event.kind = EventKind::Message {
                    text: "thanks".into(),
                    attachments: vec![],
                }
            }
            "ambiguous" => {
                pointer = None;
                AppState::lock(&state.rewards).register_turn(ReplyTurn {
                    state: vec![0.0; 18],
                    action: BotAction::Reply.index(),
                    sent_native_message_id: "another".into(),
                    scope: "discord:c1".into(),
                    scoped_guild_id: "discord:g".into(),
                    ask: "other".into(),
                    asker: "discord:u1".into(),
                    now: runtime::now(),
                });
            }
            "expired" => {
                AppState::lock(&state.rewards).settle_expired(runtime::now() + 151);
            }
            "cross-scope" => event.native_channel_id = "other".into(),
            "disabled" => {
                let mut stores = AppState::lock(&state.stores);
                AppState::lock(&state.guilds)
                    .update("discord:g", &mut *stores, |s| s.learning_enabled = false);
            }
            "unknown" => event.native_user_id.clear(),
            "reaction" => {
                pointer = Some("reacted-human");
                AppState::lock(&state.rewards).register_reply(
                    vec![0.0; 18],
                    BotAction::React.index(),
                    "reacted-human",
                    "discord:g",
                    "discord:c1",
                    runtime::now(),
                );
            }
            _ => unreachable!(),
        }
        let out = FakeOut::default();
        assert_eq!(
            handle(&state, &out, event, true, pointer).await,
            Outcome::Replied,
            "{case}"
        );
        let seen = adapter.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "{case}");
        assert!(
            !seen[0]
                .0
                .contains(crate::brain::correction::RECOVERY_INSTRUCTIONS),
            "{case}"
        );
        assert!(
            !seen[0].2.is_empty(),
            "ordinary forced request retains its existing tool boundary: {case}"
        );
    }
}

#[test]
fn correction_binding_refuses_changed_identity_scope_timestamp_signature_and_expiry() {
    let (state, _) = fixture(Mode::Text, false, "discord:g");
    let rewards = AppState::lock(&state.rewards);
    let now = runtime::now();
    let source = rewards
        .correction_source(
            "discord:c1",
            "discord:g",
            "discord:u1",
            Some("abbey-msg"),
            "that's wrong",
            now,
            true,
        )
        .unwrap();
    assert!(rewards.correction_current(&source, now));
    assert!(!rewards.correction_current(&source, now + 151));
    for field in 0..6 {
        let mut changed = source.clone();
        match field {
            0 => changed.native_id = "other".into(),
            1 => changed.scope = "discord:other".into(),
            2 => changed.guild = "discord:other".into(),
            3 => changed.asker = "discord:other".into(),
            4 => changed.created_at += 1,
            _ => {
                changed.signature =
                    crate::brain::ask_signature::AskSignature::from_text("unrelated revised source")
            }
        }
        assert!(!rewards.correction_current(&changed, now));
    }
    assert!(
        rewards
            .correction_source(
                "discord:c1",
                "discord:g",
                "discord:other",
                None,
                "that's wrong",
                now,
                true
            )
            .is_none()
    );
    assert!(
        rewards
            .correction_source(
                "discord:c1",
                "discord:g",
                "discord:u1",
                Some("abbey-msg"),
                "that's wrong",
                now,
                false
            )
            .is_none()
    );
}

#[tokio::test]
async fn correction_pre_effect_provider_fallback_stays_read_only() {
    let (state, adapter) = fixture(Mode::Fail, true, "discord:g");
    let out = FakeOut::default();
    assert_eq!(
        handle(
            &state,
            &out,
            message("that's wrong about the port", Some("g"), "u1"),
            true,
            Some("abbey-msg")
        )
        .await,
        Outcome::Replied
    );
    let seen = adapter.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(seen.iter().all(|(system, _, tools)| tools.is_empty()
        && system.contains(crate::brain::correction::RECOVERY_INSTRUCTIONS)));
    assert_eq!(out.sent.lock().unwrap().len(), 1);
}

struct WithdrawBeforeSnapshot<'a> {
    state: &'a AppState,
}
impl Outbound for WithdrawBeforeSnapshot<'_> {
    async fn typing(&self, _: &str) {
        AppState::lock(&self.state.rewards).settle_expired(runtime::now() + 151);
        tokio::task::yield_now().await;
    }
    async fn send(&self, _: &str, _: &OutboundMessage) -> Result<String, OutboundFailure> {
        panic!("source withdrawn before snapshot")
    }
    async fn edit(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
        panic!("no generation")
    }
    async fn react(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
        panic!("no reaction")
    }
    async fn fetch(&self, _: &str, _: usize) -> Result<Vec<u8>, String> {
        panic!("no attachment")
    }
}
#[tokio::test]
async fn correction_source_withdrawn_before_snapshot_never_calls_provider() {
    let (state, adapter) = fixture(Mode::Tool, true, "discord:g");
    let out = WithdrawBeforeSnapshot { state: &state };
    assert!(matches!(
        handle(
            &state,
            &out,
            message("that's wrong about the port", Some("g"), "u1"),
            true,
            Some("abbey-msg")
        )
        .await,
        Outcome::ReplyFailed(_)
    ));
    assert!(adapter.seen.lock().unwrap().is_empty());
    assert!(AppState::lock(&state.memory_queue).is_empty());
}

#[tokio::test]
async fn erasure_of_correcting_observer_fences_actual_held_generation() {
    for erased in [7, 9] {
        let (state, adapter) = fixture_for(Mode::Held, true, "discord:g", "7");
        {
            let mut rewards = AppState::lock(&state.rewards);
            let mut rows = rewards.export_pending();
            rows[0].1.asker = "discord:8".into();
            let recovery = rewards.export_recovery();
            *rewards = crate::brain::reward::RewardCollector::new();
            rewards.restore_recovered(rows, recovery).unwrap();
        }
        let mut supervisor = crate::service::ServiceSupervisor::new();
        supervisor.finish_startup();
        let mut writer = state.attach_service(supervisor.operations());
        let out = Arc::new(FakeOut::default());
        let s = state.clone();
        let o = out.clone();
        let request = tokio::spawn(async move {
            handle(
                &s,
                &*o,
                message("that's wrong about the port", Some("g"), "7"),
                true,
                Some("abbey-msg"),
            )
            .await
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            adapter.entered.notified(),
        )
        .await
        .unwrap();
        state
            .erase_learning_now("discord:g", Some(erased), runtime::now())
            .unwrap();
        adapter.release.notify_one();
        let result = request.await.unwrap();
        let text = out
            .sent
            .lock()
            .unwrap()
            .iter()
            .map(|(_, m)| m.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        if erased == 7 {
            assert!(matches!(result, Outcome::ReplyFailed(_)));
            assert!(!text.contains("8181"));
        } else {
            assert_eq!(result, Outcome::Replied);
            assert!(text.contains("8181"));
        }
        let start = supervisor.begin_draining(
            crate::service::ShutdownReason::Signal,
            tokio::time::Instant::now(),
        );
        let joined = supervisor
            .cancel_and_reap(start.budget.stage(tokio::time::Instant::now()))
            .await;
        assert_eq!(joined.outcome, crate::service::ReapOutcome::Joined);
        assert!(supervisor.outstanding().is_empty());
        writer.stop();
        writer.joined().await.unwrap();
    }
}

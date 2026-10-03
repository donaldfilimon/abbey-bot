use super::*;
use crate::{memory::PersonaContext, personal_memory::*};

pub(in crate::generation) fn authorized_context(state: &AppState) -> PersonaContext {
    let fact = "PRIVATE_MEMBER_FACT";
    let mut stores = AppState::lock(&state.stores);
    stores.memory.remember("discord:g", "discord:u", fact, 1);
    let key = fact_key("discord:g", "discord:u", fact);
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
                actor: "discord:u".into(),
                subject: "discord:u".into(),
                guild: "discord:g".into(),
                interaction_id: "synthetic-action".into(),
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
        .insert(subject_key("discord:g", "discord:u"), subject);
    drop(stores);
    let context =
        state
            .memory_service()
            .context_for("discord:g", "discord:u", "discord:c", "", 0, 0.5);
    assert_eq!(context.user_facts, vec![fact.to_owned()]);
    assert!(context.personal_memory_permits.is_context_sealed());
    assert!(context.personal_memory_permits.authorizes_personal_memory());
    assert!(GenerationGuard::capture(state, &request(&context)).is_ok());
    context
}
fn request(context: &PersonaContext) -> Ask<'_> {
    Ask {
        subject: Some(("discord:g", "discord:u")),
        session_mode: SessionMode::SourceOnly,
        scope: "discord:c",
        context,
        user_input: "My current explicit request",
        now: 1,
    }
}
fn revoke(state: &AppState) {
    let mut stores = AppState::lock(&state.stores);
    let subject = stores
        .personal_memory
        .get_mut(&subject_key("discord:g", "discord:u"))
        .unwrap();
    subject.choice = UseChoice::Off;
    subject.advance().unwrap();
    stores.personal_memory_exposure.epoch += 1;
}

#[test]
fn personal_memory_adapter_seal_binds_exact_facts_summary_and_subject() {
    let state = AppState::in_memory();
    let context = authorized_context(&state);
    assert!(GenerationGuard::capture(&state, &request(&context)).is_ok());
    for altered in [
        PersonaContext {
            user_facts: vec!["FORGED_FACT".into()],
            ..context.clone()
        },
        PersonaContext {
            channel_summary: "MIXED_HISTORY".into(),
            ..context.clone()
        },
        PersonaContext {
            user_facts: Vec::new(),
            ..context.clone()
        },
    ] {
        assert!(GenerationGuard::capture(&state, &request(&altered)).is_err());
    }
    for subject in [
        ("discord:g", "discord:other"),
        ("discord:other", "discord:u"),
    ] {
        let mut ask = request(&context);
        ask.subject = Some(subject);
        assert!(GenerationGuard::capture(&state, &ask).is_err());
    }
}

#[test]
fn personal_memory_adapter_deserialization_cannot_restore_service_seal() {
    let state = AppState::in_memory();
    let context = authorized_context(&state);
    let mut json = serde_json::to_value(&context).unwrap();
    // Even a supplied digest never becomes a process-local seal.
    json["personal_memory_permits"]["context_digest"] = serde_json::json!("forged");
    let decoded: PersonaContext = serde_json::from_value(json).unwrap();
    assert!(!decoded.personal_memory_permits.is_context_sealed());
    assert!(GenerationGuard::capture(&state, &request(&decoded)).is_err());
}

#[test]
fn personal_memory_adapter_default_empty_and_unsealed_permits_cannot_admit_material() {
    let state = AppState::in_memory();
    let valid = authorized_context(&state);
    for permits in [
        MemoryUsePermitSet::default(),
        MemoryUsePermitSet::empty(state.personal_memory_exposure_epoch()),
        state.personal_memory_permits("discord:g", "discord:u"),
    ] {
        let context = PersonaContext {
            personal_memory_permits: permits,
            ..valid.clone()
        };
        assert!(GenerationGuard::capture(&state, &request(&context)).is_err());
    }
}

#[test]
fn personal_memory_adapter_cutoff_invalidates_prepared_context_and_guard() {
    let state = AppState::in_memory();
    let context = authorized_context(&state);
    let guard = GenerationGuard::capture(&state, &request(&context)).unwrap();
    revoke(&state);
    assert!(guard.check(&state).is_err());
    assert!(GenerationGuard::capture(&state, &request(&context)).is_err());
    let fresh =
        state
            .memory_service()
            .context_for("discord:g", "discord:u", "discord:c", "", 0, 0.5);
    assert!(fresh.user_facts.is_empty());
    assert!(GenerationGuard::capture(&state, &request(&fresh)).is_ok());
}

#[test]
fn personal_memory_adapter_unattributed_source_only_is_fresh_and_history_free() {
    let state = AppState::in_memory();
    AppState::lock(&state.engine).commit("discord:c", "OLD_MIXED_HISTORY", "OLD_BOT_OUTPUT", 1);
    let context = PersonaContext::empty();
    let mut ask = request(&context);
    ask.subject = None;
    let guard = GenerationGuard::capture(&state, &ask).unwrap();
    let prepared = ask.prepare(&state, crate::persona::Persona::Abbey);
    assert_eq!(prepared.turns, [crate::llm::ChatTurn::user(ask.user_input)]);
    assert!(guard.check(&state).is_ok());
    ask.session_mode = SessionMode::Shared;
    assert!(GenerationGuard::capture(&state, &ask).is_err());
    let mixed = PersonaContext {
        channel_summary: "retained serialized history".into(),
        ..PersonaContext::empty()
    };
    ask.context = &mixed;
    ask.session_mode = SessionMode::SourceOnly;
    assert!(GenerationGuard::capture(&state, &ask).is_err());
}

#[test]
fn personal_memory_adapter_revision_policy_and_activation_barriers_reject_permits() {
    for mutation in 0..3 {
        let state = AppState::in_memory();
        let context = authorized_context(&state);
        let guard = GenerationGuard::capture(&state, &request(&context)).unwrap();
        let mut stores = AppState::lock(&state.stores);
        let subject = stores
            .personal_memory
            .get_mut(&subject_key("discord:g", "discord:u"))
            .unwrap();
        match mutation {
            0 => subject.revision += 1,
            1 => subject.policy_version += 1,
            _ => subject.activation_pending = true,
        }
        drop(stores);
        assert!(guard.check(&state).is_err());
    }
}

#[tokio::test]
async fn personal_memory_adapter_revocation_drops_inflight_work_before_provider_continuation() {
    let state = AppState::in_memory();
    let context = authorized_context(&state);
    let guard = GenerationGuard::capture(&state, &request(&context)).unwrap();
    let work = async {
        revoke(&state);
        std::future::pending::<Result<(), llm::LlmError>>().await
    };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        guard.while_current(&state, work),
    )
    .await
    .unwrap();
    assert_eq!(
        result.unwrap_err().provider_failure(),
        crate::provider::ProviderFailureKind::Cancelled
    );
}

#[tokio::test]
async fn personal_memory_adapter_stream_withdrawal_blocks_first_delivery() {
    let state = AppState::in_memory();
    let context = authorized_context(&state);
    let guard = GenerationGuard::capture(&state, &request(&context)).unwrap();
    let out = crate::pipeline::testing::FakeOut::default();
    let (tx, rx) = crate::generation::stream_owner::channel();
    let work = async {
        revoke(&state);
        tx.send("PRIVATE_MEMBER_FACT ".repeat(10)).unwrap();
        tokio::task::yield_now().await;
        Ok(llm::ModelTurn {
            text: "PRIVATE_MEMBER_FACT".into(),
            calls: Vec::new(),
        })
    };
    let result = crate::generation::stream_received(
        work,
        rx,
        &crate::generation::Delivery {
            out: &out,
            native_channel_id: "channel",
            reply_to: None,
        },
        "synthetic",
        crate::persona::Persona::Abbey,
        &crate::grounding::Grounding::new(),
        &crate::provider::ConversationEffects::default(),
        Some((&state, &guard)),
    )
    .await;
    assert!(result.is_err());
    assert!(AppState::lock(&out.sent).is_empty());
    assert!(AppState::lock(&out.edited).is_empty());
}

struct Adapter {
    id: crate::provider::ProviderId,
    state: std::sync::Mutex<Option<std::sync::Weak<AppState>>>,
    calls: std::sync::atomic::AtomicUsize,
    revoke_at: usize,
    tools: bool,
    seen: std::sync::Mutex<Vec<Vec<crate::llm::ChatTurn>>>,
}
impl crate::provider::TurnAdapter for Adapter {
    fn provider_id(&self) -> &crate::provider::ProviderId {
        &self.id
    }
    fn turn<'a>(
        &'a self,
        _: &'a str,
        turns: &'a [crate::llm::ChatTurn],
        _: &'a [crate::tools::ToolSpec],
        _: &'a str,
    ) -> crate::provider::TurnFuture<'a> {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            AppState::lock(&self.seen).push(turns.to_vec());
            if call == self.revoke_at {
                let state = AppState::lock(&self.state)
                    .as_ref()
                    .unwrap()
                    .upgrade()
                    .unwrap();
                revoke(&state);
            }
            Ok(crate::llm::ModelTurn {
                text: if self.tools && call == 1 {
                    String::new()
                } else {
                    "PRIVATE_MEMBER_FACT".into()
                },
                calls: if self.tools && call == 1 {
                    vec![crate::tools::ToolCall {
                        id: "recall-call".into(),
                        name: "list_facts".into(),
                        arguments: serde_json::json!({}),
                    }]
                } else {
                    Vec::new()
                },
            })
        })
    }
}
fn provider_state(
    revoke_at: usize,
    tools: bool,
) -> (
    std::sync::Arc<AppState>,
    std::sync::Arc<Adapter>,
    PersonaContext,
) {
    let mut state = AppState::in_memory();
    let adapter = std::sync::Arc::new(Adapter {
        id: crate::provider::ProviderId::parse("primary").unwrap(),
        state: std::sync::Mutex::new(None),
        calls: std::sync::atomic::AtomicUsize::new(0),
        revoke_at,
        tools,
        seen: std::sync::Mutex::new(Vec::new()),
    });
    std::sync::Arc::get_mut(&mut state)
        .unwrap()
        .providers
        .register_test_adapter(adapter.clone());
    *AppState::lock(&adapter.state) = Some(std::sync::Arc::downgrade(&state));
    let context = authorized_context(&state);
    (state, adapter, context)
}

#[tokio::test]
async fn personal_memory_adapter_withdrawal_after_provider_dispatch_suppresses_answer() {
    let (state, adapter, context) = provider_state(1, false);
    let out = crate::pipeline::testing::FakeOut::default();
    let result = crate::generation::generate_read_only(
        &state,
        crate::persona::Persona::Abbey,
        &request(&context),
        Some(crate::generation::Delivery {
            out: &out,
            native_channel_id: "discord:c",
            reply_to: None,
        }),
    )
    .await;
    assert_eq!(
        result.unwrap_err().provider_failure(),
        crate::provider::ProviderFailureKind::Cancelled
    );
    assert_eq!(adapter.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(AppState::lock(&out.sent).is_empty());
}

#[tokio::test]
async fn personal_memory_adapter_tool_results_share_permits_and_withdrawal_stops_continuation() {
    let (state, adapter, context) = provider_state(2, true);
    let mut host = crate::runtime::ToolScope {
        memory_turn: None,
        state: &state,
        network: crate::platform::SocialNetwork::Discord,
        scoped_guild: "discord:g".into(),
        scoped_user: "discord:u".into(),
        scoped_channel: "discord:c".into(),
        now: 1,
        persona: crate::persona::Persona::Abbey,
    };
    let result = crate::generation::generate_with_tools_without_delivery(
        &state,
        &mut host,
        &request(&context),
    )
    .await;
    assert_eq!(
        result.unwrap_err().provider_failure(),
        crate::provider::ProviderFailureKind::Cancelled
    );
    let seen = AppState::lock(&adapter.seen);
    assert_eq!(seen.len(), 2);
    assert!(
        seen[1]
            .iter()
            .any(|turn| turn.text.contains("PRIVATE_MEMBER_FACT"))
    );
    assert!(
        !state
            .personal_memory_status("discord:g", "discord:u")
            .choice
            .eq(&UseChoice::On)
    );
}

#[tokio::test]
async fn personal_memory_adapter_wrong_tool_subject_never_dispatches_provider() {
    let (state, adapter, context) = provider_state(0, true);
    let mut host = crate::runtime::ToolScope {
        memory_turn: None,
        state: &state,
        network: crate::platform::SocialNetwork::Discord,
        scoped_guild: "discord:g".into(),
        scoped_user: "discord:other".into(),
        scoped_channel: "discord:c".into(),
        now: 1,
        persona: crate::persona::Persona::Abbey,
    };
    assert!(
        crate::generation::generate_with_tools_without_delivery(
            &state,
            &mut host,
            &request(&context)
        )
        .await
        .is_err()
    );
    assert_eq!(adapter.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn personal_memory_adapter_current_request_with_use_off_still_answers() {
    let (state, adapter, _) = provider_state(0, false);
    revoke(&state);
    AppState::lock(&state.engine).commit("discord:c", "LEGACY_MIXED_SECRET", "PRIOR_BOT_SECRET", 1);
    let context =
        state
            .memory_service()
            .context_for("discord:g", "discord:u", "discord:c", "", 0, 0.5);
    let ask = request(&context);
    let output = crate::generation::generate_read_only::<crate::generation::NoDelivery>(
        &state,
        crate::persona::Persona::Abbey,
        &ask,
        None,
    )
    .await
    .unwrap();
    assert!(!output.0.is_empty());
    let seen = AppState::lock(&adapter.seen);
    assert_eq!(seen[0], [crate::llm::ChatTurn::user(ask.user_input)]);
    assert!(context.user_facts.is_empty());
}

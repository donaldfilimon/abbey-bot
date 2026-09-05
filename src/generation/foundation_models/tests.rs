use std::collections::VecDeque;
use std::sync::Mutex;

use super::*;
use crate::memory::PersonaContext;
use crate::provider::{FmConfig, FmMode, ProviderCapabilities};

fn foundation_models() -> FoundationModels {
    FoundationModels::new(
        FmConfig {
            mode: FmMode::System,
            endpoint: None,
            cli: "/usr/bin/fm".into(),
            fallback: true,
            timeout_secs: 30,
        },
        Some(&llm::Backend::OpenAiCompatible {
            endpoint: "http://127.0.0.1:8282".into(),
            model: "primary".into(),
        }),
        true,
    )
}

struct FakeFm {
    router: ProviderRouter,
    responses: Mutex<VecDeque<Result<llm::ModelTurn, llm::LlmError>>>,
    seen_turns: Mutex<Vec<Vec<llm::ChatTurn>>>,
    offered_counts: Mutex<Vec<usize>>,
}

impl FakeFm {
    fn with_responses(responses: Vec<Result<llm::ModelTurn, llm::LlmError>>) -> Self {
        Self {
            router: ProviderRouter::new(
                Some(&llm::Backend::OpenAiCompatible {
                    endpoint: "http://127.0.0.1:8282".into(),
                    model: "primary".into(),
                }),
                true,
                None,
                Some(ProviderCapabilities::text_with_tools()),
                true,
            ),
            responses: Mutex::new(responses.into()),
            seen_turns: Mutex::new(Vec::new()),
            offered_counts: Mutex::new(Vec::new()),
        }
    }
}

impl FmTurnSource for FakeFm {
    fn router(&self) -> &ProviderRouter {
        &self.router
    }

    async fn cli_turn<'a>(
        &'a self,
        _system_prompt: &'a str,
        turns: &'a [llm::ChatTurn],
        tools: &'a [crate::tools::ToolSpec],
        _call_id: &'a str,
    ) -> Result<llm::ModelTurn, llm::LlmError> {
        self.seen_turns.lock().unwrap().push(turns.to_vec());
        self.offered_counts.lock().unwrap().push(tools.len());
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(llm::LlmError::backend("no fake response".into())))
    }
}

#[test]
fn fallback_order_preserves_local_before_foundation_models() {
    assert_eq!(
        fallback_routes(false, false, false, true, true, true),
        [
            FallbackRoute::Local,
            FallbackRoute::FoundationModelsServer,
            FallbackRoute::FoundationModelsCli,
        ]
    );
}

#[test]
fn streamed_or_potentially_mutating_failures_start_no_second_provider() {
    assert!(fallback_routes(true, true, false, true, true, true).is_empty());
    assert!(fallback_routes(false, false, true, true, true, true).is_empty());
    assert_eq!(
        fallback_routes(true, false, false, false, true, true),
        [
            FallbackRoute::FoundationModelsServer,
            FallbackRoute::FoundationModelsCli,
        ]
    );
}

#[test]
fn primary_tool_rejection_is_observed_on_the_next_round() {
    let fm = foundation_models();
    assert!(primary_tools_are_available(Some(&fm), true));
    fm.router.disable_tools(ProviderRoute::Primary);
    assert!(!primary_tools_are_available(Some(&fm), true));
    assert!(
        fm.router
            .effective_capabilities(ProviderRoute::FoundationModelsCli)
            .unwrap()
            .tools,
        "primary rejection must leave the FM CLI tool route available"
    );
}

#[test]
fn dispatched_primary_tool_failure_plans_no_restart_or_duplicate_mutation() {
    let state = AppState::in_memory();
    let mut host = crate::runtime::ToolScope {
        state: &state,
        network: crate::platform::SocialNetwork::Discord,
        scoped_guild: "discord:1".into(),
        scoped_user: "discord:2".into(),
        scoped_channel: "discord:3".into(),
        now: 10,
        persona: Persona::Abbey,
    };
    let call = crate::tools::ToolCall {
        id: "primary-0".into(),
        name: "remember_fact".into(),
        arguments: serde_json::json!({"fact": "favorite color is blue"}),
    };
    let offered = crate::tools::production_tools();
    ToolAccess::Enabled(&mut host)
        .dispatch(&offered, &[call])
        .unwrap();

    let routes = fallback_routes(false, false, true, true, false, true);
    assert!(routes.is_empty(), "no second provider may be started");
    assert_eq!(
        AppState::lock(&state.stores)
            .memory
            .facts("discord:1", "discord:2"),
        ["favorite color is blue"],
        "the primary side effect must exist exactly once"
    );
}

#[tokio::test]
async fn refusal_is_final_text_and_has_no_side_effect() {
    let state = AppState::in_memory();
    let fm = FakeFm::with_responses(vec![Ok(llm::ModelTurn {
        text: "I can’t store that.".into(),
        calls: Vec::new(),
    })]);
    let mut host = crate::runtime::ToolScope {
        state: &state,
        network: crate::platform::SocialNetwork::Discord,
        scoped_guild: "discord:1".into(),
        scoped_user: "discord:2".into(),
        scoped_channel: "discord:3".into(),
        now: 10,
        persona: Persona::Abbey,
    };
    let context = PersonaContext::empty();
    let answer = generate_with_fm_cli_and_access(
        &state,
        &fm,
        ToolAccess::Enabled(&mut host),
        &Ask {
            session_mode: crate::generation::SessionMode::Shared,
            scope: "discord:3",
            context: &context,
            user_input: "remember a secret",
            now: 10,
        },
    )
    .await
    .unwrap();
    assert_eq!(answer.0, "I can’t store that.");
    assert!(
        AppState::lock(&state.stores)
            .memory
            .facts("discord:1", "discord:2")
            .is_empty()
    );
}

#[tokio::test]
async fn completed_fm_reply_uses_the_canonical_grounding_hedge() {
    let state = AppState::in_memory();
    let fm = FakeFm::with_responses(vec![Ok(llm::ModelTurn {
        text: "It shipped in 2019.".into(),
        calls: Vec::new(),
    })]);
    let context = PersonaContext::empty();
    let answer = generate_with_fm_cli_and_access(
        &state,
        &fm,
        ToolAccess::Disabled(Persona::Abbey),
        &Ask {
            session_mode: crate::generation::SessionMode::Shared,
            scope: "discord:3",
            context: &context,
            user_input: "when did it ship?",
            now: 10,
        },
    )
    .await
    .unwrap();

    assert!(
        answer.0.contains("treat these as unsupported: `2019`"),
        "{}",
        answer.0
    );
}

#[tokio::test]
async fn max_round_boundary_dispatches_no_extra_tool() {
    let calls = (0..=crate::tools::MAX_TOOL_ROUNDS)
        .map(|round| {
            Ok(llm::ModelTurn {
                text: String::new(),
                calls: vec![crate::tools::ToolCall {
                    id: format!("fm-{round}"),
                    name: "remember_fact".into(),
                    arguments: serde_json::json!({"fact": format!("fact {round}")}),
                }],
            })
        })
        .collect();
    let state = AppState::in_memory();
    let fm = FakeFm::with_responses(calls);
    let mut host = crate::runtime::ToolScope {
        state: &state,
        network: crate::platform::SocialNetwork::Discord,
        scoped_guild: "discord:1".into(),
        scoped_user: "discord:2".into(),
        scoped_channel: "discord:3".into(),
        now: 10,
        persona: Persona::Abbey,
    };
    let context = PersonaContext::empty();
    let error = generate_with_fm_cli_and_access(
        &state,
        &fm,
        ToolAccess::Enabled(&mut host),
        &Ask {
            session_mode: crate::generation::SessionMode::Shared,
            scope: "discord:3",
            context: &context,
            user_input: "keep storing",
            now: 10,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.detail(), "backend returned unrequested tool calls");
    assert_eq!(
        AppState::lock(&state.stores)
            .memory
            .facts("discord:1", "discord:2"),
        ["fact 0", "fact 1", "fact 2"]
    );
    assert_eq!(
        AppState::lock(&state.stores)
            .memory
            .user("discord:1", "discord:2")
            .expect("tool-created subject memory")
            .updated_at,
        10,
        "every tool round must reuse the ToolScope timestamp"
    );
    assert_eq!(*fm.offered_counts.lock().unwrap(), [7, 7, 7, 0]);
}

#[tokio::test]
async fn continuation_failure_never_replays_the_completed_tool() {
    let state = AppState::in_memory();
    let call = crate::tools::ToolCall {
        id: "fm-0".into(),
        name: "remember_fact".into(),
        arguments: serde_json::json!({"fact": "favorite color is blue"}),
    };
    let fm = FakeFm::with_responses(vec![
        Ok(llm::ModelTurn {
            text: String::new(),
            calls: vec![call],
        }),
        Err(llm::LlmError::backend("tool continuation rejected".into())),
        Err(llm::LlmError::backend("plain continuation failed".into())),
    ]);
    let mut host = crate::runtime::ToolScope {
        state: &state,
        network: crate::platform::SocialNetwork::Discord,
        scoped_guild: "discord:1".into(),
        scoped_user: "discord:2".into(),
        scoped_channel: "discord:3".into(),
        now: 10,
        persona: Persona::Abbey,
    };
    let context = PersonaContext::empty();
    let error = generate_with_fm_cli_and_access(
        &state,
        &fm,
        ToolAccess::Enabled(&mut host),
        &Ask {
            session_mode: crate::generation::SessionMode::Shared,
            scope: "discord:3",
            context: &context,
            user_input: "remember my favorite color",
            now: 10,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.detail(), "plain continuation failed");
    assert_eq!(
        AppState::lock(&state.stores)
            .memory
            .facts("discord:1", "discord:2"),
        ["favorite color is blue"]
    );
    let seen = fm.seen_turns.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(
        seen.iter()
            .flatten()
            .filter(|turn| !turn.tool_calls.is_empty())
            .count(),
        2,
        "the same prior call appears in continuation transcripts but was dispatched once"
    );
    assert_eq!(*fm.offered_counts.lock().unwrap(), [7, 7, 0]);
    assert!(
        fm.router
            .effective_capabilities(ProviderRoute::Primary)
            .unwrap()
            .tools,
        "FM rejection must not disable primary tools"
    );
    assert!(
        !fm.router
            .effective_capabilities(ProviderRoute::FoundationModelsCli)
            .unwrap()
            .tools,
        "only the FM CLI tool route is disabled"
    );
}

#[tokio::test]
async fn ephemeral_tool_rounds_preserve_the_shared_persona_even_on_error() {
    for fail in [false, true] {
        let state = AppState::in_memory();
        let context = PersonaContext::empty();
        AppState::lock(&state.engine).prepare("discord:3", Persona::Abbey, &context, "public", 1);
        AppState::lock(&state.engine).commit("discord:3", "public", "answer", 1);
        let last = if fail {
            Err(llm::LlmError::backend("synthetic failure".into()))
        } else {
            Ok(llm::ModelTurn {
                text: "A private answer.".into(),
                calls: Vec::new(),
            })
        };
        let fm = FakeFm::with_responses(vec![
            Ok(llm::ModelTurn {
                text: String::new(),
                calls: vec![crate::tools::ToolCall {
                    id: "switch".into(),
                    name: "switch_persona".into(),
                    arguments: serde_json::json!({"persona": "aviva"}),
                }],
            }),
            last,
        ]);
        let mut host = crate::runtime::ToolScope {
            state: &state,
            network: crate::platform::SocialNetwork::Discord,
            scoped_guild: "discord:1".into(),
            scoped_user: "discord:2".into(),
            scoped_channel: "discord:3".into(),
            now: 10,
            persona: Persona::Abbey,
        };
        let result = generate_with_fm_cli_and_access(
            &state,
            &fm,
            ToolAccess::Enabled(&mut host),
            &Ask {
                session_mode: crate::generation::SessionMode::Ephemeral,
                scope: "discord:3",
                context: &context,
                user_input: "private",
                now: 10,
            },
        )
        .await;
        assert_eq!(result.is_err(), fail);
        assert_eq!(
            host.persona,
            Persona::Aviva,
            "tool changes the private reply persona"
        );
        let engine = AppState::lock(&state.engine);
        assert_eq!(engine.session_persona("discord:3"), Some(Persona::Abbey));
        assert_eq!(engine.session_len("discord:3"), 2);
        let seen = fm.seen_turns.lock().unwrap();
        assert_eq!(seen[0][0], llm::ChatTurn::user("public"));
        assert_eq!(seen[0][2], llm::ChatTurn::user("private"));
    }
}

#[tokio::test]
async fn text_only_fm_remains_eligible_without_offering_rejected_tools() {
    let mut state = AppState::in_memory();
    let configured = crate::provider::FoundationModels::new_qualified(
        FmConfig {
            mode: FmMode::System,
            endpoint: None,
            cli: "/not-executed/fm".into(),
            fallback: true,
            timeout_secs: 1,
        },
        None,
        true,
        crate::provider::VerifiedFmCapabilities {
            server: None,
            cli: ProviderCapabilities::text_with_tools(),
        },
    );
    configured
        .router
        .disable_tools(ProviderRoute::FoundationModelsCli);
    std::sync::Arc::get_mut(&mut state)
        .unwrap()
        .foundation_models = Some(configured);
    assert!(crate::generation::fm_cli_text_available(
        state.foundation_models.as_ref()
    ));
    let fm = FakeFm::with_responses(vec![Ok(llm::ModelTurn {
        text: "A plain private answer.".into(),
        calls: Vec::new(),
    })]);
    fm.router.disable_tools(ProviderRoute::FoundationModelsCli);
    let mut host = crate::runtime::ToolScope {
        state: &state,
        network: crate::platform::SocialNetwork::Discord,
        scoped_guild: "discord:1".into(),
        scoped_user: "discord:2".into(),
        scoped_channel: "discord:3".into(),
        now: 10,
        persona: Persona::Abbey,
    };
    let context = PersonaContext::empty();
    let reply = generate_with_fm_cli_and_access(
        &state,
        &fm,
        ToolAccess::Enabled(&mut host),
        &Ask {
            session_mode: crate::generation::SessionMode::Ephemeral,
            scope: "discord:3",
            context: &context,
            user_input: "hello",
            now: 10,
        },
    )
    .await
    .unwrap();
    assert_eq!(reply.0, "A plain private answer.");
    assert_eq!(*fm.offered_counts.lock().unwrap(), [0]);
    assert_eq!(AppState::lock(&state.engine).session_len("discord:3"), 0);
    assert!(
        AppState::lock(&state.stores)
            .memory
            .facts("discord:1", "discord:2")
            .is_empty()
    );
}

//! Foundation Models fallback generation and its provider-local tool loop.

use std::future::Future;
use std::sync::atomic::Ordering;

use crate::ask;
use crate::llm;
use crate::persona::Persona;
use crate::provider::{FoundationModels, ProviderRoute, ProviderRouter};
use crate::runtime::AppState;

use super::{Ask, ToolAccess};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FallbackRoute {
    Local,
    FoundationModelsServer,
    FoundationModelsCli,
}

pub(super) fn fallback_routes(
    delivery_present: bool,
    streamed_provider_attempted: bool,
    tool_dispatch_possible: bool,
    local_available: bool,
    fm_server_available: bool,
    fm_cli_available: bool,
) -> Vec<FallbackRoute> {
    if tool_dispatch_possible || (delivery_present && streamed_provider_attempted) {
        return Vec::new();
    }
    [
        (local_available, FallbackRoute::Local),
        (fm_server_available, FallbackRoute::FoundationModelsServer),
        (fm_cli_available, FallbackRoute::FoundationModelsCli),
    ]
    .into_iter()
    .filter_map(|(available, route)| available.then_some(route))
    .collect()
}

pub(super) fn primary_tools_are_available(
    fm: Option<&FoundationModels>,
    operator_tools_enabled: bool,
) -> bool {
    operator_tools_enabled
        && fm
            .and_then(|provider| {
                provider
                    .router
                    .effective_capabilities(ProviderRoute::Primary)
            })
            .is_none_or(|capabilities| capabilities.tools)
}

pub(super) trait FmTurnSource {
    fn router(&self) -> &ProviderRouter;

    fn cli_turn<'a>(
        &'a self,
        system_prompt: &'a str,
        turns: &'a [llm::ChatTurn],
        tools: &'a [crate::tools::ToolSpec],
        call_id: &'a str,
    ) -> impl Future<Output = Result<llm::ModelTurn, llm::LlmError>> + Send + 'a;
}

impl FmTurnSource for FoundationModels {
    fn router(&self) -> &ProviderRouter {
        &self.router
    }

    async fn cli_turn<'a>(
        &'a self,
        system_prompt: &'a str,
        turns: &'a [llm::ChatTurn],
        tools: &'a [crate::tools::ToolSpec],
        call_id: &'a str,
    ) -> Result<llm::ModelTurn, llm::LlmError> {
        FoundationModels::cli_turn(self, system_prompt, turns, tools, call_id).await
    }
}

pub(super) async fn generate_with_fm_cli_and_access<F: FmTurnSource>(
    state: &AppState,
    fm: &F,
    mut access: ToolAccess<'_, '_>,
    ask: &Ask<'_>,
) -> Result<(String, Option<String>, Persona), llm::LlmError> {
    let Ask {
        scope,
        context,
        user_input,
        now,
    } = *ask;
    let fm_tools = fm
        .router()
        .effective_capabilities(ProviderRoute::FoundationModelsCli)
        .is_some_and(|capabilities| capabilities.tools);
    let vocabulary =
        (access.is_enabled() && fm_tools && state.tools_enabled.load(Ordering::Relaxed))
            .then(crate::tools::abbey_tools);
    let mut extra_turns: Vec<llm::ChatTurn> = Vec::new();
    for round_index in 0..=crate::tools::MAX_TOOL_ROUNDS {
        let persona = access.persona();
        let prepared =
            AppState::lock(&state.engine).prepare(scope, persona, context, user_input, now);
        let mut turns = prepared.turns.clone();
        turns.extend(extra_turns.iter().cloned());
        let offer = vocabulary.is_some()
            && round_index < crate::tools::MAX_TOOL_ROUNDS
            && fm
                .router()
                .effective_capabilities(ProviderRoute::FoundationModelsCli)
                .is_some_and(|capabilities| capabilities.tools);
        let tools: &[crate::tools::ToolSpec] = if offer {
            vocabulary.as_deref().unwrap_or_default()
        } else {
            &[]
        };
        let turn = match fm
            .cli_turn(
                &prepared.system_prompt,
                &turns,
                tools,
                &format!("fm-{round_index}"),
            )
            .await
        {
            Ok(turn) => turn,
            Err(error) if offer => {
                fm.router()
                    .disable_tools(ProviderRoute::FoundationModelsCli);
                tracing::warn!(error = %error, "Foundation Models CLI rejected a schema-guided tool round; disabling only its tool route");
                continue;
            }
            Err(error) => return Err(error),
        };
        if turn.calls.is_empty() {
            if turn.text.trim().is_empty() {
                return Err(llm::LlmError::backend(
                    "the FM CLI response carried no answer text".into(),
                ));
            }
            return Ok((ask::tidy_reply(persona, &turn.text), None, persona));
        }
        let results = access.dispatch(offer, &turn.calls)?;
        for call in &turn.calls {
            tracing::info!(tool = %call.name, scope, provider = "foundation_models_cli", "tool call completed");
        }
        extra_turns.push(llm::ChatTurn::assistant_calls(turn.text, turn.calls));
        extra_turns.extend(results.iter().map(llm::ChatTurn::tool_result));
    }
    Err(llm::LlmError::backend(format!(
        "the FM CLI kept calling tools for {} rounds without answering",
        crate::tools::MAX_TOOL_ROUNDS
    )))
}

#[cfg(test)]
mod tests {
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
            persona: Persona::Abbey,
        };
        let call = crate::tools::ToolCall {
            id: "primary-0".into(),
            name: "remember_fact".into(),
            arguments: serde_json::json!({"fact": "favorite color is blue"}),
        };
        ToolAccess::Enabled(&mut host)
            .dispatch(true, &[call])
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
            persona: Persona::Abbey,
        };
        let context = PersonaContext::empty();
        let answer = generate_with_fm_cli_and_access(
            &state,
            &fm,
            ToolAccess::Enabled(&mut host),
            &Ask {
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
            persona: Persona::Abbey,
        };
        let context = PersonaContext::empty();
        let error = generate_with_fm_cli_and_access(
            &state,
            &fm,
            ToolAccess::Enabled(&mut host),
            &Ask {
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
        assert_eq!(*fm.offered_counts.lock().unwrap(), [5, 5, 5, 0]);
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
            persona: Persona::Abbey,
        };
        let context = PersonaContext::empty();
        let error = generate_with_fm_cli_and_access(
            &state,
            &fm,
            ToolAccess::Enabled(&mut host),
            &Ask {
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
        assert_eq!(*fm.offered_counts.lock().unwrap(), [5, 5, 0]);
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
}

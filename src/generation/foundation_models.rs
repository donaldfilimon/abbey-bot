//! Foundation Models fallback generation and its provider-local tool loop.

use std::future::Future;
use std::sync::atomic::Ordering;

use crate::llm;
use crate::persona::Persona;
use crate::provider::{FoundationModels, ProviderRoute, ProviderRouter};
use crate::runtime::AppState;

use super::{Ask, ToolAccess, finalize_reply, grounding_for_round};

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
    let scope = ask.scope;
    let fm_tools = fm
        .router()
        .effective_capabilities(ProviderRoute::FoundationModelsCli)
        .is_some_and(|capabilities| capabilities.tools);
    let vocabulary = crate::tools::production_tools_when_enabled(
        access.is_enabled() && fm_tools && state.tools_enabled.load(Ordering::Relaxed),
    );
    let mut extra_turns: Vec<llm::ChatTurn> = Vec::new();
    let mut grounding_results: Vec<crate::tools::ToolResult> = Vec::new();
    for round_index in 0..=crate::tools::MAX_TOOL_ROUNDS {
        let persona = access.persona();
        let prepared = ask.prepare(state, persona);
        let mut turns = prepared.turns.clone();
        turns.extend(extra_turns.iter().cloned());
        let grounding = grounding_for_round(&prepared, &grounding_results);
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
            return Ok((
                finalize_reply(persona, &turn.text, &grounding),
                None,
                persona,
            ));
        }
        let results = access.dispatch(tools, &turn.calls)?;
        for call in &turn.calls {
            tracing::info!(tool = %call.name, scope, provider = "foundation_models_cli", "tool call completed");
        }
        extra_turns.push(llm::ChatTurn::assistant_calls(turn.text, turn.calls));
        extra_turns.extend(results.iter().map(llm::ChatTurn::tool_result));
        grounding_results.extend(results);
    }
    Err(llm::LlmError::backend(format!(
        "the FM CLI kept calling tools for {} rounds without answering",
        crate::tools::MAX_TOOL_ROUNDS
    )))
}

#[cfg(test)]
mod tests;

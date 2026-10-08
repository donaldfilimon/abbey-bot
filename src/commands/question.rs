//! Shared command generation with explicit private continuity audience.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn answer_question_with_memory(
    state: &AppState,
    guild: Option<u64>,
    channel: u64,
    user: u64,
    question: &str,
    forced: Option<Persona>,
    commit: Commit,
    memory: &crate::memory_gate::MemoryTurn,
) -> (
    String,
    Option<generation::consent::GenerationGuard>,
    Option<generation::DeliveryTiming>,
) {
    answer_with_visibility(
        state, guild, channel, user, question, forced, commit, memory, false,
    )
    .await
}

pub(super) async fn answer_private_with_memory(
    state: &AppState,
    guild: Option<u64>,
    channel: u64,
    user: u64,
    question: &str,
    memory: &crate::memory_gate::MemoryTurn,
) -> (
    String,
    Option<generation::consent::GenerationGuard>,
    Option<generation::DeliveryTiming>,
) {
    answer_with_visibility(
        state,
        guild,
        channel,
        user,
        question,
        None,
        Commit::No,
        memory,
        true,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn answer_with_visibility(
    state: &AppState,
    guild: Option<u64>,
    channel: u64,
    user: u64,
    question: &str,
    forced: Option<Persona>,
    commit: Commit,
    memory: &crate::memory_gate::MemoryTurn,
    private: bool,
) -> (
    String,
    Option<generation::consent::GenerationGuard>,
    Option<generation::DeliveryTiming>,
) {
    let scope = format!("discord:{channel}");
    // Same composition the message pipeline uses, so `/persona ask` and an
    // ordinary message never disagree about identical text. Session stickiness
    // still applies, but only to text neither layer has an opinion about.
    let route = routing_signals::route(question, forced);
    let routed = if route.is_decisive() {
        route.persona
    } else {
        AppState::lock(&state.engine)
            .session_persona(&scope)
            .unwrap_or(route.persona)
    };
    let scoped_guild = match guild {
        Some(g) => format!("discord:{g}"),
        None => format!("discord:dm:{user}"),
    };
    let scoped_user = format!("discord:{user}");
    let now = runtime::now();
    if !reserve_ask(state, &scoped_user, now) {
        return (ASK_COOLDOWN_REPLY.to_string(), None, None);
    }
    match state.generation_label() {
        None => (ask::degraded_reply(routed), None, None),
        Some(backend_label) => {
            // Same per-channel transcript, memory context, and tool loop the
            // pipeline uses, so a slash-command question and a DM continue
            // one thread. No streaming: an interaction followup is one post.
            let reputation = state.reputation_snapshot(&scoped_guild, &scoped_user);
            let mut context = pipeline::assemble_context(
                state,
                &scoped_guild,
                &scoped_user,
                &scope,
                question,
                reputation,
            );
            let continuity_scope = continuity_scope(guild, channel, user, private);
            if let Some((scope, audience)) = continuity_scope {
                context.continuity = state
                    .prepare_continuity_context(scope, user, channel, audience)
                    .await;
            }
            let guard = match generation::consent::GenerationGuard::capture(
                state,
                &generation::Ask {
                    subject: Some((&scoped_guild, &scoped_user)),
                    session_mode: if private {
                        generation::SessionMode::Ephemeral
                    } else {
                        generation::SessionMode::SourceOnly
                    },
                    scope: &scope,
                    context: &context,
                    user_input: question,
                    now,
                },
            ) {
                Ok(guard) => guard,
                Err(error) => {
                    return (
                        ask::render_failure(routed, backend_label, &error),
                        None,
                        None,
                    );
                }
            };
            let mut host = runtime::ToolScope {
                memory_turn: Some(memory),
                state,
                network: crate::platform::SocialNetwork::Discord,
                scoped_guild: scoped_guild.clone(),
                scoped_user: scoped_user.clone(),
                scoped_channel: scope.clone(),
                now,
                persona: routed,
            };
            let outcome = {
                generation::generate_with_tools_without_delivery(
                    state,
                    &mut host,
                    &generation::Ask {
                        subject: Some((&scoped_guild, &scoped_user)),
                        session_mode: if commit == Commit::Yes {
                            generation::SessionMode::Shared
                        } else {
                            generation::SessionMode::Ephemeral
                        },
                        scope: &scope,
                        context: &context,
                        user_input: question,
                        now,
                    },
                )
                .await
            };
            match outcome {
                Ok((answer, persona, provider_label, timing)) => {
                    if let Err(error) = guard.check_fresh(state).await {
                        return (
                            ask::render_failure(routed, backend_label, &error),
                            None,
                            None,
                        );
                    }
                    if commit == Commit::Yes {
                        AppState::lock(&state.engine).commit(&scope, question, &answer, now);
                    }
                    (
                        ask::render_answer(persona, provider_label, &answer),
                        Some(guard),
                        timing,
                    )
                }
                Err(error) => {
                    tracing::warn!(error = %error, backend = backend_label, "slash-command generation failed");
                    (
                        ask::render_failure(routed, backend_label, &error),
                        None,
                        None,
                    )
                }
            }
        }
    }
}

fn continuity_scope(
    guild: Option<u64>,
    channel: u64,
    user: u64,
    private: bool,
) -> Option<(
    crate::work::WorkScope,
    crate::runtime::continuity_context::ContinuityAudience,
)> {
    use crate::runtime::continuity_context::ContinuityAudience;
    match guild {
        None if private => Some((
            crate::work::WorkScope::Personal { owner: user },
            ContinuityAudience::PrivateInteraction,
        )),
        None => Some((
            crate::work::WorkScope::Personal { owner: user },
            ContinuityAudience::OwnerDm,
        )),
        Some(guild) if private => Some((
            crate::work::WorkScope::Team { guild, channel },
            ContinuityAudience::PrivateInteraction,
        )),
        Some(_) => None,
    }
}
#[cfg(test)]
mod continuity_tests {
    use super::*;
    #[test]
    fn continuity_private_dm_modal_keeps_its_private_delivery_authority() {
        use crate::runtime::continuity_context::ContinuityAudience;
        assert_eq!(
            continuity_scope(None, 70, 7, true),
            Some((
                crate::work::WorkScope::Personal { owner: 7 },
                ContinuityAudience::PrivateInteraction
            ))
        );
        assert_eq!(
            continuity_scope(None, 70, 7, false),
            Some((
                crate::work::WorkScope::Personal { owner: 7 },
                ContinuityAudience::OwnerDm
            ))
        );
        assert!(continuity_scope(Some(10), 20, 7, false).is_none());
    }
}

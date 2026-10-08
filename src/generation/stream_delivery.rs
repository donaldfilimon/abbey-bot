//! Concurrent producer polling and paced bounded delivery.
use super::*;

/// Generate through a streaming transport, posting the reply as soon as
/// [`STREAM_FIRST_POST_CHARS`] have arrived or [`STREAM_FIRST_POST_SECS`]
/// have passed, then editing the message every [`STREAM_EDIT_EVERY_SECS`]
/// until the stream ends; the final edit carries the tidied full text.
///
/// If the stream ends with tool calls and no text was produced or posted,
/// returns [`StreamEnd::Calls`] so the caller can run the tools and stream
/// again. A mixed text-and-tool turn is rejected: dispatching its calls would
/// let an already-visible claim get ahead of the actual side effect, while
/// ignoring them would make streaming disagree with completed generation. If
/// the stream fails after a partial message went out, that message is edited
/// to the honest failure line so a half-answer never stands as if whole.
#[cfg(test)]
pub async fn stream_reply<T: llm::StreamTransport + Sync, O: Outbound + Sync>(
    transport: &T,
    delivery: &Delivery<'_, O>,
    round: &Round<'_>,
) -> Result<StreamEnd, llm::LlmError> {
    let request =
        llm::build_stream_request(round.backend, round.system_prompt, round.turns, round.tools);
    let (tx, rx) = crate::generation::stream_owner::channel();
    stream_received(
        transport.post_stream(&request, tx),
        rx,
        delivery,
        round.backend.label(),
        round.persona,
        round.grounding,
        &ConversationEffects::default(),
        None,
    )
    .await
}

#[cfg(test)]
#[expect(
    clippy::too_many_arguments,
    reason = "preserve existing stream regression seam"
)]
pub(super) async fn stream_received<O: Outbound + Sync>(
    stream: impl Future<Output = Result<llm::ModelTurn, llm::LlmError>>,
    rx: crate::generation::stream_owner::DeltaReceiver,
    delivery: &Delivery<'_, O>,
    provider_label: &'static str,
    persona: Persona,
    grounding: &Grounding,
    effects: &ConversationEffects,
    guard: Option<(&AppState, &consent::GenerationGuard)>,
) -> Result<StreamEnd, llm::LlmError> {
    stream_received_timed(
        stream,
        rx,
        delivery,
        provider_label,
        persona,
        grounding,
        effects,
        guard,
        None,
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "stream delivery retains authorization and timing"
)]
pub(super) async fn stream_received_timed<O: Outbound + Sync>(
    stream: impl Future<Output = Result<llm::ModelTurn, llm::LlmError>>,
    rx: stream_owner::DeltaReceiver,
    delivery: &Delivery<'_, O>,
    provider_label: &'static str,
    persona: Persona,
    grounding: &Grounding,
    effects: &ConversationEffects,
    guard: Option<(&AppState, &consent::GenerationGuard)>,
    timing: Option<&timing::Timing>,
) -> Result<StreamEnd, llm::LlmError> {
    // Scoped fallback for borrowed token-free callers: both futures remain
    // owned here; no task is spawned or detached. Managed work is separately
    // retained by the service registry before entering this delivery seam.
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
    let overflow = rx.cancellation();
    let retained = rx.retained;
    let producer = async {
        let mut stream = std::pin::pin!(stream);
        let result = tokio::select! {
            biased;
            _ = overflow.cancelled() => {
                if retained { (&mut stream).await }
                else { Err(stream_owner::capacity()) }
            },
            _ = done_rx => {
                overflow.cancel();
                if retained { let _ = (&mut stream).await; }
                return;
            },
            result = &mut stream => result.and_then(stream_owner::validate),
        };
        let _ = result_tx.send(result);
    };
    let consumer = async {
        let result = deliver_received(
            async {
                result_rx
                    .await
                    .unwrap_or_else(|_| Err(stream_owner::cancelled()))
            },
            rx,
            delivery,
            provider_label,
            persona,
            grounding,
            effects,
            guard,
            timing,
        )
        .await;
        drop(done_tx);
        result
    };
    let ((), result) = tokio::join!(producer, consumer);
    result
}

#[expect(
    clippy::too_many_arguments,
    reason = "one stream boundary carries its immutable authorization guard"
)]
async fn deliver_received<O: Outbound + Sync>(
    stream: impl Future<Output = Result<llm::ModelTurn, llm::LlmError>>,
    mut rx: crate::generation::stream_owner::DeltaReceiver,
    delivery: &Delivery<'_, O>,
    provider_label: &'static str,
    persona: Persona,
    grounding: &Grounding,
    effects: &ConversationEffects,
    guard: Option<(&AppState, &consent::GenerationGuard)>,
    timing: Option<&timing::Timing>,
) -> Result<StreamEnd, llm::LlmError> {
    let Delivery {
        out,
        native_channel_id,
        reply_to: _,
    } = *delivery;
    let mut stream = std::pin::pin!(stream);
    let started = tokio::time::Instant::now();
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(STREAM_EDIT_EVERY_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tick.tick().await;
    let mut text = String::new();
    let mut posted: Option<String> = None;
    let mut last_edited_len = 0usize;
    let mut finished: Option<Result<llm::ModelTurn, llm::LlmError>> = None;

    // Post-or-edit with whatever has arrived, honouring the pacing rules.
    #[expect(
        clippy::too_many_arguments,
        reason = "stream flush preserves its authorization and timing observers"
    )]
    async fn flush<O: Outbound + Sync>(
        delivery: &Delivery<'_, O>,
        text: &str,
        grounding: &Grounding,
        posted: &mut Option<String>,
        last_edited_len: &mut usize,
        effects: &ConversationEffects,
        guard: Option<(&AppState, &consent::GenerationGuard)>,
        timing: Option<&timing::Timing>,
    ) -> Result<bool, llm::LlmError> {
        let Delivery {
            out,
            native_channel_id: channel,
            reply_to,
        } = *delivery;
        if text.trim().is_empty() || text.chars().count() == *last_edited_len {
            return Ok(false);
        }
        if let Some((state, guard)) = guard {
            guard.check_fresh(state).await?;
        }
        let visible = apply_grounding(text, grounding);
        // Delivery may be accepted remotely even if awaiting its result fails.
        // Close replay before handing provider output to the outbound adapter.
        effects.mark_visible_output();
        match posted {
            None => {
                let message = OutboundMessage {
                    text: visible,
                    reply_to_native_message_id: reply_to.map(str::to_string),
                    ..OutboundMessage::default()
                };
                let id = match out.send(channel, &message).await {
                    Ok(id) if !id.trim().is_empty() => id,
                    Ok(_) => {
                        let error = OutboundFailure::new(
                            OutboundFailureCategory::Internal,
                            DeliveryCertainty::PossiblySent,
                            None,
                        );
                        return Err(delivery_error(error, timing));
                    }
                    Err(error) => {
                        return Err(delivery_error(error, timing));
                    }
                };
                *posted = Some(id);
                if let Some(timing) = timing {
                    timing.posted();
                }
            }
            Some(id) => out
                .edit(channel, id, &visible)
                .await
                .map_err(|error| delivery_error(error, timing))?,
        }
        *last_edited_len = text.chars().count();
        Ok(true)
    }

    // A withdrawal after a successful preview must reach terminal replacement.
    // Other progressive delivery failures keep their original certainty and
    // return directly; attempting another edit would retry an uncertain effect.
    let progress = async {
        while finished.is_none() {
            if let Some((state, guard)) = guard {
                guard.check(state)?;
            }
            tokio::select! {
            // Deltas first: a chunk that arrived just before completion must
            // be posted/edited before the final state is decided.
            biased;
            Some(delta) = rx.recv() => {
                text = delta;
                if let Some(timing) = timing { timing.text(&text); }
                let due = !rx.failed() && posted.is_none()
                    && (text.chars().count() >= STREAM_FIRST_POST_CHARS
                        || started.elapsed().as_secs() >= STREAM_FIRST_POST_SECS);
                if due
                    && flush(delivery, &text, grounding, &mut posted, &mut last_edited_len, effects, guard, timing)
                        .await?
                {
                    // Pace from successful delivery, including time spent
                    // awaiting the outbound adapter; do not replay old ticks.
                    tick.reset();
                }
            }
            _ = tick.tick() => {
                if !rx.failed() && (posted.is_some() || started.elapsed().as_secs() >= STREAM_FIRST_POST_SECS)
                    && flush(delivery, &text, grounding, &mut posted, &mut last_edited_len, effects, guard, timing)
                        .await?
                {
                    // Pace from successful delivery, including time spent
                    // awaiting the outbound adapter; do not replay old ticks.
                    tick.reset();
                }
            }
            result = &mut stream => finished = Some(result),
            }
        }
        // Drain anything that arrived between the last recv and completion.
        while let Ok(delta) = rx.try_recv() {
            text = delta;
            if let Some(timing) = timing {
                timing.text(&text);
            }
        }
        if let Some((state, guard)) = guard {
            guard.check_fresh(state).await?;
        }
        Ok::<(), llm::LlmError>(())
    }
    .await;
    let result = match progress {
        Err(error) if error.kind() == llm::LlmErrorKind::ContextChanged => Err(error),
        Err(error) => return Err(error),
        Ok(()) => {
            let result = finished.expect("loop exits only when finished is set");
            if rx.failed() {
                Err(stream_owner::capacity())
            } else {
                result.and_then(stream_owner::validate)
            }
        }
    };
    match result {
        Ok(turn) => {
            if !turn.calls.is_empty() {
                if posted.is_none() && text.trim().is_empty() && turn.text.trim().is_empty() {
                    return Ok(StreamEnd::Calls(turn.calls));
                }
                let error = llm::LlmError::backend(
                    "backend returned text and tool calls in one streamed turn".into(),
                );
                if let Some(id) = &posted {
                    let failure = ask::render_failure(persona, provider_label, &error);
                    out.edit(native_channel_id, id, &failure)
                        .await
                        .map_err(|error| delivery_error(error, timing))?;
                }
                return Err(error);
            }
            // Only the validated terminal response can become the final reply.
            let tidy = finalize_reply(persona, &turn.text, grounding);
            if let Some(id) = &posted {
                effects.mark_visible_output();
                out.edit(native_channel_id, id, &tidy)
                    .await
                    .map_err(|error| delivery_error(error, timing))?;
                if let Some(timing) = timing {
                    timing.delivery.delivered();
                }
            }
            Ok(StreamEnd::Text(tidy, posted))
        }
        Err(e) => {
            if let Some(id) = &posted {
                let failure = ask::render_failure(persona, provider_label, &e);
                out.edit(native_channel_id, id, &failure)
                    .await
                    .map_err(|error| delivery_error(error, timing))?;
            }
            Err(e)
        }
    }
}

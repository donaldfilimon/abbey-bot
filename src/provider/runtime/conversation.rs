//! Conversation execution and effect-bound attempt ownership.
use super::*;

impl ProviderConversation<'_> {
    pub fn effects(&self) -> ConversationEffects {
        self.effects.clone()
    }
    pub fn label(&self) -> &'static str {
        lock(&self.effects.0)
            .selected()
            .and_then(|id| self.runtime.entries.get(id))
            .map_or("generation provider", |entry| entry.label)
    }
    pub fn tools_available(&self) -> bool {
        self.class == RequestClass::TextWithTools
            && lock(&self.effects.0).selected().is_none_or(|id| {
                self.runtime.entries[id]
                    .adapter
                    .as_ref()
                    .is_some_and(|adapter| adapter.tools_enabled())
            })
    }
    pub fn streams(&self) -> bool {
        self.streaming
            && lock(&self.effects.0)
                .selected()
                .and_then(|id| self.runtime.catalog.descriptor(id))
                .is_some_and(|d| d.declared_capabilities.streaming)
    }
    pub fn fallback(&mut self, error: &LlmError) -> bool {
        error.unavailable().is_none()
            && lock(&self.effects.0)
                .begin_fallback(error.retry_after().classify(error.provider_failure()))
    }
    pub async fn reserve(&mut self) -> Result<(), LlmError> {
        if self.lease.is_some() {
            return Ok(());
        }
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(if self.local {
                crate::runtime::voice_queue_secs(self.runtime.queue_secs)
            } else {
                self.runtime.queue_secs
            });
        loop {
            let selected = lock(&self.effects.0)
                .selected()
                .cloned()
                .or_else(|| self.initial.clone());
            let excluded = lock(&self.effects.0).excluded().clone();
            let mut reason = RouteUnavailableReason::NoConfiguredProvider;
            let mut waiting = None;
            {
                let mut state = lock(&self.runtime.state);
                for id in &self.runtime.order {
                    let entry = &self.runtime.entries[id];
                    let descriptor = self
                        .runtime
                        .catalog
                        .descriptor(id)
                        .expect("registered descriptor");
                    let admission = entry.admission(
                        descriptor,
                        self.class,
                        self.streaming,
                        self.local,
                        !state.blocks.failed(),
                    );
                    let capacity = admission.capacity_available;
                    let allowed = admission.capability_allowed;
                    state.router.set_admission(id, admission);
                    if !capacity
                        && allowed
                        && descriptor.eligibility.is_routable()
                        && (!self.local || entry.local_voice)
                        && selected.as_ref().is_none_or(|pin| pin == id)
                        && !excluded.contains(id)
                        && waiting.is_none()
                    {
                        waiting = Some(entry.slots.clone());
                    }
                }
                let mut chosen = None;
                if self.runtime.legacy_order && selected.is_none() {
                    for id in &self.runtime.order {
                        match state.router.select(
                            self.class,
                            self.runtime.clock.now_ms(),
                            Some(id),
                            &excluded,
                        ) {
                            Ok(selection) => {
                                chosen = Some(selection);
                                break;
                            }
                            Err(error) => reason = reason.max(error),
                        }
                    }
                } else {
                    match state.router.select(
                        self.class,
                        self.runtime.clock.now_ms(),
                        selected.as_ref(),
                        &excluded,
                    ) {
                        Ok(selection) => chosen = Some(selection),
                        Err(error) => reason = error,
                    }
                }
                if let Some((decision, attempt)) = chosen {
                    let id = decision.provider_id.clone();
                    let entry = &self.runtime.entries[&id];
                    lock(&self.effects.0).accept_selection(&decision);
                    self.initial = None;
                    let mut lease = AttemptLease {
                        runtime: self.runtime,
                        id,
                        attempt: Some(attempt),
                        started: self.runtime.clock.now_ms(),
                        permit: None,
                    };
                    match entry.slots.clone().try_acquire_owned() {
                        Ok(permit) => {
                            lease.permit = Some(permit);
                            self.lease = Some(lease);
                            return Ok(());
                        }
                        Err(_) => {
                            state.router.complete(
                                lease.attempt.take().expect("reserved"),
                                ProviderFailureKind::Busy,
                                RetryAfter::Absent,
                                None,
                                self.runtime.clock.now_ms(),
                            );
                            return Err(LlmError::busy());
                        }
                    }
                }
            }
            if reason != RouteUnavailableReason::Busy {
                return Err(LlmError::route_unavailable(reason));
            }
            let Some(slots) = waiting else {
                return Err(LlmError::route_unavailable(reason));
            };
            match tokio::time::timeout_at(deadline, slots.acquire_owned()).await {
                Ok(Ok(permit)) => drop(permit),
                _ => return Err(LlmError::route_unavailable(RouteUnavailableReason::Busy)),
            }
        }
    }
    pub async fn execute(
        &mut self,
        system: &str,
        turns: &[ChatTurn],
        tools: &[crate::tools::ToolSpec],
        style: ResponseStyle,
        deltas: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Result<ModelTurn, LlmError> {
        self.reserve().await?;
        let mut lease = self.lease.take().expect("reserved attempt");
        let entry = &self.runtime.entries[&lease.id];
        let result = if !tools.is_empty() && !self.tools_available() {
            Err(LlmError::classified(
                "tools are forbidden for this conversation",
                ProviderFailureKind::InvalidRequest,
            ))
        } else if let Some(adapter) = &entry.adapter {
            adapter
                .execute(AdapterRequest {
                    system,
                    turns,
                    tools,
                    call_id: "runtime-turn",
                    style,
                    deltas,
                })
                .await
                .and_then(|turn| {
                    if turn.text.trim().is_empty() && turn.calls.is_empty() {
                        Err(LlmError::backend(
                            "the response carried no answer text".into(),
                        ))
                    } else if (!adapter.tools_enabled() && !turn.calls.is_empty())
                        || turn
                            .calls
                            .iter()
                            .any(|call| !tools.iter().any(|tool| tool.name == call.name))
                    {
                        Err(LlmError::classified(
                            "unrequested tool calls",
                            ProviderFailureKind::ToolSchema,
                        ))
                    } else {
                        Ok(turn)
                    }
                })
        } else {
            Err(LlmError::classified(
                "no text adapter",
                ProviderFailureKind::Configuration,
            ))
        };
        lease.complete(result.as_ref().err());
        result
    }
}
impl AttemptLease<'_> {
    pub(super) fn complete(&mut self, error: Option<&LlmError>) {
        let Some(attempt) = self.attempt.take() else {
            return;
        };
        let now = self.runtime.clock.now_ms();
        let mut state = lock(&self.runtime.state);
        let kind = state.router.complete(
            attempt,
            error.map_or(ProviderFailureKind::Success, LlmError::provider_failure),
            error.map_or(RetryAfter::Absent, LlmError::retry_after),
            Some(now.saturating_sub(self.started)),
            now,
        );
        if let Some(events) = self.runtime.operational_events.get() {
            use crate::observability::{EventCode, EventComponent, EventOutcome, OperationalEvent};
            let failure = error.map_or(ProviderFailureKind::Success, LlmError::provider_failure);
            let outcome = match failure {
                ProviderFailureKind::Success => EventOutcome::Succeeded,
                ProviderFailureKind::Cancelled => EventOutcome::Cancelled,
                ProviderFailureKind::Timeout => EventOutcome::TimedOut,
                _ => EventOutcome::Failed,
            };
            if let Ok(event) = OperationalEvent::new(
                crate::runtime::now_millis(),
                EventComponent::Provider,
                EventCode::ProviderAttempt,
                outcome,
            ) {
                let _ = events.event(event.with_provider(self.id.clone()).with_duration(
                    std::time::Duration::from_millis(now.saturating_sub(self.started)),
                ));
            }
        }
        if let Some(kind) = kind.filter(|kind| kind.is_blocked()) {
            let entry = &self.runtime.entries[&self.id];
            state.blocks.block(blocks::BlockRecord {
                id: self.id.clone(),
                identity: entry.identity.clone(),
                qualification_witness: entry.qualification_witness.clone(),
                qualification_generation: entry.qualification_generation,
                blocked_unix_secs: Some(self.runtime.clock.unix_secs()),
                qualification_completed_unix_secs: entry.qualification_completed_unix_secs,
                reason: kind,
            });
        }
    }
}
impl Drop for AttemptLease<'_> {
    fn drop(&mut self) {
        if self.attempt.is_some() {
            self.complete(Some(&LlmError::classified(
                "provider attempt cancelled",
                ProviderFailureKind::Cancelled,
            )));
        }
    }
}

impl ImageUnderstanding for ProviderRuntime {
    async fn describe(&self, bytes: Vec<u8>) -> Result<String, VisionError> {
        self.image(false, bytes).await
    }
    async fn extract_text(&self, bytes: Vec<u8>) -> Result<String, VisionError> {
        self.image(true, bytes).await
    }
}
impl ProviderRuntime {
    async fn image(&self, ocr: bool, bytes: Vec<u8>) -> Result<String, VisionError> {
        let mut conversation = self.conversation(RequestClass::image(ocr), false, false, None);
        conversation
            .reserve()
            .await
            .map_err(|_| VisionError::internal("vision route unavailable"))?;
        let mut lease = conversation.lease.take().expect("image attempt reserved");
        let Some(adapter) = &self.entries[&lease.id].image else {
            return Err(VisionError::internal("no image adapter"));
        };
        conversation.effects.mark_image_submitted();
        let result = adapter.image(ocr, bytes).await;
        let error = result.as_ref().err().map(|error| {
            LlmError::classified("image provider failed", error.provider_failure())
                .with_retry_after(error.retry_after())
        });
        lease.complete(error.as_ref());
        result
    }
}

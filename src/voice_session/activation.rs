//! Start reservations, consent activation and Discord-session correlation.
use super::*;

pub(super) enum StopAuthority {
    Cancel,
    Preserve,
    Start(u64),
}

impl VoiceRuntime {
    pub async fn set_presence_with_discord_session(
        &self,
        session_id: String,
        status: impl Into<String>,
    ) {
        self.stop_to_inner(
            VoicePhase::PresenceOnly,
            status,
            StopAuthority::Cancel,
            None,
            Some(session_id),
            None,
        )
        .await;
    }

    /// Publish output-only presence only while this exact fallback owns its start.
    pub async fn set_presence_for_start(
        &self,
        generation: u64,
        session_id: String,
        status: impl Into<String>,
    ) -> bool {
        self.stop_to_inner(
            VoicePhase::PresenceOnly,
            status,
            StopAuthority::Start(generation),
            None,
            Some(session_id),
            None,
        )
        .await
    }

    /// A late fallback failure may stop only the generation that requested it.
    pub async fn fail_start(
        &self,
        generation: u64,
        status: impl Into<String>,
        error: crate::observability::OperationalErrorCategory,
    ) -> bool {
        {
            let _activation = self
                .activation_gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !self.start_is_current(generation) {
                return false;
            }
            self.music
                .stop("voice failed", PlaybackTermination::Errored);
        }
        self.stop_to_inner(
            VoicePhase::Failed,
            status,
            StopAuthority::Start(generation),
            None,
            None,
            Some(error),
        )
        .await
    }

    /// Reserve one potentially slow start attempt. Leave/pause/replacement
    /// invalidates this token without having to wait for model preflight.
    pub fn reserve_start(&self) -> u64 {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.accepting_work() {
            return 0;
        }
        let generation = self.start_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.pending_start_generation
            .store(generation, Ordering::SeqCst);
        self.start_changes.send_replace(generation);
        generation
    }

    /// Capture the lifecycle generation before a validated start performs its
    /// first await. This is deliberately not a pending-start reservation:
    /// channel and permission validation may still reject the request without
    /// superseding a legitimate preflight already in progress.
    #[must_use]
    pub fn start_operation_token(&self) -> u64 {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.start_generation.load(Ordering::SeqCst)
    }

    /// Publish a pending start only if no stop, withdrawal, safety event, or
    /// newer start crossed the caller's pre-await operation token. The check
    /// and reservation share the activation lock with cancellation, so an
    /// older request cannot resume after `/voice leave` and become a new start.
    pub fn reserve_start_if_unchanged(&self, operation_token: u64) -> Option<u64> {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.reserve_if_unchanged_locked(operation_token)
    }

    /// `reserve_start_if_unchanged`, plus the backend this start will use,
    /// captured in the same critical section. `/voice mode` writes the mode
    /// under the same lock and refuses while a start is pending, so a join
    /// either reserves first (and the switch is refused) or reserves after
    /// (and captures the new backend). Without this, a join could reserve
    /// and snapshot between the switch's check and its write, then activate
    /// the old backend while status reported the new one.
    pub fn reserve_start_with_backend(
        &self,
        operation_token: u64,
    ) -> Option<(u64, Option<VoiceBackendConfig>)> {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = self.reserve_if_unchanged_locked(operation_token)?;
        Some((generation, self.effective_backend()))
    }

    fn reserve_if_unchanged_locked(&self, operation_token: u64) -> Option<u64> {
        if !self.accepting_work() || self.start_generation.load(Ordering::SeqCst) != operation_token
        {
            return None;
        }
        let generation = self.start_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.pending_start_generation
            .store(generation, Ordering::SeqCst);
        self.start_changes.send_replace(generation);
        Some(generation)
    }

    /// Check every rule and write the mode in one critical section, so the
    /// decision is atomic with `reserve_start_with_backend`: a join either
    /// reserves first and this refuses, or reserves after and captures the
    /// switched backend.
    ///
    /// `phase` is passed in because reading it needs the async `inner` lock,
    /// which cannot be taken here. That is sound: the caller holds
    /// `transition` across both, and every phase transition takes it. The
    /// start reservation, the media epoch, and the verification run are all
    /// read here rather than from a snapshot, so none of them can move
    /// between the check and the write.
    pub fn switch_effective_mode(
        &self,
        requested: VoiceMode,
        phase: VoicePhase,
    ) -> Result<(), ModeSwitchRefusal> {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.pending_start_generation.load(Ordering::SeqCst) != 0 {
            return Err(ModeSwitchRefusal::Starting);
        }
        if self.media_epoch.load(Ordering::SeqCst) != 0 {
            return Err(ModeSwitchRefusal::MediaOpen);
        }
        if !phase.accepts_backend_change() {
            return Err(ModeSwitchRefusal::Active(phase));
        }
        // An armed run observes local inference only, so leaving `Local`
        // would record a cloud activation as local evidence. Read under the
        // same gate `begin_verification` arms under, in that lock order.
        if requested != VoiceMode::Local && self.verification_active() {
            return Err(ModeSwitchRefusal::VerificationArmed);
        }
        self.set_effective_mode(requested);
        Ok(())
    }

    /// Cancel a pending start and synchronously close any media gate it may
    /// have just opened. The same non-async mutex guards activation's final
    /// generation check and media store, so a stop request can never be
    /// followed by a stale start reopening media.
    pub fn cancel_pending_start(&self) {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = self.start_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.pending_start_generation.store(0, Ordering::SeqCst);
        self.media_epoch.store(0, Ordering::SeqCst);
        self.start_changes.send_replace(generation);
        self.mark_inspect_media_revoked();
    }

    /// Atomically deny a member's saved choice against final activation. Present
    /// or epoch-attested members can stop the call; other absent members can
    /// still withdraw their own agreement. The result carries exact-epoch
    /// teardown authority across later cache updates.
    pub fn change_consent(
        &self,
        user: u64,
        event: u64,
        choice: crate::voice_consent::Choice,
        now: u64,
        stop_call: bool,
    ) -> ConsentChange {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let change = self.consent.change(user, event, choice, now);
        let epoch = self.current_epoch.load(Ordering::SeqCst);
        let attested = {
            let sessions = self
                .discord_sessions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            sessions.attested_epoch == epoch && sessions.attested.contains(&user)
        };
        // Departures do not discard this epoch's immutable attested set. A
        // withdrawal while away must prevent reusing that authority on rejoin.
        let stop_call = choice.withdraws() && change.current && (stop_call || attested);
        if stop_call {
            let generation = self.start_generation.fetch_add(1, Ordering::SeqCst) + 1;
            self.pending_start_generation.store(0, Ordering::SeqCst);
            self.media_epoch.store(0, Ordering::SeqCst);
            self.start_changes.send_replace(generation);
            self.mark_inspect_media_revoked();
        }
        ConsentChange {
            epoch_to_stop: stop_call.then_some(epoch),
            saved: change.saved,
        }
    }

    /// Clear a completed/failed start reservation without invalidating a newer
    /// attempt that replaced it.
    pub fn finish_start_attempt(&self, generation: u64) {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self
            .pending_start_generation
            .compare_exchange(generation, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.start_changes.send_replace(0);
        }
    }

    #[must_use]
    pub fn start_is_current(&self, generation: u64) -> bool {
        self.accepting_work()
            && generation != 0
            && self.start_generation.load(Ordering::SeqCst) == generation
            && self.pending_start_generation.load(Ordering::SeqCst) == generation
    }

    /// Resolve as soon as replacement, leave, a gateway safety event, or any
    /// other transition invalidates this slow start attempt.
    pub async fn wait_for_start_cancellation(&self, generation: u64) {
        let mut changes = self.start_changes.subscribe();
        while self.start_is_current(generation) {
            if changes.changed().await.is_err() {
                break;
            }
        }
    }

    /// Bind Discord's opaque session id to the exact connecting runtime epoch.
    /// A later epoch automatically retires the binding so delayed disconnects
    /// can be distinguished from adverse events for the live call.
    pub async fn bind_discord_session(&self, epoch: u64, session_id: String) -> bool {
        let inner = self.inner.lock().await;
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if inner.epoch != epoch
            || inner.phase != VoicePhase::Connecting
            || self.current_epoch.load(Ordering::SeqCst) != epoch
        {
            return false;
        }
        let mut sessions = self
            .discord_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((bound_epoch, bound_id)) = sessions.current.as_ref() {
            if *bound_epoch == epoch && bound_id == &session_id {
                return true;
            }
            sessions.retire_current();
        }
        sessions.current = Some((epoch, session_id));
        true
    }

    /// Record a session Discord is retiring even when it belonged to
    /// no-audio presence rather than a conversational runtime epoch.
    pub fn remember_retired_discord_session(&self, session_id: String) {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.discord_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remember_retired(session_id);
    }

    /// Preserve and classify the actual adverse gateway payload, and close the
    /// media/start gates in the same synchronous critical section. The caller
    /// may then await Discord/actor cleanup without a transient recovery in the
    /// cache erasing the event that required revocation.
    #[must_use]
    pub fn revoke_for_discord_session(&self, session_id: &str) -> DiscordSessionEvent {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let epoch = self.current_epoch.load(Ordering::SeqCst);
        let sessions = self
            .discord_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let relation = if sessions
            .current
            .as_ref()
            .is_some_and(|(bound_epoch, bound_id)| *bound_epoch == epoch && bound_id == session_id)
        {
            0_u8
        } else if sessions.retired.iter().any(|retired| retired == session_id) {
            1
        } else {
            2
        };
        if relation == 1 {
            return DiscordSessionEvent::Retired;
        }
        let media_was_enabled = epoch != 0 && self.media_epoch.swap(0, Ordering::SeqCst) == epoch;
        let generation = self.start_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.pending_start_generation.store(0, Ordering::SeqCst);
        self.start_changes.send_replace(generation);
        self.mark_inspect_session_adverse();
        if relation == 0 {
            DiscordSessionEvent::Current {
                epoch,
                media_was_enabled,
            }
        } else {
            DiscordSessionEvent::Unknown {
                epoch,
                media_was_enabled,
            }
        }
    }

    /// Synchronously close the current media/start gates for a payload-backed
    /// permission or participant event that has no bot session id of its own.
    #[must_use]
    pub fn revoke_for_external_event(&self) -> u64 {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let epoch = self.current_epoch.load(Ordering::SeqCst);
        self.media_epoch.store(0, Ordering::SeqCst);
        let generation = self.start_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.pending_start_generation.store(0, Ordering::SeqCst);
        self.start_changes.send_replace(generation);
        self.mark_inspect_session_adverse();
        epoch
    }

    /// Atomically decide whether a participant join belongs to the immutable
    /// attestation for the current epoch. A delayed event for someone already
    /// attested to a replacement is ignored; every other join closes that
    /// replacement's gates before gateway cleanup awaits.
    #[must_use]
    pub fn revoke_for_unattested_participant(&self, user_id: u64) -> Option<u64> {
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let epoch = self.current_epoch.load(Ordering::SeqCst);
        let sessions = self
            .discord_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if sessions.attested_epoch == epoch && sessions.attested.contains(&user_id) {
            return None;
        }
        self.media_epoch.store(0, Ordering::SeqCst);
        let generation = self.start_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.pending_start_generation.store(0, Ordering::SeqCst);
        self.start_changes.send_replace(generation);
        self.mark_inspect_media_revoked();
        Some(epoch)
    }

    /// Advance an exact active epoch while atomically closing media and any
    /// pending start. Sharing `activation_gate` with `activate` makes the
    /// epoch check and media transition indivisible with respect to a final
    /// activation attempt.
    pub async fn begin(&self, participants: HashSet<u64>) -> u64 {
        let mut inner = self.inner.lock().await;
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let epoch = self.current_epoch.load(Ordering::SeqCst).saturating_add(1);
        self.media_epoch.store(0, Ordering::SeqCst);
        let mut sessions = self
            .discord_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        sessions.retire_current();
        sessions.attested_epoch = epoch;
        sessions.attested = participants.clone();
        self.dropped_input.store(0, Ordering::Relaxed);
        self.aborted_overruns.store(0, Ordering::Relaxed);
        self.barge_ins.store(0, Ordering::Relaxed);
        self.completed_turns.store(0, Ordering::Relaxed);
        inner.epoch = epoch;
        inner.phase = VoicePhase::Connecting;
        inner.status = "joining Discord voice safely".into();
        inner.consent_epoch = inner.consent_epoch.saturating_add(1);
        inner.participants = participants;
        inner.processing_mode = self.effective_mode();
        self.current_epoch.store(epoch, Ordering::SeqCst);
        self.publish_inspect_phase(VoicePhase::Connecting, false, None);
        epoch
    }

    pub async fn activate(
        &self,
        epoch: u64,
        start_generation: u64,
        status: impl Into<String>,
    ) -> bool {
        self.activate_inner(epoch, start_generation, status, None)
            .await
    }

    /// Open media and publish content-free verifier activation evidence in
    /// the same critical section as the lifecycle transition.
    pub async fn activate_verified(
        &self,
        epoch: u64,
        start_generation: u64,
        status: impl Into<String>,
        evidence: VerificationActivation,
    ) -> bool {
        self.activate_inner(epoch, start_generation, status, Some(evidence))
            .await
    }

    async fn activate_inner(
        &self,
        epoch: u64,
        start_generation: u64,
        status: impl Into<String>,
        verification: Option<VerificationActivation>,
    ) -> bool {
        let status = bounded_status(status.into());
        let mut inner = self.inner.lock().await;
        let _activation = self
            .activation_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if inner.epoch != epoch
            || inner.phase != VoicePhase::Connecting
            || !self.is_current(epoch)
            || !self.start_is_current(start_generation)
            || !self
                .consent
                .coverage(&inner.participants, inner.processing_mode)
                .is_ok_and(|missing| missing.is_empty())
        {
            return false;
        }
        inner.phase = VoicePhase::Listening;
        inner.status = status;
        self.pending_start_generation.store(0, Ordering::SeqCst);
        self.media_epoch.store(epoch, Ordering::SeqCst);
        self.publish_inspect_phase(VoicePhase::Listening, true, None);
        if let Some(evidence) = verification {
            self.record_verification_activation(evidence, inner.consent_epoch);
        }
        true
    }
}

#[cfg(test)]
mod presence_tests {
    use super::*;

    #[tokio::test]
    async fn startup_fallback_publication_refuses_stop_while_state_lock_is_held() {
        let runtime = VoiceRuntime::new(VoiceConfig::selected_only(
            1,
            2,
            VoiceBackendConfig::Disabled,
            true,
        ));
        let generation = runtime.reserve_start();
        let guard = runtime.inner.lock().await;
        let publish = runtime.set_presence_for_start(generation, "old".into(), "presence");
        tokio::pin!(publish);
        assert!(futures_util::poll!(&mut publish).is_pending());
        runtime.cancel_pending_start();
        drop(guard);
        assert!(!publish.await);
        assert_eq!(runtime.snapshot().await.phase, VoicePhase::Disconnected);
        let fresh = runtime.reserve_start();
        runtime.disconnect_for_replace("own replacement").await;
        assert!(
            runtime
                .set_presence_for_start(fresh, "new".into(), "fresh presence")
                .await
        );
        assert_eq!(runtime.snapshot().await.phase, VoicePhase::PresenceOnly);
        assert!(!runtime.media_enabled(runtime.current_epoch()));
    }
    #[tokio::test]
    async fn startup_fallback_late_error_cannot_cancel_newer_start() {
        let runtime = VoiceRuntime::new(VoiceConfig::selected_only(
            1,
            2,
            VoiceBackendConfig::Disabled,
            true,
        ));
        let old = runtime.reserve_start();
        let guard = runtime.inner.lock().await;
        let failure = runtime.fail_start(
            old,
            "late timeout",
            crate::observability::OperationalErrorCategory::Timeout,
        );
        tokio::pin!(failure);
        assert!(futures_util::poll!(&mut failure).is_pending());
        let new = runtime.reserve_start();
        drop(guard);
        assert!(!failure.await);
        assert!(runtime.start_is_current(new));
        assert_eq!(runtime.snapshot().await.phase, VoicePhase::Disconnected);
        assert!(
            runtime
                .fail_start(
                    new,
                    "current timeout",
                    crate::observability::OperationalErrorCategory::Timeout
                )
                .await
        );
        assert_eq!(runtime.snapshot().await.phase, VoicePhase::Failed);
        assert!(!runtime.start_is_current(new));
    }
}

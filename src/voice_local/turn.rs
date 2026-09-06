//! Per-turn recognition, cognition and synthesis behind actor-owned deadlines.
use super::*;

pub(super) async fn recognize_before_deadline(work: TurnWork) -> TurnOutcome {
    let turn = work.turn;
    let epoch = work.consent_epoch;
    let runtime = Arc::clone(&work.runtime);
    let deadline = work.captured_at + MAX_RECOGNITION_DELAY;
    match tokio::time::timeout_at(deadline, recognize_turn(work)).await {
        Ok(outcome) => outcome,
        Err(_) => {
            // Close capture immediately even if the actor is temporarily
            // awaiting a playback lock; the actor then retires the call.
            let _ = runtime.revoke_media(epoch);
            TurnOutcome::RecognitionExpired { turn }
        }
    }
}

async fn recognize_turn(mut work: TurnWork) -> TurnOutcome {
    let recognition_started = Instant::now();
    let transcript = match work.client.transcribe(&work.utterance.pcm).await {
        Ok(transcript) => transcript,
        Err(error) => {
            return TurnOutcome::Failed {
                turn: work.turn,
                stage: "speech recognition",
                error,
            };
        }
    };
    // Recognition is the last consumer of raw input. Keep only bounded text
    // and attribution metadata while a reply is being prepared.
    work.utterance.pcm = Vec::new();
    work.runtime
        .note_verification_stt_completion(work.consent_epoch);
    let safely_attributed = work.utterance.speaker_id.is_some() && !work.utterance.overlap;
    tracing::info!(
        turn = work.turn,
        recognition_seconds = recognition_started.elapsed().as_secs_f64(),
        safely_attributed,
        "local voice recognition finished"
    );
    let snapshot = work.runtime.snapshot().await;
    let withdrawal_authorized = if safely_attributed {
        if let Some(speaker_id) = work.utterance.speaker_id {
            work.runtime
                .epoch_attests(work.consent_epoch, speaker_id)
                .await
        } else {
            false
        }
    } else {
        false
    };
    // Consent withdrawal is asymmetric with activation: a safely attributed,
    // currently attested participant may stop voice without a wake name, but
    // positive prose can never start or resume it.
    if pre_wake_withdrawal(
        &transcript,
        &snapshot,
        safely_attributed,
        withdrawal_authorized,
    ) {
        return TurnOutcome::WithdrawConsent {
            turn: work.turn,
            user: work
                .utterance
                .speaker_id
                .expect("withdrawal requires attributed speaker"),
        };
    }
    if !is_addressed(
        &transcript,
        work.utterance.speaker_id,
        safely_attributed,
        work.wake_word_required,
        &work.wake,
        &work.wake_words,
        work.captured_at,
    )
    .await
    {
        return TurnOutcome::Ignored { turn: work.turn };
    }

    TurnOutcome::Addressed {
        work: Box::new(work),
        transcript,
        safely_attributed,
    }
}

pub(super) async fn generate_turn(
    work: TurnWork,
    transcript: String,
    safely_attributed: bool,
) -> TurnOutcome {
    let snapshot = work.runtime.snapshot().await;
    let persona = persona::route(&transcript, None).persona;
    let scope = voice_scope(
        work.guild_id,
        work.channel_id,
        work.consent_epoch,
        work.utterance.speaker_id,
        work.turn,
        safely_attributed,
    );
    if let Some(operational) = operational_voice_turn(&transcript, &snapshot) {
        let OperationalVoiceTurn::Reply(answer) = operational else {
            // Unattributed/overlapping speech may receive no operational or
            // generative answer, and it may never revoke another person's
            // session. Authorized withdrawal already returned above.
            return TurnOutcome::Ignored { turn: work.turn };
        };
        let spoken_answer = crate::offline_voice::spoken_text(&answer);
        let audio = match work.client.synthesize(&spoken_answer).await {
            Ok(audio) => audio,
            Err(error) => {
                return TurnOutcome::Failed {
                    turn: work.turn,
                    stage: "speech synthesis",
                    error,
                };
            }
        };
        return TurnOutcome::Ready {
            turn: work.turn,
            ready_at: Instant::now(),
            scope,
            transcript,
            spoken_answer,
            persist: false,
            audio,
        };
    }
    let scoped_guild = format!("discord:{}", work.guild_id);
    let scoped_user = work.utterance.speaker_id.map_or_else(
        || "discord:voice:unattributed".into(),
        |id| format!("discord:{id}"),
    );
    let context = if safely_attributed {
        let reputation = work.state.reputation_snapshot(&scoped_guild, &scoped_user);
        pipeline::assemble_context(
            &work.state,
            &scoped_guild,
            &scoped_user,
            &scope,
            &transcript,
            reputation,
        )
    } else {
        PersonaContext::empty()
    };
    let generation = generation::generate_without_delivery(
        &work.state,
        &work.backend,
        persona,
        &generation::Ask {
            session_mode: crate::generation::SessionMode::Shared,
            scope: &scope,
            context: &context,
            user_input: &transcript,
            now: runtime::now(),
        },
        Some(VOICE_SYSTEM_SUFFIX),
    )
    .await;
    let (answer, _) = match generation {
        Ok(answer) => answer,
        Err(error) => {
            return TurnOutcome::Failed {
                turn: work.turn,
                stage: "Abbey reasoning",
                error: error.to_string(),
            };
        }
    };
    let spoken_answer = crate::offline_voice::spoken_text(&answer);
    let synthesis_started = Instant::now();
    let audio = match work.client.synthesize(&spoken_answer).await {
        Ok(audio) => audio,
        Err(error) => {
            return TurnOutcome::Failed {
                turn: work.turn,
                stage: "speech synthesis",
                error,
            };
        }
    };
    tracing::info!(
        turn = work.turn,
        synthesis_seconds = synthesis_started.elapsed().as_secs_f64(),
        "local voice synthesis finished"
    );
    TurnOutcome::Ready {
        turn: work.turn,
        ready_at: Instant::now(),
        scope,
        transcript,
        spoken_answer,
        persist: safely_attributed,
        audio,
    }
}

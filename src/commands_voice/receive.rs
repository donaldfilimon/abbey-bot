//! Epoch-gated Songbird receive handling.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock, Weak};

use serenity::all::GuildId;
use songbird::events::{CoreEvent, Event, EventContext, EventHandler};
use tokio::sync::{Mutex, mpsc, watch};

use super::discord::{pause_call_for_consent, remove_call_for_consent};
use crate::offline_voice::{FRAME_SAMPLES, VoiceFrame};
use crate::vad::{EnergyVad, Vad};
use crate::voice_session::{VoicePhase, VoiceRuntime};

#[derive(Debug, PartialEq, Eq)]
enum TickDecision {
    Forward(VoiceFrame),
    RevokeForUnattested(Option<u64>),
}

/// Classify one decoded Songbird tick without touching Discord or the actor.
/// Unknown SSRCs and speakers outside the immutable consent attestation win
/// over every otherwise valid frame, so not even an attested speaker from the
/// same tick can reach STT after the boundary is observed.
fn classify_tick<'a>(
    streams: impl IntoIterator<Item = (u32, Option<&'a [i16]>)>,
    mappings: &HashMap<u32, u64>,
    attested: &HashSet<u64>,
    sequence: u64,
) -> TickDecision {
    let mut voiced: Vec<(u32, &[i16], u64, u64)> = Vec::new();
    for (ssrc, decoded_voice) in streams {
        let Some(user_id) = mappings.get(&ssrc).copied() else {
            return TickDecision::RevokeForUnattested(None);
        };
        if !attested.contains(&user_id) {
            return TickDecision::RevokeForUnattested(Some(user_id));
        }
        let Some(samples) = decoded_voice else {
            continue;
        };
        if !EnergyVad::default().is_voice(samples) {
            continue;
        }
        let energy = samples.iter().fold(0_u64, |sum, sample| {
            let value = i64::from(*sample);
            sum.saturating_add(value.unsigned_abs().saturating_mul(value.unsigned_abs()))
        });
        voiced.push((ssrc, samples, energy, user_id));
    }
    voiced.sort_unstable_by_key(|(_, _, energy, _)| std::cmp::Reverse(*energy));
    let frame = if let Some((_ssrc, samples, _, user_id)) = voiced.first() {
        let mut samples = samples.to_vec();
        samples.resize(FRAME_SAMPLES, 0);
        samples.truncate(FRAME_SAMPLES);
        VoiceFrame {
            sequence,
            speaker_id: Some(*user_id),
            samples,
            overlap: voiced.len() > 1,
        }
    } else {
        VoiceFrame::silence(sequence)
    };
    TickDecision::Forward(frame)
}

#[derive(Clone)]
struct DiscordAudioForwarder {
    tx: mpsc::Sender<VoiceFrame>,
    driver_disconnect: watch::Sender<bool>,
    call: Weak<Mutex<songbird::Call>>,
    manager: Weak<songbird::Songbird>,
    guild_id: GuildId,
    attested: Arc<HashSet<u64>>,
    mappings: Arc<RwLock<HashMap<u32, u64>>>,
    sequence: Arc<AtomicU64>,
    runtime: Arc<VoiceRuntime>,
    epoch: u64,
}

#[serenity::async_trait]
impl EventHandler for DiscordAudioForwarder {
    async fn act(&self, context: &EventContext<'_>) -> Option<Event> {
        if !self.runtime.is_current(self.epoch) {
            return Some(Event::Cancel);
        }
        match context {
            EventContext::SpeakingStateUpdate(update) => {
                if let Some(user_id) = update.user_id {
                    self.mappings
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .insert(update.ssrc, user_id.0);
                }
                None
            }
            EventContext::ClientDisconnect(disconnect) => {
                self.mappings
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .retain(|_, user_id| *user_id != disconnect.user_id.0);
                None
            }
            EventContext::VoiceTick(tick) => {
                if !self.runtime.media_enabled(self.epoch) {
                    return None;
                }
                let mappings = self
                    .mappings
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                let decision = classify_tick(
                    tick.speaking
                        .iter()
                        .map(|(ssrc, data)| (*ssrc, data.decoded_voice.as_deref())),
                    &mappings,
                    &self.attested,
                    0,
                );
                if let TickDecision::RevokeForUnattested(user_id) = decision {
                    let mut participants = self.attested.as_ref().clone();
                    if let Some(user_id) = user_id {
                        participants.insert(user_id);
                    }
                    if self.runtime.revoke_media(self.epoch) {
                        let runtime = Arc::clone(&self.runtime);
                        let call = self.call.clone();
                        let manager = self.manager.clone();
                        let guild_id = self.guild_id;
                        let epoch = self.epoch;
                        tokio::spawn(async move {
                            let Some(pause) = runtime
                                .begin_participant_pause_epoch_for_consent(
                                    epoch,
                                    participants,
                                    "audio stopped before an unknown or unattested speaker frame was forwarded",
                                )
                                .await
                            else {
                                return;
                            };
                            if let Some(call) = call.upgrade() {
                                pause_call_for_consent(&call).await;
                            }
                            pause.finish().await;
                            let transition = runtime.transition.lock().await;
                            let snapshot = runtime.snapshot().await;
                            if snapshot.epoch != epoch.saturating_add(1)
                                || snapshot.phase != VoicePhase::AwaitingConsent
                            {
                                drop(transition);
                                return;
                            }
                            if let Some(manager) = manager.upgrade() {
                                remove_call_for_consent(&manager, guild_id).await;
                            }
                            drop(transition);
                        });
                    }
                    return Some(Event::Cancel);
                }
                let TickDecision::Forward(mut frame) = decision else {
                    unreachable!("unattested tick returned above")
                };
                frame.sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
                let voiced = EnergyVad::default().is_voice(&frame.samples);
                let sent = self.runtime.with_media_enabled(self.epoch, || {
                    let sent = self.tx.try_send(frame);
                    if voiced && sent.is_ok() {
                        self.runtime.note_verification_decoded_receive();
                    }
                    sent
                })?;
                match sent {
                    Ok(()) => None,
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        self.runtime.note_dropped_input();
                        None
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => Some(Event::Cancel),
                }
            }
            EventContext::DriverDisconnect(_) => {
                let _ = self.driver_disconnect.send(true);
                Some(Event::Cancel)
            }
            _ => None,
        }
    }
}

pub(super) struct ReceiveHandlerInstall<'a> {
    pub(super) call: &'a Arc<Mutex<songbird::Call>>,
    pub(super) manager: Weak<songbird::Songbird>,
    pub(super) guild_id: GuildId,
    pub(super) runtime: &'a Arc<VoiceRuntime>,
    pub(super) epoch: u64,
    pub(super) attested: HashSet<u64>,
    pub(super) tx: mpsc::Sender<VoiceFrame>,
    pub(super) driver_disconnect: watch::Sender<bool>,
}

pub(super) async fn install_receive_handlers(input: ReceiveHandlerInstall<'_>) {
    let handler = DiscordAudioForwarder {
        tx: input.tx,
        driver_disconnect: input.driver_disconnect,
        // Avoid Call -> handler -> Call retaining the driver after leave.
        call: Arc::downgrade(input.call),
        manager: input.manager,
        guild_id: input.guild_id,
        attested: Arc::new(input.attested),
        mappings: Arc::new(RwLock::new(HashMap::new())),
        sequence: Arc::new(AtomicU64::new(0)),
        runtime: Arc::clone(input.runtime),
        epoch: input.epoch,
    };
    let mut call = input.call.lock().await;
    call.add_global_event(Event::Core(CoreEvent::SpeakingStateUpdate), handler.clone());
    call.add_global_event(Event::Core(CoreEvent::ClientDisconnect), handler.clone());
    call.add_global_event(Event::Core(CoreEvent::VoiceTick), handler.clone());
    call.add_global_event(Event::Core(CoreEvent::DriverDisconnect), handler);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voiced() -> Vec<i16> {
        vec![i16::MAX; FRAME_SAMPLES]
    }

    #[test]
    fn unknown_or_unattested_stream_prevents_every_frame_from_reaching_stt() {
        let audio = voiced();
        let attested = HashSet::from([7]);

        assert_eq!(
            classify_tick(
                [(11, Some(audio.as_slice()))],
                &HashMap::new(),
                &attested,
                1
            ),
            TickDecision::RevokeForUnattested(None)
        );

        let mappings = HashMap::from([(11, 8)]);
        assert_eq!(
            classify_tick([(11, Some(audio.as_slice()))], &mappings, &attested, 2),
            TickDecision::RevokeForUnattested(Some(8))
        );

        // One valid attested speaker must not be forwarded when any other
        // stream in the same tick is unknown.
        let mappings = HashMap::from([(10, 7)]);
        assert_eq!(
            classify_tick(
                [(10, Some(audio.as_slice())), (11, Some(audio.as_slice())),],
                &mappings,
                &attested,
                3,
            ),
            TickDecision::RevokeForUnattested(None)
        );
    }

    #[test]
    fn only_attested_speakers_are_forwarded_with_attribution() {
        let audio = voiced();
        let mappings = HashMap::from([(10, 7)]);
        let decision = classify_tick(
            [(10, Some(audio.as_slice()))],
            &mappings,
            &HashSet::from([7]),
            9,
        );
        let TickDecision::Forward(frame) = decision else {
            panic!("attested speaker must be forwarded")
        };
        assert_eq!(frame.sequence, 9);
        assert_eq!(frame.speaker_id, Some(7));
        assert!(!frame.overlap);
        assert_eq!(frame.samples.len(), FRAME_SAMPLES);
    }
}

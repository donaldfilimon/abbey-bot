//! Epoch-gated Songbird receive handling.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock, Weak};

use serenity::all::GuildId;
use songbird::events::{CoreEvent, Event, EventContext, EventHandler};
use tokio::sync::{Mutex, mpsc, watch};

use super::discord::{pause_call_for_consent, remove_call_for_consent};
use crate::offline_voice::{FRAME_SAMPLES, VoiceFrame, frame_is_voice};
use crate::voice_session::{VoicePhase, VoiceRuntime};

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
                let mut voiced: Vec<(u32, &[i16], u64, u64)> = Vec::new();
                let mut unattested = None;
                for (ssrc, data) in &tick.speaking {
                    let Some(user_id) = mappings.get(ssrc).copied() else {
                        unattested = Some(None);
                        break;
                    };
                    if !self.attested.contains(&user_id) {
                        unattested = Some(Some(user_id));
                        break;
                    }
                    let Some(samples) = data.decoded_voice.as_deref() else {
                        continue;
                    };
                    if !frame_is_voice(samples) {
                        continue;
                    }
                    let energy = samples.iter().fold(0_u64, |sum, sample| {
                        let value = i64::from(*sample);
                        sum.saturating_add(
                            value.unsigned_abs().saturating_mul(value.unsigned_abs()),
                        )
                    });
                    voiced.push((*ssrc, samples, energy, user_id));
                }
                if let Some(user_id) = unattested {
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
                                .begin_pause_epoch_for_consent(
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
                voiced.sort_unstable_by_key(|(_, _, energy, _)| std::cmp::Reverse(*energy));
                let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
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
                let sent = self
                    .runtime
                    .with_media_enabled(self.epoch, || self.tx.try_send(frame))?;
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

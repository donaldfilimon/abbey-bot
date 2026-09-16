//! Thin Discord/Songbird shell for Abbey voice.
//!
//! Commands validate runtime permission, exact-channel membership, explicit
//! participant attestation, and provider readiness while the call is muted and
//! self-deafened. Only after a public disclosure succeeds do they enable
//! decoding. The provider actors live in `voice_local` and `voice_openai`.

use std::sync::Arc;
use std::time::Duration;

use serenity::all::{ChannelId, ChannelType, GuildId};
use tokio::sync::{Mutex, mpsc, oneshot, watch};

use crate::gateway::shared::clamp_message;
use crate::offline_voice::MlxAudioClient;
use crate::voice::{VoiceBackendConfig, VoiceMode};
use crate::voice_local::LocalSession;
use crate::voice_openai::OpenAiSession;
use crate::voice_session::{SessionControl, SharedPlayback, VerificationActivation, VoiceRuntime};
use crate::{Context, Error};

mod acknowledgement;
mod play;
use play::{voice_pause, voice_play, voice_resume_music, voice_stop_music, voice_volume};
mod auto_listen;
mod consent;
mod discord;
mod events;
mod receive;
mod start;
mod supervision;
use start::start_voice;
mod ux;
mod verification;

#[cfg(test)]
mod acknowledgement_tests;

use acknowledgement::{
    AcknowledgedContext, acknowledge_with_transition, authorize_and_close_media,
    with_acknowledged_context,
};
use discord::*;
use receive::{ReceiveHandlerInstall, install_receive_handlers};

#[cfg(test)]
pub(crate) struct VoiceLeaveTransitionProbe {
    pub(crate) entered: tokio::sync::Semaphore,
    pub(crate) release: tokio::sync::Semaphore,
}

#[cfg(test)]
impl VoiceLeaveTransitionProbe {
    pub(crate) fn new() -> Self {
        Self {
            entered: tokio::sync::Semaphore::new(0),
            release: tokio::sync::Semaphore::new(0),
        }
    }
}

#[cfg(test)]
pub(crate) struct VoiceLeaveTransitionProbeKey;

#[cfg(test)]
impl serenity::prelude::TypeMapKey for VoiceLeaveTransitionProbeKey {
    type Value = Arc<VoiceLeaveTransitionProbe>;
}

#[cfg(test)]
async fn hold_voice_leave_transition_for_test(ctx: &serenity::all::Context) {
    let probe = ctx
        .data
        .read()
        .await
        .get::<VoiceLeaveTransitionProbeKey>()
        .cloned();
    if let Some(probe) = probe {
        probe.entered.add_permits(1);
        probe.release.acquire().await.unwrap().forget();
    }
}

/// Select the exact disconnected guild call's decoder before joining. Changing
/// Songbird's shared defaults here would race another guild's call creation.
async fn configure_disconnected_call(call: &Arc<Mutex<songbird::Call>>, mode: VoiceMode) {
    call.lock().await.set_config(initial_songbird_config(mode));
}

pub use auto_listen::{
    AutoListenStartup, AutoListenWhilePresent, try_auto_listen_at_startup,
    try_auto_listen_while_present,
};
pub use consent::{voice_consent, voice_notice};
pub use events::on_gateway_event;
pub use supervision::autojoin_self_deafened;
pub use ux::dispatch_ux_component;
pub use verification::voice_verify;

const INPUT_QUEUE_FRAMES: usize = 64;
const OPENAI_READY_TIMEOUT: Duration = Duration::from_secs(20);
const LOCAL_HEALTH_TIMEOUT: Duration = Duration::from_secs(600);
const SIDECAR_STATUS_TIMEOUT: Duration = Duration::from_secs(2);

/// Clear this exact slow-start reservation on every return path. A newer
/// request is unaffected because `finish_start_attempt` compares generations.
struct StartAttempt {
    runtime: Arc<VoiceRuntime>,
    generation: u64,
}

impl Drop for StartAttempt {
    fn drop(&mut self) {
        self.runtime.finish_start_attempt(self.generation);
    }
}

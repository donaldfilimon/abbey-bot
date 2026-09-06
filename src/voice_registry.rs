//! Guild-scoped voice runtime admission and ownership.
//!
//! A reservation is acquired before Discord REST preflight. The same lock
//! publishes a newly created runtime, captures its start-operation token and
//! lets `/voice leave` invalidate either side of that publication boundary.

use crate::{
    inspect::VoiceInspectRegistry,
    service::{OperationRegistry, telemetry::TelemetryRequests},
    voice::VoiceTemplate,
    voice_session::VoiceRuntime,
};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};

pub const MAX_VOICE_RUNTIMES: usize = 256;
const UNCONFIGURED: &str = "Voice is not configured.";
const STOPPING: &str = "The voice service is stopping.";
const CAPACITY: &str =
    "Voice has reached its server capacity. Restart Abbey before adding another server.";
const JOIN_PENDING: &str = "A voice join is already being checked for this server.";
const CHANNEL_CONFLICT: &str =
    "Voice is already prepared in another channel. Use `/voice leave` before moving it.";
const STORAGE_UNAVAILABLE: &str = "Voice consent storage is unavailable for this server.";
const RESERVATION_EXPIRED: &str = "This voice join was cancelled. Try `/voice join` again.";

struct Entry {
    runtime: Arc<VoiceRuntime>,
    reservation_generation: u64,
}

struct State {
    accepting: bool,
    template: Option<VoiceTemplate>,
    data_dir: Option<PathBuf>,
    home_guild: Option<u64>,
    inspect: Option<Arc<VoiceInspectRegistry>>,
    service: Option<OperationRegistry>,
    telemetry: Option<TelemetryRequests>,
    runtimes: HashMap<u64, Entry>,
    consent: HashMap<u64, Arc<crate::voice_consent_store::ConsentStore>>,
    pending: HashMap<u64, u64>,
    next_generation: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            accepting: true,
            template: None,
            data_dir: None,
            home_guild: None,
            inspect: None,
            service: None,
            telemetry: None,
            runtimes: HashMap::new(),
            consent: HashMap::new(),
            pending: HashMap::new(),
            next_generation: 0,
        }
    }
}

#[derive(Default)]
struct RegistryInner {
    state: Mutex<State>,
}

/// Process-owned runtime registry. `Default` is intentionally unconfigured.
#[derive(Default)]
pub struct VoiceRegistry {
    inner: Arc<RegistryInner>,
}

/// One guild-scoped preflight lease. Dropping it cancels only its still-current
/// unpublished reservation; it never removes a runtime.
pub struct JoinReservation {
    inner: Weak<RegistryInner>,
    guild: u64,
    generation: u64,
    operation_token: OnceLock<u64>,
}

impl JoinReservation {
    #[must_use]
    pub fn operation_token(&self) -> Option<u64> {
        self.operation_token.get().copied()
    }
}

impl Drop for JoinReservation {
    fn drop(&mut self) {
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        let mut state = lock(&inner.state);
        if state.pending.get(&self.guild) == Some(&self.generation) {
            state.pending.remove(&self.guild);
        }
    }
}

impl VoiceRegistry {
    pub fn configure(
        &self,
        template: VoiceTemplate,
        default: Option<Arc<VoiceRuntime>>,
        data_dir: Option<PathBuf>,
        inspect: Arc<VoiceInspectRegistry>,
    ) -> Result<(), &'static str> {
        let mut state = lock(&self.inner.state);
        if !state.accepting {
            return Err(STOPPING);
        }
        if state.template.is_some() {
            return Err("Voice was already configured.");
        }
        if let Some(runtime) = &default {
            if runtime.config.guild_id == 0
                || runtime.config.channel_id == 0
                || runtime.config.mode() != template.mode()
            {
                return Err("The default voice runtime does not match its template.");
            }
            state.home_guild = Some(runtime.config.guild_id);
            state
                .consent
                .insert(runtime.config.guild_id, Arc::clone(&runtime.consent));
            state.runtimes.insert(
                runtime.config.guild_id,
                Entry {
                    runtime: Arc::clone(runtime),
                    reservation_generation: 0,
                },
            );
        }
        state.template = Some(template);
        state.data_dir = data_dir;
        state.inspect = Some(inspect);
        Ok(())
    }

    #[must_use]
    pub fn is_configured(&self) -> bool {
        lock(&self.inner.state).template.is_some()
    }

    #[must_use]
    pub fn template(&self) -> Option<VoiceTemplate> {
        lock(&self.inner.state).template.clone()
    }

    #[must_use]
    pub fn get(&self, guild: u64) -> Option<Arc<VoiceRuntime>> {
        lock(&self.inner.state)
            .runtimes
            .get(&guild)
            .map(|entry| Arc::clone(&entry.runtime))
    }

    pub fn reserve_join(self: &Arc<Self>, guild: u64) -> Result<JoinReservation, &'static str> {
        if guild == 0 {
            return Err(UNCONFIGURED);
        }
        let mut state = lock(&self.inner.state);
        if state.template.is_none() {
            return Err(UNCONFIGURED);
        }
        if !state.accepting {
            return Err(STOPPING);
        }
        if state.pending.contains_key(&guild) && !state.runtimes.contains_key(&guild) {
            return Err(JOIN_PENDING);
        }
        let already_occupied = state.consent.contains_key(&guild)
            || state.runtimes.contains_key(&guild)
            || state.pending.contains_key(&guild);
        if !already_occupied && occupied_guilds(&state) >= MAX_VOICE_RUNTIMES {
            return Err(CAPACITY);
        }
        state.next_generation = state
            .next_generation
            .checked_add(1)
            .ok_or("Voice join identity is exhausted.")?;
        let generation = state.next_generation;
        state.pending.insert(guild, generation);
        let operation_token = OnceLock::new();
        if let Some(entry) = state.runtimes.get(&guild) {
            let _ = operation_token.set(entry.runtime.start_operation_token());
        }
        Ok(JoinReservation {
            inner: Arc::downgrade(&self.inner),
            guild,
            generation,
            operation_token,
        })
    }

    /// Cancel a REST preflight or invalidate a published runtime's captured
    /// start token. A leave for an unknown guild creates no registry entry.
    pub fn cancel_join(&self, guild: u64) -> bool {
        let mut state = lock(&self.inner.state);
        let cancelled = state.pending.remove(&guild).is_some();
        if let Some(entry) = state.runtimes.get(&guild) {
            entry.runtime.cancel_pending_start();
        }
        cancelled
    }

    pub async fn get_or_create(
        &self,
        guild: u64,
        channel: u64,
        reservation: &JoinReservation,
    ) -> Result<Arc<VoiceRuntime>, &'static str> {
        if guild == 0 || channel == 0 {
            return Err(UNCONFIGURED);
        }
        let Some(owner) = reservation.inner.upgrade() else {
            return Err(RESERVATION_EXPIRED);
        };
        if !Arc::ptr_eq(&owner, &self.inner) || reservation.guild != guild {
            return Err(RESERVATION_EXPIRED);
        }
        let mut state = lock(&self.inner.state);
        if !state.accepting {
            return Err(STOPPING);
        }
        if state.pending.get(&guild) != Some(&reservation.generation) {
            if let Some(entry) = state.runtimes.get(&guild)
                && entry.reservation_generation == reservation.generation
                && entry.runtime.config.channel_id == channel
            {
                return Ok(Arc::clone(&entry.runtime));
            }
            return Err(RESERVATION_EXPIRED);
        }
        if let Some(entry) = state.runtimes.get(&guild) {
            if entry.runtime.config.channel_id != channel {
                return Err(CHANNEL_CONFLICT);
            }
            let runtime = Arc::clone(&entry.runtime);
            let _ = reservation
                .operation_token
                .set(runtime.start_operation_token());
            state.pending.remove(&guild);
            return Ok(runtime);
        }

        let template = state.template.clone().ok_or(UNCONFIGURED)?;
        let config = template.for_destination(guild, channel)?;
        let consent = match state.consent.get(&guild) {
            Some(consent) => Arc::clone(consent),
            None => {
                let consent_dir = consent_directory(&state, guild)?;
                Arc::new(crate::voice_consent_store::ConsentStore::load(
                    consent_dir.as_deref(),
                    guild,
                ))
            }
        };
        let inspect = state.inspect.clone().ok_or(UNCONFIGURED)?;
        let runtime = Arc::new(VoiceRuntime::new_with_inspect(
            config,
            inspect,
            Arc::clone(&consent),
        ));
        if let Some(service) = &state.service {
            runtime.attach_service(service.clone());
        }
        if let Some(telemetry) = &state.telemetry {
            runtime.attach_telemetry(telemetry.clone());
        }
        let _ = reservation
            .operation_token
            .set(runtime.start_operation_token());
        state.runtimes.insert(
            guild,
            Entry {
                runtime: Arc::clone(&runtime),
                reservation_generation: reservation.generation,
            },
        );
        state.consent.insert(guild, Arc::clone(&consent));
        state.pending.remove(&guild);
        Ok(runtime)
    }

    /// Permanently close and remove only the exact disconnected runtime
    /// supplied by the caller. The caller must retain its transition lock
    /// through verified disconnect and this retirement call.
    pub fn retire(&self, guild: u64, runtime: &Arc<VoiceRuntime>) -> bool {
        let mut state = lock(&self.inner.state);
        if state.pending.contains_key(&guild)
            || state
                .runtimes
                .get(&guild)
                .is_none_or(|entry| !Arc::ptr_eq(&entry.runtime, runtime))
        {
            return false;
        }
        runtime.begin_draining();
        state.runtimes.remove(&guild);
        true
    }

    pub fn attach_service(&self, service: OperationRegistry) {
        let mut state = lock(&self.inner.state);
        if state.service.is_none() {
            state.service = Some(service.clone());
        }
        for entry in state.runtimes.values() {
            entry.runtime.attach_service(service.clone());
        }
    }

    pub fn attach_telemetry(&self, telemetry: TelemetryRequests) {
        let mut state = lock(&self.inner.state);
        if state.telemetry.is_none() {
            state.telemetry = Some(telemetry.clone());
        }
        for entry in state.runtimes.values() {
            entry.runtime.attach_telemetry(telemetry.clone());
        }
    }

    /// Close registry admission and every published runtime under one lock.
    pub fn begin_draining(&self) -> Vec<Arc<VoiceRuntime>> {
        let mut state = lock(&self.inner.state);
        let runtimes = state
            .runtimes
            .values()
            .map(|entry| Arc::clone(&entry.runtime))
            .collect::<Vec<_>>();
        if state.accepting {
            state.accepting = false;
            state.pending.clear();
            for runtime in &runtimes {
                runtime.begin_draining();
            }
        }
        runtimes
    }
}

fn occupied_guilds(state: &State) -> usize {
    state
        .consent
        .keys()
        .chain(state.runtimes.keys())
        .chain(state.pending.keys())
        .copied()
        .collect::<HashSet<_>>()
        .len()
}

fn consent_directory(state: &State, guild: u64) -> Result<Option<PathBuf>, &'static str> {
    let Some(data_dir) = &state.data_dir else {
        return Ok(None);
    };
    if state.home_guild == Some(guild) {
        return Ok(Some(data_dir.clone()));
    }
    let guilds = data_dir.join("voice-guilds");
    ensure_private_directory(&guilds)?;
    let guild = guilds.join(guild.to_string());
    ensure_private_directory(&guild)?;
    Ok(Some(guild))
}

fn ensure_private_directory(path: &Path) -> Result<(), &'static str> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(STORAGE_UNAVAILABLE),
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| STORAGE_UNAVAILABLE)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(STORAGE_UNAVAILABLE);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        if metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.permissions().mode() & 0o7777 != 0o700
        {
            return Err(STORAGE_UNAVAILABLE);
        }
    }
    Ok(())
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
#[path = "voice_registry/tests.rs"]
mod tests;

//! Dedicated consent persistence, independent of periodic memory snapshots.
//! Disk work is serialized outside the short snapshot lock. No grant becomes
//! usable until the latest whole ledger is durable; withdrawals deny immediately.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use crate::{
    voice::VoiceMode,
    voice_consent::{Choice, Ledger},
};

const FILE: &str = "voice-consent.json";
const PENDING: &str = "voice-consent.pending";
const UNAVAILABLE: &str = "Saved voice choices are unavailable. Voice cannot start or resume; the operator must check the consent store before any restart or retry.";

struct State {
    ledger: Ledger,
    revision: u64,
    saved_revision: u64,
    available: bool,
}

pub struct ConsentStore {
    dir: Option<PathBuf>,
    state: Mutex<State>,
    writer: Mutex<()>,
}

pub struct StoreChange {
    pub current: bool,
    pub saved: tokio::task::JoinHandle<Result<bool, &'static str>>,
}

impl ConsentStore {
    #[cfg(test)]
    pub fn acknowledged_fixture(guild: u64, users: &[u64], mode: VoiceMode) -> Self {
        let store = Self::load(None, guild);
        let mut state = store.state.lock().unwrap();
        state.available = true;
        for &user in users {
            state.ledger.apply(user, 1, Choice::Agree(mode), 1);
        }
        drop(state);
        store
    }
    pub fn load(dir: Option<&Path>, guild: u64) -> Self {
        let loaded = dir.and_then(|dir| {
            if dir.join(PENDING).exists() {
                return None;
            }
            match fs::read(dir.join(FILE)) {
                Ok(bytes) => serde_json::from_slice::<Ledger>(&bytes)
                    .ok()
                    .filter(|ledger| ledger.version == 1 && ledger.guild == guild),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Some(Ledger::new(guild)),
                Err(_) => None,
            }
        });
        let available = loaded.is_some();
        Self {
            dir: dir.map(Path::to_path_buf),
            state: Mutex::new(State {
                ledger: loaded.unwrap_or_else(|| Ledger::new(guild)),
                revision: 0,
                saved_revision: 0,
                available,
            }),
            writer: Mutex::new(()),
        }
    }

    pub fn agrees(&self, user: u64, mode: VoiceMode) -> bool {
        let Ok(state) = self.state.try_lock() else {
            return false;
        };
        state.available && state.revision == state.saved_revision && state.ledger.agrees(user, mode)
    }

    pub fn coverage(
        &self,
        users: &std::collections::HashSet<u64>,
        mode: VoiceMode,
    ) -> Result<Vec<u64>, &'static str> {
        if mode == VoiceMode::Disabled {
            return Ok(Vec::new());
        }
        let state = self.state.try_lock().map_err(|_| UNAVAILABLE)?;
        if !state.available || state.revision != state.saved_revision {
            return Err(UNAVAILABLE);
        }
        let mut missing: Vec<_> = users
            .iter()
            .copied()
            .filter(|user| !state.ledger.agrees(*user, mode))
            .collect();
        missing.sort_unstable();
        Ok(missing)
    }

    /// Reserve synchronously, then launch cancellation-independent disk work.
    /// The caller serializes reservation and media revocation against activation.
    /// Discord event order wins
    /// even if an older Agree handler was delayed before it reached this method.
    pub fn change(
        self: &Arc<Self>,
        user: u64,
        event: u64,
        choice: Choice,
        now: u64,
    ) -> StoreChange {
        let (revision, event) = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !state.available {
                (None, event)
            } else if !state.ledger.apply(user, event, choice, now) {
                (Some(0), event)
            } else {
                state.revision += 1;
                (Some(state.revision), state.ledger.members[&user].last_event)
            }
        };
        let store = Arc::clone(self);
        let saved = tokio::task::spawn_blocking(move || match revision {
            None => Err(UNAVAILABLE),
            Some(0) => Ok(false),
            Some(_) => {
                store.flush()?;
                let state = store
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                Ok(state
                    .ledger
                    .members
                    .get(&user)
                    .is_some_and(|member| member.last_event == event))
            }
        });
        StoreChange {
            current: revision != Some(0),
            saved,
        }
    }

    fn flush(&self) -> Result<(), &'static str> {
        let _writer = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = self.dir.as_ref().ok_or(UNAVAILABLE)?;
        loop {
            let (ledger, revision) = {
                let state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if !state.available {
                    return Err(UNAVAILABLE);
                }
                if state.revision == state.saved_revision {
                    return Ok(());
                }
                (state.ledger.clone(), state.revision)
            };
            if save(dir, &ledger).is_err() {
                self.state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .available = false;
                // A pending marker makes an interrupted write fail closed on
                // reload. If even creating that marker failed, invalidate the
                // old authority as a second independent best-effort boundary.
                let invalidated = fs::remove_file(dir.join(FILE)).is_ok();
                let _ = sync_dir(dir);
                tracing::error!(
                    invalidated,
                    "voice consent persistence failed; audio must remain disabled; operator intervention required before restart"
                );
                return Err(UNAVAILABLE);
            }
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.revision != revision {
                continue;
            }
            // Serialize unlink with choice reservation: a withdrawal already
            // in the desired ledger must never lose its pending marker. The
            // more expensive directory fsync stays outside the state lock.
            if fs::remove_file(dir.join(PENDING)).is_err() {
                state.available = false;
                return Err(UNAVAILABLE);
            }
            drop(state);
            if sync_dir(dir).is_err() {
                self.state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .available = false;
                return Err(UNAVAILABLE);
            }
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.revision == revision {
                state.saved_revision = revision;
                return Ok(());
            }
        }
    }
}

fn private_file(path: &Path) -> io::Result<std::fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(dir)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

fn save(dir: &Path, ledger: &Ledger) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let pending = dir.join(PENDING);
    if !pending.exists() {
        private_file(&pending)?.sync_all()?;
        sync_dir(dir)?;
    }
    let temp = dir.join("voice-consent.json.tmp");
    // Only this serialized writer owns the temp; no unrelated file is removed.
    match fs::remove_file(&temp) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut file = private_file(&temp)?;
    serde_json::to_writer(&mut file, ledger)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temp, dir.join(FILE))?;
    sync_dir(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashSet,
        sync::atomic::{AtomicU64, Ordering},
    };

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "abbey-voice-consent-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn receipt_survives_reload_but_not_wrong_guild_or_mode() {
        let dir = Scratch::new();
        let store = Arc::new(ConsentStore::load(Some(&dir.0), 10));
        assert!(
            store
                .change(20, 100, Choice::Agree(VoiceMode::Local), 5)
                .saved
                .await
                .unwrap()
                .unwrap()
        );
        let reloaded = ConsentStore::load(Some(&dir.0), 10);
        assert!(reloaded.agrees(20, VoiceMode::Local));
        assert!(!reloaded.agrees(20, VoiceMode::OpenAi));
        assert_eq!(
            reloaded.coverage(&HashSet::from([20, 21]), VoiceMode::Local),
            Ok(vec![21])
        );
        assert!(!ConsentStore::load(Some(&dir.0), 11).agrees(20, VoiceMode::Local));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(dir.0.join(FILE)).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[tokio::test]
    async fn later_stop_denies_immediately_and_wins_over_delayed_agree() {
        let dir = Scratch::new();
        let store = Arc::new(ConsentStore::load(Some(&dir.0), 10));
        store
            .change(20, 100, Choice::Agree(VoiceMode::Local), 1)
            .saved
            .await
            .unwrap()
            .unwrap();
        let late_grant = store.change(20, 101, Choice::Agree(VoiceMode::OpenAi), 2);
        let revoke = store.change(20, 102, Choice::Withdraw, 3);
        assert!(!store.agrees(20, VoiceMode::Local));
        assert!(!store.agrees(20, VoiceMode::OpenAi));
        late_grant.saved.await.unwrap().unwrap();
        assert!(revoke.saved.await.unwrap().unwrap());
        assert!(
            !store
                .change(20, 101, Choice::Agree(VoiceMode::Local), 2)
                .saved
                .await
                .unwrap()
                .unwrap()
        );
        let reloaded = ConsentStore::load(Some(&dir.0), 10);
        assert!(!reloaded.agrees(20, VoiceMode::Local));
        assert!(!reloaded.agrees(20, VoiceMode::OpenAi));
    }

    #[tokio::test]
    async fn failed_withdrawal_poison_and_pending_marker_prevent_old_grant_reload() {
        let dir = Scratch::new();
        let store = Arc::new(ConsentStore::load(Some(&dir.0), 10));
        store
            .change(20, 100, Choice::Agree(VoiceMode::Local), 1)
            .saved
            .await
            .unwrap()
            .unwrap();
        // Force replacement failure on all platforms, without relying on root
        // or OS-specific permission behavior.
        fs::create_dir(dir.0.join("voice-consent.json.tmp")).unwrap();
        assert!(
            store
                .change(20, 101, Choice::Withdraw, 2)
                .saved
                .await
                .unwrap()
                .is_err()
        );
        assert!(!store.agrees(20, VoiceMode::Local));
        assert!(dir.0.join(PENDING).exists());
        assert!(!ConsentStore::load(Some(&dir.0), 10).agrees(20, VoiceMode::Local));
    }

    #[tokio::test]
    async fn failed_grant_never_becomes_usable_and_cancelled_waiter_still_saves() {
        let dir = Scratch::new();
        let store = Arc::new(ConsentStore::load(Some(&dir.0), 10));
        fs::create_dir(dir.0.join("voice-consent.json.tmp")).unwrap();
        assert!(
            store
                .change(20, 100, Choice::Agree(VoiceMode::Local), 1)
                .saved
                .await
                .unwrap()
                .is_err()
        );
        assert!(!store.agrees(20, VoiceMode::Local));
        assert!(!ConsentStore::load(Some(&dir.0), 10).agrees(20, VoiceMode::Local));
        let other = Scratch::new();
        let store = Arc::new(ConsentStore::load(Some(&other.0), 10));
        drop(store.change(20, 100, Choice::Agree(VoiceMode::Local), 1));
        // This independent write drains the serialized writer, including the
        // explicit first choice whose Discord waiter disappeared.
        store
            .change(21, 101, Choice::Agree(VoiceMode::Local), 1)
            .saved
            .await
            .unwrap()
            .unwrap();
        assert!(ConsentStore::load(Some(&other.0), 10).agrees(20, VoiceMode::Local));
    }

    #[test]
    fn missing_configuration_corrupt_storage_and_interrupted_commit_fail_closed() {
        assert!(
            ConsentStore::load(None, 10)
                .coverage(&HashSet::from([20]), VoiceMode::Local)
                .is_err()
        );
        let dir = Scratch::new();
        fs::write(dir.0.join(FILE), b"broken").unwrap();
        assert!(
            ConsentStore::load(Some(&dir.0), 10)
                .coverage(&HashSet::new(), VoiceMode::Local)
                .is_err()
        );
        fs::write(
            dir.0.join(FILE),
            serde_json::to_vec(&Ledger::new(10)).unwrap(),
        )
        .unwrap();
        fs::write(dir.0.join(PENDING), b"").unwrap();
        assert!(
            ConsentStore::load(Some(&dir.0), 10)
                .coverage(&HashSet::new(), VoiceMode::Local)
                .is_err()
        );
    }
}

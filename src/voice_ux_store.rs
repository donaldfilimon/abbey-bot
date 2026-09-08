//! Durable voice UX session binding under `ABBEY_DATA_DIR`.
//!
//! Presentation authorization only — not consent epochs or media gates.

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};

use crate::voice_ux::{Phase, Session};

const FILE: &str = "voice-ux-sessions.json";
const PENDING: &str = "voice-ux-sessions.pending";

#[derive(Debug, Default, Serialize, Deserialize)]
struct Document {
    version: u32,
    sessions: BTreeMap<String, Session>,
}

pub struct VoiceUxStore {
    dir: Option<PathBuf>,
    state: Mutex<Document>,
}

impl VoiceUxStore {
    #[must_use]
    pub fn load(dir: Option<&Path>) -> Self {
        let document = dir
            .map(|dir| dir.join(FILE))
            .and_then(|path| match fs::read(&path) {
                Ok(bytes) => serde_json::from_slice::<Document>(&bytes)
                    .ok()
                    .filter(|doc| doc.version == 1),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Some(Document {
                    version: 1,
                    sessions: BTreeMap::new(),
                }),
                Err(_) => None,
            })
            .unwrap_or(Document {
                version: 1,
                sessions: BTreeMap::new(),
            });
        Self {
            dir: dir.map(Path::to_path_buf),
            state: Mutex::new(document),
        }
    }

    pub fn insert(&self, session: Session) -> Result<(), &'static str> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "voice UX store lock poisoned")?;
        state.sessions.insert(session.sid.clone(), session);
        self.persist_locked(&state)
    }

    pub fn get(&self, sid: &str) -> Result<Option<Session>, &'static str> {
        let state = self
            .state
            .lock()
            .map_err(|_| "voice UX store lock poisoned")?;
        Ok(state.sessions.get(sid).cloned())
    }

    /// Update phase without renewing expiry.
    pub fn set_phase(&self, sid: &str, phase: Phase) -> Result<Option<Session>, &'static str> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "voice UX store lock poisoned")?;
        let Some(session) = state.sessions.get_mut(sid) else {
            return Ok(None);
        };
        session.phase = phase;
        let cloned = session.clone();
        self.persist_locked(&state)?;
        Ok(Some(cloned))
    }

    pub fn remove(&self, sid: &str) -> Result<Option<Session>, &'static str> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "voice UX store lock poisoned")?;
        let removed = state.sessions.remove(sid);
        if removed.is_some() {
            self.persist_locked(&state)?;
        }
        Ok(removed)
    }

    fn persist_locked(&self, state: &Document) -> Result<(), &'static str> {
        let Some(dir) = &self.dir else {
            return Ok(());
        };
        fs::create_dir_all(dir).map_err(|_| "voice UX store directory unavailable")?;
        let pending = dir.join(PENDING);
        let final_path = dir.join(FILE);
        let bytes = serde_json::to_vec_pretty(state).map_err(|_| "voice UX store encode failed")?;
        {
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&pending)
                .map_err(|_| "voice UX store temp create failed")?;
            file.write_all(&bytes)
                .map_err(|_| "voice UX store temp write failed")?;
            file.sync_all()
                .map_err(|_| "voice UX store temp sync failed")?;
        }
        fs::rename(&pending, &final_path).map_err(|_| "voice UX store publish failed")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice_ux::{Phase, Session};

    fn session(sid: &str, expiry: u64) -> Session {
        Session {
            sid: sid.into(),
            guild: 1,
            user: 2,
            channel: 3,
            expiry,
            phase: Phase::Status,
        }
    }

    #[test]
    fn memory_store_create_load_update_destroy() {
        let store = VoiceUxStore::load(None);
        store.insert(session("abc", 100)).unwrap();
        let loaded = store.get("abc").unwrap().unwrap();
        assert_eq!(loaded.expiry, 100);
        assert_eq!(loaded.phase, Phase::Status);
        let updated = store
            .set_phase("abc", Phase::ConfirmLeave)
            .unwrap()
            .unwrap();
        assert_eq!(updated.phase, Phase::ConfirmLeave);
        assert_eq!(updated.expiry, 100, "expiry must not renew on phase update");
        assert!(store.get("missing").unwrap().is_none());
        assert!(store.remove("abc").unwrap().is_some());
        assert!(store.get("abc").unwrap().is_none());
    }

    #[test]
    fn durable_store_round_trips_under_data_dir() {
        let dir = std::env::temp_dir().join(format!(
            "abbey-voice-ux-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let store = VoiceUxStore::load(Some(&dir));
        store.insert(session("sidone", 50)).unwrap();
        store.set_phase("sidone", Phase::Left).unwrap();
        drop(store);
        let reloaded = VoiceUxStore::load(Some(&dir));
        let session = reloaded.get("sidone").unwrap().unwrap();
        assert_eq!(session.phase, Phase::Left);
        assert_eq!(session.expiry, 50);
        let _ = fs::remove_dir_all(&dir);
    }
}

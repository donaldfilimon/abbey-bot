//! Content-free operational blocks, deliberately separate from qualification evidence.
use super::{ProviderFailureKind, ProviderId, ProviderIdentityHashes};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};

pub(crate) struct BlockWriter {
    sender: mpsc::Sender<Option<BlockRecord>>,
    pending: Arc<AtomicUsize>,
    handle: tokio::task::JoinHandle<()>,
    failure: Arc<crate::service::failure::FailureSignal>,
    stopping: Arc<AtomicBool>,
}
impl BlockWriter {
    pub(crate) fn idle(&self) -> bool {
        self.pending.load(Ordering::SeqCst) == 0
    }
    pub(crate) fn failure(&self) -> Arc<crate::service::failure::FailureSignal> {
        self.failure.clone()
    }
    pub(crate) fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        let _ = self.sender.send(None);
    }
    pub(crate) async fn joined(&mut self) -> Result<(), tokio::task::JoinError> {
        (&mut self.handle).await
    }
}
impl Drop for BlockWriter {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BlockRecord {
    pub id: ProviderId,
    pub identity: ProviderIdentityHashes,
    pub reason: ProviderFailureKind,
    #[serde(default)]
    pub qualification_witness: Option<String>,
    #[serde(default)]
    pub qualification_generation: Option<u64>,
    #[serde(default)]
    pub blocked_unix_secs: Option<u64>,
    #[serde(default)]
    pub qualification_completed_unix_secs: Option<u64>,
}
pub(super) struct BlockStore {
    path: Option<PathBuf>,
    records: Vec<BlockRecord>,
    failed: Arc<AtomicBool>,
    writer: Option<(mpsc::Sender<Option<BlockRecord>>, Arc<AtomicUsize>)>,
}
impl BlockStore {
    pub fn memory() -> Self {
        Self {
            path: None,
            records: Vec::new(),
            failed: Arc::new(AtomicBool::new(false)),
            writer: None,
        }
    }
    pub fn records(&self) -> &[BlockRecord] {
        &self.records
    }
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }
    pub fn read(path: PathBuf) -> Result<Self, String> {
        #[cfg(unix)]
        if let Some(parent) = path.parent()
            && parent
                .try_exists()
                .map_err(|_| "cannot inspect provider block directory")?
        {
            crate::provider::manifest::validate_state_directory(
                parent,
                crate::provider::manifest::effective_user_id(),
            )
            .map_err(|_| "provider block directory must be private and owner-only")?;
        }
        if std::fs::symlink_metadata(path.with_extension("pending")).is_ok() {
            return Err("provider block publication is incomplete".into());
        }
        let records = match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => return Err("cannot read provider block state".into()),
            Ok(metadata) => {
                if !metadata.is_file() || metadata.len() > 65536 {
                    return Err("invalid provider block state file".into());
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if metadata.mode() & 0o077 != 0
                        || metadata.uid() != crate::provider::manifest::effective_user_id()
                    {
                        return Err("provider block state must be owner-only".into());
                    }
                }
                let bytes = std::fs::read(&path).map_err(|_| "cannot read provider block state")?;
                let records: Vec<BlockRecord> =
                    serde_json::from_slice(&bytes).map_err(|_| "invalid provider block state")?;
                if records.iter().any(|record| !record.reason.is_blocked()) {
                    return Err("invalid provider block reason".into());
                }
                records
            }
        };
        Ok(Self {
            path: Some(path),
            records,
            failed: Arc::new(AtomicBool::new(false)),
            writer: None,
        })
    }
    pub fn attach_writer(&mut self) -> BlockWriter {
        assert!(self.writer.is_none(), "provider writer attached once");
        let (sender, receiver) = mpsc::channel();
        let pending = Arc::new(AtomicUsize::new(0));
        let owned_pending = pending.clone();
        let mut store = Self {
            path: self.path.clone(),
            records: self.records.clone(),
            failed: self.failed.clone(),
            writer: None,
        };
        let failure = Arc::new(crate::service::failure::FailureSignal::default());
        let failed = failure.clone();
        let stopping = Arc::new(AtomicBool::new(false));
        let stopping_owned = stopping.clone();
        let handle = tokio::task::spawn_blocking(move || {
            struct Guard(Arc<AtomicBool>, Arc<crate::service::failure::FailureSignal>);
            impl Drop for Guard {
                fn drop(&mut self) {
                    if !self.0.load(Ordering::SeqCst) {
                        self.1.trigger();
                    }
                }
            }
            let _guard = Guard(stopping_owned, failed.clone());
            while let Ok(Some(record)) = receiver.recv() {
                store.block(record);
                if store.failed() {
                    failed.trigger();
                }
                owned_pending.fetch_sub(1, Ordering::SeqCst);
            }
        });
        self.writer = Some((sender.clone(), pending.clone()));
        BlockWriter {
            sender,
            pending,
            handle,
            failure,
            stopping,
        }
    }
    pub fn block(&mut self, record: BlockRecord) {
        self.records
            .retain(|old| old.id != record.id || old.identity != record.identity);
        self.records.push(record.clone());
        if let Some((sender, pending)) = &self.writer {
            pending.fetch_add(1, Ordering::SeqCst);
            if sender.send(Some(record)).is_err() {
                pending.fetch_sub(1, Ordering::SeqCst);
                self.failed.store(true, Ordering::SeqCst);
            }
            return;
        }
        if let Some(path) = &self.path {
            let result = (|| -> std::io::Result<()> {
                let parent = path
                    .parent()
                    .ok_or_else(|| std::io::Error::other("missing state directory"))?;
                let mut directory = std::fs::DirBuilder::new();
                directory.recursive(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    directory.mode(0o700);
                }
                directory.create(parent)?;
                #[cfg(unix)]
                crate::provider::manifest::validate_state_directory(
                    parent,
                    crate::provider::manifest::effective_user_id(),
                )
                .map_err(|_| std::io::Error::other("unsafe provider state directory"))?;
                let marker = path.with_extension("pending");
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options.open(&marker)?;
                file.write_all(b"provider block publication pending\n")?;
                file.sync_all()?;
                #[cfg(unix)]
                std::fs::File::open(parent)?.sync_all()?;
                let temporary = path.with_extension("new");
                let mut file = options.open(&temporary)?;
                serde_json::to_writer(&mut file, &self.records)?;
                file.sync_all()?;
                drop(file);
                std::fs::rename(&temporary, path)?;
                #[cfg(unix)]
                std::fs::File::open(parent)?.sync_all()?;
                std::fs::remove_file(marker)?;
                #[cfg(unix)]
                std::fs::File::open(parent)?.sync_all()?;
                Ok(())
            })();
            if result.is_err() {
                self.failed.store(true, Ordering::SeqCst);
                tracing::error!("provider operational block publication failed; routing disabled");
            }
        }
    }
}

#[cfg(all(test, unix))]
mod writer_tests {
    use super::*;
    #[tokio::test]
    async fn background_publication_failure_wakes_root_without_a_second_attempt() {
        use std::os::unix::fs::PermissionsExt;
        let mut entropy = [0u8; 16];
        getrandom::fill(&mut entropy).unwrap();
        let suffix: String = entropy.iter().map(|b| format!("{b:02x}")).collect();
        let directory = std::env::temp_dir().join(format!("abbey-block-writer-{suffix}"));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.join("blocks.json");
        let mut store = BlockStore::read(path.clone()).unwrap();
        let mut writer = store.attach_writer();
        // The exclusive marker collision deterministically fails actual publication.
        std::fs::write(
            path.with_extension("pending"),
            b"controlled unfinished publication",
        )
        .unwrap();
        store.block(BlockRecord {
            id: ProviderId::parse("primary").unwrap(),
            identity: ProviderIdentityHashes {
                abbey_binary_sha256: "a".repeat(64),
                provider_binary_sha256: None,
                model_sha256: None,
                os_sha256: None,
                tool_schema_sha256: "b".repeat(64),
                sandbox_sha256: None,
            },
            reason: ProviderFailureKind::ResponseSchema,
            qualification_witness: None,
            qualification_generation: None,
            blocked_unix_secs: Some(1),
            qualification_completed_unix_secs: None,
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            writer.failure().notified(),
        )
        .await
        .unwrap();
        assert!(store.failed());
        writer.stop();
        writer.joined().await.unwrap();
        assert!(writer.idle());
        std::fs::remove_dir_all(directory).unwrap();
    }
}

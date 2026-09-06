//! Content-free operational blocks, deliberately separate from qualification evidence.
use super::{ProviderFailureKind, ProviderId, ProviderIdentityHashes};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BlockRecord {
    pub id: ProviderId,
    pub identity: ProviderIdentityHashes,
    pub reason: ProviderFailureKind,
}
pub(super) struct BlockStore {
    path: Option<PathBuf>,
    records: Vec<BlockRecord>,
    failed: bool,
}
impl BlockStore {
    pub fn memory() -> Self {
        Self {
            path: None,
            records: Vec::new(),
            failed: false,
        }
    }
    pub fn records(&self) -> &[BlockRecord] {
        &self.records
    }
    pub fn failed(&self) -> bool {
        self.failed
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
            failed: false,
        })
    }
    pub fn block(
        &mut self,
        id: ProviderId,
        identity: ProviderIdentityHashes,
        reason: ProviderFailureKind,
    ) {
        self.records
            .retain(|record| record.id != id || record.identity != identity);
        self.records.push(BlockRecord {
            id,
            identity,
            reason,
        });
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
                self.failed = true;
                tracing::error!("provider operational block publication failed; routing disabled");
            }
        }
    }
}

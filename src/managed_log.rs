//! One synchronized managed writer; failures close observability through a fatal hook.
use crate::{
    observability::{FatalHook, ManagedFailure, OperationalEvent},
    readiness::private::PrivateDirectory,
};
use std::{path::Path, sync::Mutex};
pub const ROTATION_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_LINE_BYTES: usize = 16 * 1024;
pub struct ManagedLog {
    directory: PrivateDirectory,
    failed: Mutex<Option<ManagedFailure>>,
    fatal: FatalHook,
}
impl ManagedLog {
    pub fn open(home: &Path, fatal: FatalHook) -> Result<Self, ManagedFailure> {
        let directory = PrivateDirectory::open(home, &["Library", "Logs", "abbey-bot"])
            .map_err(|_| ManagedFailure::Directory)?;
        for name in [
            "abbey-bot.events.jsonl",
            "abbey-bot.events.jsonl.1",
            "abbey-bot.events.jsonl.2",
            "abbey-bot.events.jsonl.3",
            "abbey-bot.events.jsonl.4",
            "abbey-bot.events.jsonl.5",
        ] {
            directory.validate_optional(name)?;
        }
        // Prove active-file creation/sync before credentials; no fabricated event.
        directory.append_rotating(&[], ROTATION_BYTES)?;
        Ok(Self {
            directory,
            failed: Mutex::new(None),
            fatal,
        })
    }
    pub fn write(&self, event: &OperationalEvent) -> Result<(), ManagedFailure> {
        let result = (|| {
            let mut failure = self
                .failed
                .lock()
                .map_err(|_| ManagedFailure::WriterPoisoned)?;
            if let Some(error) = *failure {
                return Err(error);
            }
            let result = event.encode().and_then(|line| {
                if line.len() > MAX_LINE_BYTES {
                    return Err(ManagedFailure::LineLimit);
                }
                self.directory.append_rotating(&line, ROTATION_BYTES)
            });
            if let Err(error) = result {
                *failure = Some(error);
            }
            result
        })();
        if let Err(error) = result {
            (self.fatal)(error);
        }
        result
    }
}
#[cfg(all(test, unix))]
mod tests;

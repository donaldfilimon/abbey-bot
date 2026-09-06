//! Descriptor-relative private storage. No caller-controlled leaf names enter from
//! configuration. All mutation is serialized by the owning directory instance.
use crate::observability::ManagedFailure;
use std::path::Path;

#[cfg(unix)]
mod platform {
    use super::*;
    use rustix::fs::{self, AtFlags, Mode, OFlags};
    use std::{
        fs::File,
        io::{Read, Write},
        os::unix::fs::MetadataExt,
        path::Component,
        sync::Mutex,
    };

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(super) enum PublicationStep {
        Write,
        FileSync,
        Rename,
        DirectorySync,
    }
    pub struct PrivateDirectory {
        file: File,
        serial: Mutex<()>,
    }
    fn safe_file(file: &File) -> Result<std::fs::Metadata, ManagedFailure> {
        let meta = file.metadata().map_err(|_| ManagedFailure::File)?;
        if !meta.is_file()
            || meta.uid() != rustix::process::geteuid().as_raw()
            || meta.mode() & 0o7777 != 0o600
            || meta.nlink() != 1
        {
            return Err(ManagedFailure::UnsafeFileType);
        }
        Ok(meta)
    }
    fn file_flags() -> OFlags {
        OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
    }
    impl PrivateDirectory {
        pub fn open(home: &Path, parts: &[&str]) -> Result<Self, ManagedFailure> {
            Self::open_inner(home, parts, true)
        }
        pub fn open_existing(home: &Path, parts: &[&str]) -> Result<Self, ManagedFailure> {
            Self::open_inner(home, parts, false)
        }
        fn open_inner(home: &Path, parts: &[&str], create: bool) -> Result<Self, ManagedFailure> {
            if !home.is_absolute() {
                return Err(ManagedFailure::Directory);
            }
            let mut current: File = fs::open(
                "/",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| ManagedFailure::Directory)?
            .into();
            let components: Vec<_> = home.components().collect();
            for (index, component) in components.iter().enumerate() {
                let Component::Normal(name) = component else {
                    if matches!(component, Component::RootDir) {
                        continue;
                    }
                    return Err(ManagedFailure::Directory);
                };
                current = fs::openat(
                    &current,
                    *name,
                    OFlags::RDONLY | OFlags::DIRECTORY | file_flags(),
                    Mode::empty(),
                )
                .map_err(|_| ManagedFailure::UnsafeFileType)?
                .into();
                let meta = current.metadata().map_err(|_| ManagedFailure::Directory)?;
                let owner = rustix::process::geteuid().as_raw();
                let home_leaf = index + 1 == components.len();
                // Root-owned sticky ancestors (e.g. /tmp) permit isolated fixtures;
                // the actual HOME is always owner-controlled and non-writable by others.
                if (meta.uid() != owner && (home_leaf || meta.uid() != 0))
                    || (meta.mode() & 0o022 != 0
                        && (home_leaf || meta.uid() != 0 || meta.mode() & 0o1000 == 0))
                {
                    return Err(ManagedFailure::UnsafeFileType);
                }
            }
            for (index, part) in parts.iter().enumerate() {
                if create {
                    match fs::mkdirat(&current, *part, Mode::from_raw_mode(0o700)) {
                        Ok(()) => {}
                        Err(rustix::io::Errno::EXIST) => {}
                        Err(_) => return Err(ManagedFailure::Directory),
                    }
                }
                current = fs::openat(
                    &current,
                    *part,
                    OFlags::RDONLY | OFlags::DIRECTORY | file_flags(),
                    Mode::empty(),
                )
                .map_err(|_| ManagedFailure::UnsafeFileType)?
                .into();
                let meta = current.metadata().map_err(|_| ManagedFailure::Directory)?;
                if meta.uid() != rustix::process::geteuid().as_raw()
                    || meta.mode() & 0o022 != 0
                    || (index + 1 == parts.len() && meta.mode() & 0o7777 != 0o700)
                {
                    return Err(ManagedFailure::UnsafeFileType);
                }
            }
            Ok(Self {
                file: current,
                serial: Mutex::new(()),
            })
        }
        fn optional_file(&self, name: &str, flags: OFlags) -> Result<Option<File>, ManagedFailure> {
            match fs::openat(&self.file, name, flags | file_flags(), Mode::empty()) {
                Ok(fd) => {
                    let file: File = fd.into();
                    safe_file(&file)?;
                    Ok(Some(file))
                }
                Err(rustix::io::Errno::NOENT) => Ok(None),
                Err(_) => Err(ManagedFailure::UnsafeFileType),
            }
        }
        pub fn read_optional(
            &self,
            name: &str,
            limit: usize,
        ) -> Result<Option<Vec<u8>>, ManagedFailure> {
            self.validate_optional(name)?;
            if self.optional_file(name, OFlags::RDONLY)?.is_none() {
                return Ok(None);
            }
            self.read_required(name, limit).map(Some)
        }
        pub fn read_required(&self, name: &str, limit: usize) -> Result<Vec<u8>, ManagedFailure> {
            let _guard = self
                .serial
                .lock()
                .map_err(|_| ManagedFailure::WriterPoisoned)?;
            self.validate_optional(name)?;
            let mut file = self
                .optional_file(name, OFlags::RDONLY)?
                .ok_or(ManagedFailure::File)?;
            let before = safe_file(&file)?;
            if before.len() > limit as u64 {
                return Err(ManagedFailure::File);
            }
            let mut bytes = Vec::new();
            (&mut file)
                .take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| ManagedFailure::File)?;
            let after = safe_file(&file)?;
            if bytes.len() > limit
                || before.len() != after.len()
                || before.mtime() != after.mtime()
                || before.mtime_nsec() != after.mtime_nsec()
                || before.ctime() != after.ctime()
                || before.ctime_nsec() != after.ctime_nsec()
                || !self.same_named_file(name, &file)?
            {
                return Err(ManagedFailure::File);
            }
            Ok(bytes)
        }
        pub fn validate_optional(&self, name: &str) -> Result<(), ManagedFailure> {
            let meta = self
                .file
                .metadata()
                .map_err(|_| ManagedFailure::Directory)?;
            if !meta.is_dir()
                || meta.uid() != rustix::process::geteuid().as_raw()
                || meta.mode() & 0o7777 != 0o700
            {
                return Err(ManagedFailure::UnsafeFileType);
            }
            self.optional_file(name, OFlags::RDONLY).map(|_| ())
        }
        fn same_named_file(&self, name: &str, file: &File) -> Result<bool, ManagedFailure> {
            let held = safe_file(file)?;
            let Some(current) = self.optional_file(name, OFlags::RDONLY)? else {
                return Ok(false);
            };
            let named = safe_file(&current)?;
            Ok(held.dev() == named.dev() && held.ino() == named.ino())
        }
        pub fn publish(&self, name: &str, bytes: &[u8]) -> Result<(), ManagedFailure> {
            self.publish_with(name, bytes, |_| Ok(()))
        }
        pub(super) fn publish_with(
            &self,
            name: &str,
            bytes: &[u8],
            mut before: impl FnMut(PublicationStep) -> Result<(), ManagedFailure>,
        ) -> Result<(), ManagedFailure> {
            let _guard = self
                .serial
                .lock()
                .map_err(|_| ManagedFailure::WriterPoisoned)?;
            self.validate_optional(name)?;
            let mut entropy = [0u8; 32];
            getrandom::fill(&mut entropy).map_err(|_| ManagedFailure::Identity)?;
            let temporary = format!(".managed-{}.tmp", crate::readiness::hex(&entropy));
            let mut file: File = fs::openat(
                &self.file,
                &temporary,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | file_flags(),
                Mode::from_raw_mode(0o600),
            )
            .map_err(|_| ManagedFailure::File)?
            .into();
            let result = (|| {
                safe_file(&file)?;
                before(PublicationStep::Write)?;
                file.write_all(bytes).map_err(|_| ManagedFailure::Write)?;
                before(PublicationStep::FileSync)?;
                file.sync_all().map_err(|_| ManagedFailure::Sync)?;
                self.validate_optional(name)?;
                before(PublicationStep::Rename)?;
                fs::renameat(&self.file, &temporary, &self.file, name)
                    .map_err(|_| ManagedFailure::Rename)?;
                before(PublicationStep::DirectorySync)?;
                self.file
                    .sync_all()
                    .map_err(|_| ManagedFailure::DirectorySync)
            })();
            if result.is_err() && self.same_named_file(&temporary, &file)? {
                fs::unlinkat(&self.file, &temporary, AtFlags::empty())
                    .map_err(|_| ManagedFailure::Remove)?;
            }
            result
        }
        pub fn remove_matching(
            &self,
            name: &str,
            limit: usize,
            predicate: impl FnOnce(&[u8]) -> Result<bool, ManagedFailure>,
        ) -> Result<bool, ManagedFailure> {
            let _guard = self
                .serial
                .lock()
                .map_err(|_| ManagedFailure::WriterPoisoned)?;
            self.validate_optional(name)?;
            let Some(mut file) = self.optional_file(name, OFlags::RDONLY)? else {
                return Ok(false);
            };
            if safe_file(&file)?.len() > limit as u64 {
                return Err(ManagedFailure::Encode);
            }
            let mut bytes = Vec::new();
            (&mut file)
                .take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| ManagedFailure::File)?;
            if bytes.len() > limit {
                return Err(ManagedFailure::Encode);
            }
            if !predicate(&bytes)? || !self.same_named_file(name, &file)? {
                return Ok(false);
            }
            fs::unlinkat(&self.file, name, AtFlags::empty()).map_err(|_| ManagedFailure::Remove)?;
            self.file
                .sync_all()
                .map_err(|_| ManagedFailure::DirectorySync)?;
            Ok(true)
        }
        pub fn append_rotating(&self, line: &[u8], limit: u64) -> Result<(), ManagedFailure> {
            let _guard = self
                .serial
                .lock()
                .map_err(|_| ManagedFailure::WriterPoisoned)?;
            let names = [
                "abbey-bot.events.jsonl",
                "abbey-bot.events.jsonl.1",
                "abbey-bot.events.jsonl.2",
                "abbey-bot.events.jsonl.3",
                "abbey-bot.events.jsonl.4",
                "abbey-bot.events.jsonl.5",
            ];
            // Validate every archive before the first mutation, even without rotation.
            for name in names {
                self.validate_optional(name)?;
            }
            let mut active = self.optional_file(names[0], OFlags::WRONLY | OFlags::APPEND)?;
            let length = active
                .as_ref()
                .map_or(Ok(0), |f| safe_file(f).map(|m| m.len()))?;
            if length
                .checked_add(line.len() as u64)
                .is_none_or(|size| size > limit)
            {
                active = None;
                if self.optional_file(names[5], OFlags::RDONLY)?.is_some() {
                    fs::unlinkat(&self.file, names[5], AtFlags::empty())
                        .map_err(|_| ManagedFailure::Remove)?;
                }
                for index in (0..5).rev() {
                    if self.optional_file(names[index], OFlags::RDONLY)?.is_some() {
                        fs::renameat(&self.file, names[index], &self.file, names[index + 1])
                            .map_err(|_| ManagedFailure::Rename)?;
                    }
                }
                self.file
                    .sync_all()
                    .map_err(|_| ManagedFailure::DirectorySync)?;
            }
            let mut file = match active {
                Some(file) => file,
                None => fs::openat(
                    &self.file,
                    names[0],
                    OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | file_flags(),
                    Mode::from_raw_mode(0o600),
                )
                .map_err(|_| ManagedFailure::File)?
                .into(),
            };
            safe_file(&file)?;
            file.write_all(line).map_err(|_| ManagedFailure::Write)?;
            file.sync_all().map_err(|_| ManagedFailure::Sync)?;
            self.file
                .sync_all()
                .map_err(|_| ManagedFailure::DirectorySync)
        }
    }
}
#[cfg(unix)]
pub use platform::PrivateDirectory;

#[cfg(not(unix))]
pub struct PrivateDirectory;
#[cfg(not(unix))]
impl PrivateDirectory {
    pub fn open(_: &Path, _: &[&str]) -> Result<Self, ManagedFailure> {
        Err(ManagedFailure::UnsupportedPlatform)
    }
    pub fn open_existing(_: &Path, _: &[&str]) -> Result<Self, ManagedFailure> {
        Err(ManagedFailure::UnsupportedPlatform)
    }
    pub fn read_optional(&self, _: &str, _: usize) -> Result<Option<Vec<u8>>, ManagedFailure> {
        Err(ManagedFailure::UnsupportedPlatform)
    }
    pub fn read_required(&self, _: &str, _: usize) -> Result<Vec<u8>, ManagedFailure> {
        Err(ManagedFailure::UnsupportedPlatform)
    }
    pub fn validate_optional(&self, _: &str) -> Result<(), ManagedFailure> {
        Err(ManagedFailure::UnsupportedPlatform)
    }
    pub fn publish(&self, _: &str, _: &[u8]) -> Result<(), ManagedFailure> {
        Err(ManagedFailure::UnsupportedPlatform)
    }
    pub fn remove_matching(
        &self,
        _: &str,
        _: usize,
        _: impl FnOnce(&[u8]) -> Result<bool, ManagedFailure>,
    ) -> Result<bool, ManagedFailure> {
        Err(ManagedFailure::UnsupportedPlatform)
    }
    pub fn append_rotating(&self, _: &[u8], _: u64) -> Result<(), ManagedFailure> {
        Err(ManagedFailure::UnsupportedPlatform)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::platform::{PrivateDirectory, PublicationStep};
    use crate::{observability::ManagedFailure, readiness::tests::temporary_home};
    #[test]
    fn injected_prepublication_failures_preserve_old_document_and_remove_only_owned_temp() {
        for step in [
            PublicationStep::Write,
            PublicationStep::FileSync,
            PublicationStep::Rename,
        ] {
            let home = temporary_home();
            let directory =
                PrivateDirectory::open(&home, &[".local", "share", "abbey-bot"]).unwrap();
            directory.publish("readiness.json", b"old").unwrap();
            assert!(
                directory
                    .publish_with("readiness.json", b"new", |at| if at == step {
                        Err(ManagedFailure::Write)
                    } else {
                        Ok(())
                    })
                    .is_err()
            );
            let root = home.join(".local/share/abbey-bot");
            assert_eq!(std::fs::read(root.join("readiness.json")).unwrap(), b"old");
            assert_eq!(std::fs::read_dir(root).unwrap().count(), 1);
            std::fs::remove_dir_all(home).unwrap();
        }
    }
    #[test]
    fn directory_sync_failure_is_failure_even_after_new_bytes_are_visible() {
        let home = temporary_home();
        let directory = PrivateDirectory::open(&home, &[".local", "share", "abbey-bot"]).unwrap();
        directory.publish("readiness.json", b"old").unwrap();
        assert!(
            directory
                .publish_with("readiness.json", b"new", |at| {
                    if at == PublicationStep::DirectorySync {
                        Err(ManagedFailure::Sync)
                    } else {
                        Ok(())
                    }
                })
                .is_err()
        );
        assert_eq!(
            std::fs::read(home.join(".local/share/abbey-bot/readiness.json")).unwrap(),
            b"new"
        );
        std::fs::remove_dir_all(home).unwrap();
    }
}

//! Cleanup ownership begins at exclusive creation, before any fallible I/O.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

pub(super) struct OwnedFile(Option<PathBuf>);
impl OwnedFile {
    pub(super) fn create(path: &Path) -> io::Result<(File, Self)> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path)?;
        Ok((file, Self(Some(path.to_owned()))))
    }
    pub(super) fn write_all(&self, file: &mut File, bytes: &[u8]) -> io::Result<()> {
        #[cfg(test)]
        inject(self.0.as_deref(), Fault::Write)?;
        file.write_all(bytes)
    }
    pub(super) fn sync_all(&self, file: &File) -> io::Result<()> {
        #[cfg(test)]
        inject(self.0.as_deref(), Fault::Sync)?;
        file.sync_all()
    }
    pub(super) fn sync_directory(path: &Path) -> io::Result<()> {
        #[cfg(test)]
        inject(Some(path), Fault::DirectorySync)?;
        File::open(path)?.sync_all()
    }
    pub(super) fn read_back(path: &Path) -> io::Result<Vec<u8>> {
        #[cfg(test)]
        inject(Some(path), Fault::Readback)?;
        fs::read(path)
    }
    pub(super) fn published(&mut self) {
        self.0 = None;
    }
}
impl Drop for OwnedFile {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Fault {
    Write,
    Sync,
    DirectorySync,
    Readback,
}
#[cfg(test)]
thread_local! {
    static FAULT: std::cell::RefCell<Option<(PathBuf, Fault)>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
fn inject(path: Option<&Path>, phase: Fault) -> io::Result<()> {
    FAULT.with(|fault| {
        if fault
            .borrow()
            .as_ref()
            .is_some_and(|(target, kind)| Some(target.as_path()) == path && *kind == phase)
        {
            return Err(io::Error::other("injected owned-file I/O failure"));
        }
        Ok(())
    })
}
#[cfg(test)]
pub(super) fn with_fault<T>(path: &Path, phase: Fault, work: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            FAULT.with(|f| *f.borrow_mut() = None);
        }
    }
    FAULT.with(|f| {
        assert!(f.borrow().is_none());
        *f.borrow_mut() = Some((path.to_owned(), phase));
    });
    let _reset = Reset;
    work()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publication_retires_cleanup_before_another_invocation_creates_the_path() {
        let dir = std::env::temp_dir().join(format!("abbey-owned-publish-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pending");
        let (_file, mut owner) = OwnedFile::create(&path).unwrap();
        fs::rename(&path, dir.join("published")).unwrap();
        owner.published();
        fs::write(&path, b"next invocation").unwrap();
        drop(owner);
        assert_eq!(fs::read(&path).unwrap(), b"next invocation");
        fs::remove_dir_all(dir).unwrap();
    }
}

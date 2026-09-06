use super::*;
#[cfg(unix)]
mod unix {
    use super::*;
    use crate::observability::{EventCode, EventComponent, EventOutcome};
    use std::{
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt, symlink},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    fn home() -> std::path::PathBuf {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).unwrap();
        let home = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("abbey-log-{}", crate::readiness::hex(&bytes)));
        fs::create_dir(&home).unwrap();
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
        home
    }
    fn event() -> OperationalEvent {
        OperationalEvent::new(
            42,
            EventComponent::Process,
            EventCode::Starting,
            EventOutcome::Started,
        )
        .unwrap()
    }
    #[test]
    fn rotates_before_write_retains_five_archives_and_never_touches_legacy() {
        let home = home();
        let writer = ManagedLog::open(&home, Arc::new(|_| panic!("unexpected failure"))).unwrap();
        let dir = home.join("Library/Logs/abbey-bot");
        let active = dir.join("abbey-bot.events.jsonl");
        fs::write(dir.join("abbey-bot.log"), b"PRIVATE_LEGACY").unwrap();
        for index in 1..=7 {
            fs::OpenOptions::new()
                .write(true)
                .open(&active)
                .unwrap()
                .set_len(ROTATION_BYTES)
                .unwrap();
            writer.write(&event().with_count(index)).unwrap();
        }
        assert!(dir.join("abbey-bot.events.jsonl.5").is_file());
        assert!(!dir.join("abbey-bot.events.jsonl.6").exists());
        assert_eq!(
            fs::metadata(&active).unwrap().len(),
            event().with_count(7).encode().unwrap().len() as u64
        );
        assert_eq!(fs::metadata(&active).unwrap().mode() & 0o7777, 0o600);
        assert_eq!(
            fs::read(dir.join("abbey-bot.log")).unwrap(),
            b"PRIVATE_LEGACY"
        );
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn concurrent_lines_are_complete_and_archive_symlink_is_fatal_before_append() {
        let home = home();
        let failures = Arc::new(AtomicUsize::new(0));
        let hook = failures.clone();
        let writer = Arc::new(
            ManagedLog::open(
                &home,
                Arc::new(move |_| {
                    hook.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap(),
        );
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let w = writer.clone();
                scope.spawn(move || {
                    for _ in 0..20 {
                        w.write(&event()).unwrap();
                    }
                });
            }
        });
        let dir = home.join("Library/Logs/abbey-bot");
        let active = dir.join("abbey-bot.events.jsonl");
        let before = fs::read(&active).unwrap();
        assert_eq!(
            before
                .split(|b| *b == b'\n')
                .filter(|line| !line.is_empty())
                .count(),
            80
        );
        for line in before
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
        {
            assert!(serde_json::from_slice::<serde_json::Value>(line).is_ok());
        }
        symlink(&active, dir.join("abbey-bot.events.jsonl.5")).unwrap();
        assert!(writer.write(&event()).is_err());
        assert_eq!(failures.load(Ordering::SeqCst), 1);
        assert_eq!(fs::read(&active).unwrap(), before);
        fs::remove_file(dir.join("abbey-bot.events.jsonl.5")).unwrap();
        assert!(writer.write(&event()).is_err()); // channel stays failed after invariant loss
        fs::remove_dir_all(home).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn every_archive_is_validated_before_any_log_mutation() {
    use std::{fs, os::unix::fs::PermissionsExt, sync::Arc};
    for archive in 1..=5 {
        let home = crate::readiness::tests::temporary_home();
        let log = ManagedLog::open(&home, Arc::new(|_| {})).unwrap();
        let directory = home.join("Library/Logs/abbey-bot");
        let path = directory.join(format!("abbey-bot.events.jsonl.{archive}"));
        fs::write(&path, b"PRIVATE_ARCHIVE").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let active = directory.join("abbey-bot.events.jsonl");
        let before = fs::read(&active).unwrap();
        let event = OperationalEvent::new(
            42,
            crate::observability::EventComponent::Process,
            crate::observability::EventCode::Starting,
            crate::observability::EventOutcome::Started,
        )
        .unwrap();
        assert!(log.write(&event).is_err());
        assert_eq!(fs::read(active).unwrap(), before);
        assert_eq!(fs::read(path).unwrap(), b"PRIVATE_ARCHIVE");
        fs::remove_dir_all(home).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn exact_limit_does_not_rotate_but_next_line_does() {
    use std::{fs, sync::Arc};
    let home = crate::readiness::tests::temporary_home();
    let log = ManagedLog::open(&home, Arc::new(|_| panic!("unexpected failure"))).unwrap();
    let event = OperationalEvent::new(
        42,
        crate::observability::EventComponent::Process,
        crate::observability::EventCode::Starting,
        crate::observability::EventOutcome::Started,
    )
    .unwrap();
    let directory = home.join("Library/Logs/abbey-bot");
    let active = directory.join("abbey-bot.events.jsonl");
    fs::OpenOptions::new()
        .write(true)
        .open(&active)
        .unwrap()
        .set_len(ROTATION_BYTES - event.encode().unwrap().len() as u64)
        .unwrap();
    log.write(&event).unwrap();
    assert_eq!(fs::metadata(&active).unwrap().len(), ROTATION_BYTES);
    assert!(!directory.join("abbey-bot.events.jsonl.1").exists());
    log.write(&event).unwrap();
    assert_eq!(
        fs::metadata(directory.join("abbey-bot.events.jsonl.1"))
            .unwrap()
            .len(),
        ROTATION_BYTES
    );
    assert_eq!(
        fs::metadata(active).unwrap().len(),
        event.encode().unwrap().len() as u64
    );
    fs::remove_dir_all(home).unwrap();
}

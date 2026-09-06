use super::*;
use crate::{
    inspect::VoiceInspectRegistry,
    voice::{VoiceBackendConfig, VoiceConfig, VoiceMode},
    voice_consent::{Choice, Ledger},
    voice_session::VoiceRuntime,
};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "abbey-voice-registry-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn template() -> crate::voice::VoiceTemplate {
    VoiceConfig::selected_only(1, 10, VoiceBackendConfig::Disabled, true).template()
}

fn configured(data_dir: Option<PathBuf>) -> Arc<VoiceRegistry> {
    let registry = Arc::new(VoiceRegistry::default());
    registry
        .configure(
            template(),
            None,
            data_dir,
            Arc::new(VoiceInspectRegistry::default()),
        )
        .unwrap();
    registry
}

#[tokio::test]
async fn concurrent_creation_publishes_one_runtime_for_the_guild() {
    let registry = configured(None);
    let reservation = registry.reserve_join(2).unwrap();
    let (first, second) = tokio::join!(
        registry.get_or_create(2, 20, &reservation),
        registry.get_or_create(2, 20, &reservation)
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert!(Arc::ptr_eq(&first, &registry.get(2).unwrap()));
    assert_eq!(reservation.operation_token(), Some(0));
}

#[tokio::test]
async fn cancellation_before_publication_invalidates_only_that_guild() {
    let registry = configured(None);
    let cancelled = registry.reserve_join(2).unwrap();
    let other = registry.reserve_join(3).unwrap();
    assert!(registry.cancel_join(2));
    assert!(registry.get_or_create(2, 20, &cancelled).await.is_err());
    assert!(registry.get(2).is_none());
    assert!(registry.get_or_create(3, 30, &other).await.is_ok());
}

#[test]
fn a_second_unbound_preflight_cannot_replace_the_first() {
    let registry = configured(None);
    let first = registry.reserve_join(2).unwrap();
    assert!(registry.reserve_join(2).is_err());
    assert_eq!(first.operation_token(), None);
}

#[tokio::test]
async fn different_guilds_never_reuse_the_home_consent_ledger() {
    let root = Scratch::new();
    let mut ledger = Ledger::new(1);
    assert!(ledger.apply(99, 100, Choice::Agree(VoiceMode::Local), 1));
    fs::write(
        root.0.join("voice-consent.json"),
        serde_json::to_vec(&ledger).unwrap(),
    )
    .unwrap();

    let config = template().for_destination(1, 10).unwrap();
    let home = Arc::new(VoiceRuntime::new_with_inspect(
        config,
        Arc::new(VoiceInspectRegistry::default()),
        Arc::new(crate::voice_consent_store::ConsentStore::load(
            Some(&root.0),
            1,
        )),
    ));
    let registry = Arc::new(VoiceRegistry::default());
    registry
        .configure(
            template(),
            Some(Arc::clone(&home)),
            Some(root.0.clone()),
            Arc::new(VoiceInspectRegistry::default()),
        )
        .unwrap();

    let reservation = registry.reserve_join(2).unwrap();
    let other = registry.get_or_create(2, 20, &reservation).await.unwrap();
    assert!(home.consent.agrees(99, VoiceMode::Local));
    assert!(!other.consent.agrees(99, VoiceMode::Local));
    assert!(root.0.join("voice-guilds/2").is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(root.0.join("voice-guilds/2"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}

#[tokio::test]
async fn draining_closes_creation_and_marks_every_published_runtime() {
    let registry = configured(None);
    let reservation = registry.reserve_join(2).unwrap();
    let runtime = registry.get_or_create(2, 20, &reservation).await.unwrap();
    let pending = registry.reserve_join(3).unwrap();
    let drained = registry.begin_draining();
    assert_eq!(drained.len(), 1);
    assert!(Arc::ptr_eq(&runtime, &drained[0]));
    assert!(!runtime.accepting_work());
    assert!(registry.get_or_create(3, 30, &pending).await.is_err());
    assert!(registry.reserve_join(4).is_err());
}

#[tokio::test]
async fn capacity_is_bounded_across_published_and_pending_guilds() {
    let registry = configured(None);
    for guild in 1..=MAX_VOICE_RUNTIMES as u64 {
        let reservation = registry.reserve_join(guild).unwrap();
        registry
            .get_or_create(guild, guild + 1000, &reservation)
            .await
            .unwrap();
    }
    assert!(
        registry
            .reserve_join(MAX_VOICE_RUNTIMES as u64 + 1)
            .is_err()
    );
}

#[tokio::test]
async fn unsafe_consent_directory_fails_without_publishing() {
    let root = Scratch::new();
    fs::write(root.0.join("voice-guilds"), b"not a directory").unwrap();
    let registry = configured(Some(root.0.clone()));
    let reservation = registry.reserve_join(2).unwrap();
    assert!(registry.get_or_create(2, 20, &reservation).await.is_err());
    assert!(registry.get(2).is_none());
}

#[tokio::test]
async fn retirement_requires_the_exact_published_arc() {
    let registry = configured(None);
    let reservation = registry.reserve_join(2).unwrap();
    let runtime = registry.get_or_create(2, 20, &reservation).await.unwrap();
    let impostor = Arc::new(VoiceRuntime::new(
        template().for_destination(2, 20).unwrap(),
    ));
    assert!(!registry.retire(2, &impostor));
    assert!(registry.get(2).is_some());
    assert!(registry.retire(2, &runtime));
    assert!(registry.get(2).is_none());
    assert!(!runtime.accepting_work());

    let replacement = registry.reserve_join(2).unwrap();
    let replacement = registry.get_or_create(2, 21, &replacement).await.unwrap();
    assert!(Arc::ptr_eq(&runtime.consent, &replacement.consent));
}

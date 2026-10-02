use super::*;
use crate::community_ops::proposals::tests::fixture;
use crate::persist::community_ops::load_policy;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fake {
    local: bool,
    qualified: bool,
    text: &'static str,
    calls: AtomicUsize,
}
impl LocalGenerator for Fake {
    fn ready(&self) -> bool {
        self.local && self.qualified
    }
    async fn generate(&self, _: &str) -> Result<String, &'static str> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(self.text.into())
    }
}
struct PrivatePolicy(std::path::PathBuf);
impl Drop for PrivatePolicy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn saved(policy: &Policy) -> PrivatePolicy {
    let path = std::env::temp_dir().join(format!(
        "abbey-assessment-policy-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options.open(&path).unwrap();
    file.write_all(&serde_json::to_vec(policy).unwrap())
        .unwrap();
    PrivatePolicy(path)
}
#[tokio::test]
async fn offhost_or_unqualified_generator_never_receives_metadata() {
    let (policy, _, _, _) = fixture();
    for (local, qualified) in [(false, true), (true, false), (false, false)] {
        let fake = Fake {
            local,
            qualified,
            text: r#"{"version":1,"proposals":[]}"#,
            calls: AtomicUsize::new(0),
        };
        assert_eq!(
            generate(
                &fake,
                "public metadata",
                &CancellationToken::new(),
                &policy,
                "digest",
                || std::future::ready(load_policy(Path::new("/unavailable")))
            )
            .await,
            Err(AssessmentOutcome::LocalUnavailable)
        );
        assert_eq!(fake.calls.load(Ordering::Relaxed), 0);
    }
}
#[tokio::test]
async fn policy_stop_cancellation_and_invalid_json_cannot_publish_drafts() {
    let (policy, _, _, _) = fixture();
    let file = saved(&policy);
    let digest = load_policy(&file.0).unwrap().1;
    let fake = Fake {
        local: true,
        qualified: true,
        text: "not JSON",
        calls: AtomicUsize::new(0),
    };
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        generate(&fake, "public metadata", &cancel, &policy, &digest, || {
            std::future::ready(load_policy(&file.0))
        })
        .await,
        Err(AssessmentOutcome::Cancelled)
    );
    assert_eq!(fake.calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        generate(
            &fake,
            "public metadata",
            &CancellationToken::new(),
            &policy,
            &digest,
            || std::future::ready(load_policy(&file.0))
        )
        .await,
        Err(AssessmentOutcome::InvalidOutput)
    );
    assert_eq!(fake.calls.load(Ordering::Relaxed), 1);
    let mut stopped = policy.clone();
    stopped.mode = Mode::Stopped;
    let stopped_file = saved(&stopped);
    assert_eq!(
        generate(
            &fake,
            "public metadata",
            &CancellationToken::new(),
            &policy,
            &digest,
            || std::future::ready(load_policy(&stopped_file.0))
        )
        .await,
        Err(AssessmentOutcome::Cancelled)
    );
    assert_eq!(fake.calls.load(Ordering::Relaxed), 1);
}
#[test]
fn source_excludes_unselected_metadata_and_fails_on_incomplete_or_oversized_selection() {
    let (policy, _, _, mut proofs) = fixture();
    let category = ChannelProof {
        metadata: PublicChannelMetadata {
            id: 3,
            name: "Commons".into(),
            kind: "category".into(),
            parent: 0,
            topic: None,
        },
        public: false,
        archive_safe: false,
        permissions_digest: "category".into(),
        before_digest: "category".into(),
    };
    proofs.channels.insert(3, category);
    let mut private = proofs.channels[&4].clone();
    private.metadata.id = 9;
    private.metadata.topic = Some("private-secret-marker".into());
    private.public = false;
    proofs.channels.insert(9, private);
    let selected = source(&policy, "digest", &proofs, "assessment").unwrap();
    assert!(
        !prompt(&selected, &policy.assessment)
            .unwrap()
            .contains("private-secret-marker")
    );
    proofs.channels.get_mut(&4).unwrap().public = false;
    assert!(source(&policy, "digest", &proofs, "assessment").is_err());
    proofs.channels.get_mut(&4).unwrap().public = true;
    proofs.channels.get_mut(&4).unwrap().metadata.topic = Some("x".repeat(MAX_SOURCE_BYTES + 1));
    assert!(source(&policy, "digest", &proofs, "assessment").is_err());
}
#[tokio::test]
async fn cancellation_at_generation_completion_discards_even_valid_json() {
    struct CancelOnGenerate(CancellationToken, AtomicUsize);
    impl LocalGenerator for CancelOnGenerate {
        fn ready(&self) -> bool {
            true
        }
        async fn generate(&self, _: &str) -> Result<String, &'static str> {
            self.1.fetch_add(1, Ordering::Relaxed);
            self.0.cancel();
            Ok(r#"{"version":1,"proposals":[]}"#.into())
        }
    }
    let (policy, _, _, _) = fixture();
    let file = saved(&policy);
    let digest = load_policy(&file.0).unwrap().1;
    let fake = CancelOnGenerate(CancellationToken::new(), AtomicUsize::new(0));
    assert_eq!(
        generate(&fake, "public metadata", &fake.0, &policy, &digest, || {
            std::future::ready(load_policy(&file.0))
        })
        .await,
        Err(AssessmentOutcome::Cancelled)
    );
    assert_eq!(fake.1.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn blocked_initial_policy_proof_times_out_before_generation_dispatch() {
    let (policy, _, _, _) = fixture();
    let fake = Fake {
        local: true,
        qualified: true,
        text: r#"{"version":1,"proposals":[]}"#,
        calls: AtomicUsize::new(0),
    };
    let result = tokio::time::timeout(
        Duration::from_millis(750),
        generate(
            &fake,
            "public metadata",
            &CancellationToken::new(),
            &policy,
            "digest",
            || std::future::pending::<Result<(Policy, String), &'static str>>(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(result, Err(AssessmentOutcome::Cancelled));
    assert_eq!(fake.calls.load(Ordering::Relaxed), 0);
}

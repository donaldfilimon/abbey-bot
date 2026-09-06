use super::*;
use std::cell::Cell;

#[tokio::test]
async fn acknowledgement_and_authorization_gate_every_snapshot() {
    let reads = Cell::new(0);
    let permissions = Cell::new(0);
    let result = prepare(
        async { Err("acknowledgement unavailable".into()) },
        || panic!("envelope must wait for acknowledgement"),
        || async {
            permissions.set(permissions.get() + 1);
            Ok(Vec::new())
        },
        |_| {
            reads.set(reads.get() + 1);
            Vec::new()
        },
    )
    .await;
    assert!(result.is_err());
    assert_eq!(permissions.get(), 0);
    assert_eq!(reads.get(), 0);
    let session = MemorySession::new(1, 2, MemoryScope::Guild(3), 10).unwrap();
    let result = prepare(
        async { Ok(()) },
        || Ok(session),
        || async { Ok(Vec::new()) },
        |_| {
            reads.set(reads.get() + 1);
            Vec::new()
        },
    )
    .await
    .unwrap();
    assert!(matches!(result, Preparation::Rejected(_)));
    assert_eq!(reads.get(), 0);
}

#[tokio::test]
async fn permitted_pages_read_fresh_snapshots_without_extending_expiry() {
    let reads = Cell::new(0);
    let session = MemorySession::new(1, 2, MemoryScope::Guild(3), 10).unwrap();
    for page in [0, 1] {
        let result = prepare(
            async { Ok(()) },
            || Ok(session.navigate(page).unwrap()),
            || async { Ok(vec![DiscordPermission::ManageMessages]) },
            |_| {
                reads.set(reads.get() + 1);
                vec![format!("fresh snapshot {}", reads.get())]
            },
        )
        .await
        .unwrap();
        let Preparation::Ready(observed, facts) = result else {
            panic!("authorized snapshot")
        };
        assert_eq!(observed.expiry, session.expiry);
        assert_eq!(facts, [format!("fresh snapshot {}", reads.get())]);
    }
    assert_eq!(reads.get(), 2);
}

#[tokio::test]
async fn rejected_envelopes_and_failed_permission_refresh_never_read_snapshots() {
    let reads = Cell::new(0);
    let lookups = Cell::new(0);
    for rejection in [
        BrowserRejection::Stale,
        BrowserRejection::NotOwner,
        BrowserRejection::WrongScope,
        BrowserRejection::Expired,
    ] {
        let result = prepare(
            async { Ok(()) },
            || Err(rejection),
            || async {
                lookups.set(lookups.get() + 1);
                Ok(Vec::new())
            },
            |_| {
                reads.set(reads.get() + 1);
                Vec::new()
            },
        )
        .await
        .unwrap();
        assert!(matches!(result, Preparation::Rejected(_)));
    }
    assert_eq!(lookups.get(), 0);
    let session = MemorySession::new(1, 1, MemoryScope::Guild(3), 10).unwrap();
    let result = prepare(
        async { Ok(()) },
        || Ok(session),
        || async { Err("permission unavailable".into()) },
        |_| {
            reads.set(reads.get() + 1);
            Vec::new()
        },
    )
    .await
    .unwrap();
    assert!(matches!(result, Preparation::Rejected(_)));
    assert_eq!(reads.get(), 0);
}

#[tokio::test]
async fn permission_refresh_crossing_original_expiry_reads_no_facts() {
    let now = Cell::new(10);
    let reads = Cell::new(0);
    let scope = MemoryScope::Guild(3);
    let session = MemorySession::new(1, 1, scope, now.get()).unwrap();
    let custom_id = session.custom_id();
    let result = prepare(
        async { Ok(()) },
        || browser::validate(&custom_id, 1, &scope, now.get()),
        || async {
            now.set(session.expiry);
            Ok(Vec::new())
        },
        |_| {
            reads.set(reads.get() + 1);
            Vec::new()
        },
    )
    .await
    .unwrap();
    assert!(matches!(result, Preparation::Rejected(_)));
    assert_eq!(reads.get(), 0);
}

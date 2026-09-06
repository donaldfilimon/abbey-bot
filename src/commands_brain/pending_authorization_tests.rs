use super::memory_commands::{PendingButtonAction, authorized_pending_effect};
use super::*;
#[tokio::test]
async fn acknowledgement_failure_and_permission_revocation_construct_no_gate_or_snapshot() {
    use std::cell::Cell;
    let session = PendingComponentSession {
        command_id: 1,
        owner: 2,
        subject: 3,
        guild: Some(4),
        channel: 5,
        version: 0,
        displayed: Vec::new(),
    };
    let failed: Result<Option<()>, Error> = authorized_pending_effect(
        async { Err("synthetic acknowledgement failure".into()) },
        &session,
        || panic!("validation preceded acknowledgement"),
        || async { panic!("permissions preceded acknowledgement") },
        |_, _| async { panic!("gate preceded acknowledgement") },
    )
    .await;
    assert!(failed.is_err());
    let snapshots_and_gates = Cell::new(0);
    let denied = authorized_pending_effect(
        async { Ok(()) },
        &session,
        || Some((PendingButtonAction::Confirm, 0)),
        || async { Ok(Vec::new()) },
        |_, _| {
            snapshots_and_gates.set(snapshots_and_gates.get() + 1);
            async {}
        },
    )
    .await
    .unwrap();
    assert!(denied.is_none());
    assert_eq!(snapshots_and_gates.get(), 0);
    let order = std::cell::RefCell::new(Vec::new());
    authorized_pending_effect(
        async {
            order.borrow_mut().push("ack");
            Ok(())
        },
        &session,
        || {
            order.borrow_mut().push("envelope");
            Some((PendingButtonAction::Confirm, 0))
        },
        || {
            order.borrow_mut().push("permissions constructed");
            async {
                Ok(vec![
                    crate::command_catalog::DiscordPermission::ManageMessages,
                ])
            }
        },
        |_, _| {
            order.borrow_mut().push("snapshot and gate constructed");
            async {}
        },
    )
    .await
    .unwrap();
    assert_eq!(
        *order.borrow(),
        [
            "ack",
            "envelope",
            "permissions constructed",
            "snapshot and gate constructed"
        ]
    );
}

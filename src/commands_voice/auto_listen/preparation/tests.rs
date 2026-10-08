use super::*;
use crate::service::{ServiceSupervisor, TaskExit};
use crate::voice::VoiceConfig;
use std::sync::Arc;
use tokio::sync::{Semaphore, oneshot};

async fn present() -> Arc<VoiceRuntime> {
    let mut runtime = VoiceRuntime::new(VoiceConfig::selected_only(
        1,
        2,
        VoiceBackendConfig::Disabled,
        true,
    ));
    runtime.consent = Arc::new(
        crate::voice_consent_store::ConsentStore::acknowledged_fixture(
            1,
            &[10],
            crate::voice::VoiceMode::Local,
        ),
    );
    let runtime = Arc::new(runtime);
    runtime
        .set_presence_with_discord_session("old".into(), "presence")
        .await;
    runtime
}

enum Stop {
    Leave,
    Withdrawal,
    Draining,
}

async fn interrupted_lookup(second: bool, replacement: bool, stop: Stop) {
    let runtime = present().await;
    let operation = runtime.start_operation_token();
    let entered = Arc::new(Semaphore::new(0));
    let release = Arc::new(Semaphore::new(0));
    let mut calls = 0;
    let work = prepare_presence_upgrade(
        &runtime,
        operation,
        || AutoListenDecision::Ready,
        || {
            calls += 1;
            let hold = calls == if second { 2 } else { 1 };
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            async move {
                if hold {
                    entered.add_permits(1);
                    release.acquire().await.unwrap().forget();
                }
                Ok(())
            }
        },
    );
    tokio::pin!(work);
    tokio::select! {
        biased;
        _ = &mut work => panic!("permission future must hold preflight"),
        permit = entered.acquire() => permit.unwrap().forget(),
    }
    match stop {
        Stop::Draining => runtime.begin_draining(),
        Stop::Leave => runtime.disconnect("leave").await,
        Stop::Withdrawal => {
            let change =
                runtime.change_consent(10, 2, crate::voice_consent::Choice::Withdraw, 2, true);
            assert!(change.epoch_to_stop.is_some());
            // No configured disk in this fixture; denial must still fence preflight.
            assert!(change.saved.await.unwrap().is_err());
        }
    }
    if replacement {
        runtime
            .set_presence_with_discord_session("new".into(), "new presence")
            .await;
    }
    let stopped = runtime.start_operation_token();
    release.add_permits(1);
    assert!(
        work.await.is_err(),
        "old preflight must not reserve a new start"
    );
    assert_eq!(
        runtime.start_operation_token(),
        stopped,
        "no start reservation after stop"
    );
    assert!(!runtime.media_enabled(runtime.current_epoch()));
}

#[tokio::test]
async fn auto_listen_old_lookup_cannot_attach_to_new_presence() {
    interrupted_lookup(false, true, Stop::Leave).await;
}

#[tokio::test]
async fn auto_listen_later_permission_cannot_restart_after_leave() {
    interrupted_lookup(true, false, Stop::Leave).await;
}

#[tokio::test]
async fn auto_listen_later_permission_cannot_restart_after_draining() {
    interrupted_lookup(true, false, Stop::Draining).await;
}

#[tokio::test]
async fn auto_listen_retained_delayed_first_poll_cannot_attach_to_new_presence() {
    let runtime = present().await;
    let mut supervisor = ServiceSupervisor::new();
    supervisor.finish_startup();
    runtime.attach_service(supervisor.operations());
    let operation = runtime.start_operation_token();
    let retained = Arc::clone(&runtime);
    let (send, result) = oneshot::channel();
    super::super::super::supervision::retain_presence_upgrade(
        &runtime,
        operation,
        move |operation| async move {
            let prepared = prepare_presence_upgrade(
                &retained,
                operation,
                || AutoListenDecision::Ready,
                || async { Ok(()) },
            )
            .await;
            send.send(prepared.is_err()).unwrap();
        },
    )
    .unwrap();
    // Current-thread runtime: admission has returned, but the owner cannot poll
    // until we yield. Both lifecycle writes complete without a contended await.
    runtime.disconnect("leave before worker first poll").await;
    runtime
        .set_presence_with_discord_session("new".into(), "new presence")
        .await;
    let stopped = runtime.start_operation_token();
    assert!(
        result.await.unwrap(),
        "retained owner must carry its admitted lifecycle"
    );
    assert_eq!(supervisor.next_completion().await.exit, TaskExit::Returned);
    assert_eq!(runtime.start_operation_token(), stopped);
}

#[tokio::test]
async fn auto_listen_later_permission_cannot_restart_after_withdrawal() {
    interrupted_lookup(true, false, Stop::Withdrawal).await;
}

#[tokio::test]
async fn auto_listen_current_presence_reserves_only_after_both_permission_checks() {
    let runtime = present().await;
    let operation = runtime.start_operation_token();
    let mut calls = 0;
    let prepared = prepare_presence_upgrade(
        &runtime,
        operation,
        || AutoListenDecision::Ready,
        || {
            assert_eq!(runtime.start_operation_token(), operation);
            calls += 1;
            async { Ok(()) }
        },
    )
    .await
    .unwrap();
    assert_eq!(calls, 2);
    let PresencePreparation::Reserved(reservation) = prepared else {
        panic!("eligible presence")
    };
    let (generation, _) = *reservation;
    assert!(runtime.start_is_current(generation));
    runtime.finish_start_attempt(generation);
}

#[tokio::test]
async fn auto_listen_startup_can_reserve_from_disconnected_but_not_after_denial() {
    let runtime = present().await;
    runtime.disconnect("startup fixture").await;
    let operation = runtime.start_operation_token();
    assert!(
        reserve_auto_listen(&runtime, operation, async {
            Err("permission denied".into())
        })
        .await
        .is_err()
    );
    assert_eq!(runtime.start_operation_token(), operation);
    let (generation, _) = reserve_auto_listen(&runtime, operation, async { Ok(()) })
        .await
        .unwrap();
    assert!(runtime.start_is_current(generation));
    runtime.finish_start_attempt(generation);
}

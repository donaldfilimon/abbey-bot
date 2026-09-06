use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn running() -> ServiceSupervisor {
    let mut supervisor = ServiceSupervisor::new();
    supervisor.finish_startup();
    supervisor
}

#[test]
fn first_trigger_owns_the_deadline_including_work_before_stages() {
    let mut supervisor = running();
    let now = Instant::now();
    let first = supervisor.begin_draining(ShutdownReason::Signal, now);
    let second = supervisor.begin_draining(
        ShutdownReason::DiscordClientFailed,
        now + Duration::from_secs(9),
    );
    assert!(first.first_trigger);
    assert!(!second.first_trigger);
    assert_eq!(second.reason, ShutdownReason::Signal);
    assert_eq!(second.budget.started(), now);
    assert_eq!(second.budget.deadline(), now + Duration::from_secs(20));
    assert_eq!(
        first.budget.remaining(now + Duration::from_secs(19)),
        Duration::from_secs(1)
    );
    assert_eq!(
        first.budget.elapsed(now + Duration::from_secs(19)),
        Duration::from_secs(19)
    );
    let early = first.budget.stage(now + Duration::from_secs(2));
    assert_eq!(early.deadline, now + Duration::from_secs(7));
    assert_eq!(early.abort_at, now + Duration::from_secs(6));
    let late = first.budget.stage(now + Duration::from_secs(19));
    assert_eq!(late.deadline, first.budget.deadline());
    assert!(late.abort_at < late.deadline);
    assert!(late.abort_at > now + Duration::from_secs(19));
}

#[tokio::test]
async fn startup_and_writer_ownership_prevent_false_frozen_state() {
    let mut supervisor = ServiceSupervisor::new();
    let operations = supervisor.operations();
    assert!(matches!(
        operations.spawn_operation(OperationKind::FrameworkDispatch, |_| async {
            TaskExit::Returned
        }),
        Err(AdmissionError::NotRunning)
    ));
    supervisor.begin_draining(ShutdownReason::Signal, Instant::now());
    assert_eq!(
        supervisor.try_freeze(true),
        Err(FreezeError::StartupStillOwned)
    );
    supervisor.finish_startup();
    assert_eq!(supervisor.phase(), ServicePhase::Draining);
    assert_eq!(
        supervisor.try_freeze(false),
        Err(FreezeError::WriterNotIdle)
    );
    assert_eq!(supervisor.try_freeze(true), Ok(()));
    assert_eq!(supervisor.phase(), ServicePhase::Frozen);
    assert!(matches!(
        operations.spawn_operation(OperationKind::Episode, |_| async { TaskExit::Returned }),
        Err(AdmissionError::NotRunning)
    ));
}

#[tokio::test]
async fn every_named_service_has_fatal_unexpected_exit_and_duplicate_is_rejected() {
    for name in [TaskName::Scheduler, TaskName::Telegram, TaskName::Slack] {
        let mut supervisor = running();
        let (release, waiting) = oneshot::channel();
        let receipt = supervisor
            .spawn_service(name, |_| async move {
                let _ = waiting.await;
                TaskExit::Returned
            })
            .unwrap();
        assert!(matches!(
            supervisor.spawn_service(name, |_| async { TaskExit::Returned }),
            Err(AdmissionError::DuplicateService)
        ));
        release.send(()).unwrap();
        let completion = supervisor.next_completion().await;
        assert_eq!(
            completion.fatal,
            Some(ShutdownReason::SupervisedTaskFailed(name))
        );
        assert_eq!(completion.exit, TaskExit::Returned);
        assert_eq!(receipt.joined().await.unwrap(), completion);
        assert!(supervisor.outstanding().is_empty());
    }
}

#[tokio::test]
async fn root_observer_is_woken_by_admission_and_panic_is_categorized_without_payload() {
    let mut supervisor = running();
    let operations = supervisor.operations();
    let observer = supervisor.next_completion();
    tokio::pin!(observer);
    assert!(futures_util::poll!(&mut observer).is_pending());
    let receipt = operations
        .spawn_operation(OperationKind::FrameworkDispatch, |_| async {
            panic!("synthetic task panic");
        })
        .unwrap();
    let completion = observer.await;
    assert_eq!(completion.exit, TaskExit::Panicked);
    assert_eq!(
        completion.fatal,
        Some(ShutdownReason::OperationalInvariantFailed)
    );
    assert!(!format!("{completion:?}").contains("synthetic"));
    assert_eq!(receipt.joined().await.unwrap(), completion);
}

#[tokio::test]
async fn dropping_receipt_and_reap_waiter_never_detaches_accepted_mutation() {
    let mut supervisor = running();
    let operations = supervisor.operations();
    let (release, waiting) = oneshot::channel();
    let mutated = Arc::new(AtomicUsize::new(0));
    let changed = mutated.clone();
    let receipt = operations
        .spawn_operation(OperationKind::FrameworkDispatch, |_| async move {
            let _ = waiting.await;
            changed.fetch_add(1, Ordering::SeqCst);
            TaskExit::Returned
        })
        .unwrap();
    let id = receipt.id;
    drop(receipt);
    let shutdown = supervisor.begin_draining(ShutdownReason::Signal, Instant::now());
    {
        let reap = supervisor.cancel_and_reap(shutdown.budget.stage(Instant::now()));
        tokio::pin!(reap);
        assert!(futures_util::poll!(&mut reap).is_pending());
    }
    assert_eq!(supervisor.outstanding()[0].id, id);
    assert_eq!(
        supervisor.try_freeze(true),
        Err(FreezeError::TasksNotJoined)
    );
    release.send(()).unwrap();
    let report = supervisor
        .cancel_and_reap(shutdown.budget.stage(Instant::now()))
        .await;
    assert_eq!(report.outcome, ReapOutcome::Joined);
    assert_eq!(report.joined.len(), 1);
    assert_eq!(mutated.load(Ordering::SeqCst), 1);
    assert_eq!(supervisor.try_freeze(true), Ok(()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admission_racing_draining_is_either_owned_or_never_executed() {
    let mut supervisor = running();
    let operations = supervisor.operations();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let other = barrier.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let called = calls.clone();
    let submit = tokio::spawn(async move {
        other.wait().await;
        operations.spawn_operation(OperationKind::FrameworkDispatch, move |_| async move {
            called.fetch_add(1, Ordering::SeqCst);
            TaskExit::Returned
        })
    });
    barrier.wait().await;
    let shutdown = supervisor.begin_draining(ShutdownReason::Signal, Instant::now());
    let submitted = submit.await.unwrap();
    match submitted {
        Ok(receipt) => {
            assert_eq!(supervisor.outstanding().len(), 1);
            let report = supervisor
                .cancel_and_reap(shutdown.budget.stage(Instant::now()))
                .await;
            assert_eq!(report.outcome, ReapOutcome::Joined);
            assert_eq!(report.joined[0].id, receipt.id);
            assert!(calls.load(Ordering::SeqCst) <= 1);
        }
        Err(error) => {
            assert_eq!(error, AdmissionError::NotRunning);
            assert!(supervisor.outstanding().is_empty());
            assert_eq!(calls.load(Ordering::SeqCst), 0);
        }
    }
    assert_eq!(supervisor.try_freeze(true), Ok(()));
}

#[tokio::test]
async fn abort_is_only_a_request_until_the_actual_handle_is_joined() {
    let mut supervisor = running();
    let receipt = supervisor
        .operations()
        .spawn_operation(OperationKind::FrameworkDispatch, |_| async {
            std::future::pending::<()>().await;
            TaskExit::Returned
        })
        .unwrap();
    supervisor.begin_draining(ShutdownReason::Signal, Instant::now());
    supervisor.request_abort();
    assert_eq!(supervisor.outstanding().len(), 1);
    assert!(supervisor.outstanding()[0].abort_requested);
    assert_eq!(
        supervisor.try_freeze(true),
        Err(FreezeError::TasksNotJoined)
    );
    let completion = supervisor.next_completion().await;
    assert_eq!(completion.exit, TaskExit::Cancelled);
    assert!(completion.abort_requested);
    assert_eq!(receipt.joined().await.unwrap(), completion);
    assert_eq!(supervisor.try_freeze(true), Ok(()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn started_blocking_work_stays_owned_after_abort_and_expired_reap() {
    let mut supervisor = running();
    let (entered, started) = oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let receipt = supervisor
        .operations()
        .spawn_blocking(OperationKind::ConsentPersistence, move |_| {
            let _ = entered.send(());
            // A bounded safety escape prevents a failed assertion from hanging a
            // test runtime forever; the normal path explicitly releases the task.
            let _ = blocked.recv_timeout(Duration::from_secs(5));
            TaskExit::Returned
        })
        .unwrap();
    started.await.unwrap();
    supervisor.begin_draining(ShutdownReason::Signal, Instant::now());
    supervisor.request_abort();
    let now = Instant::now();
    let report = supervisor
        .cancel_and_reap(StageBudget {
            deadline: now,
            abort_at: now,
        })
        .await;
    let retained = report.outcome == ReapOutcome::TimedOut
        && report.outstanding.len() == 1
        && supervisor.try_freeze(true) == Err(FreezeError::TasksNotJoined);
    release.send(()).unwrap();
    let completion = supervisor.next_completion().await;
    assert!(retained);
    assert_eq!(completion.exit, TaskExit::Returned);
    assert!(completion.abort_requested);
    assert_eq!(receipt.joined().await.unwrap(), completion);
    assert_eq!(supervisor.try_freeze(true), Ok(()));
}

#[tokio::test]
async fn service_cancellation_is_neutral_only_when_its_token_was_cancelled() {
    for requested in [false, true] {
        let mut supervisor = running();
        let receipt = supervisor
            .spawn_service(TaskName::Scheduler, move |token| async move {
                if requested {
                    token.cancelled().await;
                }
                TaskExit::Cancelled
            })
            .unwrap();
        if requested {
            supervisor.request_cancellation();
        }
        let completion = supervisor.next_completion().await;
        assert_eq!(completion.fatal.is_none(), requested);
        assert_eq!(receipt.joined().await.unwrap(), completion);
    }
}

#[tokio::test]
async fn actual_join_after_stage_allowance_is_truthful_but_timed_out() {
    let mut supervisor = running();
    supervisor
        .operations()
        .spawn_operation(OperationKind::FrameworkDispatch, |_| async {
            TaskExit::Returned
        })
        .unwrap();
    supervisor.begin_draining(ShutdownReason::Signal, Instant::now());
    let joined = supervisor.next_completion().await;
    assert_eq!(joined.exit, TaskExit::Returned);
    let past = Instant::now() - Duration::from_millis(1);
    let report = supervisor
        .cancel_and_reap(StageBudget {
            deadline: past,
            abort_at: past,
        })
        .await;
    assert_eq!(report.outcome, ReapOutcome::TimedOut);
    assert!(report.outstanding.is_empty());
    assert_eq!(report.joined, vec![joined]);
    assert_eq!(supervisor.try_freeze(true), Ok(()));
}

#[tokio::test]
async fn retained_child_cleanup_cannot_be_aborted_into_false_quiescence() {
    for kind in [OperationKind::Episode, OperationKind::ProviderProcess] {
        let mut supervisor = running();
        let (entered, started) = oneshot::channel();
        let (release, wait) = oneshot::channel();
        supervisor
            .operations()
            .spawn_operation(kind, move |cancel| async move {
                cancel.cancelled().await;
                let _ = entered.send(());
                // This is the observed child.wait boundary after kill was requested.
                let _ = wait.await;
                TaskExit::Cancelled
            })
            .unwrap();
        supervisor.begin_draining(ShutdownReason::Signal, Instant::now());
        supervisor.request_cancellation();
        started.await.unwrap();
        supervisor.request_abort();
        assert_eq!(supervisor.outstanding().len(), 1);
        assert!(!supervisor.outstanding()[0].abort_requested);
        assert_eq!(
            supervisor.try_freeze(true),
            Err(FreezeError::TasksNotJoined)
        );
        release.send(()).unwrap();
        supervisor.next_completion().await;
        assert_eq!(supervisor.try_freeze(true), Ok(()));
    }
}

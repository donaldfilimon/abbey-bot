//! Actual retained generation cancellation while current native access holds a lease.
use super::*;
use crate::{
    runtime::continuity_context::{ContinuityAccessProvider, ContinuityAudience},
    work::{
        WorkAccess, WorkContentRef, WorkError, WorkScope,
        continuity::{ContinuityDraft, ProposalRegistry},
    },
};
use std::{
    collections::BTreeSet,
    future::{Future, poll_fn},
    pin::Pin,
    task::Poll,
};

const ACCESS_BOUND: Duration = Duration::from_millis(500);
const JOIN_BOUND: Duration = Duration::from_secs(1);

fn owner_access() -> WorkAccess {
    WorkAccess {
        actor: 7,
        guild: None,
        channel: 70,
        can_view: true,
        can_manage: false,
    }
}

fn seed_owner_card(state: &AppState) {
    let access = owner_access();
    let now = crate::runtime::now();
    let mut stores = AppState::lock(&state.stores);
    let project = stores
        .work
        .create_project(
            WorkAccess {
                can_manage: true,
                ..access
            },
            "Allocator",
            "create",
        )
        .unwrap();
    let id = stores
        .work
        .record_decision(project, access, "Keep checks", now, "decision")
        .unwrap();
    let mut proposals = ProposalRegistry::new([1; 16]);
    let proposal = proposals
        .propose(
            ContinuityDraft {
                scope: access.scope(),
                base_revision: 0,
                presented_text: "Continue checked allocator".into(),
                source_refs: BTreeSet::from([WorkContentRef::Decision {
                    project,
                    id,
                    revision: 1,
                }]),
            },
            &access,
            &stores.work,
            now,
        )
        .unwrap();
    let grant = proposals
        .resolve_confirmation(proposal.id, access.actor, &access.scope(), now)
        .unwrap();
    let crate::persist::Stores {
        continuity, work, ..
    } = &mut *stores;
    continuity.confirm(grant, &access, work, now).unwrap();
}

#[derive(Default)]
struct HeldFreshAccess {
    calls: AtomicUsize,
    held: Notify,
    dropped: AtomicUsize,
    drop_observed: Notify,
    // Only a failing test's cleanup signals this; passing tests never release
    // the authorization read normally. Its Drop must follow cancellation.
    cleanup_release: Notify,
}
impl ContinuityAccessProvider for HeldFreshAccess {
    fn authorize<'a>(
        &'a self,
        scope: &'a WorkScope,
        actor: u64,
        channel: u64,
    ) -> Pin<Box<dyn Future<Output = Result<WorkAccess, WorkError>> + Send + 'a>> {
        Box::pin(async move {
            assert_eq!(*scope, owner_access().scope());
            assert_eq!((actor, channel), (7, 70));
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            // 1: mint the immutable caller context. 2: fresh check before
            // preparing the generation round. 3: retained check after reserve.
            if call <= 2 {
                return Ok(owner_access());
            }
            struct DropRead<'a>(&'a HeldFreshAccess);
            impl Drop for DropRead<'_> {
                fn drop(&mut self) {
                    self.0.dropped.fetch_add(1, Ordering::SeqCst);
                    self.0.drop_observed.notify_one();
                }
            }
            let _read = DropRead(self);
            self.held.notify_one();
            self.cleanup_release.notified().await;
            Err(WorkError::Denied)
        })
    }
}

#[derive(Default)]
struct InvisibleOut {
    visible: AtomicUsize,
}
impl Outbound for InvisibleOut {
    async fn send(&self, _: &str, _: &OutboundMessage) -> Result<String, OutboundFailure> {
        self.visible.fetch_add(1, Ordering::SeqCst);
        Ok("unexpected-visible-message".into())
    }
    async fn edit(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
        self.visible.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn react(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
        self.visible.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn typing(&self, _: &str) {}
    async fn fetch(&self, _: &str, _: usize) -> Result<Vec<u8>, String> {
        Ok(vec![])
    }
}

async fn cancel_while_native_access_holds_provider_lease(shutdown: bool) {
    let (state, adapter, mut supervisor, mut writer) = fixture(Mode::Held);
    seed_owner_card(&state);
    let access = Arc::new(HeldFreshAccess::default());
    state.attach_continuity_access(access.clone()).unwrap();
    let mut context = state.memory_service().context_for(
        "discord:dm:7",
        "discord:7",
        "discord:70",
        "Continue",
        0,
        0.5,
    );
    context.continuity = state
        .prepare_continuity_context(owner_access().scope(), 7, 70, ContinuityAudience::OwnerDm)
        .await;
    assert!(context.continuity.is_some());
    assert_eq!(access.calls.load(Ordering::SeqCst), 1);
    let out = Arc::new(InvisibleOut::default());
    let owned = state.clone();
    let delivery = out.clone();
    let mut caller = tokio::spawn(async move {
        let request = Ask {
            session_mode: SessionMode::SourceOnly,
            subject: Some(("discord:dm:7", "discord:7")),
            scope: "discord:70",
            context: &context,
            user_input: "Continue",
            now: crate::runtime::now(),
        };
        generate_read_only(
            &owned,
            Persona::Abbey,
            &request,
            Some(Delivery {
                out: &*delivery,
                native_channel_id: "70",
                reply_to: None,
            }),
        )
        .await
    });
    let entered = tokio::time::timeout(JOIN_BOUND, access.held.notified()).await;
    if entered.is_err() {
        // Preserve observed ownership even when a future refactor removes the
        // expected access boundary. The strict assertion remains after cleanup.
        caller.abort();
        let _ = caller.await;
        access.cleanup_release.notify_one();
        adapter.finish.notify_one();
        reap(&mut supervisor).await;
        writer.stop();
        writer.joined().await.unwrap();
        panic!("retained post-reservation native access did not enter");
    }
    let outstanding_at_hold = supervisor.outstanding();
    let calls_at_hold = access.calls.load(Ordering::SeqCst);
    // ProviderRuntime::empty has one permit. Poll the real semaphore waiter
    // once instead of using a delay to infer that reserve has taken the lease.
    let mut next_lease = Box::pin(state.providers.hold_test_slot(&adapter.id));
    let lease_poll = poll_fn(|cx| Poll::Ready(next_lease.as_mut().poll(cx))).await;
    let (lease_was_held, early_permit) = match lease_poll {
        Poll::Pending => (true, None),
        Poll::Ready(permit) => (false, Some(permit)),
    };

    let caller_was_cancelled = if shutdown {
        supervisor.begin_draining(ShutdownReason::Signal, tokio::time::Instant::now());
        supervisor.request_cancellation();
        None
    } else {
        caller.abort();
        Some((&mut caller).await.unwrap_err().is_cancelled())
    };
    // Observe Drop before any fallback cleanup or caller-branch service cancel.
    // This distinguishes child cancellation from the existing five-second
    // native access timeout and prevents cleanup from hiding the regression.
    let read_dropped_promptly = tokio::time::timeout(ACCESS_BOUND, access.drop_observed.notified())
        .await
        .is_ok();
    if !read_dropped_promptly {
        access.cleanup_release.notify_one();
    }
    let completion = tokio::time::timeout(JOIN_BOUND, supervisor.next_completion()).await;
    let shutdown_result = if shutdown {
        match tokio::time::timeout(JOIN_BOUND, &mut caller).await {
            Ok(result) => Some(result),
            Err(_) => {
                caller.abort();
                let _ = caller.await;
                None
            }
        }
    } else {
        None
    };
    let lease_was_released = if lease_was_held {
        let reacquired = tokio::time::timeout(JOIN_BOUND, &mut next_lease).await;
        let released = reacquired.is_ok();
        if let Ok(permit) = reacquired {
            drop(permit);
        }
        released
    } else {
        // A Ready acquire future cannot be polled a second time. Keep this
        // failure path observable and release its unexpected early permit.
        drop(early_permit);
        false
    };
    drop(next_lease);
    // Always observe remaining retained owners and the separate writer before
    // evaluating regression assertions, including a failed bounded observation.
    let start = supervisor.begin_draining(ShutdownReason::Signal, tokio::time::Instant::now());
    let report = supervisor
        .cancel_and_reap(start.budget.stage(tokio::time::Instant::now()))
        .await;
    writer.stop();
    writer.joined().await.unwrap();

    assert_eq!(
        calls_at_hold, 3,
        "hold is the retained dispatch freshness read"
    );
    assert_eq!(outstanding_at_hold.len(), 1);
    assert_eq!(
        outstanding_at_hold[0].kind,
        OwnedTaskKind::Operation(OperationKind::ProviderStream)
    );
    assert!(
        lease_was_held,
        "native access must hold the reserved provider lease"
    );
    assert!(
        read_dropped_promptly,
        "native access ignored actual generation cancellation"
    );
    let completion = completion.expect("real retained ProviderStream owner did not join");
    assert_eq!(
        completion.kind,
        OwnedTaskKind::Operation(OperationKind::ProviderStream)
    );
    assert_eq!(completion.exit, crate::service::TaskExit::Returned);
    assert!(!completion.abort_requested);
    assert!(completion.fatal.is_none());
    assert_eq!(access.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(access.calls.load(Ordering::SeqCst), 3);
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
    assert_eq!(adapter.dropped.load(Ordering::SeqCst), 0);
    assert_eq!(out.visible.load(Ordering::SeqCst), 0);
    assert_eq!(AppState::lock(&state.engine).session_len("discord:70"), 0);
    assert!(AppState::lock(&state.rewards).export_pending().is_empty());
    assert!(
        lease_was_released,
        "provider lease was not reacquirable after join"
    );
    assert_eq!(report.outcome, ReapOutcome::Joined);
    assert!(report.outstanding.is_empty());
    assert!(supervisor.outstanding().is_empty());
    assert!(supervisor.try_freeze(true).is_ok());
    if shutdown {
        let error = shutdown_result
            .expect("service cancellation did not finish the generation waiter")
            .unwrap()
            .unwrap_err();
        assert_eq!(
            error.provider_failure(),
            crate::provider::ProviderFailureKind::Cancelled
        );
        assert!(error.outbound_failure().is_none());
    } else {
        assert_eq!(caller_was_cancelled, Some(true));
    }
}

#[tokio::test]
async fn retained_continuity_caller_cancel_drops_fresh_read_and_joins_reserved_lease() {
    cancel_while_native_access_holds_provider_lease(false).await;
}

#[tokio::test]
async fn retained_continuity_service_cancel_drops_fresh_read_and_joins_reserved_lease() {
    cancel_while_native_access_holds_provider_lease(true).await;
}

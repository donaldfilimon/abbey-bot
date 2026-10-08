// Apply as src/runtime/engagement_delivery/tests/task_follow_up/delivery_tests/receipt_status.rs.
// In delivery_tests.rs add:
// #[path = "delivery_tests/receipt_status.rs"] mod receipt_status;
use super::*;
use crate::engagement::{ContactCharge, ExplicitFeedback, FeedbackKind};

fn origin() -> EngagementScope {
    EngagementScope::Dm {
        member: 2,
        channel: 3,
    }
}

#[tokio::test]
async fn task_follow_up_status_counts_only_fresh_owned_current_receipts_and_explicit_feedback() {
    let h = Harness::new();
    seed_task(&h);
    let transport = WorkProven::new(&h, Race::None, false);
    h.state
        .clone()
        .deliver_engagement(&transport, transport.fake.cancel.clone(), || NOW)
        .await
        .unwrap();
    assert_eq!(h.status(), CandidateState::Sent);
    let no_feedback = h
        .state
        .task_follow_up_status(2, &origin(), &transport, || NOW)
        .await;
    assert!(no_feedback.contains("terminal 1 = useful 0 + stopped/opted out 0 + failed/non-useful 0 + no explicit feedback 1; active 0"), "{no_feedback}");
    {
        let mut stores = AppState::lock(&h.state.stores);
        let e = &mut stores.work.engagement;
        e.feedback.entry(1).or_default().insert(
            2,
            ExplicitFeedback {
                actor: 2,
                at: NOW + 1,
                kind: FeedbackKind::Useful,
            },
        );
        let mut stale = e.candidates[&1].clone();
        stale.id = 2;
        stale.source.as_mut().unwrap().message = 6;
        stale.message_id = Some(124);
        let WorkContentRef::Task { project, id, .. } = stale.work_ref.clone().unwrap() else {
            unreachable!()
        };
        let reference = WorkContentRef::Task {
            project,
            id,
            revision: 1,
        };
        stale.dedupe_key = crate::work::follow_up::task_key(
            &crate::work::follow_up::work_scope(&stale.scope),
            &reference,
        )
        .unwrap();
        stale.work_ref = Some(reference);
        e.candidates.insert(2, stale);
        e.sequence = e.sequence.max(2);
        e.charges.push(ContactCharge {
            candidate_id: 2,
            member: 2,
            local_day: "2026-10-01".into(),
            local_week: "2026-09-28".into(),
            at: NOW,
        });
        e.validate().unwrap();
    }
    let before = AppState::lock(&h.state.stores).work.engagement.clone();
    let sends = transport.fake.sends.load(Ordering::SeqCst);
    let generations = transport.generations.load(Ordering::SeqCst);
    let shown = h
        .state
        .task_follow_up_status(2, &origin(), &transport, || NOW + 2)
        .await;
    assert!(shown.contains("terminal 1 = useful 1 + stopped/opted out 0 + failed/non-useful 0 + no explicit feedback 0; active 0"), "{shown}");
    assert!(shown.contains("Candidate 1:"));
    assert!(!shown.contains("Candidate 2:"));
    assert!(shown.contains("current access or task revision could not be confirmed"));
    assert!(!shown.contains("Qualify the allocator"));
    transport.deny_work.store(true, Ordering::SeqCst);
    let hidden = h
        .state
        .task_follow_up_status(2, &origin(), &transport, || NOW + 2)
        .await;
    assert!(hidden.contains("current access could not be confirmed"));
    assert!(!hidden.contains("Shown readable task receipts"));
    assert!(!hidden.contains("terminal "));
    assert!(!hidden.contains("useful 1"));
    assert!(!hidden.contains("Candidate "));
    assert!(
        h.state
            .task_follow_up_status(9, &origin(), &transport, || NOW)
            .await
            .is_empty()
    );
    assert!(
        h.state
            .task_follow_up_status(
                2,
                &EngagementScope::Dm {
                    member: 2,
                    channel: 8
                },
                &transport,
                || NOW
            )
            .await
            .is_empty()
    );
    assert_eq!(AppState::lock(&h.state.stores).work.engagement, before);
    assert_eq!(transport.fake.sends.load(Ordering::SeqCst), sends);
    assert_eq!(transport.generations.load(Ordering::SeqCst), generations);
    assert_eq!(charges(&h), 2);
    assert!(shown.chars().count() <= 2000);
    println!("{shown}");
    h.finish().await;
}

#[tokio::test]
async fn task_follow_up_status_keeps_latest_five_bound_and_omits_erased_outcomes() {
    let h = Harness::new();
    let task = seed_task(&h);
    {
        let mut stores = AppState::lock(&h.state.stores);
        let work = &mut stores.work;
        let template = work.engagement.candidates[&1].clone();
        let native = work.tasks[&task].clone();
        for candidate_id in 2..=7 {
            let mut next_task = native.clone();
            next_task.id = 0;
            next_task.title = format!("Synthetic task {candidate_id}");
            let next_id = work
                .add_task(
                    WorkAccess {
                        actor: 2,
                        guild: None,
                        channel: 3,
                        can_view: true,
                        can_manage: false,
                    },
                    next_task,
                    &format!("receipt-bound-{candidate_id}"),
                )
                .unwrap();
            let reference = WorkContentRef::Task {
                project: native.project_id,
                id: next_id,
                revision: 0,
            };
            let mut c = template.clone();
            c.id = candidate_id;
            c.source.as_mut().unwrap().message = 100 + candidate_id;
            c.dedupe_key = crate::work::follow_up::task_key(
                &crate::work::follow_up::work_scope(&c.scope),
                &reference,
            )
            .unwrap();
            c.work_ref = Some(reference);
            work.engagement.candidates.insert(candidate_id, c);
        }
        work.engagement.sequence = work.engagement.sequence.max(7);
        work.engagement.validate().unwrap();
    }
    let transport = WorkProven::new(&h, Race::None, false);
    let shown = h
        .state
        .task_follow_up_status(2, &origin(), &transport, || NOW)
        .await;
    assert!(shown.contains("terminal 0 = useful 0 + stopped/opted out 0 + failed/non-useful 0 + no explicit feedback 0; active 5"), "{shown}");
    assert_eq!(transport.work_calls.load(Ordering::SeqCst), 5);
    assert!(!shown.contains("Candidate 1:"));
    assert!(!shown.contains("Candidate 2:"));
    assert!(shown.contains("Candidate 7:"));
    assert!(shown.chars().count() <= 2000);
    {
        let mut stores = AppState::lock(&h.state.stores);
        let e = &mut stores.work.engagement;
        assert!(e.erase_learning("discord:dm:2", Some(2)).unwrap() > 0);
        assert!(e.candidates.is_empty());
        assert!(!e.erased_identities.is_empty());
    }
    assert!(
        h.state
            .task_follow_up_status(2, &origin(), &transport, || NOW)
            .await
            .is_empty()
    );
    assert_eq!(
        transport.work_calls.load(Ordering::SeqCst),
        5,
        "erasure cannot produce hidden receipt counts or native lookups"
    );
    assert_eq!(transport.generations.load(Ordering::SeqCst), 0);
    assert_eq!(charges(&h), 0);
    println!("{shown}");
    h.finish().await;
}

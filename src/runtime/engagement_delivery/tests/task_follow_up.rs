//! Exercise the real reservation/send owner with exact canonical native work.
use super::*;
use crate::work::{WorkAccess, WorkContentRef, WorkStatus, WorkTask};
fn seed_task(h: &Harness) -> u64 {
    let access = WorkAccess {
        actor: 2,
        guild: None,
        channel: 3,
        can_view: true,
        can_manage: false,
    };
    let mut stores = AppState::lock(&h.state.stores);
    let w = &mut stores.work;
    let project = w
        .create_project(access, "Checked personal project", "task-follow-up-project")
        .unwrap();
    let task = w
        .add_task(
            access,
            WorkTask {
                id: 0,
                project_id: project,
                title: "Qualify the allocator".into(),
                owner: 2,
                assignee: None,
                goal_id: None,
                priority: 0,
                status: WorkStatus::Open,
                due_at: None,
                remind_at: None,
                reminder_revision: 0,
                snoozed_until: None,
                source: None,
                github: None,
                revision: 0,
            },
            "task-follow-up",
        )
        .unwrap();
    let e = &mut w.engagement;
    let c = e.candidates.get_mut(&1).unwrap();
    let reference = WorkContentRef::Task {
        project,
        id: task,
        revision: 0,
    };
    c.work_ref = Some(reference.clone());
    c.expires_at = Some(NOW + 3600);
    c.dedupe_key =
        crate::work::follow_up::task_key(&crate::work::follow_up::work_scope(&c.scope), &reference)
            .unwrap();
    let source = c.source.clone().unwrap();
    e.member_policies
        .get_mut(&2)
        .unwrap()
        .destinations
        .insert(source.scope.clone(), DestinationPreference::Origin);
    e.responses.insert(source.message, 5);
    e.observations
        .entry(source.scope.clone())
        .or_default()
        .insert(2, source);
    e.validate().unwrap();
    task
}

#[tokio::test]
async fn task_follow_up_legacy_transport_cannot_send_or_charge_without_work_proof() {
    let h = Harness::new();
    seed_task(&h);
    let f = Fake::new(&h, Race::None);
    h.state
        .clone()
        .deliver_engagement(&f, f.cancel.clone(), || NOW)
        .await
        .unwrap();
    assert_eq!(
        f.sends.load(Ordering::SeqCst),
        0,
        "linked work cannot use the conversation source proof alone"
    );
    assert!(
        AppState::lock(&h.state.stores)
            .work
            .engagement
            .charges
            .is_empty()
    );
    h.finish().await;
}

#[path = "task_follow_up/delivery_tests.rs"]
mod delivery_tests;

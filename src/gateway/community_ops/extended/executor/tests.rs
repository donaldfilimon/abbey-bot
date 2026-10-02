use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
struct Fake {
    observations: std::sync::Mutex<std::collections::VecDeque<u64>>,
    calls: AtomicUsize,
    fail: bool,
    readback: bool,
}
impl Executor for Fake {
    type State = u64;
    async fn observe(&self) -> Result<u64, &'static str> {
        self.observations
            .lock()
            .unwrap()
            .pop_front()
            .ok_or("observation missing")
    }
    fn before(&self, s: &u64) -> Result<serde_json::Value, &'static str> {
        Ok(serde_json::json!(s))
    }
    fn signature(&self, s: &u64) -> Result<serde_json::Value, &'static str> {
        self.before(s)
    }
    async fn mutate(&self, _: &u64) -> Result<Option<u64>, &'static str> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            Err("uncertain")
        } else {
            Ok(None)
        }
    }
    fn verify(&self, _: &u64, _: &u64, _: Option<u64>) -> Result<serde_json::Value, &'static str> {
        if self.readback {
            Ok(serde_json::json!({"verified":true}))
        } else {
            Err("wrong state")
        }
    }
}
fn fake(states: &[u64], fail: bool, readback: bool) -> Fake {
    Fake {
        observations: std::sync::Mutex::new(states.iter().copied().collect()),
        calls: AtomicUsize::new(0),
        fail,
        readback,
    }
}
fn policy() -> Policy {
    serde_json::from_value(serde_json::json!({"version":1,"guild":1,"owner":2,"mode":"apply","daily_limit":5,"daily_creations":2,"public_categories":[3],"protected_channels":[],"ordinary_roles":[],"membership_matrix":[],"actions":[{"key":"interest-v1","reason":"owner-approved interest","operation":{"kind":"create_interest_role","name":"Research"}}]})).unwrap()
}
#[tokio::test]
async fn stop_policy_drift_and_observation_drift_never_mutate() {
    for case in 0..3 {
        let p = policy();
        let a = p.actions[0].clone();
        let f = fake(if case == 2 { &[1, 2] } else { &[1, 1] }, false, true);
        let cancel = CancellationToken::new();
        if case == 0 {
            cancel.cancel();
        }
        let mut l = Ledger::default();
        let result = run_one(
            &f,
            &p,
            "digest",
            &a,
            &mut l,
            cancel,
            1,
            |_| std::future::ready(Ok(())),
            || {
                let mut changed = p.clone();
                if case == 1 {
                    changed.mode = Mode::Stopped;
                }
                std::future::ready(Ok((changed, "digest".into())))
            },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(f.calls.load(Ordering::SeqCst), 0);
        assert!(l.receipts.is_empty());
    }
}
#[tokio::test]
async fn reservation_precedes_io_and_failed_readback_is_not_replayed() {
    for (fail, readback) in [(true, true), (false, false), (false, true)] {
        let p = policy();
        let a = p.actions[0].clone();
        let f = fake(&[1, 1, 2, 2], fail, readback);
        let mut l = Ledger::default();
        let stages = std::cell::RefCell::new(Vec::new());
        let result = run_one(
            &f,
            &p,
            "digest",
            &a,
            &mut l,
            CancellationToken::new(),
            1,
            |ledger| {
                stages
                    .borrow_mut()
                    .push(ledger.receipts[&a.key].status.clone());
                std::future::ready(Ok(()))
            },
            || std::future::ready(Ok((p.clone(), "digest".into()))),
        )
        .await;
        assert_eq!(stages.borrow()[0], Status::Reserved);
        assert_eq!(f.calls.load(Ordering::SeqCst), 1);
        assert_eq!(result.is_ok(), !fail && readback);
        assert_eq!(
            l.receipts[&a.key].status,
            if !fail && readback {
                Status::Verified
            } else {
                Status::ReviewRequired
            }
        );
        assert!(l.authorize(&p, &a, 86401).is_err());
    }
}
#[tokio::test]
async fn failed_reservation_publication_cannot_mutate() {
    let p = policy();
    let a = p.actions[0].clone();
    let f = fake(&[1, 1], false, true);
    let mut l = Ledger::default();
    assert!(
        run_one(
            &f,
            &p,
            "digest",
            &a,
            &mut l,
            CancellationToken::new(),
            1,
            |_| std::future::ready(Err("disk failure")),
            || std::future::ready(Ok((p.clone(), "digest".into())))
        )
        .await
        .is_err()
    );
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn unknown_and_duplicate_requests_never_gain_authority() {
    let p = policy();
    let mut a = p.actions[0].clone();
    a.key = "invented".into();
    let f = fake(&[1], false, true);
    let mut l = Ledger::default();
    assert!(
        run_one(
            &f,
            &p,
            "digest",
            &a,
            &mut l,
            CancellationToken::new(),
            1,
            |_| std::future::ready(Ok(())),
            || std::future::ready(Ok((p.clone(), "digest".into())))
        )
        .await
        .is_err()
    );
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    let mut duplicate = p.clone();
    duplicate.actions.push(duplicate.actions[0].clone());
    assert!(duplicate.validate().is_err());
}

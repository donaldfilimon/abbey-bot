use std::sync::{
    Arc, Barrier,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use super::*;

fn install_subject_generation(stores: &mut Stores, label: &str, at: u64) {
    let subject = stores.memory.user_mut("g", "u");
    subject.facts = vec![format!("{label}-fact")];
    subject.pending_supersessions = vec![memory::PendingSupersession {
        new_fact: format!("{label}-new"),
        old_fact: format!("{label}-old"),
        at,
    }];
    subject.updated_at = at;
}

#[test]
fn subject_snapshot_clones_facts_and_pending_together() {
    let state = AppState::in_memory();
    let service = state.memory_service();
    service.remember("g", "u", "uses rust", 1).expect("store");
    service
        .remember_proposing("g", "u", "moved to zig", "uses rust", 2)
        .expect("propose");

    let (mut facts, mut pending) = service.subject_snapshot("g", "u");
    assert_eq!(facts, ["uses rust", "moved to zig"]);
    assert_eq!(
        pending,
        [memory::PendingSupersession {
            new_fact: "moved to zig".to_string(),
            old_fact: "uses rust".to_string(),
            at: 2,
        }]
    );

    facts.clear();
    pending.clear();
    let (stored_facts, stored_pending) = service.subject_snapshot("g", "u");
    assert_eq!(stored_facts, ["uses rust", "moved to zig"]);
    assert_eq!(stored_pending.len(), 1, "the snapshot must own its clones");
}

#[test]
fn subject_snapshot_isolates_guilds_and_users() {
    let state = AppState::in_memory();
    let service = state.memory_service();
    service.remember("g1", "u1", "g1-u1 old", 1).expect("seed");
    service
        .remember_proposing("g1", "u1", "g1-u1 new", "g1-u1 old", 2)
        .expect("propose");
    service.remember("g1", "u2", "g1-u2", 3).expect("seed");
    service.remember("g2", "u1", "g2-u1", 4).expect("seed");

    let (facts, pending) = service.subject_snapshot("g1", "u1");
    assert_eq!(facts, ["g1-u1 old", "g1-u1 new"]);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].old_fact, "g1-u1 old");
    assert_eq!(pending[0].new_fact, "g1-u1 new");

    assert_eq!(service.subject_snapshot("g1", "u2").0, ["g1-u2"]);
    assert!(service.subject_snapshot("g1", "u2").1.is_empty());
    assert_eq!(service.subject_snapshot("g2", "u1").0, ["g2-u1"]);
    assert!(service.subject_snapshot("g2", "u1").1.is_empty());

    assert!(
        AppState::lock(&state.stores)
            .memory
            .user("missing", "subject")
            .is_none()
    );
    assert_eq!(
        service.subject_snapshot("missing", "subject"),
        (Vec::new(), Vec::new())
    );
    assert!(
        AppState::lock(&state.stores)
            .memory
            .user("missing", "subject")
            .is_none(),
        "a snapshot read must not provision a subject"
    );
}

#[test]
fn concurrent_subject_snapshots_never_mix_generations() {
    const SNAPSHOTS: usize = 20_000;

    let state = AppState::in_memory();
    install_subject_generation(&mut AppState::lock(&state.stores), "A", 1);

    let start = Arc::new(Barrier::new(2));
    let stop = Arc::new(AtomicBool::new(false));
    let writes = Arc::new(AtomicUsize::new(0));
    let writer = {
        let state = Arc::clone(&state);
        let start = Arc::clone(&start);
        let stop = Arc::clone(&stop);
        let writes = Arc::clone(&writes);
        std::thread::spawn(move || {
            start.wait();
            let mut use_a = false;
            while !stop.load(Ordering::Acquire) {
                let label = if use_a { "A" } else { "B" };
                let at = writes.load(Ordering::Relaxed) as u64 + 2;
                install_subject_generation(&mut AppState::lock(&state.stores), label, at);
                writes.fetch_add(1, Ordering::Release);
                use_a = !use_a;
                std::thread::yield_now();
            }
        })
    };

    start.wait();
    while writes.load(Ordering::Acquire) == 0 {
        std::thread::yield_now();
    }

    let mut mismatch = None;
    for _ in 0..SNAPSHOTS {
        let snapshot = state.memory_service().subject_snapshot("g", "u");
        let consistent = match (&snapshot.0[..], &snapshot.1[..]) {
            ([fact], [pending]) if fact == "A-fact" => {
                pending.new_fact == "A-new" && pending.old_fact == "A-old"
            }
            ([fact], [pending]) if fact == "B-fact" => {
                pending.new_fact == "B-new" && pending.old_fact == "B-old"
            }
            _ => false,
        };
        if !consistent {
            mismatch = Some(snapshot);
            break;
        }
        std::thread::yield_now();
    }
    stop.store(true, Ordering::Release);
    writer.join().expect("writer did not panic");

    assert!(
        mismatch.is_none(),
        "facts and pending replacements came from different generations: {mismatch:?}"
    );
    assert!(writes.load(Ordering::Acquire) > 0);
}

#[test]
fn admitted_fact_receipt_and_pending_replacement_share_one_snapshot() {
    let state = AppState::in_memory();
    let service = state.memory_service();
    service.remember("g", "u", "uses rust", 1).expect("seed");
    let receipt = "ab".repeat(32);

    let outcome = service
        .remember_admitted(
            "g",
            "u",
            "moved to zig",
            Some("uses rust"),
            2,
            Some(&receipt),
        )
        .expect("admitted write");
    assert_eq!(
        outcome,
        RememberOutcome::Proposed {
            stored: "moved to zig".into(),
            proposed: "uses rust".into(),
        }
    );

    let (stores, recall) = service.consistent_snapshot_after(|_| {});
    assert_eq!(stores.memory.facts("g", "u"), ["uses rust", "moved to zig"]);
    assert_eq!(
        stores.memory.pending_supersessions("g", "u"),
        [memory::PendingSupersession {
            new_fact: "moved to zig".into(),
            old_fact: "uses rust".into(),
            at: 2,
        }]
    );
    assert_eq!(
        stores
            .memory_receipts
            .get(&MemoryService::receipt_key("g", "u", "moved to zig")),
        Some(&receipt)
    );
    assert_eq!(
        recall
            .facts_for_user("g", "u")
            .into_iter()
            .map(|fact| fact.text)
            .collect::<Vec<_>>(),
        ["uses rust", "moved to zig"]
    );
}

#[test]
fn concurrent_snapshots_never_split_an_admitted_fact_from_its_receipt() {
    const WRITES: usize = 500;

    let state = AppState::in_memory();
    let writer = {
        let state = Arc::clone(&state);
        std::thread::spawn(move || {
            for index in 0..WRITES {
                let user = format!("u{index}");
                state
                    .memory_service()
                    .remember_admitted(
                        "g",
                        &user,
                        "admitted fact",
                        None,
                        index as u64,
                        Some(&"ab".repeat(32)),
                    )
                    .expect("admitted write");
                std::thread::yield_now();
            }
        })
    };

    let assert_snapshot = |stores: &Stores| {
        for index in 0..WRITES {
            let user = format!("u{index}");
            let fact_present = stores
                .memory
                .facts("g", &user)
                .iter()
                .any(|fact| fact == "admitted fact");
            let receipt_present = stores
                .memory_receipts
                .contains_key(&MemoryService::receipt_key("g", &user, "admitted fact"));
            assert_eq!(
                fact_present, receipt_present,
                "snapshot split fact and receipt for {user}"
            );
        }
    };

    while !writer.is_finished() {
        let (stores, _) = state.memory_service().consistent_snapshot_after(|_| {});
        assert_snapshot(&stores);
        std::thread::yield_now();
    }
    writer.join().expect("writer did not panic");
    let (stores, _) = state.memory_service().consistent_snapshot_after(|_| {});
    assert_snapshot(&stores);
}

/// The explicit path is authoritative — it removes the named fact without
/// any confirmation step, because the caller already gave the signal.
#[test]
fn explicit_replaces_supersedes_atomically() {
    let state = AppState::in_memory();
    let service = state.memory_service();
    service.remember("g", "u", "uses rust", 1).expect("store");
    let outcome = service
        .remember_replacing("g", "u", "moved to zig", "uses rust", 2)
        .expect("supersede");
    assert_eq!(
        outcome,
        RememberOutcome::Superseded {
            stored: "moved to zig".to_string(),
            removed: "uses rust".to_string(),
        }
    );
    assert_eq!(service.facts("g", "u"), vec!["moved to zig".to_string()]);
    assert!(service.pending_supersessions("g", "u").is_empty());
}

/// A rejected fact must never cause a deletion. Validation runs before
/// the forget, so the old fact is still there afterwards.
#[test]
fn a_rejected_replacement_never_removes_the_old_fact() {
    let state = AppState::in_memory();
    let service = state.memory_service();
    service.remember("g", "u", "uses rust", 1).expect("store");
    let too_long = "x".repeat(memory::MAX_FACT_CHARS + 1);
    assert!(
        service
            .remember_replacing("g", "u", &too_long, "uses rust", 2)
            .is_err()
    );
    assert_eq!(service.facts("g", "u"), vec!["uses rust".to_string()]);
}

/// Naming a fact that is not held must not store the new one under a
/// false pretense, and must not remove anything.
#[test]
fn replacing_an_absent_fact_is_refused_without_storing() {
    let state = AppState::in_memory();
    let service = state.memory_service();
    service.remember("g", "u", "uses rust", 1).expect("store");
    assert!(
        service
            .remember_replacing("g", "u", "moved to zig", "plays banjo", 2)
            .is_err()
    );
    assert_eq!(service.facts("g", "u"), vec!["uses rust".to_string()]);
}

/// Superseding must work at the cap: the forget frees the slot the new
/// fact needs. Without forget-before-remember this silently fails.
#[test]
fn superseding_works_at_the_fact_cap() {
    let state = AppState::in_memory();
    let service = state.memory_service();
    for index in 0..memory::MAX_FACTS {
        service
            .remember("g", "u", &format!("fact number {index}"), 1)
            .expect("seed");
    }
    assert_eq!(service.facts("g", "u").len(), memory::MAX_FACTS);
    // A plain remember at the cap is rejected...
    assert_eq!(
        service.remember("g", "u", "one more", 2).expect("capped"),
        RememberOutcome::Unchanged
    );
    // ...but an explicit supersession still succeeds.
    let outcome = service
        .remember_replacing("g", "u", "one more", "fact number 0", 3)
        .expect("supersede at cap");
    assert_eq!(
        outcome,
        RememberOutcome::Superseded {
            stored: "one more".to_string(),
            removed: "fact number 0".to_string(),
        }
    );
    let facts = service.facts("g", "u");
    assert_eq!(facts.len(), memory::MAX_FACTS);
    assert!(facts.contains(&"one more".to_string()));
    assert!(!facts.contains(&"fact number 0".to_string()));
}

/// THE core invariant of this feature: a model proposal stores the new
/// fact but must never remove the old one.
#[test]
fn a_model_proposal_stores_without_removing_anything() {
    let state = AppState::in_memory();
    let service = state.memory_service();
    service.remember("g", "u", "uses rust", 1).expect("store");
    let outcome = service
        .remember_proposing("g", "u", "moved to zig", "uses rust", 2)
        .expect("propose");
    assert_eq!(
        outcome,
        RememberOutcome::Proposed {
            stored: "moved to zig".to_string(),
            proposed: "uses rust".to_string(),
        }
    );
    let facts = service.facts("g", "u");
    assert!(facts.contains(&"uses rust".to_string()), "{facts:?}");
    assert!(facts.contains(&"moved to zig".to_string()), "{facts:?}");
    assert_eq!(service.pending_supersessions("g", "u").len(), 1);
}

/// Proposing against a fact that is not held still stores the new fact —
/// it stands on its own merits — but queues nothing to contest.
#[test]
fn proposing_against_an_absent_fact_stores_without_queuing() {
    let state = AppState::in_memory();
    let service = state.memory_service();
    let outcome = service
        .remember_proposing("g", "u", "moved to zig", "never said this", 1)
        .expect("propose");
    assert_eq!(outcome, RememberOutcome::Stored("moved to zig".to_string()));
    assert!(service.pending_supersessions("g", "u").is_empty());
}

#[test]
fn confirming_a_proposal_removes_the_old_fact_exactly_once() {
    let state = AppState::in_memory();
    let service = state.memory_service();
    service.remember("g", "u", "uses rust", 1).expect("store");
    service
        .remember_proposing("g", "u", "moved to zig", "uses rust", 2)
        .expect("propose");
    assert_eq!(
        service.confirm_supersession("g", "u", "uses rust"),
        SupersessionOutcome::Confirmed("uses rust".to_string())
    );
    assert_eq!(service.facts("g", "u"), vec!["moved to zig".to_string()]);
    assert!(service.pending_supersessions("g", "u").is_empty());
    // Confirming again must not resurrect or re-remove anything.
    assert_eq!(
        service.confirm_supersession("g", "u", "uses rust"),
        SupersessionOutcome::NotPending
    );
}

/// The race the plan called out: by the time someone confirms, a bare
/// `/forget` may already have removed the old fact. That must report
/// distinctly rather than silently succeeding.
#[test]
fn confirming_after_the_old_fact_is_already_gone_reports_it() {
    let state = AppState::in_memory();
    let service = state.memory_service();
    service.remember("g", "u", "uses rust", 1).expect("store");
    service
        .remember_proposing("g", "u", "moved to zig", "uses rust", 2)
        .expect("propose");
    assert!(service.forget("g", "u", "uses rust"));
    assert_eq!(
        service.confirm_supersession("g", "u", "uses rust"),
        SupersessionOutcome::AlreadyGone("uses rust".to_string())
    );
    assert!(service.pending_supersessions("g", "u").is_empty());
}

/// A proposal is a claim that `new_fact` replaces `old_fact`. If the
/// replacement is itself removed, confirming must NOT go through — that
/// would leave the person holding neither fact, having acted on a
/// `/pending list` display that no longer matched reality.
#[test]
fn confirming_refuses_when_the_replacement_fact_is_gone() {
    let state = AppState::in_memory();
    let service = state.memory_service();
    service.remember("g", "u", "uses rust", 1).expect("store");
    service
        .remember_proposing("g", "u", "moved to zig", "uses rust", 2)
        .expect("propose");
    // The human removes the REPLACEMENT, not the original.
    assert!(service.forget("g", "u", "moved to zig"));
    assert_eq!(
        service.confirm_supersession("g", "u", "uses rust"),
        SupersessionOutcome::PremiseGone {
            old_fact: "uses rust".to_string(),
            new_fact: "moved to zig".to_string(),
        }
    );
    // The original survives: the user is not left with nothing.
    assert_eq!(service.facts("g", "u"), vec!["uses rust".to_string()]);
    // And the stale proposal is cleared rather than left to mislead again.
    assert!(service.pending_supersessions("g", "u").is_empty());
}

#[test]
fn dismissing_a_proposal_keeps_both_facts() {
    let state = AppState::in_memory();
    let service = state.memory_service();
    service.remember("g", "u", "uses rust", 1).expect("store");
    service
        .remember_proposing("g", "u", "moved to zig", "uses rust", 2)
        .expect("propose");
    assert!(service.dismiss_supersession("g", "u", "uses rust"));
    let facts = service.facts("g", "u");
    assert!(facts.contains(&"uses rust".to_string()));
    assert!(facts.contains(&"moved to zig".to_string()));
    assert!(service.pending_supersessions("g", "u").is_empty());
}

#[test]
fn remember_validates_once_and_updates_both_representations() {
    let state = AppState::in_memory();
    assert_eq!(
        state
            .memory_service()
            .remember("g", "u", "  Donald\nlikes\tRust.  ", 7),
        Ok(RememberOutcome::Stored("Donald likes Rust.".into()))
    );
    assert_eq!(
        AppState::lock(&state.stores).memory.facts("g", "u"),
        ["Donald likes Rust."]
    );
    assert_eq!(
        AppState::lock(&state.recall)
            .facts_for_user("g", "u")
            .into_iter()
            .map(|fact| fact.text)
            .collect::<Vec<_>>(),
        ["Donald likes Rust."]
    );
    assert_eq!(
        state
            .memory_service()
            .remember("g", "u", &"🦀".repeat(memory::MAX_FACT_CHARS + 1), 8),
        Err("Keep one remembered fact to 300 characters or fewer.")
    );
    assert!(
        state
            .memory_service()
            .forget("g", "u", " Donald   likes Rust. "),
        "a whitespace variant selects the normalized canonical fact"
    );
    assert!(
        state.memory_service().facts("g", "u").is_empty(),
        "canonical JSON fact is deleted"
    );
    assert!(
        AppState::lock(&state.recall)
            .facts_for_user("g", "u")
            .is_empty(),
        "WDBX projection is reconciled in the same operation"
    );
}

#[test]
fn runtime_reads_only_canonical_facts_after_migration() {
    let state = AppState::in_memory();
    AppState::lock(&state.stores)
        .memory
        .remember("g", "u", "plain only", 1);
    AppState::lock(&state.recall).remember("g", "u", "semantic only", 2);

    assert_eq!(state.memory_service().facts("g", "u"), ["plain only"]);
    assert!(!state.memory_service().forget("g", "u", "semantic only"));
}

#[test]
fn concurrent_writes_and_snapshots_never_observe_one_sided_facts() {
    let state = AppState::in_memory();
    let writer = {
        let state = Arc::clone(&state);
        std::thread::spawn(move || {
            for i in 0..memory::MAX_FACTS {
                let fact = format!("fact {i}");
                let _ = state.memory_service().remember("g", "u", &fact, i as u64);
            }
        })
    };

    while !writer.is_finished() {
        let (stores, recall) = state.memory_service().consistent_snapshot_after(|_| {});
        let plain = stores.memory.facts("g", "u").to_vec();
        let semantic: Vec<String> = recall
            .facts_for_user("g", "u")
            .into_iter()
            .map(|fact| fact.text)
            .collect();
        assert_eq!(plain, semantic);
    }
    writer.join().expect("writer did not panic");
    // The writer can finish before this thread is first scheduled, so the
    // test must assert at least one snapshot independent of the race.
    let (stores, recall) = state.memory_service().consistent_snapshot_after(|_| {});
    assert_eq!(
        stores.memory.facts("g", "u"),
        recall
            .facts_for_user("g", "u")
            .into_iter()
            .map(|fact| fact.text)
            .collect::<Vec<_>>()
    );
}

#[test]
fn first_load_recovers_legacy_wdbx_only_facts_then_marks_json_canonical() {
    let mut stores = Stores::default();
    stores.memory.remember("g", "u", "plain", 1);
    let mut recall = Recall::new();
    recall.remember("g", "u", "semantic legacy", 2);

    let (stores, recall) = reconcile_loaded(stores, recall).expect("legacy migration");
    assert_eq!(stores.memory_projection_version, MEMORY_PROJECTION_VERSION);
    assert_eq!(stores.memory.facts("g", "u"), ["plain", "semantic legacy"]);
    assert_eq!(
        recall
            .facts_for_user("g", "u")
            .into_iter()
            .map(|fact| fact.text)
            .collect::<Vec<_>>(),
        ["plain", "semantic legacy"]
    );
}

#[test]
fn canonical_json_repairs_a_crash_stale_wdbx_without_resurrecting_deletions() {
    let mut stores = Stores {
        memory_projection_version: MEMORY_PROJECTION_VERSION,
        ..Stores::default()
    };
    stores.memory.remember("g", "u", "survives", 2);
    let mut recall = Recall::new();
    recall.remember("g", "u", "deleted before crash", 1);

    let (stores, recall) = reconcile_loaded(stores, recall).expect("canonical repair");
    assert_eq!(stores.memory.facts("g", "u"), ["survives"]);
    assert_eq!(
        recall
            .facts_for_user("g", "u")
            .into_iter()
            .map(|fact| fact.text)
            .collect::<Vec<_>>(),
        ["survives"]
    );
}

#[test]
fn context_uses_the_callers_live_social_standing_not_legacy_memory() {
    let state = AppState::in_memory();
    AppState::lock(&state.stores)
        .memory
        .user_mut("g", "u")
        .reputation = 0.11;

    let context = state
        .memory_service()
        .context_for("g", "u", "c", "anything", 3, 0.83);

    assert_eq!(context.reputation, 0.83);
}

#[test]
fn a_future_projection_version_fails_closed_before_reconciliation() {
    let mut stores = Stores {
        memory_projection_version: MEMORY_PROJECTION_VERSION + 1,
        ..Stores::default()
    };
    stores.memory.remember("g", "u", "future canonical fact", 2);
    let mut recall = Recall::new();
    recall.remember("g", "u", "future semantic fact", 3);

    let error = reconcile_loaded(stores, recall).expect_err("future schema must not start");

    assert!(error.contains("unsupported memory projection version 2"));
    assert!(error.contains("supports up to 1"));
}

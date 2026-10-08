//! Canonical continuity prerequisites for the enabled command/prompt vertical slice.
use super::*;
use crate::work::{WorkAccess, WorkContentRef, continuity::*};
use std::collections::BTreeSet;

fn confirmed_stores(now: u64) -> serde_json::Value {
    let access = WorkAccess {
        actor: 7,
        guild: None,
        channel: 70,
        can_view: true,
        can_manage: false,
    };
    let mut stores = Stores::default();
    let project = stores
        .work
        .create_project(access, "Allocator", "project")
        .unwrap();
    let id = stores
        .work
        .record_decision(project, access, "Keep checks", now, "decision")
        .unwrap();
    let mut registry = ProposalRegistry::new([1; 16]);
    let p = registry
        .propose(
            ContinuityDraft {
                scope: access.scope(),
                base_revision: 0,
                presented_text: "Resume checked allocator".into(),
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
    let grant = registry
        .resolve_confirmation(p.id, access.actor, &access.scope(), now)
        .unwrap();
    let mut cards = ContinuityStore::default();
    cards.confirm(grant, &access, &stores.work, now).unwrap();
    let mut value = serde_json::to_value(stores).unwrap();
    value["continuity"] = serde_json::to_value(cards).unwrap();
    value
}

#[test]
fn continuity_legacy_canonical_document_defaults_empty() {
    let old: Stores = serde_json::from_value(serde_json::json!({})).unwrap();
    assert_eq!(
        serde_json::to_value(old).unwrap()["continuity"],
        serde_json::json!({"cards": []})
    );
}

#[test]
fn continuity_confirmed_card_round_trips_in_the_atomic_canonical_document() {
    let now = crate::runtime::now();
    let value = confirmed_stores(now);
    let stores: Stores = serde_json::from_value(value.clone()).unwrap();
    let dir = temp_dir("continuity-native-roundtrip");
    stores.save(&dir).unwrap();
    let restored = serde_json::to_value(Stores::load(&dir).unwrap()).unwrap();
    assert_eq!(restored["continuity"], value["continuity"]);
    assert_eq!(restored["work"], value["work"]);
    let disk: serde_json::Value =
        serde_json::from_slice(&fs::read(Stores::state_path(&dir)).unwrap()).unwrap();
    assert_eq!(disk["continuity"], value["continuity"]);
    assert_eq!(disk["work"], value["work"]);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn continuity_canonical_deserialization_enforces_domain_bounds() {
    let value = confirmed_stores(crate::runtime::now());
    for (field, bad) in [
        ("revision", serde_json::json!(0)),
        ("confirmed_by", serde_json::json!(8)),
        ("confirmed_text", serde_json::json!("x".repeat(1601))),
    ] {
        let mut invalid = value.clone();
        invalid["continuity"]["cards"][0][field] = bad;
        assert!(
            serde_json::from_value::<Stores>(invalid).is_err(),
            "{field}"
        );
    }
}

#[test]
fn continuity_restart_discards_expired_confirmed_cards() {
    let value = confirmed_stores(100);
    let stores: Stores = serde_json::from_value(value).unwrap();
    let dir = temp_dir("continuity-expired");
    stores.save(&dir).unwrap();
    let restored = serde_json::to_value(Stores::load(&dir).unwrap()).unwrap();
    assert_eq!(restored["continuity"], serde_json::json!({"cards": []}));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn continuity_restart_discards_missing_native_source_without_dropping_work() {
    let mut value = confirmed_stores(crate::runtime::now());
    value["work"]["decisions"].as_object_mut().unwrap().clear();
    let stores: Stores = serde_json::from_value(value.clone()).unwrap();
    let dir = temp_dir("continuity-missing-source");
    stores.save(&dir).unwrap();
    let restored = serde_json::to_value(Stores::load(&dir).unwrap()).unwrap();
    assert_eq!(restored["continuity"], serde_json::json!({"cards": []}));
    assert_eq!(restored["work"], value["work"]);
    fs::remove_dir_all(dir).unwrap();
}

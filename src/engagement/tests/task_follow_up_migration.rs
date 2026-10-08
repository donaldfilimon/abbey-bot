//! Existing deserialize seam must retain the additive native-task identity.
use super::*;

#[test]
fn follow_up_native_task_revision_zero_round_trips_additive_metadata() {
    let mut wire = serde_json::to_value(candidate(1)).unwrap();
    let reference = serde_json::json!({"Task": {"project": 7, "id": 8, "revision": 0}});
    wire["work_ref"] = reference.clone();
    wire["expires_at"] = serde_json::json!(86406);
    let decoded: Candidate = serde_json::from_value(wire)
        .expect("native task follow-up provenance is accepted and retained");
    let saved = serde_json::to_value(decoded).unwrap();
    assert_eq!(saved["work_ref"], reference);
    assert_eq!(saved["expires_at"], 86406);
}

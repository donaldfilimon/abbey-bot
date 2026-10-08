// Integrate as a child of persist::moderation_shadow::tests.
// Root owns source registration and Cargo execution; this draft is external.
use super::*;

#[test]
fn stopped_policy_cas_refusal_precedes_operational_directory_creation() {
    let f = fixture();
    assert!(!f.data.join("community-operations").exists());
    crate::persist::community_ops::set_mode(&f.policy, &f.digest, 1, 2, Mode::Stopped).unwrap();
    assert!(transact(&f.data, &f.policy, &f.digest, capture(100), || 100).is_err());
    assert!(
        !f.data.join("community-operations").exists(),
        "a queued capture with a stale pre-Stop digest must not create operational paths"
    );
    assert!(!f.policy.with_extension("mode-lock").exists());
}

#[test]
fn expired_capture_refusal_precedes_operational_directory_creation() {
    let f = fixture();
    assert!(!f.data.join("community-operations").exists());
    assert!(transact(&f.data, &f.policy, &f.digest, capture(100), || 161).is_err());
    assert!(
        !f.data.join("community-operations").exists(),
        "an expired proof cannot create an operational path before denial"
    );
    assert!(!f.policy.with_extension("mode-lock").exists());
}

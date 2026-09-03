use super::*;

#[test]
fn provider_ids_normalize_to_one_stable_environment_segment() {
    let id = ProviderId::parse("  Foundation_MODELS  ").expect("valid provider id");
    assert_eq!(id.as_str(), "foundation-models");
    assert_eq!(id.env_segment(), "FOUNDATION_MODELS");
    assert_eq!(id.to_string(), "foundation-models");
}

#[test]
fn provider_ids_reject_ambiguous_or_non_ascii_shapes() {
    for raw in ["", "-mlx", "mlx-", "mlx__local", "mlx/-local", "mıx"] {
        assert!(ProviderId::parse(raw).is_err(), "accepted {raw:?}");
    }
}

#[test]
fn provider_id_serde_writes_only_the_canonical_string() {
    let id = ProviderId::parse("Foundation_Models").expect("valid provider id");
    let encoded = serde_json::to_string(&id).expect("serialize provider id");
    assert_eq!(encoded, r#""foundation-models""#);
    assert_eq!(
        serde_json::from_str::<ProviderId>(&encoded).expect("deserialize provider id"),
        id
    );
}

#[test]
fn eligibility_is_routable_only_for_the_explicit_routable_state() {
    assert!(Eligibility::Routable.is_routable());
    assert!(!Eligibility::TemporarilyUnavailable(TemporaryUnavailableReason::Busy).is_routable());
    assert!(!Eligibility::Blocked(BlockedReason::Unqualified).is_routable());
}

#[test]
fn turn_adapter_is_object_safe() {
    fn accepts_trait_object(_adapter: Option<&dyn TurnAdapter>) {}
    accepts_trait_object(None);
}

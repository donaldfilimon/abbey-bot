use super::*;
#[test]
fn closed_event_has_only_bounded_operational_fields() {
    let event = OperationalEvent::new(
        42,
        EventComponent::Provider,
        EventCode::ProviderAttempt,
        EventOutcome::Failed,
    )
    .unwrap()
    .with_error(OperationalErrorCategory::Timeout)
    .with_duration(std::time::Duration::from_millis(17))
    .with_count(3)
    .with_task(EventTask::Scheduler);
    let encoded = event.encode().unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(doc["duration_ms"], 17);
    for forbidden in [
        "pid",
        "run_nonce",
        "executable_sha256",
        "message",
        "error",
        "details",
        "fields",
        "url",
        "user_id",
        "guild_id",
        "channel_id",
    ] {
        assert!(doc.get(forbidden).is_none());
    }
    assert!(encoded.len() < 512);
    assert!(
        OperationalEvent::new(
            u64::MAX,
            EventComponent::Process,
            EventCode::Starting,
            EventOutcome::Started
        )
        .is_err()
    );
    assert!(serde_json::from_str::<EventCode>("\"PRIVATE_CANARY\"").is_err());
}

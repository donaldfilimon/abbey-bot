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

#[test]
fn work_recall_outcomes_remain_closed_content_free_events() {
    for code in [EventCode::WorkRecallAdmission, EventCode::WorkRecallUnknown] {
        let event =
            OperationalEvent::new(1, EventComponent::WorkRecall, code, EventOutcome::Degraded)
                .unwrap();
        let encoded = serde_json::to_string(&event).unwrap();
        assert!(encoded.contains("work_recall"));
        for forbidden in ["payload", "digest", "principal", "source", "guild", "token"] {
            assert!(!encoded.contains(forbidden));
        }
        assert!(encoded.len() < 512);
    }
}

#[test]
fn text_phase_events_never_contain_sensitive_or_dynamic_fields() {
    for code in [
        EventCode::GenerationQueue,
        EventCode::GenerationFirstText,
        EventCode::DiscordFirstPost,
        EventCode::DiscordFinalDelivered,
        EventCode::GenerationCompleted,
        EventCode::GenerationFailure,
        EventCode::DiscordPostFailure,
        EventCode::EngagementQueue,
        EventCode::EngagementCompleted,
        EventCode::EngagementFailure,
    ] {
        let event = OperationalEvent::new(1, EventComponent::Provider, code, EventOutcome::Failed)
            .unwrap()
            .with_duration(std::time::Duration::from_millis(10))
            .with_error(OperationalErrorCategory::Unavailable)
            .with_provider(crate::provider::ProviderId::parse("primary").unwrap());
        let doc: serde_json::Value = serde_json::from_slice(&event.encode().unwrap()).unwrap();
        for key in doc.as_object().unwrap().keys() {
            assert!(
                [
                    "schema_version",
                    "occurred_at_unix_ms",
                    "component",
                    "code",
                    "outcome",
                    "error_category",
                    "duration_ms",
                    "provider_id"
                ]
                .contains(&key.as_str()),
                "{key}"
            );
        }
    }
}

#[test]
fn final_delivered_is_a_closed_event_code() {
    let code: EventCode = serde_json::from_str("\"discord_final_delivered\"")
        .expect("actual final delivery has a distinct closed stage");
    assert_eq!(
        serde_json::to_value(code).unwrap(),
        "discord_final_delivered"
    );
}

#[test]
fn voice_stage_events_are_closed_content_free_durations() {
    for (stage, code) in [
        (VoiceStage::Recognition, "voice_recognition"),
        (VoiceStage::Generation, "voice_generation"),
        (VoiceStage::Synthesis, "voice_synthesis"),
    ] {
        for outcome in [
            EventOutcome::Succeeded,
            EventOutcome::Failed,
            EventOutcome::Cancelled,
            EventOutcome::TimedOut,
        ] {
            let event = OperationalEvent::new(1, EventComponent::Voice, stage.code(), outcome)
                .unwrap()
                .with_duration(std::time::Duration::from_millis(17));
            let doc: serde_json::Value = serde_json::from_slice(&event.encode().unwrap()).unwrap();
            assert_eq!(doc["code"], code);
            assert_eq!(doc["duration_ms"], 17);
            assert_eq!(doc.as_object().unwrap().len(), 6);
            for key in doc.as_object().unwrap().keys() {
                assert!(
                    [
                        "schema_version",
                        "occurred_at_unix_ms",
                        "component",
                        "code",
                        "outcome",
                        "duration_ms"
                    ]
                    .contains(&key.as_str())
                );
            }
        }
    }
}

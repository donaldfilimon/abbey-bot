//! Actual actor stages recorded through the retained managed telemetry writer.
use super::*;
use crate::service::telemetry::TelemetryWriter;

type Recorded = Arc<std::sync::Mutex<Vec<serde_json::Value>>>;

fn recording(fixture: &Fixture) -> (TelemetryWriter, Recorded) {
    let (writer, events) = TelemetryWriter::recording_for_test();
    fixture.runtime.attach_telemetry(writer.requests());
    (writer, events)
}

async fn recorded(mut writer: TelemetryWriter, events: Recorded) -> Vec<serde_json::Value> {
    writer.stop();
    writer.joined().await.unwrap();
    let events = events.lock().unwrap().clone();
    events
        .into_iter()
        .filter(|event| {
            event["component"] == "voice"
                && event["code"].as_str().unwrap().starts_with("voice_")
                && event["code"] != "voice_state"
        })
        .collect()
}

fn assert_stages(events: &[serde_json::Value], expected: &[(&str, &str)]) {
    let actual: Vec<_> = events
        .iter()
        .map(|event| {
            assert!(event["duration_ms"].as_u64().is_some());
            for key in event.as_object().unwrap().keys() {
                assert!(
                    [
                        "schema_version",
                        "occurred_at_unix_ms",
                        "component",
                        "code",
                        "outcome",
                        "duration_ms"
                    ]
                    .contains(&key.as_str()),
                    "private or dynamic key: {key}"
                );
            }
            (
                event["code"].as_str().unwrap(),
                event["outcome"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn managed_voice_stages_cover_the_real_generative_turn() {
    let mut fixture = Fixture::with_gate("unused", "none").await;
    let (writer, events) = recording(&fixture);
    fixture.utterance(1).await;
    fixture.expect_playback(true).await;
    fixture.stop().await;
    assert_stages(
        &recorded(writer, events).await,
        &[
            ("voice_recognition", "succeeded"),
            ("voice_generation", "succeeded"),
            ("voice_synthesis", "succeeded"),
        ],
    );
}

#[tokio::test]
async fn managed_voice_operational_reply_has_synthesis_without_generation() {
    let mut fixture =
        Fixture::with_transcripts("Abby, are you listening?", "unused", "none", None).await;
    let (writer, events) = recording(&fixture);
    fixture.utterance(1).await;
    fixture.expect_playback(true).await;
    fixture.stop().await;
    assert_stages(
        &recorded(writer, events).await,
        &[
            ("voice_recognition", "succeeded"),
            ("voice_synthesis", "succeeded"),
        ],
    );
}

#[tokio::test]
async fn managed_voice_actor_stop_joins_and_records_each_cancelled_stage() {
    for (gate, expected) in [
        ("transcription", vec![("voice_recognition", "cancelled")]),
        (
            "generation",
            vec![
                ("voice_recognition", "succeeded"),
                ("voice_generation", "cancelled"),
            ],
        ),
        (
            "synthesis",
            vec![
                ("voice_recognition", "succeeded"),
                ("voice_generation", "succeeded"),
                ("voice_synthesis", "cancelled"),
            ],
        ),
    ] {
        let mut fixture = Fixture::with_gate("unused", gate).await;
        let (writer, events) = recording(&fixture);
        fixture.utterance(1).await;
        fixture.expect(gate).await;
        fixture.stop().await;
        assert_stages(&recorded(writer, events).await, &expected);
    }
}

#[tokio::test]
async fn managed_voice_recognition_deadline_records_timeout_after_join() {
    let mut fixture = Fixture::with_gate("unused", "transcription").await;
    let (writer, events) = recording(&fixture);
    fixture.utterance(1).await;
    fixture.expect("transcription").await;
    tokio::time::timeout(Duration::from_secs(13), async {
        while fixture
            .runtime
            .media_enabled(fixture.runtime.current_epoch())
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    fixture.stop().await;
    assert_stages(
        &recorded(writer, events).await,
        &[("voice_recognition", "timed_out")],
    );
}

async fn failed_stage(transcript: &'static str, fault: &'static str, expected: &[(&str, &str)]) {
    let mut fixture = Fixture::with_transcripts(transcript, "unused", fault, None).await;
    let (writer, events) = recording(&fixture);
    fixture.utterance(1).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        while !fixture.runtime.snapshot().await.status.contains("failed") {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    assert!(fixture.playback.lock().await.is_none());
    fixture.stop().await;
    assert_stages(&recorded(writer, events).await, expected);
}

#[tokio::test]
async fn managed_voice_recognition_failure_is_content_free() {
    failed_stage("", "none", &[("voice_recognition", "failed")]).await;
}

#[tokio::test]
async fn managed_voice_generation_failure_has_no_synthesis() {
    failed_stage(
        "Abby, say hello.",
        "generation_failure",
        &[
            ("voice_recognition", "succeeded"),
            ("voice_generation", "failed"),
        ],
    )
    .await;
}

#[tokio::test]
async fn managed_voice_synthesis_failure_after_generation_is_content_free() {
    failed_stage(
        "Abby, say hello.",
        "synthesis_failure",
        &[
            ("voice_recognition", "succeeded"),
            ("voice_generation", "succeeded"),
            ("voice_synthesis", "failed"),
        ],
    )
    .await;
}

#[tokio::test]
async fn managed_voice_operational_synthesis_failure_has_no_generation() {
    failed_stage(
        "Abby, are you listening?",
        "synthesis_failure",
        &[
            ("voice_recognition", "succeeded"),
            ("voice_synthesis", "failed"),
        ],
    )
    .await;
}

#[tokio::test]
async fn uncovered_arrival_stops_speaking_before_later_frames() {
    let mut fixture = Fixture::with_gate("unused", "none").await;
    fixture.utterance(1).await;
    fixture.expect_playback(true).await;
    assert_eq!(fixture.runtime.snapshot().await.phase, VoicePhase::Speaking);
    while fixture.events.try_recv().is_ok() {}
    let epoch = fixture.runtime.current_epoch();
    assert_eq!(
        fixture.runtime.revoke_for_unattested_participant(3),
        Some(epoch)
    );
    assert!(!fixture.runtime.media_enabled(epoch));
    fixture.utterance(3).await;
    // Observe the actor consuming all later frames with its media gate shut,
    // rather than cancelling it before it has a chance to process the queue.
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.input.capacity() != 64 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.runtime.snapshot().await.barge_ins, 0);
    assert_eq!(
        fixture.events.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    );
    assert!(!fixture.runtime.media_enabled(epoch));
    fixture.stop().await;
}

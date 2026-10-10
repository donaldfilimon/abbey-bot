//! Regression coverage through the actual local actor and Songbird track slot.
use super::*;

async fn speaking_fixture() -> Fixture {
    let mut fixture = Fixture::with_gate("unused", "none").await;
    fixture.utterance(1).await;
    fixture.expect_playback(true).await;
    fixture
}

async fn consume_frames(fixture: &Fixture) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.input.capacity() != 64 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("actor did not consume the queued input");
}

#[tokio::test]
async fn short_noise_preserves_audible_reply() {
    let mut fixture = speaking_fixture().await;
    let original = fixture.playback.lock().await.as_ref().unwrap().uuid();

    // This starts a segment, but its 40 ms of energy is rejected as too short
    // to transcribe. It must not destroy the answer already being spoken.
    fixture.frames(2, 2, true).await;
    fixture.frames(2, 25, false).await;
    consume_frames(&fixture).await;

    assert_eq!(
        fixture
            .playback
            .lock()
            .await
            .as_ref()
            .map(|track| track.uuid()),
        Some(original),
        "a rejected noise segment cancelled the audible answer"
    );
    assert_eq!(fixture.runtime.snapshot().await.barge_ins, 0);
    assert_eq!(fixture.runtime.snapshot().await.phase, VoicePhase::Speaking);

    fixture
        ._lifecycle
        .send(SessionEvent::PlaybackTerminated {
            turn: 1,
            termination: PlaybackTermination::Natural,
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.runtime.snapshot().await.completed_turns != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the reply could not complete after short noise");
    assert_eq!(fixture.runtime.snapshot().await.barge_ins, 0);
    fixture.stop().await;
}

#[tokio::test]
async fn sustained_speech_interrupts_even_after_a_noise_started_the_segment() {
    let mut fixture = speaking_fixture().await;
    let original = fixture.playback.lock().await.as_ref().unwrap().uuid();
    fixture.frames(2, 2, true).await;
    fixture.frames(2, 6, false).await;
    consume_frames(&fixture).await;
    assert_eq!(
        fixture
            .playback
            .lock()
            .await
            .as_ref()
            .map(|track| track.uuid()),
        Some(original)
    );

    // No second SpeechStarted event occurs: this is inside the existing
    // segment's 500 ms silence tail. Confirmation must observe current frames.
    fixture.frames(2, 15, true).await;
    fixture.frames(2, 1, false).await;
    consume_frames(&fixture).await;
    assert!(fixture.playback.lock().await.is_none());
    let snapshot = fixture.runtime.snapshot().await;
    assert_eq!(snapshot.barge_ins, 1);
    assert_eq!(snapshot.completed_turns, 0);
    assert_eq!(snapshot.phase, VoicePhase::Listening);
    fixture.stop().await;
}

#[tokio::test]
async fn brief_pauses_within_real_speech_still_interrupt_playback() {
    let mut fixture = speaking_fixture().await;
    fixture.frames(2, 7, true).await;
    fixture.frames(2, 5, false).await;
    fixture.frames(2, 8, true).await;
    fixture.frames(2, 1, false).await;
    consume_frames(&fixture).await;
    assert!(fixture.playback.lock().await.is_none());
    assert_eq!(fixture.runtime.snapshot().await.barge_ins, 1);
    fixture.stop().await;
}

//! Verification runs: redaction, arming rules, final leave, linearized pauses.
use super::*;

#[tokio::test]
async fn redacted_verification_spans_activation_pause_resume_and_final_leave() {
    let runtime = runtime();
    let run = runtime.begin_verification().expect("arm verification");
    assert_eq!(run.run, 1);
    assert!(runtime.verification_active());

    let first_start = runtime.reserve_start();
    let first_epoch = runtime
        .begin(HashSet::from([
            900_000_000_000_000_001,
            900_000_000_000_000_002,
        ]))
        .await;
    assert!(
        runtime
            .activate_verified(
                first_epoch,
                first_start,
                "active",
                VerificationActivation {
                    manager_authorized: true,
                    caller_present: true,
                    participant_count: 2,
                    resumed: false,
                },
            )
            .await
    );
    runtime.note_verification_decoded_receive();
    runtime.note_verification_stt_completion(first_epoch);
    runtime.note_completed_turn();
    runtime.note_barge_in();
    runtime.note_verification_barge_in_cancellation();

    let pause = runtime
        .begin_participant_pause_epoch_for_consent(
            first_epoch,
            HashSet::from([
                900_000_000_000_000_001,
                900_000_000_000_000_002,
                900_000_000_000_000_003,
            ]),
            "new participant",
        )
        .await
        .expect("active epoch pauses");
    pause.finish().await;

    let resumed_start = runtime.reserve_start();
    let resumed_epoch = runtime
        .begin(HashSet::from([
            900_000_000_000_000_001,
            900_000_000_000_000_002,
            900_000_000_000_000_003,
        ]))
        .await;
    assert!(
        runtime
            .activate_verified(
                resumed_epoch,
                resumed_start,
                "resumed",
                VerificationActivation {
                    manager_authorized: true,
                    caller_present: true,
                    participant_count: 3,
                    resumed: true,
                },
            )
            .await
    );
    runtime.disconnect("left").await;
    assert!(runtime.note_verification_final_leave(run.run));

    let verification = runtime
        .verification_snapshot()
        .expect("completed verification remains reportable");
    assert_eq!(verification.status, VerificationRunStatus::Complete);
    assert_eq!(verification.observed_checks(), 8);
    assert_eq!(verification.activations, 2);
    assert_eq!(verification.first_participant_count, Some(2));
    assert_eq!(verification.latest_participant_count, Some(3));
    assert_ne!(
        verification.first_consent_epoch,
        verification.latest_consent_epoch
    );
    assert!(!runtime.verification_active());

    let report = runtime.verification_report();
    assert!(report.contains("observed: 8/8"));
    assert!(report.contains("counters only; no audio, user or message IDs, transcripts"));
    assert!(report.contains("Manual witness still required"));
    assert!(report.contains("separate from local/source test evidence"));
    assert!(!report.contains("900000000000000001"));
    assert!(!report.contains("new participant"));
    assert!(report.chars().count() < 2_000);
}

#[tokio::test]
async fn verification_ignores_stale_stt_and_requires_final_leave_before_rearming() {
    let runtime = runtime();
    let run = runtime.begin_verification().expect("arm verification");
    assert!(runtime.begin_verification().is_err());

    let start = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::from([7])).await;
    assert!(
        runtime
            .activate_verified(
                epoch,
                start,
                "active",
                VerificationActivation {
                    manager_authorized: true,
                    caller_present: true,
                    participant_count: 1,
                    resumed: false,
                },
            )
            .await
    );
    runtime.disconnect("closed gate").await;
    runtime.note_verification_stt_completion(epoch);
    assert_eq!(
        runtime
            .verification_snapshot()
            .expect("run")
            .stt_completions,
        0
    );

    assert!(runtime.note_verification_final_leave(run.run));
    let replacement = runtime.begin_verification().expect("replace completed run");
    assert_eq!(replacement.run, 2);
    assert_eq!(replacement.observed_checks(), 0);
}

#[test]
fn a_stale_leave_token_cannot_complete_a_replacement_run() {
    let runtime = runtime();
    let first = runtime.begin_verification().expect("first run");
    assert_eq!(runtime.verification_run_token(), Some(first.run));
    assert!(runtime.note_verification_final_leave(first.run));

    let replacement = runtime.begin_verification().expect("replacement run");
    assert_ne!(first.run, replacement.run);
    assert!(
        !runtime.note_verification_final_leave(first.run),
        "an older leave must not complete a later verifier"
    );
    let snapshot = runtime
        .verification_snapshot()
        .expect("replacement remains");
    assert_eq!(snapshot.run, replacement.run);
    assert_eq!(snapshot.status, VerificationRunStatus::Armed);
    assert!(!snapshot.final_leave);
}

#[tokio::test]
async fn verification_cannot_arm_during_a_pending_or_open_media_epoch() {
    let runtime = runtime();
    let pending = runtime.reserve_start();
    assert!(runtime.begin_verification().is_err());
    runtime.finish_start_attempt(pending);

    let start = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::from([7])).await;
    assert!(runtime.activate(epoch, start, "active").await);
    assert!(runtime.begin_verification().is_err());
    runtime.disconnect("left").await;
    assert!(runtime.begin_verification().is_ok());
}

#[tokio::test]
async fn verified_activation_and_immediate_participant_pause_are_linearized() {
    let runtime = runtime();
    runtime.begin_verification().expect("arm verification");
    let start = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::from([7])).await;
    assert!(
        runtime
            .activate_verified(
                epoch,
                start,
                "active",
                VerificationActivation {
                    manager_authorized: true,
                    caller_present: true,
                    participant_count: 1,
                    resumed: false,
                },
            )
            .await
    );
    let pause = runtime
        .begin_participant_pause_epoch_for_consent(
            epoch,
            HashSet::from([7, 8]),
            "participant changed",
        )
        .await
        .expect("the exact active epoch pauses");
    pause.finish().await;

    let run = runtime.verification_snapshot().expect("run");
    assert_eq!(run.activations, 1);
    assert_eq!(run.participant_change_pauses, 1);
    assert_eq!(run.latest_participant_count, Some(2));
}

#[tokio::test]
async fn incomplete_authorization_cannot_satisfy_the_acceptance_check() {
    let runtime = runtime();
    runtime.begin_verification().expect("arm verification");
    let start = runtime.reserve_start();
    let epoch = runtime.begin(HashSet::from([7, 8])).await;
    assert!(
        runtime
            .activate_verified(
                epoch,
                start,
                "active",
                VerificationActivation {
                    manager_authorized: false,
                    caller_present: true,
                    participant_count: 2,
                    resumed: false,
                },
            )
            .await
    );
    let run = runtime.verification_snapshot().expect("run");
    assert_eq!(run.observed_checks(), 0);
    assert!(
        runtime
            .verification_report()
            .contains("Authorization: pending")
    );
}

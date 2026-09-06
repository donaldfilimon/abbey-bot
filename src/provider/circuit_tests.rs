use super::*;
const OUTCOMES: [ProviderFailureKind; 17] = [
    ProviderFailureKind::Success,
    ProviderFailureKind::TransportUnavailable,
    ProviderFailureKind::Timeout,
    ProviderFailureKind::Http5xx,
    ProviderFailureKind::RateLimited,
    ProviderFailureKind::Authentication,
    ProviderFailureKind::Authorization,
    ProviderFailureKind::Configuration,
    ProviderFailureKind::ExecutableIdentity,
    ProviderFailureKind::ModelIdentity,
    ProviderFailureKind::SandboxIdentity,
    ProviderFailureKind::ToolSchema,
    ProviderFailureKind::ResponseSchema,
    ProviderFailureKind::ProtocolDrift,
    ProviderFailureKind::Cancelled,
    ProviderFailureKind::InvalidRequest,
    ProviderFailureKind::Busy,
];
fn finish(c: &mut Circuit, kind: ProviderFailureKind, retry: RetryAfter, now: u64) {
    let permit = c.reserve(now).unwrap();
    assert_eq!(
        c.complete(permit, kind, retry, now),
        Some(retry.classify(kind))
    );
}
fn open(c: &mut Circuit, now: u64) {
    finish(c, ProviderFailureKind::Timeout, RetryAfter::Absent, now);
    finish(c, ProviderFailureKind::Timeout, RetryAfter::Absent, now);
    finish(c, ProviderFailureKind::Timeout, RetryAfter::Absent, now);
}
#[test]
fn three_failures_inclusive_rolling_window_success_prunes_but_does_not_reset() {
    let mut c = Circuit::new();
    assert_eq!(c.snapshot().phase, CircuitPhase::Closed);
    finish(&mut c, ProviderFailureKind::Timeout, RetryAfter::Absent, 0);
    finish(
        &mut c,
        ProviderFailureKind::Success,
        RetryAfter::Absent,
        100000,
    );
    finish(
        &mut c,
        ProviderFailureKind::Timeout,
        RetryAfter::Absent,
        200000,
    );
    finish(
        &mut c,
        ProviderFailureKind::Timeout,
        RetryAfter::Absent,
        300000,
    );
    assert_eq!(c.snapshot().open_until_ms, Some(360000));
    assert!(c.recent.is_empty());
    let mut c = Circuit::new();
    finish(&mut c, ProviderFailureKind::Timeout, RetryAfter::Absent, 0);
    finish(&mut c, ProviderFailureKind::Timeout, RetryAfter::Absent, 1);
    finish(
        &mut c,
        ProviderFailureKind::Success,
        RetryAfter::Absent,
        300001,
    );
    assert_eq!(c.recent.len(), 1);
    finish(
        &mut c,
        ProviderFailureKind::Timeout,
        RetryAfter::Absent,
        300001,
    );
    assert_eq!(c.snapshot().phase, CircuitPhase::Closed);
}
#[test]
fn exact_single_probe_reservation_and_five_fifteen_minute_escalation_cap() {
    let mut c = Circuit::new();
    open(&mut c, 0);
    assert_eq!(c.reserve(59999), Err(CircuitUnavailable::Open));
    for (now, deadline, level) in [
        (60000, 360000, 1),
        (360000, 1260000, 2),
        (1260000, 2160000, 2),
    ] {
        let p = c.reserve(now).unwrap();
        assert_eq!(c.snapshot().phase, CircuitPhase::HalfOpen);
        assert_eq!(c.reserve(now), Err(CircuitUnavailable::Busy));
        c.complete(p, ProviderFailureKind::Timeout, RetryAfter::Absent, now);
        assert_eq!(c.snapshot().open_until_ms, Some(deadline));
        assert_eq!(c.snapshot().opening_level, level);
    }
    finish(
        &mut c,
        ProviderFailureKind::Success,
        RetryAfter::Absent,
        2160000,
    );
    assert_eq!(c.snapshot(), Circuit::new().snapshot());
}
#[test]
fn retry_after_strict_boundaries_and_compatibility_for_all_outcomes() {
    for raw in [
        "0", "0.999", "900.001", "-1", "NaN", "inf", "1e999", "garbage", "",
    ] {
        assert_eq!(
            RetryAfter::from_seconds(Some(raw)),
            RetryAfter::Invalid,
            "{raw}"
        );
    }
    assert_eq!(RetryAfter::from_seconds(None), RetryAfter::Absent);
    for raw in ["1", "900"] {
        assert!(matches!(
            RetryAfter::from_seconds(Some(raw)),
            RetryAfter::Valid(_)
        ));
    }
    for kind in OUTCOMES {
        assert_eq!(RetryAfter::Absent.classify(kind), kind);
        assert_eq!(
            RetryAfter::Invalid.classify(kind),
            ProviderFailureKind::ProtocolDrift
        );
        assert_eq!(
            RetryAfter::Valid(Duration::ZERO).classify(kind),
            ProviderFailureKind::ProtocolDrift
        );
        let valid = RetryAfter::Valid(Duration::from_secs(1));
        assert_eq!(
            valid.classify(kind),
            if matches!(
                kind,
                ProviderFailureKind::Http5xx | ProviderFailureKind::RateLimited
            ) {
                kind
            } else {
                ProviderFailureKind::ProtocolDrift
            }
        );
    }
}
#[test]
fn complete_phase_outcome_retry_table_and_immutable_blocked_identity() {
    for phase in [
        CircuitPhase::Closed,
        CircuitPhase::Open,
        CircuitPhase::HalfOpen,
        CircuitPhase::Blocked,
    ] {
        for kind in OUTCOMES {
            for retry in [
                RetryAfter::Absent,
                RetryAfter::Valid(Duration::from_secs(120)),
                RetryAfter::Invalid,
            ] {
                let mut c = Circuit::new();
                let held = c.reserve(0).unwrap();
                if phase != CircuitPhase::Closed {
                    open(&mut c, 0);
                }
                let permit = if phase == CircuitPhase::HalfOpen {
                    c.reserve(60000).unwrap()
                } else {
                    held
                };
                if phase == CircuitPhase::Blocked {
                    c.block(ProviderFailureKind::Authentication);
                }
                let before = c.snapshot();
                let before_recent = c.recent.clone();
                let result = c.complete(permit, kind, retry, 60000);
                let after = c.snapshot();
                if phase == CircuitPhase::Blocked {
                    assert_eq!(result, None);
                    assert_eq!(after, before);
                    continue;
                }
                let effective = retry.classify(kind);
                assert_eq!(result, Some(effective));
                if effective.is_blocked() {
                    assert_eq!(after.phase, CircuitPhase::Blocked);
                    assert_eq!(after.reason, Some(effective));
                    assert!(c.recent.is_empty());
                    assert!(!after.probe_reserved);
                    continue;
                }
                if effective.is_neutral() {
                    assert_eq!(after.phase, before.phase);
                    assert_eq!(after.opening_level, before.opening_level);
                    assert_eq!(after.open_until_ms, before.open_until_ms);
                    assert_eq!(c.recent, before_recent);
                    assert!(!after.probe_reserved);
                    continue;
                }
                match phase {
                    CircuitPhase::Closed if effective == ProviderFailureKind::Success => {
                        assert_eq!(after, before);
                    }
                    CircuitPhase::Closed if retry == RetryAfter::Absent => {
                        assert_eq!(after.phase, CircuitPhase::Closed);
                        assert_eq!(c.recent.len(), 1);
                    }
                    CircuitPhase::Closed => {
                        assert_eq!(after.open_until_ms, Some(180000));
                        assert_eq!(after.opening_level, 0);
                        assert!(c.recent.is_empty());
                    }
                    CircuitPhase::Open => {
                        assert_eq!(after.phase, CircuitPhase::Open);
                        assert_eq!(
                            after.open_until_ms,
                            Some(if retry == RetryAfter::Absent {
                                60000
                            } else {
                                180000
                            })
                        );
                        assert_eq!(after.opening_level, 0);
                    }
                    CircuitPhase::HalfOpen if effective == ProviderFailureKind::Success => {
                        assert_eq!(after, Circuit::new().snapshot());
                    }
                    CircuitPhase::HalfOpen => {
                        assert_eq!(after.open_until_ms, Some(360000));
                        assert_eq!(after.opening_level, 1);
                        assert!(!after.probe_reserved);
                    }
                    CircuitPhase::Blocked => unreachable!(),
                }
            }
        }
    }
}
#[test]
fn delayed_inflight_work_never_consumes_or_closes_another_reserved_probe() {
    for kind in OUTCOMES {
        let mut c = Circuit::new();
        let old = c.reserve(0).unwrap();
        open(&mut c, 0);
        let probe = c.reserve(60000).unwrap();
        let before = c.snapshot();
        c.complete(old, kind, RetryAfter::Absent, 60001);
        if kind.is_blocked() {
            assert_eq!(c.snapshot().phase, CircuitPhase::Blocked);
            assert_eq!(
                c.complete(
                    probe,
                    ProviderFailureKind::Success,
                    RetryAfter::Absent,
                    60002
                ),
                None
            );
        } else {
            assert_eq!(c.snapshot(), before);
            assert_eq!(c.reserve(60001), Err(CircuitUnavailable::Busy));
            c.complete(
                probe,
                ProviderFailureKind::Success,
                RetryAfter::Absent,
                60002,
            );
            assert_eq!(c.snapshot().phase, CircuitPhase::Closed);
        }
    }
}
#[test]
fn retry_after_can_open_early_extend_never_shorten_and_saturates_clock_only() {
    let mut c = Circuit::new();
    let old = c.reserve(0).unwrap();
    finish(
        &mut c,
        ProviderFailureKind::RateLimited,
        RetryAfter::Valid(Duration::from_secs(1)),
        0,
    );
    assert_eq!(c.snapshot().open_until_ms, Some(1000));
    c.complete(
        old,
        ProviderFailureKind::Http5xx,
        RetryAfter::Valid(Duration::from_secs(900)),
        1,
    );
    assert_eq!(c.snapshot().open_until_ms, Some(900001));
    let p = c.reserve(900001).unwrap();
    c.complete(
        p,
        ProviderFailureKind::Http5xx,
        RetryAfter::Valid(Duration::from_secs(900)),
        900001,
    );
    assert_eq!(c.snapshot().open_until_ms, Some(1800001));
    let mut c = Circuit::new();
    finish(&mut c, ProviderFailureKind::Timeout, RetryAfter::Absent, 0);
    finish(&mut c, ProviderFailureKind::Timeout, RetryAfter::Absent, 0);
    finish(
        &mut c,
        ProviderFailureKind::Http5xx,
        RetryAfter::Valid(Duration::from_secs(1)),
        0,
    );
    assert_eq!(c.snapshot().open_until_ms, Some(60000));
    let mut c = Circuit::new();
    open(&mut c, u64::MAX - 10);
    assert_eq!(c.snapshot().open_until_ms, Some(u64::MAX));
    assert_eq!(c.reserve(u64::MAX - 1), Err(CircuitUnavailable::Open));
    assert!(c.reserve(u64::MAX).is_ok());
}
#[test]
fn neutral_probe_release_allows_exactly_one_replacement() {
    for kind in [
        ProviderFailureKind::Cancelled,
        ProviderFailureKind::InvalidRequest,
        ProviderFailureKind::Busy,
    ] {
        let mut c = Circuit::new();
        open(&mut c, 0);
        finish(&mut c, kind, RetryAfter::Absent, 60000);
        assert_eq!(c.snapshot().phase, CircuitPhase::HalfOpen);
        assert!(!c.snapshot().probe_reserved);
        assert!(c.reserve(60001).is_ok());
        assert_eq!(c.reserve(60001), Err(CircuitUnavailable::Busy));
    }
}

#[test]
fn numeric_delay_rejects_out_of_range_before_rounding_and_deadline_never_rounds_down() {
    for raw in ["0.9999999999", "900.0000000001"] {
        assert_eq!(RetryAfter::from_seconds(Some(raw)), RetryAfter::Invalid);
    }
    let delay = RetryAfter::Valid(Duration::from_nanos(1_000_000_001));
    let mut c = Circuit::new();
    finish(&mut c, ProviderFailureKind::RateLimited, delay, 0);
    assert_eq!(c.snapshot().open_until_ms, Some(1001));
}
#[test]
fn later_inflight_short_retry_never_reduces_current_open_deadline() {
    let mut c = Circuit::new();
    let old = c.reserve(0).unwrap();
    open(&mut c, 0);
    c.complete(
        old,
        ProviderFailureKind::RateLimited,
        RetryAfter::Valid(Duration::from_secs(1)),
        1,
    );
    assert_eq!(c.snapshot().open_until_ms, Some(60000));
}
#[test]
fn invalid_header_blocks_even_the_reserved_neutral_probe_and_releases_it() {
    let mut c = Circuit::new();
    open(&mut c, 0);
    let probe = c.reserve(60000).unwrap();
    assert_eq!(
        c.complete(
            probe,
            ProviderFailureKind::Cancelled,
            RetryAfter::Invalid,
            60000
        ),
        Some(ProviderFailureKind::ProtocolDrift)
    );
    assert_eq!(c.snapshot().phase, CircuitPhase::Blocked);
    assert!(!c.snapshot().probe_reserved);
    assert!(c.recent.is_empty());
}

#[test]
fn closed_neutral_does_not_even_prune_expired_failure_history() {
    for kind in [
        ProviderFailureKind::Cancelled,
        ProviderFailureKind::InvalidRequest,
        ProviderFailureKind::Busy,
    ] {
        let mut c = Circuit::new();
        finish(&mut c, ProviderFailureKind::Timeout, RetryAfter::Absent, 0);
        let before = c.recent.clone();
        let snapshot = c.snapshot();
        finish(&mut c, kind, RetryAfter::Absent, 300001);
        assert_eq!(c.recent, before);
        assert_eq!(c.snapshot(), snapshot);
    }
}

//! Shared pure UTC validation and deterministic IANA recurrence.
use crate::work::WorkError;
use chrono::{DateTime, Duration, LocalResult, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
pub(crate) fn utc(seconds: u64) -> Result<DateTime<Utc>, WorkError> {
    let at = DateTime::from_timestamp(i64::try_from(seconds).map_err(|_| WorkError::Invalid)?, 0)
        .ok_or(WorkError::Invalid)?;
    // Leave room for any IANA offset and a skipped calendar day. Chrono's
    // local date accessors panic if a valid UTC instant overflows locally.
    at.checked_add_signed(Duration::days(2))
        .ok_or(WorkError::Invalid)?;
    at.checked_sub_signed(Duration::days(2))
        .ok_or(WorkError::Invalid)?;
    Ok(at)
}

/// Calendar recurrence: choose the first occurrence in a fall-back overlap;
/// advance through a spring-forward gap to the first valid local minute.
pub(crate) fn local_hour(tz: Tz, day: NaiveDate, hour: u8) -> Result<DateTime<Utc>, WorkError> {
    let mut local = day
        .and_hms_opt(u32::from(hour), 0, 0)
        .ok_or(WorkError::Invalid)?;
    for _ in 0..=1_440 {
        match tz.from_local_datetime(&local) {
            LocalResult::Single(time) => return Ok(time.with_timezone(&Utc)),
            LocalResult::Ambiguous(a, b) => return Ok(a.min(b).with_timezone(&Utc)),
            LocalResult::None => {
                local = local
                    .checked_add_signed(Duration::minutes(1))
                    .ok_or(WorkError::Invalid)?;
            }
        }
    }
    Err(WorkError::Invalid)
}

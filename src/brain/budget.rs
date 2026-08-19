//! Per-guild budget for unsolicited actions — a refilling token bucket.
//!
//! The cooldown in `guild.rs` stops bursts (one reply per channel per N
//! seconds); this stops volume (at most `capacity` unsolicited actions per
//! guild per hour, refilling continuously). Pure: the clock is injected.

use std::collections::HashMap;

/// One bucket per scoped guild id.
#[derive(Debug, Default)]
pub struct Budget {
    buckets: HashMap<String, Bucket>,
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f32,
    last: u64,
}

impl Budget {
    /// Spend one unsolicited action for `key` if the bucket has a whole token.
    pub fn try_take(&mut self, key: &str, capacity_per_hour: u32, now: u64) -> bool {
        if capacity_per_hour == 0 {
            return false;
        }
        let bucket = self.refilled(key, capacity_per_hour, now);
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Tokens available right now, for `/admin brain` and `/stats`.
    pub fn tokens_left(&self, key: &str, capacity_per_hour: u32, now: u64) -> f32 {
        let capacity = capacity_per_hour as f32;
        match self.buckets.get(key) {
            None => capacity,
            Some(b) => refill(*b, capacity, now).tokens,
        }
    }

    fn refilled(&mut self, key: &str, capacity_per_hour: u32, now: u64) -> &mut Bucket {
        let capacity = capacity_per_hour as f32;
        let entry = self.buckets.entry(key.to_owned()).or_insert(Bucket {
            tokens: capacity,
            last: now,
        });
        *entry = refill(*entry, capacity, now);
        entry
    }
}

/// Advance a bucket to `now`: add `capacity/3600` per elapsed second, cap at
/// `capacity`. A clock that went backwards adds nothing.
fn refill(mut b: Bucket, capacity: f32, now: u64) -> Bucket {
    let elapsed = now.saturating_sub(b.last) as f32;
    b.tokens = (b.tokens + elapsed * capacity / 3600.0).min(capacity);
    b.last = now.max(b.last);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_guild_starts_full_and_spends_down_to_empty() {
        let mut b = Budget::default();
        for _ in 0..6 {
            assert!(b.try_take("discord:g", 6, 1_000));
        }
        assert!(
            !b.try_take("discord:g", 6, 1_000),
            "seventh in the same second is refused"
        );
        assert!(b.tokens_left("discord:g", 6, 1_000) < 1.0);
    }

    #[test]
    fn tokens_refill_at_capacity_per_hour_and_cap_at_capacity() {
        let mut b = Budget::default();
        for _ in 0..6 {
            assert!(b.try_take("discord:g", 6, 0));
        }
        // 6/h = one token per 600 s.
        assert!(!b.try_take("discord:g", 6, 599));
        assert!(b.try_take("discord:g", 6, 600));
        // A long idle period never overfills.
        let left = b.tokens_left("discord:g", 6, 1_000_000);
        assert!((left - 6.0).abs() < 1e-3, "{left}");
    }

    #[test]
    fn guilds_do_not_share_a_bucket() {
        let mut b = Budget::default();
        for _ in 0..6 {
            assert!(b.try_take("discord:a", 6, 0));
        }
        assert!(b.try_take("discord:b", 6, 0), "guild b is untouched");
    }

    #[test]
    fn zero_capacity_never_permits_and_time_going_backwards_is_harmless() {
        let mut b = Budget::default();
        assert!(!b.try_take("discord:g", 0, 10));
        assert!(b.try_take("discord:g", 6, 100));
        assert!(
            b.try_take("discord:g", 6, 50),
            "a clock step back does not panic or refund"
        );
    }
}

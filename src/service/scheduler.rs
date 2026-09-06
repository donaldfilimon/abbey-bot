//! One timer set; missed work is skipped instead of replayed in a burst.
use std::time::Duration;
use tokio::time::{Instant, Interval, MissedTickBehavior};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tick {
    Learn,
    Flush,
    Persist,
    Settle,
    Summary,
}
pub struct Schedule {
    learn: Interval,
    flush: Interval,
    persist: Interval,
    settle: Interval,
    summary: Interval,
}
impl Schedule {
    pub fn new() -> Self {
        fn timer(period: Duration) -> Interval {
            let mut timer = tokio::time::interval_at(Instant::now() + period, period);
            timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
            timer
        }
        Self {
            learn: timer(crate::runtime::LEARN_EVERY),
            flush: timer(crate::runtime::FLUSH_EVERY),
            persist: timer(crate::runtime::PERSIST_EVERY),
            settle: timer(crate::runtime::SETTLE_EVERY),
            summary: timer(crate::runtime::SUMMARIZE_EVERY),
        }
    }
    pub async fn next(&mut self) -> Tick {
        tokio::select! {
            _ = self.learn.tick() => Tick::Learn,
            _ = self.flush.tick() => Tick::Flush,
            _ = self.persist.tick() => Tick::Persist,
            _ = self.settle.tick() => Tick::Settle,
            _ = self.summary.tick() => Tick::Summary,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn no_initial_work_and_missed_periods_never_burst() {
        use futures_util::FutureExt;
        let mut schedule = Schedule::new();
        assert!(schedule.next().now_or_never().is_none());
        tokio::time::advance(Duration::from_secs(30)).await;
        let first = schedule.next().await;
        let second = schedule.next().await;
        assert!(matches!(
            (first, second),
            (Tick::Learn, Tick::Settle) | (Tick::Settle, Tick::Learn)
        ));
        assert!(schedule.next().now_or_never().is_none());
        tokio::time::advance(Duration::from_secs(600)).await;
        let mut ticks = Vec::new();
        for _ in 0..5 {
            ticks.push(schedule.next().await);
        }
        for expected in [
            Tick::Learn,
            Tick::Flush,
            Tick::Persist,
            Tick::Settle,
            Tick::Summary,
        ] {
            assert_eq!(ticks.iter().filter(|tick| **tick == expected).count(), 1);
        }
        assert!(schedule.next().now_or_never().is_none());
    }
}

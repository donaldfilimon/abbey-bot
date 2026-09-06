//! Sticky wakeup for failures in separately retained resource owners.
use std::sync::atomic::{AtomicBool, Ordering};
#[derive(Default)]
pub struct FailureSignal {
    failed: AtomicBool,
    wake: tokio::sync::Notify,
}
impl FailureSignal {
    pub fn trigger(&self) {
        self.failed.store(true, Ordering::Release);
        self.wake.notify_one();
    }
    pub async fn notified(&self) {
        if !self.failed.load(Ordering::Acquire) {
            self.wake.notified().await;
        }
    }
}

//! Retained managed readiness heartbeat covering startup, operation and draining.
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
pub struct RefreshOwner {
    cancel: CancellationToken,
    handle: tokio::task::JoinHandle<()>,
}
impl RefreshOwner {
    pub fn start(
        status: Arc<super::status::ManagedStatus>,
        fatal: Arc<crate::managed_service::ManagedFatalSignal>,
    ) -> Self {
        Self::start_with(
            move || status.refresh().map(|_| ()),
            Arc::new(move || fatal.trigger()),
        )
    }
    fn start_with(
        refresh: impl Fn() -> Result<(), crate::observability::ManagedFailure> + Send + 'static,
        failed: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let cancel = CancellationToken::new();
        let owned = cancel.clone();
        let handle = tokio::spawn(async move {
            struct Guard(CancellationToken, Arc<dyn Fn() + Send + Sync>);
            impl Drop for Guard {
                fn drop(&mut self) {
                    if !self.0.is_cancelled() {
                        (self.1)();
                    }
                }
            }
            let _guard = Guard(owned.clone(), failed);
            let mut interval = tokio::time::interval_at(
                tokio::time::Instant::now() + crate::readiness::REFRESH_INTERVAL,
                crate::readiness::REFRESH_INTERVAL,
            );
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    biased;
                    () = owned.cancelled() => return,
                    _ = interval.tick() => if refresh().is_err() { return; },
                }
            }
        });
        Self { cancel, handle }
    }
    pub fn stop(&self) {
        self.cancel.cancel();
    }
    pub async fn joined(&mut self) -> Result<(), tokio::task::JoinError> {
        (&mut self.handle).await
    }
}
impl Drop for RefreshOwner {
    fn drop(&mut self) {
        self.stop();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[tokio::test(start_paused = true)]
    async fn slow_startup_does_not_suspend_readiness_refresh() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let mut owner = RefreshOwner::start_with(
            move || {
                counted.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            Arc::new(|| panic!("unexpected refresh exit")),
        );
        tokio::task::yield_now().await;
        // The root can remain suspended in a controlled startup operation.
        for expected in 1..=3 {
            tokio::time::advance(crate::readiness::REFRESH_INTERVAL).await;
            tokio::task::yield_now().await;
            assert_eq!(calls.load(Ordering::SeqCst), expected);
        }
        owner.stop();
        owner.joined().await.unwrap();
        tokio::time::advance(crate::readiness::REFRESH_INTERVAL).await;
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }
}

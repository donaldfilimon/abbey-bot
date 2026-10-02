//! Retained filesystem transactions. Dropping the receiver never drops the owner.
use super::AppState;
impl AppState {
    pub(crate) async fn community_filesystem<T, F>(&self, work: F) -> Result<T, &'static str>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, &'static str> + Send + 'static,
    {
        self.service
            .get()
            .ok_or("filesystem ownership unavailable")?
            .blocking_result(crate::service::OperationKind::CommunityFilesystem, work)
            .map_err(|_| "filesystem admission closed")?
            .await
            .map_err(|_| "filesystem completion unavailable")?
    }
    pub(crate) async fn community_policy(
        &self,
        path: std::path::PathBuf,
    ) -> Result<(crate::community_ops::Policy, String), &'static str> {
        self.community_filesystem(move || crate::persist::community_ops::load_policy(&path))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{
        FreezeError, OperationKind, OwnedTaskKind, ServiceSupervisor, ShutdownReason, TaskExit,
    };
    use std::time::Duration;
    use tokio::time::Instant;

    #[tokio::test(flavor = "current_thread")]
    async fn blocked_filesystem_keeps_executor_responsive_and_owner_until_observed_join() {
        let state = AppState::in_memory();
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        assert!(state.service.set(supervisor.operations()).is_ok());
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let waiting_state = state.clone();
        let waiter = tokio::spawn(async move {
            waiting_state
                .community_filesystem(move || {
                    let _ = entered.send(());
                    blocked
                        .recv_timeout(Duration::from_secs(5))
                        .map_err(|_| "test safety timeout")?;
                    Ok(())
                })
                .await
        });
        started.await.unwrap();
        // This timer shares the single executor thread with the filesystem waiter.
        tokio::time::timeout(Duration::from_millis(100), async {
            tokio::time::sleep(Duration::from_millis(5)).await;
        })
        .await
        .unwrap();
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        supervisor.begin_draining(ShutdownReason::Signal, Instant::now());
        assert!(supervisor.operations().cancellation().is_cancelled());
        assert_eq!(supervisor.outstanding().len(), 1);
        assert_eq!(
            supervisor.outstanding()[0].kind,
            OwnedTaskKind::Operation(OperationKind::CommunityFilesystem)
        );
        supervisor.request_abort();
        assert_eq!(
            supervisor.try_freeze(true),
            Err(FreezeError::TasksNotJoined)
        );
        release.send(()).unwrap();
        let joined = supervisor.next_completion().await;
        assert_eq!(
            joined.kind,
            OwnedTaskKind::Operation(OperationKind::CommunityFilesystem)
        );
        assert_eq!(joined.exit, TaskExit::Returned);
        assert!(joined.abort_requested);
        assert!(supervisor.outstanding().is_empty());
        assert_eq!(supervisor.try_freeze(true), Ok(()));
        assert_eq!(
            state.community_filesystem(|| Ok(())).await,
            Err("filesystem admission closed")
        );
    }
}

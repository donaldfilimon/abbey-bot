//! Root-retained actor handles and a synchronous service admission boundary.
use super::*;
use crate::service::{OperationKind, OperationRegistry, TaskExit};
use std::future::Future;
use tokio::sync::oneshot;

pub enum VoiceTask {
    Supervised(oneshot::Receiver<()>),
    #[cfg(test)]
    Fixture(tokio::task::JoinHandle<()>),
}

impl VoiceTask {
    pub(super) async fn join(self) {
        match self {
            Self::Supervised(receive) => {
                let _ = receive.await;
            }
            #[cfg(test)]
            Self::Fixture(handle) => {
                let _ = handle.await;
            }
        }
    }
}

#[cfg(test)]
impl From<tokio::task::JoinHandle<()>> for VoiceTask {
    fn from(handle: tokio::task::JoinHandle<()>) -> Self {
        Self::Fixture(handle)
    }
}

impl VoiceRuntime {
    pub fn attach_telemetry(&self, events: crate::service::telemetry::TelemetryRequests) {
        let _ = self.telemetry.set(events);
    }
    pub fn attach_service(&self, registry: OperationRegistry) {
        self.consent.attach_service(registry.clone());
        let _ = self.service.set(registry);
    }

    pub fn accepting_work(&self) -> bool {
        !self.draining.load(Ordering::SeqCst)
            && self.service.get().is_none_or(OperationRegistry::is_running)
    }

    /// Called in the same first-trigger path as supervisor admission closure,
    /// before any asynchronous shutdown step can yield.
    pub fn begin_draining(&self) {
        {
            let _activation = self
                .activation_gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.draining.store(true, Ordering::SeqCst);
            let generation = self.start_generation.fetch_add(1, Ordering::SeqCst) + 1;
            self.pending_start_generation.store(0, Ordering::SeqCst);
            self.media_epoch.store(0, Ordering::SeqCst);
            self.start_changes.send_replace(generation);
            self.mark_inspect_media_revoked();
        }
        self.music.close();
    }

    pub fn spawn_actor<F>(
        &self,
        cancel: watch::Sender<bool>,
        actor: F,
    ) -> Result<VoiceTask, &'static str>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        if !self.accepting_work() {
            return Err("The voice service is stopping.");
        }
        let registry = self
            .service
            .get()
            .ok_or("Voice service ownership is unavailable.")?;
        let (send, receive) = oneshot::channel();
        registry
            .spawn_operation(OperationKind::Voice, move |token| async move {
                tokio::pin!(actor);
                tokio::select! {
                    biased;
                    _ = token.cancelled() => {
                        let _ = cancel.send(true);
                        actor.await;
                    }
                    () = &mut actor => {}
                }
                let _ = send.send(());
                TaskExit::Returned
            })
            .map_err(|_| "The voice service is stopping.")?;
        Ok(VoiceTask::Supervised(receive))
    }

    pub fn spawn_owned<F>(&self, work: F) -> Result<(), &'static str>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        if !self.accepting_work() {
            return Err("The voice service is stopping.");
        }
        self.service
            .get()
            .ok_or("Voice service ownership is unavailable.")?
            .spawn_operation(OperationKind::Voice, move |_| async move {
                work.await;
                TaskExit::Returned
            })
            .map(|_| ())
            .map_err(|_| "The voice service is stopping.")
    }

    pub fn spawn_result<T, F, Fut>(&self, work: F) -> Result<oneshot::Receiver<T>, &'static str>
    where
        T: Send + 'static,
        F: FnOnce(tokio_util::sync::CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        if !self.accepting_work() {
            return Err("The voice service is stopping.");
        }
        let (send, receive) = oneshot::channel();
        self.service
            .get()
            .ok_or("Voice service ownership is unavailable.")?
            .spawn_operation(OperationKind::Voice, move |token| async move {
                let result = work(token).await;
                let _ = send.send(result);
                TaskExit::Returned
            })
            .map_err(|_| "The voice service is stopping.")?;
        Ok(receive)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{ReapOutcome, ServiceSupervisor, ShutdownReason, StageBudget};
    use tokio::time::Instant;

    fn runtime() -> VoiceRuntime {
        VoiceRuntime::new(VoiceConfig::selected_only(
            1,
            2,
            VoiceBackendConfig::Disabled,
            true,
        ))
    }

    #[tokio::test]
    async fn cancelled_waiter_keeps_actor_and_nested_cleanup_owned() {
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let runtime = runtime();
        runtime.attach_service(supervisor.operations());
        let (cancel, mut cancelled) = watch::channel(false);
        let (entered, started) = oneshot::channel();
        let (release, wait) = oneshot::channel();
        let (closing, closed) = oneshot::channel();
        let receipt = runtime
            .spawn_actor(cancel, async move {
                let mut children = tokio::task::JoinSet::new();
                children.spawn(async move {
                    let _ = wait.await;
                });
                let _ = entered.send(());
                let _ = cancelled.changed().await;
                let _ = closing.send(());
                while children.join_next().await.is_some() {}
            })
            .unwrap();
        started.await.unwrap();
        drop(receipt);
        supervisor.begin_draining(ShutdownReason::Signal, Instant::now());
        runtime.begin_draining();
        supervisor.request_cancellation();
        closed.await.unwrap();
        let now = Instant::now();
        let report = supervisor
            .cancel_and_reap(StageBudget {
                deadline: now,
                abort_at: now,
            })
            .await;
        let retained = report.outcome == ReapOutcome::TimedOut && report.outstanding.len() == 1;
        release.send(()).unwrap();
        let completion = supervisor.next_completion().await;
        assert!(
            retained,
            "the outer voice owner must survive until its nested task joins"
        );
        assert_eq!(completion.exit, TaskExit::Returned);
        assert_eq!(supervisor.try_freeze(true), Ok(()));
    }

    #[tokio::test]
    async fn draining_permanently_closes_start_music_and_actor_admission() {
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let runtime = runtime();
        runtime.attach_service(supervisor.operations());
        let token = runtime.start_operation_token();
        let start = runtime.reserve_start_if_unchanged(token).unwrap();
        let music = runtime.music.begin(crate::player_control::Player::Spotify);
        runtime.begin_draining();
        assert!(!runtime.start_is_current(start));
        assert!(!runtime.music.current(music));
        assert_eq!(runtime.music.begin(crate::player_control::Player::Music), 0);
        assert!(
            runtime
                .reserve_start_if_unchanged(runtime.start_operation_token())
                .is_none()
        );
        assert_eq!(runtime.reserve_start(), 0);
        let (cancel, _) = watch::channel(false);
        assert!(runtime.spawn_actor(cancel, async {}).is_err());
        assert!(runtime.spawn_owned(async {}).is_err());
        assert!(runtime.spawn_result(|_| async {}).is_err());
        supervisor.begin_draining(ShutdownReason::Signal, Instant::now());
        let (cancel, _) = watch::channel(false);
        assert!(runtime.spawn_actor(cancel, async {}).is_err());
    }
}

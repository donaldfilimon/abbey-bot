//! Exclusive ownership of the process-wide native player and capture stream.
//! A guard follows actual owned work, including children whose response was dropped.
use std::sync::{Arc, Mutex, Weak};

const BUSY: &str = "The host music player is still owned by another operation. Stop music and wait for capture to finish before retrying.";

#[derive(Default)]
pub(crate) struct HostMusic {
    active: Mutex<Weak<Owner>>,
}
struct Owner {
    guild: u64,
}
#[derive(Clone)]
pub(crate) struct HostMusicLease(Arc<Owner>);

impl HostMusic {
    pub(crate) fn try_start(&self, guild: u64) -> Result<HostMusicLease, &'static str> {
        let mut active = self.active.lock().map_err(|_| BUSY)?;
        if active.upgrade().is_some() {
            return Err(BUSY);
        }
        let owner = Arc::new(Owner { guild });
        *active = Arc::downgrade(&owner);
        Ok(HostMusicLease(owner))
    }

    pub(crate) fn control(&self, guild: u64) -> Result<HostMusicLease, &'static str> {
        let active = self.active.lock().map_err(|_| BUSY)?;
        let owner = active
            .upgrade()
            .ok_or("No active host music operation. Use `/voice play` to start music.")?;
        if owner.guild != guild {
            return Err(BUSY);
        }
        Ok(HostMusicLease(owner))
    }
}

impl HostMusicLease {
    /// The owner id is checked before admitting any native player operation.
    pub(crate) fn for_guild(&self, guild: u64) -> bool {
        self.0.guild == guild
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_child_blocks_competing_start_and_foreign_control() {
        let host = HostMusic::default();
        let response_owner = host.try_start(1).unwrap();
        let child_owner = response_owner.clone();
        drop(response_owner);
        let launches = std::cell::Cell::new(0);
        if host.try_start(2).is_ok() {
            launches.set(launches.get() + 1);
        }
        if host.control(2).is_ok() {
            launches.set(launches.get() + 1);
        }
        assert_eq!(launches.get(), 0);
        assert!(
            host.try_start(1).is_err(),
            "same guild cannot race retained child cleanup"
        );
        let pause_owner = host.control(1).unwrap();
        drop(child_owner);
        assert!(
            host.try_start(2).is_err(),
            "pause child retains ownership after stream cleanup"
        );
        drop(pause_owner);
        let new_owner = host.try_start(2).unwrap();
        assert!(new_owner.for_guild(2));
        assert!(host.control(1).is_err());
    }
    #[tokio::test]
    async fn dropped_response_keeps_host_owned_until_supervised_child_finishes() {
        let host = HostMusic::default();
        let mut supervisor = crate::service::ServiceSupervisor::new();
        supervisor.finish_startup();
        let runtime = crate::voice_session::VoiceRuntime::new(
            crate::voice::VoiceConfig::selected_only(1, 2, crate::voice::VoiceBackendConfig::Disabled, true),
        );
        runtime.attach_service(supervisor.operations());
        let lease = host.try_start(1).unwrap();
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, blocked) = tokio::sync::oneshot::channel();
        let (done, finished) = tokio::sync::oneshot::channel();
        let response = runtime.spawn_result(move |_| async move {
            let _lease = lease;
            entered.send(()).unwrap();
            blocked.await.unwrap();
            drop(_lease);
            done.send(()).unwrap();
        }).unwrap();
        started.await.unwrap();
        drop(response);
        assert!(host.try_start(2).is_err());
        assert!(host.control(2).is_err());
        release.send(()).unwrap();
        finished.await.unwrap();
        assert!(host.try_start(2).is_ok());
    }

}

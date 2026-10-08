//! Preflight orchestration shared by Discord auto-listen and held-permission tests.
use super::{
    AutoListenDecision, AutoListenWhilePresent, VoiceBackendConfig, VoicePhase, VoiceRuntime,
};
use crate::commands_voice::auto_listen_gate::{UpgradeDecision, UpgradeFacts, decide_upgrade};
use std::future::Future;

pub(super) enum PresencePreparation {
    Unchanged(AutoListenWhilePresent),
    Reserved(Box<(u64, Option<VoiceBackendConfig>)>),
}

pub(super) async fn prepare_presence_upgrade<D, P, F>(
    runtime: &VoiceRuntime,
    operation: u64,
    decision: D,
    mut permissions: P,
) -> Result<PresencePreparation, String>
where
    D: FnOnce() -> AutoListenDecision,
    P: FnMut() -> F,
    F: Future<Output = Result<(), String>>,
{
    if runtime.snapshot().await.phase != VoicePhase::PresenceOnly {
        return Ok(PresencePreparation::Unchanged(
            AutoListenWhilePresent::NotPresenceOnly,
        ));
    }
    let decision = decision();
    if decision != AutoListenDecision::Ready {
        return Ok(PresencePreparation::Unchanged(
            AutoListenWhilePresent::RemainedPresent,
        ));
    }
    let permission = permissions().await;
    let phase = runtime.snapshot().await.phase;
    if decide_upgrade(UpgradeFacts {
        phase,
        auto_listen: &decision,
        current_permissions: permission.is_ok(),
    }) == UpgradeDecision::Noop
    {
        permission?;
        return Ok(PresencePreparation::Unchanged(
            if phase == VoicePhase::PresenceOnly {
                AutoListenWhilePresent::RemainedPresent
            } else {
                AutoListenWhilePresent::NotPresenceOnly
            },
        ));
    }
    reserve_auto_listen(runtime, operation, permissions())
        .await
        .map(|reservation| PresencePreparation::Reserved(Box::new(reservation)))
}

pub(super) async fn reserve_auto_listen<F>(
    runtime: &VoiceRuntime,
    operation: u64,
    permissions: F,
) -> Result<(u64, Option<VoiceBackendConfig>), String>
where
    F: Future<Output = Result<(), String>>,
{
    permissions.await?;
    runtime
        .reserve_start_with_backend(operation)
        .ok_or_else(|| "auto-listen was cancelled before activation".into())
}

#[cfg(test)]
mod tests;

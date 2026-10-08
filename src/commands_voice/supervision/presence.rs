//! Startup fallback keeps the authority of the original startup attempt.
use crate::voice_session::VoiceRuntime;
use std::future::Future;

pub(super) async fn reserve_presence_fallback<F>(
    runtime: &VoiceRuntime,
    operation: u64,
    validation: F,
) -> Result<Option<u64>, String>
where
    F: Future<Output = Result<(), String>>,
{
    validation.await?;
    Ok(runtime.reserve_start_if_unchanged(operation))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::{VoiceBackendConfig, VoiceConfig, VoiceMode};
    use std::sync::Arc;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn startup_fallback_refuses_leave_withdrawal_and_draining_during_validation() {
        for stop in 0..3 {
            let mut runtime = VoiceRuntime::new(VoiceConfig::selected_only(
                1,
                2,
                VoiceBackendConfig::Disabled,
                true,
            ));
            runtime.consent = Arc::new(
                crate::voice_consent_store::ConsentStore::acknowledged_fixture(
                    1,
                    &[10],
                    VoiceMode::Local,
                ),
            );
            let operation = runtime.start_operation_token();
            let (release, held) = oneshot::channel();
            let work = reserve_presence_fallback(&runtime, operation, async {
                held.await.unwrap();
                Ok(())
            });
            tokio::pin!(work);
            assert!(futures_util::poll!(&mut work).is_pending());
            match stop {
                0 => runtime.disconnect("leave").await,
                1 => {
                    let change = runtime.change_consent(
                        10,
                        2,
                        crate::voice_consent::Choice::Withdraw,
                        2,
                        true,
                    );
                    assert!(change.epoch_to_stop.is_some());
                    let _ = change.saved.await;
                }
                _ => runtime.begin_draining(),
            }
            let stopped = runtime.start_operation_token();
            release.send(()).unwrap();
            assert!(
                work.await.unwrap().is_none(),
                "cancelled startup must not reserve fallback ({stop})"
            );
            assert_eq!(runtime.start_operation_token(), stopped);
        }
    }

    #[tokio::test]
    async fn startup_fallback_accepts_only_current_authorized_attempt() {
        let runtime = VoiceRuntime::new(VoiceConfig::selected_only(
            1,
            2,
            VoiceBackendConfig::Disabled,
            true,
        ));
        let operation = runtime.start_operation_token();
        let generation = reserve_presence_fallback(&runtime, operation, async { Ok(()) })
            .await
            .unwrap()
            .unwrap();
        assert!(runtime.start_is_current(generation));
        runtime.finish_start_attempt(generation);
        // A failed model preflight retains only the generation it reserved.
        let fallback = reserve_presence_fallback(&runtime, generation, async { Ok(()) })
            .await
            .unwrap()
            .unwrap();
        assert!(runtime.start_is_current(fallback));
        runtime.finish_start_attempt(fallback);
    }
}

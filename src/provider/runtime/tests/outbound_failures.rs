use super::*;
use crate::pipeline::{Outbound, testing::FakeOut};
use crate::platform::OutboundMessage;

struct FailedEdit {
    recorded: FakeOut,
    failure: OutboundFailure,
}

impl Outbound for FailedEdit {
    async fn send(
        &self,
        channel: &str,
        message: &OutboundMessage,
    ) -> Result<String, OutboundFailure> {
        self.recorded.send(channel, message).await
    }
    async fn edit(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
        Err(self.failure)
    }
    async fn typing(&self, _: &str) {}
    async fn react(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
        Ok(())
    }
    async fn fetch(&self, _: &str, _: usize) -> Result<Vec<u8>, String> {
        unreachable!()
    }
}

#[tokio::test]
async fn discord_403_does_not_charge_provider_circuit() {
    for failure in [
        OutboundFailure::http(Some(403), None),
        OutboundFailure::http(Some(429), Some(500)),
        OutboundFailure::http(None, None),
    ] {
        let mut runtime = ProviderRuntime::empty();
        let primary = fake(
            &mut runtime,
            "primary",
            vec![],
            vec![
                "A complete synthetic answer that is long enough to be posted progressively."
                    .into(),
            ],
            true,
        );
        let secondary = fake(&mut runtime, "secondary", vec![], vec![], true);
        let before = lock(&runtime.state).router.snapshot();
        let mut state = crate::runtime::AppState::in_memory();
        Arc::get_mut(&mut state).unwrap().providers = runtime;
        let context = crate::memory::PersonaContext::empty();
        let out = FailedEdit {
            recorded: FakeOut::default(),
            failure,
        };
        let ask = crate::generation::Ask {
            subject: None,
            session_mode: crate::generation::SessionMode::SourceOnly,
            scope: "scope",
            context: &context,
            user_input: "question",
            now: 1,
        };
        for _ in 0..4 {
            let error = crate::generation::generate_read_only(
                &state,
                crate::persona::Persona::Abbey,
                &ask,
                Some(crate::generation::Delivery {
                    out: &out,
                    native_channel_id: "channel",
                    reply_to: None,
                }),
            )
            .await
            .unwrap_err();
            assert_eq!(error.outbound_failure(), Some(failure));
            assert_eq!(error.provider_failure(), ProviderFailureKind::Cancelled);
        }
        assert_eq!(primary.calls.load(Ordering::Relaxed), 4);
        assert_eq!(secondary.calls.load(Ordering::Relaxed), 0);
        assert_eq!(out.recorded.sent.lock().unwrap().len(), 4);
        let router = lock(&state.providers.state);
        assert_eq!(router.router.snapshot(), before);
        let reliability = router
            .router
            .profile(&primary.id, RequestClass::TextReadOnly)
            .unwrap()
            .reliability;
        assert_eq!(reliability.count(), 4);
        assert_eq!(reliability.ewma().unwrap().get(), 1.0);
    }
}

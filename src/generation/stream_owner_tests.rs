use super::*;
use std::sync::Arc;

#[tokio::test]
async fn blocked_delivery_keeps_producer_polled() {
    struct Held(tokio::sync::Notify, tokio::sync::Notify);
    impl Outbound for Held {
        async fn send(&self, _: &str, _: &OutboundMessage) -> Result<String, OutboundFailure> {
            self.0.notify_one();
            self.1.notified().await;
            Ok("posted".into())
        }
        async fn edit(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
            Ok(())
        }
        async fn typing(&self, _: &str) {}
        async fn react(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
            Ok(())
        }
        async fn fetch(&self, _: &str, _: usize) -> Result<Vec<u8>, String> {
            Ok(vec![])
        }
    }
    let out = Held(tokio::sync::Notify::new(), tokio::sync::Notify::new());
    let completed = Arc::new(tokio::sync::Notify::new());
    let (tx, rx) = crate::generation::stream_owner::channel();
    let done = completed.clone();
    let work = async move {
        tx.send("x".repeat(60)).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        done.notify_one();
        Ok(llm::ModelTurn {
            text: "finished".into(),
            calls: vec![],
        })
    };
    let delivery = Delivery {
        out: &out,
        native_channel_id: "synthetic",
        reply_to: None,
    };
    let effects = ConversationEffects::default();
    let grounding = Grounding::default();
    let run = stream_received(
        work,
        rx,
        &delivery,
        "synthetic",
        Persona::Abbey,
        &grounding,
        &effects,
        None,
    );
    let witness = async {
        out.0.notified().await;
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(1), completed.notified()).await;
        out.1.notify_one();
        assert!(
            result.is_ok(),
            "producer must complete while send remains blocked"
        );
    };
    let (result, ()) = tokio::join!(run, witness);
    assert!(result.is_ok());
}

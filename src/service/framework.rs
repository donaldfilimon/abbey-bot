//! Serenity's detached outer dispatch is only an admission shell. The complete
//! Poise future, including setup and hooks, belongs to the root registry.
use super::{OperationKind, OperationRegistry, TaskExit};
use serenity::all::{Client, Context, FullEvent};
use serenity::framework::Framework;
use std::sync::Arc;

pub struct OwnedFramework<F> {
    inner: Arc<F>,
    operations: OperationRegistry,
}
impl<F> OwnedFramework<F> {
    pub fn new(inner: F, operations: OperationRegistry) -> Self {
        Self {
            inner: Arc::new(inner),
            operations,
        }
    }
}
#[serenity::async_trait]
impl<F: Framework + 'static> Framework for OwnedFramework<F> {
    async fn init(&mut self, client: &Client) {
        // Client construction is borrowed and retained directly by main.
        Arc::get_mut(&mut self.inner)
            .expect("framework init precedes sharing")
            .init(client)
            .await;
    }
    async fn dispatch(&self, ctx: Context, event: FullEvent) {
        let inner = self.inner.clone();
        let _ = self.operations.spawn_operation(
            OperationKind::FrameworkDispatch,
            move |_| async move {
                inner.dispatch(ctx, event).await;
                TaskExit::Returned
            },
        );
    }
}

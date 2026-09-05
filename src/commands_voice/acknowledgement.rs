//! Testable interaction-acknowledgement ordering for voice commands.

use std::future::Future;
use std::ops::Deref;

/// The smallest boundary required to acknowledge an ephemeral interaction.
/// Test doubles implement it without constructing a Discord context.
pub(crate) trait EphemeralAcknowledger {
    type Error;

    fn defer_ephemeral(&self) -> impl Future<Output = Result<(), Self::Error>>;
}

impl<U, E> EphemeralAcknowledger for poise::Context<'_, U, E> {
    type Error = serenity::Error;

    async fn defer_ephemeral(&self) -> Result<(), Self::Error> {
        poise::Context::defer_ephemeral(*self).await
    }
}

/// Context token proving that its interaction was already acknowledged.
pub(crate) struct AcknowledgedContext<C>(C);

impl<C> Deref for AcknowledgedContext<C> {
    type Target = C;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Acknowledge before handing the context to any guard or network operation.
pub(crate) async fn with_acknowledged_context<C, F, Fut, T, E>(
    context: C,
    operation: F,
) -> Result<T, E>
where
    C: EphemeralAcknowledger,
    F: FnOnce(AcknowledgedContext<C>) -> Fut,
    Fut: Future<Output = Result<T, E>>,
    C::Error: Into<E>,
{
    context.defer_ephemeral().await.map_err(Into::into)?;
    operation(AcknowledgedContext(context)).await
}

/// Capability token proving an authorized leave already closed the media gate.
pub(crate) struct ClosedMediaGate(());

/// Evaluate authorization synchronously and close media only for an authorized
/// leave. Neither closure can suspend or touch Discord.
pub(crate) fn authorize_and_close_media(
    authorize: impl FnOnce() -> bool,
    close_media: impl FnOnce(),
) -> Option<ClosedMediaGate> {
    if !authorize() {
        return None;
    }
    close_media();
    Some(ClosedMediaGate(()))
}

/// Once media is closed, poll acknowledgement and transition work together.
pub(crate) async fn acknowledge_with_transition<A, W>(
    _closed_media: ClosedMediaGate,
    acknowledgement: A,
    transition_work: W,
) -> (A::Output, W::Output)
where
    A: Future,
    W: Future,
{
    tokio::join!(acknowledgement, transition_work)
}

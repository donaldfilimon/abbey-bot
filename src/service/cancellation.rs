//! Cancellation takes precedence before child I/O can poll a ready operation.
use std::future::Future;
use tokio_util::sync::CancellationToken;

pub async fn complete_or_cancelled<T>(
    cancel: Option<CancellationToken>,
    work: impl Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        biased;
        () = async {
            match cancel {
                Some(cancel) => cancel.cancelled().await,
                None => std::future::pending().await,
            }
        } => None,
        result = work => Some(result),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[tokio::test]
    async fn pre_cancelled_child_io_never_polls_simultaneously_ready_work() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let polls = Cell::new(0);
        let result = complete_or_cancelled(Some(cancel), async {
            polls.set(polls.get() + 1);
            7
        })
        .await;
        assert_eq!(result, None);
        assert_eq!(polls.get(), 0);
    }

    #[tokio::test]
    async fn child_io_runs_without_cancellation_and_stops_when_cancelled_later() {
        assert_eq!(complete_or_cancelled(None, async { 7 }).await, Some(7));
        let cancel = CancellationToken::new();
        assert_eq!(
            complete_or_cancelled(Some(cancel.clone()), async { 8 }).await,
            Some(8)
        );
        let (entered, entering) = tokio::sync::oneshot::channel();
        let owner_cancel = cancel.clone();
        let owner = tokio::spawn(async move {
            complete_or_cancelled(Some(owner_cancel), async move {
                entered.send(()).unwrap();
                std::future::pending::<()>().await;
            })
            .await
        });
        entering.await.unwrap();
        cancel.cancel();
        assert_eq!(owner.await.unwrap(), None);
    }
}

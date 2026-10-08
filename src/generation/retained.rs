//! Managed provider rounds: one reserved attempt, protected until actual join.
use super::*;
use crate::service::{OperationKind, TaskExit};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub(super) struct Request {
    pub state: Arc<AppState>,
    pub seed: crate::provider::ConversationSeed,
    pub parts: PromptParts,
    pub tools: Vec<crate::tools::ToolSpec>,
    pub style: llm::ResponseStyle,
    pub guard: consent::GenerationGuard,
    pub timing: Option<Arc<timing::ProducerTiming>>,
}

pub(super) struct Round<F> {
    pub work: F,
    pub deltas: stream_owner::DeltaReceiver,
    pub selected: tokio::sync::oneshot::Receiver<bool>,
}
pub(super) fn start(
    request: Request,
) -> Result<Round<impl Future<Output = Result<llm::ModelTurn, llm::LlmError>>>, llm::LlmError> {
    let registry = request
        .state
        .service_registry()
        .expect("owned service state")
        .clone();
    let (tx, mut rx) = stream_owner::channel();
    rx.retained = true;
    let cancellation = tx.cancellation();
    let child_cancel = cancellation.clone();
    let (send, receive) = tokio::sync::oneshot::channel();
    let (selected_tx, selected_rx) = tokio::sync::oneshot::channel();
    registry
        .spawn_operation(
            OperationKind::ProviderStream,
            move |service_cancel| async move {
                let work_cancel = child_cancel.clone();
                let handle =
                    tokio::spawn(
                        async move { produce(request, tx, work_cancel, selected_tx).await },
                    );
                let mut owner = stream_owner::StreamOwner::new(handle, child_cancel.clone());
                let result = tokio::select! {
                    biased;
                    _ = service_cancel.cancelled() => owner.cancel_and_join().await,
                    _ = child_cancel.cancelled() => owner.cancel_and_join().await,
                    result = owner.completed() => result,
                };
                let panicked = result.as_ref().is_err_and(|error| error.is_panic());
                let result = result.unwrap_or_else(|_| Err(stream_owner::cancelled()));
                let _ = send.send(result);
                if panicked {
                    TaskExit::Panicked
                } else {
                    TaskExit::Returned
                }
            },
        )
        .map_err(|_| stream_owner::cancelled())?;
    // Dropping a delivery waiter requests cancellation. The protected service
    // owner still observes the actual child handle before it can be frozen.
    struct CancelOnDrop(CancellationToken);
    impl Drop for CancelOnDrop {
        fn drop(&mut self) {
            self.0.cancel();
        }
    }
    let cancel = CancelOnDrop(cancellation);
    Ok(Round {
        work: async move {
            let _cancel = cancel;
            receive
                .await
                .unwrap_or_else(|_| Err(stream_owner::cancelled()))
        },
        deltas: rx,
        selected: selected_rx,
    })
}
async fn produce(
    request: Request,
    tx: stream_owner::DeltaSender,
    cancel: CancellationToken,
    selected: tokio::sync::oneshot::Sender<bool>,
) -> Result<llm::ModelTurn, llm::LlmError> {
    let Request {
        state,
        seed,
        parts,
        tools,
        style,
        guard,
        timing,
    } = request;
    if let Some(observer) = timing.clone() {
        tx.observe(Arc::new(move |text| observer.text(text)));
    }
    let mut conversation = seed.resume(&state.providers);
    guard.check(&state)?;
    let queued = tokio::time::Instant::now();
    let admission = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(stream_owner::cancelled()),
        result = guard.while_current(&state, conversation.reserve()) => result,
    };
    if let Some(timing) = &timing {
        timing.delivery.provider(conversation.selected_provider());
        timing.admitted(queued.elapsed(), &admission);
    }
    admission?;
    tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(stream_owner::cancelled()),
        result = guard.while_current(&state, guard.check_fresh(&state)) => result?,
    }
    let fitted = prompt_budget::fitted(&parts, conversation.prompt_budget());
    let started = || {
        if let Some(timing) = &timing {
            timing.provider_started();
        }
    };
    let streaming = conversation.streams();
    let _ = selected.send(streaming);
    let work = conversation.execute_parts_cancellable(
        &fitted,
        &tools,
        style,
        streaming.then(|| tx.clone()),
        Some(&started),
        Some(cancel.clone()),
    );
    let mut work = std::pin::pin!(work);
    // The monitor owns no execution future. Withdrawal requests cancellation,
    // then awaits the adapter's cleanup contract, including real FM kill/wait.
    let monitor = guard.while_current(&state, std::future::pending::<Result<(), llm::LlmError>>());
    let result = tokio::select! {
        biased;
        withdrawal = monitor => {
            cancel.cancel();
            let _ = (&mut work).await;
            Err(withdrawal.expect_err("consent monitor cannot complete normally"))
        },
        result = &mut work => result,
    };
    let text = tx.snapshot()?;
    if let Some(timing) = &timing {
        timing.text(&text);
    }
    guard.check(&state)?;
    if cancel.is_cancelled() {
        return Err(stream_owner::cancelled());
    }
    let result = result.and_then(stream_owner::validate);
    if result.as_ref().is_err_and(|error| {
        error
            .outbound_failure()
            .is_some_and(|failure| failure.category() == OutboundFailureCategory::Capacity)
    }) {
        cancel.cancel();
    }
    result
}

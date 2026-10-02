//! Fakeable durable reservation and execution lifecycle shared with Discord.
use super::*;
use std::future::Future;
pub(super) trait Executor {
    type State;
    fn observe(&self) -> impl Future<Output = Result<Self::State, &'static str>> + Send;
    fn before(&self, state: &Self::State) -> Result<serde_json::Value, &'static str>;
    fn signature(&self, state: &Self::State) -> Result<serde_json::Value, &'static str>;
    fn mutate(
        &self,
        state: &Self::State,
    ) -> impl Future<Output = Result<Option<u64>, &'static str>> + Send;
    fn verify(
        &self,
        before: &Self::State,
        after: &Self::State,
        created: Option<u64>,
    ) -> Result<serde_json::Value, &'static str>;
}
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_one<
    E: Executor,
    S: Future<Output = Result<(), &'static str>>,
    P: Future<Output = Result<(Policy, String), &'static str>>,
>(
    executor: &E,
    policy: &Policy,
    digest: &str,
    action: &Action,
    ledger: &mut Ledger,
    cancel: CancellationToken,
    now: u64,
    mut save: impl FnMut(&Ledger) -> S,
    mut load: impl FnMut() -> P,
) -> Result<(), &'static str> {
    policy.validate()?;
    if cancel.is_cancelled() || policy.mode == Mode::Stopped {
        return Err("operations stopped");
    }
    if ledger
        .receipts
        .get(&action.key)
        .is_some_and(|r| r.action != *action)
    {
        return Err("idempotency identity drift");
    }
    let before = executor.observe().await?;
    let recorded = executor.before(&before)?;
    if policy.mode == Mode::Propose {
        ledger
            .receipts
            .entry(action.key.clone())
            .or_insert(Receipt {
                action: action.clone(),
                policy_digest: digest.into(),
                at: now,
                status: Status::Proposed,
                before: recorded,
                observed: None,
                detail: "matrix proposal; no mutation".into(),
            });
        return save(ledger).await;
    }
    ledger.authorize(policy, action, now)?;
    let fresh = executor.observe().await?;
    if executor.signature(&before)? != executor.signature(&fresh)? {
        return Err("operation observation drift");
    }
    let (current, current_digest) = load().await?;
    if &current != policy || current_digest != digest || cancel.is_cancelled() {
        return Err("policy/stop drift");
    }
    ledger.receipts.insert(
        action.key.clone(),
        Receipt {
            action: action.clone(),
            policy_digest: digest.into(),
            at: now,
            status: Status::Reserved,
            before: recorded,
            observed: None,
            detail: "durably reserved before Discord I/O".into(),
        },
    );
    save(ledger).await?;
    let outcome =
        super::super::watched_mutation(executor.mutate(&fresh), policy, digest, &cancel, &mut load)
            .await;
    let mut observed = None;
    let mut verified = false;
    if let Ok(Ok(after)) = tokio::time::timeout(Duration::from_secs(30), executor.observe()).await {
        // Cancellation cannot roll back Discord. Reconcile a fresh readback even
        // when the request outcome was unknown; unknown creations have no ID.
        if let Ok(created) = outcome
            && let Ok(value) = executor.verify(&fresh, &after, created)
        {
            observed = Some(value);
            verified = true;
        }
        if observed.is_none() {
            observed = executor.signature(&after).ok();
        }
    }
    let receipt = ledger
        .receipts
        .get_mut(&action.key)
        .ok_or("reservation unavailable")?;
    receipt.status = if verified {
        Status::Verified
    } else {
        Status::ReviewRequired
    };
    receipt.observed = observed;
    receipt.detail = if receipt.status == Status::Verified {
        "fresh readback verified; no history or entitlement deletion"
    } else {
        "outcome requires owner review; never replay automatically"
    }
    .into();
    save(ledger).await?;
    if verified {
        Ok(())
    } else {
        Err("extended operation requires reconciliation")
    }
}
#[cfg(test)]
mod tests;

//! `/admin quarantine`, `/admin contradict` and `/admin resolve`: the
//! Discord edge of `memory_review`, the memory-edge emitters.
use super::*;
use crate::episode_gate::{EdgeReason, MemoryEdgeRequest, Reviewer, parse_digest};
use crate::memory_review;

#[derive(Debug, poise::ChoiceParameter)]
pub enum QuarantineReason {
    #[name = "operator report"]
    OperatorReport,
    #[name = "source untrusted"]
    SourceUntrusted,
    #[name = "policy violation"]
    PolicyViolation,
    #[name = "superseded by newer evidence"]
    SupersededEvidence,
}

impl QuarantineReason {
    const fn reason(&self) -> EdgeReason {
        match self {
            Self::OperatorReport => EdgeReason::OperatorReport,
            Self::SourceUntrusted => EdgeReason::SourceUntrusted,
            Self::PolicyViolation => EdgeReason::PolicyViolation,
            Self::SupersededEvidence => EdgeReason::SupersededEvidence,
        }
    }
}

#[derive(Debug, poise::ChoiceParameter)]
pub enum ReviewVerdict {
    #[name = "valid"]
    Valid,
    #[name = "invalid"]
    Invalid,
}

/// The invoker's review role, from current Discord facts: the guild owner
/// over REST, permissions from the interaction's resolved member.
async fn reviewer(ctx: Context<'_>) -> Option<Reviewer> {
    let guild_id = ctx.guild_id()?;
    let permissions = crate::commands_help::permissions_input(
        ctx.author_member()
            .await
            .and_then(|member| member.permissions)
            .unwrap_or_default(),
    );
    let is_owner = guild_id
        .to_partial_guild(ctx.http())
        .await
        .is_ok_and(|guild| guild.owner_id == ctx.author().id);
    memory_review::reviewer_for(is_owner, &permissions)
}

/// Mark a member's stored fact as suspect in the ledger. It stays visible.
#[poise::command(slash_command, guild_only, ephemeral, rename = "quarantine")]
pub async fn admin_quarantine(
    ctx: Context<'_>,
    #[description = "Whose fact it is"] member: User,
    #[description = "The fact, as stored"]
    #[max_length = 300]
    fact: String,
    #[description = "Why it is suspect"] reason: QuarantineReason,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    if reviewer(ctx).await.is_none() {
        ctx.say(memory_review::NOT_REVIEWER).await?;
        return Ok(());
    }
    let g = scoped_guild(ctx);
    let state = &ctx.data().state;
    let Some(gate) = state.gate_for(&g).cloned() else {
        ctx.say(memory_review::NO_GATE).await?;
        return Ok(());
    };
    let u = scoped_user(&member);
    let service = state.memory_service();
    let Some(selected) = service.resolve_fact(&g, &u, &fact) else {
        ctx.say(memory_review::not_found(member.id.get())).await?;
        return Ok(());
    };
    let Some(target) = service
        .receipt(&g, &u, &selected)
        .and_then(|hex| parse_digest(&hex))
    else {
        ctx.say(memory_review::NO_RECEIPT).await?;
        return Ok(());
    };
    let request = MemoryEdgeRequest::Quarantine {
        scoped_guild: g,
        target,
        reason: reason.reason(),
        now: runtime::now(),
        nonce: gate.next_nonce(),
    };
    let outcome = gate.record_memory_edge(request).await;
    ctx.say(clamp_message(memory_review::quarantine_reply(
        &outcome,
        member.id.get(),
    )))
    .await?;
    Ok(())
}

/// Record that two of a member's stored facts contradict each other. Both stay.
#[poise::command(slash_command, guild_only, ephemeral, rename = "contradict")]
pub async fn admin_contradict(
    ctx: Context<'_>,
    #[description = "Whose facts they are"] member: User,
    #[description = "One fact, as stored"]
    #[max_length = 300]
    fact: String,
    #[description = "The fact it contradicts, as stored"]
    #[max_length = 300]
    counterpart: String,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    if reviewer(ctx).await.is_none() {
        ctx.say(memory_review::NOT_REVIEWER).await?;
        return Ok(());
    }
    let g = scoped_guild(ctx);
    let state = &ctx.data().state;
    let Some(gate) = state.gate_for(&g).cloned() else {
        ctx.say(memory_review::NO_GATE).await?;
        return Ok(());
    };
    let u = scoped_user(&member);
    let service = state.memory_service();
    let (Some(first), Some(second)) = (
        service.resolve_fact(&g, &u, &fact),
        service.resolve_fact(&g, &u, &counterpart),
    ) else {
        ctx.say(memory_review::not_found(member.id.get())).await?;
        return Ok(());
    };
    if first == second {
        ctx.say(memory_review::SAME_FACT).await?;
        return Ok(());
    }
    let receipt = |selected: &str| {
        service
            .receipt(&g, &u, selected)
            .and_then(|hex| parse_digest(&hex))
    };
    let (Some(target), Some(counterpart)) = (receipt(&first), receipt(&second)) else {
        ctx.say(memory_review::NO_RECEIPT).await?;
        return Ok(());
    };
    let request = MemoryEdgeRequest::Contradict {
        scoped_guild: g,
        target,
        counterpart,
        now: runtime::now(),
        nonce: gate.next_nonce(),
    };
    let outcome = gate.record_memory_edge(request).await;
    ctx.say(clamp_message(memory_review::contradict_reply(
        &outcome,
        member.id.get(),
    )))
    .await?;
    Ok(())
}

/// Close an open memory review with a verdict. Deletes nothing.
#[poise::command(slash_command, guild_only, ephemeral, rename = "resolve")]
pub async fn admin_resolve(
    ctx: Context<'_>,
    #[description = "The edge digest from the quarantine reply"]
    #[min_length = 64]
    #[max_length = 64]
    edge: String,
    #[description = "Was the fact valid?"] verdict: ReviewVerdict,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(reviewer) = reviewer(ctx).await else {
        ctx.say(memory_review::NOT_REVIEWER).await?;
        return Ok(());
    };
    let Some(edge) = parse_digest(edge.trim()) else {
        ctx.say(memory_review::BAD_EDGE).await?;
        return Ok(());
    };
    let g = scoped_guild(ctx);
    let Some(gate) = ctx.data().state.gate_for(&g).cloned() else {
        ctx.say(memory_review::NO_GATE).await?;
        return Ok(());
    };
    let valid = matches!(verdict, ReviewVerdict::Valid);
    let request = MemoryEdgeRequest::Resolve {
        scoped_guild: g,
        scoped_user: scoped_user(ctx.author()),
        reviewer,
        edge,
        valid,
        now: runtime::now(),
        nonce: gate.next_nonce(),
    };
    let outcome = gate.record_memory_edge(request).await;
    ctx.say(clamp_message(memory_review::resolve_reply(&outcome, valid)))
        .await?;
    Ok(())
}

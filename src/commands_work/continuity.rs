//! Native human shell owns transient proposals and private, observed previews.
//! The catalog defers before these leaves fetch current authority.
use crate::{
    Context, Error,
    runtime::{AppState, continuity_context::ContinuityAudience, now},
    work::{
        WorkAccess, WorkContentRef, WorkError, WorkScope, WorkStore,
        continuity::{
            ContinuityCard, ContinuityDraft, ContinuityProposal, ProposalId, ProposalRegistry,
            ResolvedConfirmation,
        },
    },
};
use serenity::all::{CommandType, CreateEmbed, InteractionContext};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Mutex, atomic::Ordering},
};

const MAX_PENDING: usize = 256;
const MAX_TEXT_BYTES: usize = 1600;
const MAX_SOURCES: usize = 8;
const MAX_SOURCE_INPUT_BYTES: usize = 256;
const MAX_PROPOSAL_ID_BYTES: usize = 53;
const UNAVAILABLE: &str = "No confirmed continuity card is available here.";

struct Registry {
    proposals: ProposalRegistry,
    epochs: BTreeMap<ProposalId, (u64, u64)>,
    presented: BTreeSet<ProposalId>,
}

impl Registry {
    fn prune(&mut self, at: u64) {
        self.epochs.retain(|_, (_, expires)| *expires > at);
        let epochs = &self.epochs;
        self.presented.retain(|id| epochs.contains_key(id));
    }
}

/// This owner is carried only by native command Data, never by AppState/ToolScope.
/// No Serialize/Debug: pending text and IDs are private, transient shell state.
pub(crate) struct HumanContinuity {
    registry: Mutex<Registry>,
}

impl HumanContinuity {
    pub(crate) fn new() -> Result<Self, WorkError> {
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).map_err(|_| WorkError::Invalid)?;
        Ok(Self {
            registry: Mutex::new(Registry {
                proposals: ProposalRegistry::new(nonce),
                epochs: BTreeMap::new(),
                presented: BTreeSet::new(),
            }),
        })
    }

    fn propose(
        &self,
        draft: ContinuityDraft,
        access: &WorkAccess,
        work: &WorkStore,
        generation: u64,
        at: u64,
    ) -> Result<ContinuityProposal, WorkError> {
        if generation == 0 {
            return Err(WorkError::Stale);
        }
        let mut registry = AppState::lock(&self.registry);
        registry.prune(at);
        if registry.epochs.len() >= MAX_PENDING {
            return Err(WorkError::Full);
        }
        // The pure registry repeats exact-scope membership/content/source checks.
        // Do not reset it on clear: nonce/sequence IDs must never be reused.
        let proposal = registry.proposals.propose(draft, access, work, at)?;
        registry
            .epochs
            .insert(proposal.id, (generation, proposal.expires_at));
        Ok(proposal)
    }

    /// Called only after the private native preview delivery has succeeded.
    /// A known boot nonce makes later numeric IDs predictable, so creation alone
    /// must not mint confirmation authority for text that has never been shown.
    fn mark_presented(
        &self,
        proposal: &ContinuityProposal,
        access: &WorkAccess,
        generation: u64,
        at: u64,
    ) -> Result<(), WorkError> {
        if proposal.actor != access.actor || proposal.scope != access.scope() {
            return Err(WorkError::Denied);
        }
        let mut registry = AppState::lock(&self.registry);
        registry.prune(at);
        let (bound_generation, expires_at) = registry
            .epochs
            .get(&proposal.id)
            .copied()
            .ok_or(WorkError::Missing)?;
        if generation == 0
            || bound_generation != generation
            || expires_at != proposal.expires_at
            || at >= expires_at
        {
            return Err(WorkError::Stale);
        }
        registry.presented.insert(proposal.id);
        Ok(())
    }

    fn resolve(
        &self,
        id: ProposalId,
        access: &WorkAccess,
        generation: u64,
        at: u64,
    ) -> Result<(ResolvedConfirmation, u64), WorkError> {
        let mut registry = AppState::lock(&self.registry);
        registry.prune(at);
        let (bound_generation, _) = registry
            .epochs
            .get(&id)
            .copied()
            .ok_or(WorkError::Missing)?;
        if generation == 0 || bound_generation != generation || !registry.presented.contains(&id) {
            return Err(WorkError::Stale);
        }
        // Wrong actors/scopes cannot consume a still-live owner's control.
        let grant =
            registry
                .proposals
                .resolve_confirmation(id, access.actor, &access.scope(), at)?;
        registry.epochs.remove(&id);
        registry.presented.remove(&id);
        Ok((grant, bound_generation))
    }

    fn discard(&self, id: ProposalId, access: &WorkAccess, at: u64) {
        let mut registry = AppState::lock(&self.registry);
        if registry
            .proposals
            .resolve_confirmation(id, access.actor, &access.scope(), at)
            .is_ok()
        {
            registry.epochs.remove(&id);
            registry.presented.remove(&id);
        }
        registry.prune(at);
    }
}

struct NativeEnvelope {
    scope: WorkScope,
    actor: u64,
    channel: u64,
}

/// The actual native envelope, not an intent boolean supplied by a model.
/// Catalog defer must already have succeeded; this helper does no I/O.
fn native_envelope(
    ctx: Context<'_>,
    expected_leaf: &'static str,
) -> Result<NativeEnvelope, WorkError> {
    let poise::Context::Application(application) = ctx else {
        return Err(WorkError::Denied);
    };
    let interaction = application.interaction;
    let actor = interaction.user.id.get();
    let channel = interaction.channel_id.get();
    let bot = application.serenity_context.cache.current_user().id;
    if application.interaction_type != poise::CommandInteractionType::Command
        || interaction.data.kind != CommandType::ChatInput
        || interaction.data.name != "work"
        || ctx.author().bot
        || interaction.user.bot
        || actor == 0
        || channel == 0
        || interaction.application_id.get() != bot.get()
        || application.command.qualified_name != expected_leaf
        || !application.command.ephemeral
        || !application.has_sent_initial_response.load(Ordering::SeqCst)
    {
        return Err(WorkError::Denied);
    }
    let scope = match (interaction.context, interaction.guild_id) {
        (Some(InteractionContext::Guild), Some(guild)) if guild.get() != 0 => WorkScope::Team {
            guild: guild.get(),
            channel,
        },
        (Some(InteractionContext::BotDm), None) => WorkScope::Personal { owner: actor },
        _ => return Err(WorkError::Denied),
    };
    Ok(NativeEnvelope {
        scope,
        actor,
        channel,
    })
}

/// Show a card.
#[poise::command(slash_command, subcommands("show", "propose", "confirm", "clear"))]
pub async fn continuity(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Show a card.
#[poise::command(slash_command, ephemeral)]
pub async fn show(ctx: Context<'_>) -> Result<(), Error> {
    let native = native_envelope(ctx, "work continuity show")?;
    let state = &ctx.data().state;
    let Some(admitted) = state
        .prepare_continuity_context(
            native.scope.clone(),
            native.actor,
            native.channel,
            ContinuityAudience::PrivateInteraction,
        )
        .await
    else {
        // Do not reveal raw card counts, revisions or receipt state when blocked.
        return super::reply(ctx, UNAVAILABLE).await;
    };
    if !admitted.permits_private(native.actor, native.channel) || !admitted.current(state) {
        return super::reply(ctx, UNAVAILABLE).await;
    }
    let card = {
        let stores = AppState::lock(&state.stores);
        stores.continuity.card(&native.scope).cloned()
    };
    let Some(card) = card else {
        return super::reply(ctx, UNAVAILABLE).await;
    };
    if card.confirmed_text != admitted.text() || !admitted.current(state) {
        return super::reply(ctx, UNAVAILABLE).await;
    }
    let embed = card_embed(admitted.text(), &card)?;
    // Recheck native/receipt/canonical authority immediately before private output.
    if !admitted.fresh(state).await || !admitted.current(state) {
        return super::reply(ctx, UNAVAILABLE).await;
    }
    ctx.send(private_reply("Confirmed continuity for this scope.", embed))
        .await?;
    Ok(())
}

/// Preview a card.
#[poise::command(slash_command, ephemeral)]
pub async fn propose(
    ctx: Context<'_>,
    #[description = "Exact card text, at most 1600 UTF-8 bytes"] text: String,
    #[description = "Up to 8 comma-separated task:ID or decision:ID references"] sources: Option<
        String,
    >,
) -> Result<(), Error> {
    let native = native_envelope(ctx, "work continuity propose")?;
    let state = &ctx.data().state;
    let access = state
        .fresh_continuity_access(&native.scope, native.actor, native.channel)
        .await?;
    let generation = state
        .continuity_generation(&native.scope)
        .ok_or(WorkError::Stale)?;
    let proposal = {
        let stores = AppState::lock(&state.stores);
        stores.work.scope_projects(&native.scope, access, false)?;
        let source_refs = parse_sources(sources.as_deref(), &native.scope, &access, &stores.work)?;
        let base_revision = stores
            .continuity
            .card(&native.scope)
            .map_or(0, |card| card.revision);
        ctx.data().continuity.propose(
            ContinuityDraft {
                scope: native.scope.clone(),
                base_revision,
                presented_text: text,
                source_refs,
            },
            &access,
            &stores.work,
            generation,
            now(),
        )?
    };
    let delivered = async {
        let current_access = state
            .fresh_continuity_access(&native.scope, native.actor, native.channel)
            .await?;
        {
            let stores = AppState::lock(&state.stores);
            stores
                .work
                .scope_projects(&native.scope, current_access, false)?;
            require_current_sources(
                &proposal.source_refs,
                &native.scope,
                &current_access,
                &stores.work,
            )?;
            if stores
                .continuity
                .card(&native.scope)
                .map_or(0, |card| card.revision)
                != proposal.base_revision
            {
                return Err(WorkError::Stale.into());
            }
        }
        let embed = proposal_embed(&proposal)?;
        if state.continuity_generation(&native.scope) != Some(generation)
            || now() >= proposal.expires_at
        {
            return Err(WorkError::Stale.into());
        }
        ctx.send(private_reply(
            "Review this exact text. It is not saved until you confirm its proposal ID.",
            embed,
        ))
        .await?;
        let delivered_generation = state
            .continuity_generation(&native.scope)
            .ok_or(WorkError::Stale)?;
        ctx.data()
            .continuity
            .mark_presented(&proposal, &access, delivered_generation, now())?;
        Ok::<(), Error>(())
    }
    .await;
    if delivered.is_err() {
        // Never leave unseen/partly failed preview content available to confirm.
        ctx.data().continuity.discard(proposal.id, &access, now());
    }
    delivered
}

/// Confirm a card.
#[poise::command(slash_command, ephemeral)]
pub async fn confirm(
    ctx: Context<'_>,
    #[description = "Exact proposal ID from continuity propose"] proposal: String,
) -> Result<(), Error> {
    let native = native_envelope(ctx, "work continuity confirm")?;
    let state = &ctx.data().state;
    let access = state
        .fresh_continuity_access(&native.scope, native.actor, native.channel)
        .await?;
    {
        let stores = AppState::lock(&state.stores);
        // All projects must grant existing Work manager membership. MANAGE_GUILD
        // does not grant this membership, and delegated managers remain allowed.
        stores.work.scope_projects(&native.scope, access, true)?;
    }
    if proposal.len() > MAX_PROPOSAL_ID_BYTES {
        return Err(WorkError::Invalid.into());
    }
    let id = ProposalId::decode(&proposal)?;
    let generation = state
        .continuity_generation(&native.scope)
        .ok_or(WorkError::Stale)?;
    let (grant, bound_generation) =
        ctx.data()
            .continuity
            .resolve(id, &access, generation, now())?;
    // The retained mutation rechecks native/canonical authority after acquiring
    // its serial and before publication. There is no replacement text or epoch
    // argument from the client, and this operation must never be replayed for IO.
    let card = state
        .confirm_continuity(grant, bound_generation, native.actor, native.channel)
        .await?;
    super::reply(
        ctx,
        format!(
            "Confirmed continuity card revision {}. Expires <t:{}:f>.",
            card.revision, card.expires_at
        ),
    )
    .await
}

/// Clear a card.
#[poise::command(slash_command, ephemeral)]
pub async fn clear(ctx: Context<'_>) -> Result<(), Error> {
    let native = native_envelope(ctx, "work continuity clear")?;
    let state = &ctx.data().state;
    let access = state
        .fresh_continuity_access(&native.scope, native.actor, native.channel)
        .await?;
    {
        let stores = AppState::lock(&state.stores);
        stores.work.scope_projects(&native.scope, access, true)?;
    }
    // No raw-card shortcut: the retained owner handles protective generation,
    // receipt reconciliation, canonical publication and exact readback.
    let removed = state
        .clear_continuity(native.scope, native.actor, native.channel)
        .await?;
    super::reply(
        ctx,
        if removed == 0 {
            "No confirmed continuity card was available to clear."
        } else {
            "Cleared continuity for this scope."
        },
    )
    .await
}

/// Parse only typed IDs; native project/revision facts always come from Work.
fn parse_sources(
    input: Option<&str>,
    scope: &WorkScope,
    access: &WorkAccess,
    work: &WorkStore,
) -> Result<BTreeSet<WorkContentRef>, WorkError> {
    let Some(input) = input else {
        return Ok(BTreeSet::new());
    };
    if input.is_empty() || input.len() > MAX_SOURCE_INPUT_BYTES {
        return Err(WorkError::Invalid);
    }
    let mut references = BTreeSet::new();
    for token in input.split(',') {
        if references.len() >= MAX_SOURCES {
            return Err(WorkError::Invalid);
        }
        let (kind, encoded_id) = token.trim().split_once(':').ok_or(WorkError::Invalid)?;
        let id: u64 = encoded_id.parse().map_err(|_| WorkError::Invalid)?;
        if id == 0 || id.to_string() != encoded_id {
            return Err(WorkError::Invalid);
        }
        let source = current_source(kind, id, scope, access, work)?;
        if !references.insert(source) {
            return Err(WorkError::Invalid);
        }
    }
    Ok(references)
}

fn current_source(
    kind: &str,
    id: u64,
    scope: &WorkScope,
    access: &WorkAccess,
    work: &WorkStore,
) -> Result<WorkContentRef, WorkError> {
    let (project_id, source) = match kind {
        "task" => {
            let task = work.tasks.get(&id).ok_or(WorkError::Denied)?;
            if task.id != id {
                return Err(WorkError::Denied);
            }
            (
                task.project_id,
                WorkContentRef::Task {
                    project: task.project_id,
                    id,
                    revision: task.revision,
                },
            )
        }
        "decision" => {
            let decision = work.decisions.get(&id).ok_or(WorkError::Denied)?;
            if decision.id != id {
                return Err(WorkError::Denied);
            }
            (
                decision.project_id,
                WorkContentRef::Decision {
                    project: decision.project_id,
                    id,
                    revision: 1,
                },
            )
        }
        _ => return Err(WorkError::Invalid),
    };
    let project = work.projects.get(&project_id).ok_or(WorkError::Denied)?;
    if project.id != project_id || &project.scope != scope {
        return Err(WorkError::Denied);
    }
    project.authorize(*access, false)?;
    Ok(source)
}

fn require_current_sources(
    references: &BTreeSet<WorkContentRef>,
    scope: &WorkScope,
    access: &WorkAccess,
    work: &WorkStore,
) -> Result<(), WorkError> {
    if references.len() > MAX_SOURCES {
        return Err(WorkError::Invalid);
    }
    for reference in references {
        let (kind, id) = match reference {
            WorkContentRef::Task { id, .. } => ("task", *id),
            WorkContentRef::Decision { id, .. } => ("decision", *id),
        };
        if current_source(kind, id, scope, access, work)? != *reference {
            return Err(WorkError::Stale);
        }
    }
    Ok(())
}

/// Escape the transport display only. Do not trim or mutate immutable text.
/// At most two Unicode characters per original character: <=3200 chars, safely
/// inside an embed description's4096-char limit for1600-byte admitted text.
fn escape_text(text: &str) -> Result<String, WorkError> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(WorkError::Invalid);
    }
    let mut escaped = String::with_capacity(text.len().saturating_mul(2));
    for character in text.chars() {
        if matches!(
            character,
            '\\' | '`' | '*' | '_' | '~' | '|' | '>' | '[' | ']' | '#' | '<'
        ) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    Ok(escaped)
}

fn render_sources(references: &BTreeSet<WorkContentRef>) -> Result<String, WorkError> {
    if references.len() > MAX_SOURCES {
        return Err(WorkError::Invalid);
    }
    if references.is_empty() {
        return Ok("No source references selected.".into());
    }
    let mut lines = Vec::with_capacity(references.len());
    for reference in references {
        let (kind, project, id, revision) = match reference {
            WorkContentRef::Task {
                project,
                id,
                revision,
            } => ("task", project, id, revision),
            WorkContentRef::Decision {
                project,
                id,
                revision,
            } => ("decision", project, id, revision),
        };
        lines.push(format!(
            "{kind}:{id} · project #{project} · revision {revision}"
        ));
    }
    let rendered = lines.join("\n");
    if rendered.chars().count() > 1024 {
        return Err(WorkError::Invalid);
    }
    Ok(rendered)
}

fn proposal_embed(proposal: &ContinuityProposal) -> Result<CreateEmbed, WorkError> {
    let id = proposal.id.encode();
    Ok(CreateEmbed::new()
        .title("Continuity preview")
        .description(escape_text(&proposal.presented_text)?)
        .field("Base revision", proposal.base_revision.to_string(), true)
        .field("Confirm by", format!("<t:{}:f>", proposal.expires_at), true)
        .field("Source references", render_sources(&proposal.source_refs)?, false)
        .field(
            "Proposal ID",
            format!(
                "`{id}`\nUse `/work continuity confirm` with this ID. This proposal expires in 5 minutes."
            ),
            false,
        ))
}

fn card_embed(exact_text: &str, card: &ContinuityCard) -> Result<CreateEmbed, WorkError> {
    Ok(CreateEmbed::new()
        .title("Confirmed continuity")
        .description(escape_text(exact_text)?)
        .field("Revision", card.revision.to_string(), true)
        .field("Expires", format!("<t:{}:f>", card.expires_at), true)
        .field(
            "Source references",
            render_sources(&card.source_refs)?,
            false,
        ))
}

fn private_reply(content: &'static str, embed: CreateEmbed) -> poise::CreateReply {
    poise::CreateReply::default()
        .content(content)
        .embed(embed)
        .ephemeral(true)
        .allowed_mentions(crate::gateway::no_mentions())
}

#[cfg(test)]
mod tests;

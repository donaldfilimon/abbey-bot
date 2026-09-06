//! Slash commands over the learning, memory, and config surfaces
//! (`docs/spec/companionapp.md` "Full slash-command surface",
//! `docs/spec/multiguild.md` "/admin").
//!
//! Same contract as `commands.rs`: defer first, clamp every rendered answer,
//! translate Discord data into plain values and hand them to the pure modules.
//! Per-guild configuration uses Discord's `default_member_permissions`.
//! Member memory is self-service; explicit cross-member access is checked
//! against the invoker's current Discord permissions at command runtime.

use std::time::Duration;

use serenity::all::{
    Attachment, ButtonStyle, ComponentInteraction, CreateActionRow, CreateAttachment, CreateButton,
    CreateInteractionResponse, CreateInteractionResponseMessage, EditInteractionResponse,
    Permissions, User,
};

use crate::ask;
use crate::brain::telemetry::BrainView;
use crate::commands::{PersonaChoice, clamp_message};
use crate::engine;
use crate::episode_gate::LearningToggleRequest;
use crate::guild::{self, GuildSettings};
use crate::llm;
use crate::memory;
use crate::memory_gate;
use crate::persist::{PersistReport, render_component_outcome};
use crate::runtime::{self, AppState};
use crate::vision::{self, ImageUnderstanding};
use crate::{Context, Error};

const NO_GUILD: &str = "This one only works inside a server.";

const PLATFORM: &str = "discord";

const CROSS_USER_MEMORY_DENIED: &str = "You can manage only your own memory unless Discord currently grants you Manage Messages or Manage Server.";

/// The namespace a command's data lives in: the guild, or — in a DM — the
/// invoker's own one-person DM guild, matching `SocialEvent::scoped_guild_id`
/// so `/remember` in a DM and a DM conversation see the same facts.
fn scoped_guild(ctx: Context<'_>) -> String {
    match ctx.guild_id() {
        Some(g) => guild::scoped_guild_id(PLATFORM, Some(&g.get().to_string())),
        None => format!("{PLATFORM}:dm:{}", ctx.author().id.get()),
    }
}

fn scoped_user(user: &User) -> String {
    guild::scoped_user_id(PLATFORM, &user.id.get().to_string())
}

fn scoped_channel(ctx: Context<'_>) -> String {
    guild::scoped_channel_id(PLATFORM, &ctx.channel_id().get().to_string())
}

/// Discord-facing mirror of an on/off toggle.
#[derive(Debug, Clone, Copy, poise::ChoiceParameter)]
pub enum OnOff {
    #[name = "on"]
    On,
    #[name = "off"]
    Off,
}

impl OnOff {
    const fn is_on(self) -> bool {
        matches!(self, Self::On)
    }
    const fn label(self) -> &'static str {
        if self.is_on() { "on" } else { "off" }
    }
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

async fn memory_subject_authorized(ctx: Context<'_>, subject: &User) -> bool {
    let permissions = if subject.id == ctx.author().id {
        Vec::new()
    } else {
        crate::commands_help::permissions_input(
            ctx.author_member()
                .await
                .and_then(|member| member.permissions)
                .unwrap_or_default(),
        )
    };
    crate::memory_card::subject_authorized(ctx.author().id.get(), subject.id.get(), &permissions)
}

fn memory_card(state: &AppState, guild: &str, subject: &User) -> (String, bool) {
    let user = scoped_user(subject);
    let (facts, pending) = state.memory_service().subject_snapshot(guild, &user);
    let standing = {
        let stores = AppState::lock(&state.stores);
        AppState::lock(&state.social).reputation(&user, guild, &*stores)
    };
    let content = crate::memory_card::render(&crate::memory_card::MemoryCard {
        subject_id: subject.id.get(),
        facts: &facts,
        pending: &pending,
        standing,
    });
    (content, !facts.is_empty())
}

async fn send_private_no_mentions(ctx: Context<'_>, content: String) -> Result<(), Error> {
    ctx.send(
        poise::CreateReply::default()
            .content(clamp_message(content))
            .ephemeral(true)
            .allowed_mentions(crate::gateway::no_mentions()),
    )
    .await?;
    Ok(())
}

/// Store a durable fact about a member (yourself by default).
#[poise::command(slash_command, ephemeral)]
pub async fn remember(
    ctx: Context<'_>,
    #[description = "A single concise fact, stated in third person"]
    #[max_length = 300]
    fact: String,
    #[description = "Who it is about (default: you; moderators may choose another member)"]
    user: Option<User>,
    #[description = "An existing fact this replaces — it is removed only because you said so"]
    #[autocomplete = "autocomplete_fact"]
    #[max_length = 300]
    replaces: Option<String>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let g = scoped_guild(ctx);
    let subject = user.as_ref().unwrap_or(ctx.author());
    if !memory_subject_authorized(ctx, subject).await {
        ctx.say(CROSS_USER_MEMORY_DENIED).await?;
        return Ok(());
    }
    let u = scoped_user(subject);
    let state = &ctx.data().state;
    // With the episode gate configured the write is proposed first and
    // happens only on `appended` (amendment 2026-09-06). The local
    // preconditions are checked read-only before proposing, so no candidate
    // is admitted for a write that would not happen anyway.
    let fact = match memory::validated_fact(&fact) {
        Ok(fact) => fact,
        Err(message) => {
            ctx.say(message).await?;
            return Ok(());
        }
    };
    if state.gate_for(&g).is_some() {
        match replaces.as_deref() {
            None => {
                if state
                    .memory_service()
                    .remember_blocked(&g, &u, &fact)
                    .is_some()
                {
                    ctx.say("Already on record (or the fact list is full).")
                        .await?;
                    return Ok(());
                }
            }
            Some(old) => match state.memory_service().resolve_fact(&g, &u, old) {
                None => {
                    ctx.say("No remembered fact matches what you asked to replace.")
                        .await?;
                    return Ok(());
                }
                Some(selected) if selected == fact => {
                    ctx.say("Already on record (or the fact list is full).")
                        .await?;
                    return Ok(());
                }
                Some(_) => {}
            },
        }
    }
    let receipt = match memory_gate::admit_fact(state, &g, &u, &fact, replaces.as_deref()).await {
        Ok(receipt) => receipt,
        Err(message) => {
            ctx.say(message).await?;
            return Ok(());
        }
    };
    // `replaces` is an explicit human signal, so it is authoritative and needs
    // no confirmation step. Without it nothing is ever removed here.
    let outcome = match replaces.as_deref() {
        Some(old) => state
            .memory_service()
            .remember_replacing(&g, &u, &fact, old, runtime::now()),
        None => state
            .memory_service()
            .remember(&g, &u, &fact, runtime::now()),
    };
    if let Some(digest_hex) = &receipt {
        match &outcome {
            Ok(runtime::RememberOutcome::Stored(stored)) => {
                memory_gate::settle_receipts(state, &g, &u, stored, None, digest_hex);
            }
            Ok(runtime::RememberOutcome::Superseded { stored, removed }) => {
                memory_gate::settle_receipts(state, &g, &u, stored, Some(removed), digest_hex);
            }
            // Admitted, but the local store refused after all (a concurrent
            // write). The candidate stands in the ledger with nothing behind
            // it; the log is the record.
            _ => tracing::warn!("episode gate: admitted fact candidate stored nothing locally"),
        }
    }
    let reply = match outcome {
        Ok(runtime::RememberOutcome::Stored(fact)) => {
            format!("Stored about <@{}>: {fact}", subject.id.get())
        }
        Ok(runtime::RememberOutcome::Superseded { stored, removed }) => format!(
            "Stored about <@{}>: {stored}\nReplaced: {removed}",
            subject.id.get()
        ),
        Ok(runtime::RememberOutcome::Proposed { stored, proposed }) => format!(
            "Stored about <@{}>: {stored}\nProposed to replace: {proposed} — nothing was removed. Run /pending confirm to apply it.",
            subject.id.get()
        ),
        Ok(runtime::RememberOutcome::Unchanged) => {
            "Already on record (or the fact list is full).".to_string()
        }
        Err(message) => {
            ctx.say(message).await?;
            return Ok(());
        }
    };
    ctx.say(clamp_message(reply)).await?;
    Ok(())
}

async fn autocomplete_fact(ctx: Context<'_>, partial: &str) -> Vec<String> {
    let g = scoped_guild(ctx);
    let u = scoped_user(ctx.author());
    let state = &ctx.data().state;
    let facts = state.memory_service().facts(&g, &u);
    memory::autocomplete_facts(&facts, partial)
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// Forget one of your stored facts.
#[poise::command(slash_command, ephemeral)]
pub async fn forget(
    ctx: Context<'_>,
    #[description = "The fact to remove"]
    #[autocomplete = "autocomplete_fact"]
    fact: String,
    #[description = "Who it is about (default: you; moderators may choose another member)"]
    user: Option<User>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let g = scoped_guild(ctx);
    let subject = user.as_ref().unwrap_or(ctx.author());
    if !memory_subject_authorized(ctx, subject).await {
        ctx.say(CROSS_USER_MEMORY_DENIED).await?;
        return Ok(());
    }
    let u = scoped_user(subject);
    let state = &ctx.data().state;
    let Some(selected) = state.memory_service().resolve_fact(&g, &u, &fact) else {
        ctx.say("Nothing by that wording was on record.").await?;
        return Ok(());
    };
    if let Err(message) = memory_gate::admit_forget(state, &g, &u, &selected).await {
        ctx.say(message).await?;
        return Ok(());
    }
    let removed = state.memory_service().forget(&g, &u, &selected);
    if removed {
        memory_gate::drop_receipt(state, &g, &u, &selected);
    }
    ctx.say(if removed {
        "Forgotten."
    } else {
        "Nothing by that wording was on record."
    })
    .await?;
    Ok(())
}

async fn autocomplete_pending(ctx: Context<'_>, partial: &str) -> Vec<String> {
    let g = scoped_guild(ctx);
    let u = scoped_user(ctx.author());
    let state = &ctx.data().state;
    let needle = partial.to_lowercase();
    state
        .memory_service()
        .pending_supersessions(&g, &u)
        .into_iter()
        .map(|pending| pending.old_fact)
        .filter(|old| needle.is_empty() || old.to_lowercase().contains(&needle))
        .take(25)
        .collect()
}

/// Discord allows at most five action rows; each pending entry gets one Confirm/Dismiss row.
const PENDING_BUTTON_ROWS: usize = 5;
const PENDING_COMPONENT_TIMEOUT_SECS: u64 = 5 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingButtonAction {
    Confirm,
    Dismiss,
}

fn pending_button_custom_id(
    ctx_id: u64,
    action: PendingButtonAction,
    subject_id: u64,
    idx: usize,
) -> String {
    let tag = match action {
        PendingButtonAction::Confirm => "c",
        PendingButtonAction::Dismiss => "d",
    };
    format!("{ctx_id}:p:{tag}:{subject_id}:{idx}")
}

fn parse_pending_button_custom_id(
    custom_id: &str,
    ctx_id: u64,
) -> Option<(PendingButtonAction, u64, usize)> {
    let prefix = format!("{ctx_id}:p:");
    let rest = custom_id.strip_prefix(&prefix)?;
    let mut parts = rest.split(':');
    let action = match parts.next()? {
        "c" => PendingButtonAction::Confirm,
        "d" => PendingButtonAction::Dismiss,
        _ => return None,
    };
    let subject_id = parts.next()?.parse().ok()?;
    let idx = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((action, subject_id, idx))
}

fn format_pending_list_body(subject_id: u64, pending: &[memory::PendingSupersession]) -> String {
    let mut reply = format!("Proposed for <@{subject_id}> — nothing has been removed:\n");
    for (i, entry) in pending.iter().enumerate() {
        reply.push_str(&format!(
            "{}. {} → {}\n",
            i + 1,
            entry.old_fact,
            entry.new_fact
        ));
    }
    if pending.len() > PENDING_BUTTON_ROWS {
        reply.push_str(&format!(
            "Buttons cover the first {PENDING_BUTTON_ROWS}; use `/pending confirm` or `/pending dismiss` with autocomplete for the rest.\n"
        ));
    } else {
        reply.push_str(
            "Tap Confirm to remove the old fact, or Dismiss to keep both. Slash autocomplete still works.\n",
        );
    }
    reply
}

fn pending_action_rows(
    ctx_id: u64,
    subject_id: u64,
    pending: &[memory::PendingSupersession],
    version: u64,
) -> Vec<CreateActionRow> {
    pending
        .iter()
        .take(PENDING_BUTTON_ROWS)
        .enumerate()
        .map(|(idx, _entry)| {
            CreateActionRow::Buttons(vec![
                CreateButton::new(format!(
                    "{}:v:{version}",
                    pending_button_custom_id(ctx_id, PendingButtonAction::Confirm, subject_id, idx,)
                ))
                .style(ButtonStyle::Success)
                .label(format!("Confirm {}", idx + 1)),
                CreateButton::new(format!(
                    "{}:v:{version}",
                    pending_button_custom_id(ctx_id, PendingButtonAction::Dismiss, subject_id, idx,)
                ))
                .style(ButtonStyle::Secondary)
                .label(format!("Dismiss {}", idx + 1)),
            ])
        })
        .collect()
}

/// Confirm a proposed supersession behind the episode gate. A tombstone is
/// proposed only when the confirm would actually remove the old fact (a
/// pending entry names it and both facts are still held); otherwise
/// `confirm_supersession` reports its own non-removing outcome and nothing is
/// proposed. The receipt is dropped only after the fact is gone.
async fn gated_confirm(
    state: &runtime::AppState,
    scoped_guild: &str,
    scoped_user: &str,
    old_fact: &str,
) -> String {
    let service = state.memory_service();
    if service.confirm_would_remove(scoped_guild, scoped_user, old_fact)
        && let Err(message) =
            memory_gate::admit_forget(state, scoped_guild, scoped_user, old_fact).await
    {
        return message;
    }
    let outcome = service.confirm_supersession(scoped_guild, scoped_user, old_fact);
    if matches!(outcome, runtime::SupersessionOutcome::Confirmed(_)) {
        memory_gate::drop_receipt(state, scoped_guild, scoped_user, old_fact);
    }
    format_confirm_outcome(outcome)
}

fn format_confirm_outcome(outcome: runtime::SupersessionOutcome) -> String {
    match outcome {
        runtime::SupersessionOutcome::Confirmed(removed) => format!("Removed: {removed}"),
        runtime::SupersessionOutcome::AlreadyGone(old) => format!(
            "That fact was already gone, so nothing was removed. Cleared the proposal for: {old}"
        ),
        runtime::SupersessionOutcome::PremiseGone { old_fact, new_fact } => format!(
            "Refused, and nothing was removed. That proposal said {new_fact} replaces \
             {old_fact}, but {new_fact} is no longer on record, so confirming would have \
             left you holding neither. Cleared the stale proposal."
        ),
        runtime::SupersessionOutcome::NotPending => "No proposal names that fact.".to_string(),
    }
}

pub(crate) struct PendingComponentSession {
    pub command_id: u64,
    pub owner: u64,
    pub subject: u64,
    pub guild: Option<u64>,
    pub channel: u64,
    pub version: u64,
    pub displayed: Vec<memory::PendingSupersession>,
}

/// Construct no permission or effect future until acknowledgement completes.
async fn authorized_pending_effect<A, V, P, PF, E, EF, T>(
    acknowledgement: A,
    session: &PendingComponentSession,
    validate: V,
    permissions: P,
    effect: E,
) -> Result<Option<T>, Error>
where
    A: std::future::Future<Output = Result<(), Error>>,
    V: FnOnce() -> Option<(PendingButtonAction, usize)>,
    P: FnOnce() -> PF,
    PF: std::future::Future<Output = Result<Vec<crate::command_catalog::DiscordPermission>, Error>>,
    E: FnOnce(PendingButtonAction, usize) -> EF,
    EF: std::future::Future<Output = T>,
{
    acknowledgement.await?;
    let Some((action, index)) = validate() else {
        return Ok(None);
    };
    let Ok(permissions) = permissions().await else {
        return Ok(None);
    };
    if !crate::memory_card::subject_authorized(session.owner, session.subject, &permissions) {
        return Ok(None);
    }
    Ok(Some(effect(action, index).await))
}

/// Single-press adapter shared by the collector and offline Discord fixtures.
pub(crate) async fn handle_pending_press(
    ctx: &serenity::all::Context,
    press: &ComponentInteraction,
    state: &AppState,
    session: &mut PendingComponentSession,
) -> Result<bool, Error> {
    let result = {
        let session = &*session;
        authorized_pending_effect(
            async {
                press
                    .create_response(&ctx.http, CreateInteractionResponse::Acknowledge)
                    .await
                    .map_err(Error::from)
            },
            session,
            || {
                use serenity::all::{ComponentInteractionDataKind, InteractionContext};
                let scope_matches = match (session.guild, press.guild_id, press.context) {
                    (Some(expected), Some(guild), Some(InteractionContext::Guild)) => {
                        expected == guild.get()
                    }
                    (None, None, Some(InteractionContext::BotDm)) => {
                        session.owner == session.subject
                    }
                    _ => false,
                };
                if press.user.bot
                    || press.user.id.get() != session.owner
                    || press.channel_id.get() != session.channel
                    || !scope_matches
                    || press.message.author.id != ctx.cache.current_user().id
                    || !matches!(press.data.kind, ComponentInteractionDataKind::Button)
                {
                    return None;
                }
                let (action, subject, index) = parse_pending_button_custom_id(
                    press
                        .data
                        .custom_id
                        .strip_suffix(&format!(":v:{}", session.version))?,
                    session.command_id,
                )?;
                (subject == session.subject).then_some((action, index))
            },
            || async {
                match press.guild_id {
                    Some(guild) => crate::commands_help::current_permissions(
                        ctx,
                        guild,
                        press.channel_id,
                        press.user.id,
                    )
                    .await
                    .map(crate::commands_help::permissions_input),
                    None => Ok(Vec::new()),
                }
            },
            |action, index| async move {
                let guild_key = session.guild.map_or_else(
                    || format!("discord:dm:{}", session.owner),
                    |guild| format!("discord:{guild}"),
                );
                let user_key = format!("discord:{}", session.subject);
                let memory = state.memory_service();
                let pending = memory.pending_supersessions(&guild_key, &user_key);
                let status = match pending
                    .get(index)
                    .filter(|entry| session.displayed.get(index) == Some(*entry))
                {
                    Some(entry) => match action {
                        PendingButtonAction::Confirm => {
                            gated_confirm(state, &guild_key, &user_key, &entry.old_fact).await
                        }
                        PendingButtonAction::Dismiss => {
                            if memory.dismiss_supersession(&guild_key, &user_key, &entry.old_fact) {
                                "Dismissed. Both facts are kept.".to_string()
                            } else {
                                "No proposal names that fact.".to_string()
                            }
                        }
                    },
                    None => "That button is stale — refreshing the list.".to_string(),
                };
                let remaining = memory.pending_supersessions(&guild_key, &user_key);
                let body = if remaining.is_empty() {
                    format!("{status}\n\nNothing left proposed.")
                } else {
                    format!(
                        "{status}\n\n{}",
                        format_pending_list_body(session.subject, &remaining)
                    )
                };
                let rows = if remaining.is_empty() {
                    Vec::new()
                } else {
                    pending_action_rows(
                        session.command_id,
                        session.subject,
                        &remaining,
                        session.version + 1,
                    )
                };
                (body, rows, remaining)
            },
        )
        .await?
    };
    let denied = result.is_none();
    let (body, rows, remaining) = result.unwrap_or_else(|| (
        "Discord could not confirm access to these controls. Reopen `/pending list` to try again.".to_string(), Vec::new(), Vec::new()));
    press
        .edit_response(
            &ctx.http,
            EditInteractionResponse::new()
                .content(clamp_message(body))
                .components(rows)
                .allowed_mentions(crate::gateway::no_mentions()),
        )
        .await?;
    let finished = denied || remaining.is_empty();
    session.displayed = remaining;
    session.version += 1;
    Ok(finished)
}

async fn run_pending_component_session(
    ctx: Context<'_>,
    subject: &User,
    displayed: Vec<memory::PendingSupersession>,
) -> Result<(), Error> {
    let mut session = PendingComponentSession {
        command_id: ctx.id(),
        owner: ctx.author().id.get(),
        subject: subject.id.get(),
        guild: ctx.guild_id().map(|guild| guild.get()),
        channel: ctx.channel_id().get(),
        version: 0,
        displayed,
    };
    let serenity_ctx = ctx.serenity_context().clone();
    let id_prefix = format!("{}:p:", session.command_id);
    while let Some(press) = {
        let id_prefix = id_prefix.clone();
        serenity::collector::ComponentInteractionCollector::new(&serenity_ctx)
            .author_id(ctx.author().id)
            .filter(move |press| press.data.custom_id.starts_with(&id_prefix))
            .timeout(Duration::from_secs(PENDING_COMPONENT_TIMEOUT_SECS))
    }
    .await
    {
        if handle_pending_press(&serenity_ctx, &press, &ctx.data().state, &mut session).await? {
            break;
        }
    }
    Ok(())
}

/// Review or resolve supersessions the model proposed but never applied.
///
/// Human-only by construction: there is no model-callable tool that confirms a
/// supersession. A model may propose that one fact replaces another, but only
/// a person decides whether the old fact is actually removed.
#[poise::command(
    slash_command,
    ephemeral,
    subcommands("pending_list", "pending_confirm", "pending_dismiss")
)]
pub async fn pending(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Show supersessions proposed for a member, with nothing removed yet.
///
/// High-traffic path for P1 components UX: classic Action Row Confirm/Dismiss
/// buttons (serenity 0.12 / poise 0.6 have no Components V2 builders yet).
#[poise::command(slash_command, ephemeral, rename = "list")]
pub async fn pending_list(
    ctx: Context<'_>,
    #[description = "Who to review (default: you; moderators may choose another member)"]
    user: Option<User>,
) -> Result<(), Error> {
    // Acknowledge within Discord's 3s window before any store/render work.
    ctx.defer_ephemeral().await?;
    let g = scoped_guild(ctx);
    let subject = user.as_ref().unwrap_or(ctx.author());
    if !memory_subject_authorized(ctx, subject).await {
        ctx.say(CROSS_USER_MEMORY_DENIED).await?;
        return Ok(());
    }
    let u = scoped_user(subject);
    let pending = ctx
        .data()
        .state
        .memory_service()
        .pending_supersessions(&g, &u);
    if pending.is_empty() {
        ctx.say("Nothing proposed. Every remembered fact stands as stored.")
            .await?;
        return Ok(());
    }
    let subject_id = subject.id.get();
    let ctx_id = ctx.id();
    let body = format_pending_list_body(subject_id, &pending);
    let rows = pending_action_rows(ctx_id, subject_id, &pending, 0);
    ctx.send(
        poise::CreateReply::default()
            .content(clamp_message(body))
            .components(rows)
            .ephemeral(true)
            .allowed_mentions(crate::gateway::no_mentions()),
    )
    .await?;
    run_pending_component_session(ctx, subject, pending).await?;
    Ok(())
}

/// Apply one proposed supersession, removing the old fact.
#[poise::command(slash_command, ephemeral, rename = "confirm")]
pub async fn pending_confirm(
    ctx: Context<'_>,
    #[description = "The old fact to remove"]
    #[autocomplete = "autocomplete_pending"]
    old_fact: String,
    #[description = "Who it is about (default: you; moderators may choose another member)"]
    user: Option<User>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let g = scoped_guild(ctx);
    let subject = user.as_ref().unwrap_or(ctx.author());
    if !memory_subject_authorized(ctx, subject).await {
        ctx.say(CROSS_USER_MEMORY_DENIED).await?;
        return Ok(());
    }
    let u = scoped_user(subject);
    let reply = gated_confirm(&ctx.data().state, &g, &u, &old_fact).await;
    ctx.say(clamp_message(reply)).await?;
    Ok(())
}

/// Drop one proposed supersession, keeping both facts.
#[poise::command(slash_command, ephemeral, rename = "dismiss")]
pub async fn pending_dismiss(
    ctx: Context<'_>,
    #[description = "The old fact to keep"]
    #[autocomplete = "autocomplete_pending"]
    old_fact: String,
    #[description = "Who it is about (default: you; moderators may choose another member)"]
    user: Option<User>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let g = scoped_guild(ctx);
    let subject = user.as_ref().unwrap_or(ctx.author());
    if !memory_subject_authorized(ctx, subject).await {
        ctx.say(CROSS_USER_MEMORY_DENIED).await?;
        return Ok(());
    }
    let u = scoped_user(subject);
    let dropped = ctx
        .data()
        .state
        .memory_service()
        .dismiss_supersession(&g, &u, &old_fact);
    ctx.say(if dropped {
        "Dismissed. Both facts are kept."
    } else {
        "No proposal names that fact."
    })
    .await?;
    Ok(())
}

/// What Abbey remembers about a member, and their standing.
#[poise::command(slash_command, ephemeral)]
pub async fn recall(
    ctx: Context<'_>,
    #[description = "Who to look up (default: you; moderators may choose another member)"]
    user: Option<User>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let g = scoped_guild(ctx);
    let subject = user.as_ref().unwrap_or(ctx.author());
    if !memory_subject_authorized(ctx, subject).await {
        ctx.say(CROSS_USER_MEMORY_DENIED).await?;
        return Ok(());
    }
    let (content, has_facts) = memory_card(&ctx.data().state, &g, subject);
    crate::commands_memory_browser::send_summary(ctx, subject.id.get(), content, has_facts).await
}

/// Right-click a guild member and read the same bounded card as `/recall`.
#[poise::command(context_menu_command = "Abbey: memory", guild_only, ephemeral)]
pub async fn memory_context_menu(ctx: Context<'_>, user: User) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(guild_id) = ctx.guild_id() else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    if !memory_subject_authorized(ctx, &user).await {
        ctx.say(CROSS_USER_MEMORY_DENIED).await?;
        return Ok(());
    }
    let guild = guild::scoped_guild_id(PLATFORM, Some(&guild_id.get().to_string()));
    let (content, has_facts) = memory_card(&ctx.data().state, &guild, &user);
    crate::commands_memory_browser::send_summary(ctx, user.id.get(), content, has_facts).await
}

/// Your standing privately, or another member when authorized.
#[poise::command(slash_command, ephemeral)]
pub async fn reputation(
    ctx: Context<'_>,
    #[description = "Who to look up (default: you)"] user: Option<User>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let g = scoped_guild(ctx);
    let subject = user.as_ref().unwrap_or(ctx.author());
    if !memory_subject_authorized(ctx, subject).await {
        ctx.say(CROSS_USER_MEMORY_DENIED).await?;
        return Ok(());
    }
    let u = scoped_user(subject);
    let state = &ctx.data().state;
    let rep = {
        let stores = AppState::lock(&state.stores);
        AppState::lock(&state.social).reputation(&u, &g, &*stores)
    };
    ctx.say(clamp_message(format!(
        "<@{}> — reputation {rep:.2} (0 = poor, 1 = excellent)",
        subject.id.get()
    )))
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Generation-backed
// ---------------------------------------------------------------------------

/// Summarize the recent messages Abbey has seen in this channel.
#[poise::command(slash_command)]
pub async fn summarize(
    ctx: Context<'_>,
    #[description = "How many recent messages (10–200, default 50)"]
    #[min = 10]
    #[max = 200]
    count: Option<usize>,
    #[description = "Force a persona"] r#as: Option<PersonaChoice>,
) -> Result<(), Error> {
    ctx.defer().await?;
    let count = count.unwrap_or(50);
    let state = &ctx.data().state;
    let ch = scoped_channel(ctx);
    let transcript = AppState::lock(&state.stores)
        .memory
        .channel_mut(&ch)
        .render_recent(count);
    if transcript.trim().is_empty() {
        ctx.say("I have not seen any messages in this channel yet — with the MESSAGE_CONTENT intent off, only mentions and DMs reach me.")
            .await?;
        return Ok(());
    }
    let persona = r#as.map_or(crate::persona::Persona::Abbey, Into::into);
    let Some(_) = state.generation_label() else {
        ctx.say(clamp_message(ask::degraded_reply(persona))).await?;
        return Ok(());
    };
    let (system, user) = engine::summarize_prompt(persona, &transcript, count);
    let outcome = state.chat(&system, &[llm::ChatTurn::user(user)]).await;
    let reply = match outcome {
        Ok((summary, provider_label)) => {
            let summary = ask::tidy_reply(persona, &summary);
            AppState::lock(&state.stores)
                .memory
                .channel_mut(&ch)
                .summary
                .clone_from(&summary);
            ask::render_answer(persona, provider_label, &summary)
        }
        Err(e) => {
            tracing::warn!(failure = ?e.provider_failure(), "summary generation failed");
            crate::commands_help::provider_recovery(ctx, e.provider_failure())
                .await
                .to_string()
        }
    };
    ctx.say(clamp_message(reply)).await?;
    Ok(())
}

async fn fetch_attachment(state: &AppState, att: &Attachment) -> Result<Vec<u8>, String> {
    if usize::try_from(att.size).is_ok_and(|s| s > vision::MAX_IMAGE_BYTES) {
        return Err(format!(
            "that image is {} bytes; the cap is {}",
            att.size,
            vision::MAX_IMAGE_BYTES
        ));
    }
    crate::gateway::fetch_capped(&state.attachments, &att.url, vision::MAX_IMAGE_BYTES, None).await
}

/// Describe an image — and answer a question about it if you ask one.
#[poise::command(slash_command)]
pub async fn see(
    ctx: Context<'_>,
    #[description = "The image"] image: Attachment,
    #[description = "Something to ask about it"] question: Option<String>,
) -> Result<(), Error> {
    ctx.defer().await?;
    let state = &ctx.data().state;
    let Some(vision_client) = state.vision() else {
        ctx.say(
            crate::commands_help::provider_recovery(
                ctx,
                crate::provider::ProviderFailureKind::Configuration,
            )
            .await,
        )
        .await?;
        return Ok(());
    };
    let bytes = match fetch_attachment(state, &image).await {
        Ok(b) => b,
        Err(_) => {
            ctx.say("Could not read that attachment. Check that it is available and within the image size limit, then try again.")
            .await?;
            return Ok(());
        }
    };
    let description = match vision_client.describe(bytes).await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(failure = ?e.provider_failure(), "vision description failed");
            ctx.say(crate::commands_help::provider_recovery(ctx, e.provider_failure()).await)
                .await?;
            return Ok(());
        }
    };
    let persona = crate::persona::Persona::Abbey;
    let reply = match (question, state.generation_label()) {
        (Some(q), Some(_)) => {
            let folded = vision::fold_descriptions(&q, &[(image.filename.clone(), description)]);
            let outcome = {
                state
                    .chat(&ask::system_prompt(persona), &[llm::ChatTurn::user(folded)])
                    .await
            };
            match outcome {
                Ok((a, provider_label)) => {
                    ask::render_answer(persona, provider_label, &ask::tidy_reply(persona, &a))
                }
                Err(e) => {
                    tracing::warn!(failure = ?e.provider_failure(), "vision follow-up generation failed");
                    crate::commands_help::provider_recovery(ctx, e.provider_failure())
                        .await
                        .to_string()
                }
            }
        }
        _ => vision::render_see(&persona.to_string(), &description),
    };
    ctx.say(clamp_message(reply)).await?;
    Ok(())
}

/// Transcribe the text in an image.
#[poise::command(slash_command)]
pub async fn ocr(
    ctx: Context<'_>,
    #[description = "The image"] image: Attachment,
) -> Result<(), Error> {
    ctx.defer().await?;
    let state = &ctx.data().state;
    let Some(vision_client) = state.vision() else {
        ctx.say(
            crate::commands_help::provider_recovery(
                ctx,
                crate::provider::ProviderFailureKind::Configuration,
            )
            .await,
        )
        .await?;
        return Ok(());
    };
    let reply = match fetch_attachment(state, &image).await {
        Err(_) => "Could not read that attachment. Check that it is available and within the image size limit, then try again.".to_string(),
        Ok(bytes) => match vision_client.extract_text(bytes).await {
            Ok(text) => vision::render_ocr(&text),
            Err(e) => {
                tracing::warn!(failure = ?e.provider_failure(), "vision OCR failed");
                crate::commands_help::provider_recovery(ctx, e.provider_failure()).await.to_string()
            }
        },
    };
    ctx.say(clamp_message(reply)).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// /stats and /admin
// ---------------------------------------------------------------------------

/// Learning and reply-budget statistics for this server or your DM.
#[poise::command(slash_command, ephemeral)]
pub async fn stats(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let state = &ctx.data().state;
    let g = scoped_guild(ctx);
    let brain_line = {
        let brains = AppState::lock(&state.brains);
        brains.get(&g).map_or_else(
            || "Brain: not loaded for this conversation yet".to_string(),
            |b| {
                format!(
                    "brain: ε {:.3} · steps {} · buffer {} · experiences {}",
                    b.epsilon(),
                    b.step_count(),
                    b.buffer_len(),
                    brains.experience_count(&g).unwrap_or(0)
                )
            },
        )
    };
    let (budget_per_hour, tokens_left) = {
        let mut stores = AppState::lock(&state.stores);
        let settings = AppState::lock(&state.guilds).config(&g, &mut *stores);
        let left = AppState::lock(&state.budget).tokens_left(
            &g,
            settings.unsolicited_per_hour,
            runtime::now(),
        );
        (settings.unsolicited_per_hour, left)
    };
    let text = crate::scoped_stats::render_scoped_stats(&crate::scoped_stats::ScopedStatsInput {
        scope_label: if ctx.guild_id().is_some() {
            "This server"
        } else {
            "Your DM"
        },
        brain_summary: &brain_line,
        budget_per_hour,
        tokens_left,
    });
    send_private_no_mentions(ctx, text).await?;
    Ok(())
}

/// Configure Abbey for this server.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MANAGE_GUILD",
    subcommands(
        "admin_show",
        "admin_persona",
        "admin_learning",
        "admin_vision",
        "admin_cooldown",
        "admin_act",
        "admin_budget",
        "admin_brain",
        "admin_flush",
        "admin_export",
        "admin_reset",
        "admin_dashboard"
    )
)]
pub async fn admin(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

fn update_settings(
    ctx: Context<'_>,
    mutate: impl FnOnce(&mut GuildSettings),
) -> Option<(String, GuildSettings)> {
    ctx.guild_id()?;
    let g = scoped_guild(ctx);
    let state = &ctx.data().state;
    let mut stores = AppState::lock(&state.stores);
    let settings = AppState::lock(&state.guilds).update(&g, &mut *stores, mutate);
    Some((g, settings))
}

/// Show current settings.
#[poise::command(slash_command, guild_only, ephemeral, rename = "show")]
pub async fn admin_show(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some((g, settings)) = update_settings(ctx, |_| {}) else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    ctx.say(clamp_message(guild::render_settings(&g, &settings)))
        .await?;
    Ok(())
}

/// Set the default persona for this server.
#[poise::command(slash_command, guild_only, ephemeral, rename = "persona")]
pub async fn admin_persona(
    ctx: Context<'_>,
    #[description = "Who answers by default"] name: PersonaChoice,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let persona: crate::persona::Persona = name.into();
    let Some(_) = update_settings(ctx, |s| s.default_persona = persona) else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    ctx.say(format!(
        "Default persona for this server: **{}**",
        guild::persona_name(persona)
    ))
    .await?;
    Ok(())
}

/// Toggle adaptive learning (the DQN) for this server.
#[poise::command(slash_command, guild_only, ephemeral, rename = "learning")]
pub async fn admin_learning(
    ctx: Context<'_>,
    #[description = "on | off"] state: OnOff,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let on = state.is_on();
    let Some((g, _)) = update_settings(ctx, |s| s.learning_enabled = on) else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    ctx.say(format!(
        "learning is now **{}** for this server.",
        state.label()
    ))
    .await?;
    // Mirror the request into the constitutional ledger when the operator
    // configured the gate. The toggle above already applied; this never
    // blocks or fails the command, and it logs its own outcome.
    if let Some(gate) = ctx.data().state.gate_for(&g).cloned() {
        let request = LearningToggleRequest {
            scoped_guild: g,
            scoped_user: scoped_user(ctx.author()),
            now: runtime::now(),
            nonce: gate.next_nonce(),
        };
        ctx.data().state.spawn_episode(async move {
            gate.record_learning_toggle(request).await;
        });
    }
    Ok(())
}

/// Toggle image understanding for this server.
#[poise::command(slash_command, guild_only, ephemeral, rename = "vision")]
pub async fn admin_vision(
    ctx: Context<'_>,
    #[description = "on | off"] state: OnOff,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let on = state.is_on();
    let Some(_) = update_settings(ctx, |s| s.vision_enabled = on) else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    ctx.say(format!(
        "vision is now **{}** for this server.",
        state.label()
    ))
    .await?;
    Ok(())
}

/// Minimum seconds between unsolicited replies in a channel (0–600).
#[poise::command(slash_command, guild_only, ephemeral, rename = "cooldown")]
pub async fn admin_cooldown(
    ctx: Context<'_>,
    #[description = "0–600"] seconds: i64,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let secs = guild::clamp_cooldown(seconds);
    let Some(_) = update_settings(ctx, |s| s.reply_cooldown_seconds = secs) else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    ctx.say(format!("Reply cooldown: **{secs}s**")).await?;
    Ok(())
}

/// Let Abbey speak unsolicited in this server (the per-guild policy decides).
#[poise::command(slash_command, guild_only, ephemeral, rename = "act")]
pub async fn admin_act(
    ctx: Context<'_>,
    #[description = "on | off"] state: OnOff,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let on = state.is_on();
    let Some(_) = update_settings(ctx, |s| s.unsolicited = on) else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    ctx.say(if on {
        "Abbey may now speak unsolicited here — bounded by the cooldown and the hourly budget (`/admin budget`). `ABBEY_QUIET=1` on the host still silences her."
    } else {
        "Abbey will only answer mentions, DMs, and commands here."
    })
    .await?;
    Ok(())
}

/// Unsolicited actions allowed per hour in this server (1–60).
#[poise::command(slash_command, guild_only, ephemeral, rename = "budget")]
pub async fn admin_budget(
    ctx: Context<'_>,
    #[description = "1–60"] per_hour: i64,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let n = guild::clamp_budget(per_hour);
    let Some(_) = update_settings(ctx, |s| s.unsolicited_per_hour = n) else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    ctx.say(format!("Unsolicited budget: **{n}/h** for this server."))
        .await?;
    Ok(())
}

/// Inspect this server's policy: ε, steps, buffer fill, experiences.
#[poise::command(slash_command, guild_only, ephemeral, rename = "brain")]
pub async fn admin_brain(
    ctx: Context<'_>,
    #[description = "Override exploration ε (0–1); omit to show"] epsilon: Option<f64>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let g = scoped_guild(ctx);
    let state = &ctx.data().state;
    let override_eps = epsilon.map(guild::clamp_epsilon);
    if override_eps.is_some() {
        update_settings(ctx, |s| s.epsilon_override = override_eps);
    }
    let text = {
        let now = runtime::now();
        let (settings, tokens_left) = {
            let mut stores = AppState::lock(&state.stores);
            let settings = AppState::lock(&state.guilds).config(&g, &mut *stores);
            let tokens_left =
                AppState::lock(&state.budget).tokens_left(&g, settings.unsolicited_per_hour, now);
            (settings, tokens_left)
        };
        let stores = AppState::lock(&state.stores);
        let mut brains = AppState::lock(&state.brains);
        let brain = brains.brain(&g, &*stores, now);
        if let Some(eps) = override_eps {
            brain.set_epsilon(eps);
        }
        let (eps, steps, buffer) = (brain.epsilon(), brain.step_count(), brain.buffer_len());
        let experiences = brains.experience_count(&g).unwrap_or(0);
        let view = BrainView {
            scoped_guild_id: &g,
            epsilon: eps,
            learn_steps: steps,
            buffer_len: buffer,
            buffer_capacity: runtime::REPLAY_CAPACITY,
            experiences,
            budget_per_hour: settings.unsolicited_per_hour,
            tokens_left,
            topology: &runtime::TOPOLOGY,
        };
        let stats = brains.stats(&g).cloned().unwrap_or_default();
        format!(
            "{}\nact: {}",
            stats.render(&view),
            if settings.unsolicited { "on" } else { "off" }
        )
    };
    ctx.say(clamp_message(text)).await?;
    Ok(())
}

/// Flush reputation and persist everything to disk now.
fn render_admin_flush(report: &PersistReport) -> String {
    format!(
        "Persistence is {}. Canonical state: {}. WDBX projection: {}.",
        report.overall.as_str(),
        render_component_outcome(report.canonical_state),
        render_component_outcome(report.wdbx_projection)
    )
}

fn render_persistence_result(report: &PersistReport) -> String {
    format!(
        "{}\n\n{}",
        render_admin_flush(report),
        crate::operator_guidance::persistence_guidance(report)
    )
}

#[poise::command(slash_command, guild_only, ephemeral, rename = "flush")]
pub async fn admin_flush(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let state = &ctx.data().state;
    let content = match state.request_persistence().await {
        Ok(report) => render_persistence_result(&report),
        Err(error) => error.to_string(),
    };
    ctx.say(clamp_message(content)).await?;
    Ok(())
}

/// Export this server's brain snapshot as JSON.
#[poise::command(slash_command, guild_only, ephemeral, rename = "export")]
pub async fn admin_export(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let g = scoped_guild(ctx);
    let state = &ctx.data().state;
    let json = {
        let stores = AppState::lock(&state.stores);
        let mut brains = AppState::lock(&state.brains);
        let brain = brains.brain(&g, &*stores, runtime::now());
        serde_json::to_vec_pretty(&brain.export_weights()).unwrap_or_default()
    };
    let filename = format!("{}-brain.json", g.replace(':', "-"));
    ctx.send(
        poise::CreateReply::default()
            .content("Brain snapshot attached.")
            .attachment(CreateAttachment::bytes(json, filename)),
    )
    .await?;
    Ok(())
}

/// Reset this channel's conversation memory (the multi-turn transcript).
#[poise::command(slash_command, guild_only, ephemeral, rename = "reset")]
pub async fn admin_reset(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let state = &ctx.data().state;
    let ch = scoped_channel(ctx);
    let had = AppState::lock(&state.engine).reset(&ch);
    ctx.say(if had {
        "Conversation transcript for this channel cleared."
    } else {
        "There was no transcript for this channel."
    })
    .await?;
    Ok(())
}

/// Open the owner- and guild-bound classic administration dashboard.
#[poise::command(slash_command, guild_only, ephemeral, rename = "dashboard")]
pub async fn admin_dashboard(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(guild) = ctx.guild_id() else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    let session = crate::admin_dashboard::AdminSession {
        owner: ctx.author().id.get(),
        guild: guild.get(),
        expiry: runtime::now().saturating_add(crate::admin_dashboard::SESSION_SECONDS),
        page: crate::admin_dashboard::AdminPage::Overview,
    };
    let input = dashboard_input(ctx.data(), guild.get(), None);
    ctx.send(
        poise::CreateReply::default()
            .content(clamp_message(crate::admin_dashboard::render(
                session.page,
                &input,
            )))
            .components(dashboard_rows(&session))
            .ephemeral(true)
            .allowed_mentions(crate::gateway::no_mentions()),
    )
    .await?;
    Ok(())
}

fn dashboard_settings(state: &AppState, guild_id: u64) -> GuildSettings {
    let scoped = guild::scoped_guild_id(PLATFORM, Some(&guild_id.to_string()));
    let mut stores = AppState::lock(&state.stores);
    AppState::lock(&state.guilds).refresh(&scoped, &mut *stores)
}

fn dashboard_input(
    data: &crate::Data,
    guild_id: u64,
    result: Option<String>,
) -> crate::admin_dashboard::AdminViewInput {
    let scoped = guild::scoped_guild_id(PLATFORM, Some(&guild_id.to_string()));
    let settings = dashboard_settings(&data.state, guild_id);
    let (epsilon, brain_summary) = {
        let stores = AppState::lock(&data.state.stores);
        let mut brains = AppState::lock(&data.state.brains);
        let brain = brains.brain(&scoped, &*stores, runtime::now());
        (
            brain.epsilon(),
            format!(
                "Steps: {} · replay: {} · experiences: {}",
                brain.step_count(),
                brain.buffer_len(),
                brains.experience_count(&scoped).unwrap_or(0)
            ),
        )
    };
    let mut capabilities = vec!["memory"];
    if data.state.providers.generation_available() {
        capabilities.push("generation");
    }
    if data.state.providers.vision_available() {
        capabilities.push("vision");
    }
    if data
        .voice
        .as_ref()
        .is_some_and(|voice| voice.config.guild_id == guild_id)
    {
        capabilities.push("voice");
    }
    crate::admin_dashboard::AdminViewInput {
        settings,
        epsilon,
        brain_summary,
        capabilities,
        operation_result: result,
    }
}

fn dashboard_rows(session: &crate::admin_dashboard::AdminSession) -> Vec<CreateActionRow> {
    use crate::admin_dashboard::{AdminAction as A, AdminPage as P};
    let nav = [
        (P::Overview, "Overview"),
        (P::Conversation, "Conversation"),
        (P::Learning, "Learning"),
        (P::Operations, "Operations"),
    ]
    .into_iter()
    .map(|(page, label)| {
        CreateButton::new(session.custom_id(A::View(page)))
            .label(label)
            .style(if page == session.page {
                ButtonStyle::Primary
            } else {
                ButtonStyle::Secondary
            })
    })
    .collect();
    let action_rows = match session.page {
        P::Conversation => vec![
            vec![
                CreateButton::new(session.custom_id(A::SetVision(true)))
                    .label("Vision on")
                    .style(ButtonStyle::Success),
                CreateButton::new(session.custom_id(A::SetVision(false)))
                    .label("Vision off")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(session.custom_id(A::SetUnsolicited(true)))
                    .label("Act on")
                    .style(ButtonStyle::Success),
                CreateButton::new(session.custom_id(A::SetUnsolicited(false)))
                    .label("Act off")
                    .style(ButtonStyle::Secondary),
            ],
            vec![
                CreateButton::new(session.custom_id(A::SetPersona(crate::persona::Persona::Abbey)))
                    .label("Persona Abbey")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(session.custom_id(A::SetPersona(crate::persona::Persona::Aviva)))
                    .label("Persona Aviva")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(session.custom_id(A::SetPersona(crate::persona::Persona::Abi)))
                    .label("Persona Abi")
                    .style(ButtonStyle::Secondary),
            ],
            vec![
                CreateButton::new(session.custom_id(A::SetCooldown(0)))
                    .label("Cooldown 0s")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(session.custom_id(A::SetCooldown(20)))
                    .label("Cooldown 20s")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(session.custom_id(A::SetCooldown(60)))
                    .label("Cooldown 60s")
                    .style(ButtonStyle::Secondary),
            ],
        ],
        P::Learning => vec![
            vec![
                CreateButton::new(session.custom_id(A::SetLearning(true)))
                    .label("Learning on")
                    .style(ButtonStyle::Success),
                CreateButton::new(session.custom_id(A::SetLearning(false)))
                    .label("Learning off")
                    .style(ButtonStyle::Secondary),
            ],
            vec![
                CreateButton::new(session.custom_id(A::SetBudget(1)))
                    .label("Budget 1/h")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(session.custom_id(A::SetBudget(6)))
                    .label("Budget 6/h")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(session.custom_id(A::SetBudget(60)))
                    .label("Budget 60/h")
                    .style(ButtonStyle::Secondary),
            ],
            vec![
                CreateButton::new(session.custom_id(A::SetEpsilon(5)))
                    .label("Epsilon .05")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(session.custom_id(A::SetEpsilon(20)))
                    .label("Epsilon .20")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(session.custom_id(A::SetEpsilon(50)))
                    .label("Epsilon .50")
                    .style(ButtonStyle::Secondary),
            ],
        ],
        P::Operations => vec![vec![
            CreateButton::new(session.custom_id(A::Flush))
                .label("Flush")
                .style(ButtonStyle::Primary),
            CreateButton::new(session.custom_id(A::Export))
                .label("Export")
                .style(ButtonStyle::Secondary),
            CreateButton::new(session.custom_id(A::RequestReset))
                .label("Reset transcript…")
                .style(ButtonStyle::Danger),
        ]],
        P::ConfirmReset => vec![vec![
            CreateButton::new(session.custom_id(A::ConfirmReset))
                .label("Confirm channel reset")
                .style(ButtonStyle::Danger),
            CreateButton::new(session.custom_id(A::View(P::Operations)))
                .label("Cancel")
                .style(ButtonStyle::Secondary),
        ]],
        P::Overview => Vec::new(),
    };
    let mut rows = vec![CreateActionRow::Buttons(nav)];
    for actions in action_rows {
        rows.push(CreateActionRow::Buttons(actions));
    }
    rows
}

/// Central admin protocol adapter. It always acknowledges before validation,
/// permission REST reads, state reloads, or mutations.
enum AdminPreparation {
    Rejected(crate::admin_dashboard::Rejection),
    LookupUnavailable,
    PermissionDenied,
    Ready(
        crate::admin_dashboard::AdminSession,
        crate::admin_dashboard::AdminAction,
        GuildSettings,
    ),
}

async fn acknowledged_admin_preparation<A, Validate, Load, L>(
    acknowledgement: A,
    validate: Validate,
    load: Load,
) -> Result<AdminPreparation, Error>
where
    A: std::future::Future<Output = Result<(), Error>>,
    Validate: FnOnce() -> Result<
        (
            crate::admin_dashboard::AdminSession,
            crate::admin_dashboard::AdminAction,
        ),
        crate::admin_dashboard::Rejection,
    >,
    Load: FnOnce() -> L,
    L: std::future::Future<Output = Result<(Permissions, GuildSettings), Error>>,
{
    acknowledgement.await?;
    let (session, action) = match validate() {
        Ok(value) => value,
        Err(error) => return Ok(AdminPreparation::Rejected(error)),
    };
    let (permissions, settings) = match load().await {
        Ok(value) => value,
        Err(_) => return Ok(AdminPreparation::LookupUnavailable),
    };
    if !permissions.contains(Permissions::MANAGE_GUILD)
        && !permissions.contains(Permissions::ADMINISTRATOR)
    {
        return Ok(AdminPreparation::PermissionDenied);
    }
    Ok(AdminPreparation::Ready(session, action, settings))
}

pub async fn dispatch_admin_component(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &crate::Data,
) -> bool {
    if !interaction.data.custom_id.starts_with("abbey:admin:") {
        return false;
    }
    let preparation = acknowledged_admin_preparation(
        async {
            interaction
                .create_response(
                    &ctx.http,
                    CreateInteractionResponse::Defer(
                        CreateInteractionResponseMessage::new().ephemeral(true),
                    ),
                )
                .await
                .map_err(Error::from)
        },
        || {
            if interaction.user.bot || interaction.message.author.id != ctx.cache.current_user().id
            {
                return Err(crate::admin_dashboard::Rejection::Malformed);
            }
            crate::admin_dashboard::AdminSession::parse(
                &interaction.data.custom_id,
                interaction.user.id.get(),
                interaction.guild_id.map(|id| id.get()),
                runtime::now(),
            )
        },
        || async {
            let guild_id = interaction
                .guild_id
                .ok_or("Administration guild is unavailable.")?;
            let permissions = crate::commands_help::current_permissions(
                ctx,
                guild_id,
                interaction.channel_id,
                interaction.user.id,
            )
            .await?;
            let settings = dashboard_settings(&data.state, guild_id.get());
            Ok((permissions, settings))
        },
    )
    .await;
    let (mut session, action, current) = match preparation {
        Err(_) => return true,
        Ok(AdminPreparation::Rejected(error)) => {
            edit_admin(ctx, interaction, error.message(), Vec::new()).await;
            return true;
        }
        Ok(AdminPreparation::LookupUnavailable) => {
            edit_admin(
                ctx,
                interaction,
                "Discord could not confirm your current permission. Nothing changed.",
                Vec::new(),
            )
            .await;
            return true;
        }
        Ok(AdminPreparation::PermissionDenied) => {
            edit_admin(
                ctx,
                interaction,
                "Discord could not confirm that you currently have Manage Server.",
                Vec::new(),
            )
            .await;
            return true;
        }
        Ok(AdminPreparation::Ready(session, action, current)) => (session, action, current),
    };
    let guild_id = serenity::all::GuildId::new(session.guild);
    let effect = crate::admin_dashboard::reduce(action, &current);
    let mut result = None;
    use crate::admin_dashboard::AdminEffect;
    match effect {
        AdminEffect::View(page) => session.page = page,
        AdminEffect::SetLearning(value) => {
            update_dashboard_setting(data, guild_id.get(), |s| s.learning_enabled = value);
            result = Some(format!(
                "Learning is now **{}**.",
                if value { "on" } else { "off" }
            ));
            let scoped = guild::scoped_guild_id(PLATFORM, Some(&guild_id.get().to_string()));
            if let Some(gate) = data.state.gate_for(&scoped).cloned() {
                let request = LearningToggleRequest {
                    scoped_guild: scoped,
                    scoped_user: guild::scoped_user_id(
                        PLATFORM,
                        &interaction.user.id.get().to_string(),
                    ),
                    now: runtime::now(),
                    nonce: gate.next_nonce(),
                };
                data.state.spawn_episode(async move {
                    gate.record_learning_toggle(request).await;
                });
            }
        }
        AdminEffect::SetVision(value) => {
            update_dashboard_setting(data, guild_id.get(), |s| s.vision_enabled = value);
            result = Some(format!(
                "Vision is now **{}**.",
                if value { "on" } else { "off" }
            ));
        }
        AdminEffect::SetUnsolicited(value) => {
            update_dashboard_setting(data, guild_id.get(), |s| s.unsolicited = value);
            result = Some(format!(
                "Unsolicited action is now **{}**.",
                if value { "on" } else { "off" }
            ));
        }
        AdminEffect::SetPersona(value) => {
            update_dashboard_setting(data, guild_id.get(), |s| s.default_persona = value);
            result = Some(format!(
                "Default persona is now **{}**.",
                guild::persona_name(value)
            ));
        }
        AdminEffect::SetCooldown(value) => {
            update_dashboard_setting(data, guild_id.get(), |s| {
                s.reply_cooldown_seconds = guild::clamp_cooldown(i64::from(value))
            });
            result = Some(format!("Reply cooldown is now **{value}s**."));
        }
        AdminEffect::SetBudget(value) => {
            update_dashboard_setting(data, guild_id.get(), |s| {
                s.unsolicited_per_hour = guild::clamp_budget(i64::from(value))
            });
            result = Some(format!("Unsolicited budget is now **{value}/h**."));
        }
        AdminEffect::SetEpsilon(value) => {
            let epsilon = guild::clamp_epsilon(f64::from(value) / 100.0);
            update_dashboard_setting(data, guild_id.get(), |s| s.epsilon_override = Some(epsilon));
            let scoped = guild::scoped_guild_id(PLATFORM, Some(&guild_id.get().to_string()));
            let stores = AppState::lock(&data.state.stores);
            AppState::lock(&data.state.brains)
                .brain(&scoped, &*stores, runtime::now())
                .set_epsilon(epsilon);
            result = Some(format!("Exploration epsilon is now **{epsilon:.2}**."));
        }
        AdminEffect::Persist => {
            result = Some(match data.state.request_persistence().await {
                Ok(report) => render_persistence_result(&report),
                Err(error) => error.to_string(),
            });
        }
        AdminEffect::ResetChannel => {
            let scope =
                guild::scoped_channel_id(PLATFORM, &interaction.channel_id.get().to_string());
            result = Some(
                if AppState::lock(&data.state.engine).reset(&scope) {
                    "Conversation transcript for this channel cleared."
                } else {
                    "Conversation transcript for this channel was already clear."
                }
                .into(),
            );
        }
        AdminEffect::Export => {
            let scoped = guild::scoped_guild_id(PLATFORM, Some(&guild_id.get().to_string()));
            let bytes = {
                let stores = AppState::lock(&data.state.stores);
                let mut brains = AppState::lock(&data.state.brains);
                serde_json::to_vec_pretty(
                    &brains
                        .brain(&scoped, &*stores, runtime::now())
                        .export_weights(),
                )
                .unwrap_or_default()
            };
            let _ = interaction
                .edit_response(
                    &ctx.http,
                    EditInteractionResponse::new()
                        .content("Brain snapshot attached privately.")
                        .new_attachment(CreateAttachment::bytes(
                            bytes,
                            format!("{}-brain.json", scoped.replace(':', "-")),
                        ))
                        .components(dashboard_rows(&session))
                        .allowed_mentions(crate::gateway::no_mentions()),
                )
                .await;
            return true;
        }
        AdminEffect::None => result = Some("That setting already has the requested value.".into()),
    }
    let input = dashboard_input(data, guild_id.get(), result);
    edit_admin(
        ctx,
        interaction,
        &crate::admin_dashboard::render(session.page, &input),
        dashboard_rows(&session),
    )
    .await;
    true
}

fn update_dashboard_setting(
    data: &crate::Data,
    guild_id: u64,
    mutate: impl FnOnce(&mut GuildSettings),
) {
    let scoped = guild::scoped_guild_id(PLATFORM, Some(&guild_id.to_string()));
    let mut stores = AppState::lock(&data.state.stores);
    AppState::lock(&data.state.guilds).update(&scoped, &mut *stores, mutate);
}

async fn edit_admin(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    content: &str,
    rows: Vec<CreateActionRow>,
) {
    let _ = interaction
        .edit_response(
            &ctx.http,
            EditInteractionResponse::new()
                .content(clamp_message(content.to_owned()))
                .components(rows)
                .allowed_mentions(crate::gateway::no_mentions()),
        )
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persist::{PersistComponentOutcome, PersistErrorCategory, PersistReport};

    #[test]
    fn on_off_labels() {
        assert!(OnOff::On.is_on());
        assert_eq!(OnOff::Off.label(), "off");
    }

    #[test]
    fn admin_flush_copy_is_truthful_component_level_and_content_free() {
        let report = PersistReport::from_components(
            PersistComponentOutcome::Committed,
            PersistComponentOutcome::Failed(PersistErrorCategory::SyncDirectory),
        );
        let rendered = render_admin_flush(&report);
        assert_eq!(
            rendered,
            "Persistence is partial. Canonical state: committed. WDBX projection: failed (sync-directory)."
        );
        assert!(!rendered.contains('/'));
        assert!(!rendered.contains("injected"));

        assert_eq!(
            render_admin_flush(&PersistReport::memory_only()),
            "Persistence is memory-only. Canonical state: not configured. WDBX projection: not configured."
        );
        assert_eq!(
            render_admin_flush(&PersistReport::from_components(
                PersistComponentOutcome::Committed,
                PersistComponentOutcome::Committed,
            )),
            "Persistence is complete. Canonical state: committed. WDBX projection: committed."
        );
        assert_eq!(
            render_admin_flush(&PersistReport::from_components(
                PersistComponentOutcome::Failed(PersistErrorCategory::WriteTemporary),
                PersistComponentOutcome::SkippedCanonicalFailure,
            )),
            "Persistence is failed. Canonical state: failed (write-temporary). WDBX projection: skipped after canonical failure."
        );
    }

    #[test]
    fn facts_are_whitespace_normalized_and_character_bounded() {
        assert_eq!(
            memory::validated_fact("  Donald\nlikes\tRust.  "),
            Ok("Donald likes Rust.".to_string())
        );
        assert_eq!(
            memory::validated_fact(" \n\t "),
            Err("The fact must contain some text.")
        );
        assert!(memory::validated_fact(&"x".repeat(memory::MAX_FACT_CHARS)).is_ok());
        assert_eq!(
            memory::validated_fact(&"🦀".repeat(memory::MAX_FACT_CHARS + 1)),
            Err("Keep one remembered fact to 300 characters or fewer.")
        );
    }

    #[test]
    fn memory_read_adapters_are_private_and_have_the_required_contexts() {
        let reputation = reputation();
        assert!(reputation.ephemeral);
        assert!(!reputation.guild_only);

        let memory = memory_context_menu();
        assert!(memory.ephemeral);
        assert!(memory.guild_only);
        assert!(matches!(
            memory.context_menu_action,
            Some(poise::ContextMenuCommandAction::User(_))
        ));
    }

    #[tokio::test]
    async fn dashboard_adapter_acknowledges_before_validation_permission_and_reload() {
        use std::sync::{Arc, Mutex};
        let events = Arc::new(Mutex::new(Vec::new()));
        let ack_events = Arc::clone(&events);
        let validate_events = Arc::clone(&events);
        let load_events = Arc::clone(&events);
        let session = crate::admin_dashboard::AdminSession {
            owner: 1,
            guild: 2,
            expiry: 3,
            page: crate::admin_dashboard::AdminPage::Overview,
        };
        let prepared = acknowledged_admin_preparation(
            async move {
                ack_events.lock().unwrap().push("ack");
                Ok(())
            },
            || {
                validate_events.lock().unwrap().push("validate");
                Ok((
                    session,
                    crate::admin_dashboard::AdminAction::SetLearning(true),
                ))
            },
            || async move {
                load_events.lock().unwrap().push("permissions+reload");
                Ok((Permissions::MANAGE_GUILD, GuildSettings::default()))
            },
        )
        .await
        .unwrap();
        assert!(matches!(prepared, AdminPreparation::Ready(..)));
        assert_eq!(
            *events.lock().unwrap(),
            ["ack", "validate", "permissions+reload"]
        );
    }

    #[tokio::test]
    async fn revoked_permission_fails_before_any_effect_is_reduced() {
        let prepared = acknowledged_admin_preparation(
            async { Ok(()) },
            || {
                Ok((
                    crate::admin_dashboard::AdminSession {
                        owner: 1,
                        guild: 2,
                        expiry: 3,
                        page: crate::admin_dashboard::AdminPage::Learning,
                    },
                    crate::admin_dashboard::AdminAction::SetLearning(true),
                ))
            },
            || async { Ok((Permissions::empty(), GuildSettings::default())) },
        )
        .await
        .unwrap();
        assert!(matches!(prepared, AdminPreparation::PermissionDenied));
    }

    #[test]
    fn dashboard_uses_classic_bounded_rows_and_private_registration() {
        let command = admin_dashboard();
        assert!(command.ephemeral);
        for page in [
            crate::admin_dashboard::AdminPage::Overview,
            crate::admin_dashboard::AdminPage::Conversation,
            crate::admin_dashboard::AdminPage::Learning,
            crate::admin_dashboard::AdminPage::Operations,
            crate::admin_dashboard::AdminPage::ConfirmReset,
        ] {
            let session = crate::admin_dashboard::AdminSession {
                owner: u64::MAX,
                guild: u64::MAX,
                expiry: u64::MAX,
                page,
            };
            let rows = dashboard_rows(&session);
            assert!(rows.len() <= 5);
            for row in rows {
                let CreateActionRow::Buttons(buttons) = row else {
                    panic!("dashboard must use classic button rows")
                };
                assert!(buttons.len() <= 5);
            }
        }
    }
}

#[cfg(test)]
mod pending_components_tests {
    use super::{
        PendingButtonAction, format_pending_list_body, parse_pending_button_custom_id,
        pending_action_rows, pending_button_custom_id,
    };
    use crate::memory::PendingSupersession;

    #[test]
    fn custom_id_round_trips() {
        let id = pending_button_custom_id(42, PendingButtonAction::Confirm, 99, 3);
        assert_eq!(id, "42:p:c:99:3");
        assert_eq!(
            parse_pending_button_custom_id(&id, 42),
            Some((PendingButtonAction::Confirm, 99, 3))
        );
        assert!(parse_pending_button_custom_id(&id, 7).is_none());
        assert!(parse_pending_button_custom_id("42:p:x:99:3", 42).is_none());
        assert!(parse_pending_button_custom_id("42:p:c:99:3:extra", 42).is_none());
    }

    #[test]
    fn action_rows_cap_at_five() {
        let pending: Vec<_> = (0..7)
            .map(|i| PendingSupersession {
                old_fact: format!("old-{i}"),
                new_fact: format!("new-{i}"),
                at: i as u64,
            })
            .collect();
        let rows = pending_action_rows(1, 2, &pending, 0);
        assert_eq!(rows.len(), 5);
        let body = format_pending_list_body(2, &pending);
        assert!(body.contains("Buttons cover the first 5"));
        assert!(body.contains("1. old-0 → new-0"));
    }
}

#[cfg(test)]
mod pending_authorization_tests {
    use super::*;
    #[tokio::test]
    async fn acknowledgement_failure_and_permission_revocation_construct_no_gate_or_snapshot() {
        use std::cell::Cell;
        let session = PendingComponentSession {
            command_id: 1,
            owner: 2,
            subject: 3,
            guild: Some(4),
            channel: 5,
            version: 0,
            displayed: Vec::new(),
        };
        let failed: Result<Option<()>, Error> = authorized_pending_effect(
            async { Err("synthetic acknowledgement failure".into()) },
            &session,
            || panic!("validation preceded acknowledgement"),
            || async { panic!("permissions preceded acknowledgement") },
            |_, _| async { panic!("gate preceded acknowledgement") },
        )
        .await;
        assert!(failed.is_err());
        let snapshots_and_gates = Cell::new(0);
        let denied = authorized_pending_effect(
            async { Ok(()) },
            &session,
            || Some((PendingButtonAction::Confirm, 0)),
            || async { Ok(Vec::new()) },
            |_, _| {
                snapshots_and_gates.set(snapshots_and_gates.get() + 1);
                async {}
            },
        )
        .await
        .unwrap();
        assert!(denied.is_none());
        assert_eq!(snapshots_and_gates.get(), 0);
        let order = std::cell::RefCell::new(Vec::new());
        authorized_pending_effect(
            async {
                order.borrow_mut().push("ack");
                Ok(())
            },
            &session,
            || {
                order.borrow_mut().push("envelope");
                Some((PendingButtonAction::Confirm, 0))
            },
            || {
                order.borrow_mut().push("permissions constructed");
                async {
                    Ok(vec![
                        crate::command_catalog::DiscordPermission::ManageMessages,
                    ])
                }
            },
            |_, _| {
                order.borrow_mut().push("snapshot and gate constructed");
                async {}
            },
        )
        .await
        .unwrap();
        assert_eq!(
            *order.borrow(),
            [
                "ack",
                "envelope",
                "permissions constructed",
                "snapshot and gate constructed"
            ]
        );
    }
}

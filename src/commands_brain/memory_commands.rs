//! Focused memory commands command adapters.
use super::*;

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
pub(super) enum PendingButtonAction {
    Confirm,
    Dismiss,
}

pub(super) fn pending_button_custom_id(
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

pub(super) fn parse_pending_button_custom_id(
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

pub(super) fn format_pending_list_body(
    subject_id: u64,
    pending: &[memory::PendingSupersession],
) -> String {
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

pub(super) fn pending_action_rows(
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
pub(super) async fn authorized_pending_effect<A, V, P, PF, E, EF, T>(
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

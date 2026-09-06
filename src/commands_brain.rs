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

mod dashboard;
mod media;
mod memory_commands;

pub(crate) use dashboard::open_dashboard_component;
pub use dashboard::{admin_dashboard, dispatch_admin_component};
pub use media::{ocr, see, summarize};
#[cfg(test)]
pub(crate) use memory_commands::{PendingComponentSession, handle_pending_press};
pub use memory_commands::{
    forget, memory_context_menu, pending_confirm, pending_dismiss, pending_list, recall, remember,
    reputation,
};

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

// ---------------------------------------------------------------------------
// Generation-backed
// ---------------------------------------------------------------------------

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
    let Some((_, settings)) = update_settings(ctx, |s| s.unsolicited = on) else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    ctx.say(clamp_message(crate::admin_dashboard::unsolicited_status(
        &settings,
        ctx.data().state.quiet,
        ctx.data()
            .state
            .providers
            .request_readiness(crate::provider::RequestClass::text(
                ctx.data().state.providers.tools_enabled(),
            ))
            .is_ok(),
    )))
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

pub(super) fn render_persistence_result(report: &PersistReport) -> String {
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

#[cfg(test)]
mod tests;

#[cfg(test)]
mod pending_components_tests;

#[cfg(test)]
mod pending_authorization_tests;

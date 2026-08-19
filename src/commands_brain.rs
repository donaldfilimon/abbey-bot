//! Slash commands over the learning, memory, and config surfaces
//! (`docs/spec/companionapp.md` "Full slash-command surface",
//! `docs/spec/multiguild.md` "/admin").
//!
//! Same contract as `commands.rs`: defer first, clamp every rendered answer,
//! translate Discord data into plain values and hand them to the pure modules.
//! Commands that change per-guild config or forget facts are gated by
//! Discord's own `default_member_permissions`, the way `/modcall` is.

use serenity::all::{Attachment, CreateAttachment, User};

use crate::ask;
use crate::brain::state::BotAction;
use crate::commands::{PersonaChoice, clamp_message};
use crate::engine;
use crate::guild::{self, GuildSettings};
use crate::llm;
use crate::memory;
use crate::runtime::{self, AppState};
use crate::vision::{self, ImageUnderstanding};
use crate::{Context, Error};

const NO_GUILD: &str = "This one only works inside a server.";

const PLATFORM: &str = "discord";

fn scoped_guild(ctx: Context<'_>) -> Option<String> {
    ctx.guild_id()
        .map(|g| guild::scoped_guild_id(PLATFORM, Some(&g.get().to_string())))
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

/// Store a durable fact about a member (yourself by default).
#[poise::command(slash_command, guild_only, ephemeral)]
pub async fn remember(
    ctx: Context<'_>,
    #[description = "A single concise fact, stated in third person"] fact: String,
    #[description = "Who it is about (default: you)"] user: Option<User>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(g) = scoped_guild(ctx) else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    let subject = user.as_ref().unwrap_or(ctx.author());
    let u = scoped_user(subject);
    let state = &ctx.data().state;
    let now = runtime::now();
    let stored = AppState::lock(&state.stores)
        .memory
        .remember(&g, &u, &fact, now);
    let reply = if stored {
        AppState::lock(&state.recall).remember(&g, &u, &fact, now);
        format!("Stored about <@{}>: {fact}", subject.id.get())
    } else {
        "Already on record (or the fact list is full).".to_string()
    };
    ctx.say(clamp_message(reply)).await?;
    Ok(())
}

async fn autocomplete_fact(ctx: Context<'_>, partial: &str) -> Vec<String> {
    let Some(g) = scoped_guild(ctx) else {
        return Vec::new();
    };
    let u = scoped_user(ctx.author());
    let stores = AppState::lock(&ctx.data().state.stores);
    memory::autocomplete_facts(stores.memory.facts(&g, &u), partial)
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// Forget one of your stored facts.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MANAGE_MESSAGES"
)]
pub async fn forget(
    ctx: Context<'_>,
    #[description = "The fact to remove"]
    #[autocomplete = "autocomplete_fact"]
    fact: String,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(g) = scoped_guild(ctx) else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    let u = scoped_user(ctx.author());
    let state = &ctx.data().state;
    let removed = AppState::lock(&state.stores).memory.forget(&g, &u, &fact);
    if removed {
        let ids: Vec<u64> = AppState::lock(&state.recall)
            .facts_for_user(&g, &u)
            .into_iter()
            .filter(|f| f.text == fact)
            .map(|f| f.id)
            .collect();
        let mut recall = AppState::lock(&state.recall);
        for id in ids {
            recall.forget(&g, id);
        }
    }
    ctx.say(if removed {
        "Forgotten."
    } else {
        "Nothing by that wording was on record."
    })
    .await?;
    Ok(())
}

/// What Abbey remembers about a member, and their standing.
#[poise::command(slash_command, guild_only, ephemeral)]
pub async fn recall(
    ctx: Context<'_>,
    #[description = "Who to look up (default: you)"] user: Option<User>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(g) = scoped_guild(ctx) else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    let subject = user.as_ref().unwrap_or(ctx.author());
    let u = scoped_user(subject);
    let state = &ctx.data().state;
    let (facts, reputation) = {
        let stores = AppState::lock(&state.stores);
        let facts = stores.memory.facts(&g, &u).to_vec();
        let rep = AppState::lock(&state.social).reputation(&u, &g, &*stores);
        (facts, rep)
    };
    let mut out = format!(
        "**<@{}>** — standing {reputation:.2} (0 = poor, 1 = excellent)\n",
        subject.id.get()
    );
    if facts.is_empty() {
        out.push_str("No facts on record.");
    } else {
        for f in facts {
            out.push_str("• ");
            out.push_str(&f);
            out.push('\n');
        }
    }
    ctx.say(clamp_message(out)).await?;
    Ok(())
}

/// A member's reputation score in this server.
#[poise::command(slash_command, guild_only)]
pub async fn reputation(
    ctx: Context<'_>,
    #[description = "Who to look up (default: you)"] user: Option<User>,
) -> Result<(), Error> {
    ctx.defer().await?;
    let Some(g) = scoped_guild(ctx) else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    let subject = user.as_ref().unwrap_or(ctx.author());
    let u = scoped_user(subject);
    let state = &ctx.data().state;
    let rep = {
        let stores = AppState::lock(&state.stores);
        AppState::lock(&state.social).reputation(&u, &g, &*stores)
    };
    ctx.say(format!(
        "<@{}> — reputation {rep:.2} (0 = poor, 1 = excellent)",
        subject.id.get()
    ))
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Generation-backed
// ---------------------------------------------------------------------------

/// Summarize the recent messages Abbey has seen in this channel.
#[poise::command(slash_command, guild_only)]
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
    let Some(backend) = &state.backend else {
        ctx.say(clamp_message(ask::degraded_reply(persona))).await?;
        return Ok(());
    };
    let (system, user) = engine::summarize_prompt(persona, &transcript, count);
    let reply = match llm::ask_backend(&state.llm, backend, &system, &user).await {
        Ok(summary) => {
            AppState::lock(&state.stores)
                .memory
                .channel_mut(&ch)
                .summary
                .clone_from(&summary);
            ask::render_answer(persona, backend.label(), &summary)
        }
        Err(e) => ask::render_failure(persona, backend.label(), &e.0),
    };
    ctx.say(clamp_message(reply)).await?;
    Ok(())
}

async fn fetch_attachment(att: &Attachment) -> Result<Vec<u8>, String> {
    if usize::try_from(att.size).is_ok_and(|s| s > vision::MAX_IMAGE_BYTES) {
        return Err(format!(
            "that image is {} bytes; the cap is {}",
            att.size,
            vision::MAX_IMAGE_BYTES
        ));
    }
    att.download().await.map_err(|e| e.to_string())
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
    let Some(vision_client) = &state.vision else {
        ctx.say("Image understanding is not configured (ABBEY_VISION_ENDPOINT).")
            .await?;
        return Ok(());
    };
    let bytes = match fetch_attachment(&image).await {
        Ok(b) => b,
        Err(e) => {
            ctx.say(clamp_message(format!(
                "Could not read that attachment: {e}"
            )))
            .await?;
            return Ok(());
        }
    };
    let description = match vision_client.describe(&bytes).await {
        Ok(d) => d,
        Err(e) => {
            ctx.say(clamp_message(format!(
                "I couldn't read that image: {}",
                e.0
            )))
            .await?;
            return Ok(());
        }
    };
    let persona = crate::persona::Persona::Abbey;
    let reply = match (question, &state.backend) {
        (Some(q), Some(backend)) => {
            let folded = vision::fold_descriptions(&q, &[(image.filename.clone(), description)]);
            match llm::ask_backend(&state.llm, backend, &ask::system_prompt(persona), &folded).await
            {
                Ok(a) => ask::render_answer(persona, backend.label(), &a),
                Err(e) => ask::render_failure(persona, backend.label(), &e.0),
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
    let Some(vision_client) = &state.vision else {
        ctx.say("Image understanding is not configured (ABBEY_VISION_ENDPOINT).")
            .await?;
        return Ok(());
    };
    let reply = match fetch_attachment(&image).await {
        Err(e) => format!("Could not read that attachment: {e}"),
        Ok(bytes) => match vision_client.extract_text(&bytes).await {
            Ok(text) => vision::render_ocr(&text),
            Err(e) => format!("I couldn't read that image: {}", e.0),
        },
    };
    ctx.say(clamp_message(reply)).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// /stats and /admin
// ---------------------------------------------------------------------------

/// Command usage and learning statistics.
#[poise::command(slash_command, guild_only, ephemeral)]
pub async fn stats(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let state = &ctx.data().state;
    let g = scoped_guild(ctx).unwrap_or_default();
    let (interaction_text, seen) = {
        let stores = AppState::lock(&state.stores);
        (
            memory::render_stats(&stores.memory.interactions.stats()),
            stores.memory.messages_seen,
        )
    };
    let brain_line = {
        let brains = AppState::lock(&state.brains);
        brains.get(&g).map_or_else(
            || "brain: not loaded for this server yet".to_string(),
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
    let pending = AppState::lock(&state.rewards).pending_len();
    let backend = state.backend.as_ref().map_or("none", llm::Backend::label);
    let text = format!(
        "{interaction_text}\nmessages seen: {seen}\n{brain_line}\npending rewards: {pending}\nbackend: {backend} · vision: {}",
        if state.vision.is_some() { "on" } else { "off" }
    );
    ctx.say(clamp_message(text)).await?;
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
        "admin_brain",
        "admin_flush",
        "admin_export",
        "admin_reset"
    )
)]
pub async fn admin(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

fn update_settings(
    ctx: Context<'_>,
    mutate: impl FnOnce(&mut GuildSettings),
) -> Option<(String, GuildSettings)> {
    let g = scoped_guild(ctx)?;
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
    let Some(_) = update_settings(ctx, |s| s.learning_enabled = on) else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    ctx.say(format!(
        "learning is now **{}** for this server.",
        state.label()
    ))
    .await?;
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

/// Inspect this server's policy: ε, steps, buffer fill, experiences.
#[poise::command(slash_command, guild_only, ephemeral, rename = "brain")]
pub async fn admin_brain(
    ctx: Context<'_>,
    #[description = "Override exploration ε (0–1); omit to show"] epsilon: Option<f64>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(g) = scoped_guild(ctx) else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    let state = &ctx.data().state;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "ε is a 0–1 knob; f64→f32 loses nothing that matters"
    )]
    let override_eps = epsilon.map(|e| e.clamp(0.0, 1.0) as f32);
    if override_eps.is_some() {
        update_settings(ctx, |s| s.epsilon_override = override_eps);
    }
    let text = {
        let mut brains = AppState::lock(&state.brains);
        let stores = AppState::lock(&state.stores);
        let brain = brains.brain(&g, &*stores, runtime::now());
        if let Some(eps) = override_eps {
            brain.set_epsilon(eps);
        }
        let q_hint = format!(
            "actions: {}",
            BotAction::ALL
                .iter()
                .map(|a| format!("{a:?}"))
                .collect::<Vec<_>>()
                .join(" / ")
        );
        let (eps, steps, buffer) = (brain.epsilon(), brain.step_count(), brain.buffer_len());
        format!(
            "**brain — {g}**\nε {eps:.3} · learn steps {steps} · replay buffer {buffer}/{} · experiences {}\n{q_hint}\ntopology {:?}",
            runtime::REPLAY_CAPACITY,
            brains.experience_count(&g).unwrap_or(0),
            runtime::TOPOLOGY
        )
    };
    ctx.say(clamp_message(text)).await?;
    Ok(())
}

/// Flush reputation and persist everything to disk now.
#[poise::command(slash_command, guild_only, ephemeral, rename = "flush")]
pub async fn admin_flush(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let state = &ctx.data().state;
    state.persist_all();
    ctx.say(match &state.data_dir {
        Some(dir) => format!("Flushed and persisted to `{}`.", dir.display()),
        None => "Flushed in memory. ABBEY_DATA_DIR is unset, so nothing is on disk.".to_string(),
    })
    .await?;
    Ok(())
}

/// Export this server's brain snapshot as JSON.
#[poise::command(slash_command, guild_only, ephemeral, rename = "export")]
pub async fn admin_export(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(g) = scoped_guild(ctx) else {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    };
    let state = &ctx.data().state;
    let json = {
        let mut brains = AppState::lock(&state.brains);
        let stores = AppState::lock(&state.stores);
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
mod tests {
    use super::*;

    #[test]
    fn on_off_labels() {
        assert!(OnOff::On.is_on());
        assert_eq!(OnOff::Off.label(), "off");
    }
}

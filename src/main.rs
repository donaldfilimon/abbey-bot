//! Abbey Bot — Discord operational layer, Rust/serenity/poise.
//!
//! Configuration is entirely environment-driven, per the skill's standing rule
//! that tokens live in env vars and never in source:
//!
//! - `DISCORD_TOKEN` (required) — bot token.
//! - `ABBEY_GUILD_ID` (optional) — register commands to this guild only.
//!   Guild-scoped registration is instant; global registration can take up to an
//!   hour to propagate, which makes it useless during development. Unset means
//!   global, which is what you want once the command set has settled.
//! - `ANTHROPIC_API_KEY` (optional, secret) — makes `/persona ask` answer via
//!   the external Anthropic API. Same handling as `DISCORD_TOKEN`: env only.
//! - `ABBEY_BOT_LLM_ENDPOINT` (optional) — makes `/persona ask` answer via an
//!   OpenAI-compatible server, usually loopback. With neither this nor the key
//!   set, `/persona ask` replies that no generation backend is configured.
//! - `RUST_LOG` (optional) — tracing filter, defaults to `info`.
//!
//! Intents are deliberately `non_privileged()`. Nothing here reads message
//! content, presence, or the member list off the gateway; commands that need
//! guild data fetch it over REST instead. That keeps the bot deployable
//! without requesting privileged intents in the Dev Portal — and it is why
//! [`profile::summarize`] states that presence is unavailable rather than
//! guessing at it.

mod ask;
mod commands;
mod llm;
mod moderation;
mod perms;
mod persona;
mod profile;
mod server;
mod webhook;

use serenity::all::{GatewayIntents, GuildId};

/// Shared command state. Empty today; the type exists so adding state later does
/// not mean touching every command signature.
pub struct Data {}

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Context<'a> = poise::Context<'a, Data, Error>;

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Read before building anything else: a missing token should fail in the
    // first millisecond with a sentence you can act on, not inside a gateway
    // handshake error.
    let token = std::env::var("DISCORD_TOKEN")
        .map_err(|_| "DISCORD_TOKEN is not set. Export the bot token; never hardcode it.")?;

    let guild_id = match std::env::var("ABBEY_GUILD_ID") {
        Ok(raw) => {
            let parsed = raw
                .trim()
                .parse::<u64>()
                .map_err(|_| format!("ABBEY_GUILD_ID must be a numeric snowflake, got {raw:?}"))?;
            // GuildId::new PANICS on zero rather than returning an error, so a
            // literal "0" parses fine and then aborts the process -- the exact
            // opposite of failing with a sentence you can act on.
            if parsed == 0 {
                return Err("ABBEY_GUILD_ID must not be 0; that is not a valid snowflake".into());
            }
            Some(GuildId::new(parsed))
        }
        Err(_) => None,
    };

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![
                commands::persona(),
                commands::whois(),
                commands::perms(),
                commands::modcall(),
                commands::server(),
                commands::webhook(),
            ],
            on_error: |error| {
                Box::pin(async move {
                    // Structured, not `println!` — and never swallowed: a command
                    // that fails silently is indistinguishable from Discord
                    // dropping the interaction.
                    if let Err(e) = poise::builtins::on_error(error).await {
                        tracing::error!(error = %e, "error handler itself failed");
                    }
                })
            },
            ..Default::default()
        })
        .setup(move |ctx, ready, framework| {
            Box::pin(async move {
                match guild_id {
                    Some(id) => {
                        poise::builtins::register_in_guild(ctx, &framework.options().commands, id)
                            .await?;
                        tracing::info!(guild = %id, "registered guild-scoped commands (instant)");
                    }
                    None => {
                        poise::builtins::register_globally(ctx, &framework.options().commands)
                            .await?;
                        tracing::info!(
                            "registered global commands — propagation can take up to an hour"
                        );
                    }
                }
                tracing::info!(user = %ready.user.name, "connected");
                Ok(Data {})
            })
        })
        .build();

    let mut client = serenity::Client::builder(token, GatewayIntents::non_privileged())
        .framework(framework)
        .await?;

    client.start().await?;
    Ok(())
}

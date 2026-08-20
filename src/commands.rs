//! Slash commands.
//!
//! Every command here defers before doing anything that touches the network.
//! Discord invalidates an interaction token 3 seconds after it is issued, and a
//! single REST round-trip on a cold connection can eat that budget on its own —
//! so the deferral is unconditional rather than "when it looks slow". That rule
//! has one non-obvious corollary: **never take `GuildChannel` as a command
//! parameter.** poise resolves that type with a REST fetch *during argument
//! parsing*, before the command body — and therefore the defer — ever runs.
//! Take `ChannelId` (resolved fetch-free from interaction data) and fetch after
//! deferring, as `/perms` does.
//!
//! The decision logic these commands render lives in the pure modules —
//! [`crate::persona`], [`crate::profile`], [`crate::perms`],
//! [`crate::moderation`], [`crate::server`], [`crate::webhook`] — which know
//! nothing about Discord. That split is what lets
//! the decision suite run without a gateway. This file is the only one that
//! touches Discord types, and its job is translation: fetch over REST, build
//! the plain struct, hand it to a pure function, post the string back.

use std::collections::VecDeque;
use std::io::{ErrorKind as IoErrorKind, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serenity::all::{
    ChannelId, ChannelType, GuildId, Member, PartialGuild, PermissionOverwriteType, Permissions,
    User,
};
use songbird::events::{CoreEvent, Event, EventContext, EventHandler};
use songbird::input::RawAdapter;
use songbird::input::core::io::MediaSource;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;

use crate::ask;
use crate::generation;
use crate::llm;
use crate::moderation::{self, History, Severity};
use crate::perms::{self, Overwrite, Scope, Subject};
use crate::persona::{self, Persona};
use crate::pipeline;
use crate::profile::{self, ProfileFacts};
use crate::runtime::{self, AppState};
use crate::server::{self, Archetype};
use crate::voice::VoiceConfig;
use crate::webhook;
use crate::{Context, Error};

// ---------------------------------------------------------------------------
// Choice mirrors
//
// These exist so the pure modules never gain a poise dependency. One quirk is
// load-bearing and verified in the derive macro's source: doc comments on the
// variants NEVER reach Discord — the dropdown shows `#[name = "..."]` or the
// bare variant ident. Guidance for the person choosing goes in `#[name]`.
// ---------------------------------------------------------------------------

/// Discord-facing mirror of [`Archetype`].
#[derive(Debug, poise::ChoiceParameter)]
pub enum ArchetypeChoice {
    #[name = "community — public and open-join, rules gate, moderation depth"]
    Community,
    #[name = "gaming — voice-first group"]
    Gaming,
    #[name = "project — work server, structured and low noise"]
    Project,
    #[name = "friend group — small and deliberately flat"]
    FriendGroup,
}

impl From<ArchetypeChoice> for Archetype {
    fn from(choice: ArchetypeChoice) -> Self {
        match choice {
            ArchetypeChoice::Community => Self::Community,
            ArchetypeChoice::Gaming => Self::Gaming,
            ArchetypeChoice::Project => Self::Project,
            ArchetypeChoice::FriendGroup => Self::FriendGroup,
        }
    }
}

/// Discord-facing mirror of [`Severity`].
#[derive(Debug, poise::ChoiceParameter)]
pub enum SeverityChoice {
    #[name = "minor — rudeness, mild spam, derailing"]
    Minor,
    #[name = "serious — harassment, slurs, deliberate disruption"]
    Serious,
    #[name = "severe — threats, doxxing, raiding; bans on the first offence"]
    Severe,
}

impl From<SeverityChoice> for Severity {
    fn from(choice: SeverityChoice) -> Self {
        match choice {
            SeverityChoice::Minor => Self::Minor,
            SeverityChoice::Serious => Self::Serious,
            SeverityChoice::Severe => Self::Severe,
        }
    }
}

/// Discord-facing mirror of [`Persona`].
///
/// A dropdown, not a free-form string: the typo case ("no persona named
/// `abbby`") is unrepresentable, which deleted both `Persona::parse` and the
/// error branch that apologised for it.
#[derive(Debug, poise::ChoiceParameter)]
pub enum PersonaChoice {
    #[name = "abbey — direct, reads people fast"]
    Abbey,
    #[name = "aviva — analytical system-builder"]
    Aviva,
    #[name = "abi — warm rapport-builder"]
    Abi,
}

impl From<PersonaChoice> for Persona {
    fn from(choice: PersonaChoice) -> Self {
        match choice {
            PersonaChoice::Abbey => Self::Abbey,
            PersonaChoice::Aviva => Self::Aviva,
            PersonaChoice::Abi => Self::Abi,
        }
    }
}

// ---------------------------------------------------------------------------
// Shared shell helpers
// ---------------------------------------------------------------------------

/// Render a permission bitfield as the names Discord's own UI uses.
///
/// Do not be tempted to derive these from `Debug`: serenity's `Permissions`
/// prints as `Permissions(3072)`, a raw bitfield, so scraping it yields a number
/// rather than flag names. `get_permission_names` is the supported accessor.
fn permission_names(perms: Permissions) -> Vec<String> {
    perms
        .get_permission_names()
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// Fetch a member together with the guild, concurrently.
///
/// This is the whole preamble for every guild-reading command. `PartialGuild`
/// already carries `roles` and `owner_id`, so the separate `guild_id.roles()`
/// call the commands used to make was a third round-trip for data already in
/// hand — and the two fetches that remain are independent, so they run in
/// parallel rather than back to back inside the 3-second window.
///
/// Fetched over HTTP rather than read from cache: the cache is only as complete
/// as the intents we hold, and a partial cache would silently produce a thinner
/// answer rather than an error.
async fn fetch_member_and_guild(
    ctx: Context<'_>,
    guild_id: GuildId,
    user_id: serenity::all::UserId,
) -> Result<(Member, PartialGuild), Error> {
    tokio::try_join!(
        guild_id.member(ctx.http(), user_id),
        guild_id.to_partial_guild(ctx.http()),
    )
    .map_err(Into::into)
}

/// A member's highest role position; 0 when they hold only `@everyone`.
fn top_role_position(member: &Member, guild: &PartialGuild) -> u16 {
    member
        .roles
        .iter()
        .filter_map(|id| guild.roles.get(id))
        .map(|role| role.position)
        .max()
        .unwrap_or(0)
}

/// Discord's hard ceiling on message length, in codepoints.
///
/// Every command reply passes through here. Without it, a long-but-legitimate
/// answer (a channel where two overwrites each carry dozens of flags measures
/// over 2,000) fails the followup outright and the user gets a raw
/// "Message too large." instead of their walkthrough.
pub(crate) fn clamp_message(text: String) -> String {
    const LIMIT: usize = 2000;
    const MARKER: &str = "\n… (truncated to fit Discord's 2,000-character limit)";
    if text.chars().count() <= LIMIT {
        return text;
    }
    let keep = LIMIT - MARKER.chars().count();
    let mut clamped: String = text.chars().take(keep).collect();
    clamped.push_str(MARKER);
    clamped
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Abbey's persona surface: `/persona route` and `/persona ask`.
///
/// Discord never invokes the parent of a subcommand group — the client forces
/// picking a subcommand — so this body is unreachable framework wiring.
#[poise::command(slash_command, subcommands("route", "ask"))]
pub async fn persona(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Show which persona takes a request, and why.
///
/// Pure routing — no network call — but it still defers, because a command that
/// sometimes defers and sometimes does not is a command that eventually races.
/// This is the pre-split `/persona` behaviour, unchanged.
#[poise::command(slash_command)]
pub async fn route(
    ctx: Context<'_>,
    #[description = "What you want help with"] request: String,
    #[description = "Force a persona instead of routing"] r#as: Option<PersonaChoice>,
) -> Result<(), Error> {
    ctx.defer().await?;

    let route = persona::route(&request, r#as.map(Into::into));
    ctx.say(clamp_message(persona::describe(&route))).await?;
    Ok(())
}

/// Opening phrases for `/persona ask` — `IntentClassifier.suggestCompletions`
/// from the spec, surfaced where Discord can show it.
async fn autocomplete_question(_ctx: Context<'_>, partial: &str) -> Vec<String> {
    crate::brain::intent::suggest_completions(partial)
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// Ask a question; the routed persona answers via the configured backend.
///
/// The answer comes from an external or local model selected by the
/// environment ([`llm::Backend::from_env`]) — never from the bot itself, and
/// with no backend configured the reply says exactly that instead of dressing
/// a template echo up as AI. The defer is unconditional like everywhere else,
/// and more load-bearing here: an LLM round-trip exceeds Discord's 3-second
/// interaction token by design. Non-ephemeral on purpose — an answer is not an
/// accusation, unlike `/modcall`'s recommendation.
const ASK_COOLDOWN_SECONDS: u32 = 30;
const ASK_COOLDOWN_REPLY: &str = "You can ask again 30 seconds after your last accepted question.";

fn reserve_ask(state: &AppState, scoped_user: &str, now: u64) -> bool {
    AppState::lock(&state.ask_cooldown).try_reserve(scoped_user, ASK_COOLDOWN_SECONDS, now)
}

#[poise::command(slash_command)]
pub async fn ask(
    ctx: Context<'_>,
    #[description = "What you want to know"]
    #[autocomplete = "autocomplete_question"]
    #[max_length = 2000]
    question: String,
    #[description = "Force a persona instead of routing"] r#as: Option<PersonaChoice>,
) -> Result<(), Error> {
    ctx.defer().await?;

    // Same override `route` offers, and it matters more here: `route` only
    // explains a choice, while this one decides who actually answers. Hardcoding
    // None made the override available on the explanation and unavailable on the
    // answer, which is backwards.
    let routed = persona::route(&question, r#as.map(Into::into)).persona;
    let state = &ctx.data().state;
    let scope = format!("discord:{}", ctx.channel_id().get());
    let scoped_guild = match ctx.guild_id() {
        Some(g) => format!("discord:{}", g.get()),
        None => format!("discord:dm:{}", ctx.author().id.get()),
    };
    let scoped_user = format!("discord:{}", ctx.author().id.get());
    let now = runtime::now();
    if !reserve_ask(state, &scoped_user, now) {
        ctx.say(ASK_COOLDOWN_REPLY).await?;
        return Ok(());
    }
    let reply = match &state.backend {
        None => ask::degraded_reply(routed),
        Some(backend) => {
            // Same per-channel transcript, memory context, and tool loop the
            // pipeline uses, so a slash-command question and a DM continue
            // one thread. No streaming: an interaction followup is one post.
            let context =
                pipeline::assemble_context(state, &scoped_guild, &scoped_user, &scope, &question);
            let mut host = runtime::ToolScope {
                state,
                scoped_guild: scoped_guild.clone(),
                scoped_user: scoped_user.clone(),
                scoped_channel: scope.clone(),
                persona: routed,
            };
            let outcome = match state.acquire_generation().await {
                Err(busy) => Err(llm::LlmError(busy)),
                Ok(_slot) => {
                    generation::generate::<generation::NoDelivery>(
                        state,
                        &mut host,
                        &generation::Ask {
                            scope: &scope,
                            context: &context,
                            user_input: &question,
                            offer_tools: true,
                            now,
                        },
                        None,
                    )
                    .await
                }
            };
            match outcome {
                Ok((answer, _posted, persona)) => {
                    AppState::lock(&state.engine).commit(&scope, &question, &answer, now);
                    ask::render_answer(persona, backend.label(), &answer)
                }
                Err(error) => {
                    tracing::warn!(error = %error.0, backend = backend.label(), "slash-command generation failed");
                    ask::render_failure(routed, backend.label(), &error.0)
                }
            }
        }
    };
    ctx.say(clamp_message(reply)).await?;
    Ok(())
}

/// Read a member's profile.
#[poise::command(slash_command, guild_only)]
pub async fn whois(
    ctx: Context<'_>,
    #[description = "Who to read"] user: User,
) -> Result<(), Error> {
    ctx.defer().await?;

    let Some(guild_id) = ctx.guild_id() else {
        ctx.say("This one only works inside a server.").await?;
        return Ok(());
    };

    let (member, guild) = fetch_member_and_guild(ctx, guild_id, user.id).await?;

    // Highest-first, so `roles.first()` is the top role the summary reports.
    let mut named: Vec<(u16, String)> = member
        .roles
        .iter()
        .filter_map(|id| guild.roles.get(id))
        .map(|role| (role.position, role.name.to_string()))
        .collect();
    named.sort_by_key(|(position, _)| std::cmp::Reverse(*position));

    let facts = ProfileFacts {
        display_name: member.nick.clone().unwrap_or_else(|| {
            user.global_name
                .clone()
                .unwrap_or_else(|| user.name.clone())
        }),
        handle: user.name.clone(),
        is_bot: user.bot,
        nickname: member.nick.clone(),
        roles: named.into_iter().map(|(_, name)| name).collect(),
        // Discord renders this in the reader's own timezone.
        joined: member
            .joined_at
            .map(|ts| format!("<t:{}:D>", ts.unix_timestamp())),
        is_owner: user.id == guild.owner_id,
    };

    ctx.say(clamp_message(profile::summarize(&facts))).await?;
    Ok(())
}

/// Walk through how a channel's permission overwrites resolve for a member.
#[poise::command(slash_command, guild_only)]
pub async fn perms(
    ctx: Context<'_>,
    #[description = "Which channel"] channel: ChannelId,
    #[description = "Which member"] user: User,
) -> Result<(), Error> {
    // ChannelId on purpose — see the module doc. A GuildChannel parameter is
    // fetched by poise before this body runs, i.e. before this defer.
    ctx.defer().await?;

    let Some(guild_id) = ctx.guild_id() else {
        ctx.say("This one only works inside a server.").await?;
        return Ok(());
    };

    let (channel, (member, guild)) = tokio::try_join!(
        async { channel.to_channel(ctx.http()).await.map_err(Error::from) },
        fetch_member_and_guild(ctx, guild_id, user.id),
    )?;
    let Some(channel) = channel.guild() else {
        ctx.say("That is not a server channel, so it has no overwrites to walk.")
            .await?;
        return Ok(());
    };

    // Threads carry no overwrites of their own — they inherit the parent's.
    // Reading their empty list would produce "no overwrite touches them",
    // which is confidently wrong rather than merely thin.
    if matches!(
        channel.kind,
        ChannelType::PublicThread | ChannelType::PrivateThread | ChannelType::NewsThread
    ) {
        let parent = channel
            .parent_id
            .map(|id| format!(" Ask about <#{id}> instead."))
            .unwrap_or_default();
        ctx.say(format!(
            "Threads don't carry their own permission overwrites — they inherit from the parent channel.{parent}"
        ))
        .await?;
        return Ok(());
    }

    let channel_label = match channel.kind {
        ChannelType::Voice | ChannelType::Stage => format!("🔊 {}", channel.name),
        ChannelType::Category => format!("category \"{}\"", channel.name),
        _ => format!("#{}", channel.name),
    };

    let overwrites: Vec<Overwrite> = channel
        .permission_overwrites
        .iter()
        .map(|ow| {
            let scope = match ow.kind {
                // `@everyone` is the role whose id equals the guild id.
                PermissionOverwriteType::Role(id) if id.get() == guild_id.get() => Scope::Everyone,
                PermissionOverwriteType::Role(id) => Scope::Role {
                    id: id.get(),
                    name: guild
                        .roles
                        .get(&id)
                        .map(|r| r.name.to_string())
                        // A deleted role can still have a stale overwrite.
                        .unwrap_or_else(|| format!("deleted role {id}")),
                },
                PermissionOverwriteType::Member(id) => Scope::Member {
                    id: id.get(),
                    name: if id == user.id {
                        user.name.clone()
                    } else {
                        format!("<@{id}>")
                    },
                },
                // Compiler-required: the enum is #[non_exhaustive]. Carried as
                // its own scope because mapping unknowns onto Everyone once
                // promoted them to the top of the chain under a name they do
                // not have.
                _ => Scope::Unrecognized,
            };
            Overwrite {
                scope,
                allow: permission_names(ow.allow),
                deny: permission_names(ow.deny),
            }
        })
        .collect();

    let subject = Subject {
        name: user.name.clone(),
        user_id: user.id.get(),
        role_ids: member.roles.iter().map(|id| id.get()).collect(),
        // The canonical guild-level calculation — includes @everyone, returns
        // all() for the owner. Hand-rolling this is how the @everyone bug
        // happened; serenity already ships it on the guild we fetched.
        is_admin: guild
            .member_permissions(&member)
            .contains(Permissions::ADMINISTRATOR),
        is_owner: user.id == guild.owner_id,
    };

    ctx.say(clamp_message(perms::explain(
        &channel_label,
        &overwrites,
        &subject,
    )))
    .await?;
    Ok(())
}

/// Recommend a moderation action, and say whether you can actually take it.
///
/// Recommends only — this command never times out, kicks, or bans anyone. The
/// decision stays with the moderator, which is also why it is ephemeral: a
/// recommendation broadcast to the channel would be a public accusation.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MODERATE_MEMBERS"
)]
pub async fn modcall(
    ctx: Context<'_>,
    #[description = "Who the incident is about"] user: User,
    #[description = "How bad it is"] severity: SeverityChoice,
    #[description = "Prior warnings on record (default 0)"] warnings: Option<u8>,
    #[description = "Prior timeouts on record (default 0)"] timeouts: Option<u8>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;

    let Some(guild_id) = ctx.guild_id() else {
        ctx.say("This one only works inside a server.").await?;
        return Ok(());
    };

    let history = History {
        warnings: warnings.unwrap_or(0),
        timeouts: timeouts.unwrap_or(0),
    };
    let recommendation = moderation::recommend(severity.into(), history);

    // Whether *the moderator asking* can carry it out — not whether the bot
    // can. Two independent ways Discord refuses: the permission bit, and role
    // hierarchy. Report the first that applies.
    let blocker: Option<String> = match recommendation.action.required_permission() {
        None => None,
        Some(required) => {
            let ((moderator, guild), target) = tokio::try_join!(
                fetch_member_and_guild(ctx, guild_id, ctx.author().id),
                async {
                    guild_id
                        .member(ctx.http(), user.id)
                        .await
                        .map_err(Error::from)
                },
            )?;

            // The canonical calculation: includes @everyone's grants and
            // returns all() for the owner and for Administrator, which is why
            // no separate owner/admin check exists here any more.
            let held = guild.member_permissions(&moderator);

            if !held.get_permission_names().contains(&required) {
                Some(format!(
                    "You do not have **{required}**, so you cannot carry this out — hand it to someone who does."
                ))
            } else {
                moderation::hierarchy_blocker(
                    ctx.author().id == guild.owner_id,
                    top_role_position(&moderator, &guild),
                    user.id == guild.owner_id,
                    guild
                        .member_permissions(&target)
                        .contains(Permissions::ADMINISTRATOR),
                    top_role_position(&target, &guild),
                    matches!(recommendation.action, moderation::Action::Timeout(_)),
                )
                .map(str::to_string)
            }
        }
    };

    ctx.say(clamp_message(moderation::render(
        &user.name,
        &recommendation,
        blocker.as_deref(),
    )))
    .await?;
    Ok(())
}

/// Produce a server blueprint: role hierarchy, channel structure, numbered steps.
///
/// Emits a plan; it creates nothing. Building the server is a sequence of
/// destructive-ish structural changes, and those stay with a human who can see
/// what already exists.
#[poise::command(slash_command, ephemeral)]
pub async fn server(
    ctx: Context<'_>,
    #[description = "What kind of server"] kind: ArchetypeChoice,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    ctx.say(clamp_message(server::render(kind.into()))).await?;
    Ok(())
}

/// Emit the incoming-webhook setup guide for a channel.
///
/// Emit-only, like `/server`: creating the webhook is one click in a settings
/// screen the user is already looking at, and a URL minted by the bot would be
/// a credential the bot then knows. Ephemeral because it is setup chatter.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MANAGE_WEBHOOKS"
)]
pub async fn webhook(
    ctx: Context<'_>,
    #[description = "Where the webhook should post"] channel: ChannelId,
) -> Result<(), Error> {
    // ChannelId, never GuildChannel — see the module doc.
    ctx.defer_ephemeral().await?;

    let channel = channel.to_channel(ctx.http()).await?;
    let Some(channel) = channel.guild() else {
        ctx.say("That is not a server channel.").await?;
        return Ok(());
    };

    let target = match channel.kind {
        ChannelType::Category => {
            ctx.say("Webhooks attach to channels, not categories — pick a channel inside it.")
                .await?;
            return Ok(());
        }
        ChannelType::PublicThread | ChannelType::PrivateThread | ChannelType::NewsThread => {
            webhook::Target::Thread {
                label: format!("\"{}\"", channel.name),
                parent_label: channel
                    .parent_id
                    .map(|id| format!("<#{id}>"))
                    .unwrap_or_else(|| "the parent channel".to_string()),
                // The curl in the guide carries this id, so it works as pasted
                // instead of silently posting to the parent.
                thread_id: channel.id.get(),
            }
        }
        // A forum has no plain message stream: every execute must create a
        // post (thread_name) or target one (?thread_id=). The plain-channel
        // guide's curl is rejected outright there. Media channels (type 16)
        // share forum execute semantics but postdate this serenity version's
        // ChannelType, so they arrive as Unknown(16).
        ChannelType::Forum | ChannelType::Unknown(16) => webhook::Target::Forum {
            label: format!("#{}", channel.name),
        },
        ChannelType::Voice | ChannelType::Stage => webhook::Target::Channel {
            label: format!("🔊 {}", channel.name),
        },
        _ => webhook::Target::Channel {
            label: format!("#{}", channel.name),
        },
    };

    ctx.say(clamp_message(webhook::guide(&target))).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Live voice
// ---------------------------------------------------------------------------

/// Process-wide status for the one explicitly configured live voice session.
/// The key remains inside [`VoiceConfig`], whose Debug implementation redacts
/// it; status copy never includes a credential or provider payload.
pub struct VoiceRuntime {
    config: VoiceConfig,
    generation: AtomicU64,
    dropped_input: AtomicU64,
    dropped_output: AtomicU64,
    status: Mutex<String>,
}

impl VoiceRuntime {
    pub fn new(config: VoiceConfig) -> Self {
        Self {
            config,
            generation: AtomicU64::new(0),
            dropped_input: AtomicU64::new(0),
            dropped_output: AtomicU64::new(0),
            status: Mutex::new("configured; disconnected".into()),
        }
    }

    fn begin(&self) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.dropped_input.store(0, Ordering::Relaxed);
        self.dropped_output.store(0, Ordering::Relaxed);
        self.set_status(generation, "joining Discord voice");
        generation
    }

    fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        *self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = "configured; disconnected".into();
    }

    fn set_status(&self, generation: u64, status: impl Into<String>) {
        if self.generation.load(Ordering::SeqCst) == generation {
            *self
                .status
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = status.into();
        }
    }

    fn status(&self) -> String {
        self.status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

fn no_audio_songbird_config(config: &songbird::Config) -> songbird::Config {
    config
        .clone()
        .decode_mode(songbird::driver::DecodeMode::Pass)
}

async fn disable_voice_decoding(call: &Arc<tokio::sync::Mutex<songbird::Call>>) {
    let mut call = call.lock().await;
    let config = no_audio_songbird_config(call.config());
    call.set_config(config);
}

/// Establish the explicitly configured no-audio connection during an
/// operator-requested deployment. This path refuses to run when a provider key
/// exists: a restart must never begin receiving or transmitting audio.
pub async fn autojoin_self_deafened(
    ctx: &serenity::all::Context,
    runtime: Arc<VoiceRuntime>,
) -> Result<(), String> {
    if runtime.config.realtime_ready() {
        return Err(
            "ABBEY_VOICE_AUTOJOIN is allowed only without OPENAI_API_KEY; use /voice join for full-duplex mode"
                .into(),
        );
    }
    let guild_id = GuildId::new(runtime.config.guild_id);
    let channel_id = ChannelId::new(runtime.config.channel_id);
    let channel = channel_id
        .to_channel(&ctx.http)
        .await
        .map_err(|e| format!("fetching the configured channel failed: {e}"))?;
    let Some(channel) = channel.guild() else {
        return Err("the configured voice destination is not a server channel".into());
    };
    if channel.guild_id != guild_id || channel.kind != ChannelType::Voice {
        return Err(
            "the configured destination is not a voice channel in its configured server".into(),
        );
    }
    let manager = songbird::get(ctx)
        .await
        .ok_or_else(|| "Songbird was not registered in the Discord client".to_string())?;
    if manager.get(guild_id).is_some() {
        manager
            .remove(guild_id)
            .await
            .map_err(|e| format!("replacing the existing voice session failed: {e}"))?;
    }
    let call = manager.get_or_insert(guild_id);
    disable_voice_decoding(&call).await;
    let generation = runtime.begin();
    let call = manager.join(guild_id, channel_id).await.map_err(|e| {
        runtime.set_status(generation, format!("Discord join failed: {e}"));
        format!("Discord refused the voice join: {e}")
    })?;
    let safety_result = {
        let mut call = call.lock().await;
        match call.deafen(true).await {
            Ok(()) => call.mute(true).await,
            Err(error) => Err(error),
        }
    };
    if let Err(error) = safety_result {
        runtime.set_status(
            generation,
            format!("Discord mute/deafen safety state failed: {error}"),
        );
        let _ = manager.remove(guild_id).await;
        return Err(format!(
            "entering the required muted and self-deafened state failed: {error}"
        ));
    }
    runtime.set_status(
        generation,
        "Discord connected, muted and self-deafened; live speech is unavailable",
    );
    tracing::info!(guild = %guild_id, channel = %channel_id, "joined Discord voice muted and self-deafened; media decryption, audio decoding, receive delivery, and transmission are disabled");
    Ok(())
}

/// `/voice join`, `/voice leave`, and `/voice status`.
///
/// Voice is deliberately admin-triggered and bound to one env-configured
/// guild/channel. Merely deploying the code can never make Abbey listen.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MANAGE_GUILD",
    subcommands("voice_join", "voice_leave", "voice_status")
)]
pub async fn voice(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Join the configured voice channel.
// Without a Realtime key, remain self-deafened so the bot can establish
// presence without receiving participant audio.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MANAGE_GUILD",
    rename = "join"
)]
pub async fn voice_join(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(runtime) = ctx.data().voice.as_ref().cloned() else {
        ctx.say("Live voice is off. Configure ABBEY_VOICE_GUILD_ID, ABBEY_VOICE_CHANNEL_ID, and OPENAI_API_KEY, then restart Abbey.")
            .await?;
        return Ok(());
    };
    let Some(guild_id) = ctx.guild_id() else {
        ctx.say("This one only works inside a server.").await?;
        return Ok(());
    };
    if guild_id.get() != runtime.config.guild_id {
        ctx.say("Live voice is locked to a different server by the deployment configuration.")
            .await?;
        return Ok(());
    }

    let channel_id = ChannelId::new(runtime.config.channel_id);
    let channel = channel_id.to_channel(ctx.http()).await?;
    let Some(channel) = channel.guild() else {
        ctx.say("The configured voice destination is not a server channel.")
            .await?;
        return Ok(());
    };
    if channel.guild_id != guild_id || channel.kind != ChannelType::Voice {
        ctx.say("The configured destination must be a voice channel in this server (Stage channels are not supported).")
            .await?;
        return Ok(());
    }

    let manager = songbird::get(ctx.serenity_context())
        .await
        .ok_or("Songbird was not registered in the Discord client")?;
    // A repeated join is a clean session replacement. This removes the old
    // receive handler and output source before a new generation can publish
    // status, so a late WebSocket close cannot overwrite the new session.
    if manager.get(guild_id).is_some()
        && let Err(error) = manager.remove(guild_id).await
    {
        ctx.say(format!(
            "Could not replace the existing voice session cleanly: {error}"
        ))
        .await?;
        return Ok(());
    }
    if !runtime.config.realtime_ready() {
        let call = manager.get_or_insert(guild_id);
        disable_voice_decoding(&call).await;
    }
    let generation = runtime.begin();
    let call = match manager.join(guild_id, channel_id).await {
        Ok(call) => call,
        Err(error) => {
            runtime.set_status(generation, format!("Discord join failed: {error}"));
            let _ = manager.remove(guild_id).await;
            ctx.say(format!("Discord refused the voice join: {error}"))
                .await?;
            return Ok(());
        }
    };

    if !runtime.config.realtime_ready() {
        let safety_result = {
            let mut call = call.lock().await;
            match call.deafen(true).await {
                Ok(()) => call.mute(true).await,
                Err(error) => Err(error),
            }
        };
        if let Err(error) = safety_result {
            runtime.set_status(
                generation,
                format!("Discord mute/deafen safety state failed: {error}"),
            );
            let _ = manager.remove(guild_id).await;
            ctx.say(format!(
                "Joined Discord, but could not enter the required muted and self-deafened state: {error}"
            ))
            .await?;
            return Ok(());
        }
        runtime.set_status(
            generation,
            "Discord connected, muted and self-deafened; live speech is unavailable",
        );
        ctx.say(format!(
            "Joined <#{channel_id}> muted and self-deafened. I cannot receive anyone's audio or speak; `/voice leave` disconnects me."
        ))
        .await?;
        return Ok(());
    }

    let (input_tx, input_rx) = tokio::sync::mpsc::channel(50);
    let (output_tx, output_rx) = std::sync::mpsc::sync_channel(50);
    {
        let mut call = call.lock().await;
        call.add_global_event(
            Event::Core(CoreEvent::VoiceTick),
            DiscordVoiceReceiver {
                tx: input_tx,
                trailing_silence: AtomicUsize::new(0),
                runtime: Arc::clone(&runtime),
                generation,
            },
        );
        call.play_only_input(RawAdapter::new(PcmOutput::new(output_rx), 48_000, 2).into());
    }

    runtime.set_status(generation, "Discord connected; opening Realtime session");
    let task_runtime = Arc::clone(&runtime);
    tokio::spawn(async move {
        let result = run_realtime(&task_runtime, generation, input_rx, output_tx).await;
        if let Err(error) = result {
            tracing::error!(error = %error, "live voice Realtime session ended");
            task_runtime.set_status(generation, format!("Realtime stopped: {error}"));
        }
    });

    ctx.say(format!(
        "Joined <#{channel_id}> and am opening Abbey's Realtime session. Everyone in the channel should know that audio will be sent to the configured provider. Check `/voice status`; use `/voice leave` to stop it."
    ))
    .await?;
    Ok(())
}

/// Stop the Realtime session and leave Discord voice.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MANAGE_GUILD",
    rename = "leave"
)]
pub async fn voice_leave(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(guild_id) = ctx.guild_id() else {
        ctx.say("This one only works inside a server.").await?;
        return Ok(());
    };
    let Some(runtime) = ctx.data().voice.as_ref() else {
        ctx.say("Live voice is not configured.").await?;
        return Ok(());
    };
    if guild_id.get() != runtime.config.guild_id {
        ctx.say("Live voice is locked to a different server by the deployment configuration.")
            .await?;
        return Ok(());
    }
    let manager = songbird::get(ctx.serenity_context())
        .await
        .ok_or("Songbird was not registered in the Discord client")?;
    match manager.remove(guild_id).await {
        Ok(()) => {
            runtime.stop();
            ctx.say("Left voice and stopped the Realtime session.")
                .await?
        }
        Err(songbird::error::JoinError::NoCall) => {
            runtime.stop();
            ctx.say("Abbey is not in voice.").await?
        }
        Err(error) => return Err(error.into()),
    };
    Ok(())
}

/// Show destination, connection health, and bounded-queue drops.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MANAGE_GUILD",
    rename = "status"
)]
pub async fn voice_status(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(runtime) = ctx.data().voice.as_ref() else {
        ctx.say("Live voice is off (no complete voice configuration at startup).")
            .await?;
        return Ok(());
    };
    if ctx
        .guild_id()
        .is_none_or(|id| id.get() != runtime.config.guild_id)
    {
        ctx.say("Live voice is locked to a different server by the deployment configuration.")
            .await?;
        return Ok(());
    }
    let current = if let Some(manager) = songbird::get(ctx.serenity_context()).await {
        if let Some(call) = manager.get(GuildId::new(runtime.config.guild_id)) {
            call.lock()
                .await
                .current_channel()
                .map(|channel| format!("connected to <#{channel}>"))
                .unwrap_or_else(|| "not connected".into())
        } else {
            "not connected".into()
        }
    } else {
        "voice manager unavailable".into()
    };
    ctx.say(format!(
        "Abbey voice: {current}\nRealtime: {}\nModel: `{}` · voice: `{}`\nDropped bounded-queue chunks: input {} · output {}",
        runtime.status(),
        runtime.config.model,
        runtime.config.voice,
        runtime.dropped_input.load(Ordering::Relaxed),
        runtime.dropped_output.load(Ordering::Relaxed),
    ))
    .await?;
    Ok(())
}

struct DiscordVoiceReceiver {
    tx: tokio::sync::mpsc::Sender<Vec<i16>>,
    trailing_silence: AtomicUsize,
    runtime: Arc<VoiceRuntime>,
    generation: u64,
}

#[serenity::async_trait]
impl EventHandler for DiscordVoiceReceiver {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        if self.runtime.generation.load(Ordering::SeqCst) != self.generation {
            return Some(Event::Cancel);
        }
        let EventContext::VoiceTick(tick) = ctx else {
            return None;
        };
        let speakers: Vec<&[i16]> = tick
            .speaking
            .values()
            .filter_map(|voice| voice.decoded_voice.as_deref())
            .filter(|samples| !samples.is_empty())
            .collect();
        if speakers.is_empty() {
            let had_tail = self
                .trailing_silence
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
                .is_ok();
            if !had_tail {
                return None;
            }
        } else {
            // One second of zero PCM lets server/semantic VAD close the turn.
            self.trailing_silence.store(50, Ordering::Relaxed);
        }
        match self
            .tx
            .try_send(crate::voice::discord_to_realtime(&speakers))
        {
            Ok(()) => None,
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                self.runtime.dropped_input.fetch_add(1, Ordering::Relaxed);
                None
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => Some(Event::Cancel),
        }
    }
}

/// Blocking source consumed by Songbird's input parsing worker. The bounded
/// producer never blocks the async WebSocket task; dropping it ends the track.
struct PcmOutput {
    receiver: Mutex<std::sync::mpsc::Receiver<Vec<u8>>>,
    pending: VecDeque<u8>,
}

impl PcmOutput {
    fn new(receiver: std::sync::mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            receiver: Mutex::new(receiver),
            pending: VecDeque::new(),
        }
    }
}

impl Read for PcmOutput {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.pending.is_empty() {
            let chunk = self
                .receiver
                .lock()
                .map_err(|_| std::io::Error::other("voice PCM receiver lock poisoned"))?
                .recv()
                .map_err(|_| std::io::Error::from(IoErrorKind::UnexpectedEof))?;
            self.pending.extend(chunk);
        }
        let count = output.len().min(self.pending.len());
        for byte in &mut output[..count] {
            *byte = self.pending.pop_front().unwrap_or(0);
        }
        Ok(count)
    }
}

impl Seek for PcmOutput {
    fn seek(&mut self, _position: SeekFrom) -> std::io::Result<u64> {
        Err(IoErrorKind::Unsupported.into())
    }
}

impl MediaSource for PcmOutput {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

async fn run_realtime(
    runtime: &Arc<VoiceRuntime>,
    generation: u64,
    mut input: tokio::sync::mpsc::Receiver<Vec<i16>>,
    output: std::sync::mpsc::SyncSender<Vec<u8>>,
) -> Result<(), String> {
    let mut request = runtime
        .config
        .websocket_url()
        .into_client_request()
        .map_err(|e| format!("building the Realtime request failed: {e}"))?;
    let authorization = runtime
        .config
        .authorization()
        .ok_or_else(|| "OPENAI_API_KEY is not configured; Realtime cannot start".to_string())?;
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&authorization)
            .map_err(|_| "OPENAI_API_KEY contains invalid header bytes".to_string())?,
    );
    let (socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| format!("Realtime WebSocket connection failed: {e}"))?;
    let (mut writer, mut reader) = socket.split();
    let session = serde_json::json!({
        "type": "session.update",
        "session": {
            "type": "realtime",
            "model": runtime.config.model,
            "output_modalities": ["audio"],
            "instructions": runtime.config.instructions,
            "audio": {
                "input": {
                    "format": {"type": "audio/pcm", "rate": 24000},
                    "turn_detection": {"type": "semantic_vad"}
                },
                "output": {
                    "format": {"type": "audio/pcm"},
                    "voice": runtime.config.voice
                }
            }
        }
    });
    writer
        .send(Message::Text(session.to_string()))
        .await
        .map_err(|e| format!("sending Realtime session configuration failed: {e}"))?;

    loop {
        tokio::select! {
            maybe_pcm = input.recv() => {
                let Some(pcm) = maybe_pcm else {
                    let _ = writer.close().await;
                    return Ok(());
                };
                let bytes: Vec<u8> = pcm.into_iter().flat_map(i16::to_le_bytes).collect();
                let event = serde_json::json!({
                    "type": "input_audio_buffer.append",
                    "audio": base64::engine::general_purpose::STANDARD.encode(bytes),
                });
                writer.send(Message::Text(event.to_string())).await
                    .map_err(|e| format!("sending live input audio failed: {e}"))?;
            }
            message = reader.next() => {
                let message = message
                    .ok_or_else(|| "Realtime WebSocket closed".to_string())?
                    .map_err(|e| format!("reading the Realtime WebSocket failed: {e}"))?;
                let Message::Text(text) = message else {
                    if matches!(message, Message::Close(_)) {
                        return Err("Realtime provider closed the session".into());
                    }
                    continue;
                };
                let event: serde_json::Value = serde_json::from_str(&text)
                    .map_err(|e| format!("Realtime provider sent invalid JSON: {e}"))?;
                match event.get("type").and_then(serde_json::Value::as_str) {
                    Some("session.updated") => {
                        runtime.set_status(generation, "live; listening and speaking");
                    }
                    Some("response.output_audio.delta" | "response.audio.delta") => {
                        let Some(delta) = event.get("delta").and_then(serde_json::Value::as_str) else {
                            continue;
                        };
                        let pcm = base64::engine::general_purpose::STANDARD
                            .decode(delta)
                            .map_err(|e| format!("Realtime audio delta was not valid base64: {e}"))?;
                        let discord = crate::voice::realtime_to_discord(&pcm);
                        match output.try_send(discord) {
                            Ok(()) => {},
                            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                                runtime.dropped_output.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => return Ok(()),
                        }
                    }
                    Some("error") => {
                        let message = event
                            .pointer("/error/message")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("unspecified provider error");
                        return Err(format!("Realtime provider error: {message}"));
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_mirrors_map_onto_their_pure_types() {
        assert_eq!(
            Archetype::from(ArchetypeChoice::Community),
            Archetype::Community
        );
        assert_eq!(Archetype::from(ArchetypeChoice::Gaming), Archetype::Gaming);
        assert_eq!(
            Archetype::from(ArchetypeChoice::Project),
            Archetype::Project
        );
        assert_eq!(
            Archetype::from(ArchetypeChoice::FriendGroup),
            Archetype::FriendGroup
        );
        assert_eq!(Severity::from(SeverityChoice::Minor), Severity::Minor);
        assert_eq!(Severity::from(SeverityChoice::Serious), Severity::Serious);
        assert_eq!(Severity::from(SeverityChoice::Severe), Severity::Severe);
        assert_eq!(Persona::from(PersonaChoice::Abbey), Persona::Abbey);
        assert_eq!(Persona::from(PersonaChoice::Aviva), Persona::Aviva);
        assert_eq!(Persona::from(PersonaChoice::Abi), Persona::Abi);
    }

    #[test]
    fn the_ladders_permission_strings_all_exist_in_serenity() {
        // Iterates the real Action values rather than restating literals; a
        // mutation check showed a literal-pinning version passed even when the
        // ladder and the expectation were renamed in lockstep.
        let vocabulary = permission_names(Permissions::all());
        for action in [
            moderation::Action::Timeout(10),
            moderation::Action::Kick,
            moderation::Action::Ban,
        ] {
            let name = action
                .required_permission()
                .expect("these three all require a permission");
            assert!(
                vocabulary.iter().any(|known| known == name),
                "{action} names {name:?}, which serenity does not define"
            );
        }
    }

    #[test]
    fn every_permission_a_blueprint_names_exists_in_serenity() {
        // Blueprints hand out permission names as prose — now including the
        // per-archetype @everyone grants. A name serenity does not recognise
        // means a step nobody can follow.
        let vocabulary: Vec<String> = permission_names(Permissions::all());
        for archetype in Archetype::ALL {
            let bp = server::blueprint(archetype);
            let named = bp
                .roles
                .iter()
                .flat_map(|role| role.permissions.iter())
                .chain(bp.everyone.iter());
            for permission in named {
                assert!(
                    vocabulary.iter().any(|known| known == permission),
                    "{archetype:?} names {permission:?}, which serenity does not define"
                );
            }
        }
    }

    #[test]
    fn clamp_passes_short_messages_untouched() {
        let short = "fits".to_string();
        assert_eq!(clamp_message(short.clone()), short);
    }

    #[test]
    fn clamp_bounds_long_messages_at_discords_limit() {
        // Multibyte input, because the limit is codepoints, not bytes.
        let long: String = "é".repeat(2500);
        let out = clamp_message(long);
        assert!(out.chars().count() <= 2000, "{}", out.chars().count());
        assert!(
            out.ends_with("limit)"),
            "truncation must be stated, not silent"
        );
    }

    #[tokio::test]
    async fn a_five_thousand_char_backend_answer_clamps_to_discords_limit() {
        // The ask pipeline end to end with a recording fake: a 5,000-character
        // response comes back ≤ 2,000 codepoints through the existing clamp —
        // the same `clamp_message` every reply already routes through.
        // Multibyte input, because the limit is codepoints, not bytes.
        let backend = llm::Backend::OpenAiCompatible {
            endpoint: "http://127.0.0.1:8080".into(),
            model: "default".into(),
        };
        let long_answer = "é".repeat(5000);
        let canned =
            serde_json::json!({"choices": [{"message": {"content": long_answer}}]}).to_string();
        let transport = llm::RecordingTransport::returning(&canned);

        let answer = llm::ask_backend(
            &transport,
            &backend,
            &ask::system_prompt(Persona::Abbey),
            "a question",
        )
        .await
        .expect("the canned response parses");
        assert_eq!(
            answer.chars().count(),
            5000,
            "the fake answer arrives whole"
        );

        let reply = clamp_message(ask::render_answer(Persona::Abbey, backend.label(), &answer));
        assert!(reply.chars().count() <= 2000, "{}", reply.chars().count());
        assert!(
            reply.ends_with("limit)"),
            "truncation must be stated, not silent"
        );
    }

    #[test]
    fn no_audio_songbird_config_disables_decryption_and_decoding() {
        let base = songbird::Config::default().decode_mode(songbird::driver::DecodeMode::Decode(
            songbird::driver::DecodeConfig::default(),
        ));
        let no_audio = no_audio_songbird_config(&base);

        assert_eq!(no_audio.decode_mode, songbird::driver::DecodeMode::Pass);
        assert!(matches!(
            base.decode_mode,
            songbird::driver::DecodeMode::Decode(_)
        ));
    }

    #[tokio::test]
    async fn songbird_raw_pcm_input_has_a_registered_decoder() {
        let samples = vec![0_u8; 480 * 2 * std::mem::size_of::<f32>()];
        let input: songbird::input::Input =
            RawAdapter::new(std::io::Cursor::new(samples), 48_000, 2).into();
        let playable = input
            .make_playable_async(
                songbird::input::codecs::get_codec_registry(),
                songbird::input::codecs::get_probe(),
            )
            .await
            .expect("RawAdapter f32 PCM must be decodable by the deployed registry");
        assert!(playable.is_playable());
    }

    #[tokio::test]
    async fn realtime_websocket_exchanges_session_input_and_output_audio() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback fake");
        let address = listener.local_addr().expect("fake address");
        let config = VoiceConfig::from_values(
            Some("123".into()),
            Some("456".into()),
            Some("test-secret".into()),
            Some(format!("ws://{address}/realtime")),
            Some("test-model".into()),
            Some("marin".into()),
            Some("Be Abbey.".into()),
        )
        .expect("valid test config")
        .expect("voice enabled");
        let runtime = Arc::new(VoiceRuntime::new(config));
        let generation = runtime.begin();

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept fake client");
            let mut socket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("accept websocket");
            let session = socket
                .next()
                .await
                .expect("session event")
                .expect("session frame")
                .into_text()
                .expect("session text");
            let session: serde_json::Value = serde_json::from_str(&session).expect("session JSON");
            assert_eq!(session["type"], "session.update");
            assert_eq!(session["session"]["model"], "test-model");
            assert_eq!(
                session["session"]["audio"]["input"]["format"]["rate"],
                24_000
            );
            socket
                .send(Message::Text(
                    serde_json::json!({"type": "session.updated"}).to_string(),
                ))
                .await
                .expect("session acknowledgement");

            let input = socket
                .next()
                .await
                .expect("input event")
                .expect("input frame")
                .into_text()
                .expect("input text");
            let input: serde_json::Value = serde_json::from_str(&input).expect("input JSON");
            assert_eq!(input["type"], "input_audio_buffer.append");
            assert!(
                input["audio"]
                    .as_str()
                    .is_some_and(|audio| !audio.is_empty())
            );

            socket
                .send(Message::Text(
                    serde_json::json!({
                        "type": "response.output_audio.delta",
                        "delta": base64::engine::general_purpose::STANDARD.encode(32767_i16.to_le_bytes())
                    })
                    .to_string(),
                ))
                .await
                .expect("output delta");
            while let Some(Ok(message)) = socket.next().await {
                if message.is_close() {
                    break;
                }
            }
        });

        let (input_tx, input_rx) = tokio::sync::mpsc::channel(2);
        let (output_tx, output_rx) = std::sync::mpsc::sync_channel(2);
        let client_runtime = Arc::clone(&runtime);
        let client = tokio::spawn(async move {
            run_realtime(&client_runtime, generation, input_rx, output_tx).await
        });
        input_tx
            .send(vec![1000; 480])
            .await
            .expect("send fake voice tick");
        let output = tokio::task::spawn_blocking(move || {
            output_rx.recv_timeout(std::time::Duration::from_secs(5))
        })
        .await
        .expect("output wait task")
        .expect("receive converted audio");
        assert_eq!(
            output.len(),
            16,
            "one mono sample becomes two stereo frames"
        );
        assert_eq!(runtime.status(), "live; listening and speaking");

        drop(input_tx);
        client.await.expect("client task").expect("client exit");
        server.await.expect("server task");
    }

    #[test]
    fn ask_has_atomic_per_user_cost_control() {
        let state = AppState::in_memory();
        assert!(reserve_ask(&state, "discord:u1", 100));
        assert!(!reserve_ask(&state, "discord:u1", 129));
        assert!(reserve_ask(&state, "discord:u2", 129));
        assert!(reserve_ask(&state, "discord:u1", 130));
    }

    #[test]
    fn empty_permissions_render_as_nothing_not_as_a_placeholder() {
        assert!(permission_names(Permissions::empty()).is_empty());
    }

    #[test]
    fn permission_names_are_humanised() {
        let names = permission_names(Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES);
        assert!(names.iter().any(|n| n == "View Channel"), "{names:?}");
        assert!(names.iter().any(|n| n == "Send Messages"), "{names:?}");
    }

    #[test]
    fn a_single_permission_still_splits_cleanly() {
        assert_eq!(
            permission_names(Permissions::BAN_MEMBERS),
            vec!["Ban Members"]
        );
    }
}

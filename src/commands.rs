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

use serenity::all::{
    ChannelId, ChannelType, GuildId, Member, PartialGuild, PermissionOverwriteType, Permissions,
    User,
};

use crate::ask;
use crate::generation;
#[cfg(test)]
use crate::llm;
use crate::moderation::{self, History, Severity};
use crate::perms::{self, Overwrite, Scope, Subject};
use crate::persona::Persona;
use crate::pipeline;
use crate::profile::{self, ProfileFacts};
use crate::routing_signals;
use crate::runtime::{self, AppState};
use crate::server::{self, Archetype};
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
    #[name = "abbey — warm sharp friend and default"]
    Abbey,
    #[name = "aviva — concise direct expert"]
    Aviva,
    #[name = "abi — orchestration and governance"]
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
    crate::gateway::shared::clamp_message(text)
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

    // Explains both layers — canonical weights and the signal layer — so the
    // explanation matches how the same text would route. Guild defaults and
    // session stickiness are not visible from a slash command, so a message
    // neutral to both layers is still described by its prior here while the
    // pipeline would hand it to the sticky or guild persona.
    let route = routing_signals::route(&request, r#as.map(Into::into));
    ctx.say(clamp_message(routing_signals::describe(&route)))
        .await?;
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
    let reply = answer_question(ctx, &question, r#as.map(Into::into), Commit::Yes).await;
    deliver_generated_reply(&ctx.data().state, ctx.say(clamp_message(reply))).await?;
    Ok(())
}

/// Finish delivery before admitting queued model writes. Failed delivery keeps
/// the queue for the existing periodic persistence path; it cannot claim that
/// the requested fact was stored. Both interactive generation surfaces use this
/// boundary, while the pipeline owns its corresponding outbound boundary.
async fn deliver_generated_reply<T, E>(
    state: &AppState,
    delivery: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, E> {
    let delivered = delivery.await?;
    if state.episode_gate.is_some() {
        crate::memory_gate::drain(state).await;
    }
    Ok(delivered)
}

/// Whether an answered question joins the channel's running transcript.
///
/// Only a publicly posted answer may. The transcript is shared context for
/// whoever speaks in that channel next, so committing an ephemeral exchange
/// would let a private lookup steer a conversation nobody saw it enter. It also
/// keeps the message context menu from pulling a third party's words into
/// Abbey's context in a guild where Abbey holds no message-content access of
/// its own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Commit {
    Yes,
    No,
}

/// Answer `question` on this interaction's channel and render the reply.
///
/// Shared by `/persona ask` and the "Ask Abbey" message context menu so the two
/// never disagree about identical text: one routing decision, one cooldown, one
/// transcript scope, one tool loop. The caller owns the defer and the post —
/// this returns the message body, including the cooldown notice, because a
/// context menu wants it ephemeral and a slash command does not. It also owns
/// the [`Commit`] decision; see that type for why the two differ.
///
/// The cooldown is charged either way: reading an answer costs the same backend
/// call whether or not it is remembered.
async fn answer_question(
    ctx: Context<'_>,
    question: &str,
    forced: Option<Persona>,
    commit: Commit,
) -> String {
    let state = &ctx.data().state;
    let scope = format!("discord:{}", ctx.channel_id().get());
    // Same composition the message pipeline uses, so `/persona ask` and an
    // ordinary message never disagree about identical text. Session stickiness
    // still applies, but only to text neither layer has an opinion about.
    let route = routing_signals::route(question, forced);
    let routed = if route.is_decisive() {
        route.persona
    } else {
        AppState::lock(&state.engine)
            .session_persona(&scope)
            .unwrap_or(route.persona)
    };
    let scoped_guild = match ctx.guild_id() {
        Some(g) => format!("discord:{}", g.get()),
        None => format!("discord:dm:{}", ctx.author().id.get()),
    };
    let scoped_user = format!("discord:{}", ctx.author().id.get());
    let now = runtime::now();
    if !reserve_ask(state, &scoped_user, now) {
        return ASK_COOLDOWN_REPLY.to_string();
    }
    match state.generation_label() {
        None => ask::degraded_reply(routed),
        Some(backend_label) => {
            // Same per-channel transcript, memory context, and tool loop the
            // pipeline uses, so a slash-command question and a DM continue
            // one thread. No streaming: an interaction followup is one post.
            let reputation = state.reputation_snapshot(&scoped_guild, &scoped_user);
            let context = pipeline::assemble_context(
                state,
                &scoped_guild,
                &scoped_user,
                &scope,
                question,
                reputation,
            );
            let mut host = runtime::ToolScope {
                state,
                network: crate::platform::SocialNetwork::Discord,
                scoped_guild: scoped_guild.clone(),
                scoped_user: scoped_user.clone(),
                scoped_channel: scope.clone(),
                now,
                persona: routed,
            };
            let outcome = {
                generation::generate_with_tools_without_delivery(
                    state,
                    &mut host,
                    &generation::Ask {
                        session_mode: if commit == Commit::Yes {
                            generation::SessionMode::Shared
                        } else {
                            generation::SessionMode::Ephemeral
                        },
                        scope: &scope,
                        context: &context,
                        user_input: question,
                        now,
                    },
                )
                .await
            };
            match outcome {
                Ok((answer, persona, provider_label)) => {
                    if commit == Commit::Yes {
                        AppState::lock(&state.engine).commit(&scope, question, &answer, now);
                    }
                    ask::render_answer(persona, provider_label, &answer)
                }
                Err(error) => {
                    tracing::warn!(error = %error, backend = backend_label, "slash-command generation failed");
                    ask::render_failure(routed, backend_label, &error)
                }
            }
        }
    }
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

    let summary = member_profile(ctx, guild_id, &user).await?;
    ctx.say(clamp_message(summary)).await?;
    Ok(())
}

/// Render one member's profile summary.
///
/// Shared by `/whois` and the "Abbey: profile" user context menu so the right
/// click and the slash command describe a member identically. Fetches over
/// REST, so the caller must have deferred already.
async fn member_profile(ctx: Context<'_>, guild_id: GuildId, user: &User) -> Result<String, Error> {
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

    Ok(profile::summarize(&facts))
}

/// Right-click a member -> Apps -> "Abbey: profile".
///
/// Same summary `/whois` renders, reached without typing a name. Ephemeral:
/// reading someone's roles is a lookup, not an announcement about them.
#[poise::command(context_menu_command = "Abbey: profile", guild_only, ephemeral)]
pub async fn profile_context_menu(ctx: Context<'_>, user: User) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;

    let Some(guild_id) = ctx.guild_id() else {
        ctx.say("This one only works inside a server.").await?;
        return Ok(());
    };

    let summary = member_profile(ctx, guild_id, &user).await?;
    ctx.say(clamp_message(summary)).await?;
    Ok(())
}

/// Right-click a message -> Apps -> "Ask Abbey".
///
/// Routes the message's own text through the same path `/persona ask` uses, so
/// a question someone already typed does not have to be retyped. Three limits
/// are deliberate and reported rather than papered over: only text is read (an
/// image needs `/see`); if Abbey holds no message-content access to that
/// message the resolved content arrives empty, which this says plainly instead
/// of answering a blank question; and the exchange is [`Commit::No`], so a
/// third party's words never join the channel transcript through a right-click.
/// Ephemeral, because a right-click is a private lookup and should not put
/// words in the original author's thread.
#[poise::command(context_menu_command = "Ask Abbey", ephemeral)]
pub async fn ask_context_menu(
    ctx: Context<'_>,
    message: serenity::all::Message,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;

    let question = message.content.trim();
    if question.is_empty() {
        ctx.say(
            "That message carries no text Abbey can read. Attachments, embeds, and stickers are not part of this path — use `/see` for an image, or `/persona ask` to type the question.",
        )
        .await?;
        return Ok(());
    }

    let reply = answer_question(ctx, question, None, Commit::No).await;
    deliver_generated_reply(&ctx.data().state, ctx.say(clamp_message(reply))).await?;
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

    let (moderator, guild) = fetch_member_and_guild(ctx, guild_id, ctx.author().id).await?;
    let held = guild.member_permissions(&moderator);
    if !crate::commands_help::modcall_access_allowed(held) {
        ctx.say("Discord must currently grant you Moderate Members to use this command.")
            .await?;
        return Ok(());
    }
    let history = History {
        warnings: warnings.unwrap_or(0),
        timeouts: timeouts.unwrap_or(0),
    };
    let recommendation = moderation::recommend(severity.into(), history);

    // Whether *the moderator asking* can carry it out — not whether the bot
    // can. Two independent ways Discord refuses: the permission bit, and role
    // hierarchy. Report the first that applies.
    let target = guild_id.member(ctx.http(), user.id).await?;
    let blocker = recommendation.action.required_permission().and_then(|required| {
        if !held.get_permission_names().contains(&required) {
            Some(format!("You do not have **{required}**, so you cannot carry this out — hand it to someone who does."))
        } else {
            moderation::hierarchy_blocker(
                ctx.author().id == guild.owner_id,
                top_role_position(&moderator, &guild),
                user.id == guild.owner_id,
                guild.member_permissions(&target).contains(Permissions::ADMINISTRATOR),
                top_role_position(&target, &guild),
                matches!(recommendation.action, moderation::Action::Timeout(_)),
            ).map(str::to_string)
        }
    });
    if !crate::commands_help::resolved_modcall_allowed(held, Some(blocker.is_none())) {
        let refusal = blocker
            .as_deref()
            .unwrap_or("Discord must currently grant you Moderate Members to use this command.");
        ctx.say(clamp_message(moderation::render(
            &user.name,
            &recommendation,
            Some(refusal),
        )))
        .await?;
        return Ok(());
    }

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
/// Emits a plan; it creates nothing. The slash command never builds a server:
/// the boundary now lives in the plan engine's `server::diff::Change` type,
/// which has no delete variant and is driven only from the operator CLI
/// (`abbey-bot --server-plan`, dry run by default), never from a guild
/// interaction.
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

#[cfg(test)]
mod tests;

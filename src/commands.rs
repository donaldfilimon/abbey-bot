//! Slash commands.
//!
//! Every command here defers before doing anything that touches the network.
//! Discord invalidates an interaction token 3 seconds after it is issued, and a
//! single REST round-trip on a cold connection can eat that budget on its own —
//! so the deferral is unconditional rather than "when it looks slow".
//!
//! The decision logic these commands render lives in [`crate::persona`],
//! [`crate::profile`], [`crate::perms`], and [`crate::moderation`], which know
//! nothing about Discord. That split is what lets the interesting behaviour be
//! unit-tested without a gateway.

use serenity::all::{GuildChannel, PermissionOverwriteType, Permissions, RoleId, User};

use crate::moderation::{self, History, Severity};
use crate::perms::{self, Overwrite, Scope, Subject};
use crate::persona::{self, Persona};
use crate::profile::{self, ProfileFacts};
use crate::server::{self, Archetype};
use crate::{Context, Error};

/// Discord-facing mirror of [`Archetype`], for the same reason as
/// [`SeverityChoice`]: it keeps poise's derive out of `server.rs`.
#[derive(Debug, poise::ChoiceParameter)]
pub enum ArchetypeChoice {
    /// Public, open-join — rules gate and moderation depth
    Community,
    /// Voice-first gaming group
    Gaming,
    /// Work or project server — structured, low noise
    Project,
    /// Small friend group — deliberately flat
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
///
/// It exists so `moderation.rs` stays free of poise, which is what keeps the
/// escalation ladder testable without a gateway. The descriptions are the
/// dropdown text a moderator reads while choosing.
#[derive(Debug, poise::ChoiceParameter)]
pub enum SeverityChoice {
    /// Rudeness, mild spam, off-topic derailing
    Minor,
    /// Harassment, slurs, deliberate disruption
    Serious,
    /// Threats, doxxing, raiding — bans on the first offence
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

/// Render a permission bitfield as the names Discord's own UI uses.
///
/// Do not be tempted to derive these from `Debug`: serenity's `Permissions`
/// prints as `Permissions(3072)`, a raw bitfield, so scraping it yields a number
/// rather than flag names. `get_permission_names` is the supported accessor and
/// returns exactly the client-facing strings ("View Channel", "Ban Members").
fn permission_names(perms: Permissions) -> Vec<String> {
    perms
        .get_permission_names()
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// Show which persona takes a request, and why.
///
/// Pure routing — no network call — but it still defers, because a command that
/// sometimes defers and sometimes does not is a command that eventually races.
#[poise::command(slash_command, guild_only)]
pub async fn persona(
    ctx: Context<'_>,
    #[description = "What you want help with"] request: String,
    #[description = "Force a persona instead of routing (abbey / aviva / abi)"] r#as: Option<
        String,
    >,
) -> Result<(), Error> {
    ctx.defer().await?;

    let explicit = match r#as.as_deref() {
        // An unrecognised name is a typo, not a silent fallback to Abbey — say so.
        Some(raw) => match Persona::parse(raw) {
            Some(p) => Some(p),
            None => {
                ctx.say(format!(
                    "No persona named `{raw}`. Pick one of: abbey, aviva, abi — or omit it and I'll route."
                ))
                .await?;
                return Ok(());
            }
        },
        None => None,
    };

    let route = persona::route(&request, explicit);
    ctx.say(persona::describe(&route)).await?;
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

    // Fetched over HTTP rather than read from cache: the cache is only as
    // complete as the intents we hold, and a partial cache would silently
    // produce a thinner read rather than an error.
    let member = guild_id.member(ctx.http(), user.id).await?;
    let roles = guild_id.roles(ctx.http()).await?;
    let owner_id = guild_id.to_partial_guild(ctx.http()).await?.owner_id;

    // Highest-first, so `roles.first()` is the top role the summary reports.
    let mut named: Vec<(u16, String)> = member
        .roles
        .iter()
        .filter_map(|id| roles.get(id))
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
        is_owner: user.id == owner_id,
    };

    ctx.say(profile::summarize(&facts)).await?;
    Ok(())
}

/// Walk through how a channel's permission overwrites resolve for a member.
#[poise::command(slash_command, guild_only)]
pub async fn perms(
    ctx: Context<'_>,
    #[description = "Which channel"] channel: GuildChannel,
    #[description = "Which member"] user: User,
) -> Result<(), Error> {
    ctx.defer().await?;

    let Some(guild_id) = ctx.guild_id() else {
        ctx.say("This one only works inside a server.").await?;
        return Ok(());
    };

    let member = guild_id.member(ctx.http(), user.id).await?;
    let roles = guild_id.roles(ctx.http()).await?;
    let owner_id = guild_id.to_partial_guild(ctx.http()).await?.owner_id;

    // `@everyone` is the role whose id equals the guild id. Naming it that way in
    // the chain would be technically right and useless to read.
    let everyone = RoleId::new(guild_id.get());

    let overwrites: Vec<Overwrite> = channel
        .permission_overwrites
        .iter()
        .map(|ow| {
            let scope = match ow.kind {
                PermissionOverwriteType::Role(id) if id == everyone => Scope::Everyone,
                PermissionOverwriteType::Role(id) => Scope::Role(
                    roles
                        .get(&id)
                        .map(|r| r.name.to_string())
                        // A deleted role can still have a stale overwrite.
                        .unwrap_or_else(|| format!("unknown role {id}")),
                ),
                PermissionOverwriteType::Member(id) => Scope::Member(if id == user.id {
                    user.name.clone()
                } else {
                    format!("other member {id}")
                }),
                _ => Scope::Everyone,
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
        role_names: member
            .roles
            .iter()
            .filter_map(|id| roles.get(id))
            .map(|r| r.name.to_string())
            .collect(),
        is_admin: member
            .roles
            .iter()
            .filter_map(|id| roles.get(id))
            .any(|r| r.permissions.contains(Permissions::ADMINISTRATOR)),
        is_owner: user.id == owner_id,
    };

    ctx.say(perms::explain(&channel.name, &overwrites, &subject))
        .await?;
    Ok(())
}

/// Recommend a moderation action, and say whether you can actually take it.
///
/// Recommends only — this command never times out, kicks, or bans anyone. The
/// decision stays with the moderator, which is also why it is ephemeral: a
/// recommendation broadcast to the channel would be a public accusation.
#[poise::command(slash_command, guild_only, ephemeral)]
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

    // Whether *the moderator asking* can carry it out — not whether the bot can.
    // The skill's rule is to check standing before recommending escalation, and
    // advice someone cannot act on is worse than useless.
    let can_act = match recommendation.action.required_permission() {
        None => true,
        Some(name) => {
            let moderator = guild_id.member(ctx.http(), ctx.author().id).await?;
            let roles = guild_id.roles(ctx.http()).await?;
            let owner_id = guild_id.to_partial_guild(ctx.http()).await?.owner_id;

            let held = moderator
                .roles
                .iter()
                .filter_map(|id| roles.get(id))
                .fold(Permissions::empty(), |acc, role| acc | role.permissions);

            // Owner and Administrator both bypass the individual bit.
            ctx.author().id == owner_id
                || held.contains(Permissions::ADMINISTRATOR)
                || held
                    .get_permission_names()
                    .into_iter()
                    .any(|held_name| held_name == name)
        }
    };

    ctx.say(moderation::render(&user.name, &recommendation, can_act))
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
    ctx.say(server::render(kind.into())).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archetype_choice_maps_onto_the_pure_blueprints() {
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
    }

    #[test]
    fn every_permission_a_blueprint_names_exists_in_serenity() {
        // Blueprints hand out permission names as prose. A name serenity does not
        // recognise means a step nobody can follow, so pin them against the real
        // vocabulary rather than trusting the strings.
        let vocabulary: Vec<String> = permission_names(Permissions::all());
        for archetype in Archetype::ALL {
            for role in server::blueprint(archetype).roles {
                for permission in role.permissions {
                    assert!(
                        vocabulary.iter().any(|known| known == permission),
                        "{archetype:?}/{} names {permission:?}, which serenity does not define",
                        role.name
                    );
                }
            }
        }
    }

    #[test]
    fn severity_choice_maps_onto_the_pure_ladder() {
        assert_eq!(Severity::from(SeverityChoice::Minor), Severity::Minor);
        assert_eq!(Severity::from(SeverityChoice::Serious), Severity::Serious);
        assert_eq!(Severity::from(SeverityChoice::Severe), Severity::Severe);
    }

    #[test]
    fn every_permission_the_ladder_names_exists_in_serenity() {
        // `can_act` compares the ladder's permission strings against
        // `get_permission_names()`. A typo there would silently mean "you cannot
        // act" for everyone, forever — so pin the names against serenity itself.
        for (permission, expected) in [
            (Permissions::MODERATE_MEMBERS, "Moderate Members"),
            (Permissions::KICK_MEMBERS, "Kick Members"),
            (Permissions::BAN_MEMBERS, "Ban Members"),
        ] {
            assert!(
                permission_names(permission).iter().any(|n| n == expected),
                "serenity does not call it {expected:?}: {:?}",
                permission_names(permission)
            );
        }
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

//! Slash commands.
//!
//! Every command here defers before doing anything that touches the network.
//! Discord invalidates an interaction token 3 seconds after it is issued, and a
//! single REST round-trip on a cold connection can eat that budget on its own —
//! so the deferral is unconditional rather than "when it looks slow".
//!
//! The decision logic these commands render lives in [`crate::persona`],
//! [`crate::profile`], and [`crate::perms`], which know nothing about Discord.
//! That split is what lets the interesting behaviour be unit-tested without a
//! gateway.

use serenity::all::{GuildChannel, PermissionOverwriteType, Permissions, RoleId, User};

use crate::perms::{self, Overwrite, Scope, Subject};
use crate::persona::{self, Persona};
use crate::profile::{self, ProfileFacts};
use crate::{Context, Error};

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

#[cfg(test)]
mod tests {
    use super::*;

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

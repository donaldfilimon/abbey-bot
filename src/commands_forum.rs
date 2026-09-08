//! Discord adapters for `#help` forum helpers (`/forum draft|post|perms`).
//!
//! Pure policy lives in [`crate::forum`]. Defer before REST; ChannelId in, never
//! GuildChannel before acknowledgement. Gap-fill never wipes unrelated overwrite
//! bits. Brand: Abbey / Intelligence Without Limits — never Quesar.

use serenity::all::{
    ChannelId, ChannelType, CreateForumPost, CreateMessage, GuildChannel, GuildId,
    PermissionOverwrite, PermissionOverwriteType, Permissions, UserId,
};

use crate::commands::clamp_message;
use crate::forum::{self, HELP_TAG_NAMES, Template};
use crate::{Context, Error};

/// Discord-facing mirror of [`forum::Template`].
#[derive(Debug, Clone, Copy, poise::ChoiceParameter)]
pub enum TemplateChoice {
    Question,
    Bug,
    Build,
    General,
}

impl From<TemplateChoice> for Template {
    fn from(value: TemplateChoice) -> Self {
        match value {
            TemplateChoice::Question => Self::Question,
            TemplateChoice::Bug => Self::Bug,
            TemplateChoice::Build => Self::Build,
            TemplateChoice::General => Self::General,
        }
    }
}

/// Parent — Discord forces a subcommand; this body is unreachable wiring.
#[poise::command(slash_command, guild_only, subcommands("draft", "post", "perms"))]
pub async fn forum(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Ephemeral tag suggestions + first-post template (no channel mutate).
#[poise::command(slash_command, guild_only, ephemeral)]
pub async fn draft(
    ctx: Context<'_>,
    #[description = "Forum channel (defaults to #help)"] channel: Option<ChannelId>,
    #[description = "Draft title for tag matching"] title: String,
    #[description = "Optional details for the template body"] details: Option<String>,
    #[description = "First-post template"] template: Option<TemplateChoice>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let guild_id = ctx
        .guild_id()
        .ok_or("This one only works inside a server.")?;
    let details = details.unwrap_or_default();
    let template = template.map(Template::from).unwrap_or(Template::Question);

    let title = match forum::clamp_title(&title) {
        Ok(title) => title,
        Err(message) => {
            ctx.say(message).await?;
            return Ok(());
        }
    };

    let forum_channel = match resolve_forum(ctx, guild_id, channel).await? {
        Ok(channel) => channel,
        Err(message) => {
            ctx.say(message).await?;
            return Ok(());
        }
    };

    let available: Vec<String> = forum_channel
        .available_tags
        .iter()
        .map(|tag| tag.name.clone())
        .collect();
    let available_refs: Vec<&str> = available.iter().map(String::as_str).collect();
    let suggested = forum::suggest_tags(&title, &details, &available_refs);
    let body = forum::first_post_body(template, &details);

    let tag_line = if suggested.is_empty() {
        if available.is_empty() {
            format!(
                "_No tags on this forum yet._ Blueprint defaults for #help: {}",
                HELP_TAG_NAMES.join(", ")
            )
        } else {
            format!(
                "_No automatic matches._ Available: {}",
                available.join(", ")
            )
        }
    } else {
        format!("**Suggested tags:** {}", suggested.join(", "))
    };

    let reply = format!(
        "**Forum draft** for #{}\n\
         **Title:** {title}\n\
         **Template:** {} (`{}`)\n\
         {tag_line}\n\n\
         **First post preview:**\n{body}\n\n\
         Use `/forum post` to create this thread via the API.",
        forum_channel.name,
        template.label(),
        template.slug()
    );
    ctx.say(clamp_message(reply)).await?;
    Ok(())
}

/// Create a `#help` (or chosen forum) post via Discord's forum-thread API.
#[poise::command(slash_command, guild_only)]
pub async fn post(
    ctx: Context<'_>,
    #[description = "Post title (2–100 characters)"] title: String,
    #[description = "Forum channel (defaults to #help)"] channel: Option<ChannelId>,
    #[description = "Details for the first post"] details: Option<String>,
    #[description = "First-post template"] template: Option<TemplateChoice>,
    #[description = "Comma-separated tag names (default: auto-suggest)"] tags: Option<String>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let guild_id = ctx
        .guild_id()
        .ok_or("This one only works inside a server.")?;
    let details = details.unwrap_or_default();
    let template = template.map(Template::from).unwrap_or(Template::Question);

    let title = match forum::clamp_title(&title) {
        Ok(title) => title,
        Err(message) => {
            ctx.say(message).await?;
            return Ok(());
        }
    };

    let forum_channel = match resolve_forum(ctx, guild_id, channel).await? {
        Ok(channel) => channel,
        Err(message) => {
            ctx.say(message).await?;
            return Ok(());
        }
    };

    let available = &forum_channel.available_tags;
    let available_names: Vec<&str> = available.iter().map(|tag| tag.name.as_str()).collect();
    let chosen_names = resolve_tag_names(&tags, &title, &details, &available_names);
    let mut unknown = Vec::new();
    let mut applied = Vec::new();
    for name in &chosen_names {
        match available
            .iter()
            .find(|tag| tag.name.eq_ignore_ascii_case(name))
        {
            Some(tag) => applied.push(tag.id),
            None => unknown.push(name.clone()),
        }
    }
    if !unknown.is_empty() {
        ctx.say(clamp_message(format!(
            "Unknown tag(s) on #{}: {}. Available: {}",
            forum_channel.name,
            unknown.join(", "),
            available_names.join(", ")
        )))
        .await?;
        return Ok(());
    }

    let author = ctx.author();
    let body = format!(
        "{}\n\n_Requested by <@{}> via `/forum post`._",
        forum::first_post_body(template, &details),
        author.id.get()
    );
    let message = CreateMessage::new().content(clamp_message(body));
    let mut builder =
        CreateForumPost::new(title.clone(), message).audit_log_reason("Abbey /forum post assist");
    if !applied.is_empty() {
        builder = builder.set_applied_tags(applied.iter().copied());
    }

    let thread = match forum_channel
        .id
        .create_forum_post(ctx.http(), builder)
        .await
    {
        Ok(thread) => thread,
        Err(err) => {
            ctx.say(clamp_message(format!(
                "Could not create the forum post in #{}: {err}. \
                 If Abbey is missing Create Public Threads / Send Messages in Threads, \
                 an admin can run `/forum perms` to gap-fill the bot overwrite only.",
                forum_channel.name
            )))
            .await?;
            return Ok(());
        }
    };

    let tag_note = if chosen_names.is_empty() {
        "no tags".to_string()
    } else {
        format!("tags: {}", chosen_names.join(", "))
    };
    ctx.say(clamp_message(format!(
        "Created <#{}> in #{} ({tag_note}).",
        thread.id.get(),
        forum_channel.name
    )))
    .await?;
    Ok(())
}

/// Snapshot the bot's forum overwrite, then gap-fill missing required bits only.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MANAGE_GUILD"
)]
pub async fn perms(
    ctx: Context<'_>,
    #[description = "Forum channel (defaults to #help)"] channel: Option<ChannelId>,
    #[description = "When true, PUT the gap-filled bot member overwrite"] apply: Option<bool>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let guild_id = ctx
        .guild_id()
        .ok_or("This one only works inside a server.")?;
    let apply = apply.unwrap_or(false);

    let forum_channel = match resolve_forum(ctx, guild_id, channel).await? {
        Ok(channel) => channel,
        Err(message) => {
            ctx.say(message).await?;
            return Ok(());
        }
    };

    let bot_id = ctx.http().get_current_user().await?.id;
    let required = forum::required_forum_bot_permissions();
    let (existing_allow, existing_deny) = bot_member_overwrite(&forum_channel, bot_id);

    let snapshot = format!(
        "**Snapshot before mutate** on #{}\n\
         Bot member overwrite allow: {}\n\
         Bot member overwrite deny: {}\n\
         Required: {}",
        forum_channel.name,
        fmt_perms(existing_allow),
        fmt_perms(existing_deny),
        fmt_perms(required)
    );

    let Some(fill) = forum::gap_fill_overwrite(existing_allow, existing_deny, required) else {
        ctx.say(clamp_message(format!(
            "{snapshot}\n\nNo gap — Abbey already has the required forum bits on its member overwrite \
             (or they are already allowed). No PUT."
        )))
        .await?;
        return Ok(());
    };

    if !apply {
        ctx.say(clamp_message(format!(
            "{snapshot}\n\n**Would gap-fill** (add to allow, clear from deny only): {}\n\
             Re-run with `apply:True` to PUT. Other overwrite targets are untouched.",
            fmt_perms(fill.added)
        )))
        .await?;
        return Ok(());
    }

    let overwrite = PermissionOverwrite {
        allow: fill.allow,
        deny: fill.deny,
        kind: PermissionOverwriteType::Member(bot_id),
    };
    forum_channel
        .id
        .create_permission(ctx.http(), overwrite)
        .await
        .map_err(|err| -> Error {
            format!("gap-fill PUT failed on #{}: {err}", forum_channel.name).into()
        })?;

    ctx.say(clamp_message(format!(
        "{snapshot}\n\n**Applied gap-fill.** Added: {}. New allow: {}. New deny: {}.",
        fmt_perms(fill.added),
        fmt_perms(fill.allow),
        fmt_perms(fill.deny)
    )))
    .await?;
    Ok(())
}

fn fmt_perms(bits: Permissions) -> String {
    let names = forum::permission_labels(bits);
    if names.is_empty() {
        "_none_".to_string()
    } else {
        names.join(", ")
    }
}

fn bot_member_overwrite(channel: &GuildChannel, bot_id: UserId) -> (Permissions, Permissions) {
    channel
        .permission_overwrites
        .iter()
        .find_map(|overwrite| match overwrite.kind {
            PermissionOverwriteType::Member(id) if id == bot_id => {
                Some((overwrite.allow, overwrite.deny))
            }
            _ => None,
        })
        .unwrap_or((Permissions::empty(), Permissions::empty()))
}

fn resolve_tag_names(
    tags: &Option<String>,
    title: &str,
    details: &str,
    available: &[&str],
) -> Vec<String> {
    if let Some(raw) = tags {
        let parsed: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect();
        if !parsed.is_empty() {
            return parsed;
        }
    }
    forum::suggest_tags(title, details, available)
}

async fn resolve_forum(
    ctx: Context<'_>,
    guild_id: GuildId,
    channel: Option<ChannelId>,
) -> Result<Result<GuildChannel, String>, Error> {
    match channel {
        Some(id) => {
            let channel = id.to_channel(ctx.http()).await?;
            let Some(guild_channel) = channel.guild() else {
                return Ok(Err("That is not a server channel.".into()));
            };
            if guild_channel.guild_id != guild_id {
                return Ok(Err("That channel is not in this server.".into()));
            }
            if !is_forum(&guild_channel) {
                return Ok(Err(format!(
                    "#{} is not a forum channel — pick the #help forum (or another forum).",
                    guild_channel.name
                )));
            }
            Ok(Ok(guild_channel))
        }
        None => {
            let channels = guild_id.channels(ctx.http()).await?;
            let mut helps: Vec<GuildChannel> = channels
                .into_values()
                .filter(|channel| is_forum(channel) && channel.name.eq_ignore_ascii_case("help"))
                .collect();
            if helps.is_empty() {
                return Ok(Err(
                    "No #help forum found. Pass `channel:` with the forum to use.".into(),
                ));
            }
            if helps.len() > 1 {
                helps.sort_by_key(|channel| channel.id.get());
            }
            Ok(Ok(helps.remove(0)))
        }
    }
}

fn is_forum(channel: &GuildChannel) -> bool {
    matches!(channel.kind, ChannelType::Forum | ChannelType::Unknown(16))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forum::{HELP_TAG_NAMES, suggest_tags};

    #[test]
    fn resolve_tag_names_prefers_explicit_list() {
        let available = HELP_TAG_NAMES;
        let tags = Some("wdbx, missing".to_string());
        assert_eq!(
            resolve_tag_names(&tags, "abbey voice", "", available),
            vec!["wdbx".to_string(), "missing".to_string()]
        );
        assert_eq!(
            resolve_tag_names(&None, "abbey voice", "", available),
            suggest_tags("abbey voice", "", available)
        );
        assert_eq!(
            resolve_tag_names(&Some("  ,  ".into()), "build ci", "", available),
            suggest_tags("build ci", "", available)
        );
    }

    #[test]
    fn template_choice_maps_to_pure_template() {
        assert_eq!(Template::from(TemplateChoice::Bug).slug(), "bug");
        assert_eq!(Template::from(TemplateChoice::Build).label(), "Build");
    }
}

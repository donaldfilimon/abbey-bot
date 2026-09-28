//! Discord translation for durable personal and channel-bound work records.
//! The catalog guard defers before these bodies run. Current permissions are
//! fetched afresh before each command reads or changes a channel project.
use crate::work::{WorkAccess, WorkStatus, WorkTask};
use crate::{Context, Error};
use serenity::all::{Permissions, UserId};

#[poise::command(
    slash_command,
    subcommands(
        "project",
        "projects",
        "goal",
        "task",
        "decision",
        "briefing",
        "complete",
        "status",
        "snooze",
        "member",
        "preferences",
        "reset_preferences",
        "learning",
        "timing",
        "feedback",
        "automation",
        "reminder",
        "recall"
    )
)]
pub async fn work(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

async fn access(ctx: Context<'_>) -> Result<WorkAccess, Error> {
    let actor = ctx.author().id.get();
    let channel = ctx.channel_id().get();
    let Some(guild) = ctx.guild_id() else {
        return Ok(WorkAccess {
            actor,
            guild: None,
            channel,
            can_view: true,
            can_manage: false,
        });
    };
    let perms = crate::commands_help::current_permissions(
        ctx.serenity_context(),
        guild,
        ctx.channel_id(),
        ctx.author().id,
    )
    .await?;
    Ok(WorkAccess {
        actor,
        guild: Some(guild.get()),
        channel,
        can_view: perms.contains(Permissions::VIEW_CHANNEL),
        can_manage: perms.contains(Permissions::MANAGE_GUILD),
    })
}

async fn reply(ctx: Context<'_>, text: impl Into<String>) -> Result<(), Error> {
    ctx.send(
        poise::CreateReply::default()
            .content(crate::commands::clamp_message(text.into()))
            .ephemeral(true)
            .allowed_mentions(crate::gateway::no_mentions()),
    )
    .await?;
    Ok(())
}

#[poise::command(slash_command, ephemeral)]
pub async fn projects(ctx: Context<'_>) -> Result<(), Error> {
    let access = access(ctx).await?;
    let text = {
        let stores = crate::runtime::AppState::lock(&ctx.data().state.stores);
        let visible = stores.work.visible_projects(access);
        if visible.is_empty() {
            "No projects here yet. Create one with `/work project`.".to_string()
        } else {
            let mut text = String::from("Projects available in this channel:\n");
            for project in visible.iter().take(20) {
                text.push_str(&format!("#{} · {}\n", project.id, project.name));
            }
            text
        }
    };
    reply(ctx, text).await
}

#[poise::command(slash_command, ephemeral)]
pub async fn project(
    ctx: Context<'_>,
    #[description = "Project name"] name: String,
) -> Result<(), Error> {
    let access = access(ctx).await?;
    let request = ctx.id().to_string();
    let saved_text = name.clone();
    let id = ctx
        .data()
        .state
        .commit_work(move |store| store.create_project(access, &name, &request))
        .await?;
    reply(ctx, format!("Saved project #{id}: {saved_text}")).await
}

#[poise::command(slash_command, ephemeral)]
pub async fn goal(
    ctx: Context<'_>,
    #[description = "Project number"] project_id: u64,
    #[description = "Goal title"] title: String,
) -> Result<(), Error> {
    let access = access(ctx).await?;
    let request = ctx.id().to_string();
    let saved_text = title.clone();
    let id = ctx
        .data()
        .state
        .commit_work(move |store| store.add_goal(project_id, access, &title, &request))
        .await?;
    reply(ctx, format!("Saved goal #{id}: {saved_text}")).await
}

#[poise::command(slash_command, ephemeral)]
pub async fn task(
    ctx: Context<'_>,
    #[description = "Project number"] project_id: u64,
    #[description = "Task title"] title: String,
    #[description = "Assigned project member"] assignee: Option<UserId>,
    #[description = "Unix seconds, when due"] due_at: Option<u64>,
    #[description = "Priority 0 to 3"] priority: Option<u8>,
    #[description = "Goal number"] goal_id: Option<u64>,
) -> Result<(), Error> {
    let access = access(ctx).await?;
    let request = ctx.id().to_string();
    let draft = WorkTask {
        id: 0,
        project_id,
        title: title.clone(),
        owner: access.actor,
        assignee: assignee.map(|user| user.get()),
        goal_id,
        priority: priority.unwrap_or(1),
        status: WorkStatus::Open,
        due_at,
        remind_at: None,
        reminder_revision: 0,
        snoozed_until: None,
        source: None,
        github: None,
        revision: 0,
    };
    let id = ctx
        .data()
        .state
        .commit_work(move |store| store.add_task(access, draft, &request))
        .await?;
    reply(ctx, format!("Saved task #{id}: {title}")).await
}

#[poise::command(slash_command, ephemeral)]
pub async fn decision(
    ctx: Context<'_>,
    #[description = "Project number"] project_id: u64,
    #[description = "Decision to record"] text: String,
) -> Result<(), Error> {
    let access = access(ctx).await?;
    let at = crate::runtime::now();
    let request = ctx.id().to_string();
    let saved_text = text.clone();
    let id = ctx
        .data()
        .state
        .commit_work(move |store| store.record_decision(project_id, access, &text, at, &request))
        .await?;
    reply(ctx, format!("Saved decision #{id}: {saved_text}")).await
}

#[poise::command(slash_command, ephemeral)]
pub async fn briefing(
    ctx: Context<'_>,
    #[description = "Project number"] project_id: u64,
) -> Result<(), Error> {
    let access = access(ctx).await?;
    let body = crate::runtime::AppState::lock(&ctx.data().state.stores)
        .work
        .briefing(project_id, access, crate::runtime::now())?;
    reply(ctx, body).await
}

#[poise::command(slash_command, ephemeral)]
pub async fn complete(
    ctx: Context<'_>,
    #[description = "Task number"] task_id: u64,
    #[description = "Revision from the briefing or last update"] revision: u64,
) -> Result<(), Error> {
    let access = access(ctx).await?;
    let resulting_revision = ctx
        .data()
        .state
        .commit_work(move |store| {
            store.update_task(access, task_id, revision, WorkStatus::Done, None)
        })
        .await?;
    reply(
        ctx,
        task_update_reply(task_id, "is done", resulting_revision),
    )
    .await
}

#[poise::command(slash_command, ephemeral)]
pub async fn status(
    ctx: Context<'_>,
    #[description = "Task number"] task_id: u64,
    #[description = "Revision from the briefing or last update"] revision: u64,
    #[description = "open, in_progress, blocked, done, or cancelled"] state: String,
) -> Result<(), Error> {
    let status = match state.as_str() {
        "open" => WorkStatus::Open,
        "in_progress" => WorkStatus::InProgress,
        "blocked" => WorkStatus::Blocked,
        "done" => WorkStatus::Done,
        "cancelled" => WorkStatus::Cancelled,
        _ => return Err(crate::work::WorkError::Invalid.into()),
    };
    let access = access(ctx).await?;
    let resulting_revision = ctx
        .data()
        .state
        .commit_work(move |store| store.update_task(access, task_id, revision, status, None))
        .await?;
    reply(
        ctx,
        task_update_reply(
            task_id,
            &format!("is {}", status.label()),
            resulting_revision,
        ),
    )
    .await
}

#[poise::command(slash_command, ephemeral)]
pub async fn snooze(
    ctx: Context<'_>,
    #[description = "Task number"] task_id: u64,
    #[description = "Revision from the briefing or last update"] revision: u64,
    #[description = "Resume after this Unix timestamp"] until: u64,
) -> Result<(), Error> {
    let now = crate::runtime::now();
    if until <= now || until > now.saturating_add(366 * 24 * 60 * 60) {
        return Err(crate::work::WorkError::Invalid.into());
    }
    let access = access(ctx).await?;
    let resulting_revision = ctx
        .data()
        .state
        .commit_work(move |store| store.snooze_task(access, task_id, revision, until))
        .await?;
    reply(
        ctx,
        task_update_reply(
            task_id,
            &format!("snoozed until <t:{until}:f>"),
            resulting_revision,
        ),
    )
    .await
}

#[poise::command(slash_command, ephemeral)]
pub async fn member(
    ctx: Context<'_>,
    #[description = "Project number"] project_id: u64,
    #[description = "Member"] user: UserId,
    #[description = "Grant or revoke project membership"] present: bool,
) -> Result<(), Error> {
    let access = access(ctx).await?;
    ctx.data()
        .state
        .commit_work(move |store| store.set_member(project_id, access, user.get(), present))
        .await?;
    let result = if present { "added to" } else { "removed from" };
    reply(ctx, format!("Member {result} project #{project_id}.")).await
}

fn task_update_reply(task_id: u64, action: &str, revision: u64) -> String {
    format!("Task #{task_id} {action}. Revision {revision}.")
}

#[cfg(test)]
mod tests {
    #[test]
    fn task_acknowledgments_include_revision_for_the_next_update() {
        for action in ["is done", "is open", "snoozed until <t:2000000000:f>"] {
            let text = super::task_update_reply(42, action, 7);
            println!("{text}");
            assert!(text.ends_with("Revision 7."));
        }
    }
}

mod controls;
use controls::{automation, feedback, learning, preferences, reminder, reset_preferences, timing};

mod recall;
use recall::recall;

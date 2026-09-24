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
        "reset_preferences"
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
    let id = ctx
        .data()
        .state
        .commit_work(|store| store.create_project(access, &name, &request))
        .await?;
    reply(ctx, format!("Saved project #{id}: {name}")).await
}

#[poise::command(slash_command, ephemeral)]
pub async fn goal(
    ctx: Context<'_>,
    #[description = "Project number"] project_id: u64,
    #[description = "Goal title"] title: String,
) -> Result<(), Error> {
    let access = access(ctx).await?;
    let id = ctx
        .data()
        .state
        .commit_work(|store| store.add_goal(project_id, access, &title))
        .await?;
    reply(ctx, format!("Saved goal #{id}: {title}")).await
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
        .commit_work(|store| store.add_task(access, draft, &request))
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
    let id = ctx
        .data()
        .state
        .commit_work(|store| store.record_decision(project_id, access, &text, at))
        .await?;
    reply(ctx, format!("Saved decision #{id}: {text}")).await
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
    #[description = "Revision from the current briefing"] revision: u64,
) -> Result<(), Error> {
    let access = access(ctx).await?;
    ctx.data()
        .state
        .commit_work(|store| store.update_task(access, task_id, revision, WorkStatus::Done, None))
        .await?;
    reply(ctx, format!("Task #{task_id} is done.")).await
}

#[poise::command(slash_command, ephemeral)]
pub async fn status(
    ctx: Context<'_>,
    #[description = "Task number"] task_id: u64,
    #[description = "Revision from the current briefing"] revision: u64,
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
    ctx.data()
        .state
        .commit_work(|store| store.update_task(access, task_id, revision, status, None))
        .await?;
    reply(ctx, format!("Task #{task_id} is {}.", status.label())).await
}

#[poise::command(slash_command, ephemeral)]
pub async fn snooze(
    ctx: Context<'_>,
    #[description = "Task number"] task_id: u64,
    #[description = "Revision from the current briefing"] revision: u64,
    #[description = "Resume after this Unix timestamp"] until: u64,
) -> Result<(), Error> {
    let now = crate::runtime::now();
    if until <= now || until > now.saturating_add(366 * 24 * 60 * 60) {
        return Err(crate::work::WorkError::Invalid.into());
    }
    let access = access(ctx).await?;
    ctx.data()
        .state
        .commit_work(|store| {
            let task = store
                .tasks
                .get(&task_id)
                .ok_or(crate::work::WorkError::Missing)?;
            store.update_task(access, task_id, revision, task.status, Some(until))
        })
        .await?;
    reply(ctx, format!("Task #{task_id} snoozed until <t:{until}:f>.")).await
}

#[poise::command(slash_command, ephemeral)]
pub async fn preferences(ctx: Context<'_>) -> Result<(), Error> {
    let key = access(ctx).await?.preference_key();
    let profile = {
        let stores = crate::runtime::AppState::lock(&ctx.data().state.stores);
        stores
            .work
            .preferences
            .get(&key)
            .cloned()
            .unwrap_or_default()
    };
    reply(
        ctx,
        format!(
            "Learning: {}. Explicit hour: {}. Learned hour: {}. Attributable observations: {}. Use `/work reset_preferences` to clear the evidence and learned hour.",
            if profile.learning_enabled { "on" } else { "off" },
            profile.explicit_hour.map_or("none".to_string(), |hour| hour.to_string()),
            profile.learned_hour.map_or("none".to_string(), |hour| hour.to_string()),
            profile.evidence.len(),
        ),
    )
    .await
}

#[poise::command(slash_command, ephemeral)]
pub async fn reset_preferences(ctx: Context<'_>) -> Result<(), Error> {
    let key = access(ctx).await?.preference_key();
    ctx.data()
        .state
        .commit_work(|store| {
            store.preferences.entry(key).or_default().reset();
            Ok(())
        })
        .await?;
    reply(
        ctx,
        "Your learned delivery hour and feedback evidence were cleared.",
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
        .commit_work(|store| store.set_member(project_id, access, user.get(), present))
        .await?;
    let result = if present { "added to" } else { "removed from" };
    reply(ctx, format!("Member {result} project #{project_id}.")).await
}

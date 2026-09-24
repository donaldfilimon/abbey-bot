//! Private command controls; every mutation reauthorizes canonical work state
//! inside the retained durable commit before acknowledging success.
use super::{access, reply};
use crate::work::{
    WorkAutomationPolicy, WorkAutomationUpdate, WorkError, WorkFeedback, WorkPreferenceSnapshot,
};
use crate::{Context, Error};

#[poise::command(slash_command, ephemeral)]
pub async fn preferences(ctx: Context<'_>) -> Result<(), Error> {
    let access = access(ctx).await?;
    let snapshot = crate::runtime::AppState::lock(&ctx.data().state.stores)
        .work
        .preference_snapshot(access)?;
    reply(ctx, preference_reply(&snapshot, access.actor)).await
}

fn hour_text(hour: Option<u8>) -> String {
    hour.map_or_else(|| "not set".into(), |hour| format!("{hour}:00"))
}

fn preference_reply(snapshot: &WorkPreferenceSnapshot, actor: u64) -> String {
    let profile = &snapshot.profile;
    let zone = snapshot
        .timezone
        .as_deref()
        .unwrap_or("timezone not configured");
    let mut text = format!(
        "Learning: {}. Explicit hour: {}. Suggested hour: {}. Effective hour: {}:00 ({zone}). Reduced optional follow-ups: {}. Briefing ranking adjustment: {}.\n",
        if profile.learning_enabled {
            "enabled"
        } else {
            "disabled"
        },
        hour_text(profile.explicit_hour),
        hour_text(profile.learned_hour),
        snapshot.effective_hour,
        if profile.reduce_followups {
            "yes"
        } else {
            "no"
        },
        match profile.briefing_rank {
            1.. => "favor useful briefings",
            ..=-1 => "deprioritize dismissed briefings",
            0 => "none",
        }
    );
    for e in profile
        .evidence
        .iter()
        .filter(|e| e.actor == Some(actor))
        .rev()
        .take(15)
    {
        let feedback = match e.feedback {
            WorkFeedback::Useful => "useful".into(),
            WorkFeedback::Dismissed => "dismissed".into(),
            WorkFeedback::Snoozed { hour } => format!("snoozed to {hour}:00 ({zone})"),
        };
        text.push_str(&format!(
            "Delivery #{} · {feedback} · <t:{}:f>\n",
            e.delivery_id, e.at
        ));
    }
    text.push_str("Showing your own feedback only. Use /work feedback with correction to replace or remove active evidence; /work reset_preferences clears shared learned evidence (manager required in teams). Previously observed deliveries cannot count again after removal or reset.");
    text
}

#[poise::command(slash_command, ephemeral)]
pub async fn reset_preferences(ctx: Context<'_>) -> Result<(), Error> {
    let access = access(ctx).await?;
    ctx.data()
        .state
        .commit_work(move |store| store.control_preferences(access, None, None, true))
        .await?;
    reply(ctx, "Cleared this scope's feedback, learned timing, ranking and optional follow-up adjustments. Explicit timing is retained.").await
}

#[poise::command(slash_command, ephemeral)]
pub async fn learning(
    ctx: Context<'_>,
    #[description = "Enable preference learning for this scope"] enabled: bool,
) -> Result<(), Error> {
    let access = access(ctx).await?;
    ctx.data()
        .state
        .commit_work(move |store| store.control_preferences(access, Some(enabled), None, false))
        .await?;
    reply(ctx, format!("Learning {} for this scope. Disabling learning removes all learned adjustments; explicit controls remain authoritative.", if enabled { "enabled" } else { "disabled" })).await
}

#[poise::command(slash_command, ephemeral)]
pub async fn timing(
    ctx: Context<'_>,
    #[description = "Local hour 0–23; omit to clear explicit preference"] hour: Option<u8>,
) -> Result<(), Error> {
    let access = access(ctx).await?;
    let snapshot = ctx
        .data()
        .state
        .commit_work(move |store| {
            store.control_preferences(access, None, Some(hour), false)?;
            store.preference_snapshot(access)
        })
        .await?;
    reply(ctx, timing_reply(&snapshot)).await
}

fn timing_reply(snapshot: &WorkPreferenceSnapshot) -> String {
    format!(
        "Explicit optional delivery hour: {}. Effective hour: {}:00 ({}). Quiet hours, deadlines and the daily ceiling still apply.",
        hour_text(snapshot.profile.explicit_hour),
        snapshot.effective_hour,
        snapshot
            .timezone
            .as_deref()
            .unwrap_or("timezone not configured")
    )
}

#[poise::command(slash_command, ephemeral)]
pub async fn feedback(
    ctx: Context<'_>,
    #[description = "Completed delivery number"] delivery_id: u64,
    #[description = "useful, dismissed, snoozed, or remove"] response: String,
    #[description = "Local hour for snoozed feedback"] hour: Option<u8>,
    #[description = "Replace or remove your previous feedback"] correction: Option<bool>,
) -> Result<(), Error> {
    let value = match response.as_str() {
        "useful" => Some(WorkFeedback::Useful),
        "dismissed" => Some(WorkFeedback::Dismissed),
        "snoozed" => Some(WorkFeedback::Snoozed {
            hour: hour.ok_or(WorkError::Invalid)?,
        }),
        "remove" => None,
        _ => return Err(WorkError::Invalid.into()),
    };
    let access = access(ctx).await?;
    let now = crate::runtime::now();
    ctx.data()
        .state
        .commit_work(move |store| {
            store.feedback(access, delivery_id, value, correction.unwrap_or(false), now)
        })
        .await?;
    reply(ctx, "Feedback saved for this scope. At least five attributable observations are required for a learned adjustment; silence never counts against a delivery.").await
}

#[poise::command(slash_command, ephemeral)]
pub async fn automation(
    ctx: Context<'_>,
    #[description = "Explicitly enable delivery for this scope"] enabled: bool,
    #[description = "IANA timezone, such as America/New_York"] timezone: Option<String>,
    #[description = "Local briefing hour 0–23"] briefing_hour: Option<u8>,
    #[description = "Quiet hours start 0–23"] quiet_start: Option<u8>,
    #[description = "Quiet hours end 0–23"] quiet_end: Option<u8>,
    #[description = "Daily ceiling 1–4 including briefings and reminders"] daily_limit: Option<u8>,
) -> Result<(), Error> {
    let access = access(ctx).await?;
    let configured_owner = std::env::var("ABBEY_WORK_DEFAULT_TIMEZONE_USER_ID").ok();
    let update = WorkAutomationUpdate {
        enabled,
        timezone,
        briefing_hour,
        quiet_start,
        quiet_end,
        daily_limit,
    };
    let policy = ctx
        .data()
        .state
        .commit_work(move |store| {
            store.update_automation(access, update, configured_owner.as_deref())
        })
        .await?;
    let text = automation_reply(&policy);
    reply(ctx, text).await
}

fn automation_reply(policy: &WorkAutomationPolicy) -> String {
    format!(
        "Saved automation {} for this scope in {}. Briefing hour: {}; quiet hours: {}–{}; maximum {} deliveries per local day, shared by all projects, briefings and reminders. Delivery runtime is not active in this implementation stage.",
        if policy.enabled { "opt-in" } else { "disabled" },
        policy.timezone,
        policy.briefing_hour,
        policy.quiet_start,
        policy.quiet_end,
        policy.daily_limit
    )
}

#[poise::command(slash_command, ephemeral)]
pub async fn reminder(
    ctx: Context<'_>,
    #[description = "Task number"] task_id: u64,
    #[description = "Current task revision"] revision: u64,
    #[description = "Unix seconds; omit to cancel the reminder"] at: Option<u64>,
) -> Result<(), Error> {
    if at.is_some_and(|at| at <= crate::runtime::now()) {
        return Err(WorkError::Invalid.into());
    }
    let access = access(ctx).await?;
    let (revision, ceiling) = ctx
        .data()
        .state
        .commit_work(move |store| {
            store.set_reminder(access, task_id, revision, at)?;
            let ceiling = store
                .scope_automation
                .get(&access.scope().key())
                .map_or(4, |p| p.daily_limit);
            Ok((
                store
                    .tasks
                    .get(&task_id)
                    .ok_or(WorkError::Missing)?
                    .revision,
                ceiling,
            ))
        })
        .await?;
    reply(ctx, format!("Reminder saved for task #{task_id}. Revision {revision}. Delivery requires scope automation opt-in; quiet hours and the shared ceiling of {ceiling} deliveries per local day apply. The deadline is unchanged.")).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn render_preferences_and_cleared_timing_in_local_language() {
        let snapshot = crate::work::WorkPreferenceSnapshot {
            timezone: Some("Europe/London".into()),
            effective_hour: 15,
            profile: crate::work::WorkPreferenceProfile {
                evidence: vec![crate::work::PreferenceEvidence {
                    actor: Some(1),
                    scope: Some(crate::work::WorkScope::Personal { owner: 1 }),
                    kind: Some(crate::work::WorkDeliveryKind::Briefing),
                    delivery_id: 42,
                    feedback: crate::work::WorkFeedback::Snoozed { hour: 11 },
                    at: 100,
                }],
                ..Default::default()
            },
        };
        let preferences = super::preference_reply(&snapshot, 1);
        let timing = super::timing_reply(&snapshot);
        println!("{preferences}\n{timing}");
        assert!(preferences.contains("Explicit hour: not set"));
        assert!(preferences.contains("snoozed to 11:00 (Europe/London)"));
        assert!(timing.contains("Effective hour: 15:00 (Europe/London)"));
        assert!(!super::preference_reply(&snapshot, 2).contains("Delivery #42"));
        assert_eq!(super::hour_text(Some(7)), "7:00");
    }

    #[test]
    fn render_configuration_discloses_scope_and_ceiling() {
        let policy = crate::work::WorkAutomationPolicy {
            timezone: "America/New_York".into(),
            ..Default::default()
        };
        let text = super::automation_reply(&policy);
        println!("{text}");
        assert!(text.contains("maximum 4 deliveries per local day"));
        assert!(text.contains("shared by all projects"));
    }
}

//! Human-assessed contextual proposals and explicitly scoped shadow publication.
use super::*;
use crate::moderation::contextual::{self, Assessment};
use crate::moderation::shadow::{Disposition, Mutation, NewCase};

mod evidence;

#[derive(Debug, poise::ChoiceParameter)]
pub enum ContextAssessmentChoice {
    #[name = "confirmed offending in context"]
    Confirmed,
    #[name = "ambiguous or insufficient context"]
    Ambiguous,
    #[name = "quotation or member report"]
    Quotation,
}

async fn reply(ctx: Context<'_>, content: String) -> Result<(), Error> {
    ctx.send(
        poise::CreateReply::default()
            .content(clamp_message(content))
            .ephemeral(true)
            .allowed_mentions(crate::gateway::no_mentions()),
    )
    .await?;
    Ok(())
}

pub(super) async fn propose(
    ctx: Context<'_>,
    _guild: &PartialGuild,
    _moderator: &Member,
    user: &User,
    severity: Severity,
    source_message: &str,
    assessment: Option<ContextAssessmentChoice>,
) -> Result<(), Error> {
    let id = if !source_message.is_empty()
        && source_message.len() <= 20
        && source_message.bytes().all(|b| b.is_ascii_digit())
    {
        source_message.parse::<u64>().ok().filter(|id| *id != 0)
    } else {
        None
    };
    let Some(id) = id else {
        return reply(ctx, "Provide the numeric ID of a message in this channel. No contextual proposal qualified.".into()).await;
    };
    let assessment = match assessment {
        Some(ContextAssessmentChoice::Confirmed) => Assessment::ConfirmedOffending,
        Some(ContextAssessmentChoice::Quotation) => Assessment::QuotationOrReport,
        _ => Assessment::Ambiguous,
    };
    let mut candidate_id = None;
    let result = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let native = evidence::envelope(ctx)?;
        let initial = evidence::prove(ctx.http(), native, user.id.get(), id, severity, assessment).await?;
        let unsaved = |input| match contextual::qualify(input) {
            Ok(proposal) => contextual::render(proposal),
            Err(reason) => format!("{reason} No action taken. This reply is not a saved case."),
        };
        let state = &ctx.data().state;
        let Some(path) = state.community_policy_path.clone() else { return Ok(unsaved(initial.input)); };
        let (policy, digest) = match state.community_policy(path.clone()).await {
            Ok(loaded) => loaded,
            Err(_) => return Ok(format!("{} Shadow policy could not be confirmed; no case saved.", unsaved(initial.input))),
        };
        if policy.guild != native.guild || policy.owner != initial.owner {
            return Err("Current shadow policy ownership could not be confirmed. No case saved.");
        }
        let blocked = !policy.contextual_shadow.enabled
            || !policy.contextual_shadow.source_channels.contains(&native.channel)
            || policy.mode == crate::community_ops::Mode::Stopped;
        // Refresh all native facts and read the exact source last, after policy I/O.
        let fresh = evidence::prove(ctx.http(), native, user.id.get(), id, severity, assessment).await?;
        if fresh.owner != initial.owner || fresh.source != initial.source {
            return Err("The source or current authority changed. No shadow case saved; refresh before deciding.");
        }
        let new = match NewCase::human_assessed(fresh.source.clone(), fresh.input, fresh.authority()?) {
            Ok(new) => new,
            Err(_) if blocked => return Ok(unsaved(fresh.input)),
            Err(reason) => return Err(reason),
        };
        let exact_id = fresh.source.case_id()?;
        candidate_id = Some(exact_id.clone());
        if blocked && !state.existing_moderation_shadow(path.clone(), digest.clone(), new.clone()).await
            .map_err(|_| "An existing shadow receipt could not be confirmed. No new case was admitted.")? {
            return Ok(unsaved(fresh.input));
        }
        let receipt = match state.publish_moderation_shadow(path, digest, Mutation::Capture(new)).await {
            Ok(receipt) => receipt,
            Err(_) => return Ok(format!(
                "Shadow publication is unconfirmed for case `{}`. No action taken; use `/modcase show` to inspect it or explicitly retry the same source.",
                exact_id)),
        };
        let disposition = match assessment {
            Assessment::ConfirmedOffending => Disposition::ConfirmedProposal {
                timeout_minutes: match severity { Severity::Minor => None, _ => Some(contextual::MAX_CONTEXTUAL_TIMEOUT_MINUTES) },
            },
            _ => Disposition::HumanReview,
        };
        let detail = match disposition {
            Disposition::ConfirmedProposal { timeout_minutes } => format!(
                "Human-assessed proposal: delete the linked offending message. {}",
                timeout_minutes.map_or_else(|| "No timeout proposed.".into(), |n| format!("Proposed timeout: {n} minutes maximum."))),
            Disposition::HumanReview => "Ambiguous, quoted or reported content referred for human review; no sanction proposed.".into(),
        };
        let saved = match receipt.change {
            crate::moderation::shadow::Change::Changed => "Saved shadow case",
            crate::moderation::shadow::Change::AlreadyObserved => "Confirmed existing shadow case",
        };
        Ok(format!(
            "{} `{}` revision {}. No action taken.\nSource: https://discord.com/channels/{}/{}/{}\n{}\nThis case records a human moderator's assessment; Abbey did not independently classify the message. Use `/modcase show` for the receipt, `/modcase review` for an independent staff decision, or `/modcase appeal` as the current subject in the source channel.\nOperational review records contain IDs and a source digest, not message text. Learning-memory erasure does not remove these records.",
            saved, receipt.case_id, receipt.case_revision, native.guild, native.channel, id, detail))
    }).await;
    let content = match result {
        Ok(Ok(content)) => content,
        Ok(Err(reason)) => format!("{reason} No action taken."),
        Err(_) => candidate_id.map_or_else(
            || "Shadow work could not be confirmed in time. No action taken; refresh current access before retrying.".into(),
            |id| format!("Shadow work could not be confirmed in time for case `{id}`. No action taken; use `/modcase show` before retrying the same source.")),
    };
    reply(ctx, content).await
}

//! Slash requests create durable candidates; the retained scheduler owns delivery.
use super::*;
#[derive(Debug, Clone, Copy, poise::ChoiceParameter)]
pub enum InvitationKind {
    #[name = "activity"]
    Activity,
    #[name = "voice"]
    Voice,
}
impl From<InvitationKind> for EngagementKind {
    fn from(kind: InvitationKind) -> Self {
        match kind {
            InvitationKind::Activity => Self::ActivityInvite,
            InvitationKind::Voice => Self::VoiceInvite,
        }
    }
}
#[poise::command(slash_command, ephemeral)]
pub async fn invite(
    ctx: Context<'_>,
    #[description = "Request an Activity or voice invitation"] kind: InvitationKind,
) -> Result<(), Error> {
    let at = crate::runtime::now();
    if matches!(kind, InvitationKind::Voice)
        && !ctx.data().state.voice_invitation_available(&origin(ctx))
    {
        return reply(ctx,"Voice invitations are disabled: a selected local voice backend and currently eligible local text route are required. `/voice status` shows setup; this command grants no consent.").await;
    }
    let activity = if matches!(kind, InvitationKind::Activity) {
        match crate::runtime::activity_readiness::current_activity(at).await {
            Ok(r) => Some(ActivityVersion {
                origin: r.https_origin,
                digest: r.deployed_digest,
            }),
            Err(reason) => return reply(ctx, reason).await,
        }
    } else {
        None
    };
    let poise::Context::Application(application) = ctx else {
        return Err(crate::work::WorkError::Invalid.into());
    };
    let request = InvitationRequest {
        activity,
        interaction: application.interaction.id.get(),
        member: ctx.author().id.get(),
        scope: origin(ctx),
        at,
    };
    let result = ctx
        .data()
        .state
        .commit_engagement(move |s| s.request_invitation(kind.into(), request))
        .await;
    match result {
        Ok(Some(id)) => reply(ctx,format!("Saved invitation candidate {id}. The retained scheduler will recheck your explicit contact settings, quiet hours, shared contact limits and current access. Server invitations also require the server’s unsolicited policy and budget. This does not start an Activity or voice session; `/engage status` shows delivery state.")).await,
        Ok(None) => reply(ctx,"An invitation is already pending in this origin, or this exact request was already recorded. No duplicate candidate was created.").await,
        Err(crate::work::WorkError::Denied) => reply(ctx,"Invitation was not saved. Prior direct interaction or subscription establishes eligibility; you must also save a positive daily limit and IANA timezone with `/engage configure` and explicitly resume any applicable stop.").await,
        Err(error) => saved(ctx,Err(error),"").await,
    }
}
#[cfg(test)]
mod tests;

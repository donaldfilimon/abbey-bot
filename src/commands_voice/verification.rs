//! Owner/admin-scoped Discord shell for redacted live voice acceptance.

use std::sync::Arc;

use serenity::all::Permissions;

use crate::voice::VoiceMode;
use crate::voice_session::{VoicePhase, VoiceRuntime};
use crate::{Context, Error};

/// Arm or read a content-free live acceptance run.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "ADMINISTRATOR",
    required_permissions = "ADMINISTRATOR",
    subcommands("voice_verify_start", "voice_verify_report"),
    rename = "verify"
)]
pub async fn voice_verify(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

fn can_access_voice_verification(
    is_owner: bool,
    interaction_permissions: Option<Permissions>,
) -> bool {
    let mut input = crate::command_catalog::EligibilityInput::new(
        crate::command_catalog::InteractionContext::Guild,
    );
    input.application_owner = is_owner;
    input.permissions =
        crate::commands_help::permissions_input(interaction_permissions.unwrap_or_default());
    crate::command_catalog::access_allows(crate::command_catalog::AccessId::A7.rule(), &input)
}

async fn voice_verification_runtime(ctx: Context<'_>) -> Result<Arc<VoiceRuntime>, &'static str> {
    let runtime = ctx
        .data()
        .voice
        .as_ref()
        .cloned()
        .ok_or("Abbey voice is not configured.")?;
    let guild_id = ctx
        .guild_id()
        .ok_or("This command only works inside a server.")?;
    if guild_id.get() != runtime.config.guild_id {
        return Err("Abbey voice is locked to a different server by deployment configuration.");
    }
    let is_owner = ctx.framework().options().owners.contains(&ctx.author().id);
    let interaction_permissions = ctx
        .author_member()
        .await
        .and_then(|member| member.permissions);
    if !can_access_voice_verification(is_owner, interaction_permissions) {
        return Err(
            "Only the application owner or an administrator can control or read live voice verification.",
        );
    }
    Ok(runtime)
}

/// Start one local, content-free acceptance run before the consented join.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "ADMINISTRATOR",
    required_permissions = "ADMINISTRATOR",
    rename = "start"
)]
pub async fn voice_verify_start(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let runtime = match voice_verification_runtime(ctx).await {
        Ok(runtime) => runtime,
        Err(error) => {
            ctx.say(error).await?;
            return Ok(());
        }
    };
    if runtime.effective_mode() != VoiceMode::Local {
        ctx.say("Privacy-safe live verification is available only for local voice mode; disabled mode has no media and the direct cloud backup does not expose local STT completion.")
            .await?;
        return Ok(());
    }
    let snapshot = runtime.snapshot().await;
    if snapshot.start_pending
        || !matches!(
            snapshot.phase,
            VoicePhase::Disconnected | VoicePhase::PresenceOnly | VoicePhase::Failed
        )
    {
        ctx.say("Start verification before the consented join. Leave the current conversational session first so the run can observe the complete join, participant-change resume, and final leave sequence.")
            .await?;
        return Ok(());
    }
    match runtime.begin_verification() {
        Ok(run) => {
            ctx.say(format!(
                "Armed redacted live voice verification run {} in process memory. It records only fixed counters, disables conversation commits, and does not start capture. Collect unanimous current consent, then use the normal manager `/voice join consent:true` flow.",
                run.run
            ))
            .await?;
        }
        Err(error) => {
            ctx.say(error).await?;
        }
    }
    Ok(())
}

/// Render the current redacted acceptance report without ending the run.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "ADMINISTRATOR",
    required_permissions = "ADMINISTRATOR",
    rename = "report"
)]
pub async fn voice_verify_report(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let runtime = match voice_verification_runtime(ctx).await {
        Ok(runtime) => runtime,
        Err(error) => {
            ctx.say(error).await?;
            return Ok(());
        }
    };
    ctx.say(crate::commands::clamp_message(
        runtime.verification_report(),
    ))
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_surface_is_owner_or_administrator_only() {
        assert!(can_access_voice_verification(true, None));
        assert!(can_access_voice_verification(
            false,
            Some(Permissions::ADMINISTRATOR)
        ));
        assert!(!can_access_voice_verification(
            false,
            Some(Permissions::MANAGE_GUILD)
        ));
        assert!(!can_access_voice_verification(
            false,
            Some(Permissions::VIEW_CHANNEL)
        ));
        assert!(!can_access_voice_verification(false, None));
    }

    #[test]
    fn verification_subcommands_keep_the_administrator_runtime_guard() {
        let root = crate::commands_voice::voice();
        let verify = root
            .subcommands
            .iter()
            .find(|command| command.name == "verify")
            .expect("voice verify group");
        assert!(verify.guild_only);
        assert!(verify.ephemeral);
        assert_eq!(verify.required_permissions, Permissions::ADMINISTRATOR);
        assert_eq!(verify.subcommands.len(), 2);
        for command in &verify.subcommands {
            assert!(matches!(command.name.as_str(), "start" | "report"));
            assert!(command.guild_only);
            assert!(command.ephemeral);
            assert_eq!(command.required_permissions, Permissions::ADMINISTRATOR);
        }
    }
}

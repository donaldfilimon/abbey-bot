//! Poise framework translation into fixed recovery guidance.
use crate::gateway::interaction_outcomes::{delivery_failed, record_failure};
use crate::observability::{EventCode, OperationalErrorCategory};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Guidance {
    Failed,
    Check,
    Arguments,
    Permission,
    Cooldown,
    Context,
}
impl Guidance {
    const fn message(self) -> &'static str {
        match self {
            Self::Failed => {
                "Abbey could not complete this command or deliver its result. A change may already have taken effect. Check the current state before retrying a change; use `/help` for the supported workflow."
            }
            Self::Check => {
                "Abbey could not verify this command's prerequisites. Nothing was started. Open `/help` and check the task's current requirements before retrying."
            }
            Self::Arguments => {
                "Abbey could not read this command's inputs. Open `/help`, then select the command again and check its required inputs."
            }
            Self::Permission => {
                "Discord could not confirm the permissions required for this command. Ask a server manager to review access, then try again."
            }
            Self::Cooldown => "This command is cooling down. Wait a moment before trying it again.",
            Self::Context => {
                "This command is unavailable in this context. Open `/help` here to find the supported workflow."
            }
        }
    }
}

pub(super) async fn handle(error: poise::FrameworkError<'_, crate::Data, crate::Error>) {
    use poise::FrameworkError as F;
    // The central guard already delivered a reason when it returned false.
    if matches!(error, F::CommandCheckFailed { error: None, .. }) {
        return;
    }
    let guidance = match &error {
        F::CommandCheckFailed { .. } => Guidance::Check,
        F::ArgumentParse { .. }
        | F::CommandStructureMismatch { .. }
        | F::SubcommandRequired { .. } => Guidance::Arguments,
        F::MissingBotPermissions { .. }
        | F::MissingUserPermissions { .. }
        | F::NotAnOwner { .. } => Guidance::Permission,
        F::CooldownHit { .. } => Guidance::Cooldown,
        F::GuildOnly { .. } | F::DmOnly { .. } | F::NsfwOnly { .. } => Guidance::Context,
        _ => Guidance::Failed,
    };
    let category = if matches!(error, F::CommandPanic { .. }) {
        OperationalErrorCategory::Panic
    } else if guidance == Guidance::Permission {
        OperationalErrorCategory::Authorization
    } else {
        OperationalErrorCategory::Internal
    };
    if let Some(ctx) = error.ctx() {
        record_failure(&ctx.data().state, EventCode::CommandFailure, category);
        super::record_interaction(
            ctx,
            false,
            Some(crate::memory::InteractionErrorCategory::Internal),
        );
        let response = ctx
            .send(
                poise::CreateReply::default()
                    .content(crate::commands::clamp_message(guidance.message().into()))
                    .ephemeral(true)
                    .allowed_mentions(crate::gateway::no_mentions()),
            )
            .await;
        if response.is_err() {
            delivery_failed(&ctx.data().state);
        }
    } else if let F::UnknownInteraction {
        ctx,
        interaction,
        framework,
        ..
    } = &error
    {
        use serenity::all::{CreateInteractionResponse, CreateInteractionResponseMessage};
        let response = interaction
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content(crate::commands::clamp_message(
                            Guidance::Arguments.message().into(),
                        ))
                        .ephemeral(true)
                        .allowed_mentions(crate::gateway::no_mentions()),
                ),
            )
            .await;
        if response.is_err() {
            delivery_failed(&framework.user_data.state);
        }
    } else if let F::EventHandler { framework, .. }
    | F::NonCommandMessage { framework, .. }
    | F::UnknownCommand { framework, .. } = &error
    {
        record_failure(
            &framework.user_data.state,
            EventCode::CommandFailure,
            category,
        );
    } else {
        // Framework-level failures have no safe command response target.
        tracing::error!(?category, "framework callback failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_guidance_is_bounded_and_has_no_mentions_or_error_input() {
        for guidance in [
            Guidance::Failed,
            Guidance::Check,
            Guidance::Arguments,
            Guidance::Permission,
            Guidance::Cooldown,
            Guidance::Context,
        ] {
            let text = crate::commands::clamp_message(guidance.message().into());
            assert!(text.chars().count() < 2000);
            assert!(!text.contains('@'));
            assert!(!text.contains("secret"));
        }
        assert!(
            Guidance::Failed
                .message()
                .contains("already have taken effect")
        );
    }
}

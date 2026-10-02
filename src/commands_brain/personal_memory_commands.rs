//! Authenticated, private Discord entry points for retained personal-memory changes.
use super::*;
use crate::personal_memory::{
    ConsentStatus, MemberProof, MemoryConsentError, PERSONAL_MEMORY_POLICY_VERSION,
    SelfAuthorizedFactAction, UseChoice,
};

fn member_action(
    actor: u64,
    subject: u64,
    bot: bool,
    guild: Option<u64>,
    interaction: u64,
    at: u64,
    stamp: crate::personal_memory::ConsentStamp,
) -> Result<SelfAuthorizedFactAction, MemoryConsentError> {
    if bot || actor != subject || interaction == 0 {
        return Err(MemoryConsentError::InvalidProof);
    }
    SelfAuthorizedFactAction::new(
        MemberProof {
            actor: format!("discord:{actor}"),
            subject: format!("discord:{subject}"),
            guild: guild.map_or_else(
                || format!("discord:dm:{actor}"),
                |id| format!("discord:{id}"),
            ),
            interaction_id: interaction.to_string(),
            platform: "discord".into(),
            at,
            policy_version: PERSONAL_MEMORY_POLICY_VERSION,
        },
        stamp,
    )
}

pub(super) fn authenticated_action(
    ctx: Context<'_>,
    subject: u64,
) -> Result<SelfAuthorizedFactAction, MemoryConsentError> {
    // Only a Discord application interaction can supply authorship evidence.
    let poise::Context::Application(application) = ctx else {
        return Err(MemoryConsentError::InvalidProof);
    };
    let guild = scoped_guild(ctx);
    let user = scoped_user(ctx.author());
    member_action(
        ctx.author().id.get(),
        subject,
        ctx.author().bot,
        ctx.guild_id().map(|id| id.get()),
        application.interaction.id.get(),
        runtime::now(),
        ctx.data().state.personal_memory_status(&guild, &user).stamp,
    )
}

pub(super) fn failure(error: &MemoryConsentError, withdrawing: bool) -> String {
    let detail = match error {
        MemoryConsentError::Stale => {
            "The memory changed while this request was being processed. Review its current status and retry."
        }
        MemoryConsentError::Blocked => {
            "Personal memory is temporarily blocked pending persistence recovery."
        }
        MemoryConsentError::Persistence => "Durable completion could not be verified.",
        MemoryConsentError::RequestConflict => {
            "This interaction identity already belongs to a different request."
        }
        MemoryConsentError::NotFound | MemoryConsentError::UnverifiedFact => {
            "The exact selected fact is no longer available. Review your stored facts and retry."
        }
        MemoryConsentError::InvalidProof => {
            "This request could not be authenticated as your own Discord interaction."
        }
        MemoryConsentError::Bounds => "The request exceeds personal-memory limits.",
    };
    let fence = if withdrawing
        && matches!(
            error,
            MemoryConsentError::Persistence
                | MemoryConsentError::Stale
                | MemoryConsentError::Blocked
        ) {
        " Durable withdrawal is not confirmed. Review `/memory_use` for the current generated-use state; any immediate withdrawal fence remains in place."
    } else {
        " No completed change is confirmed."
    };
    format!("{detail}{fence}")
}

fn render_status(status: &ConsentStatus) -> String {
    format!(
        "Your personal memory in this conversation scope\nGenerated use: **{}**\nEligible facts: {} · facts awaiting confirmation or use consent: {}\n\nStoring facts and allowing generated use are separate choices. Generated use defaults to off. Use `/memory_use choice:on` or `choice:off`; neither deletes stored facts. Confirm a legacy fact only by supplying its exact text with `confirm`. Confirmation alone does not turn use on. Use `/recall` to review stored facts and `/forget` to delete one.",
        if status.choice == UseChoice::On {
            "on"
        } else {
            "off"
        },
        status.eligible_facts,
        status.unverified_facts,
    )
}

/// Privately review personal memory use or explicitly change your own choice.
#[poise::command(slash_command, ephemeral)]
pub async fn memory_use(
    ctx: Context<'_>,
    #[description = "Allow or withdraw generated use of your verified facts"] choice: Option<OnOff>,
    #[description = "Exact stored fact you personally confirm; does not enable generated use"]
    #[autocomplete = "super::memory_commands::autocomplete_fact"]
    #[max_length = 300]
    confirm: Option<String>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    if choice.is_some() && confirm.is_some() {
        return send_private_no_mentions(
            ctx,
            "Choose either a use-consent change or an exact fact confirmation in one request."
                .into(),
        )
        .await;
    }
    let state = &ctx.data().state;
    let guild = scoped_guild(ctx);
    let user = scoped_user(ctx.author());
    let action = match authenticated_action(ctx, ctx.author().id.get()) {
        Ok(action) => action,
        Err(error) => return send_private_no_mentions(ctx, failure(&error, false)).await,
    };
    let withdrawing = matches!(choice, Some(OnOff::Off));
    let result = if let Some(choice) = choice {
        state
            .set_personal_memory_use(
                action,
                if choice.is_on() {
                    UseChoice::On
                } else {
                    UseChoice::Off
                },
                format!("discord:{}:memory_use", ctx.id()),
            )
            .await
    } else if let Some(exact) = confirm {
        // Deliberately no fuzzy resolver: confirmation binds the exact reviewed text.
        let key = crate::personal_memory::fact_key(&guild, &user, &exact);
        state
            .confirm_personal_memory_fact(
                action,
                key,
                exact,
                format!("discord:{}:memory_use", ctx.id()),
            )
            .await
    } else {
        return send_private_no_mentions(
            ctx,
            render_status(&state.personal_memory_status(&guild, &user)),
        )
        .await;
    };
    let content = match result {
        Ok(status) => format!(
            "Your retained change completed.\n\n{}",
            render_status(&status)
        ),
        Err(error) => failure(&error, withdrawing),
    };
    send_private_no_mentions(ctx, content).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authenticated_proof_is_self_only_and_binds_actual_interaction_and_scope() {
        let stamp = crate::personal_memory::ConsentStamp {
            revision: 3,
            consent_epoch: 4,
            exposure_epoch: 5,
        };
        let action = member_action(7, 7, false, None, 123, 42, stamp).unwrap();
        assert_eq!(action.proof.guild, "discord:dm:7");
        assert_eq!(action.proof.interaction_id, "123");
        assert_eq!(action.proof.at, 42);
        assert_eq!(action.expected, stamp);
        assert!(member_action(7, 8, false, Some(9), 123, 42, stamp).is_err());
        assert!(member_action(7, 7, true, Some(9), 123, 42, stamp).is_err());
        assert!(member_action(7, 7, false, Some(9), 0, 42, stamp).is_err());
    }
    #[test]
    fn memory_use_registered_private_and_bound_to_catalog_guard() {
        let commands = crate::application_commands();
        let command = commands
            .iter()
            .find(|command| command.name == "memory_use")
            .unwrap();
        assert_eq!(command.checks.len(), 1);
        assert!(command.ephemeral);
        assert_eq!(
            command
                .custom_data
                .downcast_ref::<crate::commands_help::CatalogBinding>()
                .unwrap()
                .key,
            crate::command_catalog::CommandKey::MemoryUse
        );
        assert_eq!(
            command
                .parameters
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["choice", "confirm"]
        );
        assert!(
            crate::command_catalog::command(crate::command_catalog::CommandKey::MemoryUse).private
        );
    }

    #[test]
    fn failure_copy_never_claims_completed_change() {
        for error in [
            MemoryConsentError::Persistence,
            MemoryConsentError::Stale,
            MemoryConsentError::Blocked,
            MemoryConsentError::RequestConflict,
        ] {
            let text = failure(&error, false);
            assert!(!text.contains("Stored"));
            assert!(text.contains("No completed change"));
        }
        assert!(
            failure(&MemoryConsentError::Persistence, true)
                .contains("Durable withdrawal is not confirmed")
        );
    }
}

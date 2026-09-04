//! Persistent member-authenticated controls; no process-lifetime collectors.

use serenity::all::{
    ButtonStyle, ChannelId, ComponentInteraction, CreateActionRow, CreateButton,
    CreateInteractionResponse, CreateInteractionResponseMessage, CreateMessage,
    EditInteractionResponse, GuildId,
};

use crate::{
    Context, Error,
    gateway::shared::clamp_message,
    voice::VoiceMode,
    voice_consent::{self, Choice},
    voice_session::VoiceRuntime,
};

pub(super) fn buttons(mode: VoiceMode) -> Vec<CreateActionRow> {
    let mut buttons = Vec::new();
    if let Some(id) = voice_consent::button_id(mode) {
        buttons.push(
            CreateButton::new(id)
                .label(match mode {
                    VoiceMode::Local => "Agree — local",
                    _ => "Agree — OpenAI",
                })
                .style(ButtonStyle::Success),
        );
    }
    buttons.push(
        CreateButton::new(voice_consent::STOP_ID)
            .label("Stop / withdraw")
            .style(ButtonStyle::Danger),
    );
    vec![CreateActionRow::Buttons(buttons)]
}

fn personal_status(runtime: &VoiceRuntime, user: u64, mode: VoiceMode) -> String {
    let choice = if runtime.consent.agrees(user, mode) {
        "saved agreement"
    } else {
        "no usable saved agreement"
    };
    format!("Your choice for {}: **{choice}**.", mode.label())
}

/// Review your saved voice choice and agree or withdraw privately.
#[poise::command(slash_command, guild_only, ephemeral, rename = "consent")]
pub async fn voice_consent(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(runtime) = ctx.data().voice.as_ref().filter(|runtime| {
        ctx.guild_id()
            .is_some_and(|guild| guild.get() == runtime.config.guild_id)
    }) else {
        ctx.say("Voice choices are available only in Abbey's configured voice server.")
            .await?;
        return Ok(());
    };
    let mode = runtime.effective_mode();
    ctx.send(
        poise::CreateReply::default()
            .content(clamp_message(format!(
                "{}\n\n{}",
                voice_consent::notice(mode, runtime.config.channel_id),
                personal_status(runtime, ctx.author().id.get(), mode)
            )))
            .components(buttons(mode)),
    )
    .await?;
    Ok(())
}

/// Post the current member choice notice in the configured voice channel.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    default_member_permissions = "MANAGE_GUILD",
    required_permissions = "MANAGE_GUILD",
    rename = "notice"
)]
pub async fn voice_notice(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some(runtime) = ctx.data().voice.as_ref().filter(|runtime| {
        ctx.guild_id()
            .is_some_and(|guild| guild.get() == runtime.config.guild_id)
    }) else {
        ctx.say("Abbey voice is not configured in this server.")
            .await?;
        return Ok(());
    };
    let mode = runtime.effective_mode();
    let message = ChannelId::new(runtime.config.channel_id)
        .send_message(
            ctx.http(),
            CreateMessage::new()
                .content(clamp_message(voice_consent::notice(
                    mode,
                    runtime.config.channel_id,
                )))
                .components(buttons(mode))
                .allowed_mentions(crate::gateway::no_mentions()),
        )
        .await?;
    ctx.say(format!(
        "Member voice choices are ready: {}",
        message.link()
    ))
    .await?;
    Ok(())
}

pub(super) fn caller_may_stop(
    ctx: &serenity::all::Context,
    runtime: &VoiceRuntime,
    user: u64,
) -> bool {
    let participants = super::discord::cached_participants_from_serenity(
        ctx,
        GuildId::new(runtime.config.guild_id),
        ChannelId::new(runtime.config.channel_id),
    );
    roster_authorizes_stop(participants.as_ref(), user)
}

fn roster_authorizes_stop(
    participants: Option<&std::collections::HashSet<u64>>,
    user: u64,
) -> bool {
    // Unknown gateway membership fails closed. A known absent member may
    // withdraw their receipt without interrupting other people's call.
    participants.is_some_and(|users| users.contains(&user))
}

pub(super) async fn component(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &crate::Data,
) -> bool {
    if !interaction.data.custom_id.starts_with("abbey:voice:") {
        return false;
    }
    let Some(runtime) = data.voice.as_ref().filter(|runtime| {
        interaction
            .guild_id
            .is_some_and(|guild| guild.get() == runtime.config.guild_id)
    }) else {
        return false;
    };
    if interaction.message.author.id != ctx.cache.current_user().id || interaction.user.bot {
        return false;
    }
    let choice = voice_consent::parse_button(&interaction.data.custom_id);
    let allowed = choice.filter(|choice| match choice {
        Choice::Agree(mode) => runtime.config.backend_for(*mode).is_some(),
        Choice::Withdraw | Choice::WithdrawSpoken => true,
    });
    let user = interaction.user.id.get();
    let stop_call = allowed == Some(Choice::Withdraw) && caller_may_stop(ctx, runtime, user);
    // Negative authorization and the software gate happen before the first
    // await. The durable write is cancellation-independent and never delays
    // Discord's defer or the physical call teardown.
    let save = allowed.map(|choice| {
        runtime.change_consent(
            user,
            interaction.id.get(),
            choice,
            crate::runtime::now(),
            stop_call,
        )
    });
    let epoch_to_stop = save.as_ref().and_then(|change| change.epoch_to_stop);
    let (deferred, result) = tokio::join!(
        interaction.create_response(
            &ctx.http,
            CreateInteractionResponse::Defer(
                CreateInteractionResponseMessage::new().ephemeral(true)
            )
        ),
        async {
            if let Some(epoch) = epoch_to_stop {
                super::supervision::stop_voice_for_withdrawal(ctx, runtime, epoch).await;
            }
            match save {
                Some(change) => match change.saved.await {
                    Ok(result) => result,
                    Err(_) => Err(
                        "Saving your choice failed. Ask the operator to inspect the consent store before restarting Abbey.",
                    ),
                },
                None => Err(
                    "This notice is outdated or its processing mode is unavailable. Use /voice consent for the current notice.",
                ),
            }
        }
    );
    if deferred.is_err() {
        tracing::warn!("voice choice response could not be deferred");
        return true;
    }
    let reply = match result {
        Ok(true) if allowed == Some(Choice::Withdraw) => {
            if epoch_to_stop.is_some() {
                "Your saved agreement is withdrawn for both modes. Audio processing has stopped. You can agree again through /voice consent."
            } else {
                "Your saved agreement is withdrawn for both modes. Abbey will not activate a call containing you until you agree again."
            }
        }
        Ok(true) => {
            "Your agreement is saved for this server, processing mode and notice version. You will not need to agree again after visits or restarts. When everyone present has agreed, a manager can use /voice join consent:true or /voice resume consent:true."
        }
        Ok(false) => {
            "A newer choice has already been recorded. Use /voice consent to see or change your current choice."
        }
        Err(error) => error,
    };
    if interaction
        .edit_response(
            &ctx.http,
            EditInteractionResponse::new().content(clamp_message(reply.into())),
        )
        .await
        .is_err()
    {
        tracing::warn!("voice choice acknowledgment could not be delivered");
    }
    true
}

pub(super) fn coverage_text(
    runtime: &VoiceRuntime,
    users: &std::collections::HashSet<u64>,
    mode: VoiceMode,
) -> String {
    match runtime.consent.coverage(users, mode) {
        Ok(missing) => format!(
            "Saved voice choices: {}/{} present members covered for {}.{}",
            users.len() - missing.len(),
            users.len(),
            mode.label(),
            if missing.is_empty() {
                ""
            } else {
                " Each uncovered member must choose Agree in /voice consent or the voice notice."
            }
        ),
        Err(error) => error.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::roster_authorizes_stop;

    #[test]
    fn missing_roster_does_not_authorize_stopping_the_call() {
        assert!(!roster_authorizes_stop(None, 42));
    }

    #[test]
    fn known_absent_user_does_not_authorize_stopping_the_call() {
        let participants = HashSet::from([7, 11]);

        assert!(!roster_authorizes_stop(Some(&participants), 42));
    }

    #[test]
    fn known_present_user_authorizes_stopping_the_call() {
        let participants = HashSet::from([7, 42]);

        assert!(roster_authorizes_stop(Some(&participants), 42));
    }
}

//! Focused media command adapters.
use super::*;

/// Summarize the recent messages Abbey has seen in this channel.
#[poise::command(slash_command)]
pub async fn summarize(
    ctx: Context<'_>,
    #[description = "How many recent messages (10–200, default 50)"]
    #[min = 10]
    #[max = 200]
    count: Option<usize>,
    #[description = "Force a persona"] r#as: Option<PersonaChoice>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let _ = (count, r#as);
    ctx.send(poise::CreateReply::default()
        .content("Channel summarization is temporarily unavailable: retained messages need source authorization before Abbey can use them for generation. No channel history was read or submitted. You can type a fresh request with `/persona ask`; its answer is visible in the channel. For a private conversation, send Abbey a DM.")
        .ephemeral(true)
        .allowed_mentions(crate::gateway::no_mentions()))
        .await?;
    Ok(())
}

async fn fetch_attachment(state: &AppState, att: &Attachment) -> Result<Vec<u8>, String> {
    if usize::try_from(att.size).is_ok_and(|s| s > vision::MAX_IMAGE_BYTES) {
        return Err(format!(
            "that image is {} bytes; the cap is {}",
            att.size,
            vision::MAX_IMAGE_BYTES
        ));
    }
    crate::gateway::fetch_capped(&state.attachments, &att.url, vision::MAX_IMAGE_BYTES, None).await
}

/// Describe an image — and answer a question about it if you ask one.
#[poise::command(slash_command)]
pub async fn see(
    ctx: Context<'_>,
    #[description = "The image"] image: Attachment,
    #[description = "Something to ask about it"] question: Option<String>,
) -> Result<(), Error> {
    ctx.defer().await?;
    let state = &ctx.data().state;
    let Some(vision_client) = state.vision() else {
        ctx.say(
            crate::commands_help::provider_recovery(
                ctx,
                crate::provider::ProviderFailureKind::Configuration,
            )
            .await,
        )
        .await?;
        return Ok(());
    };
    let bytes = match fetch_attachment(state, &image).await {
        Ok(b) => b,
        Err(_) => {
            ctx.say("Could not read that attachment. Check that it is available and within the image size limit, then try again.")
            .await?;
            return Ok(());
        }
    };
    let description = match vision_client.describe_source_only(bytes).await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(failure = ?e.provider_failure(), "vision description failed");
            ctx.say(crate::commands_help::provider_recovery(ctx, e.provider_failure()).await)
                .await?;
            return Ok(());
        }
    };
    let persona = crate::persona::Persona::Abbey;
    let reply = match (question, state.generation_label()) {
        (Some(q), Some(_)) => {
            let folded = vision::fold_descriptions(&q, &[(image.filename.clone(), description)]);
            let outcome = {
                state
                    .providers
                    .chat_source_only(&ask::system_prompt(persona), &[llm::ChatTurn::user(folded)])
                    .await
            };
            match outcome {
                Ok((a, provider_label)) => {
                    ask::render_answer(persona, provider_label, &ask::tidy_reply(persona, &a))
                }
                Err(e) => {
                    tracing::warn!(failure = ?e.provider_failure(), "vision follow-up generation failed");
                    crate::commands_help::provider_recovery(ctx, e.provider_failure())
                        .await
                        .to_string()
                }
            }
        }
        _ => vision::render_see(&persona.to_string(), &description),
    };
    ctx.say(clamp_message(reply)).await?;
    Ok(())
}

/// Transcribe the text in an image.
#[poise::command(slash_command)]
pub async fn ocr(
    ctx: Context<'_>,
    #[description = "The image"] image: Attachment,
) -> Result<(), Error> {
    ctx.defer().await?;
    let state = &ctx.data().state;
    let Some(vision_client) = state.vision_for(true) else {
        ctx.say(
            crate::commands_help::provider_recovery(
                ctx,
                crate::provider::ProviderFailureKind::Configuration,
            )
            .await,
        )
        .await?;
        return Ok(());
    };
    let reply = match fetch_attachment(state, &image).await {
        Err(_) => "Could not read that attachment. Check that it is available and within the image size limit, then try again.".to_string(),
        Ok(bytes) => match vision_client.extract_text_source_only(bytes).await {
            Ok(text) => vision::render_ocr(&text),
            Err(e) => {
                tracing::warn!(failure = ?e.provider_failure(), "vision OCR failed");
                crate::commands_help::provider_recovery(ctx, e.provider_failure()).await.to_string()
            }
        },
    };
    ctx.say(clamp_message(reply)).await?;
    Ok(())
}

//! Thin Discord adapters for private image message menus.

use serenity::all::Message;

use crate::commands::clamp_message;
use crate::image_attachment::{self, AttachmentFetcher, ResolvedAttachment, Selection};
use crate::runtime::AppState;
use crate::vision::{self, ImageUnderstanding};
use crate::{Context, Error};

const NO_SUPPORTED_IMAGE: &str =
    "That message has no supported image attachment. Attach a JPEG, PNG, WebP, or GIF file.";

struct DiscordAttachmentFetcher<'a>(&'a AppState);

impl AttachmentFetcher for DiscordAttachmentFetcher<'_> {
    async fn fetch(&self, attachment: &ResolvedAttachment) -> Result<Vec<u8>, String> {
        crate::gateway::fetch_capped(
            &self.0.attachments,
            &attachment.url,
            vision::MAX_IMAGE_BYTES,
            None,
        )
        .await
    }
}

fn resolved_attachments(message: &Message) -> Vec<ResolvedAttachment> {
    message
        .attachments
        .iter()
        .map(|attachment| ResolvedAttachment {
            filename: attachment.filename.clone(),
            url: attachment.url.clone(),
            declared_size: u64::from(attachment.size),
        })
        .collect()
}

async fn selected_image(ctx: Context<'_>, message: &Message) -> Result<Option<Vec<u8>>, Error> {
    let state = &ctx.data().state;
    let selected = image_attachment::select_first_supported(
        &resolved_attachments(message),
        &DiscordAttachmentFetcher(state),
    )
    .await;
    match selected {
        Selection::Image { bytes, .. } => Ok(Some(bytes)),
        Selection::NoSupportedImage => {
            private_reply(ctx, NO_SUPPORTED_IMAGE).await?;
            Ok(None)
        }
        Selection::FetchFailed(_) => {
            private_reply(ctx, "Could not read that attachment. Check that it is available and within the image size limit, then try again.").await?;
            Ok(None)
        }
    }
}

async fn private_reply(ctx: Context<'_>, content: &str) -> Result<(), Error> {
    ctx.send(
        poise::CreateReply::default()
            .content(clamp_message(content.to_string()))
            .ephemeral(true)
            .allowed_mentions(crate::gateway::no_mentions()),
    )
    .await?;
    Ok(())
}

#[poise::command(context_menu_command = "Abbey: describe image", ephemeral)]
pub async fn describe_image(ctx: Context<'_>, message: Message) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let state = &ctx.data().state;
    let Some(vision_client) = state.vision() else {
        private_reply(
            ctx,
            crate::commands_help::provider_recovery(
                ctx,
                crate::provider::ProviderFailureKind::Configuration,
            )
            .await,
        )
        .await?;
        return Ok(());
    };
    let Some(bytes) = selected_image(ctx, &message).await? else {
        return Ok(());
    };
    let reply = match vision_client.describe(bytes).await {
        Ok(description) => vision::render_see("Abbey", &description),
        Err(error) => {
            tracing::warn!(failure = ?error.provider_failure(), "vision context-menu description failed");
            crate::commands_help::provider_recovery(ctx, error.provider_failure())
                .await
                .to_string()
        }
    };
    private_reply(ctx, &reply).await
}

#[poise::command(context_menu_command = "Abbey: read image text", ephemeral)]
pub async fn read_image_text(ctx: Context<'_>, message: Message) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let state = &ctx.data().state;
    let Some(vision_client) = state.vision() else {
        private_reply(
            ctx,
            crate::commands_help::provider_recovery(
                ctx,
                crate::provider::ProviderFailureKind::Configuration,
            )
            .await,
        )
        .await?;
        return Ok(());
    };
    let Some(bytes) = selected_image(ctx, &message).await? else {
        return Ok(());
    };
    let reply = match vision_client.extract_text(bytes).await {
        Ok(text) => vision::render_ocr(&text),
        Err(error) => {
            tracing::warn!(failure = ?error.provider_failure(), "vision context-menu OCR failed");
            crate::commands_help::provider_recovery(ctx, error.provider_failure())
                .await
                .to_string()
        }
    };
    private_reply(ctx, &reply).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_reads_only_real_attachments_in_discord_order() {
        let raw = serde_json::json!({
            "id": "10", "channel_id": "20", "author": {
                "id": "30", "username": "u", "discriminator": "0001", "avatar": null
            }, "content": "https://arbitrary.example/image.png", "timestamp": "2026-09-06T00:00:00Z",
            "edited_timestamp": null, "tts": false, "mention_everyone": false, "mentions": [],
            "mention_roles": [], "attachments": [
                {"id":"1", "filename":"first.bin", "size":3, "url":"https://cdn.discordapp.com/first", "proxy_url":"https://media.discordapp.net/first"},
                {"id":"2", "filename":"second.png", "size":4, "url":"https://cdn.discordapp.com/second", "proxy_url":"https://media.discordapp.net/second"}
            ], "embeds": [{"type":"image", "url":"https://arbitrary.example/embed.png"}],
            "pinned": false, "type": 0
        });
        let message: Message = serde_json::from_value(raw).unwrap();
        let selected = resolved_attachments(&message);
        assert_eq!(
            selected.iter().map(|a| a.url.as_str()).collect::<Vec<_>>(),
            [
                "https://cdn.discordapp.com/first",
                "https://cdn.discordapp.com/second"
            ]
        );
        assert!(
            selected
                .iter()
                .all(|a| !a.url.contains("arbitrary.example"))
        );
    }

    #[test]
    fn menu_adapters_are_private_message_commands() {
        for command in [describe_image(), read_image_text()] {
            assert!(command.ephemeral);
            assert!(matches!(
                command.context_menu_action,
                Some(poise::ContextMenuCommandAction::Message(_))
            ));
        }
    }
}

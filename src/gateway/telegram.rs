//! Telegram adapter — `TelegramOutbound`, `run_telegram`, `SecretString`.

use std::sync::Arc;
use std::time::Duration;

use crate::gateway::shared::{PollLoop, SecretString, TELEGRAM_MESSAGE_CAP, clamp, fetch_capped};
use crate::pipeline::{self, Outbound};
use crate::platform::{self, OutboundMessage, TelegramPoller, TgFile, TgResponse, TgUpdate};
use crate::runtime::AppState;

/// Telegram Bot API delivery. Token is held as `SecretString` so `Debug`
/// never prints it.
pub struct TelegramOutbound {
    token: SecretString,
    client: reqwest::Client,
}

impl TelegramOutbound {
    pub fn new(token: &str) -> Self {
        Self {
            token: SecretString::new(token.to_string()),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(65))
                .build()
                .unwrap_or_default(),
        }
    }

    fn base(&self) -> String {
        format!("https://api.telegram.org/bot{}", self.token.expose())
    }

    fn file_base(&self) -> String {
        format!("https://api.telegram.org/file/bot{}", self.token.expose())
    }

    async fn post_json(
        &self,
        method: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let response = self
            .client
            .post(format!("{}/{method}", self.base()))
            .json(body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = response.status();
        let value: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
        if !status.is_success()
            || value.get("ok").and_then(serde_json::Value::as_bool) != Some(true)
        {
            return Err(format!("telegram {method} failed: HTTP {status}"));
        }
        Ok(value)
    }
}

impl std::fmt::Debug for TelegramOutbound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramOutbound")
            .field("token", &self.token)
            .field("client", &self.client)
            .finish()
    }
}

impl Outbound for TelegramOutbound {
    async fn send(
        &self,
        native_channel_id: &str,
        message: &OutboundMessage,
    ) -> Result<String, String> {
        let clamped = OutboundMessage {
            text: clamp(&message.text, TELEGRAM_MESSAGE_CAP),
            ..message.clone()
        };
        let payload = platform::telegram_send_payload(&clamped, native_channel_id);
        let value = self.post_json("sendMessage", &payload).await?;
        Ok(value
            .pointer("/result/message_id")
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string())
            .unwrap_or_default())
    }

    async fn typing(&self, native_channel_id: &str) {
        let _ = self
            .post_json(
                "sendChatAction",
                &serde_json::json!({ "chat_id": native_channel_id, "action": "typing" }),
            )
            .await;
    }

    async fn react(
        &self,
        native_channel_id: &str,
        native_message_id: &str,
        emoji: &str,
    ) -> Result<(), String> {
        self.post_json(
            "setMessageReaction",
            &serde_json::json!({
                "chat_id": native_channel_id,
                "message_id": native_message_id.parse::<i64>().map_err(|e| e.to_string())?,
                "reaction": [{ "type": "emoji", "emoji": emoji }],
            }),
        )
        .await
        .map(|_| ())
    }

    async fn fetch(&self, url: &str, max: usize) -> Result<Vec<u8>, String> {
        let resolved = match platform::tgfile_id(url) {
            Some(file_id) => {
                let body: TgResponse<TgFile> = self
                    .client
                    .get(platform::get_file_url(&self.base(), file_id))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?
                    .json()
                    .await
                    .map_err(|e| e.to_string())?;
                let path = body
                    .result
                    .filter(|_| body.ok)
                    .and_then(|f| f.file_path)
                    .ok_or("getFile returned no file_path")?;
                platform::resolve_file_url(&self.file_base(), path.as_str())
            }
            None => url.to_string(),
        };
        fetch_capped(&self.client, &resolved, max, None).await
    }

    async fn edit(
        &self,
        native_channel_id: &str,
        native_message_id: &str,
        text: &str,
    ) -> Result<(), String> {
        self.post_json(
            "editMessageText",
            &serde_json::json!({
                "chat_id": native_channel_id,
                "message_id": native_message_id.parse::<i64>().map_err(|e| e.to_string())?,
                "text": clamp(text, TELEGRAM_MESSAGE_CAP),
            }),
        )
        .await
        .map(|_| ())
    }
}

/// Long-poll `getUpdates` forever, feeding the pipeline. Errors back off five
/// seconds and re-poll, per the spec. Uses `PollLoop` for the backoff.
pub async fn run_telegram(state: Arc<AppState>, token: String) {
    let out = TelegramOutbound::new(&token);
    let mut poller = TelegramPoller::default();
    let backoff = PollLoop::telegram();
    if let Ok(me) = out.post_json("getMe", &serde_json::json!({})).await
        && let Some(id) = me.pointer("/result/id").and_then(serde_json::Value::as_i64)
    {
        state.register_self(format!("telegram:{id}"));
    }
    tracing::info!("telegram adapter polling");
    loop {
        let url = platform::get_updates_url(&out.base(), poller.offset);
        let updates = match out.client.get(&url).send().await {
            Ok(resp) => match resp.json::<TgResponse<Vec<TgUpdate>>>().await {
                Ok(body) if body.ok => body.result.unwrap_or_default(),
                Ok(_) => {
                    tracing::warn!("telegram getUpdates returned ok=false");
                    backoff.wait().await;
                    continue;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "telegram getUpdates decode failed");
                    backoff.wait().await;
                    continue;
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "telegram getUpdates failed");
                backoff.wait().await;
                continue;
            }
        };
        poller.advance(&updates);
        for update in &updates {
            if let Some(event) = platform::translate_telegram(update) {
                let reply_to = update
                    .message
                    .as_ref()
                    .and_then(|m| m.reply_to_message.as_ref())
                    .map(|r| r.message_id.to_string());
                pipeline::handle(&state, &out, event, false, reply_to.as_deref()).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_secret_is_redacted() {
        let out = TelegramOutbound::new("super-secret-token");
        let dbg = format!("{out:?}");
        assert!(!dbg.contains("super-secret-token"));
        assert!(dbg.contains("<redacted>"));
    }

    #[test]
    fn telegram_clamp_uses_telegram_cap() {
        let long = "a".repeat(5000);
        let clamped = clamp(&long, TELEGRAM_MESSAGE_CAP);
        assert_eq!(clamped.chars().count(), TELEGRAM_MESSAGE_CAP);
    }
}

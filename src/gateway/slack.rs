//! Slack adapter — `SlackOutbound`, `run_slack`, `PollLoop`.

use std::sync::Arc;
use std::time::Duration;

pub use crate::gateway::shared::PollLoop;
use crate::gateway::shared::{SecretString, fetch_capped};
use crate::pipeline::{self, Outbound};
use crate::platform::{self, OutboundMessage, SlackEnvelope};
use crate::runtime::AppState;

/// Slack Web API delivery. Token is held as `SecretString` so `Debug` never
/// prints it.
pub struct SlackOutbound {
    bot_token: SecretString,
    client: reqwest::Client,
}

impl SlackOutbound {
    pub fn new(bot_token: &str) -> Self {
        Self {
            bot_token: SecretString::new(bot_token.to_string()),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    async fn call(
        &self,
        method: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let response = self
            .client
            .post(format!("https://slack.com/api/{method}"))
            .bearer_auth(self.bot_token.expose())
            .json(body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let value: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
        if value.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
            let err = value
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            return Err(format!("slack {method} failed: {err}"));
        }
        Ok(value)
    }
}

impl std::fmt::Debug for SlackOutbound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlackOutbound")
            .field("bot_token", &self.bot_token)
            .field("client", &self.client)
            .finish()
    }
}

impl Outbound for SlackOutbound {
    async fn send(
        &self,
        native_channel_id: &str,
        message: &OutboundMessage,
    ) -> Result<String, String> {
        let payload = platform::slack_post_message_payload(message, native_channel_id);
        let value = self.call("chat.postMessage", &payload).await?;
        Ok(value
            .get("ts")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string())
    }

    async fn typing(&self, _native_channel_id: &str) {}

    async fn react(
        &self,
        native_channel_id: &str,
        native_message_id: &str,
        emoji: &str,
    ) -> Result<(), String> {
        let name = match emoji {
            "👍" => "+1",
            "❤️" | "❤" => "heart",
            "🔥" => "fire",
            _ => "eyes",
        };
        self.call(
            "reactions.add",
            &serde_json::json!({ "channel": native_channel_id, "timestamp": native_message_id, "name": name }),
        )
        .await
        .map(|_| ())
    }

    async fn fetch(&self, url: &str, max: usize) -> Result<Vec<u8>, String> {
        let bearer = url
            .contains("files.slack.com")
            .then_some(self.bot_token.expose());
        fetch_capped(&self.client, url, max, bearer).await
    }

    async fn edit(
        &self,
        native_channel_id: &str,
        native_message_id: &str,
        text: &str,
    ) -> Result<(), String> {
        self.call(
            "chat.update",
            &serde_json::json!({ "channel": native_channel_id, "ts": native_message_id, "text": text }),
        )
        .await
        .map(|_| ())
    }
}

/// One Socket Mode frame. Only the fields the loop reads.
#[derive(Debug, serde::Deserialize)]
struct SocketFrame {
    #[serde(rename = "type")]
    kind: String,
    envelope_id: Option<String>,
    payload: Option<SlackEnvelope>,
}

/// Run Socket Mode forever: open, pump, ack, reconnect on close.
pub async fn run_slack(state: Arc<AppState>, bot_token: String, app_token: String) {
    use futures_util::{SinkExt as _, StreamExt as _};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let out = SlackOutbound::new(&bot_token);
    if let Ok(me) = out.call("auth.test", &serde_json::json!({})).await
        && let Some(id) = me.get("user_id").and_then(serde_json::Value::as_str)
    {
        state.register_self(format!("slack:{id}"));
    }
    let opener = reqwest::Client::new();
    loop {
        let url = match opener
            .post("https://slack.com/api/apps.connections.open")
            .bearer_auth(&app_token)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
        {
            Ok(resp) => match resp.json::<serde_json::Value>().await {
                Ok(v) if v.get("ok").and_then(serde_json::Value::as_bool) == Some(true) => v
                    .get("url")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                Ok(_) => {
                    tracing::warn!("apps.connections.open refused");
                    None
                }
                Err(e) => {
                    tracing::warn!(error = %e, "apps.connections.open decode failed");
                    None
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "apps.connections.open failed");
                None
            }
        };
        let Some(url) = url else {
            PollLoop::slack_open().wait().await;
            continue;
        };
        let (mut socket, _) = match tokio_tungstenite::connect_async(&url).await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!(error = %e, "slack socket connect failed");
                PollLoop::slack_open().wait().await;
                continue;
            }
        };
        tracing::info!("slack socket mode connected");
        while let Some(frame) = socket.next().await {
            let text = match frame {
                Ok(WsMessage::Text(t)) => t,
                Ok(WsMessage::Ping(p)) => {
                    let _ = socket.send(WsMessage::Pong(p)).await;
                    continue;
                }
                Ok(WsMessage::Close(_)) | Err(_) => break,
                Ok(_) => continue,
            };
            let Ok(parsed) = serde_json::from_str::<SocketFrame>(&text) else {
                continue;
            };
            if let Some(id) = &parsed.envelope_id {
                let ack = serde_json::json!({ "envelope_id": id }).to_string();
                if socket.send(WsMessage::Text(ack.into())).await.is_err() {
                    break;
                }
            }
            match parsed.kind.as_str() {
                "disconnect" => break,
                "events_api" => {
                    if let Some(event) = parsed.payload.as_ref().and_then(platform::translate_slack)
                    {
                        pipeline::handle(&state, &out, event, false, None).await;
                    }
                }
                _ => {}
            }
        }
        tracing::info!("slack socket closed; reconnecting");
        PollLoop::slack_reconnect().wait().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_loop_durations_are_distinct() {
        assert_ne!(
            PollLoop::telegram().backoff_duration(),
            PollLoop::slack_open().backoff_duration()
        );
        assert_ne!(
            PollLoop::slack_open().backoff_duration(),
            PollLoop::slack_reconnect().backoff_duration()
        );
    }

    #[test]
    fn slack_secret_is_redacted() {
        let out = SlackOutbound::new("xoxb-secret");
        let dbg = format!("{out:?}");
        assert!(!dbg.contains("xoxb-secret"));
        assert!(dbg.contains("<redacted>"));
    }
}

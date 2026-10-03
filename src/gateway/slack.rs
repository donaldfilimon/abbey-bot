//! Slack adapter — `SlackOutbound`, `run_slack`, `PollLoop`.

use std::sync::Arc;
use std::time::Duration;

pub use crate::gateway::shared::PollLoop;
use crate::gateway::shared::{SecretString, fetch_capped};
use crate::outbound_failure::{DeliveryCertainty, OutboundFailure, OutboundFailureCategory};
use crate::pipeline::{self, Outbound};
use crate::platform::{self, OutboundMessage, SlackEnvelope};
use crate::runtime::AppState;

/// Slack Web API delivery. Token is held as `SecretString` so `Debug` never
/// prints it.
pub struct SlackOutbound {
    bot_token: SecretString,
    client: reqwest::Client,
}

fn reqwest_delivery_failure(error: reqwest::Error) -> OutboundFailure {
    if error.is_builder() {
        OutboundFailure::new(
            OutboundFailureCategory::Internal,
            DeliveryCertainty::NotSent,
            None,
        )
    } else if error.is_decode() {
        OutboundFailure::new(
            OutboundFailureCategory::Internal,
            DeliveryCertainty::PossiblySent,
            None,
        )
    } else {
        OutboundFailure::new(
            OutboundFailureCategory::Transport,
            DeliveryCertainty::PossiblySent,
            None,
        )
    }
}

fn slack_response_failure(
    status: u16,
    value: &serde_json::Value,
    retry: Option<u64>,
) -> OutboundFailure {
    use DeliveryCertainty::{NotSent, PossiblySent};
    use OutboundFailureCategory::{Capacity, Internal, Permission, RateLimited, Transport};
    if (400..=599).contains(&status) {
        return OutboundFailure::http(Some(status), retry);
    }
    let (category, certainty) =
        if value.get("ok").and_then(serde_json::Value::as_bool) == Some(false) {
            match value.get("error").and_then(serde_json::Value::as_str) {
                Some(
                    "missing_scope" | "not_authed" | "invalid_auth" | "token_revoked"
                    | "account_inactive" | "no_permission" | "restricted_action" | "not_in_channel"
                    | "channel_not_found" | "is_archived",
                ) => (Permission, NotSent),
                Some("ratelimited" | "rate_limited") => (RateLimited, NotSent),
                Some("msg_too_long") => (Capacity, NotSent),
                Some(
                    "internal_error" | "fatal_error" | "service_unavailable" | "request_timeout",
                ) => (Transport, PossiblySent),
                _ => (Internal, NotSent),
            }
        } else {
            (Internal, PossiblySent)
        };
    OutboundFailure::new(category, certainty, retry)
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
    ) -> Result<serde_json::Value, OutboundFailure> {
        let response = self
            .client
            .post(format!("https://slack.com/api/{method}"))
            .bearer_auth(self.bot_token.expose())
            .json(body)
            .send()
            .await
            .map_err(reqwest_delivery_failure)?;
        let status = response.status();
        let retry = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        let value: serde_json::Value = response.json().await.map_err(|error| {
            if !status.is_success() {
                OutboundFailure::http(Some(status.as_u16()), retry)
            } else {
                reqwest_delivery_failure(error)
            }
        })?;
        if !status.is_success()
            || value.get("ok").and_then(serde_json::Value::as_bool) != Some(true)
        {
            return Err(slack_response_failure(status.as_u16(), &value, retry));
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
    ) -> Result<String, OutboundFailure> {
        let payload = platform::slack_post_message_payload(message, native_channel_id);
        let value = self.call("chat.postMessage", &payload).await?;
        value
            .get("ts")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                OutboundFailure::new(
                    OutboundFailureCategory::Internal,
                    DeliveryCertainty::PossiblySent,
                    None,
                )
            })
    }

    async fn typing(&self, _native_channel_id: &str) {}

    async fn react(
        &self,
        native_channel_id: &str,
        native_message_id: &str,
        emoji: &str,
    ) -> Result<(), OutboundFailure> {
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
    ) -> Result<(), OutboundFailure> {
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

    slack_observed(&state, crate::readiness::ConnectorState::Starting, None);
    let out = SlackOutbound::new(&bot_token);
    if let Ok(me) = out.call("auth.test", &serde_json::json!({})).await
        && let Some(id) = me.get("user_id").and_then(serde_json::Value::as_str)
    {
        state.register_self(format!("slack:{id}"));
    }
    let opener = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("static Slack request client");
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
            slack_observed(
                &state,
                crate::readiness::ConnectorState::Degraded,
                Some(crate::observability::OperationalErrorCategory::Unavailable),
            );
            PollLoop::slack_open().wait().await;
            continue;
        };
        let (mut socket, _) = match tokio::time::timeout(
            Duration::from_secs(30),
            tokio_tungstenite::connect_async(&url),
        )
        .await
        {
            Ok(Ok(pair)) => pair,
            _ => {
                slack_observed(
                    &state,
                    crate::readiness::ConnectorState::Degraded,
                    Some(crate::observability::OperationalErrorCategory::Unavailable),
                );
                tracing::warn!("slack socket connect failed");
                PollLoop::slack_open().wait().await;
                continue;
            }
        };
        slack_observed(&state, crate::readiness::ConnectorState::Connected, None);
        tracing::info!("slack socket mode connected");
        while let Ok(Some(frame)) =
            tokio::time::timeout(Duration::from_secs(90), socket.next()).await
        {
            let text = match frame {
                Ok(WsMessage::Text(t)) => t,
                Ok(WsMessage::Ping(p)) => {
                    if !tokio::time::timeout(
                        Duration::from_secs(30),
                        socket.send(WsMessage::Pong(p)),
                    )
                    .await
                    .is_ok_and(|r| r.is_ok())
                    {
                        break;
                    }
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
                if !tokio::time::timeout(
                    Duration::from_secs(30),
                    socket.send(WsMessage::Text(ack.into())),
                )
                .await
                .is_ok_and(|r| r.is_ok())
                {
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
        slack_observed(
            &state,
            crate::readiness::ConnectorState::Degraded,
            Some(crate::observability::OperationalErrorCategory::Unavailable),
        );
        tracing::info!("slack socket closed; reconnecting");
        PollLoop::slack_reconnect().wait().await;
    }
}

fn slack_observed(
    state: &AppState,
    connector: crate::readiness::ConnectorState,
    error: Option<crate::observability::OperationalErrorCategory>,
) {
    if let Some(status) = state.managed_status() {
        status.slack(connector);
        let _ = status.refresh();
    }
    if let Some(events) = state.operational_events() {
        use crate::observability::{EventCode, EventComponent, EventOutcome};
        let outcome = match connector {
            crate::readiness::ConnectorState::Connected => EventOutcome::Ready,
            crate::readiness::ConnectorState::Degraded => EventOutcome::Degraded,
            crate::readiness::ConnectorState::Starting => EventOutcome::Started,
            crate::readiness::ConnectorState::Stopped => EventOutcome::Stopped,
            crate::readiness::ConnectorState::Disabled => EventOutcome::Skipped,
        };
        let _ = events.record(
            EventComponent::Slack,
            EventCode::ConnectorState,
            outcome,
            error,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slack_api_refusals_are_not_transport_outages() {
        use crate::outbound_failure::{DeliveryCertainty, OutboundFailureCategory};
        for (value, category, certainty) in [
            (
                serde_json::json!({"ok": false, "error": "missing_scope"}),
                OutboundFailureCategory::Permission,
                DeliveryCertainty::NotSent,
            ),
            (
                serde_json::json!({"ok": false, "error": "ratelimited"}),
                OutboundFailureCategory::RateLimited,
                DeliveryCertainty::NotSent,
            ),
            (
                serde_json::json!({"ok": false, "error": "internal_error"}),
                OutboundFailureCategory::Transport,
                DeliveryCertainty::PossiblySent,
            ),
            (
                serde_json::json!({"ok": false, "error": "private-token-url"}),
                OutboundFailureCategory::Internal,
                DeliveryCertainty::NotSent,
            ),
            (
                serde_json::json!({}),
                OutboundFailureCategory::Internal,
                DeliveryCertainty::PossiblySent,
            ),
        ] {
            let failure = slack_response_failure(200, &value, Some(900));
            assert_eq!(failure.category(), category);
            assert_eq!(failure.certainty(), certainty);
            assert_eq!(failure.retry_after_secs(), Some(300));
            assert!(!format!("{failure:?} {failure}").contains("private-token-url"));
        }
    }

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

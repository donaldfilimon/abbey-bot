//! Telegram adapter — `TelegramOutbound`, `run_telegram`, `SecretString`.

use std::sync::Arc;
use std::time::Duration;

use crate::gateway::shared::{PollLoop, SecretString, TELEGRAM_MESSAGE_CAP, clamp, fetch_capped};
use crate::outbound_failure::{DeliveryCertainty, OutboundFailure, OutboundFailureCategory};
use crate::pipeline::{self, Outbound};
use crate::platform::{self, OutboundMessage, TelegramPoller, TgFile, TgResponse, TgUpdate};
use crate::runtime::AppState;

/// Render a reqwest failure without its URL. Telegram authenticates by putting
/// the bot token in the URL path, so reqwest's default error display is not
/// safe for user-facing errors or logs.
fn render_reqwest_error(error: reqwest::Error) -> String {
    error.without_url().to_string()
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

fn telegram_response_failure(status: u16, value: &serde_json::Value) -> OutboundFailure {
    let retry = value
        .pointer("/parameters/retry_after")
        .and_then(serde_json::Value::as_u64);
    if (400..=599).contains(&status) {
        return OutboundFailure::http(Some(status), retry);
    }
    if value.get("ok").and_then(serde_json::Value::as_bool) == Some(false) {
        if let Some(code) = value
            .get("error_code")
            .and_then(serde_json::Value::as_u64)
            .and_then(|code| u16::try_from(code).ok())
            .filter(|code| (400..=599).contains(code))
        {
            return OutboundFailure::http(Some(code), retry);
        }
        return OutboundFailure::new(
            OutboundFailureCategory::Internal,
            DeliveryCertainty::NotSent,
            retry,
        );
    }
    OutboundFailure::new(
        OutboundFailureCategory::Internal,
        DeliveryCertainty::PossiblySent,
        None,
    )
}

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
    ) -> Result<serde_json::Value, OutboundFailure> {
        let response = self
            .client
            .post(format!("{}/{method}", self.base()))
            .json(body)
            .send()
            .await
            .map_err(reqwest_delivery_failure)?;
        let status = response.status();
        let value: serde_json::Value = response.json().await.map_err(|error| {
            if !status.is_success() {
                OutboundFailure::http(Some(status.as_u16()), None)
            } else {
                reqwest_delivery_failure(error)
            }
        })?;
        if !status.is_success()
            || value.get("ok").and_then(serde_json::Value::as_bool) != Some(true)
        {
            return Err(telegram_response_failure(status.as_u16(), &value));
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
    ) -> Result<String, OutboundFailure> {
        let clamped = OutboundMessage {
            text: clamp(&message.text, TELEGRAM_MESSAGE_CAP),
            ..message.clone()
        };
        let payload = platform::telegram_send_payload(&clamped, native_channel_id);
        let value = self.post_json("sendMessage", &payload).await?;
        value
            .pointer("/result/message_id")
            .and_then(serde_json::Value::as_i64)
            .filter(|id| *id > 0)
            .map(|id| id.to_string())
            .ok_or_else(|| {
                OutboundFailure::new(
                    OutboundFailureCategory::Internal,
                    DeliveryCertainty::PossiblySent,
                    None,
                )
            })
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
    ) -> Result<(), OutboundFailure> {
        self.post_json(
            "setMessageReaction",
            &serde_json::json!({
                "chat_id": native_channel_id,
                "message_id": native_message_id.parse::<i64>().map_err(|_| OutboundFailure::new(OutboundFailureCategory::Internal, DeliveryCertainty::NotSent, None))?,
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
                    .map_err(render_reqwest_error)?
                    .json()
                    .await
                    .map_err(render_reqwest_error)?;
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
    ) -> Result<(), OutboundFailure> {
        self.post_json(
            "editMessageText",
            &serde_json::json!({
                "chat_id": native_channel_id,
                "message_id": native_message_id.parse::<i64>().map_err(|_| OutboundFailure::new(OutboundFailureCategory::Internal, DeliveryCertainty::NotSent, None))?,
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
    telegram_observed(&state, crate::readiness::ConnectorState::Starting, None);
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
                    telegram_observed(
                        &state,
                        crate::readiness::ConnectorState::Degraded,
                        Some(crate::observability::OperationalErrorCategory::Protocol),
                    );
                    tracing::warn!("telegram getUpdates returned ok=false");
                    backoff.wait().await;
                    continue;
                }
                Err(e) => {
                    telegram_observed(
                        &state,
                        crate::readiness::ConnectorState::Degraded,
                        Some(crate::observability::OperationalErrorCategory::Protocol),
                    );
                    tracing::warn!(
                        error = %render_reqwest_error(e),
                        "telegram getUpdates decode failed"
                    );
                    backoff.wait().await;
                    continue;
                }
            },
            Err(e) => {
                telegram_observed(
                    &state,
                    crate::readiness::ConnectorState::Degraded,
                    Some(crate::observability::OperationalErrorCategory::Unavailable),
                );
                tracing::warn!(error = %render_reqwest_error(e), "telegram getUpdates failed");
                backoff.wait().await;
                continue;
            }
        };
        telegram_observed(&state, crate::readiness::ConnectorState::Connected, None);
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

fn telegram_observed(
    state: &AppState,
    connector: crate::readiness::ConnectorState,
    error: Option<crate::observability::OperationalErrorCategory>,
) {
    if let Some(status) = state.managed_status() {
        status.telegram(connector);
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
            EventComponent::Telegram,
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
    fn telegram_refusals_and_lost_ack_are_closed_and_redacted() {
        use crate::outbound_failure::{DeliveryCertainty, OutboundFailureCategory};
        let value = serde_json::json!({
            "ok": false,
            "error_code": 429,
            "description": "private-url-and-bot-token",
            "parameters": {"retry_after": u64::MAX},
        });
        let failure = telegram_response_failure(200, &value);
        assert_eq!(failure.category(), OutboundFailureCategory::RateLimited);
        assert_eq!(failure.certainty(), DeliveryCertainty::NotSent);
        assert_eq!(failure.retry_after_secs(), Some(300));
        assert!(!format!("{failure:?} {failure}").contains("private-url-and-bot-token"));
        assert_eq!(
            telegram_response_failure(403, &value).category(),
            OutboundFailureCategory::Permission
        );
        let malformed = telegram_response_failure(200, &serde_json::json!({"result": {}}));
        assert_eq!(malformed.category(), OutboundFailureCategory::Internal);
        assert_eq!(malformed.certainty(), DeliveryCertainty::PossiblySent);
    }

    #[test]
    fn telegram_delivery_builder_errors_never_render_token_urls() {
        let secret = "synthetic-secret-token";
        let url = format!("https://api.telegram.org/bot{secret}/sendMessage");
        let error = reqwest::Client::new()
            .post(&url)
            .header("x-invalid", "invalid\nheader")
            .build()
            .unwrap_err()
            .with_url(url.parse().unwrap());
        let failure = reqwest_delivery_failure(error);
        assert_eq!(
            failure.certainty(),
            crate::outbound_failure::DeliveryCertainty::NotSent
        );
        assert!(!format!("{failure:?} {failure}").contains(secret));
        assert!(!format!("{failure:?} {failure}").contains("api.telegram.org"));
    }

    #[test]
    fn telegram_secret_is_redacted() {
        let out = TelegramOutbound::new("super-secret-token");
        let dbg = format!("{out:?}");
        assert!(!dbg.contains("super-secret-token"));
        assert!(dbg.contains("<redacted>"));
    }

    #[test]
    fn telegram_reqwest_errors_strip_token_bearing_urls() {
        let token = "synthetic-secret-token";
        let url = format!("https://api.telegram.org/bot{token}/getUpdates");
        let error = reqwest::Client::new()
            .get(&url)
            .header("x-invalid", "invalid\nheader")
            .build()
            .expect_err("invalid header value should fail request construction")
            .with_url(url.parse().expect("synthetic Telegram URL should parse"));

        assert!(
            error.url().is_some_and(|url| url.as_str().contains(token)),
            "test setup must produce a reqwest error carrying the token-bearing URL"
        );
        let rendered = render_reqwest_error(error);
        assert!(!rendered.contains(token));
        assert!(!rendered.contains("api.telegram.org"));
    }

    #[test]
    fn telegram_clamp_uses_telegram_cap() {
        let long = "a".repeat(5000);
        let clamped = clamp(&long, TELEGRAM_MESSAGE_CAP);
        assert_eq!(clamped.chars().count(), TELEGRAM_MESSAGE_CAP);
    }
}

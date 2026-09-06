//! Gateway trinity — exports, validated connector configuration, and wiring.

pub mod discord;
pub mod shared;
pub mod slack;
pub mod telegram;

#[allow(unused_imports)]
pub use discord::{DiscordOutbound, on_discord_event};
#[allow(unused_imports)]
pub(crate) use discord::{no_mentions, strip_bot_mention};
#[allow(unused_imports)]
pub use shared::{
    DISCORD_MESSAGE_CAP, Snowflake, TELEGRAM_MESSAGE_CAP, clamp, clamp_message, fetch_capped,
    parse_id,
};
#[allow(unused_imports)]
pub use shared::{PollLoop, SecretString};
#[allow(unused_imports)]
pub use slack::{SlackOutbound, run_slack};
#[allow(unused_imports)]
pub use telegram::{TelegramOutbound, run_telegram};

use std::sync::Arc;

use crate::runtime::AppState;
use crate::service::{AdmissionError, ServiceSupervisor, TaskExit, TaskName};

/// Parsed before any service spawn. Debug only exposes configuration shape.
#[derive(Debug)]
pub struct ConnectorConfig {
    telegram: Option<SecretString>,
    slack: Option<SlackConfig>,
}
#[derive(Debug)]
struct SlackConfig {
    bot: SecretString,
    app: SecretString,
}
impl ConnectorConfig {
    pub fn from_env() -> Result<Self, &'static str> {
        Self::from_get(|name| std::env::var(name))
    }
    fn from_get(
        mut read: impl FnMut(&str) -> Result<String, std::env::VarError>,
    ) -> Result<Self, &'static str> {
        fn optional(
            value: Result<String, std::env::VarError>,
        ) -> Result<Option<SecretString>, &'static str> {
            match value {
                Ok(value) => {
                    let value = value.trim();
                    Ok((!value.is_empty()).then(|| SecretString::new(value)))
                }
                Err(std::env::VarError::NotPresent) => Ok(None),
                Err(std::env::VarError::NotUnicode(_)) => {
                    Err("connector configuration is not valid Unicode")
                }
            }
        }
        let telegram = optional(read("TELEGRAM_BOT_TOKEN"))?;
        let bot = optional(read("SLACK_BOT_TOKEN"))?;
        let app = optional(read("SLACK_APP_TOKEN"))?;
        let slack = match (bot, app) {
            (None, None) => None,
            (Some(bot), Some(app)) => Some(SlackConfig { bot, app }),
            _ => return Err("Slack requires both bot and app credentials"),
        };
        Ok(Self { telegram, slack })
    }
    pub fn telegram_enabled(&self) -> bool {
        self.telegram.is_some()
    }
    pub fn slack_enabled(&self) -> bool {
        self.slack.is_some()
    }
}

/// Configuration has been validated as one unit before any named task starts.
/// Cancellation covers network requests, response bodies and reconnect waits.
pub fn start_connectors(
    state: &Arc<AppState>,
    supervisor: &mut ServiceSupervisor,
    config: ConnectorConfig,
) -> Result<(), AdmissionError> {
    if let Some(token) = config.telegram {
        let state = state.clone();
        supervisor.spawn_service(TaskName::Telegram, move |cancel| async move {
            connector_started(&state, crate::observability::EventComponent::Telegram);
            tokio::select! {
                biased;
                () = cancel.cancelled() => TaskExit::Cancelled,
                _ = telegram::run_telegram(state, token.expose().to_owned()) => TaskExit::Returned,
            }
        })?;
    }
    if let Some(config) = config.slack {
        let state = state.clone();
        supervisor.spawn_service(TaskName::Slack, move |cancel| async move {
            connector_started(&state, crate::observability::EventComponent::Slack);
            tokio::select! {
                biased;
                () = cancel.cancelled() => TaskExit::Cancelled,
                _ = slack::run_slack(state, config.bot.expose().to_owned(), config.app.expose().to_owned()) => TaskExit::Returned,
            }
        })?;
    }
    Ok(())
}

fn connector_started(state: &AppState, component: crate::observability::EventComponent) {
    if let Some(events) = state.operational_events() {
        let _ = events.record(
            component,
            crate::observability::EventCode::TaskStarted,
            crate::observability::EventOutcome::Started,
            None,
        );
    }
}

#[cfg(test)]
mod connector_configuration_tests {
    use super::*;
    fn parse(values: &[(&str, &str)]) -> Result<ConnectorConfig, &'static str> {
        ConnectorConfig::from_get(|name| {
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
                .ok_or(std::env::VarError::NotPresent)
        })
    }
    #[test]
    fn absent_or_all_blank_is_disabled_and_complete_values_are_redacted() {
        let empty = parse(&[]).unwrap();
        assert!(!empty.telegram_enabled() && !empty.slack_enabled());
        let blank = parse(&[
            ("TELEGRAM_BOT_TOKEN", " "),
            ("SLACK_BOT_TOKEN", ""),
            ("SLACK_APP_TOKEN", "\t"),
        ])
        .unwrap();
        assert!(!blank.telegram_enabled() && !blank.slack_enabled());
        let full = parse(&[
            ("TELEGRAM_BOT_TOKEN", "PRIVATE_TELEGRAM"),
            ("SLACK_BOT_TOKEN", "PRIVATE_BOT"),
            ("SLACK_APP_TOKEN", "PRIVATE_APP"),
        ])
        .unwrap();
        assert!(full.telegram_enabled() && full.slack_enabled());
        assert!(!format!("{full:?}").contains("PRIVATE"));
    }
    #[test]
    fn either_partial_slack_direction_including_blank_is_rejected() {
        for values in [
            vec![("SLACK_BOT_TOKEN", "PRIVATE")],
            vec![("SLACK_APP_TOKEN", "PRIVATE")],
            vec![("SLACK_BOT_TOKEN", "PRIVATE"), ("SLACK_APP_TOKEN", " ")],
            vec![("SLACK_BOT_TOKEN", ""), ("SLACK_APP_TOKEN", "PRIVATE")],
        ] {
            let error = parse(&values).unwrap_err();
            assert!(!error.contains("PRIVATE"));
        }
    }
    #[test]
    fn non_unicode_is_fixed_error_instead_of_disabled() {
        let error = ConnectorConfig::from_get(|_| {
            Err(std::env::VarError::NotUnicode("PRIVATE_NONUNICODE".into()))
        })
        .unwrap_err();
        assert_eq!(error, "connector configuration is not valid Unicode");
    }
}

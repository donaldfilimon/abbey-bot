//! Gateway trinity — re-export and wiring (≤80 lines).

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

use crate::persist::Stores;
use crate::runtime::AppState;

/// Spawn the Telegram adapter if `TELEGRAM_BOT_TOKEN` is set.
pub fn maybe_start_telegram(state: &Arc<AppState>) {
    let Some(token) = std::env::var("TELEGRAM_BOT_TOKEN")
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
    else {
        return;
    };
    let state = Arc::clone(state);
    tokio::spawn(telegram::run_telegram(state, token));
}

/// Spawn the Slack adapter if both tokens are set.
pub fn maybe_start_slack(state: &Arc<AppState>) {
    let read = |name: &str| {
        std::env::var(name)
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
    };
    let (Some(bot), Some(app)) = (read("SLACK_BOT_TOKEN"), read("SLACK_APP_TOKEN")) else {
        return;
    };
    let state = Arc::clone(state);
    tokio::spawn(slack::run_slack(state, bot, app));
}

/// Persist on the way out. Called from the ctrl-c handler in `main`.
pub fn shutdown(state: &AppState) {
    state.persist_all();
    if let Some(dir) = &state.data_dir {
        tracing::info!(path = %Stores::state_path(dir).display(), "state persisted on shutdown");
    }
}

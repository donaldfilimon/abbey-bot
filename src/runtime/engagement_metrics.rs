//! Transient guild aggregates; no member, source, receipt or message identity.
use super::AppState;
use crate::{engagement::EngagementScope, observability::EventCode};
#[derive(Default)]
pub(super) struct Metrics {
    attempts: u64,
    sent: u64,
    failed: u64,
    queue_ms: u64,
    post_ms: Option<u64>,
    completion_ms: Option<u64>,
}
impl AppState {
    pub(super) fn engagement_metric(
        &self,
        scope: &EngagementScope,
        code: EventCode,
        elapsed: std::time::Duration,
    ) {
        let EngagementScope::Guild { guild, .. } = scope else {
            return;
        };
        let mut metrics = Self::lock(&self.engagement_metrics);
        if !metrics.contains_key(guild) && metrics.len() >= 10_000 {
            return;
        }
        let row = metrics.entry(*guild).or_default();
        let ms = elapsed.as_millis().min(u64::MAX as u128) as u64;
        match code {
            EventCode::EngagementQueue => {
                row.attempts = row.attempts.saturating_add(1);
                row.queue_ms = ms;
            }
            EventCode::DiscordFirstPost => {
                row.sent = row.sent.saturating_add(1);
                row.post_ms = Some(ms);
            }
            EventCode::EngagementCompleted => row.completion_ms = Some(ms),
            EventCode::EngagementFailure => {
                row.failed = row.failed.saturating_add(1);
                row.completion_ms = Some(ms);
            }
            _ => {}
        }
    }
    pub(crate) fn engagement_timing_status(&self, guild: u64) -> String {
        let metrics = Self::lock(&self.engagement_metrics);
        let Some(row) = metrics.get(&guild) else {
            return "No engagement timing measured in this process for this server.".into();
        };
        let measured = |value: Option<u64>| {
            value.map_or_else(|| "not measured".to_owned(), |ms| format!("{ms} ms"))
        };
        format!(
            "This process: {} admitted attempts; {} confirmed posts; {} failed attempts. Latest queue delay: {} ms; latest confirmed post: {} from admission; latest completion: {} from admission. Generation-first-text is measured separately by the canonical provider pipeline; deterministic invitations and introductions omit generation.",
            row.attempts,
            row.sent,
            row.failed,
            row.queue_ms,
            measured(row.post_ms),
            measured(row.completion_ms)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aggregates_are_guild_local_and_dm_free() {
        let state = AppState::in_memory();
        for scope in [
            EngagementScope::Guild {
                guild: 1,
                channel: 2,
            },
            EngagementScope::Dm {
                member: 3,
                channel: 4,
            },
        ] {
            state.engagement_metric(
                &scope,
                EventCode::EngagementQueue,
                std::time::Duration::from_millis(17),
            );
        }
        state.engagement_metric(
            &EngagementScope::Guild {
                guild: 1,
                channel: 2,
            },
            EventCode::EngagementFailure,
            std::time::Duration::from_millis(23),
        );
        let text = state.engagement_timing_status(1);
        assert!(text.contains("1 admitted attempts; 0 confirmed posts; 1 failed attempts"));
        assert!(text.contains("17 ms"));
        assert!(!text.contains("member"));
        assert!(!text.contains("channel"));
        assert!(
            state
                .engagement_timing_status(9)
                .starts_with("No engagement timing measured")
        );
        assert_eq!(AppState::lock(&state.engagement_metrics).len(), 1);
    }
}

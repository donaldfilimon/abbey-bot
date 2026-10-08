//! What the per-guild policy has been doing — the observability the
//! adaptive loop needs to be debuggable rather than a black box
//! (`docs/spec/companionapp.md`: "/admin brain exposes DQN state").
//!
//! In-memory only: these describe the running process. Pure; rendering is a
//! string the command layer clamps.

use std::collections::VecDeque;

use crate::brain::reward::FeedbackAttribution;
use crate::brain::state::BotAction;

/// How many settled rewards the rolling mean covers.
pub const RECENT_REWARDS: usize = 20;

/// Process-local aggregate observations only. No member, message or text inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LearningAudit {
    pub exact: u64,
    pub unique: u64,
    pub duplicate: u64,
    pub ambiguous: u64,
    pub expired: u64,
    pub unsupported: u64,
}
impl LearningAudit {
    pub fn record(&mut self, attribution: FeedbackAttribution) {
        let count = match attribution {
            FeedbackAttribution::ExactReply => &mut self.exact,
            FeedbackAttribution::UniqueScoped => &mut self.unique,
            FeedbackAttribution::Duplicate => &mut self.duplicate,
            FeedbackAttribution::Ambiguous => &mut self.ambiguous,
            FeedbackAttribution::Expired => &mut self.expired,
            FeedbackAttribution::Unsupported => &mut self.unsupported,
        };
        *count = count.saturating_add(1);
    }
    pub fn snapshot(&self) -> Self {
        *self
    }
    pub fn render(&self) -> String {
        format!(
            "Feedback since process start: accepted exact reply/reaction {} · unique scoped {}\nRefused: duplicate {} · ambiguous {} · expired {} · unsupported {}",
            self.exact, self.unique, self.duplicate, self.ambiguous, self.expired, self.unsupported
        )
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BrainStats {
    pub last_state: Vec<f32>,
    pub last_q: Vec<f32>,
    pub last_action: Option<BotAction>,
    /// Indexed by `BotAction::index()`: stay / reply / react.
    pub action_counts: [u64; 3],
    /// Mentions and DMs answered — never policy decisions.
    pub forced_replies: u64,
    pub recent_rewards: VecDeque<f32>,
    pub settled_total: u64,
}

/// The numbers `/admin brain` shows that live outside `BrainStats`.
pub struct BrainView<'a> {
    pub epsilon: f32,
    pub learn_steps: u64,
    pub buffer_len: usize,
    pub buffer_capacity: usize,
    pub experiences: u64,
    pub budget_per_hour: u32,
    pub tokens_left: f32,
    pub topology: &'a [usize],
}

impl BrainStats {
    /// The policy chose `action` for `state` with Q-values `q`.
    pub fn record_decision(&mut self, state: &[f32], q: &[f32], action: BotAction) {
        self.last_state = state.to_vec();
        self.last_q = q.to_vec();
        self.last_action = Some(action);
        self.action_counts[action.index()] += 1;
    }

    /// A mention or DM was answered — not a policy decision.
    pub fn record_forced(&mut self) {
        self.forced_replies += 1;
    }

    /// A reward settled into this guild's replay buffer.
    pub fn record_reward(&mut self, reward: f32) {
        self.recent_rewards.push_back(reward);
        while self.recent_rewards.len() > RECENT_REWARDS {
            self.recent_rewards.pop_front();
        }
        self.settled_total += 1;
    }

    /// Mean of the rolling window, `None` before the first settlement.
    pub fn mean_recent_reward(&self) -> Option<f32> {
        if self.recent_rewards.is_empty() {
            return None;
        }
        Some(self.recent_rewards.iter().sum::<f32>() / self.recent_rewards.len() as f32)
    }

    /// The `/admin brain` body. Read it before shipping a change — a passing
    /// snapshot test is not evidence that output reads well.
    pub fn render(&self, view: &BrainView<'_>) -> String {
        let last = match self.last_action {
            None => "last decision: none yet".to_string(),
            Some(action) => {
                let q = |a: BotAction| self.last_q.get(a.index()).copied().unwrap_or(0.0);
                format!(
                    "last decision: {} · action values: stay {:.2} / reply {:.2} / react {:.2}",
                    action_name(action),
                    q(BotAction::Stay),
                    q(BotAction::Reply),
                    q(BotAction::React)
                )
            }
        };
        let mean = self
            .mean_recent_reward()
            .map_or_else(|| "n/a".to_string(), |m| format!("{m:.2}"));
        format!(
            "**brain**\n\
             ε {:.3} · learn steps {} · replay buffer {}/{} · experiences {} · topology {:?}\n\
             {last}\n\
             decisions: stay {} · reply {} · react {} · forced (mentions/DMs) {}\n\
             rewards settled: {} · mean of last {}: {mean}\n\
             budget: {:.1} of {}/h left",
            view.epsilon,
            view.learn_steps,
            view.buffer_len,
            view.buffer_capacity,
            view.experiences,
            view.topology,
            self.action_counts[BotAction::Stay.index()],
            self.action_counts[BotAction::Reply.index()],
            self.action_counts[BotAction::React.index()],
            self.forced_replies,
            self.settled_total,
            self.recent_rewards.len(),
            view.tokens_left,
            view.budget_per_hour,
        )
    }
}

/// Lower-case names for copy; `Debug` would give `Reply`.
pub const fn action_name(action: BotAction) -> &'static str {
    match action {
        BotAction::Stay => "stay",
        BotAction::Reply => "reply",
        BotAction::React => "react",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(_stats_guild: &str) -> BrainView<'_> {
        BrainView {
            epsilon: 0.1,
            learn_steps: 3,
            buffer_len: 2,
            buffer_capacity: 10_000,
            experiences: 2,
            budget_per_hour: 6,
            tokens_left: 5.0,
            topology: &[18, 64, 32, 3],
        }
    }

    #[test]
    fn audit_closed_attributions_saturate_without_identity_or_text() {
        let mut audit = LearningAudit::default();
        for attribution in [
            FeedbackAttribution::ExactReply,
            FeedbackAttribution::UniqueScoped,
            FeedbackAttribution::Duplicate,
            FeedbackAttribution::Ambiguous,
            FeedbackAttribution::Expired,
            FeedbackAttribution::Unsupported,
        ] {
            audit.record(attribution);
        }
        assert_eq!(
            audit.snapshot(),
            LearningAudit {
                exact: 1,
                unique: 1,
                duplicate: 1,
                ambiguous: 1,
                expired: 1,
                unsupported: 1
            }
        );
        audit.exact = u64::MAX;
        audit.record(FeedbackAttribution::ExactReply);
        assert_eq!(audit.snapshot().exact, u64::MAX);
        assert!(audit.render().contains("unique scoped 1"));
    }

    #[test]
    fn decisions_update_last_and_histogram() {
        let mut s = BrainStats::default();
        s.record_decision(&[0.0; 18], &[0.1, 0.7, 0.2], BotAction::Reply);
        s.record_decision(&[1.0; 18], &[0.9, 0.0, 0.0], BotAction::Stay);
        s.record_forced();
        assert_eq!(s.action_counts, [1, 1, 0]);
        assert_eq!(s.forced_replies, 1);
        assert_eq!(s.last_action, Some(BotAction::Stay));
        assert_eq!(s.last_q, vec![0.9, 0.0, 0.0]);
        assert_eq!(s.last_state.len(), 18);
    }

    #[test]
    fn rewards_keep_a_bounded_window_and_a_mean() {
        let mut s = BrainStats::default();
        assert_eq!(s.mean_recent_reward(), None);
        for i in 0..(RECENT_REWARDS + 5) {
            s.record_reward(i as f32);
        }
        assert_eq!(s.recent_rewards.len(), RECENT_REWARDS);
        assert_eq!(s.settled_total, (RECENT_REWARDS + 5) as u64);
        // The window holds 5..=24 → mean 14.5.
        assert!((s.mean_recent_reward().unwrap() - 14.5).abs() < 1e-6);
    }

    #[test]
    fn render_reads_well_and_names_the_winning_action() {
        let mut s = BrainStats::default();
        s.record_decision(&[0.0; 18], &[0.1, 0.7, 0.2], BotAction::Reply);
        s.record_reward(0.8);
        let text = s.render(&view("discord:g"));
        assert_eq!(
            text,
            "**brain**\n\
             ε 0.100 · learn steps 3 · replay buffer 2/10000 · experiences 2 · topology [18, 64, 32, 3]\n\
             last decision: reply · action values: stay 0.10 / reply 0.70 / react 0.20\n\
             decisions: stay 0 · reply 1 · react 0 · forced (mentions/DMs) 0\n\
             rewards settled: 1 · mean of last 1: 0.80\n\
             budget: 5.0 of 6/h left"
        );
    }

    #[test]
    fn render_before_any_decision_says_so() {
        let s = BrainStats::default();
        let text = s.render(&view("discord:g"));
        assert!(text.contains("last decision: none yet"), "{text}");
        let mut decided = BrainStats::default();
        decided.record_decision(&[], &[0.1, 0.2, 0.3], BotAction::Reply);
        assert!(decided.render(&view("discord:g")).contains("action values"));
        assert!(
            text.contains("rewards settled: 0 · mean of last 0: n/a"),
            "{text}"
        );
    }
}

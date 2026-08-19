//! Delayed reward collection (`docs/spec/adaptivelearning.md`, "Reward signal").
//!
//! Abbey acts now; the guild reacts over the next couple of minutes. Each reply
//! is held open for a settlement window while reactions, human replies, and
//! deletions accumulate evidence, then closes into a single-step experience.
//!
//! Pure: the clock is injected (`now` in unix seconds) and nothing is written
//! anywhere — [`RewardCollector::settle_expired`] hands the settled experiences
//! back for the caller to route to the per-guild brain.

use std::collections::HashMap;

use crate::brain::replay::Experience;
use crate::brain::state::BotAction;

/// How long a reply stays open for evidence, in seconds (2.5 min).
pub const SETTLEMENT_WINDOW_SECS: u64 = 150;

/// Reward a reply starts at: mildly negative, so engagement has to earn it back.
const REPLY_BASELINE: f32 = -0.2;
/// Positive reactions beyond this many earn nothing more.
const MAX_POSITIVE_REACTIONS: u8 = 3;
/// Settled rewards are clamped to this magnitude.
const REWARD_CLAMP: f32 = 3.0;

const POSITIVE_EMOJI: [&str; 6] = ["👍", "❤️", "🔥", "😂", "💯", "⭐"];
const NEGATIVE_EMOJI: [&str; 4] = ["👎", "💀", "😡", "🤮"];

/// A reply awaiting settlement.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Pending {
    pub state: Vec<f32>,
    pub action: usize,
    pub scoped_guild_id: String,
    pub reward: f32,
    pub positive_reactions: u8,
    /// Unix seconds at registration.
    pub created_at: u64,
    pub settle_immediately: bool,
}

/// Holds replies open for their settlement window and closes them into
/// experiences. Keyed by the native id of the message Abbey sent.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RewardCollector {
    pending: HashMap<String, Pending>,
}

impl RewardCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take everything still open — for persistence, so a restart inside the
    /// settlement window does not drop the reward. `restore` puts it back.
    pub fn export_pending(&self) -> Vec<(String, Pending)> {
        let mut rows: Vec<(String, Pending)> = self
            .pending
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }

    /// Restore previously exported rows (existing keys win).
    pub fn restore(&mut self, rows: Vec<(String, Pending)>) {
        for (k, v) in rows {
            self.pending.entry(k).or_insert(v);
        }
    }

    /// Number of replies still awaiting settlement.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Open a reply for evidence. Starts at −0.2; engagement earns it back.
    pub fn register_reply(
        &mut self,
        state: Vec<f32>,
        action: usize,
        sent_native_message_id: impl Into<String>,
        scoped_guild_id: impl Into<String>,
        now: u64,
    ) {
        self.pending.insert(
            sent_native_message_id.into(),
            Pending {
                state,
                action,
                scoped_guild_id: scoped_guild_id.into(),
                reward: REPLY_BASELINE,
                positive_reactions: 0,
                created_at: now,
                settle_immediately: false,
            },
        );
    }

    /// Silence settles instantly at 0 — there is nothing to wait for. Pure
    /// constructor; the caller hands the experience to the brain registry.
    pub fn silence_experience(state: Vec<f32>) -> Experience {
        Experience {
            next_state: state.clone(),
            state,
            action: BotAction::Stay.index(),
            reward: 0.0,
            done: true,
        }
    }

    /// A reaction on one of Abbey's messages. Removed reactions and unknown
    /// targets are ignored. Positive reactions earn +1 each, capped at three;
    /// negative reactions cost −1 each, uncapped (the settle-time clamp bounds it).
    pub fn reaction(&mut self, emoji: &str, target_native_message_id: &str, added: bool) {
        if !added {
            return;
        }
        let Some(p) = self.pending.get_mut(target_native_message_id) else {
            return;
        };
        if POSITIVE_EMOJI.contains(&emoji) {
            if p.positive_reactions < MAX_POSITIVE_REACTIONS {
                p.reward += 1.0;
                p.positive_reactions += 1;
            }
        } else if NEGATIVE_EMOJI.contains(&emoji) {
            p.reward -= 1.0;
        }
    }

    /// A human replied to one of Abbey's messages: +0.5.
    pub fn human_replied(&mut self, to_native_message_id: &str) {
        if let Some(p) = self.pending.get_mut(to_native_message_id) {
            p.reward += 0.5;
        }
    }

    /// One of Abbey's messages was deleted: −2.0, and it settles on the next sweep.
    pub fn abbey_message_deleted(&mut self, native_message_id: &str) {
        if let Some(p) = self.pending.get_mut(native_message_id) {
            p.reward = -2.0;
            p.settle_immediately = true;
        }
    }

    /// Drain every entry flagged for immediate settlement or older than the
    /// window (strictly older — an entry exactly at the window stays open).
    ///
    /// Each becomes a bandit-style episode: single step, `done = true`,
    /// `next_state == state`, reward clamped to ±3. The gamma term in the
    /// Bellman update zeroes out via `done` — deliberate; conversational credit
    /// assignment beyond one exchange is not worth the variance.
    pub fn settle_expired(&mut self, now: u64) -> Vec<(String, Experience)> {
        let expired: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, p)| {
                p.settle_immediately || now.saturating_sub(p.created_at) > SETTLEMENT_WINDOW_SECS
            })
            .map(|(k, _)| k.clone())
            .collect();
        expired
            .into_iter()
            .filter_map(|key| self.pending.remove(&key))
            .map(|p| {
                let exp = Experience {
                    next_state: p.state.clone(),
                    state: p.state,
                    action: p.action,
                    reward: p.reward.clamp(-REWARD_CLAMP, REWARD_CLAMP),
                    done: true,
                };
                (p.scoped_guild_id, exp)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_rewards_export_and_restore_across_a_restart() {
        let mut a = RewardCollector::new();
        a.register_reply(vec![0.0; 3], 1, "m1", "discord:g", 10);
        a.reaction("👍", "m1", true);
        let rows = a.export_pending();
        let json = serde_json::to_string(&rows).unwrap();
        let back: Vec<(String, Pending)> = serde_json::from_str(&json).unwrap();
        let mut b = RewardCollector::new();
        b.restore(back);
        assert_eq!(b.pending_len(), 1);
        let settled = b.settle_expired(10 + SETTLEMENT_WINDOW_SECS + 1);
        assert_eq!(settled.len(), 1);
        assert!(
            (settled[0].1.reward - 0.8).abs() < 1e-6,
            "the reaction survived the restart"
        );
    }

    const T0: u64 = 1_700_000_000;

    fn collector_with_reply() -> RewardCollector {
        let mut c = RewardCollector::new();
        c.register_reply(
            vec![0.5, 0.25],
            BotAction::Reply.index(),
            "msg-1",
            "g-1",
            T0,
        );
        c
    }

    fn reward_of(c: &RewardCollector, id: &str) -> f32 {
        c.pending[id].reward
    }

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn reply_starts_mildly_negative() {
        let c = collector_with_reply();
        assert_eq!(c.pending_len(), 1);
        assert!(approx(reward_of(&c, "msg-1"), -0.2));
        assert_eq!(c.pending["msg-1"].positive_reactions, 0);
        assert!(!c.pending["msg-1"].settle_immediately);
        assert_eq!(c.pending["msg-1"].created_at, T0);
    }

    #[test]
    fn three_positives_cap_and_fourth_is_ignored() {
        let mut c = collector_with_reply();
        c.reaction("👍", "msg-1", true);
        c.reaction("❤️", "msg-1", true);
        c.reaction("🔥", "msg-1", true);
        assert!(approx(reward_of(&c, "msg-1"), 2.8));
        c.reaction("💯", "msg-1", true);
        assert!(
            approx(reward_of(&c, "msg-1"), 2.8),
            "fourth positive ignored"
        );
        assert_eq!(c.pending["msg-1"].positive_reactions, 3);
    }

    #[test]
    fn negative_reaction_subtracts_one_without_cap() {
        let mut c = collector_with_reply();
        c.reaction("👎", "msg-1", true);
        assert!(approx(reward_of(&c, "msg-1"), -1.2));
        c.reaction("💀", "msg-1", true);
        c.reaction("😡", "msg-1", true);
        c.reaction("🤮", "msg-1", true);
        assert!(approx(reward_of(&c, "msg-1"), -4.2));
    }

    #[test]
    fn removed_reactions_unknown_emoji_and_unknown_targets_are_ignored() {
        let mut c = collector_with_reply();
        c.reaction("👍", "msg-1", false);
        c.reaction("🙂", "msg-1", true);
        c.reaction("👍", "msg-other", true);
        assert!(approx(reward_of(&c, "msg-1"), -0.2));
        assert_eq!(c.pending_len(), 1);
    }

    #[test]
    fn human_reply_adds_half() {
        let mut c = collector_with_reply();
        c.human_replied("msg-1");
        assert!(approx(reward_of(&c, "msg-1"), 0.3));
        c.human_replied("msg-other");
        assert_eq!(c.pending_len(), 1);
    }

    #[test]
    fn deletion_settles_immediately_at_minus_two() {
        let mut c = collector_with_reply();
        c.reaction("👍", "msg-1", true);
        c.abbey_message_deleted("msg-1");
        let settled = c.settle_expired(T0 + 1);
        assert_eq!(settled.len(), 1);
        let (guild, exp) = &settled[0];
        assert_eq!(guild, "g-1");
        assert_eq!(exp.reward, -2.0);
        assert!(exp.done);
        assert_eq!(exp.state, exp.next_state);
        assert_eq!(exp.action, BotAction::Reply.index());
        assert_eq!(c.pending_len(), 0);
        c.abbey_message_deleted("msg-other");
        assert_eq!(c.pending_len(), 0);
    }

    #[test]
    fn settles_only_strictly_after_the_window() {
        let mut c = collector_with_reply();
        assert!(c.settle_expired(T0).is_empty());
        assert!(
            c.settle_expired(T0 + SETTLEMENT_WINDOW_SECS).is_empty(),
            "exactly at the window stays open"
        );
        assert_eq!(c.pending_len(), 1);
        let settled = c.settle_expired(T0 + SETTLEMENT_WINDOW_SECS + 1);
        assert_eq!(settled.len(), 1);
        assert!(approx(settled[0].1.reward, -0.2));
        assert!(settled[0].1.done);
        assert_eq!(settled[0].1.state, vec![0.5, 0.25]);
        assert_eq!(settled[0].1.next_state, vec![0.5, 0.25]);
        assert_eq!(c.pending_len(), 0);
        assert!(c.settle_expired(T0 + 10_000).is_empty());
    }

    #[test]
    fn settled_reward_is_clamped_to_plus_minus_three() {
        let mut c = collector_with_reply();
        for _ in 0..10 {
            c.reaction("👎", "msg-1", true);
        }
        let settled = c.settle_expired(T0 + SETTLEMENT_WINDOW_SECS + 1);
        assert_eq!(settled[0].1.reward, -3.0);

        let mut c = collector_with_reply();
        for e in ["👍", "❤️", "🔥"] {
            c.reaction(e, "msg-1", true);
        }
        c.human_replied("msg-1");
        c.human_replied("msg-1");
        // −0.2 + 3 + 0.5 + 0.5 = 3.8 → clamped to 3.
        let settled = c.settle_expired(T0 + SETTLEMENT_WINDOW_SECS + 1);
        assert_eq!(settled[0].1.reward, 3.0);
    }

    #[test]
    fn settle_drains_only_expired_entries() {
        let mut c = collector_with_reply();
        c.register_reply(vec![1.0], 1, "msg-2", "g-2", T0 + 100);
        let settled = c.settle_expired(T0 + SETTLEMENT_WINDOW_SECS + 1);
        assert_eq!(settled.len(), 1);
        assert_eq!(settled[0].0, "g-1");
        assert_eq!(c.pending_len(), 1);
        assert!(c.pending.contains_key("msg-2"));
    }

    #[test]
    fn silence_experience_is_done_with_zero_reward_and_stay() {
        let exp = RewardCollector::silence_experience(vec![0.1, 0.2, 0.3]);
        assert_eq!(exp.reward, 0.0);
        assert!(exp.done);
        assert_eq!(exp.action, BotAction::Stay.index());
        assert_eq!(exp.state, vec![0.1, 0.2, 0.3]);
        assert_eq!(exp.next_state, exp.state);
    }
}

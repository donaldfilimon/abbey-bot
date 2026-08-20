//! Delayed reward collection (`docs/spec/adaptivelearning.md`, "Reward signal").
//!
//! Abbey acts now; the guild reacts over the next couple of minutes. Each reply
//! is held open for a settlement window while reactions, human replies, and
//! deletions accumulate evidence, then closes into a single-step experience.
//!
//! Pure: the clock is injected (`now` in unix seconds) and nothing is written
//! anywhere — [`RewardCollector::settle_expired`] hands the settled experiences
//! back for the caller to route to the per-guild brain.
//!
//! Two reward channels settle into one number:
//!
//! - the **immediate heuristic** (`Pending::reward`) — the baseline, reactions,
//!   an untyped human reply, a deletion. Unchanged from before delayed
//!   outcomes existed.
//! - the **delayed channel** (`Pending::delayed_sum` / `delayed_count`) — typed
//!   [`ReplyOutcome`]s credited to the turn by [`RewardCollector::observe_reply_to`]
//!   or [`RewardCollector::observe_in_scope`].
//!
//! [`outcome::blend`] combines them at settlement and returns the immediate
//! value *untouched* when no outcome ever arrived. That is the whole
//! degradation story: a turn nobody engaged with settles at exactly the number
//! it settled at before this channel existed.

use std::collections::HashMap;

use crate::brain::outcome::{self, ReplyOutcome};
use crate::brain::replay::Experience;
use crate::brain::state::BotAction;

/// How long a reply stays open for evidence, in seconds (2.5 min).
pub const SETTLEMENT_WINDOW_SECS: u64 = 150;

/// How long a turn stays attributable to a later observation in its channel.
///
/// Bound to [`SETTLEMENT_WINDOW_SECS`] deliberately rather than tuned
/// separately: a second, independent TTL could drift past the settlement
/// window — crediting an observation to a turn already drained, or expiring
/// attribution while the turn was still open. One number, one lifetime. Turns
/// nothing ever attributes to are not leaked: they expire through
/// [`RewardCollector::settle_expired`] like any other.
pub const ATTRIBUTION_TTL_SECS: u64 = SETTLEMENT_WINDOW_SECS;

/// Reward a reply starts at: mildly negative, so engagement has to earn it back.
const REPLY_BASELINE: f32 = -0.2;
/// Positive reactions beyond this many earn nothing more.
const MAX_POSITIVE_REACTIONS: u8 = 3;
/// Settled rewards are clamped to this magnitude.
const REWARD_CLAMP: f32 = 3.0;

const POSITIVE_EMOJI: [&str; 6] = ["👍", "❤️", "🔥", "😂", "💯", "⭐"];
const NEGATIVE_EMOJI: [&str; 4] = ["👎", "💀", "😡", "🤮"];

/// Everything needed to open an attributable turn.
///
/// A struct rather than more parameters because the argument list is already
/// at Clippy's limit, and because these travel together: state and action are
/// what the policy did, `scope` and `ask` are what a later observation needs
/// to find its way back here.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplyTurn {
    pub state: Vec<f32>,
    pub action: usize,
    /// Native id of the message Abbey sent — the turn id, and the map key.
    pub sent_native_message_id: String,
    /// Scoped channel id. Attribution scope for follow-ups that are not
    /// Discord reply-tos. Empty means "not attributable by scope".
    pub scope: String,
    pub scoped_guild_id: String,
    /// The user message this turn answered, as the human wrote it — what
    /// [`outcome::classify`] compares a later question against to decide
    /// whether it is the same ask, the same topic, or unrelated.
    ///
    /// The *raw* text, not the vision-enriched text the model was prompted
    /// with: folded-in image descriptions are Abbey's prose, not the human's,
    /// and padding the ask with them would depress every later overlap ratio.
    pub ask: String,
    /// Scoped id of the human this turn answered. Corroborates a marker-only
    /// outcome that arrives with no reply-to pointer.
    pub asker: String,
    /// Unix seconds.
    pub now: u64,
}

/// A reply awaiting settlement.
///
/// Persisted (`persist.rs` writes `pending_rewards` to disk), so every field
/// added after the first release carries `#[serde(default)]` — a state file
/// written by an older build must still load, and a failure here takes the
/// whole `Stores` load down, not just the reward ledger.
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
    /// Scoped channel id, for scope-keyed attribution. Empty on turns opened
    /// without conversational context and on rows restored from an older
    /// state file — both simply cannot be credited by scope.
    #[serde(default)]
    pub scope: String,
    /// The ask this turn answered. Empty is handled the same way.
    #[serde(default)]
    pub ask: String,
    /// Scoped id of the human this turn answered. Empty means no marker-only
    /// outcome can be corroborated, so none is credited by scope.
    #[serde(default)]
    pub asker: String,
    /// Sum of the typed delayed outcomes credited to this turn.
    #[serde(default)]
    pub delayed_sum: f32,
    /// How many typed outcomes are in `delayed_sum`. Zero means the delayed
    /// channel is silent and settlement uses the immediate heuristic alone.
    #[serde(default)]
    pub delayed_count: u16,
}

/// Holds replies open for their settlement window and closes them into
/// experiences. Keyed by the native id of the message Abbey sent — the turn
/// id. `(scope, turn id)` is the attribution key: `scope` narrows to a
/// channel, the turn id names the exact action that earned the outcome.
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
    ///
    /// The context-free form: no channel scope and no ask, so the turn can be
    /// credited only by an explicit reply-to or a reaction, never by a
    /// same-channel follow-up. Right for a bare reaction, whose "turn" is the
    /// *user's* message id — nobody replies to a reaction, and there is no
    /// Abbey text for them to thank. Use [`Self::register_turn`] for anything
    /// Abbey actually said.
    pub fn register_reply(
        &mut self,
        state: Vec<f32>,
        action: usize,
        sent_native_message_id: impl Into<String>,
        scoped_guild_id: impl Into<String>,
        now: u64,
    ) {
        self.register_turn(ReplyTurn {
            state,
            action,
            sent_native_message_id: sent_native_message_id.into(),
            scope: String::new(),
            scoped_guild_id: scoped_guild_id.into(),
            ask: String::new(),
            asker: String::new(),
            now,
        });
    }

    /// Open a reply for evidence, carrying the context that makes a later
    /// observation attributable. Same −0.2 baseline and same settlement.
    pub fn register_turn(&mut self, turn: ReplyTurn) {
        self.pending.insert(
            turn.sent_native_message_id,
            Pending {
                state: turn.state,
                action: turn.action,
                scoped_guild_id: turn.scoped_guild_id,
                reward: REPLY_BASELINE,
                positive_reactions: 0,
                created_at: turn.now,
                settle_immediately: false,
                scope: turn.scope,
                ask: turn.ask,
                asker: turn.asker,
                delayed_sum: 0.0,
                delayed_count: 0,
            },
        );
    }

    /// The ask a specific open turn answered, if that turn is still open.
    pub fn open_ask(&self, turn_id: &str) -> Option<&str> {
        self.pending
            .get(turn_id)
            .map(|p| p.ask.as_str())
            .filter(|a| !a.is_empty())
    }

    /// Turn id of the newest still-attributable turn in `scope`.
    ///
    /// Newest wins because a channel's most recent Abbey turn is what a bare
    /// follow-up is almost always reacting to. Ties on `created_at` break on
    /// the turn id so the choice is deterministic — `HashMap` iteration order
    /// is not, and a nondeterministic reward would be untestable and
    /// unreproducible across restarts.
    ///
    /// Skips turns outside [`ATTRIBUTION_TTL_SECS`] and turns already flagged
    /// for immediate settlement (a deleted message must not absorb credit for
    /// what someone said afterwards). The TTL is checked here rather than
    /// left to sweep timing, so attribution does not depend on when the
    /// scheduler last ran.
    pub fn newest_open_turn_in_scope(&self, scope: &str, now: u64) -> Option<&str> {
        if scope.is_empty() {
            return None;
        }
        self.pending
            .iter()
            .filter(|(_, p)| {
                p.scope == scope
                    && !p.settle_immediately
                    && now.saturating_sub(p.created_at) <= ATTRIBUTION_TTL_SECS
            })
            .max_by(|a, b| a.1.created_at.cmp(&b.1.created_at).then(a.0.cmp(b.0)))
            .map(|(k, _)| k.as_str())
    }

    /// The ask of the newest attributable turn in `scope`.
    pub fn open_ask_in_scope(&self, scope: &str, now: u64) -> Option<&str> {
        let turn = self.newest_open_turn_in_scope(scope, now)?;
        self.open_ask(turn)
    }

    /// Credit a typed outcome to the turn named by an explicit reply-to.
    ///
    /// Returns whether it landed on an open turn. This touches only the
    /// delayed channel: [`Self::human_replied`] remains the immediate
    /// heuristic's "a human engaged at all" credit. A Discord reply-to
    /// legitimately feeds both — one records *that* someone engaged, the other
    /// records *what they said*, which is the blend, not a double count.
    pub fn observe_reply_to(&mut self, turn_id: &str, outcome: ReplyOutcome) -> bool {
        match self.pending.get_mut(turn_id) {
            Some(p) => {
                credit(p, outcome);
                true
            }
            None => false,
        }
    }

    /// Credit a typed outcome to the newest attributable turn in `scope`.
    ///
    /// The (scope, turn id) path: a follow-up question or a thank-you posted
    /// as an ordinary channel message carries no reply-to pointer, so the only
    /// way back to the action that earned it is "the last thing Abbey said
    /// here, if it is still recent".
    ///
    /// `observer` is the scoped id of whoever spoke. A marker-only outcome
    /// ([`ReplyOutcome::needs_the_original_asker`]) is credited only when it
    /// comes from the human the turn answered — otherwise "thanks Carol!" in a
    /// busy channel would land on Abbey's open turn at full weight, which is
    /// the highest-frequency way this path could lie. Topical outcomes carry
    /// their own corroboration and may come from anyone.
    ///
    /// **Still a heuristic.** The reply-to path is the precise one; this trades
    /// precision for coverage, bounded by the TTL, by the asker check, by the
    /// ±3 settlement clamp, and by the fact that
    /// [`ReplyOutcome::NoEngagement`] — the most common classification — costs
    /// nothing.
    ///
    /// Returns the turn id credited, if any.
    pub fn observe_in_scope(
        &mut self,
        scope: &str,
        observer: &str,
        outcome: ReplyOutcome,
        now: u64,
    ) -> Option<String> {
        let turn = self.newest_open_turn_in_scope(scope, now)?.to_owned();
        let p = self.pending.get_mut(&turn)?;
        if outcome.needs_the_original_asker() && (p.asker.is_empty() || p.asker != observer) {
            return None;
        }
        credit(p, outcome);
        Some(turn)
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
    ///
    /// The **untyped** engagement credit, unchanged: it records that someone
    /// bothered to reply, and says nothing about whether the reply helped.
    /// Still the whole story when the message body is unreadable — without the
    /// MESSAGE_CONTENT intent Discord delivers an empty body, and there is
    /// nothing for [`outcome::classify`] to read.
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
    /// assignment beyond one exchange is not worth the variance. The delayed
    /// outcome does not change that: it is credit for *this* action, folded
    /// into this action's reward, not a bootstrapped future value.
    ///
    /// The settled reward is [`outcome::blend`] of the immediate heuristic and
    /// the delayed channel. With no typed outcome the blend is the identity,
    /// so this is byte-for-byte the number it produced before.
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
                let blended = outcome::blend(p.reward, p.delayed_sum, p.delayed_count);
                let exp = Experience {
                    next_state: p.state.clone(),
                    state: p.state,
                    action: p.action,
                    reward: blended.clamp(-REWARD_CLAMP, REWARD_CLAMP),
                    done: true,
                };
                (p.scoped_guild_id, exp)
            })
            .collect()
    }
}

/// Fold one typed outcome into a pending turn's delayed channel.
///
/// [`ReplyOutcome::NoEngagement`] is recorded as nothing at all — not as a
/// zero-valued sample. A zero sample would still increment the count and drag
/// a later thanks toward the middle, which would make weak evidence quietly
/// dilute strong evidence. Attribution still *succeeded*; it just cost
/// nothing, which is the honest reading of "the human did not visibly react".
fn credit(p: &mut Pending, outcome: ReplyOutcome) {
    let value = outcome.delayed_value();
    if value == 0.0 {
        return;
    }
    p.delayed_sum += value;
    p.delayed_count = p.delayed_count.saturating_add(1);
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

    // ---- delayed outcome channel -------------------------------------------

    const CHAN: &str = "discord:g-1:c-1";
    /// The human whose ask the test turns answered.
    const ASKER: &str = "discord:alice";
    /// Someone else in the same channel.
    const BYSTANDER: &str = "discord:bob";

    fn turn(id: &str, scope: &str, ask: &str, now: u64) -> ReplyTurn {
        ReplyTurn {
            state: vec![0.5, 0.25],
            action: BotAction::Reply.index(),
            sent_native_message_id: id.to_owned(),
            scope: scope.to_owned(),
            scoped_guild_id: "g-1".to_owned(),
            ask: ask.to_owned(),
            asker: ASKER.to_owned(),
            now,
        }
    }

    fn collector_with_turn() -> RewardCollector {
        let mut c = RewardCollector::new();
        c.register_turn(turn(
            "abbey-1",
            CHAN,
            "how do I configure the voice gateway timeout?",
            T0,
        ));
        c
    }

    fn settled_reward(mut c: RewardCollector) -> f32 {
        let settled = c.settle_expired(T0 + SETTLEMENT_WINDOW_SECS + 1);
        assert_eq!(settled.len(), 1);
        settled[0].1.reward
    }

    #[test]
    fn a_pending_row_written_before_delayed_outcomes_still_loads() {
        // Exactly the shape an older build wrote: no scope, ask, delayed_sum,
        // or delayed_count. If this ever fails, every existing state file on
        // disk fails to load — and it takes the whole `Stores` load with it,
        // not just the reward ledger.
        let legacy = r#"[["m1",{
            "state":[0.0,0.0,0.0],
            "action":1,
            "scoped_guild_id":"discord:g",
            "reward":0.8,
            "positive_reactions":1,
            "created_at":10,
            "settle_immediately":false
        }]]"#;
        let rows: Vec<(String, Pending)> =
            serde_json::from_str(legacy).expect("legacy pending rows must still deserialize");
        let mut c = RewardCollector::new();
        c.restore(rows);
        assert_eq!(c.pending_len(), 1);
        assert_eq!(c.pending["m1"].scope, "");
        assert_eq!(c.pending["m1"].ask, "");
        assert_eq!(c.pending["m1"].delayed_count, 0);
        // And it settles at the number the old build would have produced.
        let settled = c.settle_expired(10 + SETTLEMENT_WINDOW_SECS + 1);
        assert!(approx(settled[0].1.reward, 0.8));
    }

    #[test]
    fn with_no_outcome_the_settled_reward_is_bit_identical_to_the_old_path() {
        // Same turn opened both ways; the delayed channel stays silent.
        let mut old = RewardCollector::new();
        old.register_reply(vec![0.5, 0.25], BotAction::Reply.index(), "m", "g-1", T0);
        old.reaction("👍", "m", true);
        old.human_replied("m");

        let mut new = collector_with_turn();
        new.reaction("👍", "abbey-1", true);
        new.human_replied("abbey-1");

        assert_eq!(
            settled_reward(new).to_bits(),
            settled_reward(old).to_bits(),
            "conversational context must not move the number by itself"
        );
    }

    #[test]
    fn an_explicit_reply_to_credits_exactly_that_turn() {
        let mut c = collector_with_turn();
        c.register_turn(turn("abbey-2", CHAN, "and the retry budget?", T0 + 5));
        assert!(c.observe_reply_to("abbey-1", ReplyOutcome::ExplicitThanks));
        assert_eq!(c.pending["abbey-1"].delayed_count, 1);
        assert!(approx(c.pending["abbey-1"].delayed_sum, 1.0));
        assert_eq!(
            c.pending["abbey-2"].delayed_count, 0,
            "the newer turn earned nothing"
        );
        assert!(
            !c.observe_reply_to("no-such-turn", ReplyOutcome::ExplicitThanks),
            "an unknown target is ignored, not invented"
        );
    }

    #[test]
    fn a_scoped_observation_lands_on_the_newest_turn_in_that_channel() {
        let mut c = collector_with_turn();
        c.register_turn(turn("abbey-2", CHAN, "and the retry budget?", T0 + 5));
        c.register_turn(turn("elsewhere", "discord:g-1:c-2", "unrelated", T0 + 9));

        let credited = c.observe_in_scope(CHAN, ASKER, ReplyOutcome::Correction, T0 + 10);
        assert_eq!(credited.as_deref(), Some("abbey-2"), "newest turn in scope");
        assert!(approx(c.pending["abbey-2"].delayed_sum, -1.0));
        assert_eq!(c.pending["abbey-1"].delayed_count, 0);
        assert_eq!(
            c.pending["elsewhere"].delayed_count, 0,
            "another channel's turn is out of scope"
        );
        assert_eq!(
            c.observe_in_scope("discord:g-9:c-9", ASKER, ReplyOutcome::Correction, T0 + 10),
            None,
            "a scope with no open turn credits nothing"
        );
    }

    #[test]
    fn a_turn_opened_without_a_scope_is_unreachable_by_scope() {
        let mut c = RewardCollector::new();
        c.register_reply(vec![0.0], BotAction::React.index(), "react-1", "g-1", T0);
        assert_eq!(c.newest_open_turn_in_scope("", T0), None);
        assert_eq!(
            c.observe_in_scope(CHAN, ASKER, ReplyOutcome::ExplicitThanks, T0),
            None
        );
        assert_eq!(c.pending["react-1"].delayed_count, 0);
        // …but an explicit reply-to still reaches it.
        assert!(c.observe_reply_to("react-1", ReplyOutcome::ExplicitThanks));
    }

    #[test]
    fn a_bystanders_thanks_does_not_land_on_abbeys_turn() {
        // "thanks Carol!" from someone Abbey never answered is the commonest
        // way scope attribution could lie, so a marker-only outcome needs the
        // original asker to corroborate it.
        let mut c = collector_with_turn();
        assert_eq!(
            c.observe_in_scope(CHAN, BYSTANDER, ReplyOutcome::ExplicitThanks, T0 + 1),
            None
        );
        assert_eq!(
            c.observe_in_scope(CHAN, BYSTANDER, ReplyOutcome::Correction, T0 + 1),
            None
        );
        assert_eq!(c.pending["abbey-1"].delayed_count, 0);
        assert_eq!(
            settled_reward(collector_with_turn()).to_bits(),
            settled_reward(c).to_bits()
        );
    }

    #[test]
    fn a_bystanders_topical_follow_up_still_counts() {
        // Shared content words are their own corroboration, so this one does
        // not need to come from the asker.
        let mut c = collector_with_turn();
        assert_eq!(
            c.observe_in_scope(CHAN, BYSTANDER, ReplyOutcome::FollowUpQuestion, T0 + 1),
            Some("abbey-1".to_owned())
        );
        assert!(approx(settled_reward(c), 0.2));
    }

    #[test]
    fn a_turn_with_no_recorded_asker_cannot_corroborate_a_marker() {
        // Rows restored from a state file written before `asker` existed.
        let mut c = RewardCollector::new();
        c.register_turn(ReplyTurn {
            asker: String::new(),
            ..turn("abbey-1", CHAN, "how do I build it?", T0)
        });
        assert_eq!(
            c.observe_in_scope(CHAN, ASKER, ReplyOutcome::ExplicitThanks, T0 + 1),
            None,
            "an empty asker matches nobody, not everybody"
        );
        // The topical path is unaffected.
        assert!(
            c.observe_in_scope(CHAN, ASKER, ReplyOutcome::RephrasedSameAsk, T0 + 1)
                .is_some()
        );
    }

    #[test]
    fn an_explicit_reply_to_needs_no_asker_check() {
        // The pointer is the corroboration: Discord says which message this
        // answers, so a third party thanking Abbey for someone else's answer
        // is still real evidence about that answer.
        let mut c = collector_with_turn();
        assert!(c.observe_reply_to("abbey-1", ReplyOutcome::ExplicitThanks));
        assert!(approx(settled_reward(c), 0.8));
    }

    #[test]
    fn attribution_expires_with_the_settlement_window() {
        let mut c = collector_with_turn();
        assert_eq!(
            c.newest_open_turn_in_scope(CHAN, T0 + ATTRIBUTION_TTL_SECS),
            Some("abbey-1"),
            "exactly at the TTL is still attributable"
        );
        assert_eq!(
            c.newest_open_turn_in_scope(CHAN, T0 + ATTRIBUTION_TTL_SECS + 1),
            None,
            "one second past and the turn is no longer a candidate"
        );
        assert_eq!(
            c.observe_in_scope(
                CHAN,
                ASKER,
                ReplyOutcome::ExplicitThanks,
                T0 + ATTRIBUTION_TTL_SECS + 1
            ),
            None
        );
        assert_eq!(c.pending["abbey-1"].delayed_count, 0);

        // Unattributed, it does not leak: the sweep drains it.
        let settled = c.settle_expired(T0 + SETTLEMENT_WINDOW_SECS + 1);
        assert_eq!(settled.len(), 1);
        assert!(approx(settled[0].1.reward, -0.2), "the untouched baseline");
        assert_eq!(c.pending_len(), 0);
    }

    #[test]
    fn a_deleted_turn_cannot_absorb_a_later_observation() {
        let mut c = collector_with_turn();
        c.abbey_message_deleted("abbey-1");
        assert_eq!(c.newest_open_turn_in_scope(CHAN, T0 + 1), None);
        assert_eq!(
            c.observe_in_scope(CHAN, ASKER, ReplyOutcome::ExplicitThanks, T0 + 1),
            None
        );
        assert!(approx(settled_reward(c), -2.0), "deletion still wins");
    }

    #[test]
    fn same_second_turns_break_their_tie_deterministically() {
        // Two turns registered in the same second: the choice must not depend
        // on HashMap iteration order, so run it repeatedly on fresh maps.
        for _ in 0..64 {
            let mut c = RewardCollector::new();
            c.register_turn(turn("aaa", CHAN, "first", T0));
            c.register_turn(turn("zzz", CHAN, "second", T0));
            assert_eq!(c.newest_open_turn_in_scope(CHAN, T0), Some("zzz"));
        }
    }

    #[test]
    fn the_ask_travels_with_the_turn_for_topic_comparison() {
        let mut c = collector_with_turn();
        assert_eq!(
            c.open_ask("abbey-1"),
            Some("how do I configure the voice gateway timeout?")
        );
        assert_eq!(c.open_ask_in_scope(CHAN, T0 + 1), c.open_ask("abbey-1"));
        assert_eq!(c.open_ask("missing"), None);
        assert_eq!(
            c.open_ask_in_scope(CHAN, T0 + ATTRIBUTION_TTL_SECS + 1),
            None,
            "an expired turn offers no ask"
        );

        // The round trip the pipeline performs: ask → classify → credit.
        let ask = c.open_ask_in_scope(CHAN, T0 + 1).unwrap().to_owned();
        let o = crate::brain::outcome::classify(
            "how can the voice gateway timeout be configured?",
            Some(&ask),
        );
        assert_eq!(o, Some(ReplyOutcome::RephrasedSameAsk));
        c.observe_in_scope(CHAN, ASKER, o.unwrap(), T0 + 1);
        assert!(
            approx(settled_reward(c), -0.7),
            "-0.2 baseline + 1.0 × -0.5"
        );
    }

    #[test]
    fn thanks_and_correction_settle_on_opposite_sides_of_silence() {
        let silent = settled_reward(collector_with_turn());

        let mut thanked = collector_with_turn();
        thanked.human_replied("abbey-1");
        thanked.observe_reply_to("abbey-1", ReplyOutcome::ExplicitThanks);
        // −0.2 baseline + 0.5 engaged + 1.0 × 1.0 typed.
        assert!(approx(settled_reward(thanked), 1.3));

        let mut corrected = collector_with_turn();
        corrected.human_replied("abbey-1");
        corrected.observe_reply_to("abbey-1", ReplyOutcome::Correction);
        // −0.2 + 0.5 − 1.0: worse than saying nothing at all.
        let corrected = settled_reward(corrected);
        assert!(approx(corrected, -0.7));
        assert!(
            corrected < silent,
            "a reply that had to be corrected must cost more than silence"
        );
    }

    #[test]
    fn no_engagement_neither_penalises_nor_dilutes() {
        let baseline = settled_reward(collector_with_turn());

        let mut c = collector_with_turn();
        for _ in 0..5 {
            assert!(c.observe_reply_to("abbey-1", ReplyOutcome::NoEngagement));
        }
        assert_eq!(c.pending["abbey-1"].delayed_count, 0, "recorded as nothing");
        assert_eq!(
            settled_reward(c).to_bits(),
            baseline.to_bits(),
            "weak evidence is not a negative"
        );

        // And it does not drag a real signal toward the middle.
        let mut c = collector_with_turn();
        c.observe_reply_to("abbey-1", ReplyOutcome::NoEngagement);
        c.observe_reply_to("abbey-1", ReplyOutcome::ExplicitThanks);
        assert!(approx(settled_reward(c), 0.8), "-0.2 + full 1.0, not 0.5");
    }

    #[test]
    fn several_outcomes_average_rather_than_accumulate() {
        let mut c = collector_with_turn();
        for _ in 0..8 {
            c.observe_reply_to("abbey-1", ReplyOutcome::FollowUpQuestion);
        }
        assert_eq!(c.pending["abbey-1"].delayed_count, 8);
        // Eight follow-ups are still one follow-up's worth of verdict, so a
        // chatty thread cannot saturate the ±3 clamp on engagement alone.
        assert!(approx(settled_reward(c), 0.2));
    }

    #[test]
    fn the_blended_reward_is_still_clamped() {
        let mut c = collector_with_turn();
        for e in ["👍", "❤️", "🔥"] {
            c.reaction(e, "abbey-1", true);
        }
        c.human_replied("abbey-1");
        c.observe_reply_to("abbey-1", ReplyOutcome::ExplicitThanks);
        // −0.2 + 3 + 0.5 + 1.0 = 4.3 → 3.
        assert_eq!(settled_reward(c), 3.0);
    }

    #[test]
    fn the_delayed_channel_survives_a_restart() {
        let mut a = collector_with_turn();
        a.observe_in_scope(CHAN, ASKER, ReplyOutcome::ExplicitThanks, T0 + 1);
        let json = serde_json::to_string(&a.export_pending()).unwrap();
        let rows: Vec<(String, Pending)> = serde_json::from_str(&json).unwrap();
        let mut b = RewardCollector::new();
        b.restore(rows);
        assert_eq!(b.pending["abbey-1"].scope, CHAN);
        assert_eq!(b.pending["abbey-1"].delayed_count, 1);
        // Scope attribution keeps working against the restored row.
        assert_eq!(
            b.newest_open_turn_in_scope(CHAN, T0 + 2),
            Some("abbey-1"),
            "the ledger key survives, so a follow-up after a restart still lands"
        );
        assert!(approx(settled_reward(b), 0.8));
    }
}

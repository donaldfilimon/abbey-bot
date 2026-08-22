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
mod tests;

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

mod attribution;
mod recovery;
pub use attribution::{FeedbackAttribution, ReactionKey};
pub use recovery::RewardRecovery;
pub(crate) use recovery::pending_rows;

use crate::brain::ask_signature::AskSignature;
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
    /// [`outcome::classify_signature`] compares a later question against to decide
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
    /// Bounded lexical context; old raw asks migrate on read, never publication.
    #[serde(default, alias = "ask")]
    pub ask_signature: AskSignature,
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
    /// Legacy rows have no dedup ledger: preserve their reward but refuse reactions.
    #[serde(default)]
    pub reaction_tracking: bool,
}

/// Holds replies open for their settlement window and closes them into
/// experiences. Keyed by the native id of the message Abbey sent — the turn
/// id. `(scope, turn id)` is the attribution key: `scope` narrows to a
/// channel, the turn id names the exact action that earned the outcome.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RewardCollector {
    pending: HashMap<String, Pending>,
    #[serde(default)]
    recovery: RewardRecovery,
}

impl RewardCollector {
    pub(crate) fn erase_learning(
        &mut self,
        scope: &str,
        member: Option<&str>,
        ledger: crate::brain::erasure::ErasureLedger,
    ) -> (usize, usize) {
        let before = self.pending.len();
        let removed: std::collections::HashSet<_> = self
            .pending
            .iter()
            .filter(|(_, p)| p.scoped_guild_id == scope && member.is_none_or(|m| m == p.asker))
            .map(|(id, _)| id.clone())
            .collect();
        let reaction_before = self.recovery.reactions.len();
        let own: Vec<_> = self
            .recovery
            .reactions
            .iter()
            .filter(|r| {
                self.pending
                    .get(&r.key.message)
                    .is_some_and(|p| p.scoped_guild_id == scope)
                    && member.is_none_or(|m| super::addenda::member_hash(m) == r.key.reactor_hash)
            })
            .map(|r| r.key.clone())
            .collect();
        for key in own {
            if let Some(index) = self.recovery.reactions.iter().position(|r| r.key == key) {
                let row = self.recovery.reactions.remove(index);
                if let Some(p) = self.pending.get_mut(&key.message) {
                    match row.contribution {
                        recovery::ReactionContribution::Positive => {
                            p.reward -= 1.0;
                            p.positive_reactions = p.positive_reactions.saturating_sub(1);
                        }
                        recovery::ReactionContribution::Negative => p.reward += 1.0,
                        recovery::ReactionContribution::Capped => (),
                    }
                }
            }
        }
        self.pending.retain(|id, _| !removed.contains(id));
        self.recovery
            .reactions
            .retain(|r| !removed.contains(&r.key.message));
        self.recovery.erasure = ledger;
        (
            before - self.pending.len(),
            reaction_before - self.recovery.reactions.len(),
        )
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// Take everything still open — for persistence, so a restart inside the
    /// settlement window does not drop the reward. `restore_recovered` restores
    /// these rows together with their canonical dedup metadata.
    pub fn export_pending(&self) -> Vec<(String, Pending)> {
        let mut rows: Vec<(String, Pending)> = self
            .pending
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }

    /// Number of replies still awaiting settlement.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Aggregate pending count and oldest age for a guild, with caller time.
    pub fn pending_age(&self, guild: &str, now: u64) -> (usize, Option<u64>) {
        let mut count = 0;
        let mut oldest = None;
        for p in self.pending.values().filter(|p| p.scoped_guild_id == guild) {
            count += 1;
            let age = now.saturating_sub(p.created_at);
            oldest = Some(oldest.map_or(age, |old: u64| old.max(age)));
        }
        (count, oldest)
    }

    /// Open a reply for evidence. Starts at −0.2; engagement earns it back.
    ///
    /// The context-free form has a scope but no ask. A React action can be
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
        scope: impl Into<String>,
        now: u64,
    ) {
        self.register_turn(ReplyTurn {
            state,
            action,
            sent_native_message_id: sent_native_message_id.into(),
            scope: scope.into(),
            scoped_guild_id: scoped_guild_id.into(),
            ask: String::new(),
            asker: String::new(),
            now,
        });
    }

    /// Open a reply for evidence, carrying the context that makes a later
    /// observation attributable. Same −0.2 baseline and same settlement.
    pub fn register_turn(&mut self, turn: ReplyTurn) {
        if self
            .recovery
            .erasure
            .blocks(&turn.scoped_guild_id, &turn.asker, turn.now)
        {
            return;
        }
        self.prune_recovery(turn.now);
        // Never replace a still-credited row, a retained closed turn, or an old
        // creation timestamp retired from the bounded recovery window.
        // Restored legacy rows may exceed today's admission bound. They are
        // never recreated here; freeze new work until that finite cohort drains.
        if self.pending.values().any(|p| !p.reaction_tracking)
            || self.pending.contains_key(&turn.sent_native_message_id)
            || self
                .recovery
                .settled
                .iter()
                .any(|row| row.message == turn.sent_native_message_id)
            || self.recovery.retired_through.is_some_and(|t| turn.now <= t)
            || self.pending.len() + self.recovery.settled.len() >= recovery::MAX_RECOVERY_ENTRIES
        {
            return;
        }
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
                ask_signature: AskSignature::from_text(&turn.ask),
                asker: turn.asker,
                delayed_sum: 0.0,
                delayed_count: 0,
                reaction_tracking: true,
            },
        );
    }

    /// The ask a specific open turn answered, if that turn is still open.
    pub fn open_ask(&self, turn_id: &str) -> Option<&AskSignature> {
        self.pending
            .get(turn_id)
            .map(|p| &p.ask_signature)
            .filter(|a| !a.token_hashes.is_empty())
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
        self.prune_recovery(now);
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
            .filter_map(|key| {
                let p = self.pending.remove(&key)?;
                self.recovery.reactions.retain(|row| row.key.message != key);
                if p.reaction_tracking {
                    self.recovery.settled.push(recovery::SettledTurn {
                        message: key,
                        scope: p.scope.clone(),
                        created_at: p.created_at,
                        closed_at: now.max(p.created_at),
                    });
                } else {
                    // Legacy contributions have no reaction keys to retain.
                    // Retire original creation time without allocating one
                    // marker per carryover row; still-open rows are untouched.
                    self.recovery.retired_through = Some(
                        self.recovery
                            .retired_through
                            .map_or(p.created_at, |old| old.max(p.created_at)),
                    );
                }
                Some(p)
            })
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

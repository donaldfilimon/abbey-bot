//! Per-guild style addenda: bounded, expiring, template-only prompt lines.
//!
//! Members' closed-vocabulary feedback ([`crate::brain::style_signal`]) is
//! recorded here as `(member hash, signal, time)`. When enough distinct
//! members agree inside a window ([`Policy`]), one fixed sentence for that
//! knob and direction ([`template`]) becomes active for a TTL and is rendered
//! after the persona core. Nothing a member or a model wrote is stored or
//! rendered: the ledger holds only enum values, keyed hashes and timestamps,
//! and [`AddendaLedger::render`] emits only `&'static str` templates. This is
//! the prompt-injection boundary for self-adjusting style.
//!
//! An operator can [`AddendaLedger::revert`] one knob or
//! [`AddendaLedger::clear`] them all. Either records a [`Suppression`]: for
//! the TTL that knob is neither observed nor re-applied. Rendering filters by
//! `now`, so an expired addendum never renders even before the next tick.
//!
//! Member keys are hashed with [`crate::wyhash`] under [`MEMBER_KEY_SEED`], a
//! fixed domain-separation key, so the ledger never holds a raw id while a
//! later erasure can still recompute a member's hash from their key.
//!
//! Pure: callers pass `now`; no clock, no I/O, no randomness.

use serde::{Deserialize, Serialize};

use crate::brain::style_signal::{StyleKnob, StyleSignal};
use crate::wyhash;

/// Domain-separation key for member hashes ("style_mk").
pub const MEMBER_KEY_SEED: u64 = 0x7374_796c_655f_6d6b;
/// Observations a guild's ledger keeps before the oldest fall off.
pub const MAX_OBSERVATIONS: usize = 64;
/// Observations one member may hold for one signal; a repeat refreshes the
/// oldest instead of adding, so one member cannot flood the ledger.
pub const MAX_PER_MEMBER_SIGNAL: usize = 2;
/// Hard ceiling on rendered bytes, whatever policy applied the addenda.
pub const MAX_RENDER_BYTES: usize = 400;
/// First line of a non-empty render.
const HEADER: &str = "Style preferences this server has asked for:";

/// The fixed limits addenda apply and expire under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Net signals (minus the opposite direction) needed inside the window.
    pub signals: usize,
    /// Distinct members among those signals.
    pub distinct_users: usize,
    pub window_secs: u64,
    pub ttl_secs: u64,
    pub max_addenda: usize,
    pub max_bytes: usize,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            signals: 5,
            distinct_users: 3,
            window_secs: 7 * 24 * 3600,
            ttl_secs: 14 * 24 * 3600,
            max_addenda: 4,
            max_bytes: MAX_RENDER_BYTES,
        }
    }
}

/// The one sentence each signal may contribute. Static text only.
pub fn template(signal: StyleSignal) -> &'static str {
    match signal {
        StyleSignal::TooLong => "Keep replies brief: a few sentences unless asked for detail.",
        StyleSignal::TooShort => {
            "Give fuller answers: include the key detail and a short example when it helps."
        }
        StyleSignal::TooFormal => "Use a relaxed, conversational tone.",
        StyleSignal::TooCasual => "Use a clear, professional tone.",
        StyleSignal::NoEmoji => "Do not use emoji.",
        StyleSignal::MoreEmoji => "An occasional emoji is welcome where it fits.",
        StyleSignal::PreferCode => "When code helps, show it in a fenced code block.",
        StyleSignal::FewerFollowUps => "Suppress optional contextual follow-ups in this server.",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Observation {
    member: u64,
    signal: StyleSignal,
    at: u64,
}

/// One active style line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Addendum {
    pub signal: StyleSignal,
    pub expires_at: u64,
}

/// A knob an operator reverted: not observed or re-applied before `until`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suppression {
    pub knob: StyleKnob,
    pub until: u64,
}

/// What `/admin addenda list` shows for one guild at one instant.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddendaStatus {
    /// Addenda render only while the guild has learning on.
    pub learning_enabled: bool,
    pub active: Vec<Addendum>,
    pub suppressions: Vec<Suppression>,
    pub pending: Vec<FeedbackEvidence>,
    pub policy: Policy,
}

/// Content-free evidence still pending, not the evidence consumed on apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeedbackEvidence {
    pub signal: StyleSignal,
    pub supporting: usize,
    pub opposing: usize,
    pub distinct_members: usize,
}

/// Every knob, in render order.
const KNOBS: [StyleKnob; 5] = [
    StyleKnob::Length,
    StyleKnob::Formality,
    StyleKnob::Emoji,
    StyleKnob::Code,
    StyleKnob::FollowUp,
];

/// What a [`AddendaLedger::tick`] changed, for the operator log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddendumChange {
    Applied(StyleSignal),
    Expired(StyleSignal),
}

/// One guild's observations and active addenda.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddendaLedger {
    #[serde(default)]
    observations: Vec<Observation>,
    /// At most one per knob, kept in knob order.
    #[serde(default)]
    active: Vec<Addendum>,
    /// At most one per knob, so bounded by the five knobs.
    #[serde(default)]
    suppressed: Vec<Suppression>,
}

/// The keyed hash a member key is stored as.
pub fn member_hash(member_key: &str) -> u64 {
    wyhash::hash(MEMBER_KEY_SEED, member_key.as_bytes())
}

impl AddendaLedger {
    pub(crate) fn erase_member(&mut self, key: &str) -> usize {
        let before = self.observations.len();
        let member = member_hash(key);
        self.observations.retain(|o| o.member != member);
        before - self.observations.len()
    }

    pub(crate) fn record_count(&self) -> usize {
        self.observations.len() + self.active.len() + self.suppressed.len()
    }

    /// Project only current observations without pruning or exposing member hashes.
    pub fn pending_evidence(&self, policy: &Policy, now: u64) -> Vec<FeedbackEvidence> {
        [
            StyleSignal::TooLong,
            StyleSignal::TooShort,
            StyleSignal::TooFormal,
            StyleSignal::TooCasual,
            StyleSignal::NoEmoji,
            StyleSignal::MoreEmoji,
            StyleSignal::PreferCode,
            StyleSignal::FewerFollowUps,
        ]
        .into_iter()
        .filter_map(|signal| {
            if self.is_suppressed(signal.knob(), now) {
                return None;
            }
            let current = |o: &&Observation| o.at <= now && now - o.at <= policy.window_secs;
            let mut members: Vec<_> = self
                .observations
                .iter()
                .filter(current)
                .filter(|o| o.signal == signal)
                .map(|o| o.member)
                .collect();
            let supporting = members.len();
            if supporting == 0 {
                return None;
            }
            members.sort_unstable();
            members.dedup();
            let opposing = signal.opposite().map_or(0, |opposite| {
                self.observations
                    .iter()
                    .filter(current)
                    .filter(|o| o.signal == opposite)
                    .count()
            });
            Some(FeedbackEvidence {
                signal,
                supporting,
                opposing,
                distinct_members: members.len(),
            })
        })
        .collect()
    }

    /// Record one signal from `member_key` (never stored raw).
    /// A signal for a suppressed knob is dropped.
    pub fn observe(&mut self, member_key: &str, signal: StyleSignal, now: u64) {
        if self.is_suppressed(signal.knob(), now) {
            return;
        }
        let member = member_hash(member_key);
        let mut own: Vec<usize> = self
            .observations
            .iter()
            .enumerate()
            .filter(|(_, o)| o.member == member && o.signal == signal)
            .map(|(index, _)| index)
            .collect();
        if own.len() >= MAX_PER_MEMBER_SIGNAL {
            own.sort_by_key(|&index| self.observations[index].at);
            self.observations.remove(own[0]);
        }
        self.observations.push(Observation {
            member,
            signal,
            at: now,
        });
        if self.observations.len() > MAX_OBSERVATIONS {
            let excess = self.observations.len() - MAX_OBSERVATIONS;
            self.observations.drain(..excess);
        }
    }

    /// Expire, prune, and apply whatever the evidence now supports.
    pub fn tick(&mut self, policy: &Policy, now: u64) -> Vec<AddendumChange> {
        let mut changes = Vec::new();
        self.active.retain(|addendum| {
            let live = now < addendum.expires_at;
            if !live {
                changes.push(AddendumChange::Expired(addendum.signal));
            }
            live
        });
        self.observations
            .retain(|o| o.at <= now && now - o.at <= policy.window_secs);
        self.suppressed.retain(|s| now < s.until);

        let mut candidates: Vec<StyleSignal> = self.observations.iter().map(|o| o.signal).collect();
        candidates.sort();
        candidates.dedup();
        for signal in candidates {
            if self.active.len() >= policy.max_addenda
                || self.active.iter().any(|a| a.signal.knob() == signal.knob())
                || self.is_suppressed(signal.knob(), now)
                || !self.supported(signal, policy)
            {
                continue;
            }
            let mut next: Vec<StyleSignal> = self.active.iter().map(|a| a.signal).collect();
            next.push(signal);
            next.sort_by_key(|s| s.knob());
            if render_signals(&next).len() > policy.max_bytes.min(MAX_RENDER_BYTES) {
                continue;
            }
            self.active.push(Addendum {
                signal,
                expires_at: now.saturating_add(policy.ttl_secs),
            });
            self.active.sort_by_key(|a| a.signal.knob());
            let knob = signal.knob();
            self.observations.retain(|o| o.signal.knob() != knob);
            changes.push(AddendumChange::Applied(signal));
        }
        changes
    }

    fn supported(&self, signal: StyleSignal, policy: &Policy) -> bool {
        let count = |wanted: StyleSignal| {
            self.observations
                .iter()
                .filter(|o| o.signal == wanted)
                .count()
        };
        let net = count(signal).saturating_sub(signal.opposite().map_or(0, count));
        let mut members: Vec<u64> = self
            .observations
            .iter()
            .filter(|o| o.signal == signal)
            .map(|o| o.member)
            .collect();
        members.sort_unstable();
        members.dedup();
        net >= policy.signals && members.len() >= policy.distinct_users
    }

    /// Remove `knob`'s addendum and its evidence, and suppress the knob for
    /// the TTL. True when an addendum was active; the suppression is recorded
    /// either way.
    pub fn revert(&mut self, knob: StyleKnob, policy: &Policy, now: u64) -> bool {
        let reverted = self.active(now).iter().any(|a| a.signal.knob() == knob);
        self.active.retain(|a| a.signal.knob() != knob);
        self.observations.retain(|o| o.signal.knob() != knob);
        let until = now.saturating_add(policy.ttl_secs);
        match self.suppressed.iter_mut().find(|s| s.knob == knob) {
            Some(existing) => existing.until = until,
            None => self.suppressed.push(Suppression { knob, until }),
        }
        self.suppressed.sort_by_key(|s| s.knob);
        reverted
    }

    /// [`Self::revert`] every knob; returns how many addenda were active.
    pub fn clear(&mut self, policy: &Policy, now: u64) -> usize {
        KNOBS
            .into_iter()
            .filter(|&knob| self.revert(knob, policy, now))
            .count()
    }

    fn is_suppressed(&self, knob: StyleKnob, now: u64) -> bool {
        self.suppressed
            .iter()
            .any(|s| s.knob == knob && now < s.until)
    }

    /// Nothing observed, active or suppressed: the ledger can be dropped.
    pub fn is_empty(&self) -> bool {
        self.observations.is_empty() && self.active.is_empty() && self.suppressed.is_empty()
    }

    /// The addenda live at `now`, in render (knob) order.
    pub fn active(&self, now: u64) -> Vec<Addendum> {
        self.active
            .iter()
            .filter(|a| now < a.expires_at && !self.is_suppressed(a.signal.knob(), now))
            .copied()
            .collect()
    }

    /// The suppressions live at `now`, in knob order.
    pub fn suppressions(&self, now: u64) -> Vec<Suppression> {
        self.suppressed
            .iter()
            .filter(|s| now < s.until)
            .copied()
            .collect()
    }

    /// The fixed templates for the addenda live at `now`, at most
    /// [`MAX_RENDER_BYTES`]; empty when nothing is live.
    pub fn render(&self, now: u64) -> String {
        let signals: Vec<StyleSignal> = self.active(now).iter().map(|a| a.signal).collect();
        render_signals(&signals)
    }
}

/// Header plus one line per signal, in the given order, dropping whole lines
/// from the end until the text fits [`MAX_RENDER_BYTES`].
fn render_signals(signals: &[StyleSignal]) -> String {
    let mut kept = signals.len();
    loop {
        if kept == 0 {
            return String::new();
        }
        let text = std::iter::once(HEADER.to_string())
            .chain(
                signals[..kept]
                    .iter()
                    .map(|&s| format!("- {}", template(s))),
            )
            .collect::<Vec<_>>()
            .join("\n");
        if text.len() <= MAX_RENDER_BYTES {
            return text;
        }
        kept -= 1;
    }
}

#[cfg(test)]
mod tests;

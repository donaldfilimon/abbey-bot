//! Bounded canonical recovery metadata, published atomically with pending rows.
use super::*;
use std::collections::HashSet;

pub(super) const MAX_RECOVERY_ENTRIES: usize = 4096;
const CLOSED_RETENTION_SECS: u64 = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum ReactionContribution {
    Positive,
    Negative,
    Capped,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct ReactionRecord {
    pub key: ReactionKey,
    pub contribution: ReactionContribution,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct SettledTurn {
    pub scope: String,
    pub message: String,
    pub created_at: u64,
    pub closed_at: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RewardRecovery {
    #[serde(default)]
    pub(crate) erasure: crate::brain::erasure::ErasureLedger,
    #[serde(default, deserialize_with = "bounded_rows")]
    pub(super) reactions: Vec<ReactionRecord>,
    #[serde(default, deserialize_with = "bounded_rows")]
    pub(super) settled: Vec<SettledTurn>,
    /// Creation-time floor for retired markers, not wall-clock authority.
    /// Legacy settlement and tracked-marker pruning advance only to an
    /// actually closed turn's original creation time. New deliveries use
    /// caller time and are newer in normal operation.
    /// A regressed clock or an old replay fails closed; it cannot reset credit.
    #[serde(default)]
    pub(super) retired_through: Option<u64>,
}

fn bounded_rows<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    rows_with_limit(deserializer, |_| true)
}

/// Old canonical documents had no pending-row cap. Preserve their finite
/// carryover cohort, including after it is republished with explicit false
/// tracking flags. Only new tracked rows consume the new recovery budget.
pub(crate) fn pending_rows<'de, D>(deserializer: D) -> Result<Vec<(String, Pending)>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    rows_with_limit(deserializer, |row: &(String, Pending)| {
        row.1.reaction_tracking
    })
}

fn rows_with_limit<'de, D, T>(deserializer: D, counts: fn(&T) -> bool) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    struct Rows<T>(fn(&T) -> bool);
    impl<'de, T: serde::Deserialize<'de>> serde::de::Visitor<'de> for Rows<T> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("at most 4096 tracked reward recovery records")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
            let mut rows = Vec::new();
            let mut tracked = 0;
            while let Some(row) = seq.next_element()? {
                tracked += usize::from((self.0)(&row));
                if tracked > MAX_RECOVERY_ENTRIES {
                    return Err(serde::de::Error::custom(
                        "reward recovery capacity exceeded",
                    ));
                }
                rows.push(row);
            }
            Ok(rows)
        }
    }
    deserializer.deserialize_seq(Rows(counts))
}

fn identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.contains('\u{1f}')
}

impl RewardRecovery {
    /// Reject inconsistent ledgers rather than silently dropping dedup keys and
    /// allowing a replay to add a second contribution. Legacy rows remain inert
    /// to reactions until naturally settled; they cannot reconstruct old keys.
    pub fn validate(&self, pending: &[(String, Pending)]) -> Result<(), &'static str> {
        self.erasure.validate()?;
        if self.reactions.len() > MAX_RECOVERY_ENTRIES
            || self.settled.len() + pending.iter().filter(|(_, p)| p.reaction_tracking).count()
                > MAX_RECOVERY_ENTRIES
        {
            return Err("reward recovery capacity exceeded");
        }
        let mut keys = HashSet::new();
        for (id, p) in pending {
            if !identity(id)
                || !keys.insert(id.as_str())
                || BotAction::from_index(p.action).is_none()
                || !p.ask_signature.valid()
                || !p.reward.is_finite()
                || !p.delayed_sum.is_finite()
                || p.state.iter().any(|v| !v.is_finite())
                || p.positive_reactions > MAX_POSITIVE_REACTIONS
                || (!p.scope.is_empty() && !identity(&p.scope))
            {
                return Err("invalid pending reward");
            }
        }
        for row in &self.settled {
            if !identity(&row.message)
                || (!row.scope.is_empty() && !identity(&row.scope))
                || row.closed_at < row.created_at
                || !keys.insert(row.message.as_str())
            {
                return Err("invalid settled reward");
            }
        }
        let mut reactions = HashSet::new();
        for row in &self.reactions {
            let key = &row.key;
            if !identity(&key.scope) || !identity(&key.message) || !reactions.insert(key) {
                return Err("invalid reaction identity");
            }
            let Some((_, p)) = pending.iter().find(|(id, _)| id == &key.message) else {
                return Err("reaction without pending reward");
            };
            let positive = POSITIVE_EMOJI.contains(&key.emoji.as_str());
            if !p.reaction_tracking
                || p.scope != key.scope
                || match row.contribution {
                    ReactionContribution::Positive | ReactionContribution::Capped => !positive,
                    ReactionContribution::Negative => !NEGATIVE_EMOJI.contains(&key.emoji.as_str()),
                }
            {
                return Err("invalid reaction contribution");
            }
        }
        for (id, p) in pending.iter().filter(|(_, p)| p.reaction_tracking) {
            let positives = self
                .reactions
                .iter()
                .filter(|r| {
                    &r.key.message == id && r.contribution == ReactionContribution::Positive
                })
                .count();
            if positives != usize::from(p.positive_reactions) {
                return Err("reaction count does not match pending reward");
            }
        }
        Ok(())
    }
}

impl RewardCollector {
    pub fn export_recovery(&self) -> RewardRecovery {
        let mut recovery = self.recovery.clone();
        recovery.reactions.sort_by(|a, b| {
            (
                &a.key.scope,
                &a.key.message,
                a.key.reactor_hash,
                &a.key.emoji,
            )
                .cmp(&(
                    &b.key.scope,
                    &b.key.message,
                    b.key.reactor_hash,
                    &b.key.emoji,
                ))
        });
        recovery.settled.sort_by(|a, b| a.message.cmp(&b.message));
        recovery
    }

    pub fn restore_recovered(
        &mut self,
        rows: Vec<(String, Pending)>,
        recovery: RewardRecovery,
    ) -> Result<(), &'static str> {
        recovery.validate(&rows)?;
        if !self.pending.is_empty() || self.recovery != RewardRecovery::default() {
            return Err("reward collector already initialized");
        }
        self.pending = rows.into_iter().collect();
        self.recovery = recovery;
        Ok(())
    }

    pub(super) fn prune_recovery(&mut self, now: u64) {
        self.recovery.settled.retain(|row| {
            if now.saturating_sub(row.closed_at) >= CLOSED_RETENTION_SECS {
                self.recovery.retired_through = Some(
                    self.recovery
                        .retired_through
                        .map_or(row.created_at, |old| old.max(row.created_at)),
                );
                false
            } else {
                true
            }
        });
    }
}

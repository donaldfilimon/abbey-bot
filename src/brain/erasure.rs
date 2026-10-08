//! Linkable learning deletion. Pure caller-time state transitions; facts are not
//! part of this authority. Aggregate influence requires a separate scoped reset.
use crate::{
    brain::{reward::RewardCollector, social::SocialBrain},
    persist::Stores,
    work::WorkError,
};
use serde::{Deserialize, Serialize};

pub const RETENTION_SECS: u64 = 300;
pub const MAX_TOMBSTONES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tombstone {
    pub subject: String,
    pub reactor: Option<String>,
    pub at: u64,
    pub until: u64,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ErasureLedger {
    pub rows: Vec<Tombstone>,
}
fn digest(value: impl Serialize) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(serde_json::to_vec(&value).expect("closed learning identity"))
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn subject(scope: &str, member: Option<&str>) -> String {
    digest(("learning-erasure-v1", scope, member))
}
fn reactor(scope: &str, hash: u64) -> String {
    digest(("learning-reactor-erasure-v1", scope, hash))
}
impl ErasureLedger {
    pub fn validate(&self) -> Result<(), &'static str> {
        let valid = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit());
        if self.rows.len() > MAX_TOMBSTONES {
            return Err("learning erasure capacity exceeded");
        }
        let mut keys = std::collections::BTreeSet::new();
        for r in &self.rows {
            if !valid(&r.subject)
                || r.reactor.as_deref().is_some_and(|r| !valid(r))
                || r.at.checked_add(RETENTION_SECS) != Some(r.until)
                || !keys.insert(&r.subject)
            {
                return Err("invalid learning erasure tombstone");
            }
        }
        Ok(())
    }
    pub fn blocks(&self, scope: &str, member: &str, admitted_at: u64) -> bool {
        let scope_key = subject(scope, None);
        let member_key = subject(scope, Some(member));
        self.rows
            .iter()
            .any(|r| (r.subject == scope_key || r.subject == member_key) && admitted_at <= r.until)
    }
    pub fn blocks_hash(&self, scope: &str, hash: u64, admitted_at: u64) -> bool {
        let scope_key = subject(scope, None);
        let member_key = reactor(scope, hash);
        self.rows.iter().any(|r| {
            (r.subject == scope_key || r.reactor.as_ref() == Some(&member_key))
                && admitted_at <= r.until
        })
    }
    pub fn insert(&mut self, scope: &str, member: Option<&str>, now: u64) -> Result<(), WorkError> {
        if scope.is_empty()
            || scope.len() > 256
            || scope.contains('\u{1f}')
            || member.is_some_and(|m| m.is_empty() || m.len() > 256 || m.contains('\u{1f}'))
        {
            return Err(WorkError::Invalid);
        }
        let until = now.checked_add(RETENTION_SECS).ok_or(WorkError::Full)?;
        let key = subject(scope, member);
        if let Some(r) = self.rows.iter_mut().find(|r| r.subject == key) {
            if now > r.at {
                r.at = now;
                r.until = until;
            }
            return Ok(());
        }
        if self.rows.len() >= MAX_TOMBSTONES {
            return Err(WorkError::Full);
        }
        self.rows.push(Tombstone {
            subject: key,
            reactor: member.map(|m| reactor(scope, super::addenda::member_hash(m))),
            at: now,
            until,
        });
        self.rows.sort_by(|a, b| a.subject.cmp(&b.subject));
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LearningEraseReport {
    pub pending: usize,
    pub reactions: usize,
    pub social: usize,
    pub style: usize,
    pub continuity: usize,
    pub engagement: usize,
    pub aggregate_reset: bool,
}
impl LearningEraseReport {
    pub fn render(&self) -> String {
        format!(
            "Removed linkable learning records in this scope: {} pending replies, {} reactions, {} reputation records/events, {} style records, {} continuity cards, {} engagement records.\n{}\nStored facts, server settings and global contact limits, timezone and stops are preserved. Minimized member-linked contact and project-linked safety counters remain until their protected budget windows expire. Bounded erasure and recomputable replay-safety markers remain to reject old callbacks and repeated requests. Late learning callbacks are blocked for at least 300 seconds. New learning can resume afterward when enabled.",
            self.pending,
            self.reactions,
            self.social,
            self.style,
            self.continuity,
            self.engagement,
            if self.aggregate_reset {
                "Scoped learning reset also discarded learned policy weights, replay, aggregate statistics and consumed style influence."
            } else {
                "Individual erasure cannot undo your contribution to aggregate policy weights or already-consumed style support. A server manager must separately confirm a scoped learning reset to remove that aggregate influence."
            }
        )
    }
}

pub struct LearningEraseState<'a> {
    pub stores: &'a mut Stores,
    pub rewards: &'a mut RewardCollector,
    pub social: &'a mut SocialBrain,
}
/// Discord native member controls call this with their authenticated snowflake.
pub fn erase_member(
    state: &mut LearningEraseState<'_>,
    scope: &str,
    member: u64,
    now: u64,
) -> Result<LearningEraseReport, WorkError> {
    if member == 0 {
        return Err(WorkError::Invalid);
    }
    erase(state, scope, Some(&format!("discord:{member}")), now)
}
pub fn reset_scope(
    state: &mut LearningEraseState<'_>,
    scope: &str,
    confirmed_manager: bool,
    now: u64,
) -> Result<LearningEraseReport, WorkError> {
    if !confirmed_manager {
        return Err(WorkError::Denied);
    }
    erase(state, scope, None, now)
}
fn erase(
    state: &mut LearningEraseState<'_>,
    scope: &str,
    member: Option<&str>,
    now: u64,
) -> Result<LearningEraseReport, WorkError> {
    // Allocate protection first. Saturation refuses the entire removal, never
    // evicts an unexpired protection and never produces an unprotected success.
    let mut ledger = state.rewards.export_recovery().erasure;
    ledger.insert(scope, member, now)?;
    let mut engagement_state = state.stores.work.engagement.clone();
    let engagement = engagement_state.erase_learning(
        scope,
        member
            .and_then(|m| m.strip_prefix("discord:"))
            .and_then(|m| m.parse().ok()),
    )?;
    let (pending, reactions) = state.rewards.erase_learning(scope, member, ledger);
    let stores = &mut state.stores;
    let before = stores.reputations.len() + stores.events.len();
    stores.reputations.retain(|key, _| {
        let Some((g, u)) = key.split_once('\u{1f}') else {
            return true;
        };
        g != scope || member.is_some_and(|m| m != u)
    });
    stores
        .events
        .retain(|e| e.guild_id != scope || member.is_some_and(|m| m != e.user_id));
    state.social.erase(scope, member);
    let social = before - stores.reputations.len() - stores.events.len();
    let style = match member {
        Some(m) => stores
            .addenda
            .get_mut(scope)
            .map_or(0, |l| l.erase_member(&format!("{scope}\u{1f}{m}"))),
        None => stores.addenda.remove(scope).map_or(0, |l| l.record_count()),
    };
    stores.work.engagement = engagement_state;
    if member.is_none() {
        stores.brains.remove(scope);
    }
    stores.pending_rewards = state.rewards.export_pending();
    stores.reward_recovery = state.rewards.export_recovery();
    Ok(LearningEraseReport {
        pending,
        reactions,
        social,
        style,
        continuity: 0,
        engagement,
        aggregate_reset: member.is_none(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn erasure_digest_cutoff_is_exact_bounded_monotonic_and_private() {
        let mut ledger = ErasureLedger::default();
        ledger.insert("discord:1", Some("discord:7"), 100).unwrap();
        ledger.insert("discord:1", Some("discord:7"), 99).unwrap();
        assert!(ledger.blocks("discord:1", "discord:7", 400));
        assert!(!ledger.blocks("discord:1", "discord:7", 401));
        assert!(!ledger.blocks("discord:1", "discord:8", 100));
        assert!(!ledger.blocks("discord:3", "discord:7", 100));
        let json = serde_json::to_string(&ledger).unwrap();
        assert!(!json.contains("discord"));
        let back: ErasureLedger = serde_json::from_str(&json).unwrap();
        back.validate().unwrap();
        assert!(back.blocks("discord:1", "discord:7", 100));
        for i in 1..MAX_TOMBSTONES {
            ledger
                .insert("discord:3", Some(&format!("discord:{i}")), 100)
                .unwrap();
        }
        let before = ledger.clone();
        assert_eq!(
            ledger.insert("discord:9", None, 10000),
            Err(WorkError::Full)
        );
        assert_eq!(ledger, before);
        ledger
            .insert("discord:1", Some("discord:7"), 20000)
            .unwrap();
        assert_eq!(ledger.rows.len(), MAX_TOMBSTONES);
        assert!(ledger.blocks("discord:1", "discord:7", 20300));
        assert_eq!(
            ledger.insert("discord:1", None, u64::MAX),
            Err(WorkError::Full)
        );
    }
}

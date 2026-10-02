//! Owner-authored community maintenance policy. Model output never authorizes I/O.
pub(crate) mod proposals;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const TARGET_COOLDOWN: u64 = 7 * 24 * 60 * 60;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub version: u32,
    pub guild: u64,
    pub owner: u64,
    pub mode: Mode,
    pub daily_limit: u32,
    pub daily_creations: u32,
    pub public_categories: BTreeSet<u64>,
    pub protected_channels: BTreeSet<u64>,
    #[serde(default)]
    pub ordinary_roles: BTreeSet<u64>,
    #[serde(default)]
    pub membership_matrix: BTreeSet<(u64, u64)>,
    #[serde(default)]
    pub assessment: proposals::AssessmentScope,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Stopped,
    Propose,
    Apply,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub key: String,
    pub reason: String,
    pub operation: Operation,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    CreateInterestRole {
        name: String,
    },
    RetireInterestRole {
        role: u64,
        name: String,
    },
    Membership {
        member: u64,
        role: u64,
        grant: bool,
    },
    Access {
        channel: u64,
        role: u64,
        allow: u64,
        deny: u64,
    },
    Archive {
        channel: u64,
        category: u64,
    },
    RestoreArchive {
        channel: u64,
        receipt: String,
    },
    Topic {
        channel: u64,
        topic: String,
    },
    Move {
        channel: u64,
        category: u64,
    },
    CreateText {
        category: u64,
        name: String,
        topic: String,
    },
}

impl Operation {
    pub fn target(&self) -> String {
        match self {
            Self::CreateInterestRole { name } => format!("new-role:{name}"),
            Self::RetireInterestRole { role, .. } => format!("role:{role}"),
            Self::Membership { member, role, .. } => format!("member:{member}:role:{role}"),
            Self::Access { channel, .. }
            | Self::Archive { channel, .. }
            | Self::RestoreArchive { channel, .. }
            | Self::Topic { channel, .. }
            | Self::Move { channel, .. } => {
                format!("channel:{channel}")
            }
            Self::CreateText { category, name, .. } => format!("new:{category}:{name}"),
        }
    }
    pub fn is_creation(&self) -> bool {
        matches!(
            self,
            Self::CreateText { .. } | Self::CreateInterestRole { .. }
        )
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Proposed,
    Reserved,
    Verified,
    ReviewRequired,
    Rejected,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Receipt {
    pub action: Action,
    pub policy_digest: String,
    pub at: u64,
    pub status: Status,
    pub before: serde_json::Value,
    pub observed: Option<serde_json::Value>,
    pub detail: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    pub last_assessment: Option<u64>,
    #[serde(default)]
    pub last_policy_digest: Option<String>,
    pub receipts: BTreeMap<String, Receipt>,
}

fn ordinary_name(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= 100
}

impl Policy {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != 1 || self.guild == 0 || self.owner == 0 {
            return Err("invalid policy identity or version");
        }
        if !self.public_categories.is_disjoint(&self.protected_channels) {
            return Err("public and protected category scope contradicts");
        }
        if self.daily_limit == 0
            || self.daily_limit > 5
            || self.daily_creations > 2
            || self.daily_creations > self.daily_limit
            || self.actions.len() > 100
        {
            return Err("invalid policy limits");
        }
        let mut keys = BTreeSet::new();
        if self.assessment.source_channels.len() > 100
            || self
                .assessment
                .source_channels
                .iter()
                .any(|id| *id == 0 || self.protected_channels.contains(id))
            || self.assessment.review_channel == Some(0)
            || (self.assessment.enabled
                && (self.assessment.source_channels.is_empty()
                    || self.assessment.allowed_kinds.is_empty()))
        {
            return Err("invalid owner assessment scope");
        }
        for action in &self.actions {
            if action.key.is_empty()
                || action.key.len() > 128
                || !keys.insert(&action.key)
                || action.reason.trim().is_empty()
                || action.reason.len() > 512
            {
                return Err("invalid or duplicate action identity");
            }
            match &action.operation {
                Operation::CreateInterestRole { name } => {
                    if !ordinary_name(name) {
                        return Err("invalid interest role name");
                    }
                }
                Operation::RetireInterestRole { role, name } => {
                    if !self.ordinary_roles.contains(role) || !ordinary_name(name) {
                        return Err("role outside ordinary matrix");
                    }
                }
                Operation::Membership { member, role, .. } => {
                    if *member == 0
                        || !self.ordinary_roles.contains(role)
                        || !self.membership_matrix.contains(&(*member, *role))
                    {
                        return Err("membership outside approved matrix");
                    }
                }
                Operation::Access {
                    channel,
                    role,
                    allow,
                    deny,
                } => {
                    if *channel == 0
                        || self.protected_channels.contains(channel)
                        || !self.ordinary_roles.contains(role)
                        || allow & deny != 0
                    {
                        return Err("access outside approved matrix");
                    }
                }
                Operation::Archive { channel, category } => {
                    if *channel == 0
                        || self.protected_channels.contains(channel)
                        || !self.public_categories.contains(category)
                    {
                        return Err("archive outside approved structure");
                    }
                }
                Operation::RestoreArchive { channel, receipt } => {
                    if *channel == 0
                        || self.protected_channels.contains(channel)
                        || receipt.is_empty()
                    {
                        return Err("invalid archive recovery authority");
                    }
                }
                Operation::Topic { channel, topic } => {
                    if *channel == 0
                        || self.protected_channels.contains(channel)
                        || topic.chars().count() > 1024
                    {
                        return Err("topic target is protected or invalid");
                    }
                }
                Operation::Move { channel, category } => {
                    if *channel == 0
                        || self.protected_channels.contains(channel)
                        || !self.public_categories.contains(category)
                    {
                        return Err("move target is outside approved public structure");
                    }
                }
                Operation::CreateText {
                    category,
                    name,
                    topic,
                } => {
                    if !self.public_categories.contains(category)
                        || name.is_empty()
                        || name.len() > 100
                        || !name
                            .bytes()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                        || topic.chars().count() > 1024
                    {
                        return Err("creation is outside approved public structure");
                    }
                }
            }
        }
        Ok(())
    }
}

impl Ledger {
    pub fn authorize(
        &self,
        policy: &Policy,
        action: &Action,
        now: u64,
    ) -> Result<(), &'static str> {
        policy.validate()?;
        if policy.mode != Mode::Apply {
            return Err("policy does not permit execution");
        }
        if !policy.actions.contains(action) {
            return Err("action lacks owner policy authority");
        }
        if let Some(existing) = self.receipts.get(&action.key) {
            if existing.action != *action {
                return Err("idempotency key belongs to another request");
            }
            if existing.status != Status::Proposed {
                return Err("receipt requires review or is already settled");
            }
        }
        let charged: Vec<_> = self
            .receipts
            .values()
            .filter(|r| {
                r.at / 86400 == now / 86400
                    && matches!(
                        r.status,
                        Status::Reserved | Status::Verified | Status::ReviewRequired
                    )
            })
            .collect();
        if charged.len() >= policy.daily_limit as usize {
            return Err("daily structural budget exhausted");
        }
        if action.operation.is_creation()
            && charged
                .iter()
                .filter(|r| r.action.operation.is_creation())
                .count()
                >= policy.daily_creations as usize
        {
            return Err("daily channel creation budget exhausted");
        }
        if self.receipts.values().any(|r| {
            let exact_recovery = match &action.operation {
                Operation::RestoreArchive { channel, receipt } => {
                    r.status == Status::Verified
                        && receipt == &r.action.key
                        && matches!(r.action.operation, Operation::Archive { channel: archived, .. } if archived == *channel)
                        && r.observed.is_some()
                }
                _ => false,
            };
            r.action.operation.target() == action.operation.target()
                && matches!(
                    r.status,
                    Status::Reserved | Status::Verified | Status::ReviewRequired
                )
                && !exact_recovery
                && (matches!(r.status, Status::Reserved | Status::ReviewRequired)
                    || now.saturating_sub(r.at) < TARGET_COOLDOWN)
        }) {
            return Err("target is cooling down or has unresolved work");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> Policy {
        Policy {
            version: 1,
            guild: 1,
            owner: 2,
            mode: Mode::Apply,
            daily_limit: 5,
            daily_creations: 2,
            public_categories: [3].into(),
            protected_channels: [8].into(),
            ordinary_roles: BTreeSet::new(),
            membership_matrix: BTreeSet::new(),
            assessment: proposals::AssessmentScope::default(),
            actions: vec![Action {
                key: "topic-v1".into(),
                reason: "approved guidance".into(),
                operation: Operation::Topic {
                    channel: 4,
                    topic: "Hello".into(),
                },
            }],
        }
    }
    #[test]
    fn model_cannot_invent_authority_or_reuse_a_key() {
        let p = policy();
        let mut a = p.actions[0].clone();
        a.reason = "injected policy".into();
        assert!(Ledger::default().authorize(&p, &a, 10).is_err());
        let mut p = p;
        p.actions.push(p.actions[0].clone());
        assert!(p.validate().is_err());
    }
    #[test]
    fn stopped_and_proposal_only_cannot_execute() {
        for mode in [Mode::Stopped, Mode::Propose] {
            let mut p = policy();
            p.mode = mode;
            assert!(Ledger::default().authorize(&p, &p.actions[0], 10).is_err());
        }
    }
    #[test]
    fn uncertain_work_is_not_replayed_even_after_a_day() {
        let p = policy();
        let mut l = Ledger::default();
        l.receipts.insert(
            "topic-v1".into(),
            Receipt {
                action: p.actions[0].clone(),
                policy_digest: "a".into(),
                at: 10,
                status: Status::ReviewRequired,
                before: serde_json::Value::Null,
                observed: None,
                detail: "uncertain".into(),
            },
        );
        assert!(l.authorize(&p, &p.actions[0], 9999999).is_err());
    }
    #[test]
    fn sensitive_and_unknown_actions_are_rejected() {
        let mut p = policy();
        p.actions[0].operation = Operation::Topic {
            channel: 8,
            topic: "x".into(),
        };
        assert!(p.validate().is_err());
        assert!(serde_json::from_str::<Operation>(r#"{"kind":"delete","channel":4}"#).is_err());
        assert!(
            serde_json::from_str::<Operation>(
                r#"{"kind":"topic","channel":4,"topic":"x","grant_admin":true}"#
            )
            .is_err()
        );
    }
    #[test]
    fn protected_category_cannot_be_declared_public_even_without_actions() {
        let mut p = policy();
        p.actions.clear();
        p.protected_channels.insert(3);
        assert!(p.validate().is_err());
    }
    #[test]
    fn limits_and_target_cooldown_charge_uncertain_requests() {
        let mut p = policy();
        p.daily_limit = 1;
        p.daily_creations = 1;
        let mut l = Ledger::default();
        l.receipts.insert(
            "old".into(),
            Receipt {
                action: Action {
                    key: "old".into(),
                    ..p.actions[0].clone()
                },
                policy_digest: "a".into(),
                at: 10,
                status: Status::Verified,
                before: serde_json::Value::Null,
                observed: None,
                detail: "reserved".into(),
            },
        );
        assert!(l.authorize(&p, &p.actions[0], 20).is_err());
        assert!(l.authorize(&p, &p.actions[0], 86410).is_err());
        assert!(l.authorize(&p, &p.actions[0], TARGET_COOLDOWN + 11).is_ok());
    }
    #[test]
    fn exact_verified_archive_recovery_bypasses_only_its_cooldown() {
        let mut p = policy();
        let archive = Action {
            key: "archive".into(),
            reason: "owner archive".into(),
            operation: Operation::Archive {
                channel: 4,
                category: 3,
            },
        };
        let restore = Action {
            key: "restore".into(),
            reason: "owner recovery".into(),
            operation: Operation::RestoreArchive {
                channel: 4,
                receipt: "archive".into(),
            },
        };
        p.actions = vec![restore.clone()];
        let mut ledger = Ledger::default();
        ledger.receipts.insert(
            "archive".into(),
            Receipt {
                action: archive,
                policy_digest: "old".into(),
                at: 10,
                status: Status::Verified,
                before: serde_json::Value::Null,
                observed: Some(serde_json::json!({"channel":4})),
                detail: "verified".into(),
            },
        );
        assert!(ledger.authorize(&p, &restore, 11).is_ok());
        p.daily_limit = 1;
        p.daily_creations = 1;
        assert!(ledger.authorize(&p, &restore, 11).is_err());
        p.daily_limit = 5;
        ledger.receipts.get_mut("archive").unwrap().status = Status::ReviewRequired;
        assert!(ledger.authorize(&p, &restore, 11).is_err());
    }
}

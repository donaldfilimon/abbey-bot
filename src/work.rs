//! Pure chief-of-staff work records. The Discord shell supplies current
//! permission facts and time; this module neither reads a clock nor performs I/O.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkScope {
    Personal { owner: u64 },
    Team { guild: u64, channel: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkProject {
    pub id: u64,
    pub name: String,
    pub scope: WorkScope,
    pub managers: BTreeSet<u64>,
    pub members: BTreeSet<u64>,
    pub revision: u64,
    #[serde(default)]
    pub allowed_github_repositories: BTreeSet<GitHubRepository>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GitHubRepository {
    pub installation: u64,
    pub owner: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitHubItemKind {
    Issue,
    PullRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubReference {
    pub repository: GitHubRepository,
    pub kind: GitHubItemKind,
    pub number: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubSnapshot {
    pub title: String,
    pub state: GitHubState,
    pub refreshed_at: u64,
    pub stale: bool,
    pub etag: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitHubState {
    Open,
    Closed,
    Merged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkStatus {
    Open,
    InProgress,
    Blocked,
    Done,
    Cancelled,
}

impl WorkStatus {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InProgress => "in progress",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkGoal {
    pub id: u64,
    pub project_id: u64,
    pub title: String,
    pub owner: u64,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkTask {
    pub id: u64,
    pub project_id: u64,
    pub title: String,
    pub owner: u64,
    pub assignee: Option<u64>,
    pub goal_id: Option<u64>,
    pub priority: u8,
    pub status: WorkStatus,
    pub due_at: Option<u64>,
    /// Explicit UTC seconds. A deadline alone never requests a notification.
    #[serde(default)]
    pub remind_at: Option<u64>,
    #[serde(default)]
    pub reminder_revision: u64,
    pub snoozed_until: Option<u64>,
    pub source: Option<String>,
    #[serde(default)]
    pub github: Option<GitHubReference>,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkDecision {
    pub id: u64,
    pub project_id: u64,
    pub author: u64,
    pub text: String,
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkAutomationPolicy {
    pub enabled: bool,
    pub destination: Option<u64>,
    pub timezone: String,
    pub quiet_start: u8,
    pub quiet_end: u8,
    pub daily_limit: u8,
    pub briefing_hour: u8,
}

impl Default for WorkAutomationPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            destination: None,
            timezone: String::new(),
            quiet_start: 22,
            quiet_end: 8,
            daily_limit: 4,
            briefing_hour: 9,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkFeedback {
    Useful,
    Dismissed,
    Snoozed { hour: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreferenceEvidence {
    /// Missing on legacy rows: these observations never train preferences.
    #[serde(default)]
    pub actor: Option<u64>,
    #[serde(default)]
    pub scope: Option<WorkScope>,
    #[serde(default)]
    pub kind: Option<WorkDeliveryKind>,
    pub delivery_id: u64,
    pub feedback: WorkFeedback,
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkPreferenceProfile {
    pub learning_enabled: bool,
    pub explicit_hour: Option<u8>,
    pub learned_hour: Option<u8>,
    pub evidence: Vec<PreferenceEvidence>,
    #[serde(default)]
    pub reduce_followups: bool,
    #[serde(default)]
    pub briefing_rank: i8,
}

impl Default for WorkPreferenceProfile {
    fn default() -> Self {
        Self {
            learning_enabled: true,
            explicit_hour: None,
            learned_hour: None,
            evidence: Vec::new(),
            reduce_followups: false,
            briefing_rank: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryState {
    Attempting,
    Sent,
    ReviewRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkDeliveryReceipt {
    pub id: u64,
    pub project_id: u64,
    pub recipient: u64,
    pub local_day: String,
    pub at: u64,
    pub state: DeliveryState,
    pub message_id: Option<u64>,
    /// None identifies a legacy project-scoped receipt.
    #[serde(default)]
    pub scope: Option<WorkScope>,
    #[serde(default)]
    pub kind: Option<WorkDeliveryKind>,
    #[serde(default)]
    pub coverage: Vec<ReminderCoverage>,
    #[serde(default)]
    pub task_ids: Vec<u64>,
    #[serde(default)]
    pub dedupe_keys: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkDeliveryKind {
    Briefing,
    Reminder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderCoverage {
    pub task_id: u64,
    pub reminder_revision: u64,
    pub remind_at: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkStore {
    pub sequence: u64,
    pub projects: BTreeMap<u64, WorkProject>,
    pub goals: BTreeMap<u64, WorkGoal>,
    pub tasks: BTreeMap<u64, WorkTask>,
    pub decisions: BTreeMap<u64, WorkDecision>,
    /// Legacy project policies are retained for loading only; never opt a scope in.
    pub automation: BTreeMap<u64, WorkAutomationPolicy>,
    /// Canonical WorkScope key -> explicitly configured policy.
    pub scope_automation: BTreeMap<String, WorkAutomationPolicy>,
    /// Explicit configuring principal; missing legacy grants never authorize sends.
    pub scope_automation_actors: BTreeMap<String, u64>,
    pub preferences: BTreeMap<String, WorkPreferenceProfile>,
    pub deliveries: BTreeMap<u64, WorkDeliveryReceipt>,
    pub request_ids: BTreeMap<String, u64>,
    pub actions: crate::action_approval::ActionStore,
    pub github_snapshots: BTreeMap<String, GitHubSnapshot>,
}

/// Fresh permission facts from Discord REST. Cached or model-inferred facts
/// must never construct this value.
#[derive(Debug, Clone, Copy)]
pub struct WorkAccess {
    pub actor: u64,
    pub guild: Option<u64>,
    pub channel: u64,
    pub can_view: bool,
    pub can_manage: bool,
}

impl WorkAccess {
    #[cfg(test)]
    pub fn preference_key(self) -> String {
        match self.guild {
            Some(guild) => format!("team:{guild}:{}:{}", self.channel, self.actor),
            None => format!("personal:{}", self.actor),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkError {
    Denied,
    Missing,
    Invalid,
    Full,
    Stale,
    Persistence,
}

impl std::fmt::Display for WorkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Denied => "This workspace is not available to you in this channel.",
            Self::Missing => "That work item is unavailable.",
            Self::Invalid => "Check the supplied work item fields.",
            Self::Full => "This workspace has reached its record limit.",
            Self::Stale => "This item changed. Refresh it before acting.",
            Self::Persistence => "The change could not be durably saved. Refresh before retrying.",
        })
    }
}

impl std::error::Error for WorkError {}

mod github;
mod policy;
pub use policy::initial_timezone;
mod registry;
mod schedule;

#[cfg(test)]
mod tests;

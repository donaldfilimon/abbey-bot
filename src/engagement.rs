//! Pure bounded member contact records. Callers own time, authorization and I/O.
use crate::work::WorkError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Stable string wire keys also support JSON maps keyed by origin scope.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum EngagementScope {
    Guild { guild: u64, channel: u64 },
    Dm { member: u64, channel: u64 },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DestinationPreference {
    Origin,
    Private,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklySubscription {
    pub weekday: u8,
    pub hour: u8,
    pub scope: EngagementScope,
    pub destination: DestinationPreference,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MemberPolicy {
    pub revision: u64,
    pub daily_limit: Option<u8>,
    pub weekly_limit: Option<u8>,
    pub timezone: Option<String>,
    pub quiet_start: u8,
    pub quiet_end: u8,
    pub global_stop: bool,
    pub stopped_guilds: BTreeSet<u64>,
    pub stopped_scopes: BTreeSet<EngagementScope>,
    pub snoozed_until: Option<u64>,
    pub weekly_subscription: Option<WeeklySubscription>,
    pub destinations: BTreeMap<EngagementScope, DestinationPreference>,
}
impl Default for MemberPolicy {
    fn default() -> Self {
        Self {
            revision: 0,
            daily_limit: None,
            weekly_limit: None,
            timezone: None,
            quiet_start: 22,
            quiet_end: 8,
            global_stop: false,
            stopped_scopes: BTreeSet::new(),
            stopped_guilds: BTreeSet::new(),
            snoozed_until: None,
            weekly_subscription: None,
            destinations: BTreeMap::new(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    pub scope: EngagementScope,
    pub message: u64,
    pub author: u64,
    pub revision: u64,
    pub at: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EngagementKind {
    FollowUp,
    WeeklyCheckIn,
    ActivityInvite,
    VoiceInvite,
    ConversationStarter,
    UnansweredQuestion,
    Welcome,
    ProjectCheckIn,
    Introduction,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CommunityFeature {
    Starters,
    Questions,
    Welcomes,
    Projects,
    Introductions,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GuildFeaturePolicy {
    pub revision: u64,
    pub enabled: BTreeSet<CommunityFeature>,
    pub channels: BTreeMap<CommunityFeature, BTreeSet<u64>>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntroductionState {
    Pending,
    Ready,
    Consumed,
    Cancelled,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Introduction {
    pub id: u64,
    pub revision: u64,
    pub scope: EngagementScope,
    pub members: [u64; 2],
    pub approved_self_descriptions: [Option<String>; 2],
    pub approvals: [Option<u64>; 2],
    pub destination: u64,
    pub state: IntroductionState,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CandidateState {
    Pending,
    Reserved,
    Sent,
    Cancelled,
    Rejected,
    ReviewRequired,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub id: u64,
    pub kind: EngagementKind,
    pub source: Option<SourceRef>,
    pub member: Option<u64>,
    pub scope: EngagementScope,
    pub due_at: u64,
    pub revision: u64,
    pub state: CandidateState,
    pub dedupe_key: String,
    pub policy_revision: u64,
    pub destination: DestinationPreference,
    pub message_id: Option<u64>,
    pub introduction_id: Option<u64>,
    #[serde(default)]
    pub work_ref: Option<crate::work::WorkContentRef>,
    #[serde(default)]
    pub expires_at: Option<u64>,
    #[serde(default)]
    pub follow_up_reason: Option<crate::work::follow_up::FollowUpDecision>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContactCharge {
    pub candidate_id: u64,
    pub member: u64,
    pub local_day: String,
    pub local_week: String,
    pub at: u64,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EngagementStore {
    pub community_cursor: Option<SourceRef>,
    pub community_receipts: BTreeMap<u64, community::CommunityReceipt>,
    pub invitation_requests: BTreeMap<u64, InvitationRequest>,
    pub suppressed_invitation_requests: BTreeMap<u64, SuppressedInvitationRequest>,
    pub feedback: BTreeMap<u64, BTreeMap<u64, ExplicitFeedback>>,
    pub erased_identities: BTreeSet<String>,
    pub safety_pruned_through: u64,
    pub sequence: u64,
    pub member_policies: BTreeMap<u64, MemberPolicy>,
    /// Direct-interaction source identities only; never inferred from memory.
    pub weekly_assessments: BTreeMap<u64, u64>,
    pub responses: BTreeMap<u64, u64>,
    pub observations: BTreeMap<EngagementScope, BTreeMap<u64, SourceRef>>,
    pub eligibility: BTreeMap<u64, BTreeSet<SourceRef>>,
    pub guild_features: BTreeMap<u64, GuildFeaturePolicy>,
    pub candidates: BTreeMap<u64, Candidate>,
    pub introductions: BTreeMap<u64, Introduction>,
    pub charges: Vec<ContactCharge>,
    /// Minimized linkable safety accounting retained after learning erasure.
    pub erased_contact_charges: Vec<erasure::ErasedContactCharge>,
    pub erased_community_charges: Vec<erasure::ErasedCommunityCharge>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvitationRequest {
    #[serde(default)]
    pub activity: Option<ActivityVersion>,
    pub interaction: u64,
    pub member: u64,
    pub scope: EngagementScope,
    pub at: u64,
}
pub mod classifier;
pub mod community;
mod erasure;
mod erasure_identity;
pub mod introductions;
pub mod invitations;
pub mod lifecycle;
mod loading;
mod task_follow_up;
mod task_receipt_status;
pub(crate) use task_receipt_status::TaskReceiptAggregate;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuppressedInvitationRequest {
    pub blocking_candidate: u64,
    pub kind: EngagementKind,
    pub request: InvitationRequest,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityVersion {
    pub origin: String,
    pub digest: String,
}
pub mod readiness;

mod policy;
pub mod schedule;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedbackKind {
    Useful,
    Dismissed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplicitFeedback {
    pub actor: u64,
    pub at: u64,
    pub kind: FeedbackKind,
}

//! Validated canonical loading; unfinished sends are consumed, never replayed.
use super::*;
#[derive(Default, Deserialize)]
#[serde(remote = "EngagementStore", default, deny_unknown_fields)]
struct LoadedEngagementStore {
    community_cursor: Option<SourceRef>,
    community_receipts: BTreeMap<u64, community::CommunityReceipt>,
    invitation_requests: BTreeMap<u64, InvitationRequest>,
    suppressed_invitation_requests: BTreeMap<u64, SuppressedInvitationRequest>,
    feedback: BTreeMap<u64, BTreeMap<u64, ExplicitFeedback>>,
    sequence: u64,
    member_policies: BTreeMap<u64, MemberPolicy>,
    weekly_assessments: BTreeMap<u64, u64>,
    responses: BTreeMap<u64, u64>,
    observations: BTreeMap<EngagementScope, BTreeMap<u64, SourceRef>>,
    eligibility: BTreeMap<u64, BTreeSet<SourceRef>>,
    guild_features: BTreeMap<u64, GuildFeaturePolicy>,
    candidates: BTreeMap<u64, Candidate>,
    introductions: BTreeMap<u64, Introduction>,
    charges: Vec<ContactCharge>,
}
impl<'de> Deserialize<'de> for EngagementStore {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut store = LoadedEngagementStore::deserialize(deserializer)?;
        store.validate().map_err(serde::de::Error::custom)?;
        store.recover_reserved();
        Ok(store)
    }
}

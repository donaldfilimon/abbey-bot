//! Bounded human-assessed operational cases. No classifier, transport or sanctions.
//! Serialized evidence never becomes a capability; fresh native authority is required.
use super::contextual::{self, Assessment, Input};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

mod model;
pub use model::*;

pub const MAX_CASES: usize = 1_000;
const PROOF_TTL_SECONDS: u64 = 60;

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ShadowScope {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub source_channels: BTreeSet<u64>,
    #[serde(default)]
    pub review_channel: Option<u64>,
}
impl ShadowScope {
    pub fn is_disabled_default(&self) -> bool {
        !self.enabled && self.source_channels.is_empty() && self.review_channel.is_none()
    }
    pub fn validate(&self, protected: &BTreeSet<u64>) -> Result<(), &'static str> {
        if self.source_channels.len() > 100
            || self.source_channels.contains(&0)
            || !self.source_channels.is_disjoint(protected)
            || self.review_channel == Some(0)
            || self.enabled && self.source_channels.is_empty()
        {
            return Err("invalid contextual shadow scope");
        }
        Ok(())
    }
}

/// A pure projection of the current owner-authored operations policy.
#[derive(Debug, Clone)]
pub struct ShadowPolicy {
    pub guild: u64,
    pub owner: u64,
    pub stopped: bool,
    pub scope: ShadowScope,
}
impl ShadowPolicy {
    fn validate(&self) -> Result<(), &'static str> {
        if self.guild == 0 || self.owner == 0 {
            return Err("invalid contextual policy identity");
        }
        self.scope.validate(&BTreeSet::new())
    }
    fn scope_digest(&self) -> Result<String, &'static str> {
        digest(&(self.guild, self.owner, &self.scope))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceVersion {
    pub guild: u64,
    pub channel: u64,
    pub message: u64,
    pub author: u64,
    pub created_at: u64,
    pub edited_at: Option<u64>,
    pub evidence_digest: String,
}
impl SourceVersion {
    /// The shell supplies exact native IDs/timestamps and current message text.
    /// Only the scoped digest survives this call; content is never retained.
    pub fn capture(
        guild: u64,
        channel: u64,
        message: u64,
        author: u64,
        created_at: u64,
        edited_at: Option<u64>,
        content: &str,
    ) -> Result<Self, &'static str> {
        if content.trim().is_empty() || content.len() > 16_000 {
            return Err("contextual source content unavailable or exceeds bound");
        }
        let source = Self {
            guild,
            channel,
            message,
            author,
            created_at,
            edited_at,
            evidence_digest: digest(&(
                "abbey-contextual-source-v1",
                guild,
                channel,
                message,
                author,
                created_at,
                edited_at,
                content,
            ))?,
        };
        source.validate()?;
        Ok(source)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if [
            self.guild,
            self.channel,
            self.message,
            self.author,
            self.created_at,
        ]
        .contains(&0)
            || self.edited_at.is_some_and(|at| at < self.created_at)
            || !is_digest(&self.evidence_digest)
        {
            return Err("invalid contextual source attribution");
        }
        Ok(())
    }
    pub fn case_id(&self) -> Result<String, &'static str> {
        self.validate()?;
        digest(&("abbey-contextual-case-v1", self))
    }
}

/// Native facts are never deserialized. Their producer must positively match
/// returned guild/member/channel/role IDs and recompute overwrites over REST.
#[derive(Debug, Clone)]
pub struct NativeAuthorityFacts {
    pub guild: u64,
    pub owner: u64,
    pub actor: u64,
    pub origin: u64,
    pub at: u64,
    pub current_member: bool,
    pub can_view_origin: bool,
    pub can_view_source: bool,
    pub complete_permissions: bool,
    pub can_delete: bool,
    pub can_timeout: bool,
    pub observed_source: Option<SourceVersion>,
}
#[derive(Debug, Clone)]
pub struct FreshAuthority {
    facts: NativeAuthorityFacts,
}
impl FreshAuthority {
    /// Check native membership/view, current owner identity and proof age before
    /// an infrastructure snapshot. Exact case/staff/subject checks still follow.
    pub fn authorize_snapshot(&self, policy: &ShadowPolicy, now: u64) -> Result<(), &'static str> {
        self.check(policy, now)
    }
    pub fn verified(facts: NativeAuthorityFacts) -> Result<Self, &'static str> {
        if [
            facts.guild,
            facts.owner,
            facts.actor,
            facts.origin,
            facts.at,
        ]
        .contains(&0)
            || !facts.current_member
            || !facts.can_view_origin
            || !facts.complete_permissions
        {
            return Err("current contextual access is unproved");
        }
        if let Some(source) = &facts.observed_source {
            source.validate()?;
            if !facts.can_view_source
                || source.guild != facts.guild
                || source.created_at > facts.at
                || source.edited_at.is_some_and(|at| at > facts.at)
            {
                return Err("contextual source guild mismatch");
            }
        }
        Ok(Self { facts })
    }
    fn check(&self, policy: &ShadowPolicy, now: u64) -> Result<(), &'static str> {
        policy.validate()?;
        if self.facts.guild != policy.guild
            || self.facts.owner != policy.owner
            || self.facts.at > now
            || now - self.facts.at > PROOF_TTL_SECONDS
        {
            return Err("contextual authority is stale or mismatched");
        }
        Ok(())
    }
    fn staff(&self, origin: u64) -> Result<(), &'static str> {
        if self.facts.origin != origin || !self.facts.can_delete || !self.facts.can_timeout {
            return Err("current contextual staff authority is unproved");
        }
        Ok(())
    }
    fn source(&self, source: &SourceVersion) -> bool {
        self.facts.observed_source.as_ref() == Some(source)
    }
}

#[derive(Debug, Clone)]
pub struct NewCase {
    source: SourceVersion,
    input: Input,
    authority: FreshAuthority,
}
impl NewCase {
    fn check(&self, policy: &ShadowPolicy, now: u64) -> Result<(), &'static str> {
        self.authority.check(policy, now)?;
        self.authority.staff(self.source.channel)?;
        if self.source.author == policy.owner
            || self.source.guild != policy.guild
            || self.source.created_at > now
            || self.source.edited_at.is_some_and(|at| at > now)
        {
            return Err("contextual capture attribution is invalid");
        }
        Ok(())
    }
    pub fn human_assessed(
        source: SourceVersion,
        input: Input,
        authority: FreshAuthority,
    ) -> Result<Self, &'static str> {
        source.validate()?;
        if source.guild != input.guild
            || source.channel != input.channel
            || source.message != input.message
            || source.author != input.source_author
            || source.author != input.target
            || !authority.source(&source)
        {
            return Err("contextual source version mismatched");
        }
        // Prove native scope/exclusions/authority even for referral cases;
        // ambiguity does not grant a weaker moderation capability.
        contextual::qualify(Input {
            assessment: Assessment::ConfirmedOffending,
            ..input
        })?;
        if authority.facts.actor == source.author {
            return Err("contextual assessor cannot target herself");
        }
        Ok(Self {
            source,
            input,
            authority,
        })
    }
}
#[derive(Debug, Clone)]
pub struct ExpectedCase {
    pub id: String,
    pub revision: u64,
}
#[derive(Debug, Clone)]
pub enum Mutation {
    Capture(NewCase),
    Review {
        expected: ExpectedCase,
        authority: FreshAuthority,
        decision: ReviewDecision,
    },
    Appeal {
        expected: ExpectedCase,
        authority: FreshAuthority,
        reason: AppealReason,
    },
    ResolveAppeal {
        expected: ExpectedCase,
        authority: FreshAuthority,
        decision: AppealDecision,
    },
}

impl Mutation {
    pub fn authorize_snapshot(&self, policy: &ShadowPolicy, now: u64) -> Result<(), &'static str> {
        match self {
            Self::Capture(new) => new.check(policy, now),
            Self::Review { authority, .. }
            | Self::Appeal { authority, .. }
            | Self::ResolveAppeal { authority, .. } => authority.authorize_snapshot(policy, now),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Changed,
    AlreadyObserved,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CaseStore {
    pub version: u32,
    pub revision: u64,
    pub cases: BTreeMap<String, Case>,
}
impl Default for CaseStore {
    fn default() -> Self {
        Self {
            version: 1,
            revision: 0,
            cases: BTreeMap::new(),
        }
    }
}
impl CaseStore {
    /// A read-only exact retry probe; serialized provenance never grants access.
    pub fn has_existing_capture(
        &self,
        new: &NewCase,
        policy: &ShadowPolicy,
        policy_digest: &str,
        now: u64,
    ) -> Result<bool, &'static str> {
        self.validate()?;
        if !is_digest(policy_digest) {
            return Err("invalid contextual policy digest");
        }
        new.check(policy, now)?;
        let id = new.source.case_id()?;
        if !self.cases.contains_key(&id) {
            return Ok(false);
        }
        let mut candidate = self.clone();
        let (observed, change) =
            candidate.apply(Mutation::Capture(new.clone()), policy, policy_digest, now)?;
        Ok(observed == id && change == Change::AlreadyObserved)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != 1 || self.cases.len() > MAX_CASES {
            return Err("invalid contextual case store");
        }
        let mut events = 0_u64;
        for (key, case) in &self.cases {
            case.validate()?;
            if key != &case.id {
                return Err("contextual case key mismatch");
            }
            events = events
                .checked_add(case.revision)
                .ok_or("contextual revision exhausted")?;
        }
        if events != self.revision {
            return Err("contextual store revision mismatch");
        }
        Ok(())
    }
    pub fn apply(
        &mut self,
        mutation: Mutation,
        policy: &ShadowPolicy,
        policy_digest: &str,
        now: u64,
    ) -> Result<(String, Change), &'static str> {
        let mut next = self.clone();
        let receipt = next.apply_inner(mutation, policy, policy_digest, now)?;
        *self = next;
        Ok(receipt)
    }
    fn apply_inner(
        &mut self,
        mutation: Mutation,
        policy: &ShadowPolicy,
        policy_digest: &str,
        now: u64,
    ) -> Result<(String, Change), &'static str> {
        self.validate()?;
        policy.validate()?;
        if !is_digest(policy_digest) {
            return Err("invalid contextual policy digest");
        }
        let id = match mutation {
            Mutation::Capture(new) => {
                new.check(policy, now)?;
                let id = new.source.case_id()?;
                let assessment = HumanAssessment::from(new.input.assessment);
                let severity = CaseSeverity::from(new.input.severity);
                if let Some(old) = self.cases.get(&id) {
                    if old.captured.owner != policy.owner
                        || old.source != new.source
                        || old.assessment != assessment
                        || old.severity != severity
                    {
                        return Err(
                            "contextual repeated assessment conflicts; independent review required",
                        );
                    }
                    return Ok((id, Change::AlreadyObserved));
                }
                if policy.stopped
                    || !policy.scope.enabled
                    || !policy.scope.source_channels.contains(&new.source.channel)
                {
                    return Err("contextual capture is disabled or outside scope");
                }
                if self.cases.len() == MAX_CASES {
                    return Err("contextual case store is full; owner review required");
                }
                let disposition = if new.input.assessment == Assessment::ConfirmedOffending {
                    Disposition::ConfirmedProposal {
                        timeout_minutes: contextual::qualify(new.input)?.timeout_minutes,
                    }
                } else {
                    Disposition::HumanReview
                };
                self.cases.insert(
                    id.clone(),
                    Case {
                        id: id.clone(),
                        revision: 1,
                        source: new.source,
                        origin: CaseOrigin::HumanModerator,
                        assessment,
                        severity,
                        disposition,
                        captured: Provenance::new(&new.authority, policy, policy_digest, now)?,
                        review: None,
                        appeal: None,
                    },
                );
                id
            }
            mutation => {
                let (expected, authority) = match &mutation {
                    Mutation::Review {
                        expected,
                        authority,
                        ..
                    }
                    | Mutation::Appeal {
                        expected,
                        authority,
                        ..
                    }
                    | Mutation::ResolveAppeal {
                        expected,
                        authority,
                        ..
                    } => (expected, authority),
                    Mutation::Capture(_) => unreachable!(),
                };
                authority.check(policy, now)?;
                let case = self
                    .cases
                    .get_mut(&expected.id)
                    .ok_or("contextual case unavailable")?;
                if case.source.guild != policy.guild || case.captured.owner != policy.owner {
                    return Err("contextual case policy identity mismatch");
                }
                let actor = authority.facts.actor;
                let provenance = Provenance::new(authority, policy, policy_digest, now)?;
                let review_origin = policy.scope.review_channel.unwrap_or(case.source.channel);
                let current_source = authority.source(&case.source);
                let change = match &mutation {
                    Mutation::Review { decision, .. } => {
                        authority.staff(review_origin)?;
                        if [case.source.author, case.captured.actor].contains(&actor) {
                            return Err("contextual review requires an independent moderator");
                        }
                        if *decision != ReviewDecision::NeedsContext && !current_source {
                            return Err("current contextual evidence is unavailable or changed");
                        }
                        if let Some(old) = &case.review {
                            if old.expected_revision != expected.revision
                                || old.decision != *decision
                                || old.provenance.actor != actor
                            {
                                return Err("contextual receipt conflicts; refresh case");
                            }
                            true
                        } else {
                            check_revision(case, expected)?;
                            // A late review cannot invalidate the independence of a resolved appeal.
                            if case
                                .appeal
                                .as_ref()
                                .and_then(|a| a.resolution.as_ref())
                                .is_some_and(|r| r.provenance.actor == actor)
                            {
                                return Err(
                                    "contextual appeal resolver cannot become its reviewer",
                                );
                            }
                            case.review = Some(Review {
                                expected_revision: expected.revision,
                                decision: *decision,
                                provenance,
                            });
                            false
                        }
                    }
                    Mutation::Appeal { reason, .. } => {
                        if actor != case.source.author
                            || authority.facts.origin != case.source.channel
                        {
                            return Err(
                                "only the current case subject may appeal in the source origin",
                            );
                        }
                        if let Some(old) = &case.appeal {
                            if old.expected_revision != expected.revision
                                || old.reason != *reason
                                || old.provenance.actor != actor
                            {
                                return Err("contextual receipt conflicts; refresh case");
                            }
                            true
                        } else {
                            check_revision(case, expected)?;
                            case.appeal = Some(Appeal {
                                expected_revision: expected.revision,
                                reason: *reason,
                                provenance,
                                resolution: None,
                            });
                            false
                        }
                    }
                    Mutation::ResolveAppeal { decision, .. } => {
                        authority.staff(review_origin)?;
                        if [case.source.author, case.captured.actor].contains(&actor)
                            || case
                                .review
                                .as_ref()
                                .is_some_and(|r| r.provenance.actor == actor)
                        {
                            return Err("contextual appeal requires an independent resolver");
                        }
                        if *decision != AppealDecision::NeedsContext && !current_source {
                            return Err(
                                "current contextual appeal evidence unavailable or changed",
                            );
                        }
                        check_revision_or_resolution(case, expected)?;
                        let appeal = case
                            .appeal
                            .as_mut()
                            .ok_or("contextual appeal unavailable")?;
                        if let Some(old) = &appeal.resolution {
                            if old.expected_revision != expected.revision
                                || old.decision != *decision
                                || old.provenance.actor != actor
                            {
                                return Err("contextual receipt conflicts; refresh case");
                            }
                            true
                        } else {
                            appeal.resolution = Some(AppealResolution {
                                expected_revision: expected.revision,
                                decision: *decision,
                                provenance,
                            });
                            false
                        }
                    }
                    Mutation::Capture(_) => unreachable!(),
                };
                if change {
                    return Ok((expected.id.clone(), Change::AlreadyObserved));
                }
                case.revision = case
                    .revision
                    .checked_add(1)
                    .ok_or("contextual revision exhausted")?;
                expected.id.clone()
            }
        };
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("contextual revision exhausted")?;
        self.validate()?;
        Ok((id, Change::Changed))
    }
    pub fn inspect(
        &self,
        id: &str,
        authority: &FreshAuthority,
        policy: &ShadowPolicy,
        now: u64,
    ) -> Result<&Case, &'static str> {
        self.validate()?;
        authority.check(policy, now)?;
        let case = self.cases.get(id).ok_or("contextual case unavailable")?;
        if case.source.guild != policy.guild || case.captured.owner != policy.owner {
            return Err("contextual case unavailable");
        }
        let subject = authority.facts.actor == case.source.author
            && authority.facts.origin == case.source.channel;
        if !subject {
            authority.staff(policy.scope.review_channel.unwrap_or(case.source.channel))?;
        }
        Ok(case)
    }
    pub fn measured_counts(
        &self,
        authority: &FreshAuthority,
        policy: &ShadowPolicy,
        now: u64,
    ) -> Result<Counts, &'static str> {
        self.validate()?;
        authority.check(policy, now)?;
        let origin = policy
            .scope
            .review_channel
            .unwrap_or(authority.facts.origin);
        if policy.scope.review_channel.is_none() && !policy.scope.source_channels.contains(&origin)
        {
            return Err("contextual aggregate origin is outside scope");
        }
        authority.staff(origin)?;
        let mut counts = Counts::default();
        for case in self
            .cases
            .values()
            .filter(|c| c.source.guild == policy.guild && c.captured.owner == policy.owner)
        {
            match case.disposition {
                Disposition::ConfirmedProposal { .. } => counts.confirmed_proposals += 1,
                Disposition::HumanReview => counts.human_review_referrals += 1,
            }
            match case.review.as_ref().map(|r| r.decision) {
                Some(ReviewDecision::Agree) => counts.review_agree += 1,
                Some(ReviewDecision::Disagree) => counts.review_disagree += 1,
                Some(ReviewDecision::NeedsContext) => counts.review_needs_context += 1,
                None => counts.unreviewed += 1,
            }
            if let Some(appeal) = &case.appeal {
                match appeal.resolution.as_ref().map(|r| r.decision) {
                    Some(AppealDecision::Upheld) => counts.appeal_upheld += 1,
                    Some(AppealDecision::Rejected) => counts.appeal_rejected += 1,
                    Some(AppealDecision::NeedsContext) => counts.appeal_needs_context += 1,
                    None => counts.appeal_open += 1,
                }
            }
        }
        Ok(counts)
    }
}
fn check_revision(case: &Case, expected: &ExpectedCase) -> Result<(), &'static str> {
    if case.id != expected.id || case.revision != expected.revision {
        return Err("contextual case changed; refresh before deciding");
    }
    Ok(())
}
fn check_revision_or_resolution(case: &Case, expected: &ExpectedCase) -> Result<(), &'static str> {
    if case
        .appeal
        .as_ref()
        .and_then(|a| a.resolution.as_ref())
        .is_none()
    {
        check_revision(case, expected)?;
    }
    Ok(())
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub confirmed_proposals: usize,
    pub human_review_referrals: usize,
    pub unreviewed: usize,
    pub review_agree: usize,
    pub review_disagree: usize,
    pub review_needs_context: usize,
    pub appeal_open: usize,
    pub appeal_upheld: usize,
    pub appeal_rejected: usize,
    pub appeal_needs_context: usize,
}
fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn digest(value: &impl Serialize) -> Result<String, &'static str> {
    let bytes = serde_json::to_vec(value).map_err(|_| "contextual digest failed")?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[cfg(test)]
pub(crate) mod tests;

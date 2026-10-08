//! Serialized closed case evidence; loading this module never grants authority.
use super::super::{
    Severity,
    contextual::{self, Assessment},
};
use super::{FreshAuthority, ShadowPolicy, SourceVersion, is_digest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HumanAssessment {
    ConfirmedOffending,
    Ambiguous,
    QuotationOrReport,
}
impl From<Assessment> for HumanAssessment {
    fn from(value: Assessment) -> Self {
        match value {
            Assessment::ConfirmedOffending => Self::ConfirmedOffending,
            Assessment::Ambiguous => Self::Ambiguous,
            Assessment::QuotationOrReport => Self::QuotationOrReport,
        }
    }
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaseSeverity {
    Minor,
    Serious,
    Severe,
}
impl From<Severity> for CaseSeverity {
    fn from(value: Severity) -> Self {
        match value {
            Severity::Minor => Self::Minor,
            Severity::Serious => Self::Serious,
            Severity::Severe => Self::Severe,
        }
    }
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaseOrigin {
    HumanModerator,
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Disposition {
    ConfirmedProposal { timeout_minutes: Option<u32> },
    HumanReview,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub actor: u64,
    pub origin: u64,
    pub owner: u64,
    pub at: u64,
    pub policy_digest: String,
    pub scope_digest: String,
}
impl Provenance {
    pub(super) fn new(
        authority: &FreshAuthority,
        policy: &ShadowPolicy,
        policy_digest: &str,
        now: u64,
    ) -> Result<Self, &'static str> {
        authority.check(policy, now)?;
        if !is_digest(policy_digest) {
            return Err("invalid contextual policy digest");
        }
        Ok(Self {
            actor: authority.facts.actor,
            origin: authority.facts.origin,
            owner: policy.owner,
            at: now,
            policy_digest: policy_digest.into(),
            scope_digest: policy.scope_digest()?,
        })
    }
    pub(super) fn validate(&self) -> Result<(), &'static str> {
        if [self.actor, self.origin, self.owner, self.at].contains(&0)
            || !is_digest(&self.policy_digest)
            || !is_digest(&self.scope_digest)
        {
            return Err("invalid contextual provenance");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Agree,
    Disagree,
    NeedsContext,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub expected_revision: u64,
    pub decision: ReviewDecision,
    pub provenance: Provenance,
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppealReason {
    ContextMissing,
    AttributionWrong,
    AssessmentDisputed,
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppealDecision {
    Upheld,
    Rejected,
    NeedsContext,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppealResolution {
    pub expected_revision: u64,
    pub decision: AppealDecision,
    pub provenance: Provenance,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Appeal {
    pub expected_revision: u64,
    pub reason: AppealReason,
    pub provenance: Provenance,
    pub resolution: Option<AppealResolution>,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub revision: u64,
    pub source: SourceVersion,
    pub origin: CaseOrigin,
    pub assessment: HumanAssessment,
    pub severity: CaseSeverity,
    pub disposition: Disposition,
    pub captured: Provenance,
    pub review: Option<Review>,
    pub appeal: Option<Appeal>,
}
impl Case {
    pub(super) fn validate(&self) -> Result<(), &'static str> {
        self.source.validate()?;
        self.captured.validate()?;
        let expected = match self.assessment {
            HumanAssessment::ConfirmedOffending => Disposition::ConfirmedProposal {
                timeout_minutes: match self.severity {
                    CaseSeverity::Minor => None,
                    CaseSeverity::Serious | CaseSeverity::Severe => {
                        Some(contextual::MAX_CONTEXTUAL_TIMEOUT_MINUTES)
                    }
                },
            },
            _ => Disposition::HumanReview,
        };
        if self.id != self.source.case_id()?
            || self.disposition != expected
            || self.captured.origin != self.source.channel
            || self.captured.actor == self.source.author
            || self.captured.owner == self.source.author
            || self.captured.at < self.source.created_at
            || self
                .source
                .edited_at
                .is_some_and(|at| self.captured.at < at)
        {
            return Err("invalid contextual case");
        }
        let mut events = Vec::new();
        if let Some(review) = &self.review {
            review.provenance.validate()?;
            if [self.source.author, self.captured.actor].contains(&review.provenance.actor) {
                return Err("contextual review is not independent");
            }
            events.push((review.expected_revision, &review.provenance));
        }
        if let Some(appeal) = &self.appeal {
            appeal.provenance.validate()?;
            if appeal.provenance.actor != self.source.author
                || appeal.provenance.origin != self.source.channel
            {
                return Err("invalid contextual appellant");
            }
            events.push((appeal.expected_revision, &appeal.provenance));
            if let Some(resolution) = &appeal.resolution {
                resolution.provenance.validate()?;
                if [self.source.author, self.captured.actor].contains(&resolution.provenance.actor)
                    || self
                        .review
                        .as_ref()
                        .is_some_and(|r| r.provenance.actor == resolution.provenance.actor)
                    || resolution.expected_revision <= appeal.expected_revision
                {
                    return Err("contextual appeal resolution is not independent");
                }
                events.push((resolution.expected_revision, &resolution.provenance));
            }
        }
        events.sort_by_key(|(revision, _)| *revision);
        let mut last_at = self.captured.at;
        for (index, (revision, provenance)) in events.iter().enumerate() {
            if *revision != index as u64 + 1
                || provenance.at < last_at
                || provenance.owner != self.captured.owner
            {
                return Err("invalid contextual event order");
            }
            last_at = provenance.at;
        }
        if self.revision != events.len() as u64 + 1 {
            return Err("invalid contextual revision");
        }
        Ok(())
    }
}

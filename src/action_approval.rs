//! Pure, durable approval state for external writes. The Discord and GitHub
//! shells supply current authority and target facts at confirmation and again
//! before execution; this module never makes a network call.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionTarget {
    DiscordChannel {
        guild: u64,
        channel: u64,
    },
    DiscordMessage {
        guild: u64,
        channel: u64,
        message: u64,
    },
    GitHubRepository {
        installation: u64,
        owner: String,
        repo: String,
    },
    GitHubIssue {
        installation: u64,
        owner: String,
        repo: String,
        issue: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionOperation {
    DiscordSendMessage {
        content: String,
    },
    DiscordEditOwnMessage {
        content: String,
    },
    DiscordCreateThread {
        name: String,
        content: String,
    },
    DiscordCreateForumPost {
        name: String,
        content: String,
    },
    DiscordCreateEvent {
        name: String,
        starts_at: u64,
        ends_at: u64,
    },
    DiscordAddChannel {
        name: String,
        parent: Option<u64>,
    },
    GitHubCreateIssue {
        title: String,
        body: String,
    },
    GitHubUpdateIssue {
        title: Option<String>,
        body: Option<String>,
    },
    GitHubComment {
        body: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ActionPermission {
    DiscordSendMessages,
    DiscordManageMessages,
    DiscordManageThreads,
    DiscordManageChannels,
    DiscordManageEvents,
    GitHubIssuesWrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionSpec {
    pub target: ActionTarget,
    pub operation: ActionOperation,
    pub required: BTreeSet<ActionPermission>,
}

impl ActionSpec {
    fn valid(&self) -> bool {
        let (target_ok, needed) = match (&self.operation, &self.target) {
            (
                ActionOperation::DiscordSendMessage { content },
                ActionTarget::DiscordChannel { guild, channel },
            ) => (
                *guild != 0
                    && *channel != 0
                    && !content.trim().is_empty()
                    && content.len() <= 2_000,
                ActionPermission::DiscordSendMessages,
            ),
            (
                ActionOperation::DiscordEditOwnMessage { content },
                ActionTarget::DiscordMessage {
                    guild,
                    channel,
                    message,
                },
            ) => (
                *guild != 0
                    && *channel != 0
                    && *message != 0
                    && !content.trim().is_empty()
                    && content.len() <= 2_000,
                ActionPermission::DiscordManageMessages,
            ),
            (
                ActionOperation::DiscordCreateThread { name, content }
                | ActionOperation::DiscordCreateForumPost { name, content },
                ActionTarget::DiscordChannel { guild, channel },
            ) => (
                *guild != 0
                    && *channel != 0
                    && !name.trim().is_empty()
                    && name.len() <= 100
                    && !content.trim().is_empty()
                    && content.len() <= 2_000,
                ActionPermission::DiscordManageThreads,
            ),
            (
                ActionOperation::DiscordCreateEvent {
                    name,
                    starts_at,
                    ends_at,
                },
                ActionTarget::DiscordChannel { guild, channel },
            ) => (
                *guild != 0
                    && *channel != 0
                    && !name.trim().is_empty()
                    && name.len() <= 100
                    && starts_at < ends_at,
                ActionPermission::DiscordManageEvents,
            ),
            (
                ActionOperation::DiscordAddChannel { name, .. },
                ActionTarget::DiscordChannel { guild, channel },
            ) => (
                *guild != 0 && *channel != 0 && !name.trim().is_empty() && name.len() <= 100,
                ActionPermission::DiscordManageChannels,
            ),
            (
                ActionOperation::GitHubCreateIssue { title, body },
                ActionTarget::GitHubRepository {
                    installation,
                    owner,
                    repo,
                },
            ) => (
                *installation != 0
                    && valid_repo(owner, repo)
                    && !title.trim().is_empty()
                    && title.len() <= 256
                    && body.len() <= 65_000,
                ActionPermission::GitHubIssuesWrite,
            ),
            (
                ActionOperation::GitHubUpdateIssue { title, body },
                ActionTarget::GitHubIssue {
                    installation,
                    owner,
                    repo,
                    issue,
                },
            ) => (
                *installation != 0
                    && *issue != 0
                    && valid_repo(owner, repo)
                    && (title.is_some() || body.is_some())
                    && title
                        .as_ref()
                        .is_none_or(|text| !text.trim().is_empty() && text.len() <= 256)
                    && body.as_ref().is_none_or(|text| text.len() <= 65_000),
                ActionPermission::GitHubIssuesWrite,
            ),
            (
                ActionOperation::GitHubComment { body },
                ActionTarget::GitHubIssue {
                    installation,
                    owner,
                    repo,
                    issue,
                },
            ) => (
                *installation != 0
                    && *issue != 0
                    && valid_repo(owner, repo)
                    && !body.trim().is_empty()
                    && body.len() <= 65_000,
                ActionPermission::GitHubIssuesWrite,
            ),
            _ => return false,
        };
        target_ok && self.required == BTreeSet::from([needed])
    }
}

fn valid_repo(owner: &str, repo: &str) -> bool {
    [owner, repo].into_iter().all(|part| {
        !part.is_empty()
            && part.len() <= 100
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionState {
    Proposed,
    Approved,
    Executing,
    Verified,
    ReviewRequired,
    Failed,
    Invalidated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionProposal {
    id: u64,
    spec: ActionSpec,
    requester: u64,
    expires_at: u64,
    target_fingerprint: String,
    content_digest: String,
    state: ActionState,
    approver: Option<u64>,
    result_reference: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionError {
    Invalid,
    Denied,
    Expired,
    Stale,
    AlreadyHandled,
    Missing,
    Full,
}

#[derive(Debug, Clone)]
pub struct ActionFacts<'a> {
    pub human_principal: Option<u64>,
    pub current_permissions: &'a BTreeSet<ActionPermission>,
    pub target_fingerprint: &'a str,
    pub now: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ActionStore {
    sequence: u64,
    proposals: BTreeMap<u64, ActionProposal>,
}

fn content_digest(spec: &ActionSpec, requester: u64, expires_at: u64, target: &str) -> String {
    let mut hasher = Sha256::new();
    // All fields are typed. JSON field order is stable for these structs and
    // BTreeSet serializes in its defined order.
    let encoded = serde_json::to_vec(&(spec, requester, expires_at, target))
        .expect("action proposal types serialize");
    hasher.update(encoded);
    let mut out = String::with_capacity(64);
    for byte in hasher.finalize() {
        use std::fmt::Write;
        write!(&mut out, "{byte:02x}").expect("writing to String cannot fail");
    }
    out
}

impl ActionProposal {
    pub fn spec(&self) -> &ActionSpec {
        &self.spec
    }

    pub fn requester(&self) -> u64 {
        self.requester
    }

    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }

    pub fn target_fingerprint(&self) -> &str {
        &self.target_fingerprint
    }

    pub fn approver(&self) -> Option<u64> {
        self.approver
    }

    pub fn result_reference(&self) -> Option<&str> {
        self.result_reference.as_deref()
    }

    pub fn content_digest(&self) -> &str {
        &self.content_digest
    }

    pub fn state(&self) -> ActionState {
        self.state
    }
}

/// Both outcomes are successful durable transitions. Commit this result through
/// `AppState::commit_work` before inspecting it: only `Ready` permits a send.
/// Converting `Invalidated` into the closure's error would discard revocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeginOutcome {
    Ready(ActionSpec),
    Invalidated(ActionError),
}

impl ActionStore {
    pub fn proposal(&self, id: u64) -> Option<&ActionProposal> {
        self.proposals.get(&id)
    }

    /// Reclaim terminal records and abandoned, expired approvals. Uncertain or
    /// in-flight attempts are retained regardless of age for human review.
    /// Pass caller-injected current time, and persist this transition.
    pub fn compact(&mut self, now: u64) {
        self.proposals.retain(|_, proposal| match proposal.state {
            ActionState::Verified | ActionState::Failed | ActionState::Invalidated => false,
            ActionState::Proposed | ActionState::Approved => now < proposal.expires_at,
            ActionState::Executing | ActionState::ReviewRequired => true,
        });
    }

    pub fn propose(
        &mut self,
        spec: ActionSpec,
        requester: u64,
        expires_at: u64,
        target_fingerprint: &str,
    ) -> Result<u64, ActionError> {
        if requester == 0
            || expires_at == 0
            || target_fingerprint.is_empty()
            || target_fingerprint.len() > 256
            || !spec.valid()
        {
            return Err(ActionError::Invalid);
        }
        let id = self.sequence.checked_add(1).ok_or(ActionError::Full)?;
        if self.proposals.len() >= 10_000 {
            // A caller can additionally compact expired proposals with its
            // current time before proposing. Zero reclaims only terminal rows.
            self.compact(0);
            if self.proposals.len() >= 10_000 {
                return Err(ActionError::Full);
            }
        }
        self.sequence = id;
        let digest = content_digest(&spec, requester, expires_at, target_fingerprint);
        self.proposals.insert(
            id,
            ActionProposal {
                id,
                spec,
                requester,
                expires_at,
                target_fingerprint: target_fingerprint.to_string(),
                content_digest: digest,
                state: ActionState::Proposed,
                approver: None,
                result_reference: None,
            },
        );
        Ok(id)
    }

    pub fn confirm(
        &mut self,
        id: u64,
        displayed_digest: &str,
        facts: ActionFacts<'_>,
    ) -> Result<(), ActionError> {
        let proposal = self.proposals.get_mut(&id).ok_or(ActionError::Missing)?;
        if proposal.state != ActionState::Proposed {
            return Err(ActionError::AlreadyHandled);
        }
        validate(proposal, displayed_digest, &facts)?;
        proposal.approver = facts.human_principal;
        proposal.state = ActionState::Approved;
        Ok(())
    }

    /// Persist both Ready and Invalidated outcomes before interpreting them.
    /// A crash after Ready leaves a review-required attempt, never a replay.
    pub fn begin(&mut self, id: u64, facts: ActionFacts<'_>) -> Result<BeginOutcome, ActionError> {
        let proposal = self.proposals.get_mut(&id).ok_or(ActionError::Missing)?;
        if proposal.state != ActionState::Approved {
            return Err(ActionError::AlreadyHandled);
        }
        if proposal.approver != facts.human_principal {
            return Err(ActionError::Denied);
        }
        if let Err(reason) = validate(proposal, &proposal.content_digest, &facts) {
            proposal.state = ActionState::Invalidated;
            proposal.approver = None;
            return Ok(BeginOutcome::Invalidated(reason));
        }
        proposal.state = ActionState::Executing;
        Ok(BeginOutcome::Ready(proposal.spec.clone()))
    }

    pub fn finish(
        &mut self,
        id: u64,
        state: ActionState,
        result_reference: Option<String>,
    ) -> Result<(), ActionError> {
        let proposal = self.proposals.get_mut(&id).ok_or(ActionError::Missing)?;
        if proposal.state != ActionState::Executing
            || !matches!(
                state,
                ActionState::Verified | ActionState::ReviewRequired | ActionState::Failed
            )
        {
            return Err(ActionError::AlreadyHandled);
        }
        proposal.state = state;
        proposal.result_reference = result_reference;
        Ok(())
    }

    pub fn mark_interrupted(&mut self) {
        for proposal in self.proposals.values_mut() {
            if proposal.state == ActionState::Executing {
                proposal.state = ActionState::ReviewRequired;
            }
        }
    }
}

fn validate(
    proposal: &ActionProposal,
    displayed_digest: &str,
    facts: &ActionFacts<'_>,
) -> Result<(), ActionError> {
    if facts.human_principal.is_none() || facts.human_principal == Some(0) {
        return Err(ActionError::Denied);
    }
    if facts.now >= proposal.expires_at {
        return Err(ActionError::Expired);
    }
    if displayed_digest != proposal.content_digest
        || facts.target_fingerprint != proposal.target_fingerprint
        || content_digest(
            &proposal.spec,
            proposal.requester,
            proposal.expires_at,
            &proposal.target_fingerprint,
        ) != proposal.content_digest
    {
        return Err(ActionError::Stale);
    }
    if !proposal.spec.required.is_subset(facts.current_permissions) {
        return Err(ActionError::Denied);
    }
    Ok(())
}

#[cfg(test)]
mod tests;

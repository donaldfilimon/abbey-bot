//! Typed delivery authority and immutable native-content provenance. A private
//! principal is not a Discord channel; resolving a DM belongs to the shell.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkDestination {
    Personal { principal: u64 },
    TeamChannel { channel: u64 },
    TeamPrivate { principal: u64 },
}

impl WorkDestination {
    pub(super) fn validate(&self, scope: &WorkScope, actor: u64) -> Result<(), WorkError> {
        let valid = match (self, scope) {
            (Self::Personal { principal }, WorkScope::Personal { owner }) => {
                *principal != 0 && principal == owner && *principal == actor
            }
            (Self::TeamPrivate { principal }, WorkScope::Team { .. }) => {
                *principal != 0 && *principal == actor
            }
            (
                Self::TeamChannel { channel },
                WorkScope::Team {
                    channel: origin, ..
                },
            ) => *channel != 0 && channel == origin,
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(WorkError::Denied)
        }
    }
}

/// Native immutable identity/revision consumed by the future recall projection.
/// Decisions are immutable and therefore use revision 1. No inferred evidence
/// or merely authorized empty project is a source.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WorkContentRef {
    Task {
        project: u64,
        id: u64,
        revision: u64,
    },
    Decision {
        project: u64,
        id: u64,
        revision: u64,
    },
}

impl WorkContentRef {
    pub(super) fn id(&self) -> u64 {
        match self {
            Self::Task { id, .. } | Self::Decision { id, .. } => *id,
        }
    }

    fn project(&self) -> u64 {
        match self {
            Self::Task { project, .. } | Self::Decision { project, .. } => *project,
        }
    }

    fn valid(&self) -> bool {
        match self {
            Self::Task { project, id, .. } => *project != 0 && *id != 0,
            Self::Decision {
                project,
                id,
                revision,
            } => *project != 0 && *id != 0 && *revision == 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveredProvenance {
    pub version: u8,
    pub source_refs: BTreeSet<WorkContentRef>,
    pub contributing_projects: BTreeSet<u64>,
    /// SHA-256 of the exact frozen UTF-8 transport body.
    pub rendered_digest: String,
}

impl DeliveredProvenance {
    pub fn validate(&self) -> Result<(), WorkError> {
        if self.version != 1
            || self.source_refs.is_empty()
            || self.source_refs.len() > 32
            || self.source_refs.iter().any(|r| !r.valid())
            || self
                .source_refs
                .iter()
                .map(WorkContentRef::id)
                .collect::<BTreeSet<_>>()
                .len()
                != self.source_refs.len()
            || self.contributing_projects
                != self
                    .source_refs
                    .iter()
                    .map(WorkContentRef::project)
                    .collect()
            || self.rendered_digest.len() != 64
            || !self
                .rendered_digest
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(WorkError::Invalid);
        }
        Ok(())
    }
}

impl WorkDeliveryReceipt {
    /// Requires fresh access to the ORIGINAL work scope. Passing DM facts cannot
    /// construct team authorization. Private feedback also binds exact principal.
    pub(super) fn feedback_destination_matches(&self, access: WorkAccess) -> bool {
        if self
            .provenance
            .as_ref()
            .is_some_and(|p| p.validate().is_err())
        {
            return false;
        }
        match (&self.destination, &self.scope) {
            (Some(destination), Some(scope)) => {
                destination.validate(scope, access.actor).is_ok()
                    && match destination {
                        WorkDestination::TeamPrivate { .. } => self.recipient != 0,
                        _ => self.recipient == access.channel,
                    }
            }
            (None, _) => self.recipient == access.channel, // Existing native learning only.
            _ => false,
        }
    }
}

impl WorkStore {
    pub(super) fn content_refs(&self, scope: &WorkScope) -> BTreeSet<WorkContentRef> {
        let projects: BTreeSet<_> = self
            .projects
            .values()
            .filter(|p| &p.scope == scope)
            .map(|p| p.id)
            .collect();
        self.tasks
            .values()
            .filter(|t| projects.contains(&t.project_id))
            .map(|t| WorkContentRef::Task {
                project: t.project_id,
                id: t.id,
                revision: t.revision,
            })
            .chain(
                self.decisions
                    .values()
                    .filter(|d| projects.contains(&d.project_id))
                    .map(|d| WorkContentRef::Decision {
                        project: d.project_id,
                        id: d.id,
                        revision: 1,
                    }),
            )
            .collect()
    }

    pub(super) fn target(
        &self,
        scope: &WorkScope,
        access: WorkAccess,
        policy: &WorkAutomationPolicy,
    ) -> Result<WorkDestination, WorkError> {
        let target = policy.delivery_target.clone().unwrap_or(match scope {
            WorkScope::Personal { owner } => WorkDestination::Personal { principal: *owner },
            WorkScope::Team { channel, .. } => WorkDestination::TeamChannel { channel: *channel },
        });
        target.validate(scope, access.actor)?;
        // Legacy destination remains origin/personal channel. Private DM resolution
        // is deliberately not persisted as a channel-policy permission grant.
        if policy.enabled && policy.destination != Some(access.channel) {
            return Err(WorkError::Denied);
        }
        Ok(target)
    }
}

impl WorkStore {
    /// Canonical bounded fallback renderer. The runtime builder must preserve this
    /// exact source/body pairing, or construct and validate a new frozen draft.
    pub(super) fn render_delivery(
        &self,
        refs: &[WorkContentRef],
        omitted: usize,
    ) -> Result<(String, DeliveredProvenance), WorkError> {
        use sha2::{Digest, Sha256};
        let mut lines = Vec::new();
        for source in refs {
            lines.push(match source {
                WorkContentRef::Task {
                    project,
                    id,
                    revision,
                } => {
                    let task = self.tasks.get(id).ok_or(WorkError::Missing)?;
                    if task.project_id != *project || task.revision != *revision {
                        return Err(WorkError::Stale);
                    }
                    format!(
                        "Task #{id} r{revision}: {} — {} (priority {}; deadline {}).",
                        shortened(&task.title, 80),
                        task.status.label(),
                        task.priority,
                        task.due_at.map_or("none".into(), |v| v.to_string())
                    )
                }
                WorkContentRef::Decision {
                    project,
                    id,
                    revision,
                } => {
                    let decision = self.decisions.get(id).ok_or(WorkError::Missing)?;
                    if decision.project_id != *project || *revision != 1 {
                        return Err(WorkError::Stale);
                    }
                    format!("Decision #{id}: {}", shortened(&decision.text, 100))
                }
            });
        }
        if omitted > 0 {
            lines.push(format!("Partial update: {omitted} additional work item(s) omitted; inspect the workspace for the full list."));
        }
        let body = lines.join("\n");
        if body.chars().count() > 1900 {
            return Err(WorkError::Full);
        }
        let provenance = DeliveredProvenance {
            version: 1,
            source_refs: refs.iter().cloned().collect(),
            contributing_projects: refs.iter().map(WorkContentRef::project).collect(),
            rendered_digest: Sha256::digest(body.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        };
        provenance.validate()?;
        Ok((body, provenance))
    }
}

impl WorkStore {
    /// Exclude scheduling-only edits from optional work changes. Coverage still
    /// records the exact full native revision actually rendered in a delivery.
    pub(super) fn content_fingerprints(&self, scope: &WorkScope) -> BTreeMap<u64, String> {
        use sha2::{Digest, Sha256};
        let digest = |text: String| {
            Sha256::digest(text.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        let mut values = BTreeMap::new();
        for source in self.content_refs(scope) {
            match source {
                WorkContentRef::Task { id, .. } => {
                    let t = &self.tasks[&id];
                    let value = serde_json::json!([
                        t.title, t.status, t.priority, t.owner, t.assignee, t.goal_id, t.due_at,
                        t.source, t.github
                    ]);
                    values.insert(id, digest(value.to_string()));
                }
                WorkContentRef::Decision { id, .. } => {
                    let d = &self.decisions[&id];
                    values.insert(
                        id,
                        digest(serde_json::json!([d.text, d.author, d.at]).to_string()),
                    );
                }
            }
        }
        values
    }
}

fn shortened(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut shown: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        shown.push_str("… [shortened]");
    }
    shown
}

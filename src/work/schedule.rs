//! Pure scope scheduler. Every timestamp is injected UTC seconds. The runtime
//! must use commit_work to durably reserve before sending, and freshly verify
//! destination identity/access. A plan alone never authorizes network delivery.
use super::*;
use chrono::{DateTime, Duration, Utc};
use chrono::{LocalResult, NaiveDate, TimeZone, Timelike};
use chrono_tz::Tz;

impl WorkScope {
    pub fn key(&self) -> String {
        match self {
            Self::Personal { owner } => format!("personal:{owner}"),
            Self::Team { guild, channel } => format!("team:{guild}:{channel}"),
        }
    }
}

impl WorkAutomationPolicy {
    pub fn validate(&self) -> Result<(), WorkError> {
        if self.quiet_start >= 24
            || self.quiet_end >= 24
            || self.briefing_hour >= 24
            || self.daily_limit == 0
            || self.daily_limit > 4
            || (!self.timezone.is_empty() && self.timezone.parse::<Tz>().is_err())
            || self.destination == Some(0)
            || self
                .delivery_target
                .as_ref()
                .is_some_and(|target| match target {
                    WorkDestination::Personal { principal }
                    | WorkDestination::TeamPrivate { principal } => *principal == 0,
                    WorkDestination::TeamChannel { channel } => *channel == 0,
                })
            || (self.enabled
                && (self.destination.is_none_or(|id| id == 0) || self.timezone.is_empty()))
        {
            return Err(WorkError::Invalid);
        }
        Ok(())
    }

    fn quiet(&self, hour: u32) -> bool {
        let start = u32::from(self.quiet_start);
        let end = u32::from(self.quiet_end);
        if start < end {
            hour >= start && hour < end
        } else if start > end {
            hour >= start || hour < end
        } else {
            false // Equal boundaries explicitly disable quiet hours.
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkBatch {
    pub scope: WorkScope,
    pub destination: u64,
    pub target: WorkDestination,
    pub content_refs: BTreeSet<WorkContentRef>,
    pub rendered_body: String,
    pub provenance: DeliveredProvenance,
    pub local_day: String,
    pub kind: WorkDeliveryKind,
    pub task_ids: Vec<u64>,
    pub coverage: Vec<ReminderCoverage>,
    pub dedupe_keys: BTreeSet<String>,
    // Replanning validates both configuration and records before reservation.
    policy: WorkAutomationPolicy,
    revisions: Vec<(u64, u64)>,
    project_revisions: Vec<(u64, u64)>,
}

fn utc(seconds: u64) -> Result<DateTime<Utc>, WorkError> {
    let at = DateTime::from_timestamp(i64::try_from(seconds).map_err(|_| WorkError::Invalid)?, 0)
        .ok_or(WorkError::Invalid)?;
    // Leave room for any IANA offset and a skipped calendar day. Chrono's
    // local date accessors panic if a valid UTC instant overflows locally.
    at.checked_add_signed(Duration::days(2))
        .ok_or(WorkError::Invalid)?;
    at.checked_sub_signed(Duration::days(2))
        .ok_or(WorkError::Invalid)?;
    Ok(at)
}

/// Calendar recurrence: choose the first occurrence in a fall-back overlap;
/// advance through a spring-forward gap to the first valid local minute.
fn local_hour(tz: Tz, day: NaiveDate, hour: u8) -> Result<DateTime<Utc>, WorkError> {
    let mut local = day
        .and_hms_opt(u32::from(hour), 0, 0)
        .ok_or(WorkError::Invalid)?;
    for _ in 0..=1_440 {
        match tz.from_local_datetime(&local) {
            LocalResult::Single(time) => return Ok(time.with_timezone(&Utc)),
            LocalResult::Ambiguous(a, b) => return Ok(a.min(b).with_timezone(&Utc)),
            LocalResult::None => {
                local = local
                    .checked_add_signed(Duration::minutes(1))
                    .ok_or(WorkError::Invalid)?;
            }
        }
    }
    Err(WorkError::Invalid)
}

impl WorkStore {
    pub(crate) fn scope_projects(
        &self,
        scope: &WorkScope,
        access: WorkAccess,
        manager: bool,
    ) -> Result<Vec<&WorkProject>, WorkError> {
        let projects: Vec<_> = self
            .projects
            .values()
            .filter(|p| &p.scope == scope)
            .collect();
        if projects.is_empty() {
            return Err(WorkError::Missing);
        }
        for project in &projects {
            project.authorize(access, manager)?;
        }
        Ok(projects)
    }

    /// Explicit opt-in: caller chooses an IANA timezone (Donald's initial
    /// personal choice is America/New_York). Never infer one for other users.
    pub fn configure_automation(
        &mut self,
        scope: &WorkScope,
        access: WorkAccess,
        mut policy: WorkAutomationPolicy,
    ) -> Result<(), WorkError> {
        self.scope_projects(scope, access, true)?;
        policy.validate()?;
        self.target(scope, access, &policy)?;
        let old = self.scope_automation.get(&scope.key());
        policy.revision = old.map_or(0, |p| p.revision);
        let changed = old != Some(&policy)
            || (policy.enabled
                && self.scope_automation_actors.get(&scope.key()) != Some(&access.actor));
        if changed {
            policy.revision = policy.revision.checked_add(1).ok_or(WorkError::Full)?;
        }
        // Missing coverage (including legacy state) seeds history once. Pausing,
        // subscriber changes and policy edits never reset scope coverage.
        if policy.enabled && !self.change_coverage.contains_key(&scope.key()) {
            self.change_coverage
                .insert(scope.key(), self.content_refs(scope));
        }
        if policy.enabled && !self.change_fingerprints.contains_key(&scope.key()) {
            self.change_fingerprints
                .insert(scope.key(), self.content_fingerprints(scope));
        }
        if policy.enabled {
            self.scope_automation_actors
                .insert(scope.key(), access.actor);
        } else {
            self.scope_automation_actors.remove(&scope.key());
        }
        self.scope_automation.insert(scope.key(), policy);
        Ok(())
    }

    /// Changing explicit timing re-arms only that reminder, without changing
    /// its deadline. Status/title/priority revisions do not re-arm reminders.
    pub fn set_reminder(
        &mut self,
        access: WorkAccess,
        id: u64,
        revision: u64,
        remind_at: Option<u64>,
    ) -> Result<(), WorkError> {
        let task = self.tasks.get(&id).ok_or(WorkError::Missing)?;
        self.project(task.project_id, access)?;
        if task.revision != revision {
            return Err(WorkError::Stale);
        }
        if let Some(at) = remind_at {
            utc(at)?;
        }
        if task.remind_at == remind_at {
            return Ok(());
        }
        let next = task.revision.checked_add(1).ok_or(WorkError::Full)?;
        let reminder_next = task
            .reminder_revision
            .checked_add(1)
            .ok_or(WorkError::Full)?;
        let task = self.tasks.get_mut(&id).ok_or(WorkError::Missing)?;
        task.remind_at = remind_at;
        task.reminder_revision = reminder_next;
        task.revision = next;
        self.recall.task_changed(task.project_id, id, next);
        Ok(())
    }

    /// Explicit snooze postpones an existing reminder, never creates one from
    /// a deadline. The owned transaction persists both revisions together.
    pub fn snooze_task(
        &mut self,
        access: WorkAccess,
        id: u64,
        revision: u64,
        until: u64,
    ) -> Result<u64, WorkError> {
        utc(until)?;
        let task = self.tasks.get(&id).ok_or(WorkError::Missing)?;
        self.project(task.project_id, access)?;
        if task.revision != revision {
            return Err(WorkError::Stale);
        }
        let status = task.status;
        let reminder = task.remind_at.is_some();
        // Preflight overflow before either mutation.
        revision
            .checked_add(if reminder { 2 } else { 1 })
            .ok_or(WorkError::Full)?;
        if reminder {
            task.reminder_revision
                .checked_add(1)
                .ok_or(WorkError::Full)?;
        }
        let revision = self.update_task(access, id, revision, status, Some(until))?;
        if reminder {
            self.set_reminder(access, id, revision, Some(until))?;
        }
        Ok(self.tasks.get(&id).ok_or(WorkError::Missing)?.revision)
    }

    /// At most one batch per call. Missed days collapse into one latest briefing;
    /// overdue reminders collapse into that briefing, or one reminder batch.
    /// Quiet hours defer without consuming coverage or quota.
    pub fn next_batch(
        &self,
        scope: &WorkScope,
        access: WorkAccess,
        now: u64,
    ) -> Result<Option<WorkBatch>, WorkError> {
        let projects = self.scope_projects(scope, access, false)?;
        let Some(policy) = self.scope_automation.get(&scope.key()) else {
            return Ok(None);
        };
        policy.validate()?;
        if !policy.enabled || self.scope_automation_actors.get(&scope.key()) != Some(&access.actor)
        {
            return Ok(None);
        }
        self.scope_projects(scope, access, true)?;
        let destination = policy.destination.ok_or(WorkError::Invalid)?;
        let target = self.target(scope, access, policy)?;
        let tz = policy
            .timezone
            .parse::<Tz>()
            .map_err(|_| WorkError::Invalid)?;
        let now_utc = utc(now)?;
        let local = now_utc.with_timezone(&tz);
        let local_day = local.format("%Y-%m-%d").to_string();
        let receipts: Vec<_> = self
            .deliveries
            .values()
            .filter(|r| {
                r.scope.as_ref().map_or_else(
                    || {
                        self.projects
                            .get(&r.project_id)
                            .is_some_and(|p| &p.scope == scope)
                    },
                    |receipt_scope| receipt_scope == scope,
                )
            })
            .collect();
        // Recalculate legacy/current receipt dates from UTC in the current
        // timezone so a policy edit cannot reset today's already spent quota.
        let attempts = receipts
            .iter()
            .filter(|r| {
                utc(r.at).is_ok_and(|at| at.with_timezone(&tz).date_naive() == local.date_naive())
            })
            .count();
        if policy.quiet(local.hour()) || attempts >= usize::from(policy.daily_limit) {
            return Ok(None);
        }
        let project_ids: BTreeSet<_> = projects.iter().map(|p| p.id).collect();
        let tasks: Vec<_> = self
            .tasks
            .values()
            .filter(|t| {
                project_ids.contains(&t.project_id)
                    && !matches!(t.status, WorkStatus::Done | WorkStatus::Cancelled)
                    && t.snoozed_until.is_none_or(|until| until <= now)
            })
            .collect();
        let briefing_hour = self
            .preference_profile(access)?
            .effective_hour(policy.briefing_hour);
        let mut occurrence_day = local.date_naive();
        let today_at = local_hour(tz, occurrence_day, briefing_hour)?;
        // An evening occurrence inside overnight quiet hours becomes eligible
        // the following morning. Keep its original date as its identity; the
        // receipt's local_day and UTC at still charge the actual delivery day.
        if now_utc < today_at
            && policy.quiet_start > policy.quiet_end
            && briefing_hour >= policy.quiet_start
        {
            occurrence_day = occurrence_day.pred_opt().ok_or(WorkError::Invalid)?;
        }
        let briefing_key = format!("{}:{occurrence_day}:briefing:scope", scope.key());
        let briefing = !tasks.is_empty()
            && now_utc >= local_hour(tz, occurrence_day, briefing_hour)?
            && !receipts.iter().any(|r| {
                r.dedupe_keys.contains(&briefing_key)
                    || (r.kind == Some(WorkDeliveryKind::Briefing)
                        && utc(r.at).is_ok_and(|at| {
                            at.with_timezone(&tz).date_naive() == local.date_naive()
                        }))
            });
        let coverage: Vec<_> = tasks
            .iter()
            .filter_map(|t| {
                let at = t.remind_at.filter(|at| *at <= now)?;
                let covered = ReminderCoverage {
                    task_id: t.id,
                    reminder_revision: t.reminder_revision,
                    remind_at: at,
                };
                (!receipts.iter().any(|r| r.coverage.contains(&covered))).then_some(covered)
            })
            .collect();
        let changes: BTreeSet<_> = self
            .change_coverage
            .get(&scope.key())
            .map(|covered| {
                self.content_refs(scope)
                    .difference(covered)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default(); // Legacy missing baseline never announces history.
        let current_fingerprints = self.content_fingerprints(scope);
        let changes: BTreeSet<_> = changes
            .into_iter()
            .filter(|source| {
                let id = source.id();
                let eligible = match source {
                    WorkContentRef::Task { .. } => self
                        .tasks
                        .get(&id)
                        .is_some_and(|t| t.snoozed_until.is_none_or(|until| until <= now)),
                    _ => true,
                };
                eligible
                    && self
                        .change_fingerprints
                        .get(&scope.key())
                        .and_then(|f| f.get(&id))
                        != current_fingerprints.get(&id)
            })
            .collect();
        let changes = if self.preference_profile(access)?.reduce_followups {
            BTreeSet::new()
        } else {
            changes
        };
        if !briefing && coverage.is_empty() && changes.is_empty() {
            return Ok(None);
        }
        let mut task_ids: Vec<_> = if briefing {
            tasks.iter().map(|t| t.id).collect()
        } else {
            coverage.iter().map(|c| c.task_id).collect()
        };
        let mut content_refs: BTreeSet<_> = tasks
            .iter()
            .filter(|t| task_ids.contains(&t.id))
            .map(|t| WorkContentRef::Task {
                project: t.project_id,
                id: t.id,
                revision: t.revision,
            })
            .collect();
        // Explicit reminders and scheduled briefings take precedence over optional changes.
        let selected_changes = !briefing && coverage.is_empty();
        if selected_changes {
            content_refs = changes;
        }
        let mut ordered: Vec<_> = content_refs.into_iter().collect();
        ordered.sort_by_key(|source| match source {
            WorkContentRef::Task { id, .. } => {
                let task = &self.tasks[id];
                (
                    0,
                    std::cmp::Reverse(task.priority),
                    task.due_at.unwrap_or(u64::MAX),
                    *id,
                )
            }
            WorkContentRef::Decision { id, .. } => (1, std::cmp::Reverse(0), u64::MAX, *id),
        });
        let omitted = ordered.len().saturating_sub(8);
        ordered.truncate(8);
        content_refs = ordered.iter().cloned().collect();
        task_ids = ordered
            .iter()
            .filter_map(|r| match r {
                WorkContentRef::Task { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        let coverage: Vec<_> = coverage
            .into_iter()
            .filter(|c| task_ids.contains(&c.task_id))
            .collect();
        let (rendered_body, provenance) = self.render_delivery(&ordered, omitted)?;
        let mut dedupe_keys: BTreeSet<_> = coverage
            .iter()
            .map(|c| {
                format!(
                    "{}:{local_day}:reminder:{}:{}:{}",
                    scope.key(),
                    c.task_id,
                    c.reminder_revision,
                    c.remind_at
                )
            })
            .collect();
        if briefing {
            dedupe_keys.insert(briefing_key);
        }
        let revisions = tasks
            .iter()
            .filter(|t| task_ids.contains(&t.id))
            .map(|t| (t.id, t.revision))
            .collect();
        Ok(Some(WorkBatch {
            scope: scope.clone(),
            destination,
            target,
            content_refs,
            rendered_body,
            provenance,
            local_day,
            kind: if briefing {
                WorkDeliveryKind::Briefing
            } else if selected_changes {
                WorkDeliveryKind::Changes
            } else {
                WorkDeliveryKind::Reminder
            },
            task_ids,
            coverage,
            dedupe_keys,
            policy: policy.clone(),
            revisions,
            project_revisions: projects.iter().map(|p| (p.id, p.revision)).collect(),
        }))
    }

    /// Recheck local authority after reservation without replaying quota/dedupe.
    pub(crate) fn validate_reserved_batch(
        &self,
        access: WorkAccess,
        batch: &WorkBatch,
        now: u64,
    ) -> Result<(), WorkError> {
        let projects = self.scope_projects(&batch.scope, access, true)?;
        if self.scope_automation.get(&batch.scope.key()) != Some(&batch.policy)
            || !batch.policy.enabled
            || self.scope_automation_actors.get(&batch.scope.key()) != Some(&access.actor)
            || projects
                .iter()
                .map(|p| (p.id, p.revision))
                .collect::<Vec<_>>()
                != batch.project_revisions
            || batch
                .revisions
                .iter()
                .any(|(id, revision)| self.tasks.get(id).is_none_or(|t| t.revision != *revision))
        {
            return Err(WorkError::Stale);
        }
        self.target(&batch.scope, access, &batch.policy)?;
        let tz = batch
            .policy
            .timezone
            .parse::<Tz>()
            .map_err(|_| WorkError::Invalid)?;
        let local = utc(now)?.with_timezone(&tz);
        if batch.policy.quiet(local.hour())
            || local.format("%Y-%m-%d").to_string() != batch.local_day
        {
            return Err(WorkError::Stale);
        }
        Ok(())
    }

    /// Invoke inside commit_work with freshly obtained access facts, then wait
    /// for its durable result before sending. Attempts consume quota forever;
    /// recovery must mark unfinished attempts ReviewRequired without resending.
    pub fn reserve_batch(
        &mut self,
        access: WorkAccess,
        batch: &WorkBatch,
        now: u64,
    ) -> Result<u64, WorkError> {
        if matches!(batch.target, WorkDestination::TeamPrivate { .. }) {
            return Err(WorkError::Denied);
        }
        self.reserve_resolved_batch(access, batch, batch.destination, now)
    }

    /// A private target requires separately resolved transport; it never reuses
    /// the origin team channel or falls back after failure. Fresh origin access
    /// is required alongside the resolved DM channel supplied by the shell.
    pub fn reserve_private_batch(
        &mut self,
        access: WorkAccess,
        batch: &WorkBatch,
        dm_channel: u64,
        now: u64,
    ) -> Result<u64, WorkError> {
        if !matches!(batch.target, WorkDestination::TeamPrivate { principal } if principal == access.actor)
            || dm_channel == 0
            || dm_channel == access.channel
        {
            return Err(WorkError::Denied);
        }
        self.reserve_resolved_batch(access, batch, dm_channel, now)
    }

    fn reserve_resolved_batch(
        &mut self,
        access: WorkAccess,
        batch: &WorkBatch,
        transport_channel: u64,
        now: u64,
    ) -> Result<u64, WorkError> {
        if self.next_batch(&batch.scope, access, now)?.as_ref() != Some(batch) {
            return Err(WorkError::Stale);
        }
        if self.deliveries.len() >= 10_000 {
            return Err(WorkError::Full);
        }
        let id = self.next_id()?;
        self.deliveries.insert(
            id,
            WorkDeliveryReceipt {
                id,
                project_id: batch.project_revisions.first().ok_or(WorkError::Missing)?.0,
                recipient: transport_channel,
                destination: Some(batch.target.clone()),
                policy_revision: batch.policy.revision,
                provenance: Some(batch.provenance.clone()),
                local_day: batch.local_day.clone(),
                at: now,
                state: DeliveryState::Attempting,
                message_id: None,
                scope: Some(batch.scope.clone()),
                kind: Some(batch.kind),
                coverage: batch.coverage.clone(),
                task_ids: batch.task_ids.clone(),
                dedupe_keys: batch.dedupe_keys.clone(),
            },
        );
        let current_fingerprints = self.content_fingerprints(&batch.scope);
        let fingerprints = self
            .change_fingerprints
            .entry(batch.scope.key())
            .or_default();
        for source in &batch.content_refs {
            if let Some(fingerprint) = current_fingerprints.get(&source.id()) {
                fingerprints.insert(source.id(), fingerprint.clone());
            }
        }
        let covered = self.change_coverage.entry(batch.scope.key()).or_default();
        for source in &batch.content_refs {
            covered.retain(|old| !same_content(old, source));
            covered.insert(source.clone());
        }
        Ok(id)
    }
}

#[cfg(test)]
mod tests;

fn same_content(a: &WorkContentRef, b: &WorkContentRef) -> bool {
    match (a, b) {
        (WorkContentRef::Task { id: a, .. }, WorkContentRef::Task { id: b, .. })
        | (WorkContentRef::Decision { id: a, .. }, WorkContentRef::Decision { id: b, .. }) => {
            a == b
        }
        _ => false,
    }
}

#[cfg(test)]
mod delivery_tests;

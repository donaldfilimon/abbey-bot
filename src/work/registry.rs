//! Validated work transitions. Callers mutate a candidate snapshot, persist it,
//! and publish it only after observing the durable result.
use super::*;

const MAX_RECORDS: usize = 10_000;

impl WorkProject {
    pub fn authorize(&self, access: WorkAccess, manager: bool) -> Result<(), WorkError> {
        let correct_scope = match self.scope {
            WorkScope::Personal { owner } => access.guild.is_none() && access.actor == owner,
            WorkScope::Team { guild, channel } => {
                access.guild == Some(guild) && access.channel == channel && access.can_view
            }
        };
        if access.actor == 0
            || !correct_scope
            || !self.members.contains(&access.actor)
            || (manager && !self.managers.contains(&access.actor))
        {
            return Err(WorkError::Denied);
        }
        Ok(())
    }
}

fn valid_text(text: &str, max: usize) -> Result<(), WorkError> {
    if text.trim().is_empty()
        || text.chars().count() > max
        || text.chars().any(|c| c.is_control() && c != '\n')
    {
        Err(WorkError::Invalid)
    } else {
        Ok(())
    }
}

impl WorkStore {
    pub fn visible_projects(&self, access: WorkAccess) -> Vec<&WorkProject> {
        self.projects
            .values()
            .filter(|project| project.authorize(access, false).is_ok())
            .collect()
    }

    pub(super) fn next_id(&mut self) -> Result<u64, WorkError> {
        if self.projects.len() + self.goals.len() + self.tasks.len() + self.decisions.len()
            >= MAX_RECORDS
        {
            return Err(WorkError::Full);
        }
        self.sequence = self.sequence.checked_add(1).ok_or(WorkError::Full)?;
        Ok(self.sequence)
    }

    pub fn project(&self, id: u64, access: WorkAccess) -> Result<&WorkProject, WorkError> {
        let project = self.projects.get(&id).ok_or(WorkError::Missing)?;
        project.authorize(access, false)?;
        Ok(project)
    }

    pub fn create_project(
        &mut self,
        access: WorkAccess,
        name: &str,
        request_id: &str,
    ) -> Result<u64, WorkError> {
        valid_text(name, 100)?;
        if access.actor == 0
            || access.channel == 0
            || access.guild == Some(0)
            || request_id.is_empty()
            || request_id.len() > 100
        {
            return Err(WorkError::Invalid);
        }
        if access.guild.is_some() && (!access.can_view || !access.can_manage) {
            return Err(WorkError::Denied);
        }
        let key = format!(
            "{}:{}:{}:project:{request_id}",
            access.actor,
            access.guild.unwrap_or(0),
            access.channel
        );
        if let Some(id) = self.request_ids.get(&key) {
            return Ok(*id);
        }
        let id = self.next_id()?;
        let scope = access.guild.map_or(
            WorkScope::Personal {
                owner: access.actor,
            },
            |guild| WorkScope::Team {
                guild,
                channel: access.channel,
            },
        );
        self.projects.insert(
            id,
            WorkProject {
                id,
                name: name.trim().to_string(),
                scope,
                managers: BTreeSet::from([access.actor]),
                members: BTreeSet::from([access.actor]),
                revision: 0,
                allowed_github_repositories: BTreeSet::new(),
            },
        );
        self.request_ids.insert(key, id);
        Ok(id)
    }

    pub fn set_member(
        &mut self,
        project_id: u64,
        access: WorkAccess,
        user: u64,
        present: bool,
    ) -> Result<(), WorkError> {
        let project = self
            .projects
            .get_mut(&project_id)
            .ok_or(WorkError::Missing)?;
        project.authorize(access, true)?;
        if matches!(project.scope, WorkScope::Personal { .. })
            || user == 0
            || project.managers.contains(&user)
        {
            return Err(WorkError::Invalid);
        }
        if present {
            project.members.insert(user);
        } else {
            project.members.remove(&user);
        }
        project.revision = project.revision.checked_add(1).ok_or(WorkError::Full)?;
        Ok(())
    }

    pub fn add_goal(
        &mut self,
        project_id: u64,
        access: WorkAccess,
        title: &str,
        request_id: &str,
    ) -> Result<u64, WorkError> {
        self.project(project_id, access)?;
        valid_text(title, 300)?;
        if request_id.is_empty() || request_id.len() > 100 {
            return Err(WorkError::Invalid);
        }
        let key = format!("{}:{project_id}:goal:{request_id}", access.actor);
        if let Some(id) = self.request_ids.get(&key) {
            return Ok(*id);
        }
        let id = self.next_id()?;
        self.goals.insert(
            id,
            WorkGoal {
                id,
                project_id,
                title: title.trim().to_string(),
                owner: access.actor,
                complete: false,
            },
        );
        self.request_ids.insert(key, id);
        Ok(id)
    }

    pub fn add_task(
        &mut self,
        access: WorkAccess,
        mut task: WorkTask,
        request_id: &str,
    ) -> Result<u64, WorkError> {
        let project = self.project(task.project_id, access)?;
        valid_text(&task.title, 300)?;
        if task.priority > 3
            || task.github.as_ref().is_some_and(|reference| {
                reference.validate().is_err()
                    || !project.allows_github_repository(&reference.repository)
            })
            || task
                .assignee
                .is_some_and(|id| !project.members.contains(&id))
            || task.goal_id.is_some_and(|id| {
                self.goals
                    .get(&id)
                    .is_none_or(|goal| goal.project_id != task.project_id)
            })
            || task
                .source
                .as_ref()
                .is_some_and(|source| source.len() > 500 || !source.starts_with("https://"))
            || request_id.is_empty()
            || request_id.len() > 100
        {
            return Err(WorkError::Invalid);
        }
        let key = format!("{}:{}:task:{request_id}", access.actor, task.project_id);
        if let Some(id) = self.request_ids.get(&key) {
            return Ok(*id);
        }
        let id = self.next_id()?;
        if let Some(reference) = &mut task.github {
            reference.repository = reference.repository.canonical();
        }
        task.id = id;
        task.owner = access.actor;
        task.revision = 0;
        self.tasks.insert(id, task);
        self.request_ids.insert(key, id);
        Ok(id)
    }

    pub fn update_task(
        &mut self,
        access: WorkAccess,
        id: u64,
        revision: u64,
        status: WorkStatus,
        snoozed_until: Option<u64>,
    ) -> Result<u64, WorkError> {
        let task = self.tasks.get(&id).ok_or(WorkError::Missing)?;
        self.project(task.project_id, access)?;
        if task.revision != revision {
            return Err(WorkError::Stale);
        }
        let task = self.tasks.get_mut(&id).ok_or(WorkError::Missing)?;
        let next = task.revision.checked_add(1).ok_or(WorkError::Full)?;
        task.status = status;
        task.snoozed_until = snoozed_until;
        task.revision = next;
        self.recall.task_changed(task.project_id, id, next);
        Ok(next)
    }

    pub fn record_decision(
        &mut self,
        project_id: u64,
        access: WorkAccess,
        text: &str,
        at: u64,
        request_id: &str,
    ) -> Result<u64, WorkError> {
        self.project(project_id, access)?;
        valid_text(text, 1_000)?;
        if request_id.is_empty() || request_id.len() > 100 {
            return Err(WorkError::Invalid);
        }
        let key = format!("{}:{project_id}:decision:{request_id}", access.actor);
        if let Some(id) = self.request_ids.get(&key) {
            return Ok(*id);
        }
        let id = self.next_id()?;
        self.decisions.insert(
            id,
            WorkDecision {
                id,
                project_id,
                author: access.actor,
                text: text.trim().to_string(),
                at,
            },
        );
        self.request_ids.insert(key, id);
        Ok(id)
    }

    pub fn briefing(
        &self,
        project_id: u64,
        access: WorkAccess,
        now: u64,
    ) -> Result<String, WorkError> {
        let project = self.project(project_id, access)?;
        let mut tasks: Vec<_> = self
            .tasks
            .values()
            .filter(|task| {
                task.project_id == project_id
                    && !matches!(task.status, WorkStatus::Done | WorkStatus::Cancelled)
            })
            .collect();
        tasks.sort_by_key(|task| {
            (
                std::cmp::Reverse(task.priority),
                task.due_at.unwrap_or(u64::MAX),
                task.id,
            )
        });
        let mut out = format!(
            "**{} · work briefing**\nRecorded work, viewed <t:{now}:R>.\n",
            project.name
        );
        if tasks.is_empty() {
            out.push_str("No open tasks. Add one with `/work task`.\n");
        }
        for task in tasks.iter().take(10) {
            out.push_str(&format!(
                "\n#{} · {} · {}{} · revision {}",
                task.id,
                task.status.label(),
                task.title,
                task.due_at
                    .map_or(String::new(), |due| format!(" · due <t:{due}:R>")),
                task.revision
            ));
            if let Some(reference) = &task.github
                && project.allows_github_repository(&reference.repository)
            {
                let url = reference.url();
                if let Some(snapshot) = self.github_snapshot(reference) {
                    let title = github::inert_title(&snapshot.title);
                    let state = match snapshot.state {
                        GitHubState::Open => "open",
                        GitHubState::Closed => "closed",
                        GitHubState::Merged => "merged",
                    };
                    let freshness = if snapshot.stale { "stale" } else { "refreshed" };
                    out.push_str(&format!(
                        "\n  GitHub: {title} ({state}, {freshness} <t:{}:R>) · {url}",
                        snapshot.refreshed_at
                    ));
                } else {
                    out.push_str(&format!("\n  GitHub: awaiting first refresh · {url}"));
                }
            }
        }
        for decision in self
            .decisions
            .values()
            .rev()
            .filter(|decision| decision.project_id == project_id)
            .take(3)
        {
            out.push_str(&format!("\nDecision #{}: {}", decision.id, decision.text));
        }
        Ok(out)
    }
}

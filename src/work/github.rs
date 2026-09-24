//! Pure repository allowlist and snapshot policy. Repository text is data;
//! only the typed reference and manager-granted allowlist affect access.
//! Repository identity folds ASCII case for GitHub owner and name while the
//! installation ID remains exact. New links and cache keys use lowercase;
//! persisted mixed-case links and snapshots remain readable and revocable.
//! Legacy URL migration is explicit and manager-only, and accepts canonical
//! issue/PR URLs only when one installed repository identity matches.
//! A failed read marks the retained snapshot stale; a conditional 304 may
//! advance its observation time. Network polling and issue writes are separate
//! shell work and never derive authority from GitHub title/body text.
use super::*;

impl GitHubRepository {
    pub fn validate(&self) -> Result<(), WorkError> {
        if self.installation == 0 || !valid_part(&self.owner) || !valid_part(&self.name) {
            return Err(WorkError::Invalid);
        }
        Ok(())
    }

    pub fn canonical(&self) -> Self {
        Self {
            installation: self.installation,
            owner: self.owner.to_ascii_lowercase(),
            name: self.name.to_ascii_lowercase(),
        }
    }

    pub fn same_identity(&self, other: &Self) -> bool {
        self.installation == other.installation
            && self.owner.eq_ignore_ascii_case(&other.owner)
            && self.name.eq_ignore_ascii_case(&other.name)
    }
}

fn valid_part(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part.len() <= 100
        && part
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// GitHub titles are untrusted display data, never commands or policy input.
pub(super) fn inert_title(title: &str) -> String {
    let mut out = String::new();
    for ch in title.chars().filter(|ch| !ch.is_control()).take(120) {
        match ch {
            '@' => out.push_str("@\u{200b}"),
            '\\' | '*' | '_' | '~' | '`' | '|' | '<' | '>' | '[' | ']' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

fn parse_legacy_source(
    source: &str,
    repositories: &BTreeSet<GitHubRepository>,
) -> Option<GitHubReference> {
    let path = source.strip_prefix("https://github.com/")?;
    let mut parts = path.split('/');
    let (Some(owner), Some(name), Some(kind), Some(number), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return None;
    };
    if !valid_part(owner)
        || !valid_part(name)
        || number.starts_with('0')
        || !number.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let number = number.parse::<u64>().ok().filter(|number| *number > 0)?;
    let kind = match kind {
        "issues" => GitHubItemKind::Issue,
        "pull" => GitHubItemKind::PullRequest,
        _ => return None,
    };
    let mut matches = repositories.iter().filter(|repo| {
        repo.owner.eq_ignore_ascii_case(owner) && repo.name.eq_ignore_ascii_case(name)
    });
    let repository = matches.next()?.canonical();
    if matches.any(|candidate| !candidate.same_identity(&repository)) {
        return None;
    }
    Some(GitHubReference {
        repository,
        kind,
        number,
    })
}

impl GitHubReference {
    fn same_identity(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.number == other.number
            && self.repository.same_identity(&other.repository)
    }

    pub fn validate(&self) -> Result<(), WorkError> {
        self.repository.validate()?;
        if self.number == 0 {
            return Err(WorkError::Invalid);
        }
        Ok(())
    }

    pub fn key(&self) -> String {
        let kind = match self.kind {
            GitHubItemKind::Issue => "issues",
            GitHubItemKind::PullRequest => "pull",
        };
        let repository = self.repository.canonical();
        format!(
            "{}:{}/{}/{kind}/{}",
            repository.installation, repository.owner, repository.name, self.number
        )
    }

    pub fn url(&self) -> String {
        let kind = match self.kind {
            GitHubItemKind::Issue => "issues",
            GitHubItemKind::PullRequest => "pull",
        };
        let repository = self.repository.canonical();
        format!(
            "https://github.com/{}/{}/{kind}/{}",
            repository.owner, repository.name, self.number
        )
    }
}

impl WorkStore {
    pub fn github_snapshot(&self, reference: &GitHubReference) -> Option<&GitHubSnapshot> {
        let key = reference.key();
        self.github_snapshots
            .iter()
            .filter(|(candidate, _)| candidate.eq_ignore_ascii_case(&key))
            .map(|(_, snapshot)| snapshot)
            .max_by_key(|snapshot| snapshot.refreshed_at)
    }

    /// Explicit manager migration of old URL sources. Ambiguous installations
    /// and noncanonical URLs stay as inert legacy sources.
    pub fn migrate_github_sources(
        &mut self,
        project_id: u64,
        access: WorkAccess,
    ) -> Result<usize, WorkError> {
        let project = self.projects.get(&project_id).ok_or(WorkError::Missing)?;
        project.authorize(access, true)?;
        let mut changes = Vec::new();
        for task in self
            .tasks
            .values()
            .filter(|task| task.project_id == project_id && task.github.is_none())
        {
            if let Some(reference) = task.source.as_deref().and_then(|source| {
                parse_legacy_source(source, &project.allowed_github_repositories)
            }) {
                task.revision.checked_add(1).ok_or(WorkError::Full)?;
                changes.push((task.id, reference));
            }
        }
        let count = changes.len();
        for (id, reference) in changes {
            let task = self.tasks.get_mut(&id).ok_or(WorkError::Missing)?;
            task.github = Some(reference);
            task.revision += 1;
        }
        Ok(count)
    }

    pub fn allow_github_repository(
        &mut self,
        project_id: u64,
        access: WorkAccess,
        repository: GitHubRepository,
        allowed: bool,
    ) -> Result<(), WorkError> {
        repository.validate()?;
        let repository = repository.canonical();
        let project = self
            .projects
            .get_mut(&project_id)
            .ok_or(WorkError::Missing)?;
        project.authorize(access, true)?;
        let next_revision = project.revision.checked_add(1).ok_or(WorkError::Full)?;
        project
            .allowed_github_repositories
            .retain(|candidate| !candidate.same_identity(&repository));
        if allowed {
            project.allowed_github_repositories.insert(repository);
        }
        project.revision = next_revision;
        Ok(())
    }

    pub fn link_github(
        &mut self,
        task_id: u64,
        revision: u64,
        access: WorkAccess,
        mut reference: GitHubReference,
    ) -> Result<(), WorkError> {
        reference.validate()?;
        let task = self.tasks.get(&task_id).ok_or(WorkError::Missing)?;
        let project = self.project(task.project_id, access)?;
        if !project.allows_github_repository(&reference.repository) {
            return Err(WorkError::Denied);
        }
        if task.revision != revision {
            return Err(WorkError::Stale);
        }
        let next_revision = task.revision.checked_add(1).ok_or(WorkError::Full)?;
        let task = self.tasks.get_mut(&task_id).ok_or(WorkError::Missing)?;
        reference.repository = reference.repository.canonical();
        task.github = Some(reference);
        task.revision = next_revision;
        self.recall
            .task_changed(task.project_id, task_id, next_revision);
        Ok(())
    }

    pub fn record_github_snapshot(
        &mut self,
        reference: &GitHubReference,
        mut snapshot: GitHubSnapshot,
    ) -> Result<(), WorkError> {
        reference.validate()?;
        if snapshot.title.trim().is_empty()
            || snapshot.title.len() > 500
            || snapshot.etag.as_ref().is_some_and(|etag| etag.len() > 500)
        {
            return Err(WorkError::Invalid);
        }
        if !self.active_github_link(reference) {
            return Err(WorkError::Missing);
        }
        if self
            .github_snapshot(reference)
            .is_some_and(|previous| snapshot.refreshed_at < previous.refreshed_at)
        {
            return Err(WorkError::Stale);
        }
        if self.github_snapshot(reference).is_none() && self.github_snapshots.len() >= 10_000 {
            return Err(WorkError::Full);
        }
        snapshot.stale = false;
        let key = reference.key();
        self.github_snapshots
            .retain(|candidate, _| !candidate.eq_ignore_ascii_case(&key));
        self.github_snapshots.insert(key, snapshot);
        Ok(())
    }

    /// A 304 confirms that the retained snapshot is current without replacing
    /// its title or state. The caller supplies the observation time.
    pub fn confirm_github_not_modified(
        &mut self,
        reference: &GitHubReference,
        refreshed_at: u64,
    ) -> Result<(), WorkError> {
        reference.validate()?;
        if !self.active_github_link(reference) {
            return Err(WorkError::Missing);
        }
        let snapshot = self.github_snapshot(reference).ok_or(WorkError::Missing)?;
        if snapshot.etag.is_none() || refreshed_at < snapshot.refreshed_at {
            return Err(WorkError::Stale);
        }
        let mut snapshot = snapshot.clone();
        snapshot.refreshed_at = refreshed_at;
        snapshot.stale = false;
        let key = reference.key();
        self.github_snapshots
            .retain(|candidate, _| !candidate.eq_ignore_ascii_case(&key));
        self.github_snapshots.insert(key, snapshot);
        Ok(())
    }

    pub fn mark_github_stale(&mut self, reference: &GitHubReference) {
        let key = reference.key();
        for (candidate, snapshot) in &mut self.github_snapshots {
            if candidate.eq_ignore_ascii_case(&key) {
                snapshot.stale = true;
            }
        }
    }

    pub fn active_github_link(&self, reference: &GitHubReference) -> bool {
        self.tasks.values().any(|task| {
            task.github
                .as_ref()
                .is_some_and(|linked| linked.same_identity(reference))
                && self
                    .projects
                    .get(&task.project_id)
                    .is_some_and(|project| project.allows_github_repository(&reference.repository))
        })
    }
}

impl WorkProject {
    pub fn allows_github_repository(&self, repository: &GitHubRepository) -> bool {
        self.allowed_github_repositories
            .iter()
            .any(|candidate| candidate.same_identity(repository))
    }
}

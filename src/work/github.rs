//! Pure repository allowlist and snapshot policy. Repository text is data;
//! only the typed reference and manager-granted allowlist affect access.
use super::*;

impl GitHubRepository {
    pub fn validate(&self) -> Result<(), WorkError> {
        if self.installation == 0 || !valid_part(&self.owner) || !valid_part(&self.name) {
            return Err(WorkError::Invalid);
        }
        Ok(())
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
    if !valid_part(owner) || !valid_part(name) || number.starts_with('0') {
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
    let repository = matches.next()?.clone();
    if matches.next().is_some() {
        return None;
    }
    Some(GitHubReference {
        repository,
        kind,
        number,
    })
}

impl GitHubReference {
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
        format!(
            "{}:{}/{}/{kind}/{}",
            self.repository.installation, self.repository.owner, self.repository.name, self.number
        )
    }

    pub fn url(&self) -> String {
        let kind = match self.kind {
            GitHubItemKind::Issue => "issues",
            GitHubItemKind::PullRequest => "pull",
        };
        format!(
            "https://github.com/{}/{}/{kind}/{}",
            self.repository.owner, self.repository.name, self.number
        )
    }
}

impl WorkStore {
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
        let project = self
            .projects
            .get_mut(&project_id)
            .ok_or(WorkError::Missing)?;
        project.authorize(access, true)?;
        let next_revision = project.revision.checked_add(1).ok_or(WorkError::Full)?;
        if allowed {
            project.allowed_github_repositories.insert(repository);
        } else {
            project.allowed_github_repositories.remove(&repository);
        }
        project.revision = next_revision;
        Ok(())
    }

    pub fn link_github(
        &mut self,
        task_id: u64,
        revision: u64,
        access: WorkAccess,
        reference: GitHubReference,
    ) -> Result<(), WorkError> {
        reference.validate()?;
        let task = self.tasks.get(&task_id).ok_or(WorkError::Missing)?;
        let project = self.project(task.project_id, access)?;
        if !project
            .allowed_github_repositories
            .contains(&reference.repository)
        {
            return Err(WorkError::Denied);
        }
        if task.revision != revision {
            return Err(WorkError::Stale);
        }
        let next_revision = task.revision.checked_add(1).ok_or(WorkError::Full)?;
        let task = self.tasks.get_mut(&task_id).ok_or(WorkError::Missing)?;
        task.github = Some(reference);
        task.revision = next_revision;
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
            .github_snapshots
            .get(&reference.key())
            .is_some_and(|previous| snapshot.refreshed_at < previous.refreshed_at)
        {
            return Err(WorkError::Stale);
        }
        if !self.github_snapshots.contains_key(&reference.key())
            && self.github_snapshots.len() >= 10_000
        {
            return Err(WorkError::Full);
        }
        snapshot.stale = false;
        self.github_snapshots.insert(reference.key(), snapshot);
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
        let snapshot = self
            .github_snapshots
            .get_mut(&reference.key())
            .ok_or(WorkError::Missing)?;
        if snapshot.etag.is_none() || refreshed_at < snapshot.refreshed_at {
            return Err(WorkError::Stale);
        }
        snapshot.refreshed_at = refreshed_at;
        snapshot.stale = false;
        Ok(())
    }

    pub fn mark_github_stale(&mut self, reference: &GitHubReference) {
        if let Some(snapshot) = self.github_snapshots.get_mut(&reference.key()) {
            snapshot.stale = true;
        }
    }

    pub fn active_github_link(&self, reference: &GitHubReference) -> bool {
        self.tasks.values().any(|task| {
            task.github.as_ref() == Some(reference)
                && self.projects.get(&task.project_id).is_some_and(|project| {
                    project
                        .allowed_github_repositories
                        .contains(&reference.repository)
                })
        })
    }
}

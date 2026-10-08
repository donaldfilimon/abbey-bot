//! Explicit forum resolution policy. No transport or live identities.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionState {
    Solved,
    Unresolved,
}

#[derive(Debug, Clone, Copy)]
pub struct ResolutionFacts<'a> {
    pub is_thread_author: bool,
    pub can_manage_thread: bool,
    pub current_tags: &'a [u64],
    pub solved_tag_id: Option<u64>,
    pub unresolved_tag_id: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionError {
    Denied,
    InvalidTags,
}

pub fn resolution_tags(
    facts: ResolutionFacts<'_>,
    desired: ResolutionState,
) -> Result<Vec<u64>, ResolutionError> {
    if !facts.is_thread_author && !facts.can_manage_thread {
        return Err(ResolutionError::Denied);
    }
    let (Some(solved), Some(unresolved)) = (facts.solved_tag_id, facts.unresolved_tag_id) else {
        return Err(ResolutionError::InvalidTags);
    };
    if solved == 0 || unresolved == 0 || solved == unresolved || facts.current_tags.len() > 5 {
        return Err(ResolutionError::InvalidTags);
    }
    for (index, id) in facts.current_tags.iter().enumerate() {
        if *id == 0 || facts.current_tags[..index].contains(id) {
            return Err(ResolutionError::InvalidTags);
        }
    }
    let selected = match desired {
        ResolutionState::Solved => solved,
        ResolutionState::Unresolved => unresolved,
    };
    let mut tags: Vec<u64> = facts
        .current_tags
        .iter()
        .copied()
        .filter(|id| (*id != solved && *id != unresolved) || *id == selected)
        .collect();
    if !tags.contains(&selected) {
        tags.push(selected);
    }
    if tags.len() > 5 {
        return Err(ResolutionError::InvalidTags);
    }
    Ok(tags)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(tags: &[u64]) -> ResolutionFacts<'_> {
        ResolutionFacts {
            is_thread_author: true,
            can_manage_thread: false,
            current_tags: tags,
            solved_tag_id: Some(10),
            unresolved_tag_id: Some(11),
        }
    }

    #[test]
    fn same_name_without_authority_denied() {
        let mut input = facts(&[11, 20]);
        input.is_thread_author = false;
        assert_eq!(
            resolution_tags(input, ResolutionState::Solved),
            Err(ResolutionError::Denied)
        );
    }

    #[test]
    fn resolve_preserves_unrelated_tags() {
        assert_eq!(
            resolution_tags(facts(&[20, 11, 21, 22]), ResolutionState::Solved),
            Ok(vec![20, 21, 22, 10])
        );
        assert_eq!(
            resolution_tags(facts(&[20, 10, 11, 21]), ResolutionState::Unresolved),
            Ok(vec![20, 11, 21])
        );
    }

    #[test]
    fn current_manager_can_resolve_without_being_author() {
        let mut input = facts(&[11]);
        input.is_thread_author = false;
        input.can_manage_thread = true;
        assert_eq!(
            resolution_tags(input, ResolutionState::Solved),
            Ok(vec![10])
        );
    }

    #[test]
    fn missing_or_duplicate_status_tags_refuse() {
        for (solved, unresolved) in [
            (None, Some(11)),
            (Some(10), None),
            (Some(10), Some(10)),
            (Some(0), Some(11)),
        ] {
            let mut input = facts(&[20]);
            input.solved_tag_id = solved;
            input.unresolved_tag_id = unresolved;
            assert_eq!(
                resolution_tags(input, ResolutionState::Solved),
                Err(ResolutionError::InvalidTags)
            );
        }
    }

    #[test]
    fn tag_capacity_never_drops_unrelated_tags() {
        assert_eq!(
            resolution_tags(facts(&[20, 21, 22, 23, 24]), ResolutionState::Solved),
            Err(ResolutionError::InvalidTags)
        );
        assert_eq!(
            resolution_tags(facts(&[20, 21, 22, 23, 11]), ResolutionState::Solved),
            Ok(vec![20, 21, 22, 23, 10])
        );
    }

    #[test]
    fn corrupt_current_tags_refuse() {
        for tags in [vec![0], vec![20, 20], vec![20, 21, 22, 23, 24, 25]] {
            assert_eq!(
                resolution_tags(facts(&tags), ResolutionState::Solved),
                Err(ResolutionError::InvalidTags)
            );
        }
    }

    #[test]
    fn already_selected_status_is_idempotent() {
        let current = [20, 10, 21];
        assert_eq!(
            resolution_tags(facts(&current), ResolutionState::Solved),
            Ok(current.to_vec())
        );
        assert_eq!(
            resolution_tags(facts(&[]), ResolutionState::Unresolved),
            Ok(vec![11])
        );
    }
}

//! Pure authorization and rendering for member memory reads.

use crate::command_catalog::{
    self, AccessId, DiscordPermission, EligibilityInput, InteractionContext,
};
use crate::memory::PendingSupersession;

pub const MAX_CARD_FACTS: usize = 6;
pub const MAX_CARD_PENDING: usize = 2;
const MAX_CARD_ITEM_CHARS: usize = 160;

/// The one A1 authorization used by every Discord memory adapter.
pub fn subject_authorized(
    actor_id: u64,
    subject_id: u64,
    permissions: &[DiscordPermission],
) -> bool {
    let mut input = EligibilityInput::new(InteractionContext::Guild);
    input.self_subject = Some(actor_id == subject_id);
    input.permissions = permissions.to_vec();
    command_catalog::access_allows(AccessId::A1.rule(), &input)
}

pub struct MemoryCard<'a> {
    pub subject_id: u64,
    pub facts: &'a [String],
    pub pending: &'a [PendingSupersession],
    pub standing: f64,
}

fn item(text: &str) -> String {
    let mut chars = text.chars();
    let mut bounded: String = chars.by_ref().take(MAX_CARD_ITEM_CHARS).collect();
    if chars.next().is_some() {
        bounded.push('…');
    }
    bounded
}

/// A bounded canonical view shared by `/recall` and `Abbey: memory`.
pub fn render(card: &MemoryCard<'_>) -> String {
    let mut out = format!(
        "**<@{}>** — standing {:.2} (0 = poor, 1 = excellent)\n",
        card.subject_id,
        card.standing.clamp(0.0, 1.0)
    );
    out.push_str("Facts:\n");
    if card.facts.is_empty() {
        out.push_str("• No facts on record.\n");
    } else {
        for fact in card.facts.iter().take(MAX_CARD_FACTS) {
            out.push_str("• ");
            out.push_str(&item(fact));
            out.push('\n');
        }
        if card.facts.len() > MAX_CARD_FACTS {
            out.push_str(&format!(
                "• …and {} more.\n",
                card.facts.len() - MAX_CARD_FACTS
            ));
        }
    }
    out.push_str("Pending replacements:\n");
    if card.pending.is_empty() {
        out.push_str("• None.");
    } else {
        for pending in card.pending.iter().take(MAX_CARD_PENDING) {
            out.push_str("• ");
            out.push_str(&item(&pending.old_fact));
            out.push_str(" → ");
            out.push_str(&item(&pending.new_fact));
            out.push('\n');
        }
        if card.pending.len() > MAX_CARD_PENDING {
            out.push_str(&format!(
                "• …and {} more. Use `/pending list` to review them.",
                card.pending.len() - MAX_CARD_PENDING
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a1_is_self_or_current_memory_moderator() {
        assert!(subject_authorized(1, 1, &[]));
        assert!(!subject_authorized(1, 2, &[]));
        assert!(subject_authorized(
            1,
            2,
            &[DiscordPermission::ManageMessages]
        ));
        assert!(subject_authorized(1, 2, &[DiscordPermission::ManageServer]));
        assert!(subject_authorized(
            1,
            2,
            &[DiscordPermission::Administrator]
        ));
        assert!(!subject_authorized(
            1,
            2,
            &[DiscordPermission::ModerateMembers]
        ));
    }

    #[test]
    fn card_bounds_facts_and_pending_separately() {
        let facts = (0..20).map(|n| format!("fact {n}")).collect::<Vec<_>>();
        let pending = (0..9)
            .map(|n| PendingSupersession {
                old_fact: format!("old {n}"),
                new_fact: format!("new {n}"),
                at: n,
            })
            .collect::<Vec<_>>();
        let rendered = render(&MemoryCard {
            subject_id: 42,
            facts: &facts,
            pending: &pending,
            standing: 3.0,
        });
        assert!(rendered.contains("standing 1.00"));
        assert!(rendered.contains("…and 14 more"));
        assert!(rendered.contains("…and 7 more"));
        assert!(!rendered.contains("fact 6"));
        assert!(!rendered.contains("old 2"));
        assert!(rendered.chars().count() < 2_000);
    }

    #[test]
    fn maximum_length_items_leave_both_sections_visible() {
        let facts = vec!["f".repeat(300); 20];
        let pending = vec![
            PendingSupersession {
                old_fact: "o".repeat(300),
                new_fact: "n".repeat(300),
                at: 1,
            };
            9
        ];
        let rendered = render(&MemoryCard {
            subject_id: u64::MAX,
            facts: &facts,
            pending: &pending,
            standing: 0.5,
        });
        assert!(rendered.contains("Pending replacements:"));
        assert!(rendered.contains("Use `/pending list`"));
        assert!(rendered.chars().count() < 2_000);
    }
}

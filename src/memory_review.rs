//! Pure decisions for the memory review commands (`/admin quarantine`,
//! `/admin resolve`), the first memory-edge emitter (amendment 2026-09-16).
//!
//! A quarantine never hides or deletes the fact; it records in the WDBX
//! ledger that a moderator considers it suspect. Only a human with a
//! governance role may review, and WDBX refuses a resolution from anyone
//! else, so the role check here and the ledger's check agree.

use crate::command_catalog::DiscordPermission;
use crate::episode_gate::{GateOutcome, Reviewer};

pub const NOT_REVIEWER: &str = "Only the server owner, or a member Discord currently grants Administrator or Manage Server, can review memory.";
pub const NO_GATE: &str = "Memory review needs the constitutional episode gate, which is not configured for this server. Nothing was recorded.";
pub const NO_RECEIPT: &str = "That fact has no ledger receipt (it was stored before the episode gate covered this server), so there is nothing to quarantine. Nothing was recorded.";
pub const BAD_EDGE: &str =
    "That is not an edge digest. Paste the 64-character hex digest from the quarantine reply.";

/// The governance role a reviewer holds, strongest first. `None` means the
/// invoker may not review.
pub fn reviewer_for(is_owner: bool, permissions: &[DiscordPermission]) -> Option<Reviewer> {
    if is_owner {
        Some(Reviewer::Owner)
    } else if permissions.contains(&DiscordPermission::Administrator) {
        Some(Reviewer::Administrator)
    } else if permissions.contains(&DiscordPermission::ManageServer) {
        Some(Reviewer::Manager)
    } else {
        None
    }
}

pub fn not_found(subject_id: u64) -> String {
    format!("No fact by that wording is on record for <@{subject_id}>. Nothing was recorded.")
}

fn refused(outcome: &GateOutcome) -> Option<String> {
    match outcome {
        GateOutcome::Appended { .. } => None,
        GateOutcome::Rejected { detail } => Some(format!(
            "The ledger refused it, and nothing was recorded (`{detail}`)."
        )),
        GateOutcome::Unavailable { detail } => Some(format!(
            "The ledger did not answer, so it is unknown whether anything was recorded (`{detail}`)."
        )),
    }
}

pub fn quarantine_reply(outcome: &GateOutcome, subject_id: u64) -> String {
    if let Some(message) = refused(outcome) {
        return format!(
            "{message} A fact that is already quarantined, or has been forgotten, cannot be quarantined again."
        );
    }
    let GateOutcome::Appended { digest_hex, .. } = outcome else {
        unreachable!("refused covers every other outcome");
    };
    format!(
        "Quarantined a fact about <@{subject_id}>. It stays on record and visible; the ledger now marks it suspect.\n\
         To close this review: `/admin resolve edge:{digest_hex}`"
    )
}

pub fn resolve_reply(outcome: &GateOutcome, valid: bool) -> String {
    if let Some(message) = refused(outcome) {
        return format!(
            "{message} Only an open quarantine or contradiction can be resolved, and only once."
        );
    }
    let verdict = if valid { "valid" } else { "invalid" };
    format!(
        "Review closed: the fact was judged **{verdict}**. Nothing was deleted; use `/forget` if it should go."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn appended() -> GateOutcome {
        GateOutcome::Appended {
            digest_hex: "ab".repeat(32),
            sequence: "7".into(),
        }
    }

    #[test]
    fn only_governance_roles_review_and_the_strongest_wins() {
        use DiscordPermission::*;
        assert_eq!(reviewer_for(true, &[]), Some(Reviewer::Owner));
        assert_eq!(reviewer_for(true, &[Administrator]), Some(Reviewer::Owner));
        assert_eq!(
            reviewer_for(false, &[ManageServer, Administrator]),
            Some(Reviewer::Administrator)
        );
        assert_eq!(
            reviewer_for(false, &[ManageServer]),
            Some(Reviewer::Manager)
        );
        // Moderation rights over messages or members are not memory review.
        assert_eq!(
            reviewer_for(false, &[ManageMessages, ModerateMembers, ManageRoles]),
            None
        );
        assert_eq!(reviewer_for(false, &[]), None);
    }

    #[test]
    fn a_quarantine_reply_hands_back_the_edge_to_resolve() {
        let reply = quarantine_reply(&appended(), 42);
        println!("{reply}");
        assert!(reply.contains("<@42>"));
        assert!(reply.contains("stays on record"));
        assert!(reply.contains(&format!("`/admin resolve edge:{}`", "ab".repeat(32))));
    }

    #[test]
    fn refusals_say_nothing_was_recorded_or_that_it_is_unknown() {
        let rejected = GateOutcome::Rejected {
            detail: "FailedPrecondition: episode_transition_invalid".into(),
        };
        let quarantine = quarantine_reply(&rejected, 42);
        println!("{quarantine}");
        assert!(quarantine.contains("nothing was recorded"));
        assert!(quarantine.contains("episode_transition_invalid"));
        assert!(!quarantine.contains("/admin resolve"));

        let unavailable = GateOutcome::Unavailable {
            detail: "timed out".into(),
        };
        let resolve = resolve_reply(&unavailable, true);
        println!("{resolve}");
        assert!(resolve.contains("unknown whether anything was recorded"));
        assert!(!resolve.contains("Review closed"));
    }

    #[test]
    fn a_resolution_names_the_verdict_and_deletes_nothing() {
        let valid = resolve_reply(&appended(), true);
        let invalid = resolve_reply(&appended(), false);
        println!("{valid}\n{invalid}");
        assert!(valid.contains("**valid**"));
        assert!(invalid.contains("**invalid**"));
        assert!(invalid.contains("Nothing was deleted"));
    }
}

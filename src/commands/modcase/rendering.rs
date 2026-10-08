//! Bounded content-free human copy; native callers prove private access before rendering.
use crate::moderation::shadow::{
    AppealDecision, Case, Counts, Disposition, HumanAssessment, ReviewDecision,
};
pub(super) fn case(case: &Case, counts: Option<Counts>) -> String {
    let assessment = match case.assessment {
        HumanAssessment::ConfirmedOffending => "confirmed offending in context",
        HumanAssessment::Ambiguous => "ambiguous; human context review",
        HumanAssessment::QuotationOrReport => "quotation or report; human context review",
    };
    let proposal = match case.disposition {
        Disposition::HumanReview => {
            "No deletion or timeout proposed; human context review required.".into()
        }
        Disposition::ConfirmedProposal {
            timeout_minutes: Some(minutes),
        } => format!(
            "Human proposal: delete the attributed message; timeout at most {minutes} minutes."
        ),
        Disposition::ConfirmedProposal {
            timeout_minutes: None,
        } => "Human proposal: delete the attributed message; no timeout proposed.".into(),
    };
    let review = case
        .review
        .as_ref()
        .map_or("Not independently reviewed.", |r| match r.decision {
            ReviewDecision::Agree => "Independent human review: agrees.",
            ReviewDecision::Disagree => "Independent human review: disagrees.",
            ReviewDecision::NeedsContext => {
                "Independent human review: needs context; original evidence is not proved."
            }
        });
    let appeal = case
        .appeal
        .as_ref()
        .map_or("No subject appeal recorded.", |a| {
            a.resolution
                .as_ref()
                .map_or("Subject appeal is open.", |r| match r.decision {
                    AppealDecision::Upheld => "Subject appeal upheld by an independent moderator.",
                    AppealDecision::Rejected => {
                        "Subject appeal rejected by an independent moderator."
                    }
                    AppealDecision::NeedsContext => {
                        "Appeal resolution needs context; original evidence is not proved."
                    }
                })
        });
    let aggregate = counts.map_or_else(String::new, |counts| format!(
        "\nHuman assessment records: proposals {}; context referrals {}.\nIndependent human reviews: agree {}; disagree {}; needs context {}; unreviewed {}.\nSubject appeals: open {}; upheld {}; rejected {}; needs context {}. These counts measure human records, not classifier accuracy or enforcement readiness.",
        counts.confirmed_proposals, counts.human_review_referrals,
        counts.review_agree, counts.review_disagree, counts.review_needs_context, counts.unreviewed,
        counts.appeal_open, counts.appeal_upheld, counts.appeal_rejected, counts.appeal_needs_context
    ));
    format!(
        "Shadow case: {}\nRevision: {}\nSource: https://discord.com/channels/{}/{}/{}\nHuman assessment: {}. Abbey did not independently classify the message.\n{}\n{}\n{}\nNo action taken. This operational evidence is separate from personal memory; /forget_learning does not delete it. A live moderation pilot, independent classifier agreement and enforcement activation remain unqualified.{}",
        case.id,
        case.revision,
        case.source.guild,
        case.source.channel,
        case.source.message,
        assessment,
        proposal,
        review,
        appeal,
        aggregate
    )
}

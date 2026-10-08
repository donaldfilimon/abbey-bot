//! Bounded correction admission. Caller-fed attribution, time and authority;
//! repair grants no factual or tool authority and never retains raw asks.
use super::ask_signature::AskSignature;
use super::reward::FeedbackAttribution;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CorrectionDecision {
    Ignore,
    Repair { turn_id: u64 },
}

pub fn evaluate_correction(
    attribution: FeedbackAttribution,
    quoted: bool,
    source_turn: Option<u64>,
    authorized: bool,
) -> CorrectionDecision {
    match (attribution, quoted, source_turn, authorized) {
        (
            FeedbackAttribution::ExactReply | FeedbackAttribution::UniqueScoped,
            false,
            Some(turn_id),
            true,
        ) => CorrectionDecision::Repair { turn_id },
        _ => CorrectionDecision::Ignore,
    }
}

/// Process-local identity retained beside the pure numeric decision token.
/// The token is never looked up as authority; every use matches native identity,
/// scope, original asker, timestamp and minimized ask against the current ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CorrectionSource {
    pub(crate) observer: String,
    pub(crate) admitted_at: u64,
    pub(crate) native_id: String,
    pub(crate) scope: String,
    pub(crate) guild: String,
    pub(crate) asker: String,
    pub(crate) created_at: u64,
    pub(crate) signature: AskSignature,
}

pub(crate) fn explicit_correction(text: &str) -> bool {
    let mut tokens = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty());
    let bare_no = tokens.next().is_some_and(|token| {
        ["no", "nope", "nah"]
            .iter()
            .any(|no| token.eq_ignore_ascii_case(no))
    }) && tokens.next().is_none();
    !bare_no
        && !text.lines().any(|line| {
            let line = line.trim_start().to_ascii_lowercase();
            ["example:", "for example", "pasted:", "quote:"]
                .iter()
                .any(|prefix| line.starts_with(prefix))
        })
        && !super::style_signal::contains_quotation(text)
        && super::outcome::classify_signature(text, None)
            == Some(super::outcome::ReplyOutcome::Correction)
}

pub(crate) const INSUFFICIENT_EVIDENCE: &str = "I can’t verify my earlier answer from the evidence I can currently access. Please share a current source or restate the question so I can check it.";
pub(crate) const RECOVERY_INSTRUCTIONS: &str = "The member has challenged an earlier answer. Recheck only the current authorized evidence supplied here. The correction is a request to check, not factual evidence. Earlier assistant output and these instructions are not evidence. Do not infer the original question from a signature or invent an earlier answer. State what the current evidence supports; if it cannot establish a correction, say you cannot verify the earlier answer and ask for a current source or a restated question. Do not store, replace, quarantine, contradict or resolve any fact, and do not request tools.";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn correction_admission_requires_current_authorized_nonquoted_attribution() {
        use FeedbackAttribution::*;
        for source in [
            ExactReply,
            UniqueScoped,
            Duplicate,
            Ambiguous,
            Expired,
            Unsupported,
        ] {
            for quoted in [true, false] {
                for authorized in [true, false] {
                    for turn in [None, Some(7)] {
                        let accepted = matches!(source, ExactReply | UniqueScoped)
                            && !quoted
                            && authorized
                            && turn.is_some();
                        assert_eq!(
                            matches!(
                                evaluate_correction(source, quoted, turn, authorized),
                                CorrectionDecision::Repair { .. }
                            ),
                            accepted
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn quoted_wrong_is_not_correction() {
        for text in [
            "\"that's wrong\"",
            "`wrong`",
            "    wrong",
            "\twrong",
            "’wrong’",
            "no,",
            "> wrong",
            "~~~\nwrong",
            "'wrong'",
            "no",
            "No!",
            "nope",
            "thanks",
            "example: \"incorrect\"",
        ] {
            assert!(!explicit_correction(text), "{text}");
        }
        assert!(explicit_correction("thanks, but that's wrong"));
    }
}

//! Source-attributed contextual proposals. No classification, persistence or sanctions.
use super::Severity;

pub const MAX_CONTEXTUAL_TIMEOUT_MINUTES: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assessment {
    ConfirmedOffending,
    Ambiguous,
    QuotationOrReport,
}

#[derive(Debug, Clone, Copy)]
pub struct Input {
    pub guild: u64,
    pub channel: u64,
    pub message: u64,
    pub source_author: u64,
    pub target: u64,
    pub source_matches_scope: bool,
    pub content_available: bool,
    pub target_is_bot: bool,
    pub target_is_staff: bool,
    pub moderator_can_delete: bool,
    pub moderator_can_timeout: bool,
    pub hierarchy_allows: bool,
    pub assessment: Assessment,
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Proposal {
    pub guild: u64,
    pub channel: u64,
    pub message: u64,
    pub timeout_minutes: Option<u32>,
}

pub fn qualify(input: Input) -> Result<Proposal, &'static str> {
    if input.guild == 0
        || input.channel == 0
        || input.message == 0
        || !input.source_matches_scope
        || input.source_author != input.target
    {
        return Err(
            "Source scope or member attribution does not match. No contextual proposal qualified.",
        );
    }
    if input.target_is_bot || input.target_is_staff {
        return Err(
            "Bot, staff and owner targets are excluded from this contextual pilot. Human review required.",
        );
    }
    if !input.content_available {
        return Err("Message content is unavailable. Context cannot be qualified.");
    }
    if input.assessment != Assessment::ConfirmedOffending {
        return Err(
            "Ambiguous, quoted or reported content requires human review; no contextual proposal qualified.",
        );
    }
    if !input.moderator_can_delete || !input.moderator_can_timeout || !input.hierarchy_allows {
        return Err(
            "Current moderator permissions or target hierarchy do not qualify this proposal.",
        );
    }
    Ok(Proposal {
        guild: input.guild,
        channel: input.channel,
        message: input.message,
        timeout_minutes: match input.severity {
            Severity::Minor => None,
            Severity::Serious | Severity::Severe => Some(MAX_CONTEXTUAL_TIMEOUT_MINUTES),
        },
    })
}

pub fn render(proposal: Proposal) -> String {
    let timeout = proposal.timeout_minutes.map_or_else(
        || "No timeout proposed.".into(),
        |minutes| format!("Proposed timeout: {minutes} minutes maximum."),
    );
    format!(
        "Contextual proposal only — no action taken.\nSource: https://discord.com/channels/{}/{}/{}\nBasis: the invoking moderator explicitly assessed this message as offending in context; Abbey did not independently classify it.\nProposed action: delete this offending message. {} Kicks and bans are outside this pilot. Inspect the linked message and surrounding discussion before a human decision.\nOperational review persistence and live pilot qualification are unavailable; this private reply is not a saved case.",
        proposal.guild, proposal.channel, proposal.message, timeout
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> Input {
        Input {
            guild: 1,
            channel: 2,
            message: 3,
            source_author: 4,
            target: 4,
            source_matches_scope: true,
            content_available: true,
            target_is_bot: false,
            target_is_staff: false,
            moderator_can_delete: true,
            moderator_can_timeout: true,
            hierarchy_allows: true,
            assessment: Assessment::ConfirmedOffending,
            severity: Severity::Severe,
        }
    }
    #[test]
    fn severity_never_exceeds_contextual_ceiling_and_source_is_exact() {
        for severity in [Severity::Minor, Severity::Serious, Severity::Severe] {
            let proposal = qualify(Input {
                severity,
                ..input()
            })
            .unwrap();
            assert!(proposal.timeout_minutes.is_none_or(|minutes| minutes <= 10));
            let text = render(proposal);
            assert!(text.contains("https://discord.com/channels/1/2/3"));
            assert!(text.contains("no action taken"));
            assert!(text.contains("not a saved case"));
        }
    }
    #[test]
    fn ambiguity_bot_staff_scope_and_permission_fail_closed() {
        for changed in [
            Input {
                assessment: Assessment::Ambiguous,
                ..input()
            },
            Input {
                assessment: Assessment::QuotationOrReport,
                ..input()
            },
            Input {
                target_is_bot: true,
                ..input()
            },
            Input {
                target_is_staff: true,
                ..input()
            },
            Input {
                source_author: 5,
                ..input()
            },
            Input {
                source_matches_scope: false,
                ..input()
            },
            Input {
                content_available: false,
                ..input()
            },
            Input {
                moderator_can_delete: false,
                ..input()
            },
            Input {
                moderator_can_timeout: false,
                ..input()
            },
            Input {
                hierarchy_allows: false,
                ..input()
            },
            Input {
                message: 0,
                ..input()
            },
        ] {
            assert!(qualify(changed).is_err());
        }
    }
}

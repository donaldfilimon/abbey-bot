//! Moderation escalation.
//!
//! The skill's rule for a mod decision is short: *action plus a one-line reason,
//! no moralizing.* That shape is the whole design here — this module returns a
//! decision and a justification, and deliberately produces no commentary about
//! the person.
//!
//! Two properties are worth stating because they are easy to erode later:
//!
//! - **Severity outranks history.** A severe incident bans on the first offence;
//!   it does not walk up the ladder. Making history able to soften severity would
//!   mean a clean record buys tolerance for a threat, which is backwards.
//! - **The ladder never skips downward.** More history can only produce an equal
//!   or harsher action, never a lighter one.
//!
//! Nothing here talks to Discord, so the ladder is unit-tested directly.

/// How bad the incident is. Judged by a human; this module does not classify.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Rudeness, mild spam, off-topic derailing.
    Minor,
    /// Harassment, slurs, deliberate disruption.
    Serious,
    /// Threats, doxxing, raid participation, sexual content involving minors.
    Severe,
}

/// Prior moderation actions against this member, as the moderator knows them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct History {
    pub warnings: u8,
    pub timeouts: u8,
}

/// Discord refuses a timeout longer than 28 days. Encoded so a future ladder
/// extension fails a test here rather than an API call in production.
pub const MAX_TIMEOUT_MINUTES: u32 = 28 * 24 * 60;

/// What to do about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Log it, say nothing. First brush with a minor thing.
    Note,
    /// Public or DM warning.
    Warn,
    /// Communication timeout, in minutes.
    Timeout(u32),
    /// Removable, rejoinable.
    Kick,
    /// Removable, not rejoinable.
    Ban,
}

impl Action {
    /// The Discord permission a moderator needs to carry this out. `None` means
    /// the action is social rather than an API call.
    pub const fn required_permission(self) -> Option<&'static str> {
        match self {
            Self::Note | Self::Warn => None,
            Self::Timeout(_) => Some("Moderate Members"),
            Self::Kick => Some("Kick Members"),
            Self::Ban => Some("Ban Members"),
        }
    }

    /// Rank on the ladder. Exists so the monotonicity property can be asserted;
    /// nothing in the running bot needs it, so it is test-only rather than
    /// carried as dead weight.
    #[cfg(test)]
    pub const fn severity_rank(self) -> u8 {
        match self {
            Self::Note => 0,
            Self::Warn => 1,
            Self::Timeout(_) => 2,
            Self::Kick => 3,
            Self::Ban => 4,
        }
    }
}

/// Build a timeout the ladder can actually ask Discord for.
///
/// The clamp is real enforcement, not decoration: every `Action::Timeout` in
/// this module is constructed here, so a future ladder rung that reaches past 28
/// days gets capped instead of producing a request the API rejects at runtime.
const fn timeout(minutes: u32) -> Action {
    Action::Timeout(if minutes > MAX_TIMEOUT_MINUTES {
        MAX_TIMEOUT_MINUTES
    } else {
        minutes
    })
}

/// Render a duration the way a human would say it.
fn humanize(minutes: u32) -> String {
    match minutes {
        m if m % (24 * 60) == 0 && m >= 24 * 60 => {
            let days = m / (24 * 60);
            format!("{days} day{}", if days == 1 { "" } else { "s" })
        }
        m if m % 60 == 0 && m >= 60 => {
            let hours = m / 60;
            format!("{hours} hour{}", if hours == 1 { "" } else { "s" })
        }
        m => format!("{m} minutes"),
    }
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Note => f.write_str("Note"),
            Self::Warn => f.write_str("Warn"),
            Self::Timeout(m) => write!(f, "Timeout {}", humanize(*m)),
            Self::Kick => f.write_str("Kick"),
            Self::Ban => f.write_str("Ban"),
        }
    }
}

/// A decision plus the one line that justifies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recommendation {
    pub action: Action,
    pub reason: String,
}

/// Recommend an action.
pub fn recommend(severity: Severity, history: History) -> Recommendation {
    match severity {
        // Severity outranks history entirely — no ladder walk.
        Severity::Severe => Recommendation {
            action: Action::Ban,
            reason: "severe incident; severity decides this regardless of record".to_string(),
        },

        Severity::Serious => match history.timeouts {
            0 => Recommendation {
                action: timeout(60),
                reason: "first serious incident".to_string(),
            },
            1 => Recommendation {
                action: timeout(24 * 60),
                reason: "second serious incident after a prior timeout".to_string(),
            },
            2 => Recommendation {
                action: timeout(7 * 24 * 60),
                reason: "third serious incident; last step before removal".to_string(),
            },
            n => Recommendation {
                action: Action::Ban,
                reason: format!("{n} prior timeouts have not changed the behaviour"),
            },
        },

        Severity::Minor => match history.warnings {
            0 => Recommendation {
                action: Action::Note,
                reason: "first minor incident; nothing on record yet".to_string(),
            },
            1 => Recommendation {
                action: Action::Warn,
                reason: "second minor incident; put it on record".to_string(),
            },
            2 => Recommendation {
                action: timeout(10),
                reason: "warned twice already".to_string(),
            },
            3 => Recommendation {
                action: timeout(60),
                reason: "pattern of minor incidents after a timeout".to_string(),
            },
            n => Recommendation {
                action: Action::Kick,
                reason: format!("{n} warnings without change; rejoinable if they want to reset"),
            },
        },
    }
}

/// Format for Discord. Action, reason, and — when the action needs a permission
/// the moderator lacks — the fact that they cannot actually do it. That last part
/// is the difference between advice and advice you can act on.
pub fn render(subject: &str, rec: &Recommendation, moderator_can_act: bool) -> String {
    let mut out = format!("**{}** — {}. {}", subject, rec.action, rec.reason);

    if let Some(permission) = rec.action.required_permission()
        && !moderator_can_act
    {
        out.push_str(&format!(
            "\n\n⚠️ You do not have **{permission}**, so you cannot carry this out — hand it to someone who does."
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_outranks_history_completely() {
        let clean = recommend(Severity::Severe, History::default());
        let dirty = recommend(
            Severity::Severe,
            History {
                warnings: 9,
                timeouts: 9,
            },
        );
        assert_eq!(clean.action, Action::Ban);
        // A spotless record must not soften a severe incident.
        assert_eq!(clean.action, dirty.action);
    }

    #[test]
    fn a_first_minor_incident_is_only_noted() {
        assert_eq!(
            recommend(Severity::Minor, History::default()).action,
            Action::Note
        );
    }

    #[test]
    fn serious_incidents_escalate_through_timeouts_then_ban() {
        let at = |timeouts| {
            recommend(
                Severity::Serious,
                History {
                    timeouts,
                    ..Default::default()
                },
            )
            .action
        };
        assert_eq!(at(0), Action::Timeout(60));
        assert_eq!(at(1), Action::Timeout(24 * 60));
        assert_eq!(at(2), Action::Timeout(7 * 24 * 60));
        assert_eq!(at(3), Action::Ban);
    }

    #[test]
    fn the_ladder_never_gets_lighter_as_history_grows() {
        for severity in [Severity::Minor, Severity::Serious, Severity::Severe] {
            let mut previous = 0;
            for n in 0..=8u8 {
                let action = recommend(
                    severity,
                    History {
                        warnings: n,
                        timeouts: n,
                    },
                )
                .action;
                let rank = action.severity_rank();
                assert!(
                    rank >= previous,
                    "{severity:?} softened at n={n}: {action} ranked {rank} after {previous}"
                );
                previous = rank;
            }
        }
    }

    #[test]
    fn no_recommended_timeout_exceeds_what_discord_accepts() {
        for severity in [Severity::Minor, Severity::Serious, Severity::Severe] {
            for n in 0..=255u8 {
                if let Action::Timeout(minutes) = recommend(
                    severity,
                    History {
                        warnings: n,
                        timeouts: n,
                    },
                )
                .action
                {
                    assert!(
                        minutes <= MAX_TIMEOUT_MINUTES,
                        "{severity:?} at n={n} would ask Discord for {minutes} minutes"
                    );
                }
            }
        }
    }

    #[test]
    fn social_actions_need_no_permission_but_api_actions_do() {
        assert_eq!(Action::Note.required_permission(), None);
        assert_eq!(Action::Warn.required_permission(), None);
        assert_eq!(
            Action::Timeout(10).required_permission(),
            Some("Moderate Members")
        );
        assert_eq!(Action::Kick.required_permission(), Some("Kick Members"));
        assert_eq!(Action::Ban.required_permission(), Some("Ban Members"));
    }

    #[test]
    fn durations_read_the_way_people_say_them() {
        assert_eq!(humanize(10), "10 minutes");
        assert_eq!(humanize(60), "1 hour");
        assert_eq!(humanize(24 * 60), "1 day");
        assert_eq!(humanize(7 * 24 * 60), "7 days");
    }

    #[test]
    fn render_warns_when_the_moderator_cannot_actually_act() {
        let rec = recommend(Severity::Serious, History::default());
        let blocked = render("frankie", &rec, false);
        assert!(blocked.contains("Moderate Members"), "{blocked}");

        let allowed = render("frankie", &rec, true);
        assert!(!allowed.contains("⚠️"), "{allowed}");
    }

    #[test]
    fn render_never_appends_a_permission_note_to_a_social_action() {
        // Note needs no permission, so `false` must not produce a warning.
        let rec = recommend(Severity::Minor, History::default());
        assert!(!render("frankie", &rec, false).contains("⚠️"));
    }

    #[test]
    fn output_carries_no_commentary_about_the_person() {
        for severity in [Severity::Minor, Severity::Serious, Severity::Severe] {
            let out = render("frankie", &recommend(severity, History::default()), true);
            for moralizing in ["unacceptable", "toxic", "should be ashamed", "disgusting"] {
                assert!(!out.to_lowercase().contains(moralizing), "{out}");
            }
        }
    }
}

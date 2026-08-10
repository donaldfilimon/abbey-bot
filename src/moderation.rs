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

/// Discord refuses a timeout longer than 28 days.
///
/// Note precisely what this buys: `timeout()` *clamps* to it, so an over-long
/// rung is silently capped rather than caught. The ladder sweep test therefore
/// cannot fail; `timeout_clamps_beyond_discords_ceiling` is what actually
/// exercises the constant.
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

        // Keyed on timeouts, but warnings are still consulted: telling a
        // moderator "first serious incident" when they just typed warnings:7
        // contradicts their own input.
        Severity::Serious => match history.timeouts {
            0 if history.warnings == 0 => Recommendation {
                action: timeout(60),
                reason: "first serious incident".to_string(),
            },
            0 => Recommendation {
                action: timeout(60),
                reason: format!(
                    "first serious incident, though {} prior warning(s) precede it",
                    history.warnings
                ),
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

        // Same rule the other way: prior timeouts mean the record is not empty,
        // whatever the warning count says.
        Severity::Minor => match history.warnings {
            0 if history.timeouts == 0 => Recommendation {
                action: Action::Note,
                reason: "first minor incident; nothing on record yet".to_string(),
            },
            0 => Recommendation {
                action: Action::Warn,
                reason: format!(
                    "minor, but {} prior timeout(s) are on record",
                    history.timeouts
                ),
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

/// Format for Discord. Action, reason, and — when the caller has established
/// the moderator cannot actually do it — the reason why. That last part is the
/// difference between advice and advice you can act on.
///
/// The blocker is a caller-supplied sentence rather than a bool, because "you
/// lack the permission" is only one of the ways Discord will refuse: role
/// hierarchy and owner-targeting are others, and this module cannot know them.
pub fn render(subject: &str, rec: &Recommendation, blocked: Option<&str>) -> String {
    let mut out = format!("**{}** — {}. {}", subject, rec.action, rec.reason);

    if let Some(reason) = blocked {
        out.push_str(&format!("\n\n⚠️ {reason}"));
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
    fn neither_counter_is_ignored() {
        // Regression: each ladder read one counter and implicitly asserted the
        // other was empty, so Minor with 5 prior timeouts still reported
        // "nothing on record yet" -- contradicting the moderator's own input.
        // The monotonicity test could not see it because it varied both
        // counters in lockstep.
        let minor = recommend(
            Severity::Minor,
            History {
                warnings: 0,
                timeouts: 5,
            },
        );
        assert_ne!(minor.action, Action::Note);
        assert!(
            !minor.reason.contains("nothing on record"),
            "{}",
            minor.reason
        );

        let serious = recommend(
            Severity::Serious,
            History {
                warnings: 7,
                timeouts: 0,
            },
        );
        assert!(
            serious.reason.contains("prior warning"),
            "{}",
            serious.reason
        );
    }

    #[test]
    fn timeout_clamps_beyond_discords_ceiling() {
        assert_eq!(timeout(60 * 24 * 60), Action::Timeout(MAX_TIMEOUT_MINUTES));
        assert_eq!(timeout(10), Action::Timeout(10));
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
    fn render_appends_the_blocker_verbatim_and_only_when_present() {
        let rec = recommend(Severity::Serious, History::default());
        let blocked = render(
            "frankie",
            &rec,
            Some("You do not have **Moderate Members**."),
        );
        assert!(
            blocked.contains("⚠️ You do not have **Moderate Members**."),
            "{blocked}"
        );

        let allowed = render("frankie", &rec, None);
        assert!(!allowed.contains("⚠️"), "{allowed}");
    }

    #[test]
    fn output_carries_no_commentary_about_the_person() {
        for severity in [Severity::Minor, Severity::Serious, Severity::Severe] {
            let out = render("frankie", &recommend(severity, History::default()), None);
            for moralizing in ["unacceptable", "toxic", "should be ashamed", "disgusting"] {
                assert!(!out.to_lowercase().contains(moralizing), "{out}");
            }
        }
    }
}

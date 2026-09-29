//! `/admin addenda list|revert|clear`: inspect and revert this server's
//! style addenda ([`crate::brain::addenda`]).
//!
//! Authorization is the `/admin learning` rule, owned by the catalog guard.
//! Each adapter defers, reads or mutates under the `AppState` locks without
//! awaiting, then sends one clamped private reply rendered by the pure
//! functions below.

use super::*;
use crate::brain::addenda::{AddendaStatus, Policy, template};
use crate::brain::style_signal::{StyleKnob, StyleSignal};

/// Discord-facing mirror of [`StyleKnob`].
#[derive(Debug, Clone, Copy, poise::ChoiceParameter)]
pub enum KnobChoice {
    #[name = "length"]
    Length,
    #[name = "formality"]
    Formality,
    #[name = "emoji"]
    Emoji,
    #[name = "code"]
    Code,
}

impl From<KnobChoice> for StyleKnob {
    fn from(choice: KnobChoice) -> Self {
        match choice {
            KnobChoice::Length => Self::Length,
            KnobChoice::Formality => Self::Formality,
            KnobChoice::Emoji => Self::Emoji,
            KnobChoice::Code => Self::Code,
        }
    }
}

const fn knob_name(knob: StyleKnob) -> &'static str {
    match knob {
        StyleKnob::Length => "length",
        StyleKnob::Formality => "formality",
        StyleKnob::Emoji => "emoji",
        StyleKnob::Code => "code",
    }
}

const fn direction(signal: StyleSignal) -> &'static str {
    match signal {
        StyleSignal::TooLong => "shorter",
        StyleSignal::TooShort => "fuller",
        StyleSignal::TooFormal => "more casual",
        StyleSignal::TooCasual => "more formal",
        StyleSignal::NoEmoji => "no emoji",
        StyleSignal::MoreEmoji => "some emoji",
        StyleSignal::PreferCode => "code blocks",
    }
}

/// Seconds left, rounded: under an hour, whole hours below two days, then
/// whole days.
fn time_left(secs: u64) -> String {
    const HOUR: u64 = 3600;
    const DAY: u64 = 24 * HOUR;
    let plural = |n: u64, unit: &str| format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
    if secs < HOUR {
        "under an hour".to_owned()
    } else if secs < 2 * DAY {
        plural((secs + HOUR / 2) / HOUR, "hour")
    } else {
        plural((secs + DAY / 2) / DAY, "day")
    }
}

fn suppression_days() -> String {
    time_left(Policy::default().ttl_secs)
}

fn render_list(status: &AddendaStatus, now: u64) -> String {
    let mut out = String::new();
    if status.active.is_empty() {
        out.push_str(
            "No style addenda are active in this server. Addenda form only in servers with learning enabled (`/admin learning on`), when several members give Abbey the same style feedback.",
        );
    } else {
        out.push_str("**Style addenda for this server**");
        if !status.learning_enabled {
            out.push_str(
                "\nLearning is off here, so these are inactive: Abbey does not receive them.",
            );
        }
        for addendum in &status.active {
            out.push_str(&format!(
                "\n- **{}** · {} · {} left\n  Prompt line: \"{}\"",
                knob_name(addendum.signal.knob()),
                direction(addendum.signal),
                time_left(addendum.expires_at.saturating_sub(now)),
                template(addendum.signal)
            ));
        }
    }
    if !status.suppressions.is_empty() {
        out.push_str("\n\n**Suppressed** (not re-applied until the time runs out)");
        for suppression in &status.suppressions {
            out.push_str(&format!(
                "\n- **{}** · {} left",
                knob_name(suppression.knob),
                time_left(suppression.until.saturating_sub(now))
            ));
        }
    }
    out
}

fn render_revert(knob: StyleKnob, reverted: Option<StyleSignal>) -> String {
    let name = knob_name(knob);
    let days = suppression_days();
    match reverted {
        Some(signal) => format!(
            "Reverted the **{name}** addendum (\"{}\"). Abbey will not re-form the {name} addendum in this server for {days}.",
            template(signal)
        ),
        None => format!(
            "No **{name}** addendum was active. The suppression is recorded anyway: Abbey will not re-form the {name} addendum in this server for {days}."
        ),
    }
}

fn render_clear(cleared: usize) -> String {
    let days = suppression_days();
    match cleared {
        0 => format!(
            "No style addenda were active. The suppression is recorded anyway: Abbey will not apply any style addendum in this server for {days}."
        ),
        1 => format!(
            "Cleared 1 style addendum. Abbey will not apply any style addendum in this server for {days}."
        ),
        n => format!(
            "Cleared {n} style addenda. Abbey will not apply any style addendum in this server for {days}."
        ),
    }
}

/// Review or revert the style lines Abbey adopted from member feedback.
#[poise::command(
    slash_command,
    guild_only,
    ephemeral,
    rename = "addenda",
    subcommands("addenda_list", "addenda_revert", "addenda_clear")
)]
pub async fn admin_addenda(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// List active style addenda and suppressions, with the time left.
#[poise::command(slash_command, guild_only, ephemeral, rename = "list")]
pub async fn addenda_list(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    if ctx.guild_id().is_none() {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    }
    let now = runtime::now();
    let status = ctx.data().state.addenda_status(&scoped_guild(ctx), now);
    send_private_no_mentions(ctx, render_list(&status, now)).await
}

/// Revert one knob's addendum and keep it from re-forming for 14 days.
#[poise::command(slash_command, guild_only, ephemeral, rename = "revert")]
pub async fn addenda_revert(
    ctx: Context<'_>,
    #[description = "Which style knob"] knob: KnobChoice,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    if ctx.guild_id().is_none() {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    }
    let knob = StyleKnob::from(knob);
    let reverted = ctx
        .data()
        .state
        .revert_addendum(&scoped_guild(ctx), knob, runtime::now());
    send_private_no_mentions(ctx, render_revert(knob, reverted)).await
}

/// Revert every addendum and keep all of them from re-forming for 14 days.
#[poise::command(slash_command, guild_only, ephemeral, rename = "clear")]
pub async fn addenda_clear(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    if ctx.guild_id().is_none() {
        ctx.say(NO_GUILD).await?;
        return Ok(());
    }
    let cleared = ctx
        .data()
        .state
        .clear_addenda(&scoped_guild(ctx), runtime::now());
    send_private_no_mentions(ctx, render_clear(cleared)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::addenda::{Addendum, Suppression};

    const DAY: u64 = 24 * 3600;

    #[test]
    fn time_left_rounds_to_hours_or_days() {
        assert_eq!(time_left(0), "under an hour");
        assert_eq!(time_left(3599), "under an hour");
        assert_eq!(time_left(3600), "1 hour");
        assert_eq!(time_left(5 * 3600 + 1800), "6 hours");
        assert_eq!(time_left(2 * DAY - 1), "48 hours");
        assert_eq!(time_left(2 * DAY), "2 days");
        assert_eq!(time_left(14 * DAY), "14 days");
        assert_eq!(time_left(13 * DAY + 13 * 3600), "14 days");
    }

    fn listed() -> AddendaStatus {
        AddendaStatus {
            learning_enabled: true,
            active: vec![
                Addendum {
                    signal: StyleSignal::TooLong,
                    expires_at: 13 * DAY + 5,
                },
                Addendum {
                    signal: StyleSignal::NoEmoji,
                    expires_at: 20 * 3600 + 5,
                },
            ],
            suppressions: vec![Suppression {
                knob: StyleKnob::Code,
                until: 14 * DAY + 5,
            }],
        }
    }

    #[test]
    fn list_shows_knob_direction_template_and_time_left() {
        let text = render_list(&listed(), 5);
        assert!(
            text.contains("**length** · shorter · 13 days left"),
            "{text}"
        );
        assert!(text.contains(template(StyleSignal::TooLong)), "{text}");
        assert!(
            text.contains("**emoji** · no emoji · 20 hours left"),
            "{text}"
        );
        assert!(text.contains("**code** · 14 days left"), "{text}");
        assert!(!text.contains("inactive"), "{text}");
    }

    #[test]
    fn list_marks_addenda_inactive_while_learning_is_off() {
        let status = AddendaStatus {
            learning_enabled: false,
            ..listed()
        };
        assert!(render_list(&status, 5).contains("these are inactive"));
    }

    #[test]
    fn empty_list_says_so_and_names_the_learning_requirement() {
        let text = render_list(&AddendaStatus::default(), 0);
        assert!(text.starts_with("No style addenda are active"), "{text}");
        assert!(text.contains("learning enabled"), "{text}");
        assert!(!text.contains("Suppressed"), "{text}");
    }

    #[test]
    fn revert_and_clear_replies_say_what_happened_and_the_suppression() {
        let done = render_revert(StyleKnob::Length, Some(StyleSignal::TooLong));
        assert!(
            done.starts_with("Reverted the **length** addendum"),
            "{done}"
        );
        assert!(done.contains("for 14 days"), "{done}");
        let none = render_revert(StyleKnob::Emoji, None);
        assert!(
            none.starts_with("No **emoji** addendum was active"),
            "{none}"
        );
        assert!(none.contains("recorded anyway"), "{none}");
        assert!(render_clear(2).starts_with("Cleared 2 style addenda."));
        assert!(render_clear(1).starts_with("Cleared 1 style addendum."));
        assert!(render_clear(0).contains("recorded anyway"));
    }

    #[test]
    fn every_reply_fits_one_discord_message() {
        let full = AddendaStatus {
            learning_enabled: false,
            active: [
                StyleSignal::TooShort,
                StyleSignal::TooCasual,
                StyleSignal::MoreEmoji,
                StyleSignal::PreferCode,
            ]
            .into_iter()
            .map(|signal| Addendum {
                signal,
                expires_at: 14 * DAY,
            })
            .collect(),
            suppressions: Vec::new(),
        };
        assert!(render_list(&full, 0).len() < 2000);
    }

    #[test]
    fn adapters_are_private_guild_only_and_catalog_bound() {
        let group = admin_addenda();
        assert!(group.guild_only && group.ephemeral);
        let names: Vec<_> = group.subcommands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["list", "revert", "clear"]);
        for leaf in &group.subcommands {
            assert!(leaf.guild_only && leaf.ephemeral, "{}", leaf.name);
        }
        for key in [
            crate::command_catalog::CommandKey::AdminAddendaList,
            crate::command_catalog::CommandKey::AdminAddendaRevert,
            crate::command_catalog::CommandKey::AdminAddendaClear,
        ] {
            let spec = crate::command_catalog::command(key);
            let learning =
                crate::command_catalog::command(crate::command_catalog::CommandKey::AdminLearning);
            assert_eq!(spec.eligibility, learning.eligibility, "{}", spec.name);
            assert_eq!(spec.registration, learning.registration, "{}", spec.name);
            assert!(spec.private, "{}", spec.name);
        }
    }

    #[test]
    fn print_rendered_replies() {
        for text in [
            render_list(&listed(), 5),
            render_list(
                &AddendaStatus {
                    learning_enabled: false,
                    ..listed()
                },
                5,
            ),
            render_list(&AddendaStatus::default(), 0),
            render_revert(StyleKnob::Length, Some(StyleSignal::TooLong)),
            render_revert(StyleKnob::Emoji, None),
            render_clear(2),
            render_clear(0),
        ] {
            println!("---\n{text}");
        }
    }
}

//! Wiring for per-guild style addenda ([`crate::brain::addenda`]).
//!
//! Only Discord guild scopes take part, and only while the guild has
//! `learning_enabled`: the pipeline observes a member's own message, the
//! learning tick applies and expires addenda, and prompt assembly reads the
//! rendered templates. With learning off (the default) nothing is observed,
//! nothing ticks, and [`AppState::style_addenda`] is empty, so every prompt is
//! byte-identical to a build without this module. Rendering filters by the
//! caller's `now`, so an expired addendum never reaches a prompt even before
//! the next tick. `/admin addenda` reads, reverts and clears through the
//! methods below, whatever the learning setting. Locks follow the `AppState`
//! order (`stores` then `guilds`) and are never held across an await.

use super::*;
use crate::brain::addenda::{AddendaStatus, AddendumChange, Policy};
use crate::brain::style_signal;
use crate::brain::style_signal::StyleKnob;

/// Whether `scoped_guild` is a Discord guild: `discord:<id>`, not a
/// `discord:dm:<user>` scope and not another network.
fn is_discord_guild(scoped_guild: &str) -> bool {
    scoped_guild
        .strip_prefix("discord:")
        .is_some_and(|id| !id.is_empty() && !id.starts_with("dm:"))
}

/// The member key a ledger hashes: guild-qualified, so one member's hashes
/// are unlinkable across guilds yet recomputable from the ids.
fn member_key(scoped_guild: &str, scoped_user: &str) -> String {
    format!("{scoped_guild}\u{1f}{scoped_user}")
}

impl AppState {
    /// Guild-local reduction never changes a member policy or another kind.
    pub(crate) fn follow_up_reduced(
        &self,
        scope: &crate::engagement::EngagementScope,
        now: u64,
    ) -> bool {
        let crate::engagement::EngagementScope::Guild { guild, .. } = scope else {
            return false;
        };
        let status = self.addenda_status(&format!("discord:{guild}"), now);
        status.learning_enabled
            && status
                .active
                .iter()
                .any(|a| a.signal == style_signal::StyleSignal::FewerFollowUps)
    }

    /// Record the style signal in a member's own message, if it carries one.
    /// The caller has already established that the guild has learning on.
    pub fn observe_style(&self, scoped_guild: &str, scoped_user: &str, text: &str, now: u64) {
        if !is_discord_guild(scoped_guild) {
            return;
        }
        let Some(signal) = style_signal::classify(text) else {
            return;
        };
        let mut stores = Self::lock(&self.stores);
        if stores
            .reward_recovery
            .erasure
            .blocks(scoped_guild, scoped_user, now)
        {
            return;
        }
        stores
            .addenda
            .entry(scoped_guild.to_owned())
            .or_default()
            .observe(&member_key(scoped_guild, scoped_user), signal, now);
    }

    /// Apply and expire addenda for every guild whose learning is on; drop
    /// ledgers left with nothing in them.
    pub(super) fn tick_addenda(&self, now: u64) {
        let policy = Policy::default();
        let mut stores = Self::lock(&self.stores);
        let enabled: Vec<String> = {
            let mut guilds = Self::lock(&self.guilds);
            let keys: Vec<String> = stores.addenda.keys().cloned().collect();
            keys.into_iter()
                .filter(|guild| guilds.config(guild, &mut *stores).learning_enabled)
                .collect()
        };
        for guild in enabled {
            let Some(ledger) = stores.addenda.get_mut(&guild) else {
                continue;
            };
            for change in ledger.tick(&policy, now) {
                match change {
                    AddendumChange::Applied(signal) => {
                        tracing::info!(guild = %guild, signal = ?signal, "style addendum applied");
                    }
                    AddendumChange::Expired(signal) => {
                        tracing::info!(guild = %guild, signal = ?signal, "style addendum expired");
                    }
                }
            }
        }
        stores.addenda.retain(|_, ledger| !ledger.is_empty());
    }

    /// The rendered addenda for `scoped_guild`'s prompts: fixed templates
    /// only, empty unless it is a Discord guild with learning on.
    pub fn style_addenda(&self, scoped_guild: &str, now: u64) -> String {
        if !is_discord_guild(scoped_guild) {
            return String::new();
        }
        let mut stores = Self::lock(&self.stores);
        if !Self::lock(&self.guilds)
            .config(scoped_guild, &mut *stores)
            .learning_enabled
        {
            return String::new();
        }
        stores
            .addenda
            .get(scoped_guild)
            .map(|ledger| ledger.render(now))
            .unwrap_or_default()
    }

    /// The live addenda and suppressions for `/admin addenda list`, shown
    /// even while learning is off (they then do not render).
    pub fn addenda_status(&self, scoped_guild: &str, now: u64) -> AddendaStatus {
        let mut stores = Self::lock(&self.stores);
        let learning_enabled = Self::lock(&self.guilds)
            .config(scoped_guild, &mut *stores)
            .learning_enabled;
        let ledger = stores.addenda.get(scoped_guild);
        let policy = Policy::default();
        AddendaStatus {
            learning_enabled,
            active: ledger.map(|l| l.active(now)).unwrap_or_default(),
            suppressions: ledger.map(|l| l.suppressions(now)).unwrap_or_default(),
            pending: ledger
                .map(|l| l.pending_evidence(&policy, now))
                .unwrap_or_default(),
            policy,
        }
    }

    /// Revert `knob` and suppress it for the TTL. Returns the signal that was
    /// active, if any; the suppression is recorded either way.
    pub fn revert_addendum(
        &self,
        scoped_guild: &str,
        knob: StyleKnob,
        now: u64,
    ) -> Option<style_signal::StyleSignal> {
        let mut stores = Self::lock(&self.stores);
        let ledger = stores.addenda.entry(scoped_guild.to_owned()).or_default();
        let was = ledger
            .active(now)
            .into_iter()
            .find(|a| a.signal.knob() == knob)
            .map(|a| a.signal);
        let reverted = ledger.revert(knob, &Policy::default(), now);
        tracing::info!(guild = %scoped_guild, knob = ?knob, reverted, "style addendum reverted by operator");
        was
    }

    /// Revert every knob; returns how many addenda were active.
    pub fn clear_addenda(&self, scoped_guild: &str, now: u64) -> usize {
        let mut stores = Self::lock(&self.stores);
        let cleared = stores
            .addenda
            .entry(scoped_guild.to_owned())
            .or_default()
            .clear(&Policy::default(), now);
        tracing::info!(guild = %scoped_guild, cleared, "style addenda cleared by operator");
        cleared
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_discord_guild_scopes_take_part() {
        assert!(is_discord_guild("discord:123"));
        assert!(is_discord_guild("discord:g"));
        assert!(!is_discord_guild("discord:dm:42"));
        assert!(!is_discord_guild("discord:"));
        assert!(!is_discord_guild("telegram:123"));
        assert!(!is_discord_guild("slack:T1"));
    }

    fn set_learning(state: &AppState, guild: &str, on: bool) {
        let mut stores = AppState::lock(&state.stores);
        AppState::lock(&state.guilds).update(guild, &mut *stores, |s| s.learning_enabled = on);
    }

    #[test]
    fn learn_all_applies_addenda_and_learning_off_silences_them() {
        let state = AppState::in_memory();
        let guild = "discord:77";
        set_learning(&state, guild, true);
        let now = now();
        for user in [
            "discord:1",
            "discord:1",
            "discord:2",
            "discord:2",
            "discord:3",
        ] {
            state.observe_style(guild, user, "too long", now);
        }
        assert_eq!(
            state.style_addenda(guild, now),
            "",
            "not applied before a tick"
        );
        state.learn_all();
        let rendered = state.style_addenda(guild, now);
        assert!(rendered.contains("Keep replies brief"), "{rendered}");
        let stored = serde_json::to_string(&AppState::lock(&state.stores).addenda).unwrap();
        assert!(
            !stored.contains("discord:1") && !stored.contains("discord:2"),
            "{stored}"
        );

        set_learning(&state, guild, false);
        assert_eq!(state.style_addenda(guild, now), "");
        // Kept in storage so `/admin addenda list` can show them as inactive.
        let status = state.addenda_status(guild, now);
        assert!(!status.learning_enabled);
        assert_eq!(status.active.len(), 1);
    }

    /// Five "too long" signals from three members, then a learn tick.
    fn applied(state: &AppState, guild: &str, now: u64) {
        set_learning(state, guild, true);
        for user in [
            "discord:1",
            "discord:1",
            "discord:2",
            "discord:2",
            "discord:3",
        ] {
            state.observe_style(guild, user, "too long", now);
        }
        state.tick_addenda(now);
        assert!(!state.style_addenda(guild, now).is_empty());
    }

    #[test]
    fn an_expired_addendum_never_renders_even_before_a_tick() {
        let state = AppState::in_memory();
        let guild = "discord:78";
        applied(&state, guild, 100);
        let expiry = 100 + Policy::default().ttl_secs;
        assert!(!state.style_addenda(guild, expiry - 1).is_empty());
        assert_eq!(state.style_addenda(guild, expiry), "");
        assert!(state.addenda_status(guild, expiry).active.is_empty());
    }

    #[test]
    fn revert_and_clear_silence_render_and_record_suppression() {
        let state = AppState::in_memory();
        let guild = "discord:79";
        applied(&state, guild, 100);
        assert_eq!(
            state.revert_addendum(guild, StyleKnob::Length, 101),
            Some(style_signal::StyleSignal::TooLong)
        );
        assert_eq!(state.style_addenda(guild, 101), "");
        assert_eq!(state.revert_addendum(guild, StyleKnob::Length, 102), None);
        // The suppression survives the learn tick's empty-ledger sweep.
        state.tick_addenda(103);
        assert_eq!(state.addenda_status(guild, 103).suppressions.len(), 1);

        // A guild that never observed anything can still be cleared.
        let fresh = "discord:80";
        assert_eq!(state.clear_addenda(fresh, 5), 0);
        let status = state.addenda_status(fresh, 5);
        assert!(status.active.is_empty());
        assert_eq!(
            status
                .suppressions
                .iter()
                .map(|s| s.knob)
                .collect::<Vec<_>>(),
            vec![
                StyleKnob::Length,
                StyleKnob::Formality,
                StyleKnob::Emoji,
                StyleKnob::Code,
                StyleKnob::FollowUp
            ]
        );
        set_learning(&state, fresh, true);
        for feedback in [
            "too short",
            "too formal",
            "more emoji",
            "prefer code",
            "fewer follow-ups",
        ] {
            for member in [
                "discord:1",
                "discord:1",
                "discord:2",
                "discord:2",
                "discord:3",
            ] {
                state.observe_style(fresh, member, feedback, 6);
            }
        }
        state.tick_addenda(7);
        assert!(state.addenda_status(fresh, 7).active.is_empty());
        assert_eq!(state.style_addenda(fresh, 7), "");
    }

    #[test]
    fn dm_and_other_network_scopes_never_observe() {
        let state = AppState::in_memory();
        for scope in ["discord:dm:5", "telegram:9"] {
            state.observe_style(scope, "discord:5", "too long", 1);
            assert_eq!(state.style_addenda(scope, 1), "");
        }
        assert!(AppState::lock(&state.stores).addenda.is_empty());
    }

    #[test]
    fn pending_status_is_guild_isolated_and_marks_retained_feedback_learning_off() {
        let state = AppState::in_memory();
        let guild = "discord:81";
        set_learning(&state, guild, true);
        state.observe_style(guild, "discord:private-member", "too long", 10);
        let status = state.addenda_status(guild, 10);
        assert_eq!(status.pending[0].supporting, 1);
        assert_eq!(status.policy, Policy::default());
        assert!(!format!("{status:?}").contains("private-member"));
        assert!(state.addenda_status("discord:82", 10).pending.is_empty());
        set_learning(&state, guild, false);
        let off = state.addenda_status(guild, 11);
        assert!(!off.learning_enabled);
        assert_eq!(off.pending[0].supporting, 1);
    }
}

#[cfg(test)]
mod reduction_tests {
    use super::*;
    #[test]
    fn reduction_stays_guild_local_and_never_changes_member_policy() {
        let state = AppState::in_memory();
        {
            let mut stores = AppState::lock(&state.stores);
            AppState::lock(&state.guilds)
                .update("discord:1", &mut *stores, |s| s.learning_enabled = true);
        }
        for user in [
            "discord:7",
            "discord:7",
            "discord:8",
            "discord:8",
            "discord:9",
        ] {
            state.observe_style("discord:1", user, "fewer follow-ups", 10);
        }
        state.tick_addenda(10);
        let scope = crate::engagement::EngagementScope::Guild {
            guild: 1,
            channel: 2,
        };
        assert!(state.follow_up_reduced(&scope, 10));
        assert!(!state.follow_up_reduced(
            &crate::engagement::EngagementScope::Guild {
                guild: 3,
                channel: 2
            },
            10
        ));
        assert!(!state.follow_up_reduced(
            &crate::engagement::EngagementScope::Dm {
                member: 7,
                channel: 2
            },
            10
        ));
        assert!(
            AppState::lock(&state.stores)
                .work
                .engagement
                .member_policies
                .is_empty()
        );
        assert!(!state.follow_up_reduced(&scope, 10 + Policy::default().ttl_secs));
        state.revert_addendum("discord:1", StyleKnob::FollowUp, 11);
        assert!(!state.follow_up_reduced(&scope, 11));
    }
}

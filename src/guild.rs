//! Per-guild configuration, cooldown, and id namespacing
//! (`docs/spec/multiguild.md`, `docs/spec/platforms.md`).
//!
//! The isolation invariant: everything learned, remembered, or configured is
//! keyed by a *scoped* id — `"{platform}:{native}"` — so two platforms' ids
//! can never collide. The helpers at the bottom are the only place that
//! format is spelled out.
//!
//! Pure: no serenity, no poise, no clock. Time is injected as unix seconds.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::persona::Persona;

/// `/admin cooldown` ceiling (seconds).
pub const MAX_COOLDOWN_SECONDS: u32 = 600;
/// Spec default for the unsolicited-reply gap.
pub const DEFAULT_COOLDOWN_SECONDS: u32 = 20;
/// Unsolicited actions per guild per hour when nothing else is configured.
pub const DEFAULT_BUDGET_PER_HOUR: u32 = 6;
/// Ceiling for `/admin budget`.
pub const MAX_BUDGET_PER_HOUR: u32 = 60;

fn default_true() -> bool {
    true
}

fn default_budget_per_hour() -> u32 {
    DEFAULT_BUDGET_PER_HOUR
}

/// Lowercase wire name of a persona (`"abbey" | "aviva" | "abi"`), the form
/// the `guild_configs.default_persona` column and `/admin persona` use.
pub const fn persona_name(persona: Persona) -> &'static str {
    match persona {
        Persona::Abbey => "abbey",
        Persona::Aviva => "aviva",
        Persona::Abi => "abi",
    }
}

/// Parse a persona name case-insensitively; `None` for anything unknown.
pub fn parse_persona(name: &str) -> Option<Persona> {
    match name.trim().to_ascii_lowercase().as_str() {
        "abbey" => Some(Persona::Abbey),
        "aviva" => Some(Persona::Aviva),
        "abi" => Some(Persona::Abi),
        _ => None,
    }
}

/// Serde bridge so `Persona` (which does not derive serde) round-trips as its
/// lowercase wire name.
mod persona_serde {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    use super::{Persona, parse_persona, persona_name};

    pub fn serialize<S: Serializer>(persona: &Persona, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(persona_name(*persona))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Persona, D::Error> {
        let name = String::deserialize(d)?;
        parse_persona(&name).ok_or_else(|| D::Error::custom(format!("unknown persona `{name}`")))
    }
}

/// Snapshot value handed to hot paths.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuildSettings {
    pub enabled: bool,
    #[serde(with = "persona_serde")]
    pub default_persona: Persona,
    /// DQN on/off for this guild. **Opt-in and default-off** per constitutional
    /// decision 31: a guild that has never chosen must not be learning from its
    /// members. `#[serde(default)]` makes an absent field read as off rather
    /// than as a deserialize error, so the safe value is also the one a
    /// truncated or older document produces.
    #[serde(default)]
    pub learning_enabled: bool,
    /// Kept for the spec's row shape and old documents; voice is out of
    /// scope (no `voice.md` was supplied), so nothing reads or renders it.
    #[serde(default = "default_true")]
    pub voice_enabled: bool,
    pub vision_enabled: bool,
    /// Minimum gap between unsolicited replies in one channel.
    pub reply_cooldown_seconds: u32,
    /// `None` = the global exploration schedule.
    pub epsilon_override: Option<f32>,
    /// May Abbey speak unsolicited here (reply/react chosen by the policy)?
    /// Opt-in per guild via `/admin act on`; `ABBEY_QUIET=1` overrides.
    #[serde(default)]
    pub unsolicited: bool,
    /// Hourly budget of unsolicited actions for the whole guild.
    #[serde(default = "default_budget_per_hour")]
    pub unsolicited_per_hour: u32,
    /// Reply language hint.
    pub locale: String,
}

impl Default for GuildSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            default_persona: Persona::Abbey,
            learning_enabled: false,
            voice_enabled: true,
            vision_enabled: true,
            reply_cooldown_seconds: DEFAULT_COOLDOWN_SECONDS,
            epsilon_override: None,
            unsolicited: false,
            unsolicited_per_hour: DEFAULT_BUDGET_PER_HOUR,
            locale: "en".to_owned(),
        }
    }
}

/// The `guild_configs` row store, keyed by scoped guild id.
pub trait GuildConfigStore {
    fn load(&self, scoped_guild_id: &str) -> Option<GuildSettings>;
    fn save(&mut self, scoped_guild_id: &str, settings: &GuildSettings);
}

/// HashMap-backed store: the test double, and only that.
///
/// It is `#[cfg(test)]`, so it cannot be a production backend at all. The
/// durable one is `impl GuildConfigStore for Stores` in `src/persist.rs`, a
/// JSON file store with an atomic temp-write plus rename. This comment
/// previously called this type "the default backend until a durable one is
/// wired in", which stopped being true once `Stores` landed.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct InMemoryGuildConfigStore {
    pub rows: HashMap<String, GuildSettings>,
}

#[cfg(test)]
impl InMemoryGuildConfigStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
impl GuildConfigStore for InMemoryGuildConfigStore {
    fn load(&self, scoped_guild_id: &str) -> Option<GuildSettings> {
        self.rows.get(scoped_guild_id).cloned()
    }

    fn save(&mut self, scoped_guild_id: &str, settings: &GuildSettings) {
        self.rows
            .insert(scoped_guild_id.to_owned(), settings.clone());
    }
}

/// Write-through config cache with lazy hydrate. Read on every inbound
/// event, so it must not hit the store per message; auto-provisions defaults
/// the first time a guild is seen.
#[derive(Debug, Default)]
pub struct GuildRegistry {
    cache: HashMap<String, GuildSettings>,
}

impl GuildRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Settings for a guild: cache → store → freshly provisioned defaults
    /// (which are saved, so the guild exists in the store from first contact).
    pub fn config(
        &mut self,
        scoped_guild_id: &str,
        store: &mut dyn GuildConfigStore,
    ) -> GuildSettings {
        if let Some(cached) = self.cache.get(scoped_guild_id) {
            return cached.clone();
        }
        let settings = match store.load(scoped_guild_id) {
            Some(found) => found,
            None => {
                let defaults = GuildSettings::default();
                store.save(scoped_guild_id, &defaults);
                defaults
            }
        };
        self.cache
            .insert(scoped_guild_id.to_owned(), settings.clone());
        settings
    }

    /// Mutate a guild's settings, updating the cache and writing through.
    /// Returns the settings after mutation.
    pub fn update(
        &mut self,
        scoped_guild_id: &str,
        store: &mut dyn GuildConfigStore,
        mutate: impl FnOnce(&mut GuildSettings),
    ) -> GuildSettings {
        let mut settings = self.config(scoped_guild_id, store);
        mutate(&mut settings);
        store.save(scoped_guild_id, &settings);
        self.cache
            .insert(scoped_guild_id.to_owned(), settings.clone());
        settings
    }

    /// Drop the cached entry; the next `config` re-hydrates from the store.
    pub fn evict(&mut self, scoped_guild_id: &str) {
        self.cache.remove(scoped_guild_id);
    }

    /// Whether the guild is currently cached.
    pub fn is_cached(&self, scoped_guild_id: &str) -> bool {
        self.cache.contains_key(scoped_guild_id)
    }
}

/// Guards against Abbey dominating an active channel. Checked before the brain
/// is consulted for unsolicited replies; mentions and slash commands bypass it.
#[derive(Debug, Default)]
pub struct ReplyCooldown {
    /// scoped channel id → unix seconds of the last unsolicited reply.
    last_reply_at: HashMap<String, u64>,
}

impl ReplyCooldown {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` when no reply has been sent in this channel yet, or at least
    /// `cooldown_seconds` have elapsed since the last one.
    pub fn permitted(&self, scoped_channel_id: &str, cooldown_seconds: u32, now: u64) -> bool {
        match self.last_reply_at.get(scoped_channel_id) {
            None => true,
            Some(last) => now.saturating_sub(*last) >= u64::from(cooldown_seconds),
        }
    }

    pub fn record_reply(&mut self, scoped_channel_id: &str, now: u64) {
        self.last_reply_at.insert(scoped_channel_id.to_owned(), now);
    }

    /// Check and record in one step, so two messages handled concurrently in
    /// the same channel cannot both observe "permitted" and both speak. The
    /// reservation stands even if the action then fails to send — a cooldown
    /// that errs toward quiet is the safe direction.
    pub fn try_reserve(
        &mut self,
        scoped_channel_id: &str,
        cooldown_seconds: u32,
        now: u64,
    ) -> bool {
        if !self.permitted(scoped_channel_id, cooldown_seconds, now) {
            return false;
        }
        self.record_reply(scoped_channel_id, now);
        true
    }
}

/// Clamp a user-supplied cooldown into `0..=MAX_COOLDOWN_SECONDS`.
pub fn clamp_cooldown(seconds: i64) -> u32 {
    seconds.clamp(0, i64::from(MAX_COOLDOWN_SECONDS)) as u32
}

/// `/admin budget` input → `1..=MAX_BUDGET_PER_HOUR`. Zero is not a valid
/// budget: "never" is `/admin act off`, which also stops learning from the
/// guild, and the distinction matters.
pub fn clamp_budget(per_hour: i64) -> u32 {
    per_hour.clamp(1, i64::from(MAX_BUDGET_PER_HOUR)) as u32
}

fn on_off(flag: bool) -> &'static str {
    if flag { "on" } else { "off" }
}

/// The `/admin show` text.
pub fn render_settings(scoped_guild_id: &str, settings: &GuildSettings) -> String {
    format!(
        "**Abbey — {scoped_guild_id}**\npersona: {} · learning: {} · vision: {} · cooldown: {}s · act: {} · budget: {}/h",
        persona_name(settings.default_persona),
        on_off(settings.learning_enabled),
        on_off(settings.vision_enabled),
        settings.reply_cooldown_seconds,
        on_off(settings.unsolicited),
        settings.unsolicited_per_hour,
    )
}

/// `"{platform}:{native}"`, with `"dm"` standing in for a guild-less context.
pub fn scoped_guild_id(platform: &str, native_guild_id: Option<&str>) -> String {
    format!("{platform}:{}", native_guild_id.unwrap_or("dm"))
}

/// `"{platform}:{native_channel_id}"`.
pub fn scoped_channel_id(platform: &str, native_channel_id: &str) -> String {
    format!("{platform}:{native_channel_id}")
}

/// `"{platform}:{native_user_id}"`.
pub fn scoped_user_id(platform: &str, native_user_id: &str) -> String {
    format!("{platform}:{native_user_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: &str = "discord:42";

    /// Constitutional decision 31: adaptive learning is opt-in and default-off.
    /// A guild that has never chosen must not be learning from its members, and
    /// `commands_brain.rs` already provides the operator toggle that makes
    /// opting in reachable.
    #[test]
    fn learning_is_off_until_a_guild_opts_in() {
        assert!(
            !GuildSettings::default().learning_enabled,
            "adaptive learning must be opt-in; a default of true learns from a guild that never consented"
        );
    }

    #[test]
    fn defaults_match_spec() {
        let d = GuildSettings::default();
        assert!(d.enabled);
        assert_eq!(d.default_persona, Persona::Abbey);
        assert!(
            !d.learning_enabled,
            "learning is opt-in; see learning_is_off_until_a_guild_opts_in"
        );
        assert!(d.voice_enabled);
        assert!(d.vision_enabled);
        assert_eq!(d.reply_cooldown_seconds, 20);
        assert_eq!(d.epsilon_override, None);
        assert_eq!(d.locale, "en");
    }

    #[test]
    fn settings_round_trip_with_lowercase_persona() {
        let s = GuildSettings {
            default_persona: Persona::Aviva,
            epsilon_override: Some(0.25),
            ..GuildSettings::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"default_persona\":\"aviva\""), "{json}");
        let back: GuildSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn unknown_persona_name_fails_to_deserialize() {
        let json = r#"{"enabled":true,"default_persona":"zed","learning_enabled":true,
            "voice_enabled":true,"vision_enabled":true,"reply_cooldown_seconds":20,
            "epsilon_override":null,"locale":"en"}"#;
        assert!(serde_json::from_str::<GuildSettings>(json).is_err());
    }

    #[test]
    fn persona_names_parse_both_ways() {
        for p in [Persona::Abbey, Persona::Aviva, Persona::Abi] {
            assert_eq!(parse_persona(persona_name(p)), Some(p));
        }
        assert_eq!(parse_persona(" AVIVA "), Some(Persona::Aviva));
        assert_eq!(parse_persona("nope"), None);
    }

    #[test]
    fn first_contact_provisions_and_saves_defaults() {
        let mut store = InMemoryGuildConfigStore::new();
        let mut reg = GuildRegistry::new();
        assert!(store.load(G).is_none());
        let s = reg.config(G, &mut store);
        assert_eq!(s, GuildSettings::default());
        assert_eq!(store.load(G), Some(GuildSettings::default()));
        assert!(reg.is_cached(G));
    }

    #[test]
    fn config_hydrates_from_store_and_then_caches() {
        let mut store = InMemoryGuildConfigStore::new();
        let stored = GuildSettings {
            voice_enabled: false,
            ..GuildSettings::default()
        };
        store.save(G, &stored);

        let mut reg = GuildRegistry::new();
        assert!(!reg.config(G, &mut store).voice_enabled);
        // Change the store behind the cache: cached value wins until evicted.
        store.save(G, &GuildSettings::default());
        assert!(!reg.config(G, &mut store).voice_enabled);
        reg.evict(G);
        assert!(!reg.is_cached(G));
        assert!(reg.config(G, &mut store).voice_enabled);
    }

    #[test]
    fn update_writes_through() {
        let mut store = InMemoryGuildConfigStore::new();
        let mut reg = GuildRegistry::new();
        let s = reg.update(G, &mut store, |s| {
            s.default_persona = Persona::Abi;
            s.reply_cooldown_seconds = 45;
        });
        assert_eq!(s.default_persona, Persona::Abi);
        let row = store.load(G).unwrap();
        assert_eq!(row.default_persona, Persona::Abi);
        assert_eq!(row.reply_cooldown_seconds, 45);
        assert_eq!(reg.config(G, &mut store), row);
    }

    #[test]
    fn cooldown_first_reply_always_permitted() {
        let cd = ReplyCooldown::new();
        assert!(cd.permitted("discord:c1", 20, 0));
        assert!(cd.permitted("discord:c1", 600, 0));
    }

    #[test]
    fn cooldown_boundary_is_inclusive() {
        let mut cd = ReplyCooldown::new();
        cd.record_reply("discord:c1", 1_000);
        assert!(!cd.permitted("discord:c1", 20, 1_000));
        assert!(!cd.permitted("discord:c1", 20, 1_019));
        assert!(cd.permitted("discord:c1", 20, 1_020));
        assert!(cd.permitted("discord:c1", 20, 5_000));
        // Zero cooldown never blocks; other channels are unaffected.
        assert!(cd.permitted("discord:c1", 0, 1_000));
        assert!(cd.permitted("discord:c2", 20, 1_000));
        // Atomic reservation: the first taker wins, the second in the same
        // second is refused, and the window is measured from the reservation.
        let mut cd = ReplyCooldown::new();
        assert!(cd.try_reserve("discord:c3", 20, 2_000));
        assert!(!cd.try_reserve("discord:c3", 20, 2_000));
        assert!(!cd.try_reserve("discord:c3", 20, 2_019));
        assert!(cd.try_reserve("discord:c3", 20, 2_020));
    }

    #[test]
    fn cooldown_clamps_into_range() {
        assert_eq!(clamp_cooldown(-5), 0);
        assert_eq!(clamp_cooldown(0), 0);
        assert_eq!(clamp_cooldown(20), 20);
        assert_eq!(clamp_cooldown(600), 600);
        assert_eq!(clamp_cooldown(601), 600);
        assert_eq!(clamp_cooldown(i64::MAX), 600);
    }

    #[test]
    fn render_settings_matches_admin_show() {
        // Both flags are set explicitly so this asserts the *rendering* of a
        // known mixed state rather than whatever the defaults happen to be.
        // `learning_enabled` in particular is now opt-in and default-off.
        let s = GuildSettings {
            learning_enabled: true,
            vision_enabled: false,
            ..GuildSettings::default()
        };
        assert_eq!(
            render_settings("discord:42", &s),
            "**Abbey — discord:42**\npersona: abbey · learning: on · vision: off · cooldown: 20s · act: off · budget: 6/h"
        );
    }

    #[test]
    fn new_settings_default_to_not_acting_with_six_per_hour() {
        let s = GuildSettings::default();
        assert!(!s.unsolicited, "opt-in, never opt-out");
        assert_eq!(s.unsolicited_per_hour, DEFAULT_BUDGET_PER_HOUR);
    }

    #[test]
    fn an_old_document_without_the_new_fields_still_loads() {
        let old = r#"{"enabled":true,"default_persona":"abbey","learning_enabled":true,"voice_enabled":true,"vision_enabled":true,"reply_cooldown_seconds":20,"epsilon_override":null,"locale":"en"}"#;
        let s: GuildSettings = serde_json::from_str(old).expect("older row loads");
        assert!(!s.unsolicited);
        assert_eq!(s.unsolicited_per_hour, 6);
    }

    #[test]
    fn budget_clamps_to_one_through_sixty() {
        assert_eq!(clamp_budget(0), 1);
        assert_eq!(clamp_budget(-5), 1);
        assert_eq!(clamp_budget(6), 6);
        assert_eq!(clamp_budget(999), MAX_BUDGET_PER_HOUR);
    }

    #[test]
    fn scoped_ids_are_namespaced() {
        assert_eq!(scoped_guild_id("discord", Some("123")), "discord:123");
        assert_eq!(scoped_guild_id("discord", None), "discord:dm");
        assert_eq!(scoped_channel_id("discord", "9"), "discord:9");
        assert_eq!(scoped_user_id("telegram", "u1"), "telegram:u1");
    }
}

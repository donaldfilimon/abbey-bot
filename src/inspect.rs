//! Pure renders for the Inspect skill pack. No serenity, no songbird, no clock.
//!
//! `ToolScope` gathers already-redacted inputs; this module only formats them.

use crate::guild::GuildSettings;
use crate::memory::PendingSupersession;
use crate::tools::{self, InspectAspect, MAX_RESULT_CHARS};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

/// The complete voice vocabulary exposed by Inspect. It deliberately carries
/// no participant, consent, media, provider, counter, or timestamp detail.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum VoiceInspectState {
    #[default]
    Off,
    Presence,
    AwaitingConsent,
    Active,
    Paused,
}

impl VoiceInspectState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Presence => "presence",
            Self::AwaitingConsent => "awaiting-consent",
            Self::Active => "active",
            Self::Paused => "paused",
        }
    }
}

/// Guild-keyed, Songbird-free voice state published by the central voice
/// lifecycle. Missing entries are `off`; publishing `off` removes the entry.
#[derive(Debug, Default)]
pub struct VoiceInspectRegistry {
    states: Mutex<HashMap<String, VoiceInspectState>>,
}

impl VoiceInspectRegistry {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, VoiceInspectState>> {
        self.states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[must_use]
    pub fn state_for(&self, scoped_guild: &str) -> VoiceInspectState {
        self.lock().get(scoped_guild).copied().unwrap_or_default()
    }

    pub fn publish(&self, scoped_guild: &str, state: VoiceInspectState) {
        let mut states = self.lock();
        if state == VoiceInspectState::Off {
            states.remove(scoped_guild);
        } else {
            states.insert(scoped_guild.to_owned(), state);
        }
    }

    /// Close an active media view immediately when consent/media is revoked.
    pub fn mark_media_revoked(&self, scoped_guild: &str) {
        let mut states = self.lock();
        if states.get(scoped_guild) == Some(&VoiceInspectState::Active) {
            states.insert(scoped_guild.to_owned(), VoiceInspectState::Paused);
        }
    }

    /// Actor failure or another adverse session event must never leave stale
    /// active/presence copy visible to Inspect.
    pub fn mark_session_adverse(&self, scoped_guild: &str) {
        let mut states = self.lock();
        if matches!(
            states.get(scoped_guild),
            Some(VoiceInspectState::Active | VoiceInspectState::Presence)
        ) {
            states.insert(scoped_guild.to_owned(), VoiceInspectState::Paused);
        }
    }
}

/// Redacted process facts for the `runtime` Inspect aspect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeInspect {
    pub generation_configured: bool,
    pub tools_on: bool,
    pub vision_on: bool,
    pub quiet: bool,
    pub data: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderRouteLabel {
    Primary,
    FoundationModelsServer,
    FoundationModelsCli,
    Vision,
}

impl ProviderRouteLabel {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::FoundationModelsServer => "foundation-models-server",
            Self::FoundationModelsCli => "foundation-models-cli",
            Self::Vision => "vision",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderProvenance {
    Configuration,
    QualifiedManifest,
}

impl ProviderProvenance {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::QualifiedManifest => "qualified-manifest",
        }
    }
}

/// Closed, content-free provider view. Ineligible routes have every capability
/// cleared by construction, so configured-but-unavailable features cannot be
/// mistaken for effective capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderRouteInspect {
    route: ProviderRouteLabel,
    routable: bool,
    text: bool,
    tools: bool,
    vision: bool,
    ocr: bool,
    provenance: ProviderProvenance,
}

impl ProviderRouteInspect {
    #[must_use]
    pub const fn new(
        route: ProviderRouteLabel,
        routable: bool,
        text: bool,
        tools: bool,
        vision: bool,
        ocr: bool,
        provenance: ProviderProvenance,
    ) -> Self {
        Self {
            route,
            routable,
            text: routable && text,
            tools: routable && tools,
            vision: routable && vision,
            ocr: routable && ocr,
            provenance,
        }
    }
}

pub fn render_status(
    aspect: InspectAspect,
    runtime: &RuntimeInspect,
    guild_line: Option<&str>,
    voice: VoiceInspectState,
    providers: &[ProviderRouteInspect],
) -> String {
    match aspect {
        InspectAspect::Runtime => render_runtime(runtime),
        InspectAspect::Guild => guild_line
            .unwrap_or("No guild settings on record.")
            .to_string(),
        InspectAspect::Voice => render_voice(voice),
        InspectAspect::Provider => render_provider(providers),
        InspectAspect::All => tools::truncate(&format!(
            "{}\n{}\n{}\n{}",
            render_runtime(runtime),
            guild_line.unwrap_or("No guild settings on record."),
            render_voice(voice),
            render_provider(providers),
        )),
    }
}

pub fn render_runtime(runtime: &RuntimeInspect) -> String {
    format!(
        "generation: {} · tools: {} · vision: {} · quiet: {} · data: {}",
        if runtime.generation_configured {
            "configured"
        } else {
            "off"
        },
        on_off(runtime.tools_on),
        on_off(runtime.vision_on),
        on_off(runtime.quiet),
        if runtime.data { "yes" } else { "no" },
    )
}

pub fn render_provider(providers: &[ProviderRouteInspect]) -> String {
    if providers.is_empty() {
        return "provider: unavailable".into();
    }
    providers
        .iter()
        .map(|provider| {
            format!(
                "provider {}: routable {} · text {} · tools {} · vision {} · ocr {} · provenance {}",
                provider.route.as_str(),
                yes_no(provider.routable),
                yes_no(provider.text),
                yes_no(provider.tools),
                yes_no(provider.vision),
                yes_no(provider.ocr),
                provider.provenance.as_str(),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn render_voice(voice: VoiceInspectState) -> String {
    format!("voice: {}", voice.as_str())
}

/// Guild body without the markdown header (keeps `all` under the cap).
pub fn render_guild_body(settings: &GuildSettings, tokens_left: f32) -> String {
    format!(
        "persona: {} · learning: {} · vision: {} · cooldown: {}s · act: {} · budget: {}/h ({:.1} left)",
        crate::guild::persona_name(settings.default_persona),
        on_off(settings.learning_enabled),
        on_off(settings.vision_enabled),
        settings.reply_cooldown_seconds,
        on_off(settings.unsolicited),
        settings.unsolicited_per_hour,
        tokens_left,
    )
}

const PENDING_HEADER: &str =
    "Pending (not applied — old fact still stands; they confirm with /pending):";

pub fn render_facts(facts: &[String], pending: &[PendingSupersession]) -> String {
    if facts.is_empty() && pending.is_empty() {
        return "Nothing on record.".into();
    }

    // Pending replacements remain the higher-priority part of the snapshot:
    // users need the complete old/new pair before deciding whether to confirm
    // it. The zero-displayed candidate reserves both categories' exact
    // remainder lines before any item is admitted.
    let displayed_pending = (0..=pending.len())
        .rev()
        .find(|&count| {
            render_facts_candidate(facts, pending, 0, count)
                .chars()
                .count()
                <= MAX_RESULT_CHARS
        })
        .unwrap_or(0);
    let displayed_facts = (0..=facts.len())
        .rev()
        .find(|&count| {
            render_facts_candidate(facts, pending, count, displayed_pending)
                .chars()
                .count()
                <= MAX_RESULT_CHARS
        })
        .unwrap_or(0);

    let body = render_facts_candidate(facts, pending, displayed_facts, displayed_pending);
    debug_assert!(body.chars().count() <= MAX_RESULT_CHARS);
    body
}

fn render_facts_candidate(
    facts: &[String],
    pending: &[PendingSupersession],
    displayed_facts: usize,
    displayed_pending: usize,
) -> String {
    debug_assert!(displayed_facts <= facts.len());
    debug_assert!(displayed_pending <= pending.len());

    let mut body = String::new();
    if !facts.is_empty() {
        body.push_str(&format!("Facts ({}):", facts.len()));
        for fact in facts.iter().take(displayed_facts) {
            body.push_str(&format!("\n• {fact}"));
        }
        let omitted_facts = facts.len() - displayed_facts;
        if omitted_facts > 0 {
            body.push_str(&format!("\n… ({omitted_facts} more facts)"));
        }
    }

    if !pending.is_empty() {
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(PENDING_HEADER);
        for item in pending.iter().take(displayed_pending) {
            body.push_str(&format!("\n• {} → {}", item.old_fact, item.new_fact));
        }
        let omitted_pending = pending.len() - displayed_pending;
        if omitted_pending > 0 {
            body.push_str(&format!(
                "\n… ({omitted_pending} more pending replacements)"
            ));
        }
    }

    body
}

fn on_off(flag: bool) -> &'static str {
    if flag { "on" } else { "off" }
}

fn yes_no(flag: bool) -> &'static str {
    if flag { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persona::Persona;

    fn runtime() -> RuntimeInspect {
        RuntimeInspect {
            generation_configured: true,
            tools_on: true,
            vision_on: true,
            quiet: false,
            data: true,
        }
    }

    #[test]
    fn runtime_line_has_no_path_or_url() {
        let line = render_runtime(&runtime());
        assert!(line.contains("generation: configured"));
        assert!(line.contains("data: yes"));
        assert!(!line.contains("127.0.0.1"));
        assert!(!line.contains("/Users"));
        assert!(!line.contains("http"));
    }

    #[test]
    fn voice_render_has_exactly_the_coarse_vocabulary() {
        let cases = [
            (VoiceInspectState::Off, "voice: off"),
            (VoiceInspectState::Presence, "voice: presence"),
            (
                VoiceInspectState::AwaitingConsent,
                "voice: awaiting-consent",
            ),
            (VoiceInspectState::Active, "voice: active"),
            (VoiceInspectState::Paused, "voice: paused"),
        ];
        for (state, expected) in cases {
            assert_eq!(render_voice(state), expected);
        }
    }

    #[test]
    fn voice_registry_is_guild_scoped_and_adverse_events_close_active_copy() {
        let registry = VoiceInspectRegistry::default();
        assert_eq!(registry.state_for("discord:one"), VoiceInspectState::Off);

        registry.publish("discord:one", VoiceInspectState::Presence);
        assert_eq!(
            registry.state_for("discord:one"),
            VoiceInspectState::Presence
        );
        assert_eq!(registry.state_for("discord:two"), VoiceInspectState::Off);
        registry.mark_session_adverse("discord:one");
        assert_eq!(registry.state_for("discord:one"), VoiceInspectState::Paused);

        registry.publish("discord:one", VoiceInspectState::Active);
        registry.mark_media_revoked("discord:one");
        assert_eq!(registry.state_for("discord:one"), VoiceInspectState::Paused);

        registry.publish("discord:one", VoiceInspectState::Off);
        assert_eq!(registry.state_for("discord:one"), VoiceInspectState::Off);
    }

    #[test]
    fn provider_render_uses_only_closed_safe_fields() {
        let routes = [
            ProviderRouteInspect::new(
                ProviderRouteLabel::Primary,
                true,
                true,
                true,
                false,
                false,
                ProviderProvenance::Configuration,
            ),
            ProviderRouteInspect::new(
                ProviderRouteLabel::FoundationModelsCli,
                false,
                true,
                true,
                true,
                true,
                ProviderProvenance::QualifiedManifest,
            ),
        ];
        let text = render_provider(&routes);
        assert!(text.contains("provider primary: routable yes"));
        assert!(text.contains("foundation-models-cli: routable no"));
        assert!(text.contains("text no · tools no · vision no · ocr no"));
        for canary in [
            "http://",
            "/Users/",
            "gemma",
            "on-device model",
            "hash",
            "manifest path",
            "api key",
            "credential",
            "provider error",
        ] {
            assert!(!text.contains(canary), "leaked canary {canary:?}: {text}");
        }
    }

    #[test]
    fn no_provider_is_reported_without_inventing_capability() {
        assert_eq!(render_provider(&[]), "provider: unavailable");
    }

    #[test]
    fn facts_empty() {
        assert_eq!(render_facts(&[], &[]), "Nothing on record.");
    }

    #[test]
    fn facts_and_pending_disclose_unapplied() {
        let pending = [PendingSupersession {
            new_fact: "moved to zig".into(),
            old_fact: "uses rust".into(),
            at: 1,
        }];
        let text = render_facts(&["builds in nightly Rust".into()], &pending);
        assert!(text.contains("• builds in nightly Rust"));
        assert!(text.contains("not applied"));
        assert!(text.contains("uses rust → moved to zig"));
    }

    #[test]
    fn long_fact_list_names_the_remainder() {
        let facts: Vec<String> = (0..80)
            .map(|i| format!("fact-{i} is a reasonably long remembered sentence"))
            .collect();
        let text = render_facts(&facts, &[]);
        assert!(text.contains("more facts"), "{text}");
        assert!(text.chars().count() <= MAX_RESULT_CHARS);
    }

    #[test]
    fn long_fact_and_pending_lists_name_each_exact_remainder() {
        let facts: Vec<String> = (0..80)
            .map(|i| format!("fact-{i} is a reasonably long remembered sentence"))
            .collect();
        let pending: Vec<PendingSupersession> = (0..12)
            .map(|i| PendingSupersession {
                new_fact: format!("new-{i} replacement with enough detail to consume space"),
                old_fact: format!("old-{i} fact with enough detail to consume space"),
                at: i,
            })
            .collect();

        let text = render_facts(&facts, &pending);
        let displayed_facts = text
            .lines()
            .filter(|line| line.starts_with("• fact-"))
            .count();
        let displayed_pending = text
            .lines()
            .filter(|line| line.starts_with("• old-"))
            .count();

        assert!(displayed_facts < facts.len(), "{text}");
        assert!(displayed_pending < pending.len(), "{text}");
        assert!(text.contains(&format!("… ({} more facts)", facts.len() - displayed_facts)));
        assert!(text.contains(&format!(
            "… ({} more pending replacements)",
            pending.len() - displayed_pending
        )));
        assert!(text.chars().count() <= MAX_RESULT_CHARS);
    }

    #[test]
    fn oversized_pending_pair_is_omitted_whole() {
        let pending = [PendingSupersession {
            new_fact: format!("new-oversized-{}", "n".repeat(MAX_RESULT_CHARS)),
            old_fact: format!("old-oversized-{}", "o".repeat(MAX_RESULT_CHARS)),
            at: 1,
        }];

        let text = render_facts(&[], &pending);

        assert!(!text.contains("old-oversized"), "{text}");
        assert!(!text.contains("new-oversized"), "{text}");
        assert!(text.contains("… (1 more pending replacements)"), "{text}");
        assert!(text.chars().count() <= MAX_RESULT_CHARS);
    }

    #[test]
    fn pending_pair_at_boundary_is_never_partially_rendered() {
        let pending = [
            PendingSupersession {
                new_fact: "first new".into(),
                old_fact: "first old".into(),
                at: 1,
            },
            PendingSupersession {
                new_fact: format!("second-new-{}", "n".repeat(MAX_RESULT_CHARS)),
                old_fact: "second-old".into(),
                at: 2,
            },
        ];

        let text = render_facts(&[], &pending);

        assert!(text.contains("• first old → first new"), "{text}");
        assert!(!text.contains("second-old"), "{text}");
        assert!(!text.contains("second-new"), "{text}");
        assert!(text.contains("… (1 more pending replacements)"), "{text}");
        assert!(text.chars().count() <= MAX_RESULT_CHARS);
    }

    #[test]
    fn guild_body_keeps_admin_fields() {
        let settings = GuildSettings {
            default_persona: Persona::Aviva,
            learning_enabled: true,
            vision_enabled: false,
            reply_cooldown_seconds: 20,
            unsolicited: true,
            unsolicited_per_hour: 6,
            ..GuildSettings::default()
        };
        let line = render_guild_body(&settings, 4.0);
        assert!(line.contains("persona: aviva"));
        assert!(line.contains("act: on"));
        assert!(line.contains("4.0 left"));
        assert!(!line.contains("**Abbey"));
    }
}

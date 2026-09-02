//! Pure, bounded renders for the Inspect skill pack.
//!
//! `ToolScope` gathers already-scoped inputs; this module only formats them.

use crate::guild::GuildSettings;
use crate::memory::PendingSupersession;
use crate::tools::{self, InspectAspect, MAX_RESULT_CHARS};

/// Redacted process facts for the runtime Inspect line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeInspect {
    pub backend_label: &'static str,
    pub tools_on: bool,
    pub vision_on: bool,
    pub quiet: bool,
    pub data: bool,
    pub fm: &'static str,
}

pub fn render_status(
    aspect: InspectAspect,
    runtime: &RuntimeInspect,
    guild_line: Option<&str>,
) -> String {
    match aspect {
        InspectAspect::Runtime => render_runtime(runtime),
        InspectAspect::Guild => guild_line
            .unwrap_or("No guild settings on record.")
            .to_string(),
        // Fixed copy cannot expose configured-but-unavailable provider data
        // or process-global rich voice state.
        InspectAspect::Voice => "voice: off".into(),
        InspectAspect::Provider => "provider: unavailable".into(),
        InspectAspect::All => tools::truncate(&format!(
            "{}\n{}\nvoice: off\nprovider: unavailable",
            render_runtime(runtime),
            guild_line.unwrap_or("No guild settings on record."),
        )),
    }
}

pub fn render_runtime(runtime: &RuntimeInspect) -> String {
    format!(
        "backend: {} · tools: {} · vision: {} · quiet: {} · data: {} · fm: {}",
        runtime.backend_label,
        on_off(runtime.tools_on),
        on_off(runtime.vision_on),
        on_off(runtime.quiet),
        if runtime.data { "yes" } else { "no" },
        runtime.fm,
    )
}

/// Guild body without the markdown header.
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

    // Pending replacements are higher-priority because users need a complete
    // old/new pair before deciding whether to confirm it. Candidate rendering
    // reserves both categories' exact remainder lines before admitting items.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persona::Persona;

    fn runtime() -> RuntimeInspect {
        RuntimeInspect {
            backend_label: "configured OpenAI-compatible endpoint",
            tools_on: true,
            vision_on: true,
            quiet: false,
            data: true,
            fm: "off",
        }
    }

    #[test]
    fn runtime_line_has_no_path_or_url() {
        let line = render_runtime(&runtime());
        assert!(line.contains("backend: configured OpenAI-compatible endpoint"));
        assert!(line.contains("data: yes"));
        assert!(!line.contains("127.0.0.1"));
        assert!(!line.contains("/Users"));
        assert!(!line.contains("http"));
    }

    #[test]
    fn provider_and_voice_are_fixed_safe_copy() {
        assert_eq!(
            render_status(InspectAspect::Voice, &runtime(), None),
            "voice: off"
        );
        assert_eq!(
            render_status(InspectAspect::Provider, &runtime(), None),
            "provider: unavailable"
        );
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

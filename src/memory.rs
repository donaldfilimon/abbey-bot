//! Pure, serde-derived memory state: per-user facts and standing, per-channel
//! rolling context, and the slash-command interaction log.
//!
//! This is the port of the spec's Fluent models (`UserMemory`, `ChannelContext`,
//! `InteractionLog`, `GuildMessage` — bot-architecture.md) and of
//! platforms.md's `MemoryAssembler`, as plain structs with no database behind
//! them. The orchestrator owns persistence (it serializes a [`MemoryBank`] to
//! JSON); nothing here touches the filesystem, the network, or the clock. Every
//! mutation takes `now: u64` (unix seconds) from the caller, so the suite runs
//! with no `SystemTime` and timestamps are assertable.
//!
//! Scoping is the caller's job: keys are the already-scoped ids the platform
//! layer hands over (`"{guild}:{user}"` for users, the scoped channel id for
//! channels). Nothing here imports serenity or poise.

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

/// Most recent messages a channel keeps verbatim. Older ones survive only
/// through `ChannelContext::summary`.
pub const RECENT_CAP: usize = 50;
/// Facts one user may hold in one guild. Exact duplicates never count twice.
pub const MAX_FACTS: usize = 100;
/// Longest durable fact, measured in Unicode scalar values after whitespace
/// normalization. This is shared by slash commands and model tools so every
/// write path enforces the same storage/context bound.
pub const MAX_FACT_CHARS: usize = 300;
/// Interaction entries kept in process before the oldest fall off.
pub const INTERACTION_CAP: usize = 1000;
/// The standing a never-seen user starts with, per `MemoryAssembler`
/// (`userMem?.reputation ?? 0.5`).
pub const DEFAULT_REPUTATION: f64 = 0.5;
/// Discord's hard cap on autocomplete choices.
pub const AUTOCOMPLETE_MAX_CHOICES: usize = 25;
/// Discord's hard cap on an autocomplete choice name, in characters.
pub const AUTOCOMPLETE_MAX_CHARS: usize = 100;

fn default_reputation() -> f64 {
    DEFAULT_REPUTATION
}

/// Collapse all whitespace in a durable fact to single ASCII spaces.
#[must_use]
pub fn normalize_fact_text(fact: &str) -> String {
    fact.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Normalize and validate one durable fact before it reaches either memory
/// representation. The returned messages are safe, fixed user-facing copy.
pub fn validated_fact(fact: &str) -> Result<String, &'static str> {
    let normalized = normalize_fact_text(fact);
    if normalized.is_empty() {
        return Err("The fact must contain some text.");
    }
    if normalized.chars().count() > MAX_FACT_CHARS {
        return Err("Keep one remembered fact to 300 characters or fewer.");
    }
    Ok(normalized)
}

/// Per-user fact store + reputation (bot-architecture.md `UserMemory`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserMemory {
    #[serde(default)]
    pub facts: Vec<String>,
    /// 0.0 – 1.0.
    #[serde(default = "default_reputation")]
    pub reputation: f64,
    #[serde(default)]
    pub interaction_count: u64,
    #[serde(default)]
    pub updated_at: u64,
}

impl Default for UserMemory {
    fn default() -> Self {
        Self {
            facts: Vec::new(),
            reputation: DEFAULT_REPUTATION,
            interaction_count: 0,
            updated_at: 0,
        }
    }
}

/// One verbatim recent message (the in-memory stand-in for `GuildMessage`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentMessage {
    pub author: String,
    pub text: String,
    pub at: u64,
}

/// Rolling per-channel context (bot-architecture.md `ChannelContext`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChannelContext {
    /// Compressed summary of older traffic; free text, filled by `/summarize`.
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub message_count: u64,
    /// Oldest at the front, newest at the back; never longer than
    /// [`RECENT_CAP`].
    #[serde(default)]
    pub recent: VecDeque<RecentMessage>,
    #[serde(default)]
    pub updated_at: u64,
    /// The scoped guild this channel's traffic belongs to, recorded by the
    /// pipeline so the summariser can check the guild's opt-in. `None` until
    /// the first message arrives through the pipeline.
    #[serde(default)]
    pub guild: Option<String>,
    /// `message_count` when `summary` was last refreshed by the rolling
    /// summariser; the scheduler re-summarises after [`SUMMARY_EVERY_MESSAGES`]
    /// more have arrived.
    #[serde(default)]
    pub summarized_at_count: u64,
}

/// New messages in a channel before its rolling summary is refreshed.
pub const SUMMARY_EVERY_MESSAGES: u64 = 30;

impl ChannelContext {
    /// Whether the rolling summariser should run for this channel now.
    pub fn summary_due(&self) -> bool {
        self.message_count >= self.summarized_at_count + SUMMARY_EVERY_MESSAGES
            && !self.recent.is_empty()
    }

    /// Append one message, dropping the oldest past [`RECENT_CAP`]. Counts
    /// every message ever seen, not just the ones still held.
    pub fn push_recent(&mut self, author: &str, text: &str, now: u64) {
        self.recent.push_back(RecentMessage {
            author: author.to_string(),
            text: text.to_string(),
            at: now,
        });
        while self.recent.len() > RECENT_CAP {
            self.recent.pop_front();
        }
        self.message_count += 1;
        self.updated_at = now;
    }

    /// The last `limit` messages, oldest first, one `author: text` line each —
    /// the transcript `/summarize` hands the model.
    pub fn render_recent(&self, limit: usize) -> String {
        let skip = self.recent.len().saturating_sub(limit);
        self.recent
            .iter()
            .skip(skip)
            .map(|m| format!("{}: {}", m.author, m.text))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// One slash-command invocation (bot-architecture.md `InteractionLog` row).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractionEntry {
    pub command: String,
    pub user_id: String,
    pub guild_id: String,
    pub channel_id: String,
    pub succeeded: bool,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub at: u64,
}

/// Slash-command usage analytics, capped at [`INTERACTION_CAP`] entries.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InteractionLog {
    #[serde(default)]
    pub entries: VecDeque<InteractionEntry>,
}

/// What `/stats` reports.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InteractionStats {
    pub total: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub per_command: BTreeMap<String, u64>,
}

impl InteractionLog {
    pub fn record(&mut self, entry: InteractionEntry) {
        self.entries.push_back(entry);
        while self.entries.len() > INTERACTION_CAP {
            self.entries.pop_front();
        }
    }

    pub fn stats(&self) -> InteractionStats {
        let mut stats = InteractionStats::default();
        for entry in &self.entries {
            stats.total += 1;
            if entry.succeeded {
                stats.succeeded += 1;
            } else {
                stats.failed += 1;
            }
            *stats.per_command.entry(entry.command.clone()).or_insert(0) += 1;
        }
        stats
    }
}

/// The `/stats` reply body. Commands are listed alphabetically (the map is
/// ordered), so the output is stable across runs.
pub fn render_stats(stats: &InteractionStats) -> String {
    let mut out = format!(
        "**Interactions:** {} total — {} succeeded, {} failed",
        stats.total, stats.succeeded, stats.failed
    );
    if stats.per_command.is_empty() {
        out.push_str("\nNo commands recorded yet.");
    } else {
        out.push_str("\n**By command:**");
        for (command, count) in &stats.per_command {
            out.push_str(&format!("\n• /{command}: {count}"));
        }
    }
    out
}

/// What a persona sees about the conversation it is answering in
/// (bot-architecture.md `PersonaContext`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonaContext {
    pub channel_summary: String,
    pub user_facts: Vec<String>,
    pub reputation: f64,
}

impl PersonaContext {
    /// The spec's `.empty`: nothing known, neutral standing.
    pub fn empty() -> Self {
        Self {
            channel_summary: String::new(),
            user_facts: Vec::new(),
            reputation: DEFAULT_REPUTATION,
        }
    }

    /// The context block appended to the system prompt, focused on `query`.
    ///
    /// Originally a line-for-line port of appleintelligence.md's `render(_ c:)`,
    /// which joined *every* stored fact. At the hundred-fact cap that put tens
    /// of thousands of characters of biography in front of every message, so
    /// facts are now ranked for the message being answered — see
    /// [`crate::recall`]. Pass the user's message as `query`; an empty query
    /// degrades to newest-first, never to nothing.
    ///
    /// Ranking is not forgetting: held-back facts are counted in the output so
    /// the model knows more is on file, and `/recall` still lists everything.
    pub fn render(&self, query: &str) -> String {
        let mut out = String::new();
        if !self.channel_summary.is_empty() {
            out.push_str(&format!(
                "Recent channel context: {}\n",
                self.channel_summary
            ));
        }
        if !self.user_facts.is_empty() {
            let picked = crate::recall::select(
                &self.user_facts,
                query,
                crate::recall::MAX_CONTEXT_FACTS,
                crate::recall::FACT_CONTEXT_CHARS,
            );
            out.push_str(&format!(
                "Known about this user: {}",
                picked.facts.join("; ")
            ));
            if picked.omitted > 0 {
                // Disclose the trim rather than implying this is everything on
                // file; the model can offer to look further instead of
                // asserting from a partial view.
                out.push_str(&format!(
                    " (+{} more remembered facts not shown for this message)",
                    picked.omitted
                ));
            }
            out.push('\n');
        }
        // A bare number is unusable context: a model reading "User standing:
        // 0.50" cannot tell the scale, the neutral point, or whether the value
        // is something to act on or to repeat back. Naming all three makes the
        // signal actionable and keeps an internal score from surfacing as
        // chat text.
        out.push_str(&format!(
            "User standing: {:.2} on a 0.00-1.00 scale where {DEFAULT_REPUTATION:.2} is neutral \
             (higher is a longer history of constructive participation). This is internal \
             context for judging tone and benefit of the doubt; never mention, quote, or explain \
             it to the user.",
            self.reputation
        ));
        out
    }
}

impl Default for PersonaContext {
    fn default() -> Self {
        Self::empty()
    }
}

/// The in-process store. The orchestrator persists it whole as JSON.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MemoryBank {
    /// Keyed `"{scoped_guild}:{scoped_user}"`.
    #[serde(default)]
    pub users: BTreeMap<String, UserMemory>,
    /// Keyed by scoped channel id.
    #[serde(default)]
    pub channels: BTreeMap<String, ChannelContext>,
    #[serde(default)]
    pub interactions: InteractionLog,
    #[serde(default)]
    pub messages_seen: u64,
}

fn user_key(guild: &str, user: &str) -> String {
    format!("{guild}:{user}")
}

impl MemoryBank {
    pub fn user(&self, guild: &str, user: &str) -> Option<&UserMemory> {
        self.users.get(&user_key(guild, user))
    }

    /// The user's memory, provisioning a default one on first sight.
    pub fn user_mut(&mut self, guild: &str, user: &str) -> &mut UserMemory {
        self.users.entry(user_key(guild, user)).or_default()
    }

    /// Store one fact. Returns `false` — and stores nothing — when the exact
    /// fact is already held or the user is at [`MAX_FACTS`].
    pub fn remember(&mut self, guild: &str, user: &str, fact: &str, now: u64) -> bool {
        let memory = self.user_mut(guild, user);
        if memory.facts.iter().any(|f| f == fact) || memory.facts.len() >= MAX_FACTS {
            return false;
        }
        memory.facts.push(fact.to_string());
        memory.updated_at = now;
        true
    }

    /// Drop one fact by exact text. Returns whether anything was removed.
    pub fn forget(&mut self, guild: &str, user: &str, fact: &str) -> bool {
        let Some(memory) = self.users.get_mut(&user_key(guild, user)) else {
            return false;
        };
        let before = memory.facts.len();
        memory.facts.retain(|f| f != fact);
        memory.facts.len() != before
    }

    pub fn facts(&self, guild: &str, user: &str) -> &[String] {
        self.user(guild, user).map_or(&[], |m| m.facts.as_slice())
    }

    pub fn channel_mut(&mut self, scoped_channel: &str) -> &mut ChannelContext {
        self.channels.entry(scoped_channel.to_string()).or_default()
    }

    /// One gateway message: pushed into the channel's recent window, counted.
    pub fn record_message(&mut self, scoped_channel: &str, author: &str, text: &str, now: u64) {
        self.channel_mut(scoped_channel)
            .push_recent(author, text, now);
        self.messages_seen += 1;
    }

    /// `MemoryAssembler.context(for:)`: what is known, defaults where nothing is.
    /// Scoped channel ids whose rolling summary is due.
    pub fn channels_due_for_summary(&self) -> Vec<String> {
        self.channels
            .iter()
            .filter(|(_, c)| c.summary_due())
            .map(|(k, _)| k.clone())
            .collect()
    }

    pub fn context_for(&self, guild: &str, user: &str, scoped_channel: &str) -> PersonaContext {
        let memory = self.user(guild, user);
        PersonaContext {
            channel_summary: self
                .channels
                .get(scoped_channel)
                .map(|c| c.summary.clone())
                .unwrap_or_default(),
            user_facts: memory.map(|m| m.facts.clone()).unwrap_or_default(),
            reputation: memory.map_or(DEFAULT_REPUTATION, |m| m.reputation),
        }
    }
}

/// `/forget` autocomplete (companion-app.md): case-insensitive substring
/// matches, at most [`AUTOCOMPLETE_MAX_CHOICES`], each cut to
/// [`AUTOCOMPLETE_MAX_CHARS`] characters — both are Discord-enforced caps that
/// fail the interaction rather than truncating.
pub fn autocomplete_facts<'a>(facts: &'a [String], partial: &str) -> Vec<&'a str> {
    let needle = partial.to_lowercase();
    facts
        .iter()
        .filter(|f| f.to_lowercase().contains(&needle))
        .take(AUTOCOMPLETE_MAX_CHOICES)
        .map(|f| {
            let end = f
                .char_indices()
                .nth(AUTOCOMPLETE_MAX_CHARS)
                .map_or(f.len(), |(i, _)| i);
            &f[..end]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_fact_validation_is_shared_and_unicode_bounded() {
        assert_eq!(
            validated_fact("  Donald\nlikes\tRust.  "),
            Ok("Donald likes Rust.".to_string())
        );
        assert_eq!(
            validated_fact(" \n\t "),
            Err("The fact must contain some text.")
        );
        assert!(validated_fact(&"x".repeat(MAX_FACT_CHARS)).is_ok());
        assert_eq!(
            validated_fact(&"🦀".repeat(MAX_FACT_CHARS + 1)),
            Err("Keep one remembered fact to 300 characters or fewer.")
        );
    }

    #[test]
    fn remember_dedupes_exact_duplicates() {
        let mut bank = MemoryBank::default();
        assert!(bank.remember("g", "u", "likes rust", 10));
        assert!(!bank.remember("g", "u", "likes rust", 11));
        // Case differs, so it is a different fact — dedupe is exact.
        assert!(bank.remember("g", "u", "Likes Rust", 12));
        assert_eq!(bank.facts("g", "u"), ["likes rust", "Likes Rust"]);
        assert_eq!(bank.user("g", "u").expect("provisioned").updated_at, 12);
    }

    #[test]
    fn remember_caps_facts_at_one_hundred() {
        let mut bank = MemoryBank::default();
        for i in 0..MAX_FACTS {
            assert!(bank.remember("g", "u", &format!("fact {i}"), 1));
        }
        assert!(!bank.remember("g", "u", "one too many", 2));
        assert_eq!(bank.facts("g", "u").len(), MAX_FACTS);
    }

    #[test]
    fn forget_removes_by_exact_text_and_reports_absence() {
        let mut bank = MemoryBank::default();
        bank.remember("g", "u", "a", 1);
        bank.remember("g", "u", "b", 1);
        assert!(bank.forget("g", "u", "a"));
        assert!(!bank.forget("g", "u", "a"));
        assert!(!bank.forget("g", "nobody", "a"));
        assert_eq!(bank.facts("g", "u"), ["b"]);
    }

    #[test]
    fn users_are_scoped_per_guild() {
        let mut bank = MemoryBank::default();
        bank.remember("g1", "u", "here", 1);
        assert!(bank.facts("g2", "u").is_empty());
        assert_eq!(bank.user("g2", "u"), None);
        bank.user_mut("g2", "u").reputation = 0.9;
        assert_eq!(bank.user("g1", "u").expect("kept").reputation, 0.5);
    }

    #[test]
    fn a_channel_is_due_for_summary_every_thirty_messages() {
        let mut bank = MemoryBank::default();
        for i in 0..29 {
            bank.record_message("discord:c", "a", &format!("m{i}"), i);
        }
        assert!(bank.channels_due_for_summary().is_empty());
        bank.record_message("discord:c", "a", "m29", 29);
        assert_eq!(bank.channels_due_for_summary(), ["discord:c"]);
        let ctx = bank.channel_mut("discord:c");
        ctx.summary = "so far".into();
        ctx.summarized_at_count = ctx.message_count;
        assert!(bank.channels_due_for_summary().is_empty());
    }

    #[test]
    fn recent_window_caps_at_fifty_and_keeps_the_newest() {
        let mut bank = MemoryBank::default();
        for i in 0..60 {
            bank.record_message("c", "alice", &format!("m{i}"), i);
        }
        let channel = &bank.channels["c"];
        assert_eq!(channel.recent.len(), RECENT_CAP);
        assert_eq!(channel.recent.front().expect("some").text, "m10");
        assert_eq!(channel.recent.back().expect("some").text, "m59");
        assert_eq!(channel.message_count, 60);
        assert_eq!(channel.updated_at, 59);
        assert_eq!(bank.messages_seen, 60);
    }

    #[test]
    fn render_recent_is_oldest_to_newest_and_limited() {
        let mut ctx = ChannelContext::default();
        ctx.push_recent("a", "one", 1);
        ctx.push_recent("b", "two", 2);
        ctx.push_recent("a", "three", 3);
        assert_eq!(ctx.render_recent(2), "b: two\na: three");
        assert_eq!(ctx.render_recent(10), "a: one\nb: two\na: three");
        assert_eq!(ctx.render_recent(0), "");
    }

    fn entry(command: &str, ok: bool) -> InteractionEntry {
        InteractionEntry {
            command: command.to_string(),
            user_id: "u".into(),
            guild_id: "g".into(),
            channel_id: "c".into(),
            succeeded: ok,
            error: (!ok).then(|| "boom".to_string()),
            duration_ms: 5,
            at: 1,
        }
    }

    #[test]
    fn stats_count_totals_outcomes_and_per_command() {
        let mut log = InteractionLog::default();
        log.record(entry("ask", true));
        log.record(entry("ask", false));
        log.record(entry("whois", true));
        let stats = log.stats();
        assert_eq!(stats.total, 3);
        assert_eq!(stats.succeeded, 2);
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.per_command["ask"], 2);
        assert_eq!(stats.per_command["whois"], 1);
    }

    #[test]
    fn interaction_log_caps_at_one_thousand() {
        let mut log = InteractionLog::default();
        for i in 0..(INTERACTION_CAP + 5) {
            let mut e = entry("ask", true);
            e.at = i as u64;
            log.record(e);
        }
        assert_eq!(log.entries.len(), INTERACTION_CAP);
        assert_eq!(log.entries.front().expect("some").at, 5);
    }

    #[test]
    fn render_stats_snapshot() {
        let mut log = InteractionLog::default();
        log.record(entry("whois", true));
        log.record(entry("ask", false));
        log.record(entry("ask", true));
        assert_eq!(
            render_stats(&log.stats()),
            "**Interactions:** 3 total — 2 succeeded, 1 failed\n**By command:**\n• /ask: 2\n• /whois: 1"
        );
        assert_eq!(
            render_stats(&InteractionStats::default()),
            "**Interactions:** 0 total — 0 succeeded, 0 failed\nNo commands recorded yet."
        );
    }

    #[test]
    fn persona_context_render_snapshot() {
        let ctx = PersonaContext {
            channel_summary: "talking about deploys".into(),
            user_facts: vec!["likes rust".into(), "runs a homelab".into()],
            reputation: 0.5,
        };
        // Both facts fit the budget, so a focused render shows both and adds
        // no "not shown" note. The query names only "rust", so that fact
        // outranks the homelab one — a query naming both would tie them and
        // let recency decide the order instead.
        assert_eq!(
            ctx.render("a rust question"),
            "Recent channel context: talking about deploys\nKnown about this user: likes rust; runs a homelab\nUser standing: 0.50 on a 0.00-1.00 scale where 0.50 is neutral (higher is a longer history of constructive participation). This is internal context for judging tone and benefit of the doubt; never mention, quote, or explain it to the user."
        );
        // Empty context renders only the standing line — no blank headers.
        assert!(
            PersonaContext::empty()
                .render("anything")
                .starts_with("User standing: 0.50 on a 0.00-1.00 scale")
        );
        assert!(!PersonaContext::empty().render("anything").contains('\n'));
        let mut high = PersonaContext::empty();
        high.reputation = 0.8765;
        assert!(high.render("").starts_with("User standing: 0.88 "));
        // The score is decision support, not chat material: the instruction
        // that keeps an internal number out of replies must travel with it.
        assert!(
            high.render("")
                .contains("never mention, quote, or explain it")
        );
    }

    #[test]
    fn render_focuses_facts_on_the_message_and_discloses_the_trim() {
        // Ten facts, one obviously about the question. The prompt must lead
        // with that one and must not silently imply it is the whole file.
        let mut user_facts: Vec<String> = (0..9).map(|i| format!("unrelated fact {i}")).collect();
        user_facts.push("deploys with kubernetes".into());
        let ctx = PersonaContext {
            channel_summary: String::new(),
            user_facts,
            reputation: DEFAULT_REPUTATION,
        };

        let rendered = ctx.render("how do I roll back a kubernetes deploy?");
        assert!(
            rendered.contains("Known about this user: deploys with kubernetes"),
            "the relevant fact must come first: {rendered}"
        );
        assert!(
            rendered.contains("(+2 more remembered facts not shown for this message)"),
            "the trim must be disclosed: {rendered}"
        );
        // Ranking is not forgetting: everything is still on file.
        assert_eq!(ctx.user_facts.len(), 10);
    }

    #[test]
    fn a_short_fact_list_is_never_trimmed_by_focusing() {
        // The common case — a handful of facts — must behave exactly as it did
        // before relevance selection existed, whatever the message says.
        let ctx = PersonaContext {
            channel_summary: String::new(),
            user_facts: vec!["likes tea".into(), "uses nixos".into()],
            reputation: DEFAULT_REPUTATION,
        };
        let rendered = ctx.render("totally unrelated question about pottery");
        assert!(rendered.contains("likes tea"));
        assert!(rendered.contains("uses nixos"));
        assert!(!rendered.contains("not shown for this message"));
    }

    #[test]
    fn context_for_assembles_from_both_stores_with_defaults() {
        let mut bank = MemoryBank::default();
        assert_eq!(bank.context_for("g", "u", "c"), PersonaContext::empty());
        bank.remember("g", "u", "fact", 1);
        bank.user_mut("g", "u").reputation = 0.7;
        bank.channel_mut("c").summary = "summary".into();
        let ctx = bank.context_for("g", "u", "c");
        assert_eq!(ctx.channel_summary, "summary");
        assert_eq!(ctx.user_facts, ["fact"]);
        assert_eq!(ctx.reputation, 0.7);
    }

    #[test]
    fn autocomplete_is_case_insensitive_and_capped() {
        let facts: Vec<String> = (0..40).map(|i| format!("Likes Thing {i}")).collect();
        let matches = autocomplete_facts(&facts, "likes thing");
        assert_eq!(matches.len(), AUTOCOMPLETE_MAX_CHOICES);
        assert_eq!(matches[0], "Likes Thing 0");
        assert!(!autocomplete_facts(&facts, "THING 3").is_empty());
        assert!(autocomplete_facts(&facts, "zzz").is_empty());
        // Empty partial matches everything (capped).
        assert_eq!(
            autocomplete_facts(&facts, "").len(),
            AUTOCOMPLETE_MAX_CHOICES
        );
    }

    #[test]
    fn autocomplete_truncates_each_choice_to_one_hundred_chars() {
        let long = "é".repeat(150);
        let facts = vec![long];
        let matches = autocomplete_facts(&facts, "é");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].chars().count(), AUTOCOMPLETE_MAX_CHARS);
    }

    #[test]
    fn bank_round_trips_through_json() {
        let mut bank = MemoryBank::default();
        bank.remember("g", "u", "fact", 1);
        bank.record_message("c", "a", "hi", 2);
        bank.interactions.record(entry("ask", true));
        let json = serde_json::to_string(&bank).expect("serializes");
        let back: MemoryBank = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back, bank);
        // A bare object deserializes to defaults, so an older file still loads.
        let minimal: UserMemory = serde_json::from_str("{}").expect("defaults");
        assert_eq!(minimal.reputation, DEFAULT_REPUTATION);
    }
}

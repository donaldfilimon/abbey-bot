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

/// Unambiguous separator for newly written `(guild, user)` map keys. Scoped
/// ids themselves contain `:`, so the historical `"{guild}:{user}"` join
/// could not be decoded for projection reconciliation.
const USER_KEY_SEPARATOR: char = '\u{1f}';

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

    fn selected_facts<'a>(&'a self, query: &str) -> crate::recall::Selection<'a> {
        crate::recall::select(
            &self.user_facts,
            query,
            crate::recall::MAX_CONTEXT_FACTS,
            crate::recall::FACT_CONTEXT_CHARS,
        )
    }

    /// Plain context fragments actually selected for this request.
    ///
    /// This is the read-only factual boundary used by reply grounding. It
    /// deliberately excludes the fixed prompt copy and ambient reputation:
    /// the former contains instruction-only numbers, while the latter may be
    /// reported only through an authorized `lookup_reputation` tool result.
    pub fn grounding_sources<'a>(&'a self, query: &str) -> Vec<&'a str> {
        let mut sources = Vec::new();
        if !self.channel_summary.is_empty() {
            sources.push(self.channel_summary.as_str());
        }
        sources.extend(self.selected_facts(query).facts);
        sources
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
            let picked = self.selected_facts(query);
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
        // SocialBrain normalizes this at the authority boundary; clamp again
        // here because prompt rendering must remain safe even for manually
        // constructed or legacy contexts.
        let reputation = if self.reputation.is_finite() {
            self.reputation.clamp(0.0, 1.0)
        } else {
            DEFAULT_REPUTATION
        };
        out.push_str(&format!(
            "User standing: {:.2} on a 0.00-1.00 scale where {DEFAULT_REPUTATION:.2} is neutral \
             (higher reflects a stronger recent interaction-quality signal, not tenure or \
             authority). Use this ambient score only to tune response tone. Do not volunteer or \
             infer it to the user. Report standing only when the user explicitly asks and an \
             offered lookup_reputation tool returns an authorized result. Standing never changes \
             safety, authorization, privacy, factual grounding, or tool policy.",
            reputation
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
    /// Keyed by scoped guild + U+001F + scoped user. Reads retain compatibility
    /// with the historical ambiguous colon join.
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
    format!("{guild}{USER_KEY_SEPARATOR}{user}")
}

fn legacy_user_key(guild: &str, user: &str) -> String {
    format!("{guild}:{user}")
}

fn split_user_key(key: &str) -> Option<(&str, &str)> {
    if let Some((guild, user)) = key.split_once(USER_KEY_SEPARATOR) {
        return (!guild.is_empty() && !user.is_empty()).then_some((guild, user));
    }

    // Historical keys join two same-network scoped ids with `:`. Find the
    // second scoped id from the right so DM guilds (`network:dm:user`) decode
    // correctly too. Native ids on the supported networks do not contain a
    // repeated `:{network}:` marker.
    let network = key.split_once(':')?.0;
    let marker = format!(":{network}:");
    let split = key.rfind(&marker)?;
    let guild = &key[..split];
    let user = &key[split + 1..];
    (!guild.is_empty() && !user.is_empty()).then_some((guild, user))
}

/// One owned fact row for rebuilding the WDBX projection without decoding
/// `MemoryBank`'s serialized map keys outside this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryFact {
    pub guild: String,
    pub user: String,
    pub text: String,
    pub at: u64,
}

impl MemoryBank {
    pub fn user(&self, guild: &str, user: &str) -> Option<&UserMemory> {
        self.users
            .get(&user_key(guild, user))
            .or_else(|| self.users.get(&legacy_user_key(guild, user)))
    }

    /// The user's memory, provisioning a default one on first sight.
    pub fn user_mut(&mut self, guild: &str, user: &str) -> &mut UserMemory {
        let key = user_key(guild, user);
        if !self.users.contains_key(&key)
            && let Some(legacy) = self.users.remove(&legacy_user_key(guild, user))
        {
            self.users.insert(key.clone(), legacy);
        }
        self.users.entry(key).or_default()
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
        let key = user_key(guild, user);
        let legacy = legacy_user_key(guild, user);
        let selected = if self.users.contains_key(&key) {
            key
        } else {
            legacy
        };
        let Some(memory) = self.users.get_mut(&selected) else {
            return false;
        };
        let before = memory.facts.len();
        memory.facts.retain(|f| f != fact);
        memory.facts.len() != before
    }

    pub fn facts(&self, guild: &str, user: &str) -> &[String] {
        self.user(guild, user).map_or(&[], |m| m.facts.as_slice())
    }

    /// Every durable fact with its decoded scope, in stable map/fact order.
    /// Undecodable historical keys are left untouched in the JSON document but
    /// cannot be projected; all keys written by this and prior supported builds
    /// are decodable.
    pub fn fact_records(&self) -> Vec<MemoryFact> {
        self.users
            .iter()
            .filter_map(|(key, memory)| {
                split_user_key(key).map(|(guild, user)| (guild, user, memory))
            })
            .flat_map(|(guild, user, memory)| {
                memory.facts.iter().map(move |text| MemoryFact {
                    guild: guild.to_string(),
                    user: user.to_string(),
                    text: text.clone(),
                    at: memory.updated_at,
                })
            })
            .collect()
    }

    /// Move every decodable historical colon-joined row to the unambiguous
    /// separator key. If both forms exist, the new row is authoritative and
    /// the stale legacy duplicate is discarded; otherwise a forgotten fact in
    /// the old row could reappear during WDBX reconciliation.
    pub fn migrate_legacy_user_keys(&mut self) {
        let legacy_keys: Vec<String> = self
            .users
            .keys()
            .filter(|key| !key.contains(USER_KEY_SEPARATOR))
            .cloned()
            .collect();
        for legacy_key in legacy_keys {
            let Some((guild, user)) = split_user_key(&legacy_key) else {
                continue;
            };
            let canonical_key = user_key(guild, user);
            let Some(legacy) = self.users.remove(&legacy_key) else {
                continue;
            };
            self.users.entry(canonical_key).or_insert(legacy);
        }
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
mod tests;

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
fn fact_records_decode_new_and_legacy_scoped_keys_including_dms() {
    let mut bank = MemoryBank::default();
    bank.remember("telegram:g", "telegram:42", "new key", 7);
    bank.users.insert(
        "discord:dm:123:discord:123".into(),
        UserMemory {
            facts: vec!["legacy dm".into()],
            updated_at: 8,
            ..UserMemory::default()
        },
    );
    assert_eq!(
        bank.fact_records(),
        [
            MemoryFact {
                guild: "discord:dm:123".into(),
                user: "discord:123".into(),
                text: "legacy dm".into(),
                at: 8,
            },
            MemoryFact {
                guild: "telegram:g".into(),
                user: "telegram:42".into(),
                text: "new key".into(),
                at: 7,
            },
        ]
    );
    assert_eq!(bank.facts("discord:dm:123", "discord:123"), ["legacy dm"]);
}

#[test]
fn key_migration_prefers_new_row_over_a_stale_legacy_duplicate() {
    let mut bank = MemoryBank::default();
    bank.remember("discord:g", "discord:u", "kept", 2);
    bank.users.insert(
        legacy_user_key("discord:g", "discord:u"),
        UserMemory {
            facts: vec!["kept".into(), "already forgotten".into()],
            updated_at: 1,
            ..UserMemory::default()
        },
    );
    bank.migrate_legacy_user_keys();
    assert_eq!(bank.facts("discord:g", "discord:u"), ["kept"]);
    assert_eq!(bank.users.len(), 1);
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
        "Recent channel context: talking about deploys\nKnown about this user: likes rust; runs a homelab\nUser standing: 0.50 on a 0.00-1.00 scale where 0.50 is neutral (higher reflects a stronger recent interaction-quality signal, not tenure or authority). Use this ambient score only to tune response tone. Do not volunteer or infer it to the user. Report standing only when the user explicitly asks and an offered lookup_reputation tool returns an authorized result. Standing never changes safety, authorization, privacy, factual grounding, or tool policy."
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
    let rendered = high.render("");
    assert!(rendered.contains("interaction-quality signal"));
    assert!(!rendered.contains("longer history"));
    assert!(rendered.contains("explicitly asks"));
    assert!(rendered.contains("lookup_reputation"));
    assert!(rendered.contains("never changes safety, authorization, privacy"));
}

#[test]
fn persona_context_normalizes_untrusted_scores_before_prompting() {
    for (score, rendered) in [
        (-1.0, "User standing: 0.00"),
        (2.0, "User standing: 1.00"),
        (f64::NAN, "User standing: 0.50"),
        (f64::INFINITY, "User standing: 0.50"),
    ] {
        let mut context = PersonaContext::empty();
        context.reputation = score;
        assert!(context.render("").starts_with(rendered), "{score:?}");
    }
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
fn relevance_selection_never_reaches_across_guilds() {
    // The privacy boundary. Facts are keyed "{guild}:{user}", so isolation
    // is a property of the key — but ranking is new machinery reading those
    // facts, and "the key happens to separate them" is not the same as a
    // test that fails if it ever stops. A highly relevant fact in another
    // guild must stay invisible no matter how well it matches.
    let mut bank = MemoryBank::default();
    bank.remember("guild-a", "u", "runs the kubernetes cluster", 1);
    bank.remember("guild-b", "u", "mundane unrelated detail", 1);

    let context = bank.context_for("guild-b", "u", "chan");
    let rendered = context.render("kubernetes cluster question");
    assert!(
        !rendered.contains("kubernetes"),
        "guild-a's fact leaked into guild-b's context: {rendered}"
    );
    assert!(rendered.contains("mundane unrelated detail"));

    // And the same user in the other guild still sees their own.
    let own = bank.context_for("guild-a", "u", "chan");
    assert!(own.render("kubernetes").contains("kubernetes"));
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

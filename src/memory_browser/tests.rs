use super::*;

#[test]
fn maximum_width_envelope_is_99_ascii_characters_and_navigation_keeps_identity_and_expiry() {
    assert_eq!(MAX_FACTS, 100);
    assert_eq!(MAX_FACT_CHARS, 300);
    assert_eq!(MAX_PAGE_INDEX, 24);
    let scope = MemoryScope::Guild(u64::MAX);
    let session = MemorySession::new(u64::MAX, u64::MAX, scope, u64::MAX - LIFETIME_SECS).unwrap();
    for number in 0..=MAX_PAGE_INDEX {
        let next = session.navigate(number).unwrap();
        let id = next.custom_id();
        assert!(id.is_ascii());
        assert!(id.len() <= 99);
        if number >= 10 {
            assert_eq!(id.len(), 99);
        }
        assert_eq!(next.owner, session.owner);
        assert_eq!(next.subject, session.subject);
        assert!(next.scope == session.scope);
        assert_eq!(next.expiry, u64::MAX);
        let parsed = validate(&id, u64::MAX, &scope, u64::MAX - 1).unwrap();
        assert!(parsed == next);
        assert_eq!(
            validate(&id, u64::MAX, &scope, u64::MAX).err(),
            Some(BrowserRejection::Expired)
        );
    }
    assert!(session.navigate(MAX_PAGE_INDEX + 1).is_none());
    assert!(session.navigate(u8::MAX).is_none());
}

#[test]
fn new_sessions_reject_zero_identities_invalid_dm_subjects_and_expiry_overflow() {
    for (owner, subject, scope) in [
        (0, 2, MemoryScope::Guild(3)),
        (1, 0, MemoryScope::Guild(3)),
        (1, 2, MemoryScope::Guild(0)),
        (1, 2, MemoryScope::BotDm),
        (0, 0, MemoryScope::BotDm),
    ] {
        assert!(MemorySession::new(owner, subject, scope, 100).is_none());
    }
    assert!(MemorySession::new(1, 1, MemoryScope::BotDm, u64::MAX - 899).is_none());
    let dm = MemorySession::new(1, 1, MemoryScope::BotDm, 100).unwrap();
    assert_eq!(dm.custom_id(), "abbey:mem:v1:1:1:d:1000:0");
    assert!(validate(&dm.custom_id(), 1, &MemoryScope::BotDm, 100).unwrap() == dm);
}

#[test]
fn parser_rejects_noncanonical_malformed_unknown_and_out_of_range_fields() {
    let scope = MemoryScope::Guild(3);
    for id in [
        "abbey:mem:v2:1:2:3:1000:0",
        "abbey:help:v1:1:2:3:1000:0",
        "Abbey:mem:v1:1:2:3:1000:0",
        "abbey:mem:v1:0:2:3:1000:0",
        "abbey:mem:v1:1:0:3:1000:0",
        "abbey:mem:v1:1:2:0:1000:0",
        "abbey:mem:v1:01:2:3:1000:0",
        "abbey:mem:v1:1:02:3:1000:0",
        "abbey:mem:v1:1:2:03:1000:0",
        "abbey:mem:v1:1:2:3:01000:0",
        "abbey:mem:v1:1:2:3:1000:00",
        "abbey:mem:v1:+1:2:3:1000:0",
        "abbey:mem:v1:1:-2:3:1000:0",
        "abbey:mem:v1:1:2:+3:1000:0",
        "abbey:mem:v1:1:2:3:+1000:0",
        "abbey:mem:v1:1:2:3:1000:+0",
        "abbey:mem:v1:1:2:3:1000:25",
        "abbey:mem:v1:1:2:3:1000:256",
        "abbey:mem:v1:1:2:3:1000:",
        "abbey:mem:v1:1:2:3:1000",
        "abbey:mem:v1:1:2:3:1000:0:extra",
        "abbey:mem:v1:1:2:3:1000:0:",
        "abbey:mem:v1:1:2:3:1000:🦀",
        "abbey:mem:v1:１:2:3:1000:0",
        "abbey:mem:v1:1:2:D:1000:0",
        "abbey:mem:v1:1:2:d:1000:0",
        "abbey:mem:v1:18446744073709551616:2:3:1000:0",
        "abbey:mem:v1:1:18446744073709551616:3:1000:0",
        "abbey:mem:v1:1:2:18446744073709551616:1000:0",
        "abbey:mem:v1:1:2:3:18446744073709551616:0",
        "abbey:mem:v1:1:2:3:1000:18446744073709551616",
    ] {
        assert_eq!(
            validate(id, 1, &scope, 100).err(),
            Some(BrowserRejection::Stale),
            "{id}"
        );
    }
    assert_eq!(
        validate(&"a".repeat(101), 1, &scope, 100).err(),
        Some(BrowserRejection::Stale)
    );
    for (index, _) in "abbey:mem:v1:1:2:3:1000:0".split(':').enumerate() {
        let mut fields: Vec<_> = "abbey:mem:v1:1:2:3:1000:0".split(':').collect();
        fields[index] = "";
        assert_eq!(
            validate(&fields.join(":"), 1, &scope, 100).err(),
            Some(BrowserRejection::Stale)
        );
    }
}

#[test]
fn parser_enforces_actor_scope_and_exact_expiry_boundaries() {
    let guild = MemorySession::new(1, 2, MemoryScope::Guild(3), 100).unwrap();
    let id = guild.custom_id();
    assert_eq!(
        validate(&id, 2, &MemoryScope::Guild(3), 100).err(),
        Some(BrowserRejection::NotOwner)
    );
    assert_eq!(
        validate(&id, 1, &MemoryScope::Guild(4), 100).err(),
        Some(BrowserRejection::WrongScope)
    );
    assert_eq!(
        validate(&id, 1, &MemoryScope::BotDm, 100).err(),
        Some(BrowserRejection::WrongScope)
    );
    assert_eq!(
        validate(&id, 1, &MemoryScope::Guild(3), 99).err(),
        Some(BrowserRejection::Stale)
    );
    assert!(validate(&id, 1, &MemoryScope::Guild(3), 100).is_ok());
    assert!(validate(&id, 1, &MemoryScope::Guild(3), 999).is_ok());
    assert_eq!(
        validate(&id, 1, &MemoryScope::Guild(3), 1000).err(),
        Some(BrowserRejection::Expired)
    );
    assert_eq!(
        validate(&id, 1, &MemoryScope::Guild(3), 1001).err(),
        Some(BrowserRejection::Expired)
    );
    let dm = MemorySession::new(1, 1, MemoryScope::BotDm, 100).unwrap();
    assert_eq!(
        validate(&dm.custom_id(), 1, &MemoryScope::Guild(3), 100).err(),
        Some(BrowserRejection::WrongScope)
    );
    assert_eq!(
        validate(&dm.custom_id(), 2, &MemoryScope::BotDm, 100).err(),
        Some(BrowserRejection::NotOwner)
    );
}

#[test]
fn every_supported_fact_is_reachable_complete_and_rendered_under_the_message_limit() {
    let facts: Vec<String> = (0..MAX_FACTS)
        .map(|number| format!("{number:03}:{}", "🦀".repeat(MAX_FACT_CHARS - 4)))
        .collect();
    let before = facts.clone();
    let mut seen = Vec::new();
    for index in 0..=MAX_PAGE_INDEX {
        let current = page(&facts, index);
        assert_eq!(current.validity, Ok(()));
        assert_eq!(current.index, index);
        assert_eq!(current.total_pages, 25);
        assert_eq!(current.total_facts, MAX_FACTS);
        assert_eq!(current.facts.len(), FACTS_PER_PAGE);
        seen.extend(current.facts.iter());
        let rendered = render(u64::MAX, &current);
        for fact in current.facts {
            assert!(rendered.contains(fact));
        }
        assert!(rendered.contains(&format!("Page {} of 25 · 100 facts", index + 1)));
        assert!(rendered.chars().count() <= 2000);
        assert!(!rendered.contains('…'));
    }
    assert_eq!(seen, facts.iter().collect::<Vec<_>>());
    assert_eq!(facts, before);
    assert_eq!(page(&facts, u8::MAX).index, MAX_PAGE_INDEX);
}

#[test]
fn empty_and_shrinking_snapshots_have_honest_current_page_counts() {
    let facts: Vec<_> = (0..100).map(|n| format!("fact {n}")).collect();
    let shortened = page(&facts[..5], 24);
    assert_eq!(shortened.index, 1);
    assert_eq!(shortened.total_pages, 2);
    assert_eq!(shortened.total_facts, 5);
    assert_eq!(shortened.facts, &facts[4..5]);
    assert!(render(1, &shortened).contains("5. fact 4"));
    for requested in [0, 1, 24, u8::MAX] {
        let empty = page(&[], requested);
        assert_eq!(empty.validity, Ok(()));
        assert_eq!(empty.index, 0);
        assert_eq!(empty.total_pages, 1);
        assert_eq!(empty.total_facts, 0);
        assert!(empty.facts.is_empty());
        let text = render(1, &empty);
        assert!(text.contains("Page 1 of 1 · 0 facts"));
        assert!(text.contains("No facts on record."));
    }
    for count in 1..=MAX_FACTS {
        let last = page(&facts[..count], MAX_PAGE_INDEX);
        assert_eq!(
            usize::from(last.total_pages),
            count.div_ceil(FACTS_PER_PAGE)
        );
        assert_eq!(usize::from(last.index), (count - 1) / FACTS_PER_PAGE);
        assert_eq!(last.facts.last(), facts[..count].last());
    }
}

#[test]
fn invalid_legacy_snapshots_never_claim_a_successful_full_or_partial_page() {
    let too_many = vec!["legacy-count-canary".to_string(); MAX_FACTS + 1];
    let mut too_long = vec!["ordinary fact".to_string(); MAX_FACTS];
    too_long[MAX_FACTS - 1] = "🦀".repeat(MAX_FACT_CHARS + 1);
    for (facts, reason) in [
        (too_many, SnapshotRejection::TooManyFacts),
        (too_long, SnapshotRejection::FactTooLong),
    ] {
        let before = facts.clone();
        assert_eq!(validate_snapshot(&facts), Err(reason));
        for requested in [0, MAX_PAGE_INDEX] {
            let invalid = page(&facts, requested);
            assert_eq!(invalid.validity, Err(reason));
            assert!(invalid.facts.is_empty());
            let rendered = render(u64::MAX, &invalid);
            assert_eq!(rendered, UNAVAILABLE);
            assert!(!rendered.contains("Page "));
            assert!(!rendered.contains("ordinary fact"));
            assert!(!rendered.contains("canary"));
            assert!(rendered.chars().count() <= 2000);
        }
        assert_eq!(facts, before);
    }
    assert_eq!(validate_snapshot(&["🦀".repeat(MAX_FACT_CHARS)]), Ok(()));
}

#[test]
fn recovery_messages_are_fixed_bounded_and_actionable() {
    for rejection in [
        BrowserRejection::Stale,
        BrowserRejection::NotOwner,
        BrowserRejection::WrongScope,
        BrowserRejection::Expired,
    ] {
        let message = rejection.message();
        assert!(message.contains("`/recall`"));
        assert!(message.chars().count() < 2000);
        assert!(!message.contains("abbey:mem:") && !message.contains("<@"));
    }
}

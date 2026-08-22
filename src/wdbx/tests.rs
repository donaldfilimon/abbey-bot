use super::*;

const SAMPLE: &str = "# ABI-WDBX v1\n\
{\"type\":\"vector\",\"id\":1,\"values\":[0.5,-0.25,0.125]}\n\
{\"type\":\"kv\",\"key\":\"completion:1\",\"value\":\"{\\\"kind\\\":\\\"completion\\\"}\"}\n\
{\"type\":\"block\",\"hash\":\"abc\",\"prev_hash\":\"0\",\"sequence\":0}\n";

#[test]
fn parse_then_render_round_trips_including_unknown_records() {
    let store = WdbxStore::parse(SAMPLE).expect("parses");
    assert_eq!(store.vector(1), Some(&[0.5, -0.25, 0.125][..]));
    assert_eq!(
        store.get_kv("completion:1"),
        Some("{\"kind\":\"completion\"}")
    );
    let rendered = store.render();
    assert_eq!(
        rendered, SAMPLE,
        "render must reproduce the sample byte for byte"
    );
    // And unknown lines survive a second pass too.
    let again = WdbxStore::parse(&rendered).expect("re-parses");
    assert_eq!(again, store);
}

#[test]
fn vectors_round_trip_exactly_through_json() {
    let mut store = WdbxStore::new();
    let v = text_embedding("round trip").to_vec();
    let id = store.insert_vector(v.clone());
    let back = WdbxStore::parse(&store.render()).expect("parses");
    let got = back.vector(id).expect("vector present");
    assert!(got.iter().zip(&v).all(|(a, b)| a.to_bits() == b.to_bits()));
}

#[test]
fn header_is_required() {
    assert!(matches!(
        WdbxStore::parse(""),
        Err(WdbxError::MissingHeader { .. })
    ));
    let err = WdbxStore::parse("{\"type\":\"kv\",\"key\":\"a\",\"value\":\"b\"}\n")
        .expect_err("no header");
    assert!(err.to_string().contains("ABI-WDBX v1"), "{err}");
    // A header-only file is a valid empty store; CRLF is tolerated.
    assert_eq!(
        WdbxStore::parse("# ABI-WDBX v1\r\n").expect("parses"),
        WdbxStore::new()
    );
}

#[test]
fn checksum_trailer_is_dropped_and_comments_preserved() {
    let text = "# ABI-WDBX v1\n# note\n# checksum:deadbeef\n";
    let store = WdbxStore::parse(text).expect("parses");
    assert_eq!(store.render(), "# ABI-WDBX v1\n# note\n");
}

#[test]
fn malformed_lines_are_errors_not_panics() {
    let cases = [
        "# ABI-WDBX v1\nnot json\n",
        "# ABI-WDBX v1\n[1,2,3]\n",
        "# ABI-WDBX v1\n{\"key\":\"no type\"}\n",
        "# ABI-WDBX v1\n{\"type\":\"kv\",\"key\":\"a\"}\n",
        "# ABI-WDBX v1\n{\"type\":\"kv\",\"key\":\"a\",\"value\":7}\n",
        "# ABI-WDBX v1\n{\"type\":\"vector\",\"id\":-1,\"values\":[]}\n",
        "# ABI-WDBX v1\n{\"type\":\"vector\",\"id\":1,\"values\":[\"x\"]}\n",
        "# ABI-WDBX v1\n{\"type\":\"vector\",\"id\":1,\"values\":[]}\n{\"type\":\"vector\",\"id\":1,\"values\":[]}\n",
    ];
    for text in cases {
        let err = WdbxStore::parse(text).expect_err(text);
        assert!(
            matches!(err, WdbxError::MalformedLine { line: 2 | 3, .. }),
            "{text}: {err}"
        );
    }
}

#[test]
fn vector_ids_are_monotonic_even_after_removal_and_reload() {
    let mut store = WdbxStore::new();
    let a = store.insert_vector(vec![1.0]);
    let b = store.insert_vector(vec![1.0]);
    assert!(b > a);
    assert!(store.remove_vector(b));
    let c = store.insert_vector(vec![1.0]);
    assert!(c > b, "a removed id is never reissued");
    let reloaded = WdbxStore::parse(&store.render()).expect("parses");
    let mut reloaded = reloaded;
    assert!(reloaded.insert_vector(vec![1.0]) > c);
}

#[test]
fn search_ranks_identical_text_first_and_honours_filter() {
    let mut store = WdbxStore::new();
    let target = store.insert_vector(text_embedding("abbey likes tea").to_vec());
    let other = store.insert_vector(text_embedding("the server restarts nightly").to_vec());
    let query = text_embedding("abbey likes tea");
    let hits = store.search(&query, 5, |_| true);
    assert_eq!(hits[0].0, target);
    assert!((hits[0].1 - 1.0).abs() < 1e-5);
    assert_eq!(hits.len(), 2);
    let filtered = store.search(&query, 5, |id| id == other);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].0, other);
    assert!(store.search(&query, 0, |_| true).is_empty());
}

#[test]
fn recall_never_crosses_guilds() {
    let mut memory = Recall::new();
    let a_id = memory.remember("guild-a", "user-1", "abbey prefers earl grey", 10);
    memory.remember("guild-b", "user-1", "abbey prefers coffee", 11);

    let in_a = memory.recall_for_guild_admin("guild-a", "abbey prefers earl grey", 10);
    assert_eq!(in_a.len(), 1);
    assert_eq!(in_a[0].id, a_id);
    assert_eq!(in_a[0].text, "abbey prefers earl grey");
    assert_eq!(in_a[0].user, "user-1");
    assert_eq!(in_a[0].at, 10);

    let in_b = memory.recall_for_guild_admin("guild-b", "abbey prefers earl grey", 10);
    assert_eq!(in_b.len(), 1, "guild B sees only its own fact");
    assert_eq!(in_b[0].text, "abbey prefers coffee");

    assert!(
        memory
            .recall_for_guild_admin("guild-c", "abbey", 10)
            .is_empty()
    );
    assert_eq!(memory.count("guild-a"), 1);
    assert_eq!(memory.count("guild-b"), 1);
    assert_eq!(memory.count("guild-c"), 0);
    // A guild id that is a prefix of another must not match it.
    assert_eq!(memory.count("guild"), 0);
}

#[test]
fn recall_ranks_the_closest_fact_first() {
    let mut memory = Recall::new();
    memory.remember("g", "u", "the weekly raid is on thursday", 1);
    let best = memory.remember("g", "u", "donald's favourite editor is helix", 2);
    memory.remember("g", "u", "welcome channel is #lobby", 3);
    let hits = memory.recall_for_guild_admin("g", "favourite editor", 2);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, best);
    assert!(hits[0].score >= hits[1].score);
}

#[test]
fn person_scoped_recall_never_crosses_users_in_one_guild() {
    let mut memory = Recall::new();
    let alice = memory.remember("g", "alice", "alice's launch code is violet", 1);
    memory.remember("g", "bob", "bob's launch code is orange", 2);

    let hits = memory.recall_for_user("g", "alice", "launch code", 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, alice);
    assert_eq!(hits[0].user, "alice");
    assert_eq!(hits[0].text, "alice's launch code is violet");
    assert!(
        memory
            .recall_for_user("g", "carol", "launch code", 10)
            .is_empty()
    );
}

#[test]
fn numeric_fact_ids_remain_searchable_past_lexicographic_order() {
    let mut memory = Recall::new();
    for id in 1..=12 {
        memory.remember("g", "u", &format!("fact number {id}"), id);
    }
    let hits = memory.recall_for_user("g", "u", "fact number 2", 20);
    assert_eq!(hits.len(), 12, "all numeric ids pass the guild/user filter");
}

#[test]
fn forget_removes_only_within_its_guild() {
    let mut memory = Recall::new();
    let id = memory.remember("g1", "u", "fact one", 1);
    assert!(!memory.forget("g2", id), "another guild cannot forget it");
    assert_eq!(memory.count("g1"), 1);
    assert!(memory.forget("g1", id));
    assert!(!memory.forget("g1", id), "second forget reports absence");
    assert_eq!(memory.count("g1"), 0);
    assert!(
        memory
            .recall_for_guild_admin("g1", "fact one", 5)
            .is_empty()
    );
    assert_eq!(
        memory.store().vector_count(),
        0,
        "vector goes with the fact"
    );
}

#[test]
fn facts_for_user_lists_that_user_only_oldest_first() {
    let mut memory = Recall::new();
    let first = memory.remember("g", "alice", "alice likes cats", 1);
    memory.remember("g", "bob", "bob likes dogs", 2);
    let second = memory.remember("g", "alice", "alice plays bass", 3);
    memory.remember("other", "alice", "alice elsewhere", 4);
    let facts = memory.facts_for_user("g", "alice");
    assert_eq!(
        facts.iter().map(|f| f.id).collect::<Vec<_>>(),
        vec![first, second]
    );
    assert!(facts.iter().all(|f| f.score == 1.0));
    assert!(memory.facts_for_user("g", "carol").is_empty());
}

#[test]
fn fact_kv_is_double_encoded_json_under_the_scoped_key() {
    let mut memory = Recall::new();
    let id = memory.remember("123:456", "u", "hello", 99);
    let raw = memory
        .store()
        .get_kv(&format!("mem:123:456:{id}"))
        .expect("kv under scoped key");
    let parsed: serde_json::Value = serde_json::from_str(raw).expect("value is JSON");
    assert_eq!(parsed["user"], "u");
    assert_eq!(parsed["text"], "hello");
    assert_eq!(parsed["at"], 99);
}

#[test]
fn projection_reconcile_replaces_only_memory_records() {
    let mut memory = Recall::new();
    let stale_id = memory.remember("g", "u", "stale fact", 1);
    let unrelated_id = memory.store.insert_vector(vec![0.25, 0.5]);
    memory.store.put_kv("completion:1", "kept");
    memory
        .store
        .unknown
        .push(r#"{"type":"block","hash":"abc","prev_hash":"0","sequence":0}"#.into());

    memory.reconcile_memory_facts([
        ("g".into(), "u".into(), "canonical fact".into(), 2),
        ("other".into(), "v".into(), "second fact".into(), 3),
    ]);
    assert_eq!(
        memory
            .all_memory_facts()
            .into_iter()
            .map(|(guild, fact)| (guild, fact.user, fact.text, fact.at))
            .collect::<Vec<_>>(),
        [
            ("g".into(), "u".into(), "canonical fact".into(), 2),
            ("other".into(), "v".into(), "second fact".into(), 3),
        ]
    );
    assert!(memory.store.vector(stale_id).is_none());
    assert_eq!(memory.store.vector(unrelated_id), Some(&[0.25, 0.5][..]));
    assert_eq!(memory.store.get_kv("completion:1"), Some("kept"));
    assert!(memory.store.render().contains(r#"{"type":"block""#));

    let stable = memory.clone();
    memory.reconcile_memory_facts([
        ("g".into(), "u".into(), "canonical fact".into(), 2),
        ("other".into(), "v".into(), "second fact".into(), 3),
    ]);
    assert_eq!(memory, stable, "an identical projection does not churn ids");
}

#[test]
fn load_and_save_round_trip_in_a_temp_dir() {
    let dir = std::env::temp_dir().join(format!(
        "abbey-bot-wdbx-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("memory.wdbx");

    let loaded_missing = Recall::load(&path).expect("missing file is empty memory");
    assert_eq!(loaded_missing.count("g"), 0);

    let mut memory = Recall::new();
    let id = memory.remember("g", "u", "persisted fact", 42);
    memory.save(&path).expect("saves");
    assert!(
        !dir.join("memory.wdbx.tmp").exists(),
        "temp file is renamed away"
    );

    let text = fs::read_to_string(&path).expect("readable");
    assert!(text.starts_with("# ABI-WDBX v1\n"));

    let back = Recall::load(&path).expect("loads");
    assert_eq!(back, memory);
    let hits = back.recall_for_guild_admin("g", "persisted fact", 1);
    assert_eq!(hits[0].id, id);
    assert_eq!(hits[0].at, 42);

    let err = WdbxStore::load(&dir.join("nope.wdbx")).expect_err("missing file");
    assert!(matches!(err, WdbxError::Io { op: "read", .. }), "{err}");

    fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn error_display_is_one_sentence_each() {
    let missing = WdbxError::MissingHeader {
        found: "garbage".into(),
    };
    assert_eq!(
        missing.to_string(),
        "WDBX store header is `garbage`, expected `# ABI-WDBX v1`"
    );
    let bad = WdbxError::MalformedLine {
        line: 4,
        reason: "not JSON".into(),
    };
    assert_eq!(bad.to_string(), "WDBX store line 4 is malformed: not JSON");
}

/// Cross-implementation conformance: the exact bytes this module writes are
/// pinned here, and `wdbx`'s `abi-wdbx` parses the identical file in its own
/// suite (`crates/abi-wdbx/tests/abbey_bot_projection_conformance.rs`).
///
/// Why a shared fixture instead of a dependency: this crate pins **stable
/// 1.97.1** (`rust-toolchain.toml`), while `abi-compute` — which `abi-wdbx`
/// depends on — requires `#![feature(portable_simd)]` on nightly. Linking
/// `abi-wdbx` here, even as a dev-dependency, is not possible without reversing
/// this crate's pinned-stable decision. The same shape is already used for the
/// Zig wyhash reference vectors in `src/wyhash.rs`.
///
/// The fixture's trailing `block` record is a **real one, copied from abi's own
/// `wdbx-sample.seg.jsonl`**, with a 32-byte array hash. That detail matters:
/// the `SAMPLE` constant at the top of this file uses `"hash":"abc"`, which
/// `abi-wdbx` rejects with `expected 32 characters, got 3`. So the older
/// round-trip test above exercises a block abi could never have written, and
/// only this fixture proves the projection preserves a real one.
///
/// If this fails you changed the writer. Regenerate, then copy the file to
/// `wdbx/crates/abi-wdbx/tests/golden/abbey-bot-projection.seg.jsonl`. The two
/// copies are a deliberate, documented residual: no single toolchain compiles
/// both crates, so neither side can generate the other's copy.
#[test]
fn writer_output_matches_the_cross_implementation_conformance_fixture() {
    let mut store = WdbxStore::new();
    store.put_kv(
        "mem:guild-1:1",
        "{\"user\":\"alice\",\"text\":\"likes rust\",\"at\":1000}",
    );
    store.put_kv("completion:1", "{\"kind\":\"completion\"}");
    assert_eq!(store.insert_vector(vec![0.5, -0.25, 0.125]), 1);
    assert_eq!(store.insert_vector(vec![-1.0, 0.0, 1.0]), 2);

    let fixture = include_str!("../../tests/fixtures/wdbx_v1_conformance.seg.jsonl");
    let without_block: String = fixture
        .lines()
        .filter(|line| !line.contains("\"type\":\"block\""))
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(
        store.render(),
        without_block,
        "writer output drifted from the cross-implementation conformance fixture"
    );
}

/// A record kind this projection does not model must survive verbatim, because
/// abi owns the format and may add kinds this side has never heard of. Losing
/// one would silently corrupt a shared file.
#[test]
fn the_conformance_fixture_round_trips_byte_identically() {
    let fixture = include_str!("../../tests/fixtures/wdbx_v1_conformance.seg.jsonl");
    let parsed = WdbxStore::parse(fixture).expect("fixture parses");
    assert_eq!(
        parsed.render(),
        fixture,
        "a real abi block record was not preserved verbatim"
    );
    assert_eq!(parsed.vector(1), Some(&[0.5, -0.25, 0.125][..]));
    assert_eq!(parsed.vector(2), Some(&[-1.0, 0.0, 1.0][..]));
    assert_eq!(
        parsed.get_kv("mem:guild-1:1"),
        Some("{\"user\":\"alice\",\"text\":\"likes rust\",\"at\":1000}")
    );
}

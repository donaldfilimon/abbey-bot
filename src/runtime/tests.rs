use super::*;
use crate::brain::social::ReputationStore;
use crate::guild::GuildSettings;
use crate::persist::{PersistComponentOutcome, PersistErrorCategory, PersistOverall};

#[test]
fn dqn_round_trips_through_the_brain_trait() {
    let mut a = fresh_brain();
    let json = Brain::export_json(&a);
    assert!(json.contains("\"topology\""));
    assert!(Brain::import_json(&mut a, &json));
    assert!(!Brain::import_json(&mut a, "{not json"));
    assert!(
        !Brain::import_json(
            &mut a,
            "{\"topology\":[1,2],\"layers\":[],\"epsilon\":0.1,\"step_count\":0}"
        ),
        "topology drift is rejected, not silently accepted"
    );
}

#[test]
fn process_persistence_reports_memory_only_without_calling_a_sink() {
    let sink = crate::persist::tests::RuntimeRecordingSink::success();
    let state = AppState::in_memory_with_persistence(None, Arc::new(sink.clone()));
    let report = state.persist_all_at(42);
    assert_eq!(report.overall, PersistOverall::MemoryOnly);
    assert_eq!(
        report.canonical_state,
        PersistComponentOutcome::NotConfigured
    );
    assert_eq!(
        report.wdbx_projection,
        PersistComponentOutcome::NotConfigured
    );
    assert!(sink.attempts().is_empty());
}

#[test]
fn canonical_failure_skips_the_wdbx_projection() {
    let sink = crate::persist::tests::RuntimeRecordingSink::fail_canonical(
        PersistErrorCategory::SyncTemporary,
    );
    let state = AppState::in_memory_with_persistence(
        Some(PathBuf::from("/injected/state")),
        Arc::new(sink.clone()),
    );
    let report = state.persist_all_at(42);
    assert_eq!(report.overall, PersistOverall::Failed);
    assert_eq!(
        report.canonical_state,
        PersistComponentOutcome::Failed(PersistErrorCategory::SyncTemporary)
    );
    assert_eq!(
        report.wdbx_projection,
        PersistComponentOutcome::SkippedCanonicalFailure
    );
    assert_eq!(sink.attempts(), ["canonical"]);
}

#[test]
fn both_durable_components_committing_is_complete() {
    let sink = crate::persist::tests::RuntimeRecordingSink::success();
    let state = AppState::in_memory_with_persistence(
        Some(PathBuf::from("/injected/state")),
        Arc::new(sink.clone()),
    );
    let report = state.persist_all_at(42);
    assert_eq!(report.overall, PersistOverall::Complete);
    assert_eq!(report.canonical_state, PersistComponentOutcome::Committed);
    assert_eq!(report.wdbx_projection, PersistComponentOutcome::Committed);
    assert_eq!(sink.attempts(), ["canonical", "wdbx"]);
}

#[test]
fn projection_failure_is_partial_after_a_canonical_commit() {
    let sink = crate::persist::tests::RuntimeRecordingSink::fail_projection(
        PersistErrorCategory::SyncDirectory,
    );
    let state = AppState::in_memory_with_persistence(
        Some(PathBuf::from("/injected/state")),
        Arc::new(sink.clone()),
    );
    let report = state.persist_all_at(42);
    assert_eq!(report.overall, PersistOverall::Partial);
    assert_eq!(report.canonical_state, PersistComponentOutcome::Committed);
    assert_eq!(
        report.wdbx_projection,
        PersistComponentOutcome::Failed(PersistErrorCategory::SyncDirectory)
    );
    assert_eq!(sink.attempts(), ["canonical", "wdbx"]);
}

#[test]
fn queue_and_concurrency_parse_with_fallbacks() {
    assert_eq!(queue_secs_from_value(None), DEFAULT_QUEUE_SECS);
    assert_eq!(queue_secs_from_value(Some("30".into())), 30);
    assert_eq!(queue_secs_from_value(Some("0".into())), DEFAULT_QUEUE_SECS);
}

#[test]
fn voice_queue_never_shorter_than_text_and_has_a_floor() {
    assert_eq!(voice_queue_secs(1), DEFAULT_VOICE_QUEUE_SECS);
    assert_eq!(
        voice_queue_secs(DEFAULT_QUEUE_SECS),
        DEFAULT_VOICE_QUEUE_SECS
    );
    assert_eq!(
        voice_queue_secs(DEFAULT_VOICE_QUEUE_SECS),
        DEFAULT_VOICE_QUEUE_SECS
    );
    assert_eq!(voice_queue_secs(240), 240);
}

#[test]
fn vision_transport_routes_every_loopback_shape_to_the_no_proxy_client() {
    let transport = HttpVisionTransport::default();
    for endpoint in [
        "http://127.0.0.1:11434/v1/chat/completions",
        "http://localhost:8080/v1/chat/completions",
        "http://[::1]:8181/v1/chat/completions",
    ] {
        assert!(std::ptr::eq(
            transport.client_for(endpoint),
            &transport.loopback_client
        ));
    }
    assert!(std::ptr::eq(
        transport.client_for("https://vision.example.com/v1/chat/completions"),
        &transport.remote_client
    ));
}

#[tokio::test]
async fn rolling_summaries_do_nothing_without_a_backend_and_keep_channels_due() {
    let state = AppState::in_memory();
    {
        let mut stores = AppState::lock(&state.stores);
        for i in 0..30 {
            stores
                .memory
                .record_message("discord:c", "a", &format!("m{i}"), i);
        }
        stores.memory.channel_mut("discord:c").guild = Some("discord:g".into());
    }
    assert_eq!(state.refresh_summaries().await, 0);
    assert_eq!(
        AppState::lock(&state.stores)
            .memory
            .channels_due_for_summary(),
        ["discord:c"],
        "still due — nothing consumed the marker"
    );
}

#[test]
fn hour_of_day_wraps_at_24() {
    assert_eq!(hour_of_day(0), 0);
    assert_eq!(hour_of_day(3600 * 25), 1);
    assert_eq!(hour_of_day(3600 * 23 + 59), 23);
}

#[test]
fn topology_matches_the_spec() {
    assert_eq!(TOPOLOGY, [18, 64, 32, 3]);
}

#[test]
fn voice_inspect_is_exact_guild_only_and_dm_safe() {
    let state = AppState::in_memory();
    state
        .voice_inspect
        .publish("discord:g", crate::inspect::VoiceInspectState::Active);
    let mut scope = ToolScope {
        memory_turn: None,
        state: &state,
        network: SocialNetwork::Discord,
        scoped_guild: "discord:g".into(),
        scoped_user: "discord:u".into(),
        scoped_channel: "discord:c".into(),
        now: 10,
        persona: crate::persona::Persona::Abbey,
    };

    assert_eq!(
        crate::tools::ToolHost::inspect_status(&mut scope, crate::tools::InspectAspect::Voice),
        "voice: active"
    );
    scope.scoped_guild = "discord:other".into();
    assert_eq!(
        crate::tools::ToolHost::inspect_status(&mut scope, crate::tools::InspectAspect::Voice),
        "voice: off"
    );
    scope.scoped_guild = "discord:dm:u".into();
    assert_eq!(
        crate::tools::ToolHost::inspect_status(&mut scope, crate::tools::InspectAspect::Voice),
        "voice: off"
    );
}

#[test]
fn configured_but_ineligible_fm_routes_publish_no_capabilities() {
    let mut state = AppState::in_memory();
    Arc::get_mut(&mut state)
        .expect("unique state")
        .providers
        .set_fm(Some(FoundationModels::new(
            crate::provider::FmConfig {
                mode: crate::provider::FmMode::System,
                endpoint: Some("http://127.0.0.1:8899".into()),
                cli: PathBuf::from("/usr/bin/fm"),
                fallback: false,
                timeout_secs: 30,
            },
            None,
            true,
        )));
    let rendered = crate::inspect::render_provider(&state.provider_inspect());

    assert!(
        rendered.contains("foundation-models-server: routable no"),
        "{rendered}"
    );
    assert!(
        rendered.contains("foundation-models-cli: routable no"),
        "{rendered}"
    );
    assert_eq!(
        rendered
            .matches("text no · tools no · vision no · ocr no")
            .count(),
        2,
        "{rendered}"
    );
    assert!(!rendered.contains("127.0.0.1"), "{rendered}");
    assert!(!rendered.contains("/usr/bin"), "{rendered}");
}

#[test]
fn unknown_guild_inspect_is_non_provisioning() {
    let state = AppState::in_memory();
    let mut scope = ToolScope {
        memory_turn: None,
        state: &state,
        network: SocialNetwork::Discord,
        scoped_guild: "discord:missing".into(),
        scoped_user: "discord:u".into(),
        scoped_channel: "discord:c".into(),
        now: 10,
        persona: crate::persona::Persona::Abbey,
    };

    let rendered =
        crate::tools::ToolHost::inspect_status(&mut scope, crate::tools::InspectAspect::Guild);

    assert_eq!(rendered, "No guild settings on record.");
    assert!(AppState::lock(&state.stores).guilds.is_empty());
    assert!(!AppState::lock(&state.guilds).is_cached("discord:missing"));
}

#[test]
fn durable_guild_inspect_does_not_fill_the_cache() {
    let state = AppState::in_memory();
    AppState::lock(&state.stores).guilds.insert(
        "discord:g".into(),
        GuildSettings {
            default_persona: crate::persona::Persona::Abi,
            ..GuildSettings::default()
        },
    );
    let mut scope = ToolScope {
        memory_turn: None,
        state: &state,
        network: SocialNetwork::Discord,
        scoped_guild: "discord:g".into(),
        scoped_user: "discord:u".into(),
        scoped_channel: "discord:c".into(),
        now: 10,
        persona: crate::persona::Persona::Abbey,
    };

    let rendered =
        crate::tools::ToolHost::inspect_status(&mut scope, crate::tools::InspectAspect::Guild);

    assert!(rendered.contains("persona: abi"), "{rendered}");
    assert!(!AppState::lock(&state.guilds).is_cached("discord:g"));
}

#[test]
fn cached_guild_inspect_uses_the_recorded_settings_and_injected_time() {
    let state = AppState::in_memory();
    {
        let mut stores = AppState::lock(&state.stores);
        let mut guilds = AppState::lock(&state.guilds);
        guilds.update("discord:g", &mut *stores, |settings| {
            settings.default_persona = crate::persona::Persona::Aviva;
            settings.unsolicited_per_hour = 6;
        });
    }
    {
        let mut budget = AppState::lock(&state.budget);
        for _ in 0..6 {
            assert!(budget.try_take("discord:g", 6, 10_000));
        }
    }
    let mut scope = ToolScope {
        memory_turn: None,
        state: &state,
        network: SocialNetwork::Discord,
        scoped_guild: "discord:g".into(),
        scoped_user: "discord:u".into(),
        scoped_channel: "discord:c".into(),
        now: 10_600,
        persona: crate::persona::Persona::Abbey,
    };

    let rendered =
        crate::tools::ToolHost::inspect_status(&mut scope, crate::tools::InspectAspect::Guild);

    assert!(rendered.contains("persona: aviva"), "{rendered}");
    assert!(rendered.contains("(1.0 left)"), "{rendered}");
    assert!(AppState::lock(&state.guilds).is_cached("discord:g"));
}

#[test]
fn all_tool_memory_writes_use_the_scope_timestamp() {
    let state = AppState::in_memory();
    state
        .memory_service()
        .remember("discord:g", "discord:u", "uses rust", 1)
        .expect("seed");
    let mut scope = ToolScope {
        memory_turn: None,
        state: &state,
        network: SocialNetwork::Discord,
        scoped_guild: "discord:g".into(),
        scoped_user: "discord:u".into(),
        scoped_channel: "discord:c".into(),
        now: 4_242,
        persona: crate::persona::Persona::Abbey,
    };

    crate::tools::ToolHost::remember_fact(&mut scope, "moved to zig", Some("uses rust"));
    crate::tools::ToolHost::remember_fact(&mut scope, "likes compilers", None);

    let stores = AppState::lock(&state.stores);
    let memory = stores
        .memory
        .user("discord:g", "discord:u")
        .expect("subject memory");
    assert_eq!(memory.updated_at, 4_242);
    assert_eq!(memory.pending_supersessions.len(), 1);
    assert_eq!(memory.pending_supersessions[0].at, 4_242);
}

#[test]
fn list_facts_isolated_to_the_exact_canonical_subject() {
    let state = AppState::in_memory();
    let service = state.memory_service();
    service
        .remember("discord:g", "discord:u", "own fact", 1)
        .expect("own fact");
    service
        .remember_proposing("discord:g", "discord:u", "own replacement", "own fact", 2)
        .expect("own pending");
    service
        .remember("discord:g", "discord:other", "other user fact", 3)
        .expect("other user");
    service
        .remember("discord:other", "discord:u", "other guild fact", 4)
        .expect("other guild");
    let mut scope = ToolScope {
        memory_turn: None,
        state: &state,
        network: SocialNetwork::Discord,
        scoped_guild: "discord:g".into(),
        scoped_user: "discord:u".into(),
        scoped_channel: "discord:c".into(),
        now: 10,
        persona: crate::persona::Persona::Abbey,
    };

    let rendered = crate::tools::ToolHost::list_facts(&mut scope);

    assert!(rendered.contains("own fact"), "{rendered}");
    assert!(rendered.contains("own replacement"), "{rendered}");
    assert!(!rendered.contains("other user fact"), "{rendered}");
    assert!(!rendered.contains("other guild fact"), "{rendered}");
}

/// The safety property at its real integration point: a model calling
/// `remember_fact` with `supersedes` must PROPOSE, never delete. Verified
/// here through the actual `ToolHost` impl rather than by reading the
/// routing — `remember_proposing` being correct in isolation would not
/// prove `ToolScope` routes to it instead of `remember_replacing`.
#[test]
fn a_model_supersedes_argument_proposes_and_never_deletes() {
    let state = AppState::in_memory();
    state
        .memory_service()
        .remember("discord:g", "discord:u", "uses rust", 1)
        .expect("seed");
    let mut scope = ToolScope {
        memory_turn: None,
        state: &state,
        network: SocialNetwork::Discord,
        scoped_guild: "discord:g".into(),
        scoped_user: "discord:u".into(),
        scoped_channel: "discord:c".into(),
        now: 10,
        persona: crate::persona::Persona::Abbey,
    };

    let reply =
        crate::tools::ToolHost::remember_fact(&mut scope, "moved to zig", Some("uses rust"));
    assert!(reply.contains("Proposed to replace"), "{reply}");

    // BOTH facts must survive. The model does not get to delete.
    let facts = state.memory_service().facts("discord:g", "discord:u");
    assert!(facts.contains(&"uses rust".to_string()), "{facts:?}");
    assert!(facts.contains(&"moved to zig".to_string()), "{facts:?}");
    assert_eq!(
        state
            .memory_service()
            .pending_supersessions("discord:g", "discord:u")
            .len(),
        1
    );
}

#[test]
fn tool_memory_uses_the_shared_fact_validator() {
    let state = AppState::in_memory();
    let mut scope = ToolScope {
        memory_turn: None,
        state: &state,
        network: SocialNetwork::Discord,
        scoped_guild: "discord:g".into(),
        scoped_user: "discord:u".into(),
        scoped_channel: "discord:c".into(),
        now: 10,
        persona: crate::persona::Persona::Abbey,
    };
    assert_eq!(
        crate::tools::ToolHost::remember_fact(&mut scope, "  Donald\nlikes\tRust.  ", None),
        "Stored: Donald likes Rust."
    );
    assert_eq!(
        state.memory_service().facts("discord:g", "discord:u"),
        ["Donald likes Rust."]
    );
    assert_eq!(
        crate::tools::ToolHost::remember_fact(
            &mut scope,
            &"🦀".repeat(crate::memory::MAX_FACT_CHARS + 1),
            None
        ),
        "Keep one remembered fact to 300 characters or fewer."
    );
    assert_eq!(
        state.memory_service().facts("discord:g", "discord:u").len(),
        1
    );
}

#[test]
fn explicit_reputation_ids_are_scoped_to_the_conversation_network() {
    for (network, expected) in [
        (SocialNetwork::Discord, 0.61),
        (SocialNetwork::Telegram, 0.72),
        (SocialNetwork::Slack, 0.83),
    ] {
        let state = AppState::in_memory();
        let guild = format!("{}:g", network.as_str());
        let user = format!("{}:42", network.as_str());
        AppState::lock(&state.stores).store_reputation(&guild, &user, expected, 1);
        let mut scope = ToolScope {
            memory_turn: None,
            state: &state,
            network,
            scoped_guild: guild,
            scoped_user: format!("{}:self", network.as_str()),
            scoped_channel: format!("{}:c", network.as_str()),
            now: 10,
            persona: crate::persona::Persona::Abbey,
        };
        let native_id = if network == SocialNetwork::Discord {
            "<@42>"
        } else {
            "42"
        };
        assert_eq!(
            crate::tools::ToolHost::lookup_reputation(&mut scope, Some(native_id)),
            format!("Reputation {expected:.2} (0 = poor, 1 = excellent).")
        );
    }
}

#[test]
fn conflicting_scoped_reputation_id_cannot_escape_the_current_network() {
    let state = AppState::in_memory();
    AppState::lock(&state.stores).store_reputation("telegram:g", "discord:42", 0.99, 1);
    let mut scope = ToolScope {
        memory_turn: None,
        state: &state,
        network: SocialNetwork::Telegram,
        scoped_guild: "telegram:g".into(),
        scoped_user: "telegram:self".into(),
        scoped_channel: "telegram:c".into(),
        now: 10,
        persona: crate::persona::Persona::Abbey,
    };
    assert_eq!(
        crate::tools::ToolHost::lookup_reputation(&mut scope, Some("discord:42")),
        "Reputation 0.50 (0 = poor, 1 = excellent)."
    );
}

#[test]
fn settled_rewards_reach_the_guild_stats() {
    let state = AppState::in_memory();
    {
        let stores = AppState::lock(&state.stores);
        let mut brains = AppState::lock(&state.brains);
        brains.brain("discord:g", &*stores, 0);
    }
    AppState::lock(&state.rewards).register_reply(vec![0.0; 18], 1, "m1", "discord:g", 0);
    AppState::lock(&state.rewards).reaction("👍", "m1", true);
    // settle_rewards reads the real clock; the entry is 150 s+ old by any clock.
    state.settle_rewards();
    let brains = AppState::lock(&state.brains);
    let stats = brains.stats("discord:g").expect("loaded");
    assert_eq!(stats.settled_total, 1);
    assert!((stats.mean_recent_reward().unwrap() - 0.8).abs() < 1e-6);
    assert_eq!(brains.get("discord:g").unwrap().buffer_len(), 1);
    drop(brains);
    assert!(AppState::lock(&state.budget).try_take("discord:g", 6, 0));
}

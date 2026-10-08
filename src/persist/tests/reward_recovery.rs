use super::*;
use crate::brain::reward::{FeedbackAttribution, ReactionKey, ReplyTurn, RewardCollector};

fn collector() -> RewardCollector {
    let mut c = RewardCollector::new();
    c.register_turn(ReplyTurn {
        state: vec![0.0; 18],
        action: 1,
        sent_native_message_id: "m".into(),
        scope: "discord:c".into(),
        scoped_guild_id: "discord:g".into(),
        ask: "fixture question".into(),
        asker: "discord:u".into(),
        now: 100,
    });
    c
}

#[test]
fn canonical_pending_action_bounds_reject_tracked_and_legacy_rows_atomically() {
    let dir = temp_dir("pending-action-bounds");
    let valid = Stores {
        pending_rewards: collector().export_pending(),
        ..Default::default()
    };
    valid.save(&dir).unwrap();
    let original = fs::read(Stores::state_path(&dir)).unwrap();
    for tracked in [false, true] {
        for action in [3, usize::MAX] {
            let mut invalid = valid.clone();
            invalid.pending_rewards[0].1.action = action;
            invalid.pending_rewards[0].1.reaction_tracking = tracked;
            let mut rewards = RewardCollector::new();
            assert_eq!(
                rewards.restore_recovered(
                    invalid.pending_rewards.clone(),
                    invalid.reward_recovery.clone(),
                ),
                Err("invalid pending reward"),
                "action {action}, tracked {tracked} must fail before restore"
            );
            assert_eq!(rewards.pending_len(), 0);
            assert_eq!(rewards.export_recovery(), Default::default());
            assert!(invalid.save(&dir).is_err());
            assert_eq!(fs::read(Stores::state_path(&dir)).unwrap(), original);
            fs::write(
                Stores::state_path(&dir),
                serde_json::to_vec(&invalid).unwrap(),
            )
            .unwrap();
            assert!(Stores::load(&dir).is_err());
            fs::write(Stores::state_path(&dir), &original).unwrap();
        }
        for action in [0, 1, 2] {
            fs::write(Stores::state_path(&dir), &original).unwrap();
            let mut compatible = valid.clone();
            compatible.pending_rewards[0].1.action = action;
            compatible.pending_rewards[0].1.reaction_tracking = tracked;
            compatible.save(&dir).unwrap();
            let loaded = Stores::load(&dir).unwrap();
            let mut rewards = RewardCollector::new();
            rewards
                .restore_recovered(loaded.pending_rewards, loaded.reward_recovery)
                .unwrap();
            assert_eq!(rewards.settle_expired(251)[0].1.action, action);
        }
        fs::write(Stores::state_path(&dir), &original).unwrap();
    }
    fs::remove_dir_all(dir).unwrap();
}
fn key() -> ReactionKey {
    ReactionKey {
        scope: "discord:c".into(),
        message: "m".into(),
        reactor_hash: 7,
        emoji: "👍".into(),
    }
}
fn save_reload(c: &RewardCollector, tag: &str) -> RewardCollector {
    let dir = temp_dir(tag);
    let stores = Stores {
        pending_rewards: c.export_pending(),
        reward_recovery: c.export_recovery(),
        ..Default::default()
    };
    stores.save(&dir).unwrap();
    let loaded = Stores::load(&dir).unwrap();
    assert!(stores.payload_eq(&loaded));
    let mut restored = RewardCollector::new();
    restored
        .restore_recovered(loaded.pending_rewards, loaded.reward_recovery)
        .unwrap();
    fs::remove_dir_all(dir).unwrap();
    restored
}

#[test]
fn canonical_atomic_stores_recover_active_removed_and_settled_reactions() {
    let mut c = collector();
    c.reaction(key(), true, 100);
    let mut c = save_reload(&c, "reaction-active");
    assert_eq!(c.reaction(key(), true, 101), FeedbackAttribution::Duplicate);
    assert_eq!(
        c.reaction(key(), false, 101),
        FeedbackAttribution::ExactReply
    );
    let mut c = save_reload(&c, "reaction-removed");
    assert_eq!(
        c.reaction(key(), false, 102),
        FeedbackAttribution::Duplicate
    );
    assert!((c.settle_expired(251)[0].1.reward + 0.2).abs() < 1e-6);
    let mut c = save_reload(&c, "reaction-settled");
    assert_eq!(c.reaction(key(), true, 252), FeedbackAttribution::Expired);
    assert!(c.settle_expired(253).is_empty());
}

#[test]
fn stores_payload_equality_includes_settled_protection() {
    let mut c = collector();
    c.settle_expired(251);
    let with = Stores {
        reward_recovery: c.export_recovery(),
        ..Default::default()
    };
    assert!(!with.payload_eq(&Stores::default()));
    let old: Stores = serde_json::from_str("{}").unwrap();
    assert_eq!(old.reward_recovery, Default::default());
}

#[test]
fn canonical_load_and_save_refuse_inconsistent_recovery() {
    let mut c = collector();
    c.reaction(key(), true, 100);
    let invalid = Stores {
        pending_rewards: c.export_pending(),
        ..Default::default()
    };
    let dir = temp_dir("invalid-reward");
    assert!(invalid.save(&dir).is_err());
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        Stores::state_path(&dir),
        serde_json::to_vec(&invalid).unwrap(),
    )
    .unwrap();
    assert!(Stores::load(&dir).is_err());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn bounded_deserialization_rejects_oversized_and_invalid_reaction_state() {
    let mut c = collector();
    c.reaction(key(), true, 100);
    let stores = Stores {
        pending_rewards: c.export_pending(),
        reward_recovery: c.export_recovery(),
        ..Default::default()
    };
    let mut json = serde_json::to_value(&stores).unwrap();
    let row = json["reward_recovery"]["reactions"][0].clone();
    json["reward_recovery"]["reactions"] = serde_json::Value::Array(vec![row; 4097]);
    assert!(serde_json::from_value::<Stores>(json).is_err());
    let mut json = serde_json::to_value(&stores).unwrap();
    json["reward_recovery"]["reactions"][0]["contribution"] = serde_json::json!("invented");
    assert!(serde_json::from_value::<Stores>(json).is_err());
}

#[test]
fn pending_bound_and_recovery_duplicates_fail_closed() {
    let mut c = collector();
    c.reaction(key(), true, 100);
    let stores = Stores {
        pending_rewards: c.export_pending(),
        reward_recovery: c.export_recovery(),
        ..Default::default()
    };
    let mut json = serde_json::to_value(&stores).unwrap();
    let mut pending = json["pending_rewards"][0].clone();
    pending[1]["positive_reactions"] = serde_json::json!(0);
    let tracked: Vec<_> = (0..4097)
        .map(|index| {
            let mut row = pending.clone();
            row[0] = serde_json::json!(format!("tracked-{index}"));
            row
        })
        .collect();
    // Distinct otherwise valid NEW rows must still hit the admission bound.
    let decoded_rows: Vec<(String, crate::brain::reward::Pending)> =
        serde_json::from_value(serde_json::Value::Array(tracked.clone())).unwrap();
    assert!(
        crate::brain::reward::RewardRecovery::default()
            .validate(&decoded_rows)
            .is_err()
    );
    assert!(
        RewardCollector::new()
            .restore_recovered(decoded_rows, Default::default())
            .is_err()
    );
    json["pending_rewards"] = serde_json::Value::Array(tracked);
    assert!(serde_json::from_value::<Stores>(json).is_err());
    for field in ["duplicate", "scope", "emoji"] {
        let mut json = serde_json::to_value(&stores).unwrap();
        let reactions = &mut json["reward_recovery"]["reactions"];
        match field {
            "duplicate" => {
                let row = reactions[0].clone();
                reactions.as_array_mut().unwrap().push(row);
            }
            "scope" => reactions[0]["key"]["scope"] = serde_json::json!("slack:wrong"),
            _ => reactions[0]["key"]["emoji"] = serde_json::json!("unsupported"),
        }
        let loaded: Stores = serde_json::from_value(json).unwrap();
        assert!(
            loaded
                .reward_recovery
                .validate(&loaded.pending_rewards)
                .is_err(),
            "{field}"
        );
    }
}

#[test]
fn legacy_rows_above_new_capacity_keep_canonical_data_and_settle_once() {
    let dir = temp_dir("legacy-4097-distinct");
    let mut unrelated = Stores::default();
    unrelated.guilds.insert(
        "discord:legacy".into(),
        GuildSettings {
            default_persona: Persona::Aviva,
            reply_cooldown_seconds: 47,
            learning_enabled: true,
            ..Default::default()
        },
    );
    assert!(unrelated.memory.remember(
        "discord:legacy",
        "discord:member",
        "fixture prefers rust",
        90
    ));
    unrelated.store_reputation("discord:legacy", "discord:member", 0.62, 2);
    let mut document = serde_json::to_value(&unrelated).unwrap();
    document.as_object_mut().unwrap().remove("reward_recovery");
    let rows: Vec<_> = (0..4097)
        .map(|index| {
            serde_json::json!([
                format!("legacy-{index}"), {
                    "state":vec![index as f32; 18], "action":1,
                    "scoped_guild_id":"discord:legacy",
                    "reward":([-4.0, -1.2, 0.0, 0.8, 4.0][index % 5]),
                    "positive_reactions":index % 4,
                    "created_at":100 + index % 2, "settle_immediately":false,
                    "scope":"discord:legacy-channel", "ask":"historical question",
                    "asker":"discord:member", "delayed_sum":if index % 3 == 0 {1.0} else {-1.0},
                    "delayed_count":if index % 3 == 0 {2} else {0}
                }
            ])
        })
        .collect();
    document["pending_rewards"] = serde_json::Value::Array(rows);
    let expected: Vec<(String, crate::brain::reward::Pending)> =
        serde_json::from_value(document["pending_rewards"].clone()).unwrap();
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        Stores::state_path(&dir),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();

    let mut stores = Stores::load(&dir).expect("4097 distinct pre-ledger rows must load");
    let mut c = RewardCollector::new();
    c.restore_recovered(
        stores.pending_rewards.clone(),
        stores.reward_recovery.clone(),
    )
    .unwrap();
    assert_eq!(c.pending_len(), 4097);
    // The legacy cohort has no contribution provenance, so incoming reactions
    // cannot edit any of these mixed preexisting values.
    assert_eq!(
        c.reaction(
            ReactionKey {
                scope: "discord:legacy-channel".into(),
                message: "legacy-0".into(),
                reactor_hash: 1,
                emoji: "👍".into()
            },
            true,
            100
        ),
        FeedbackAttribution::Unsupported
    );
    assert!(
        c.settle_expired(250).is_empty(),
        "exactly150s stays pending"
    );

    let mut settled = Vec::new();
    for time in [250, 251, 252] {
        let before = c.pending_len();
        c.register_reply(
            vec![0.0],
            1,
            "too-early-new",
            "discord:legacy",
            "discord:legacy-channel",
            1000,
        );
        assert_eq!(
            c.pending_len(),
            before,
            "finite carryover refuses new tracking until drained"
        );
        stores.pending_rewards = c.export_pending();
        stores.reward_recovery = c.export_recovery();
        stores
            .save(&dir)
            .expect("legacy cohort publishes without truncation");
        let loaded = Stores::load(&dir).unwrap();
        assert_eq!(loaded.guilds, unrelated.guilds);
        assert_eq!(loaded.memory, unrelated.memory);
        assert_eq!(loaded.reputations, unrelated.reputations);
        let remaining: std::collections::HashMap<_, _> =
            loaded.pending_rewards.iter().cloned().collect();
        for (id, pending) in &expected {
            if let Some(actual) = remaining.get(id) {
                assert_eq!(actual, pending, "unconsumed {id}");
            }
        }
        assert_eq!(remaining.len() + settled.len(), 4097);
        c = RewardCollector::new();
        c.restore_recovered(
            loaded.pending_rewards.clone(),
            loaded.reward_recovery.clone(),
        )
        .unwrap();
        stores = loaded;
        settled.extend(c.settle_expired(time));
    }
    assert_eq!(settled.len(), 4097);
    assert_eq!(c.pending_len(), 0);
    let mut seen = std::collections::HashSet::new();
    for (guild, exp) in settled {
        assert_eq!(guild, "discord:legacy");
        let index = exp.state[0] as usize;
        assert!(seen.insert(index), "settled once");
        let pending = &expected[index].1;
        let reward = crate::brain::outcome::blend(
            pending.reward,
            pending.delayed_sum,
            pending.delayed_count,
        )
        .clamp(-3.0, 3.0);
        assert_eq!(exp.reward.to_bits(), reward.to_bits());
        assert_eq!(exp.state, pending.state);
    }
    stores.pending_rewards = c.export_pending();
    stores.reward_recovery = c.export_recovery();
    let recovery = serde_json::to_value(&stores.reward_recovery).unwrap();
    assert!(recovery["settled"].as_array().unwrap().len() <= 4096);
    assert!(recovery["reactions"].as_array().unwrap().is_empty());
    stores.save(&dir).unwrap();
    let loaded = Stores::load(&dir).unwrap();
    assert_eq!(loaded.guilds, unrelated.guilds);
    assert_eq!(loaded.memory, unrelated.memory);
    assert_eq!(loaded.reputations, unrelated.reputations);
    let mut restarted = RewardCollector::new();
    restarted
        .restore_recovered(loaded.pending_rewards, loaded.reward_recovery)
        .unwrap();
    assert!(restarted.settle_expired(1000).is_empty());
    restarted.register_reply(
        vec![0.0],
        1,
        "legacy-0",
        "discord:legacy",
        "discord:legacy-channel",
        100,
    );
    assert_eq!(
        restarted.pending_len(),
        0,
        "compact retirement prevents old creation replay"
    );
    restarted.register_reply(
        vec![0.0],
        1,
        "fresh",
        "discord:legacy",
        "discord:legacy-channel",
        1000,
    );
    assert_eq!(
        restarted.pending_len(),
        1,
        "new tracking resumes after carryover drains"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn legacy_ask_migrates_without_raw_republication() {
    let dir = temp_dir("legacy-ask-minimized");
    let c = collector();
    let stores = Stores {
        pending_rewards: c.export_pending(),
        ..Default::default()
    };
    let mut document = serde_json::to_value(&stores).unwrap();
    let row = document["pending_rewards"][0][1].as_object_mut().unwrap();
    row.remove("ask_signature");
    row.insert(
        "ask".into(),
        serde_json::json!("PRIVATE fixture gateway timeout?"),
    );
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        Stores::state_path(&dir),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();
    let loaded = Stores::load(&dir).unwrap();
    loaded.save(&dir).unwrap();
    let published = fs::read_to_string(Stores::state_path(&dir)).unwrap();
    assert!(!published.contains("PRIVATE fixture gateway timeout"));
    assert!(!published.contains("\"ask\":"));
    let reopened = Stores::load(&dir).unwrap();
    assert!(loaded.payload_eq(&reopened));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn minimized_pending_default_malformed_and_negative_zero_survive_canonical_paths() {
    let dir = temp_dir("minimized-pending-boundaries");
    let c = collector();
    let stores = Stores {
        pending_rewards: c.export_pending(),
        ..Default::default()
    };
    let mut document = serde_json::to_value(&stores).unwrap();
    document["pending_rewards"][0][1]
        .as_object_mut()
        .unwrap()
        .remove("ask_signature");
    document["pending_rewards"][0][1]["reward"] = serde_json::json!(-0.0_f32);
    let migrated: Stores = serde_json::from_value(document.clone()).unwrap();
    migrated.save(&dir).unwrap();
    let loaded = Stores::load(&dir).unwrap();
    let mut restored = RewardCollector::new();
    restored
        .restore_recovered(loaded.pending_rewards, loaded.reward_recovery)
        .unwrap();
    assert_eq!(
        restored.settle_expired(251)[0].1.reward.to_bits(),
        (-0.0_f32).to_bits()
    );
    assert!(
        !fs::read_to_string(Stores::state_path(&dir))
            .unwrap()
            .contains("\"ask\":")
    );
    for malformed in [
        serde_json::json!(42),
        serde_json::json!([]),
        serde_json::json!({"token_hashes":[1,1],"markers":{"correction":false,"thanks":false,"question":false}}),
        serde_json::json!({"token_hashes":(0..33).collect::<Vec<_>>(),"markers":{"correction":false,"thanks":false,"question":false}}),
    ] {
        let mut bad = document.clone();
        bad["pending_rewards"][0][1]["ask"] = malformed;
        fs::write(Stores::state_path(&dir), serde_json::to_vec(&bad).unwrap()).unwrap();
        assert!(Stores::load(&dir).is_err());
    }
    let mut invalid = stores;
    invalid.pending_rewards[0].1.ask_signature.token_hashes = vec![1, 1];
    assert!(invalid.save(&dir).is_err());
    fs::remove_dir_all(dir).unwrap();
}

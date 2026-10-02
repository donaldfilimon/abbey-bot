use super::*;
use crate::tools::ToolHost;

#[test]
fn model_memory_tool_cannot_store_replace_or_queue_personal_facts() {
    let state = AppState::in_memory();
    state
        .memory_service()
        .remember("discord:g", "discord:u", "member's chosen fact", 1)
        .unwrap();
    let turn = crate::memory_gate::MemoryTurn::default();
    let mut scope = ToolScope {
        memory_turn: Some(&turn),
        state: &state,
        network: SocialNetwork::Discord,
        scoped_guild: "discord:g".into(),
        scoped_user: "discord:u".into(),
        scoped_channel: "discord:c".into(),
        now: 10,
        persona: crate::persona::Persona::Abbey,
    };
    for replaces in [None, Some("member's chosen fact")] {
        let result = scope.remember_fact("model-generated inference", replaces);
        assert!(result.contains("not stored or queued"));
        assert!(result.contains("`/remember`"));
        assert_eq!(
            state.memory_service().facts("discord:g", "discord:u"),
            ["member's chosen fact"]
        );
        assert!(
            state
                .memory_service()
                .pending_supersessions("discord:g", "discord:u")
                .is_empty()
        );
        assert!(AppState::lock(&state.memory_queue).is_empty());
    }
}

#[test]
fn model_memory_refusal_happens_before_episode_gate_admission() {
    use crate::episode_gate::{EpisodeGate, EpisodeGateConfig};
    use std::sync::Arc;
    let mut state = AppState::in_memory();
    let temp = std::env::temp_dir();
    let config = serde_json::json!({
        "abi_cli": temp.join("consent-test-cli-must-never-run"),
        "endpoint": "http://127.0.0.1:9", "token_file": temp.join("consent-test-token-must-never-read"),
        "policy_version": "policy_v1", "contract_revision": 2,
        "contract_digest": "01".repeat(32), "timeout_secs": 1
    });
    Arc::get_mut(&mut state).unwrap().episode_gate = Some(Arc::new(EpisodeGate::new(
        EpisodeGateConfig::from_json(&config.to_string()).unwrap(),
    )));
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
    assert!(
        scope
            .remember_fact("inferred fact", None)
            .contains("not stored or queued")
    );
    assert!(AppState::lock(&state.memory_queue).is_empty());
    assert!(
        state
            .memory_service()
            .facts("discord:g", "discord:u")
            .is_empty()
    );
    let counters = state.episode_gate.as_ref().unwrap().counters();
    assert_eq!(
        (counters.appended, counters.rejected, counters.unavailable),
        (0, 0, 0)
    );
}

use super::*;
use crate::tools::ToolHost;

#[test]
fn personal_memory_adapter_tools_cannot_inspect_legacy_facts_pending_or_recent_history() {
    let state = AppState::in_memory();
    state
        .memory_service()
        .remember("discord:g", "discord:u", "LEGACY_PERSONAL_SECRET", 1)
        .unwrap();
    {
        let mut stores = AppState::lock(&state.stores);
        stores.memory.propose_supersession(
            "discord:g",
            "discord:u",
            "PENDING_SECRET",
            "LEGACY_PERSONAL_SECRET",
            1,
        );
        stores
            .memory
            .record_message("discord:c", "other member", "MIXED_HISTORY_SECRET", 1);
    }
    let mut host = ToolScope {
        memory_turn: None,
        state: &state,
        network: SocialNetwork::Discord,
        scoped_guild: "discord:g".into(),
        scoped_user: "discord:u".into(),
        scoped_channel: "discord:c".into(),
        now: 2,
        persona: crate::persona::Persona::Abbey,
    };
    for result in [
        host.recall("SECRET"),
        host.list_facts(),
        host.recent_messages(10),
    ] {
        assert!(!result.contains("LEGACY_PERSONAL_SECRET"));
        assert!(!result.contains("PENDING_SECRET"));
        assert!(!result.contains("MIXED_HISTORY_SECRET"));
    }
    assert_eq!(
        state
            .memory_service()
            .subject_snapshot("discord:g", "discord:u")
            .0,
        ["LEGACY_PERSONAL_SECRET"]
    );
}

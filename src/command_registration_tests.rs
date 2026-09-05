use super::*;
use serde_json::{Value, json};
use serenity::all::{Command, CommandType};

fn entry_point_fixture() -> Command {
    serde_json::from_value(json!({
        "id": "11",
        "type": 4,
        "application_id": "22",
        "guild_id": null,
        "name": "launch",
        "name_localized": null,
        "name_localizations": null,
        "description": "Launch Abbey's Activity",
        "description_localized": null,
        "description_localizations": null,
        "options": [],
        "default_member_permissions": null,
        "dm_permission": null,
        "nsfw": false,
        "integration_types": [0, 1],
        "contexts": [0, 1, 2],
        "version": "33",
        "handler": 2
    }))
    .expect("valid Discord Entry Point fixture")
}

#[test]
fn global_bulk_registration_preserves_the_complete_entry_point_contract() {
    let generated = vec![serenity::all::CreateCommand::new("help").description("Abbey help")];
    let merged = merge_entry_point_commands(generated, [entry_point_fixture()]);
    assert_eq!(merged.len(), 2);

    let preserved = serde_json::to_value(&merged[1]).expect("serialize merged command");
    assert_eq!(
        preserved,
        json!({
            "name": "launch",
            "name_localizations": {},
            "description": "Launch Abbey's Activity",
            "description_localizations": {},
            "options": [],
            "type": 4,
            "integration_types": [0, 1],
            "contexts": [0, 1, 2],
            "nsfw": false,
            "handler": 2
        })
    );
}

#[test]
fn global_merge_ignores_non_entry_point_commands() {
    let ordinary: Command = serde_json::from_value(json!({
        "id": "44",
        "type": 1,
        "application_id": "22",
        "guild_id": null,
        "name": "old-command",
        "name_localized": null,
        "name_localizations": null,
        "description": "old",
        "description_localized": null,
        "description_localizations": null,
        "options": [],
        "default_member_permissions": null,
        "dm_permission": null,
        "nsfw": false,
        "integration_types": [0],
        "contexts": [0],
        "version": "55",
        "handler": null
    }))
    .expect("valid ordinary command fixture");

    let merged = merge_entry_point_commands(Vec::new(), [ordinary]);
    assert!(merged.is_empty());
}

#[test]
fn guild_registration_payload_never_invents_an_entry_point() {
    let payload = poise::builtins::create_application_commands(&application_commands());
    let encoded = serde_json::to_value(payload).expect("serialize guild registration payload");
    let commands = encoded.as_array().expect("command list");
    assert!(!commands.iter().any(|command: &Value| {
        command.get("type").and_then(Value::as_u64)
            == Some(u64::from(u8::from(CommandType::PrimaryEntryPoint)))
    }));
}

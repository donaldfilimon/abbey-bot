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
        "name_localizations": {"fr": "lancer"},
        "description": "Launch Abbey's Activity",
        "description_localized": null,
        "description_localizations": {"fr": "Lancer Abbey"},
        "options": [],
        "default_member_permissions": "32",
        "dm_permission": null,
        "nsfw": true,
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
            "name_localizations": {"fr": "lancer"},
            "description": "Launch Abbey's Activity",
            "description_localizations": {"fr": "Lancer Abbey"},
            "default_member_permissions": "32",
            "options": [],
            "type": 4,
            "integration_types": [0, 1],
            "contexts": [0, 1, 2],
            "nsfw": true,
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

/// Export the exact source registration request for an operator's read-only
/// comparison with Discord. This starts no client and needs no bot credential.
/// Unix only: creation relies on owner-only mode bits, not inherited ACLs.
#[cfg(unix)]
#[test]
#[ignore = "operator export: set ABBEY_COMMAND_PAYLOAD_OUTPUT to a new private file"]
fn export_command_registration_payload() {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let destination = std::env::var_os("ABBEY_COMMAND_PAYLOAD_OUTPUT")
        .expect("ABBEY_COMMAND_PAYLOAD_OUTPUT must name a new private output file");
    let payload = poise::builtins::create_application_commands(&application_commands());
    let mut encoded = serde_json::to_vec_pretty(&payload).expect("serialize registration payload");
    encoded.push(b'\n');

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    let mut output = options
        .open(destination)
        .expect("create new command payload file without replacing existing files or symlinks");
    output.write_all(&encoded).expect("write command payload");
    output.sync_all().expect("sync command payload");
}

#[tokio::test]
async fn registration_runs_global_then_optional_home_and_propagates_each_failure() {
    use std::sync::Mutex;

    for home in [None, Some(serenity::all::GuildId::new(42))] {
        for failed_scope in [None, Some("global"), Some("home")] {
            let calls = Mutex::new(Vec::new());
            let result = register_command_scopes(
                home,
                async {
                    calls.lock().unwrap().push("global");
                    if failed_scope == Some("global") {
                        Err("global failed")
                    } else {
                        Ok(())
                    }
                },
                |id| {
                    assert_eq!(Some(id), home);
                    async {
                        calls.lock().unwrap().push("home");
                        if failed_scope == Some("home") {
                            Err("home failed")
                        } else {
                            Ok(())
                        }
                    }
                },
            )
            .await;
            let expected_calls = if home.is_some() && failed_scope != Some("global") {
                vec!["global", "home"]
            } else {
                vec!["global"]
            };
            assert_eq!(*calls.lock().unwrap(), expected_calls);
            assert_eq!(
                result,
                match failed_scope {
                    Some("global") => Err("global failed"),
                    Some("home") if home.is_some() => Err("home failed"),
                    _ => Ok(()),
                }
            );
        }
    }
}

#[test]
fn entry_point_legacy_dm_restriction_is_migrated_without_overriding_contexts() {
    let mut entry = entry_point_fixture();
    entry.dm_permission = Some(false);
    for contexts in [None, Some(vec![serenity::all::InteractionContext::BotDm])] {
        entry.contexts = contexts.clone();
        let merged = merge_entry_point_commands(Vec::new(), [entry.clone()]);
        let encoded = serde_json::to_value(&merged[0]).unwrap();
        assert_eq!(
            encoded["contexts"],
            if contexts.is_some() {
                json!([1])
            } else {
                json!([0])
            }
        );
    }
}

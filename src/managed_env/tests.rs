use super::*;
#[test]
fn parser_preserves_supported_quotes_exports_and_never_evaluates_shell_text() {
    let home = Path::new("/fixture");
    let env=ManagedEnvironment::parse(b" # comment\nexport DISCORD_TOKEN='PRIVATE_TOKEN'\nABBEY_VOICE_INSTRUCTIONS=\"literal $(touch /never) `private`\"\nABBEY_DATA_DIR=/untrusted\nHOME=/untrusted\n",home).unwrap();
    assert_eq!(format!("{env:?}"), "ManagedEnvironment([redacted])");
    let values = env.into_values();
    assert_eq!(
        values[std::ffi::OsStr::new("DISCORD_TOKEN")],
        "PRIVATE_TOKEN"
    );
    assert_eq!(
        values[std::ffi::OsStr::new("ABBEY_VOICE_INSTRUCTIONS")],
        "literal $(touch /never) `private`"
    );
    assert_eq!(
        values[std::ffi::OsStr::new("ABBEY_DATA_DIR")],
        "/fixture/.local/share/abbey-bot"
    );
    assert_eq!(values[std::ffi::OsStr::new("HOME")], "/fixture");
}
#[test]
fn parser_rejects_ambiguous_or_incomplete_configuration_without_values() {
    for raw in [
        "DISCORD_TOKEN=\"\"",
        "DISCORD_TOKEN=PRIVATE\nDISCORD_TOKEN=again",
        "DISCORD_TOKEN='PRIVATE",
        "DISCORD_TOKEN=PRIVATE\nABBEY_VOICE_GUILD_ID=1",
        "DISCORD_TOKEN=PRIVATE\nABBEY_VOICE_GUILD_ID=1\nABBEY_VOICE_CHANNEL_ID=2",
        "DISCORD_TOKEN=PRIVATE\n1BAD=value",
    ] {
        let error = ManagedEnvironment::parse(raw.as_bytes(), Path::new("/fixture")).unwrap_err();
        assert!(!format!("{error:?} {error}").contains("PRIVATE"));
    }
}
#[test]
fn privacy_failure_prevents_any_owner_file_access() {
    assert_eq!(
        ManagedEnvironment::load_after_privacy(
            Path::new("/does-not-exist"),
            &PersistReport::memory_only()
        )
        .unwrap_err(),
        ManagedEnvironmentError::PrivacyNotCommitted
    );
}
#[cfg(unix)]
#[test]
fn owner_file_reader_accepts_partial_projection_but_rejects_symlink_and_mode() {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };
    let home = crate::readiness::tests::temporary_home();
    let directory = home.join(".config/abbey-bot");
    fs::create_dir_all(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let path = directory.join("env");
    fs::write(&path, b"DISCORD_TOKEN=PRIVATE_TOKEN\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let report = PersistReport::from_components(
        PersistComponentOutcome::Committed,
        PersistComponentOutcome::Failed(crate::persist::PersistErrorCategory::ProjectionEncode),
    );
    assert!(ManagedEnvironment::load_after_privacy(&home, &report).is_ok());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(ManagedEnvironment::load_after_privacy(&home, &report).is_err());
    fs::remove_file(&path).unwrap();
    symlink(home.join("sentinel"), &path).unwrap();
    assert!(ManagedEnvironment::load_after_privacy(&home, &report).is_err());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn installer_and_runtime_share_literal_environment_validation_cases() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../tests/fixtures/managed-environment-v1.json"
    ))
    .unwrap();
    assert_eq!(cases.len(), 34);
    for case in cases {
        let raw = match case.get("document").and_then(serde_json::Value::as_str) {
            Some(text) => text.as_bytes().to_vec(),
            None => case["bytes_hex"]
                .as_str()
                .unwrap()
                .as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect(),
        };
        assert_eq!(
            ManagedEnvironment::parse(&raw, Path::new("/fixture")).is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
    }
}

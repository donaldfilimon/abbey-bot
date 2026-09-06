use super::*;
#[cfg(unix)]
mod unix {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };
    fn home() -> std::path::PathBuf {
        let home = crate::readiness::tests::temporary_home();
        let config = home.join(".config/abbey-bot");
        fs::create_dir_all(&config).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(
            config.join("env"),
            b"DISCORD_TOKEN=PRIVATE_CREDENTIAL_CANARY\n",
        )
        .unwrap();
        fs::set_permissions(config.join("env"), fs::Permissions::from_mode(0o600)).unwrap();
        home
    }
    #[test]
    fn real_preflight_rewrites_legacy_rows_before_env_and_never_logs_canaries() {
        let home = home();
        let data = home.join(".local/share/abbey-bot");
        fs::create_dir_all(&data).unwrap();
        fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(data.join(persist::STATE_FILE),br#"{"memory":{"interactions":{"entries":[{"command":"PRIVATE_COMMAND_CANARY","user_id":"PRIVATE_USER_CANARY","guild_id":"PRIVATE_GUILD_CANARY","channel_id":"PRIVATE_CHANNEL_CANARY","succeeded":false,"error":"PRIVATE_ERROR_CANARY","duration_ms":17,"at":42}]}}}"#).unwrap();
        fs::set_permissions(
            data.join(persist::STATE_FILE),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let service = begin(&home).unwrap();
        assert_eq!(
            service.privacy_report.overall,
            persist::PersistOverall::Complete
        );
        let canonical = fs::read_to_string(data.join(persist::STATE_FILE)).unwrap();
        assert!(!canonical.contains("PRIVATE_"));
        assert!(canonical.contains("at_unix_ms"));
        let logs =
            fs::read_to_string(home.join("Library/Logs/abbey-bot/abbey-bot.events.jsonl")).unwrap();
        assert!(!logs.contains("PRIVATE_"));
        assert!(!logs.contains(home.to_str().unwrap()));
        assert!(!service.fatal.pending());
        assert!(!data.join("readiness.json").exists()); // preflight alone is not ready
        assert!(data.join("bootstrap-status.json").is_file());
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn unsafe_log_parent_publishes_fixed_bootstrap_without_reading_environment() {
        let home = home();
        let env = home.join(".config/abbey-bot/env");
        fs::remove_file(&env).unwrap();
        symlink(home.join("does-not-exist"), home.join("Library")).unwrap();
        assert!(matches!(begin(&home), Err(ManagedStartupFailure::Logging)));
        let bytes = fs::read(home.join(".local/share/abbey-bot/bootstrap-status.json")).unwrap();
        let status: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(status["phase"], "failed");
        assert_eq!(status["code"], "log_directory");
        assert!(
            !String::from_utf8(bytes)
                .unwrap()
                .contains(home.to_str().unwrap())
        );
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn privacy_decode_failure_wins_over_missing_credentials() {
        let home = home();
        fs::remove_file(home.join(".config/abbey-bot/env")).unwrap();
        let data = home.join(".local/share/abbey-bot");
        fs::create_dir_all(&data).unwrap();
        fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(data.join(persist::STATE_FILE), b"PRIVATE_CORRUPT_STATE").unwrap();
        fs::set_permissions(
            data.join(persist::STATE_FILE),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        assert!(matches!(begin(&home), Err(ManagedStartupFailure::State)));
        assert_eq!(
            fs::read(data.join(persist::STATE_FILE)).unwrap(),
            b"PRIVATE_CORRUPT_STATE"
        );
        fs::remove_dir_all(home).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn unsafe_active_or_archive_reports_log_file_without_env_access() {
    use std::{fs, os::unix::fs::PermissionsExt};
    for leaf in ["abbey-bot.events.jsonl", "abbey-bot.events.jsonl.3"] {
        let home = crate::readiness::tests::temporary_home();
        let directory = home.join("Library/Logs/abbey-bot");
        fs::create_dir_all(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(directory.join(leaf), b"PRIVATE_FILE").unwrap();
        fs::set_permissions(directory.join(leaf), fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(begin(&home), Err(ManagedStartupFailure::Logging)));
        let status: serde_json::Value = serde_json::from_slice(
            &fs::read(home.join(".local/share/abbey-bot/bootstrap-status.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(status["code"], "log_file");
        assert!(!home.join(".config").exists());
        fs::remove_dir_all(home).unwrap();
    }
}

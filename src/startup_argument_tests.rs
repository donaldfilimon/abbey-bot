#[test]
fn managed_panic_hook_child() {
    if std::env::var_os("ABBEY_TEST_MANAGED_PANIC").is_some() {
        super::configure_managed_panic_hook(None);
        panic!("MANAGED-PANIC-PRIVATE-CANARY");
    }
}
#[test]
fn managed_panic_payload_cannot_reach_process_output() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "startup_argument_tests::managed_panic_hook_child",
            "--nocapture",
        ])
        .env_clear()
        .env("ABBEY_TEST_MANAGED_PANIC", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    for bytes in [&output.stdout, &output.stderr] {
        assert!(!String::from_utf8_lossy(bytes).contains("MANAGED-PANIC-PRIVATE-CANARY"));
    }
}
#[test]
fn managed_service_mode_requires_exactly_one_argument() {
    assert_eq!(
        super::parse_startup_arguments(
            ["--managed-service"]
                .into_iter()
                .map(std::ffi::OsString::from)
        )
        .unwrap(),
        super::StartupAction::ManagedDiscord
    );
    for tail in [
        "--managed-service",
        "--provider-self-test",
        "--voice-self-test",
        "private-canary",
    ] {
        assert!(
            super::parse_startup_arguments(
                ["--managed-service", tail]
                    .into_iter()
                    .map(std::ffi::OsString::from)
            )
            .is_err()
        );
    }
}

use super::*;

fn parse(arguments: &[&str]) -> Result<StartupAction, String> {
    parse_startup_arguments(arguments.iter().map(std::ffi::OsString::from))
}

#[test]
fn no_arguments_starts_the_discord_service() {
    assert_eq!(parse(&[]).unwrap(), StartupAction::Discord);
}

#[test]
fn exact_voice_self_test_has_one_create_new_output() {
    assert_eq!(
        parse(&["--voice-self-test", "audition.wav"]).unwrap(),
        StartupAction::VoiceSelfTest(std::path::PathBuf::from("audition.wav"))
    );
    assert!(parse(&["--voice-self-test"]).is_err());
    assert!(parse(&["--voice-self-test", "one.wav", "two.wav"]).is_err());
}

#[test]
fn server_plan_hands_its_arguments_to_the_engine_parser() {
    let action = parse(&[
        "--server-plan",
        "blueprints/mlai-community.toml",
        "--guild",
        "42",
    ])
    .unwrap();
    assert_eq!(
        action,
        StartupAction::ServerPlan(server::run::Options {
            plan: std::path::PathBuf::from("blueprints/mlai-community.toml"),
            guild_id: 42,
            stage: server::diff::Stage::Additive,
            category: None,
            apply: false,
        })
    );
    assert!(parse(&["--server-plan"]).is_err());
    assert!(
        parse(&["--server-plan", "p.toml"]).is_err(),
        "--guild is required"
    );
    assert!(parse(&["--server-plan", "p.toml", "--guild", "0"]).is_err());
}

#[test]
fn an_unknown_or_mistyped_mode_cannot_start_discord() {
    assert!(parse(&["--voice-self-tset", "audition.wav"]).is_err());
    assert!(parse(&["unexpected"]).is_err());
}

#[test]
fn provider_self_test_requires_exact_target_and_json_mode() {
    assert_eq!(
        provider_self_test_usage(),
        "usage: abbey-bot --provider-self-test primary|fm|all --json"
    );
    assert_eq!(provider_self_test::SelfTestExit::Success.code(), 0);
    assert_eq!(provider_self_test::SelfTestExit::ProbeFailure.code(), 1);
    assert_eq!(provider_self_test::SelfTestExit::Configuration.code(), 2);
    assert_eq!(
        parse(&["--provider-self-test", "primary", "--json"]).unwrap(),
        StartupAction::ProviderSelfTest(provider::QualificationTarget::Primary)
    );
    assert_eq!(
        parse(&["--provider-self-test", "fm", "--json"]).unwrap(),
        StartupAction::ProviderSelfTest(provider::QualificationTarget::Fm)
    );
    assert_eq!(
        parse(&["--provider-self-test", "all", "--json"]).unwrap(),
        StartupAction::ProviderSelfTest(provider::QualificationTarget::All)
    );
    for invalid in [
        &["--provider-self-test"][..],
        &["--provider-self-test", "pcc", "--json"],
        &["--provider-self-test", "fm"],
        &["--provider-self-test", "fm", "--json", "extra"],
    ] {
        assert!(parse(invalid).is_err(), "accepted {invalid:?}");
    }
}

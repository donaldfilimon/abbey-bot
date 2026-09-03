use super::*;

#[cfg(unix)]
const MANIFEST_PATH: &str = "/private/state/providers.json";
#[cfg(windows)]
const MANIFEST_PATH: &str = r"C:\private\state\providers.json";
#[cfg(unix)]
const STATE_PATH: &str = "/private/state";
#[cfg(windows)]
const STATE_PATH: &str = r"C:\private\state";
#[cfg(unix)]
const SANDBOX_PATH: &str = "/usr/bin/sandbox-exec";
#[cfg(windows)]
const SANDBOX_PATH: &str = r"C:\Windows\System32\sandbox.exe";
#[cfg(unix)]
const CLAUDE_PATH: &str = "/usr/local/bin/claude";
#[cfg(windows)]
const CLAUDE_PATH: &str = r"C:\private\bin\claude.exe";
#[cfg(unix)]
const DEBUG_MANIFEST_PATH: &str = "/private/config/manifest.json";
#[cfg(windows)]
const DEBUG_MANIFEST_PATH: &str = r"C:\private\config\manifest.json";
#[cfg(unix)]
const DEBUG_SANDBOX_PATH: &str = "/private/bin/sandbox";
#[cfg(windows)]
const DEBUG_SANDBOX_PATH: &str = r"C:\private\bin\sandbox.exe";
#[cfg(unix)]
const DEBUG_CLAUDE_PATH: &str = "/private/bin/claude";
#[cfg(windows)]
const DEBUG_CLAUDE_PATH: &str = r"C:\private\bin\claude.exe";
use std::path::Path;

fn id(raw: &str) -> ProviderId {
    ProviderId::parse(raw).expect("valid provider ID")
}

#[test]
fn empty_allowlists_deny_all_and_discovery_is_explicit() {
    let config = ProviderConfig::from_iter([
        (DISCOVERY, ""),
        (CLOUD_ALLOW, ""),
        (AGENT_CLI_ALLOW, ""),
        ("UNRELATED_SECRET", "must-not-be-read"),
    ])
    .expect("empty policy parses");

    assert!(config.discovery.is_empty());
    assert!(config.cloud_allow.is_empty());
    assert!(config.agent_cli_allow.is_empty());
    assert!(config.providers.is_empty());
}

#[test]
fn parses_every_approved_policy_and_provider_specific_setting() {
    let config = ProviderConfig::from_iter([
        (DISCOVERY, "mlx,ollama,foundation_models,claude"),
        (ORDER, "mlx,ollama,foundation-models,claude"),
        (DISABLED, "cursor"),
        (MANIFEST, MANIFEST_PATH),
        (STATE_DIR, STATE_PATH),
        (CLOUD_ALLOW, "anthropic"),
        (AGENT_CLI_ALLOW, "claude"),
        (SANDBOX_RUNNER, SANDBOX_PATH),
        (SANDBOX_PROFILE, "abbey-provider-v2"),
        ("ABBEY_PROVIDER_MLX_ENDPOINT", "http://127.0.0.1:8282"),
        ("ABBEY_PROVIDER_MLX_MODEL", "/private/models/mlx"),
        (
            "ABBEY_PROVIDER_MLX_MODEL_IDENTITY",
            "73bcf09092aa277861d5a191b989b666f7f32e8f",
        ),
        ("ABBEY_PROVIDER_CLAUDE_BINARY", CLAUDE_PATH),
        ("ABBEY_PROVIDER_CLAUDE_CREDENTIAL", "secret-value"),
    ])
    .expect("approved provider environment parses");

    assert_eq!(
        config.order,
        ["mlx", "ollama", "foundation-models", "claude"]
            .map(id)
            .to_vec()
    );
    assert_eq!(config.manifest, Some(PathBuf::from(MANIFEST_PATH)));
    assert_eq!(config.state_dir, Some(PathBuf::from(STATE_PATH)));
    assert!(config.disabled.contains(&id("cursor")));
    assert!(config.cloud_allow.contains(&id("anthropic")));
    assert!(config.agent_cli_allow.contains(&id("claude")));
    assert_eq!(config.providers.len(), 6);

    let mlx = config.provider(&id("mlx")).expect("MLX settings");
    assert_eq!(mlx.endpoint.as_deref(), Some("http://127.0.0.1:8282"));
    assert_eq!(mlx.model.as_deref(), Some("/private/models/mlx"));
    assert_eq!(
        mlx.model_identity.as_deref(),
        Some("73bcf09092aa277861d5a191b989b666f7f32e8f")
    );

    let claude = config.provider(&id("claude")).expect("Claude settings");
    assert_eq!(claude.binary.as_deref(), Some(Path::new(CLAUDE_PATH)));
    assert_eq!(
        claude
            .credential
            .as_ref()
            .map(ProviderCredential::expose_secret),
        Some("secret-value")
    );
}

#[test]
fn debug_output_never_contains_private_configuration_values() {
    let config = ProviderConfig::from_iter([
        (MANIFEST, DEBUG_MANIFEST_PATH),
        (STATE_DIR, STATE_PATH),
        (SANDBOX_RUNNER, DEBUG_SANDBOX_PATH),
        (SANDBOX_PROFILE, "private-profile"),
        ("ABBEY_PROVIDER_CLAUDE_BINARY", DEBUG_CLAUDE_PATH),
        ("ABBEY_PROVIDER_CLAUDE_ENDPOINT", "https://private.example"),
        ("ABBEY_PROVIDER_CLAUDE_MODEL", "private-model"),
        ("ABBEY_PROVIDER_CLAUDE_MODEL_IDENTITY", "private-hash"),
        ("ABBEY_PROVIDER_CLAUDE_CREDENTIAL", "super-secret"),
    ])
    .expect("private config parses");

    let debug = format!("{config:?}");
    for canary in [
        DEBUG_MANIFEST_PATH,
        STATE_PATH,
        DEBUG_SANDBOX_PATH,
        "private-profile",
        DEBUG_CLAUDE_PATH,
        "private.example",
        "private-model",
        "private-hash",
        "super-secret",
    ] {
        assert!(!debug.contains(canary), "debug leaked {canary:?}");
    }
    assert!(debug.contains("credential_configured: true"));
}

#[test]
fn malformed_lists_and_canonical_duplicates_fail_closed() {
    for value in ["mlx,,ollama", "mlx,MLX", "mlx,☃"] {
        let error = ProviderConfig::from_iter([(ORDER, value)]).expect_err("must fail");
        assert!(error.to_string().starts_with(ORDER));
        assert!(!error.to_string().contains(value));
    }
}

#[test]
fn provider_paths_and_endpoints_are_exact_and_credential_free() {
    for (name, value) in [
        ("ABBEY_PROVIDER_CLAUDE_BINARY", "relative/claude"),
        ("ABBEY_PROVIDER_CLAUDE_BINARY", "/usr/local/../bin/claude"),
        ("ABBEY_PROVIDER_MLX_ENDPOINT", "http://user:pass@127.0.0.1"),
        ("ABBEY_PROVIDER_MLX_ENDPOINT", "http://127.0.0.1?key=secret"),
        ("ABBEY_PROVIDER_CLAUDE_CREDENTIAL", "   "),
        ("ABBEY_PROVIDER_CLAUDE_CREDENTIAL", " secret-value"),
    ] {
        let error = ProviderConfig::from_iter([(name, value)]).expect_err("must fail");
        let rendered = error.to_string();
        if !value.trim().is_empty() {
            assert!(!rendered.contains(value.trim()));
        }
        assert!(!rendered.contains("secret"));
    }
}

#[test]
fn unknown_provider_suffix_fails_without_echoing_the_variable() {
    let error = ProviderConfig::from_iter([(
        "ABBEY_PROVIDER_CLAUDE_AMBIENT_KEYCHAIN_SESSION",
        "do-not-echo",
    )])
    .expect_err("unsupported setting must fail");
    assert_eq!(
        error.to_string(),
        "an ABBEY_PROVIDER_* variable uses an unsupported setting suffix"
    );
}

#[cfg(unix)]
#[test]
fn non_unicode_provider_values_fail_without_reproducing_bytes() {
    use std::os::unix::ffi::OsStringExt as _;

    let raw = OsString::from_vec(vec![0xff, b's', b'e', b'c', b'r', b'e', b't']);
    let error =
        ProviderConfig::from_iter([(OsString::from("ABBEY_PROVIDER_CLAUDE_CREDENTIAL"), raw)])
            .expect_err("non-Unicode credential must fail");
    assert_eq!(
        error.to_string(),
        "ABBEY_PROVIDER_CLAUDE_CREDENTIAL: value was not Unicode"
    );
}

#[test]
fn normalized_provider_variable_aliases_cannot_overwrite_each_other() {
    let error = ProviderConfig::from_iter([
        ("ABBEY_PROVIDER_FOUNDATION_MODELS_MODEL", "first"),
        ("ABBEY_PROVIDER_FOUNDATION-MODELS_MODEL", "second"),
    ])
    .expect_err("noncanonical alias must fail");
    assert!(
        error
            .to_string()
            .starts_with("ABBEY_PROVIDER_FOUNDATION_MODELS_MODEL")
    );
    assert!(!error.to_string().contains("first"));
    assert!(!error.to_string().contains("second"));
}

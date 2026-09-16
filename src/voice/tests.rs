use super::*;

#[test]
fn music_command_channel_config_is_optional_nonzero_and_requires_voice_scope() {
    for value in [None, Some(""), Some("  ")] {
        let mut values = music_destination();
        values.music_command_channel = value.map(str::to_owned);
        assert_eq!(
            VoiceConfig::from_values(values)
                .unwrap()
                .unwrap()
                .music_command_channel_id,
            None
        );
    }
    let mut values = music_destination();
    values.music_command_channel = Some("1545633393402843236".into());
    assert_eq!(
        VoiceConfig::from_values(values)
            .unwrap()
            .unwrap()
            .music_command_channel_id,
        Some(1545633393402843236)
    );
    for value in ["0", "bad", "18446744073709551616"] {
        let mut values = music_destination();
        values.music_command_channel = Some(value.into());
        assert!(
            VoiceConfig::from_values(values)
                .unwrap_err()
                .contains("ABBEY_MUSIC_COMMAND_CHANNEL_ID")
        );
    }
    assert!(
        VoiceConfig::from_values(VoiceEnv {
            music_command_channel: Some("2".into()),
            ..VoiceEnv::default()
        })
        .unwrap_err()
        .contains("requires ABBEY_VOICE_GUILD_ID")
    );
}

fn destination() -> VoiceEnv {
    VoiceEnv {
        guild: Some("123".into()),
        channel: Some("456".into()),
        ..VoiceEnv::default()
    }
}

// The music channel is parsed before the mode, so it is pinned under
// `disabled`, the one mode every platform accepts. `destination()` alone
// selects `local`, which fails closed off macOS before the channel is
// ever compared.
fn music_destination() -> VoiceEnv {
    VoiceEnv {
        mode: Some("disabled".into()),
        ..destination()
    }
}

#[test]
fn voice_is_off_without_a_destination() {
    assert!(
        VoiceConfig::from_values(VoiceEnv::default())
            .unwrap()
            .is_none()
    );
    let values = VoiceEnv {
        openai_key: Some("unrelated-key".into()),
        ..VoiceEnv::default()
    };
    assert!(VoiceConfig::from_values(values).unwrap().is_none());
}

#[test]
fn explicit_mode_without_a_destination_builds_only_a_dynamic_template() {
    let environment = VoiceEnvironment::from_values(VoiceEnv {
        mode: Some("disabled".into()),
        ..VoiceEnv::default()
    })
    .unwrap()
    .unwrap();
    assert_eq!(environment.template.mode(), VoiceMode::Disabled);
    assert!(environment.default.is_none());
}

#[test]
fn configured_destination_and_dynamic_template_share_one_backend_policy() {
    let environment = VoiceEnvironment::from_values(music_destination())
        .unwrap()
        .unwrap();
    let default = environment.default.unwrap();
    assert_eq!(environment.template.mode(), default.mode());
    assert_eq!(default.guild_id, 123);
    assert_eq!(default.channel_id, 456);
    let dynamic = environment.template.for_destination(789, 987).unwrap();
    assert_eq!(dynamic.mode(), default.mode());
    assert_eq!(dynamic.guild_id, 789);
    assert_eq!(dynamic.channel_id, 987);
    assert_eq!(dynamic.music_command_channel_id, None);
}

#[test]
fn partial_destination_fails_closed() {
    let values = VoiceEnv {
        guild: Some("123".into()),
        ..VoiceEnv::default()
    };
    assert!(
        VoiceConfig::from_values(values)
            .unwrap_err()
            .contains("ABBEY_VOICE_CHANNEL_ID")
    );
}

#[test]
fn destination_defaults_to_local_even_when_a_cloud_key_exists() {
    let mut values = destination();
    values.openai_key = Some("must-not-select-cloud".into());
    let result = VoiceConfig::from_values(values);
    if cfg!(target_os = "macos") {
        let config = result.unwrap().unwrap();
        assert_eq!(config.mode(), VoiceMode::Local);
        assert!(config.openai().is_none());
    } else {
        assert!(result.unwrap_err().contains("supported only on macOS"));
    }
}

#[test]
fn openai_realtime_is_never_retained_even_with_a_key() {
    // OpenAI Realtime was removed: a present OPENAI_API_KEY must not make
    // cloud audio switchable via `/voice mode`.
    let mut values = destination();
    values.openai_key = Some("must-not-retain-cloud".into());
    let result = VoiceConfig::from_values(values);
    if !cfg!(target_os = "macos") {
        assert!(result.unwrap_err().contains("supported only on macOS"));
        return;
    }
    let config = result.unwrap().unwrap();
    assert_eq!(config.mode(), VoiceMode::Local);
    assert!(config.openai().is_none());
    assert!(config.available_openai().is_none());
    assert!(config.backend_for(VoiceMode::OpenAi).is_none());
}

#[test]
fn a_mode_with_no_environment_is_not_switchable_to() {
    // OpenAI Realtime is removed: nothing is switchable to openai.
    let result = VoiceConfig::from_values(destination());
    if !cfg!(target_os = "macos") {
        assert!(result.unwrap_err().contains("supported only on macOS"));
        return;
    }
    let config = result.unwrap().unwrap();
    assert!(config.available_openai().is_none());
    assert!(config.backend_for(VoiceMode::OpenAi).is_none());
    // Disabled is always reachable: it takes no configuration to stop.
    assert!(config.backend_for(VoiceMode::Disabled).is_some());
}

#[test]
fn selecting_openai_is_rejected_at_parse() {
    let mut values = destination();
    values.mode = Some("openai".into());
    values.openai_key = Some("irrelevant".into());
    let error = VoiceConfig::from_values(values).unwrap_err();
    assert!(
        error.contains("removed") || error.contains("openai"),
        "{error}"
    );
}

#[test]
fn a_snapshot_backend_always_agrees_with_its_own_mode() {
    // `start_voice` trusts `backend.mode()` to describe the backend it is
    // about to connect. If those could disagree, the public consent notice
    // could name a different backend than the actor that connects.
    let config = VoiceConfig::selected_only(1, 2, VoiceBackendConfig::Disabled, true);
    assert_eq!(
        config.backend_for(VoiceMode::Disabled).map(|b| b.mode()),
        Some(VoiceMode::Disabled)
    );
}

#[test]
fn a_directly_built_config_retains_nothing() {
    let config = VoiceConfig::selected_only(1, 2, VoiceBackendConfig::Disabled, true);
    assert_eq!(config.mode(), VoiceMode::Disabled);
    assert!(config.available_local().is_none());
    assert!(config.available_openai().is_none());
    assert!(config.backend_for(VoiceMode::Local).is_none());
}

#[test]
fn the_command_parser_accepts_every_environment_alias() {
    // `/voice mode` reuses this parser precisely so the two surfaces cannot
    // drift; a second parser in the command shell rejected off/offline.
    for (input, expected) in [
        ("disabled", VoiceMode::Disabled),
        ("off", VoiceMode::Disabled),
        ("local", VoiceMode::Local),
        ("offline", VoiceMode::Local),
    ] {
        assert_eq!(
            VoiceMode::parse(Some(input.into())).unwrap(),
            expected,
            "{input}"
        );
    }
    let openai_err = VoiceMode::parse(Some("openai".into())).unwrap_err();
    assert!(openai_err.contains("removed"), "{openai_err}");
    assert!(VoiceMode::parse(Some("nonsense".into())).is_err());
}

#[test]
fn openai_mode_cannot_become_effective_without_a_retained_backend() {
    let mut values = destination();
    values.openai_key = Some("must-not-enable-cloud".into());
    let result = VoiceConfig::from_values(values);
    if !cfg!(target_os = "macos") {
        assert!(result.unwrap_err().contains("supported only on macOS"));
        return;
    }
    let runtime = crate::voice_session::VoiceRuntime::new(result.unwrap().unwrap());
    runtime.set_effective_mode(VoiceMode::OpenAi);
    assert!(runtime.config.openai().is_none());
    assert!(
        runtime.effective_backend().is_none(),
        "OpenAI Realtime must not be available as an effective backend"
    );
}

#[test]
fn local_voice_is_rejected_outside_macos() {
    let mut values = destination();
    values.mode = Some("local".into());
    let result = VoiceConfig::from_values(values);
    if cfg!(target_os = "macos") {
        assert_eq!(result.unwrap().unwrap().mode(), VoiceMode::Local);
    } else {
        assert!(result.unwrap_err().contains("supported only on macOS"));
    }
}

#[test]
fn openai_mode_is_rejected_even_with_a_complete_cloud_env() {
    let mut values = destination();
    values.mode = Some("openai".into());
    values.openai_key = Some("super-secret".into());
    values.instructions = Some("PRIVATE_VOICE_INSTRUCTIONS_CANARY".into());
    let error = VoiceConfig::from_values(values).unwrap_err();
    assert!(error.contains("removed"), "{error}");
}

#[test]
fn disabled_mode_needs_no_provider() {
    let mut values = destination();
    values.mode = Some("disabled".into());
    let config = VoiceConfig::from_values(values).unwrap().unwrap();
    assert_eq!(config.mode(), VoiceMode::Disabled);
}

#[test]
fn remote_plaintext_openai_websocket_helper_still_rejects_non_loopback_ws() {
    // Endpoint helper retained for defense-in-depth / historical contracts;
    // mode selection already refuses openai before build_openai runs.
    assert!(
        validate_openai_endpoint("ws://example.com/realtime")
            .unwrap_err()
            .contains("ws only on loopback")
    );
}

#[test]
fn openai_key_can_reach_only_the_exact_official_remote_endpoint() {
    assert!(validate_openai_endpoint("wss://api.openai.com/v1/realtime").is_ok());
    for endpoint in [
        "wss://api.openai.com/v1/realtime?model=attacker",
        "wss://api.openai.com/v1/realtime#fragment",
    ] {
        let error = validate_openai_endpoint(endpoint).unwrap_err();
        assert!(
            error.contains("query, or a fragment"),
            "{endpoint}: {error}"
        );
    }
    for endpoint in [
        "wss://example.com/v1/realtime",
        "wss://api.openai.com.evil.example/v1/realtime",
        "wss://api.openai.com/realtime",
        "wss://api.openai.com/v1/realtime/",
        "wss://api.openai.com:443/v1/realtime",
        "wss://api.openai.com:8443/v1/realtime",
        "WSS://api.openai.com/v1/realtime",
        "wss://@api.openai.com/v1/realtime",
    ] {
        let error = validate_openai_endpoint(endpoint).unwrap_err();
        assert!(
            error.contains("only to wss://api.openai.com/v1/realtime"),
            "{endpoint}: {error}"
        );
    }
}

#[test]
fn loopback_ws_remains_available_for_realtime_test_doubles() {
    for endpoint in [
        "ws://127.0.0.1:8182/realtime",
        "ws://localhost:8182/v1/realtime",
        "ws://[::1]:8182/test",
    ] {
        assert!(validate_openai_endpoint(endpoint).is_ok(), "{endpoint}");
        assert!(
            validate_openai_endpoint_for_build(endpoint, false)
                .unwrap_err()
                .contains("only to the test build"),
            "production path accepted {endpoint}"
        );
    }
}

#[test]
fn operator_env_presence_withholds_values_and_flags_a_local_voice_llm_gap() {
    let presence = OperatorEnvPresence::from_get(|name| match name {
        "DISCORD_TOKEN" => Some("secret-must-not-appear".into()),
        "ABBEY_VOICE_GUILD_ID" => Some("1".into()),
        "ABBEY_VOICE_CHANNEL_ID" => Some("2".into()),
        "ABBEY_BOT_LLM_ENDPOINT" => Some("   ".into()),
        _ => None,
    });
    let rendered = format!("{presence:?}");
    assert!(!rendered.contains("secret"));
    assert!(presence.discord_token);
    assert!(presence.voice_guild_id && presence.voice_channel_id);
    assert!(!presence.llm_endpoint);
    assert!(presence.local_voice_llm_gap(true, false).is_some());
    assert!(presence.local_voice_llm_gap(true, true).is_none());
    assert!(presence.local_voice_llm_gap(false, false).is_none());
}

#[test]
fn wake_names_are_token_bounded_and_case_insensitive() {
    let words = VoiceConfig::default_wake_words();
    assert!(contains_wake_name("Abbey, can you help?", &words));
    assert!(contains_wake_name("Abby, can you help?", &words));
    assert!(contains_wake_name("AVIVA be direct", &words));
    assert!(contains_wake_name("abi: orchestrate", &words));
    assert!(!contains_wake_name("an abbeylike building", &words));
    assert!(!contains_wake_name("ordinary speech", &words));
}

#[test]
fn wake_words_default_when_unset_or_unusable() {
    let default = VoiceConfig::default_wake_words();
    assert_eq!(parse_wake_words(None), default);
    assert_eq!(parse_wake_words(Some("   ".into())), default);
    // Every candidate is rejected, so the guild is not left unaddressable.
    assert_eq!(parse_wake_words(Some("42, !!, ,".into())), default);
    assert_eq!(
        parse_wake_words(Some(format!("{}, abbey", "a".repeat(33)))),
        vec!["abbey".to_string()]
    );
}

#[test]
fn wake_words_are_trimmed_lowercased_and_replace_the_default() {
    assert_eq!(
        parse_wake_words(Some("  Nova , HELIX,nova  ".into())),
        vec!["nova".to_string(), "helix".to_string(), "nova".to_string()]
    );
    // A custom list replaces the default rather than extending it.
    assert!(!contains_wake_name(
        "abbey are you there",
        &parse_wake_words(Some("nova".into()))
    ));
}

#[test]
fn mode_without_destination_fails_unless_disabled() {
    let values = VoiceEnv {
        mode: Some("local".into()),
        ..VoiceEnv::default()
    };
    assert!(VoiceConfig::from_values(values).is_err());
    let values = VoiceEnv {
        mode: Some("disabled".into()),
        ..VoiceEnv::default()
    };
    assert!(VoiceConfig::from_values(values).unwrap().is_none());
}

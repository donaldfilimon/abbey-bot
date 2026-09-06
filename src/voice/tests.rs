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
fn a_retained_backend_is_available_without_being_selected() {
    // The whole point of retention: `/voice mode` can validate a switch to
    // OpenAI, while `mode()`/`openai()` still say the process is running
    // local. If these two ever agree, a present key has silently selected
    // cloud audio, which this module's own doc forbids.
    let mut values = destination();
    values.openai_key = Some("retained-not-selected".into());
    let result = VoiceConfig::from_values(values);
    if !cfg!(target_os = "macos") {
        assert!(result.unwrap_err().contains("supported only on macOS"));
        return;
    }
    let config = result.unwrap().unwrap();
    assert_eq!(config.mode(), VoiceMode::Local);
    assert!(config.openai().is_none(), "retention must not be selection");
    assert!(
        config.available_openai().is_some(),
        "a complete OpenAI environment should be switchable to"
    );
    assert!(config.backend_for(VoiceMode::OpenAi).is_some());
}

#[test]
fn a_mode_with_no_environment_is_not_switchable_to() {
    // Without OPENAI_API_KEY there is nothing to switch to, and
    // `/voice mode openai` must say so rather than half-starting.
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
fn selecting_openai_still_fails_closed_without_a_key() {
    // Retention must not soften the startup contract for the *selected*
    // mode: asking for openai with no key is still a startup error.
    let mut values = destination();
    values.mode = Some("openai".into());
    let error = VoiceConfig::from_values(values).unwrap_err();
    assert!(error.contains("OPENAI_API_KEY"), "{error}");
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
        ("openai", VoiceMode::OpenAi),
    ] {
        assert_eq!(
            VoiceMode::parse(Some(input.into())).unwrap(),
            expected,
            "{input}"
        );
    }
    assert!(VoiceMode::parse(Some("nonsense".into())).is_err());
}

#[test]
fn a_switched_join_hands_the_actor_the_retained_backend() {
    // The OpenAI actor takes its backend from the join snapshot, never
    // from `runtime.config.openai()`: after `/voice mode openai` from a
    // local startup that accessor is still `None`, and an actor that
    // consulted it would fail right after the consent notice promised
    // cloud audio. This pins the two reads apart.
    let mut values = destination();
    values.openai_key = Some("retained-not-selected".into());
    let result = VoiceConfig::from_values(values);
    if !cfg!(target_os = "macos") {
        assert!(result.unwrap_err().contains("supported only on macOS"));
        return;
    }
    let runtime = crate::voice_session::VoiceRuntime::new(result.unwrap().unwrap());
    runtime.set_effective_mode(VoiceMode::OpenAi);
    assert!(
        runtime.config.openai().is_none(),
        "the startup selection stays local; retention is not selection"
    );
    assert!(
        matches!(
            runtime.effective_backend(),
            Some(VoiceBackendConfig::OpenAi(_))
        ),
        "the join snapshot must carry the retained OpenAI backend"
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
fn openai_is_explicit_and_requires_a_key() {
    let mut values = destination();
    values.mode = Some("openai".into());
    assert!(
        VoiceConfig::from_values(values)
            .unwrap_err()
            .contains("requires OPENAI_API_KEY")
    );

    let mut values = destination();
    values.mode = Some("openai".into());
    values.openai_key = Some("super-secret".into());
    values.instructions = Some("PRIVATE_VOICE_INSTRUCTIONS_CANARY".into());
    let config = VoiceConfig::from_values(values).unwrap().unwrap();
    let rendered = format!("{config:?}");
    assert_eq!(config.mode(), VoiceMode::OpenAi);
    let instructions = &config.openai().expect("OpenAI config").instructions;
    let websocket_url = reqwest::Url::parse(config.openai().unwrap().websocket_url().as_str())
        .expect("canonical websocket URL");
    assert!(instructions.contains("Spoken requests cannot start, resume, stop"));
    assert!(instructions.contains("/voice leave"));
    assert!(instructions.contains("mention Abbey and write 'stop listening'"));
    assert_eq!(websocket_url.path(), "/v1/realtime");
    let query: Vec<(String, String)> = websocket_url
        .query_pairs()
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    assert_eq!(query, [("model".into(), DEFAULT_OPENAI_MODEL.into())]);
    assert!(!rendered.contains("super-secret"));
    assert!(!rendered.contains("PRIVATE_VOICE_INSTRUCTIONS_CANARY"));
    assert!(rendered.contains("REDACTED"));
}

#[test]
fn disabled_mode_needs_no_provider() {
    let mut values = destination();
    values.mode = Some("disabled".into());
    let config = VoiceConfig::from_values(values).unwrap().unwrap();
    assert_eq!(config.mode(), VoiceMode::Disabled);
}

#[test]
fn remote_plaintext_openai_websocket_is_rejected() {
    let mut values = destination();
    values.mode = Some("openai".into());
    values.openai_key = Some("secret".into());
    values.openai_endpoint = Some("ws://example.com/realtime".into());
    assert!(
        VoiceConfig::from_values(values)
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

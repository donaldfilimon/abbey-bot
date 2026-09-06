use super::*;

fn queued_command_state() -> std::sync::Arc<AppState> {
    use crate::tools::ToolHost as _;
    let mut state = AppState::in_memory();
    let missing = std::env::temp_dir().join(format!(
        "abbey-command-drain-{}-missing-abi",
        std::process::id()
    ));
    assert!(!missing.exists(), "synthetic ABI path must be absent");
    let config = serde_json::json!({
        "abi_cli": missing,
        "endpoint": "http://127.0.0.1:50051",
        "token_file": std::env::temp_dir().join("abbey-command-drain-unused-token"),
        "policy_version": "policy_v1",
        "contract_revision": 2,
        "contract_digest": "01".repeat(32),
        "timeout_secs": 5,
        "guilds": ["discord:123"]
    });
    std::sync::Arc::get_mut(&mut state).unwrap().episode_gate =
        Some(std::sync::Arc::new(crate::episode_gate::EpisodeGate::new(
            crate::episode_gate::EpisodeGateConfig::from_json(&config.to_string()).unwrap(),
        )));
    let mut host = runtime::ToolScope {
        state: &state,
        network: crate::platform::SocialNetwork::Discord,
        scoped_guild: "discord:123".into(),
        scoped_user: "discord:42".into(),
        scoped_channel: "discord:channel".into(),
        now: 10,
        persona: Persona::Abbey,
    };
    assert!(
        host.remember_fact("likes compilers", None)
            .starts_with("Queued")
    );
    state
}

#[tokio::test]
async fn generated_reply_finishes_delivery_before_draining_real_gated_queue() {
    let state = queued_command_state();
    let (sent, received) = tokio::sync::oneshot::channel();
    let completion = deliver_generated_reply(&state, async {
        received.await.unwrap();
        Ok::<_, ()>("delivered")
    });
    tokio::pin!(completion);
    // Drive the actual production helper into its pending delivery await.
    tokio::select! {
        biased;
        _ = &mut completion => panic!("delivery has not completed"),
        _ = std::future::ready(()) => {}
    }
    assert_eq!(AppState::lock(&state.memory_queue).len(), 1);
    assert!(
        state
            .memory_service()
            .facts("discord:123", "discord:42")
            .is_empty()
    );
    sent.send(()).unwrap();
    assert_eq!(completion.await, Ok("delivered"));
    // The unavailable fake gate refuses: the queue is handled after the
    // reply, but delivering a reply does not imply any fact was stored.
    assert!(AppState::lock(&state.memory_queue).is_empty());
    assert!(
        state
            .memory_service()
            .facts("discord:123", "discord:42")
            .is_empty()
    );
}

#[tokio::test]
async fn failed_generated_reply_preserves_queue_and_stores_nothing() {
    let state = queued_command_state();
    let result = deliver_generated_reply(&state, async { Err::<(), _>("delivery failed") }).await;
    assert_eq!(result, Err("delivery failed"));
    assert_eq!(AppState::lock(&state.memory_queue).len(), 1);
    assert!(
        state
            .memory_service()
            .facts("discord:123", "discord:42")
            .is_empty()
    );
}

#[test]
fn choice_mirrors_map_onto_their_pure_types() {
    assert_eq!(
        Archetype::from(ArchetypeChoice::Community),
        Archetype::Community
    );
    assert_eq!(Archetype::from(ArchetypeChoice::Gaming), Archetype::Gaming);
    assert_eq!(
        Archetype::from(ArchetypeChoice::Project),
        Archetype::Project
    );
    assert_eq!(
        Archetype::from(ArchetypeChoice::FriendGroup),
        Archetype::FriendGroup
    );
    assert_eq!(Severity::from(SeverityChoice::Minor), Severity::Minor);
    assert_eq!(Severity::from(SeverityChoice::Serious), Severity::Serious);
    assert_eq!(Severity::from(SeverityChoice::Severe), Severity::Severe);
    assert_eq!(Persona::from(PersonaChoice::Abbey), Persona::Abbey);
    assert_eq!(Persona::from(PersonaChoice::Aviva), Persona::Aviva);
    assert_eq!(Persona::from(PersonaChoice::Abi), Persona::Abi);
}

#[test]
fn the_ladders_permission_strings_all_exist_in_serenity() {
    // Iterates the real Action values rather than restating literals; a
    // mutation check showed a literal-pinning version passed even when the
    // ladder and the expectation were renamed in lockstep.
    let vocabulary = permission_names(Permissions::all());
    for action in [
        moderation::Action::Timeout(10),
        moderation::Action::Kick,
        moderation::Action::Ban,
    ] {
        let name = action
            .required_permission()
            .expect("these three all require a permission");
        assert!(
            vocabulary.iter().any(|known| known == name),
            "{action} names {name:?}, which serenity does not define"
        );
    }
}

#[test]
fn every_permission_a_blueprint_names_exists_in_serenity() {
    // Blueprints hand out permission names as prose — now including the
    // per-archetype @everyone grants. A name serenity does not recognise
    // means a step nobody can follow.
    let vocabulary: Vec<String> = permission_names(Permissions::all());
    for archetype in Archetype::ALL {
        let bp = server::blueprint(archetype);
        let named = bp
            .roles
            .iter()
            .flat_map(|role| role.permissions.iter())
            .chain(bp.everyone.iter())
            .chain(
                bp.categories
                    .iter()
                    .flat_map(|category| category.channels)
                    .flat_map(|channel| channel.deny_everyone.iter()),
            );
        for permission in named {
            assert!(
                vocabulary.iter().any(|known| known == permission),
                "{archetype:?} names {permission:?}, which serenity does not define"
            );
        }
    }
}

#[test]
fn clamp_passes_short_messages_untouched() {
    let short = "fits".to_string();
    assert_eq!(clamp_message(short.clone()), short);
}

#[test]
fn clamp_bounds_long_messages_at_discords_limit() {
    // Multibyte input, because the limit is codepoints, not bytes.
    let long: String = "é".repeat(2500);
    let out = clamp_message(long);
    assert!(out.chars().count() <= 2000, "{}", out.chars().count());
    assert!(
        out.ends_with("limit)"),
        "truncation must be stated, not silent"
    );
}

#[tokio::test]
async fn a_five_thousand_char_backend_answer_clamps_to_discords_limit() {
    // The ask pipeline end to end with a recording fake: a 5,000-character
    // response comes back ≤ 2,000 codepoints through the existing clamp —
    // the same `clamp_message` every reply already routes through.
    // Multibyte input, because the limit is codepoints, not bytes.
    let backend = llm::Backend::OpenAiCompatible {
        endpoint: "http://127.0.0.1:8080".into(),
        model: "default".into(),
    };
    let long_answer = "é".repeat(5000);
    let canned = serde_json::json!({
        "choices": [{"message": {"content": long_answer}, "finish_reason": "stop"}]
    })
    .to_string();
    let transport = llm::RecordingTransport::returning(&canned);

    let answer = llm::ask_backend(
        &transport,
        &backend,
        &ask::system_prompt(Persona::Abbey),
        "a question",
    )
    .await
    .expect("the canned response parses");
    assert_eq!(
        answer.chars().count(),
        5000,
        "the fake answer arrives whole"
    );

    let reply = clamp_message(ask::render_answer(Persona::Abbey, backend.label(), &answer));
    assert!(reply.chars().count() <= 2000, "{}", reply.chars().count());
    assert!(
        reply.ends_with("limit)"),
        "truncation must be stated, not silent"
    );
}

#[cfg(any())]
#[test]
fn no_audio_songbird_config_disables_decryption_and_decoding() {
    let base = songbird::Config::default().decode_mode(songbird::driver::DecodeMode::Decode(
        songbird::driver::DecodeConfig::default(),
    ));
    let no_audio = no_audio_songbird_config(&base);

    assert_eq!(no_audio.decode_mode, songbird::driver::DecodeMode::Pass);
    assert!(matches!(
        base.decode_mode,
        songbird::driver::DecodeMode::Decode(_)
    ));
}

#[cfg(any())]
#[tokio::test]
async fn songbird_raw_pcm_input_has_a_registered_decoder() {
    let samples = vec![0_u8; 480 * 2 * std::mem::size_of::<f32>()];
    let input: songbird::input::Input =
        RawAdapter::new(std::io::Cursor::new(samples), 48_000, 2).into();
    let playable = input
        .make_playable_async(
            songbird::input::codecs::get_codec_registry(),
            songbird::input::codecs::get_probe(),
        )
        .await
        .expect("RawAdapter f32 PCM must be decodable by the deployed registry");
    assert!(playable.is_playable());
}

#[cfg(any())]
#[tokio::test]
async fn realtime_websocket_exchanges_session_input_and_output_audio() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback fake");
    let address = listener.local_addr().expect("fake address");
    let config = VoiceConfig::from_values(
        Some("123".into()),
        Some("456".into()),
        Some("test-secret".into()),
        Some(format!("ws://{address}/realtime")),
        Some("test-model".into()),
        Some("marin".into()),
        Some("Be Abbey.".into()),
    )
    .expect("valid test config")
    .expect("voice enabled");
    let runtime = Arc::new(VoiceRuntime::new(config));
    let generation = runtime.begin();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept fake client");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("accept websocket");
        let session = socket
            .next()
            .await
            .expect("session event")
            .expect("session frame")
            .into_text()
            .expect("session text");
        let session: serde_json::Value = serde_json::from_str(&session).expect("session JSON");
        assert_eq!(session["type"], "session.update");
        assert_eq!(session["session"]["model"], "test-model");
        assert_eq!(
            session["session"]["audio"]["input"]["format"]["rate"],
            24_000
        );
        socket
            .send(Message::Text(
                serde_json::json!({"type": "session.updated"})
                    .to_string()
                    .into(),
            ))
            .await
            .expect("session acknowledgement");

        let input = socket
            .next()
            .await
            .expect("input event")
            .expect("input frame")
            .into_text()
            .expect("input text");
        let input: serde_json::Value = serde_json::from_str(&input).expect("input JSON");
        assert_eq!(input["type"], "input_audio_buffer.append");
        assert!(
            input["audio"]
                .as_str()
                .is_some_and(|audio| !audio.is_empty())
        );

        socket
                .send(Message::Text(
                    serde_json::json!({
                        "type": "response.output_audio.delta",
                        "delta": base64::engine::general_purpose::STANDARD.encode(32767_i16.to_le_bytes())
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("output delta");
        while let Some(Ok(message)) = socket.next().await {
            if message.is_close() {
                break;
            }
        }
    });

    let (input_tx, input_rx) = tokio::sync::mpsc::channel(2);
    let (output_tx, output_rx) = std::sync::mpsc::sync_channel(2);
    let client_runtime = Arc::clone(&runtime);
    let client = tokio::spawn(async move {
        run_realtime(&client_runtime, generation, input_rx, output_tx).await
    });
    input_tx
        .send(vec![1000; 480])
        .await
        .expect("send fake voice tick");
    let output = tokio::task::spawn_blocking(move || {
        output_rx.recv_timeout(std::time::Duration::from_secs(5))
    })
    .await
    .expect("output wait task")
    .expect("receive converted audio");
    assert_eq!(
        output.len(),
        16,
        "one mono sample becomes two stereo frames"
    );
    assert_eq!(runtime.status(), "live; listening and speaking");

    drop(input_tx);
    client.await.expect("client task").expect("client exit");
    server.await.expect("server task");
}

#[test]
fn ask_has_atomic_per_user_cost_control() {
    let state = AppState::in_memory();
    assert!(reserve_ask(&state, "discord:u1", 100));
    assert!(!reserve_ask(&state, "discord:u1", 129));
    assert!(reserve_ask(&state, "discord:u2", 129));
    assert!(reserve_ask(&state, "discord:u1", 130));
}

#[test]
fn empty_permissions_render_as_nothing_not_as_a_placeholder() {
    assert!(permission_names(Permissions::empty()).is_empty());
}

#[test]
fn permission_names_are_humanised() {
    let names = permission_names(Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES);
    assert!(names.iter().any(|n| n == "View Channel"), "{names:?}");
    assert!(names.iter().any(|n| n == "Send Messages"), "{names:?}");
}

#[test]
fn a_single_permission_still_splits_cleanly() {
    assert_eq!(
        permission_names(Permissions::BAN_MEMBERS),
        vec!["Ban Members"]
    );
}

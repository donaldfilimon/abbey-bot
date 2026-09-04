use super::*;

#[tokio::test]
async fn spoken_reply_policy_only_adds_reasoning_control_to_the_qualified_request() {
    for endpoint in [
        "http://127.0.0.1:11434",
        "http://localhost:11434/",
        "http://[::1]:11434",
    ] {
        let backend = Backend::OpenAiCompatible {
            endpoint: endpoint.into(),
            model: "gemma4:12b".into(),
        };
        let turns = [ChatTurn::user("Abby, hello.")];
        let default_request =
            build_chat_request_with_tools(&backend, "Persona and grounding", &turns, &[]);
        let transport = RecordingTransport::returning(
            r#"{"choices":[{"message":{"content":"Hello."},"finish_reason":"stop"}]}"#,
        );
        let reply = chat_turn_with_style(
            &transport,
            &backend,
            "Persona and grounding",
            &turns,
            &[],
            ResponseStyle::Spoken,
        )
        .await
        .unwrap();
        assert_eq!(reply.text, "Hello.");
        let mut spoken_request = transport.recorded();
        assert_eq!(
            spoken_request
                .body
                .as_object_mut()
                .unwrap()
                .remove("reasoning_effort"),
            Some(json!("none"))
        );
        assert_eq!(spoken_request, default_request);

        chat_turn(&transport, &backend, "Persona and grounding", &turns, &[])
            .await
            .unwrap();
        assert_eq!(transport.recorded(), default_request);
    }
}

#[tokio::test]
async fn spoken_reply_policy_preserves_other_backends_and_tool_requests() {
    let local = |endpoint: &str, model: &str| Backend::OpenAiCompatible {
        endpoint: endpoint.into(),
        model: model.into(),
    };
    let cases = [
        (local("http://127.0.0.1:11434", "gemma4:12b"), true),
        (local("http://127.0.0.1:11434", "another-model"), false),
        (local("http://127.0.0.1:8181", "gemma4:12b"), false),
        (local("https://example.test:11434", "gemma4:12b"), false),
        (local("http://127.0.0.1:11434/proxy", "gemma4:12b"), false),
        (
            Backend::Anthropic {
                api_key: "synthetic-key".into(),
            },
            false,
        ),
    ];
    for (backend, offer_tools) in cases {
        let tools = if offer_tools {
            crate::tools::production_tools()
        } else {
            Vec::new()
        };
        let turns = [ChatTurn::user("Synthetic question")];
        let expected =
            build_chat_request_with_tools(&backend, "Persona and grounding", &turns, &tools);
        let canned_reply = match &backend {
            Backend::Anthropic { .. } => {
                r#"{"content":[{"type":"text","text":"Hello."}],"stop_reason":"end_turn"}"#
            }
            Backend::OpenAiCompatible { .. } => {
                r#"{"choices":[{"message":{"content":"Hello."},"finish_reason":"stop"}]}"#
            }
        };
        let transport = RecordingTransport::returning(canned_reply);
        chat_turn_with_style(
            &transport,
            &backend,
            "Persona and grounding",
            &turns,
            &tools,
            ResponseStyle::Spoken,
        )
        .await
        .unwrap();
        assert_eq!(transport.recorded(), expected);
    }
}

#[test]
fn debug_never_prints_the_api_key() {
    // A derived Debug on either type prints the credential in full. These
    // assertions fail the moment someone re-derives it — which is the only
    // way this protection stays real, since the leak paths (tracing fields,
    // panic messages, CI assertion output) are all invisible until they fire.
    const SECRET: &str = "sk-ant-super-secret-value";

    let backend = Backend::Anthropic {
        api_key: SECRET.to_string(),
    };
    let shown = format!("{backend:?}");
    assert!(!shown.contains(SECRET), "backend leaked the key: {shown}");
    assert!(shown.contains("<redacted>"), "{shown}");

    const SYSTEM_CANARY: &str = "PRIVATE_SYSTEM_PROMPT_CANARY";
    const MESSAGE_CANARY: &str = "PRIVATE_MESSAGE_CANARY";
    let request = build_request(&backend, SYSTEM_CANARY, MESSAGE_CANARY);
    let shown = format!("{request:?}");
    assert!(!shown.contains(SECRET), "request leaked the key: {shown}");
    assert!(
        !shown.contains(SYSTEM_CANARY),
        "request leaked its prompt: {shown}"
    );
    assert!(
        !shown.contains(MESSAGE_CANARY),
        "request leaked its message: {shown}"
    );
    assert!(shown.contains("<redacted>"), "{shown}");
    // Non-secret fields must still be legible, or the redaction has made
    // the type useless for the debugging it exists to serve.
    assert!(shown.contains("api.anthropic.com"), "{shown}");
}

#[test]
fn the_local_model_name_comes_from_the_env_and_defaults_honestly() {
    let named = Backend::from_values(
        None,
        Some("http://127.0.0.1:11434".into()),
        Some(" gemma4:26b ".into()),
    )
    .expect("selected");
    assert_eq!(
        named,
        Backend::OpenAiCompatible {
            endpoint: "http://127.0.0.1:11434".into(),
            model: "gemma4:26b".into()
        }
    );
    let unnamed = Backend::from_values(
        None,
        Some("http://127.0.0.1:8080".into()),
        Some("  ".into()),
    )
    .expect("selected");
    assert!(
        matches!(unnamed, Backend::OpenAiCompatible { model, .. } if model == DEFAULT_LOCAL_MODEL)
    );
    let request = build_request(&named, "S", "Q");
    assert_eq!(request.body["model"], "gemma4:26b");
    assert_eq!(request.body["max_tokens"], LOCAL_MAX_TOKENS);
}

#[test]
fn stream_request_sets_stream_only_on_the_local_path() {
    let local = Backend::OpenAiCompatible {
        endpoint: "http://127.0.0.1:11434".into(),
        model: "gemma4:e4b".into(),
    };
    assert_eq!(
        build_stream_request(&local, "S", &[ChatTurn::user("Q")], &[]).body["stream"],
        true
    );
    let anthropic = Backend::Anthropic {
        api_key: "k".into(),
    };
    assert!(
        build_stream_request(&anthropic, "S", &[ChatTurn::user("Q")], &[])
            .body
            .get("stream")
            .is_none()
    );
}

#[test]
fn tooled_requests_serialize_both_shapes_and_untooled_is_byte_identical() {
    let tools = crate::tools::abbey_tools();
    let call = crate::tools::ToolCall {
        id: "c1".into(),
        name: "recall".into(),
        arguments: json!({"query": "rust"}),
    };
    let result = crate::tools::ToolResult {
        call_id: "c1".into(),
        name: "recall".into(),
        content: "• nightly".into(),
    };
    let turns = vec![
        ChatTurn::user("what do you remember?"),
        ChatTurn::assistant_calls("", vec![call]),
        ChatTurn::tool_result(&result),
    ];
    let local = Backend::OpenAiCompatible {
        endpoint: "http://127.0.0.1:11434".into(),
        model: "gpt-oss:20b".into(),
    };
    let body = build_chat_request_with_tools(&local, "S", &turns, &tools).body;
    assert_eq!(body["tools"].as_array().unwrap().len(), 5);
    assert_eq!(
        body["messages"][2]["tool_calls"][0]["function"]["name"],
        "recall"
    );
    assert_eq!(
        body["messages"][2]["tool_calls"][0]["function"]["arguments"],
        "{\"query\":\"rust\"}"
    );
    assert_eq!(body["messages"][3]["role"], "tool");
    assert_eq!(body["messages"][3]["tool_call_id"], "c1");
    let anthropic = Backend::Anthropic {
        api_key: "k".into(),
    };
    let body = build_chat_request_with_tools(&anthropic, "S", &turns, &tools).body;
    assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
    assert_eq!(body["messages"][2]["role"], "user");
    assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
    assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "c1");
    // No tools → the classic body, byte for byte.
    let plain = [ChatTurn::user("Q")];
    assert_eq!(
        build_chat_request(&local, "S", &plain),
        build_chat_request_with_tools(&local, "S", &plain, &[])
    );
    assert!(
        build_chat_request(&local, "S", &plain)
            .body
            .get("tools")
            .is_none()
    );
}

#[test]
fn extract_turn_reads_calls_and_text_on_both_backends() {
    let local = Backend::OpenAiCompatible {
        endpoint: "e".into(),
        model: "m".into(),
    };
    let raw = r#"{"choices":[{"message":{"role":"assistant","content":"","tool_calls":[{"id":"call_1","type":"function","function":{"name":"remember_fact","arguments":"{\"fact\":\"x\"}"}}]},"finish_reason":"tool_calls"}]}"#;
    let turn = extract_turn(&local, raw).unwrap();
    assert_eq!(turn.calls.len(), 1);
    assert_eq!(turn.calls[0].arguments["fact"], "x");
    let anthropic = Backend::Anthropic {
        api_key: "k".into(),
    };
    let raw = r#"{"content":[{"type":"text","text":"Sure."},{"type":"tool_use","id":"t1","name":"recall","input":{"query":"q"}}],"stop_reason":"tool_use"}"#;
    let turn = extract_turn(&anthropic, raw).unwrap();
    assert_eq!(turn.text, "Sure.");
    assert_eq!(turn.calls[0].name, "recall");
    // Empty both ways falls back to extract_text's honest error.
    assert!(
        extract_turn(
            &local,
            r#"{"choices":[{"message":{"content":""},"finish_reason":"stop"}]}"#
        )
        .is_err()
    );
}

#[test]
fn sse_accumulator_merges_streamed_tool_calls_whole_and_fragmented() {
    let mut acc = SseAccumulator::default();
    acc.feed(b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"id\":\"call_a\",\"type\":\"function\",\"index\":0,\"function\":{\"name\":\"recall\",\"arguments\":\"{\\\"query\\\":\\\"ru\"}}]},\"finish_reason\":null}]}\n").unwrap();
    acc.feed(b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"st\\\"}\"}}]},\"finish_reason\":null}]}\n").unwrap();
    acc.feed(
        b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\ndata: [DONE]\n",
    )
    .unwrap();
    acc.finish().unwrap();
    let calls = acc.tool_calls().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_a");
    assert_eq!(calls[0].arguments["query"], "rust");
}

#[test]
fn sse_accumulator_handles_chunks_split_mid_line_and_done() {
    let mut acc = SseAccumulator::default();
    let a = acc.feed(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"del",
        ).unwrap();
    assert_eq!(a, vec!["Hel".to_string()]);
    let b = acc.feed(
            b"ta\":{\"content\":\"lo\"},\"finish_reason\":null}]}\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n",
        ).unwrap();
    assert_eq!(b, vec!["lo".to_string()]);
    assert!(!acc.is_done());
    let c = acc.feed(b"data: [DONE]\n").unwrap();
    assert!(c.is_empty());
    assert!(acc.is_done());
    acc.finish().unwrap();
}

#[test]
fn completed_responses_bound_calls_and_reject_truncation() {
    let local = Backend::OpenAiCompatible {
        endpoint: "e".into(),
        model: "m".into(),
    };
    let calls: Vec<Value> = (0..=MAX_TOOL_CALLS_PER_TURN)
        .map(|index| {
            json!({
                "id": format!("c{index}"),
                "type": "function",
                "function": {"name": "recall", "arguments": "{}"},
            })
        })
        .collect();
    let raw = json!({
        "choices": [{
            "message": {"content": "", "tool_calls": calls},
            "finish_reason": "tool_calls",
        }]
    })
    .to_string();
    assert!(extract_turn(&local, &raw).is_err());
    assert!(
        extract_turn(
            &local,
            r#"{"choices":[{"message":{"content":"partial"},"finish_reason":"length"}]}"#
        )
        .is_err()
    );

    let anthropic = Backend::Anthropic {
        api_key: "k".into(),
    };
    assert!(
        extract_turn(
            &anthropic,
            r#"{"content":[{"type":"text","text":"partial"}],"stop_reason":"max_tokens"}"#
        )
        .is_err()
    );
    assert!(extract_turn(&anthropic, r#"{"content":[{"type":"text","text":"done"}]}"#).is_err());
    let content: Vec<Value> = (0..=MAX_TOOL_CALLS_PER_TURN)
        .map(|index| {
            json!({
                "type": "tool_use",
                "id": format!("t{index}"),
                "name": "recall",
                "input": {"query": "q"},
            })
        })
        .collect();
    let raw = json!({"content": content, "stop_reason": "tool_use"}).to_string();
    assert!(extract_turn(&anthropic, &raw).is_err());
}

#[test]
fn timeout_override_parses_and_falls_back() {
    assert_eq!(timeout_from_value(None), DEFAULT_TIMEOUT_SECS);
    assert_eq!(timeout_from_value(Some(" 45 ".into())), 45);
    assert_eq!(timeout_from_value(Some("0".into())), DEFAULT_TIMEOUT_SECS);
    assert_eq!(
        timeout_from_value(Some("soon".into())),
        DEFAULT_TIMEOUT_SECS
    );
}

#[test]
fn a_reasoning_only_response_is_named_as_such() {
    let backend = Backend::OpenAiCompatible {
        endpoint: "http://127.0.0.1:11434".into(),
        model: "gemma4:26b".into(),
    };
    let raw = r#"{"choices":[{"message":{"role":"assistant","content":"","reasoning":"thinking hard"},"finish_reason":"stop"}]}"#;
    let err = extract_text(&backend, raw).expect_err("no answer");
    assert!(err.detail().contains("reasoning"), "{err}");
    assert_eq!(err.kind(), LlmErrorKind::ResponseBudget);
    let plain =
        r#"{"choices":[{"message":{"role":"assistant","content":""},"finish_reason":"stop"}]}"#;
    assert_eq!(
        extract_text(&backend, plain)
            .expect_err("no answer")
            .detail(),
        "the response carried no answer text"
    );
}

#[test]
fn a_key_bearing_backend_is_not_confused_with_a_loopback_one() {
    let local = Backend::OpenAiCompatible {
        endpoint: "http://127.0.0.1:8080".to_string(),
        model: "gemma4:26b".to_string(),
    };
    let shown = format!("{local:?}");
    // The loopback endpoint is not a secret and stays visible.
    assert!(shown.contains("127.0.0.1:8080"), "{shown}");
    assert!(!shown.contains("<redacted>"), "{shown}");
}

#[test]
fn no_env_values_selects_no_backend_so_the_suite_needs_no_network() {
    // The gate rule, in code: the suite runs with no env vars and no
    // network. With neither variable set, selection yields no backend, so
    // `/persona ask` resolves to the degradation reply without any
    // transport existing at all — there is no code path from "no env" to
    // the network.
    assert_eq!(Backend::from_values(None, None, None), None);
}

#[test]
fn blank_env_values_count_as_unset() {
    // `.env.example` ships blank assignments; copying it unfilled must not
    // select a backend that cannot work.
    assert_eq!(
        Backend::from_values(Some("  ".into()), Some(String::new()), None),
        None
    );
}

#[test]
fn endpoint_policy_allows_https_and_loopback_http_only() {
    let backend = |endpoint: &str| Backend::OpenAiCompatible {
        endpoint: endpoint.into(),
        model: "m".into(),
    };
    assert!(backend("https://models.example.com").validate().is_ok());
    assert!(backend("http://127.0.0.1:11434").validate().is_ok());
    assert!(backend("http://[::1]:11434").validate().is_ok());
    assert!(backend("http://models.example.com").validate().is_err());
    assert!(
        backend("https://user:secret@models.example.com")
            .validate()
            .is_err()
    );
    assert!(
        backend("https://models.example.com?token=secret")
            .validate()
            .is_err()
    );
    assert!(backend("file:///tmp/model").validate().is_err());
}

#[test]
fn live_transport_routes_every_loopback_shape_to_the_no_proxy_client() {
    let transport = HttpTransport::default();
    for endpoint in [
        "http://127.0.0.1:11434/v1/chat/completions",
        "http://localhost:8080/v1/chat/completions",
        "http://[::1]:8181/v1/chat/completions",
    ] {
        assert!(std::ptr::eq(
            transport.client_for(endpoint),
            &transport.loopback_client
        ));
    }
    assert!(std::ptr::eq(
        transport.client_for("https://api.anthropic.com/v1/messages"),
        &transport.remote_client
    ));
}

#[test]
fn anthropic_wins_when_both_backends_are_configured() {
    let backend = Backend::from_values(
        Some("key".into()),
        Some("http://127.0.0.1:8080".into()),
        None,
    )
    .expect("a backend is selected");
    assert!(matches!(backend, Backend::Anthropic { .. }));
}

#[tokio::test]
async fn recording_transport_pins_the_exact_anthropic_request_shape() {
    // Never a live request in tests: the recording fake captures precisely
    // what a call would send, and the wire contract is pinned as literals —
    // URL, the x-api-key + anthropic-version header pair, model, body.
    let backend = Backend::Anthropic {
        api_key: "test-key-not-real".into(),
    };
    let transport = RecordingTransport::returning(
        r#"{"content":[{"type":"text","text":"the answer"}],"stop_reason":"end_turn"}"#,
    );

    let answer = ask_backend(&transport, &backend, "SYSTEM PROMPT", "the question")
        .await
        .expect("the canned response parses");
    assert_eq!(answer, "the answer");

    let request = transport.recorded();
    assert_eq!(request.url, "https://api.anthropic.com/v1/messages");
    assert_eq!(
        request.headers,
        vec![
            ("x-api-key", "test-key-not-real".to_string()),
            ("anthropic-version", "2023-06-01".to_string()),
        ]
    );
    assert_eq!(
        request.body,
        json!({
            "model": "claude-sonnet-5",
            "max_tokens": 1024,
            "thinking": {"type": "disabled"},
            "system": "SYSTEM PROMPT",
            "messages": [{"role": "user", "content": "the question"}],
        })
    );
}

#[tokio::test]
async fn recording_transport_pins_the_openai_compatible_request_shape() {
    let backend = Backend::OpenAiCompatible {
        // Trailing slash on purpose: the join must not produce `//v1`.
        endpoint: "http://127.0.0.1:8080/".into(),
        model: "default".into(),
    };
    let transport = RecordingTransport::returning(
        r#"{"choices":[{"message":{"content":"local answer"},"finish_reason":"stop"}]}"#,
    );

    let answer = ask_backend(&transport, &backend, "SYSTEM PROMPT", "the question")
        .await
        .expect("the canned response parses");
    assert_eq!(answer, "local answer");

    let request = transport.recorded();
    assert_eq!(request.url, "http://127.0.0.1:8080/v1/chat/completions");
    assert!(request.headers.is_empty(), "loopback sends no auth headers");
    assert_eq!(
        request.body,
        json!({
            "model": "default",
            "max_tokens": 4096,
            "messages": [
                {"role": "system", "content": "SYSTEM PROMPT"},
                {"role": "user", "content": "the question"},
            ],
        })
    );
}

#[test]
fn extraction_refuses_a_response_with_no_text() {
    // An empty content array is what a refusal looks like; it must surface
    // as an error, never as an empty string presented as an answer.
    let backend = Backend::Anthropic {
        api_key: "k".into(),
    };
    assert!(extract_text(&backend, r#"{"content":[]}"#).is_err());
    assert!(extract_text(&backend, "not json at all").is_err());
}

#[test]
fn extraction_reads_the_first_text_block_only() {
    let backend = Backend::Anthropic {
        api_key: "k".into(),
    };
    let raw = r#"{"content":[{"type":"thinking","thinking":"…"},{"type":"text","text":"visible"}],"stop_reason":"end_turn"}"#;
    assert_eq!(extract_text(&backend, raw).expect("parses"), "visible");
}

#[test]
fn chat_request_keeps_system_top_level_and_alternates_on_anthropic() {
    let backend = Backend::Anthropic {
        api_key: "k".into(),
    };
    let turns = [
        ChatTurn::user("q1"),
        ChatTurn::assistant("a1"),
        ChatTurn::user("q2"),
    ];
    let request = build_chat_request(&backend, "SYS", &turns);
    assert_eq!(request.body["system"], "SYS");
    assert_eq!(
        request.body["messages"],
        json!([
            {"role": "user", "content": "q1"},
            {"role": "assistant", "content": "a1"},
            {"role": "user", "content": "q2"},
        ])
    );
}

#[test]
fn chat_request_puts_system_first_on_openai_compatible() {
    let backend = Backend::OpenAiCompatible {
        endpoint: "http://127.0.0.1:8080".into(),
        model: "default".into(),
    };
    let turns = [
        ChatTurn::user("q1"),
        ChatTurn::assistant("a1"),
        ChatTurn::user("q2"),
    ];
    let request = build_chat_request(&backend, "SYS", &turns);
    assert!(request.body.get("system").is_none(), "no top-level system");
    assert_eq!(
        request.body["messages"],
        json!([
            {"role": "system", "content": "SYS"},
            {"role": "user", "content": "q1"},
            {"role": "assistant", "content": "a1"},
            {"role": "user", "content": "q2"},
        ])
    );
}

#[test]
fn single_question_request_is_the_one_turn_chat_request() {
    for backend in [
        Backend::Anthropic {
            api_key: "k".into(),
        },
        Backend::OpenAiCompatible {
            endpoint: "http://127.0.0.1:1".into(),
            model: "default".into(),
        },
    ] {
        assert_eq!(
            build_request(&backend, "S", "Q"),
            build_chat_request(&backend, "S", &[ChatTurn::user("Q")])
        );
    }
}

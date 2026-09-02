use super::*;
use crate::pipeline::testing::FakeOut;

/// A streaming transport that replays canned deltas with small pauses.
struct FakeStream {
    deltas: Vec<&'static str>,
    fail_at_end: bool,
    calls: Vec<crate::tools::ToolCall>,
}

impl llm::StreamTransport for FakeStream {
    async fn post_stream(
        &self,
        _request: &llm::LlmRequest,
        on_delta: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<llm::ModelTurn, llm::LlmError> {
        let mut full = String::new();
        for d in &self.deltas {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            full.push_str(d);
            let _ = on_delta.send((*d).to_string());
        }
        if self.fail_at_end {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            return Err(llm::LlmError::backend("upstream died".into()));
        }
        Ok(llm::ModelTurn {
            text: full,
            calls: self.calls.clone(),
        })
    }
}

fn prepared() -> crate::engine::PreparedTurn {
    crate::engine::Engine::new().prepare("c1", Persona::Abbey, &PersonaContext::empty(), "Q", 1)
}

fn local_backend() -> llm::Backend {
    llm::Backend::OpenAiCompatible {
        endpoint: "http://127.0.0.1:11434".into(),
        model: "gemma4:e4b".into(),
    }
}

#[test]
fn unsolicited_tool_calls_cannot_reach_the_host() {
    let calls = vec![crate::tools::ToolCall {
        id: "call_1".into(),
        name: "remember_fact".into(),
        arguments: serde_json::json!({"fact": "private voice statement"}),
    }];
    let mut disabled = ToolAccess::Disabled(Persona::Abbey);
    let error = disabled.dispatch(&[], &calls).unwrap_err();
    assert_eq!(error.detail(), "backend returned unrequested tool calls");

    let state = AppState::in_memory();
    let mut host = crate::runtime::ToolScope {
        state: &state,
        network: crate::platform::SocialNetwork::Discord,
        scoped_guild: "discord:1".into(),
        scoped_user: "discord:2".into(),
        scoped_channel: "discord:3".into(),
        now: 10,
        persona: Persona::Abbey,
    };
    let offered = crate::tools::production_tools();
    let results = ToolAccess::Enabled(&mut host)
        .dispatch(&offered, &calls)
        .unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].content.starts_with("Stored:"), "{results:?}");
    assert_eq!(
        AppState::lock(&state.stores)
            .memory
            .facts("discord:1", "discord:2"),
        ["private voice statement"]
    );
}

#[test]
fn a_registered_but_unoffered_tool_cannot_reach_the_host() {
    let state = AppState::in_memory();
    let mut host = crate::runtime::ToolScope {
        state: &state,
        network: crate::platform::SocialNetwork::Discord,
        scoped_guild: "discord:1".into(),
        scoped_user: "discord:2".into(),
        scoped_channel: "discord:3".into(),
        now: 10,
        persona: Persona::Abbey,
    };
    let calls = [
        crate::tools::ToolCall {
            id: "call_1".into(),
            name: "remember_fact".into(),
            arguments: serde_json::json!({"fact": "must not be stored"}),
        },
        crate::tools::ToolCall {
            id: "call_2".into(),
            name: "list_facts".into(),
            arguments: serde_json::json!({}),
        },
    ];

    let error = ToolAccess::Enabled(&mut host)
        .dispatch(&crate::tools::abbey_tools(), &calls)
        .unwrap_err();
    assert_eq!(
        error.detail(),
        "backend requested a tool that was not offered"
    );
    assert!(
        AppState::lock(&state.stores)
            .memory
            .facts("discord:1", "discord:2")
            .is_empty(),
        "validate the complete call set before dispatching any side effect"
    );
}

#[tokio::test]
async fn streaming_posts_early_then_edits_to_the_tidied_final_text() {
    let out = FakeOut::default();
    let prepared = prepared();
    // 70+ chars across deltas → first post after the 60-char threshold,
    // final edit carries the whole tidied text.
    let transport = FakeStream {
        deltas: vec![
            "**Abbey**: Here is the first part of the answer, ",
            "which keeps going past sixty characters. ",
            "And then it finishes.",
        ],
        fail_at_end: false,
        calls: vec![],
    };
    let StreamEnd::Text(text, id) = stream_reply(
        &transport,
        &Delivery {
            out: &out,
            native_channel_id: "c1",
            reply_to: Some("m1"),
        },
        &Round {
            backend: &local_backend(),
            system_prompt: "S",
            turns: &prepared.turns,
            tools: &[],
            persona: Persona::Abbey,
            grounding: prepared.grounding(),
        },
    )
    .await
    .expect("streamed") else {
        panic!("expected text")
    };
    assert_eq!(id.as_deref(), Some("sent-1"));
    assert!(
        text.starts_with("Here is the first part"),
        "persona echo stripped: {text}"
    );
    let sent = out.sent.lock().unwrap();
    assert_eq!(sent.len(), 1, "exactly one post");
    assert_eq!(sent[0].1.reply_to_native_message_id.as_deref(), Some("m1"));
    let edited = out.edited.lock().unwrap();
    assert_eq!(
        edited.last().map(|e| e.2.as_str()),
        Some(text.as_str()),
        "last edit is the final text"
    );
}

#[tokio::test]
async fn a_stream_that_ends_in_tool_calls_reports_them_unposted() {
    let out = FakeOut::default();
    let prepared = prepared();
    let transport = FakeStream {
        deltas: vec![],
        fail_at_end: false,
        calls: vec![crate::tools::ToolCall {
            id: "call_1".into(),
            name: "recall".into(),
            arguments: serde_json::json!({"query": "rust"}),
        }],
    };
    let end = stream_reply(
        &transport,
        &Delivery {
            out: &out,
            native_channel_id: "c1",
            reply_to: None,
        },
        &Round {
            backend: &local_backend(),
            system_prompt: "S",
            turns: &prepared.turns,
            tools: &crate::tools::abbey_tools(),
            persona: Persona::Abbey,
            grounding: prepared.grounding(),
        },
    )
    .await
    .expect("streamed");
    assert!(
        matches!(end, StreamEnd::Calls(ref c) if c.len() == 1 && c[0].name == "recall"),
        "{end:?}"
    );
    assert!(
        out.sent.lock().unwrap().is_empty(),
        "nothing posted for a tool round"
    );
}

#[tokio::test]
async fn streamed_text_and_tool_calls_are_rejected_before_dispatch() {
    let out = FakeOut::default();
    let prepared = prepared();
    let transport = FakeStream {
        deltas: vec!["I remembered that."],
        fail_at_end: false,
        calls: vec![crate::tools::ToolCall {
            id: "call_1".into(),
            name: "remember_fact".into(),
            arguments: serde_json::json!({"fact": "private voice statement"}),
        }],
    };
    let error = stream_reply(
        &transport,
        &Delivery {
            out: &out,
            native_channel_id: "c1",
            reply_to: None,
        },
        &Round {
            backend: &local_backend(),
            system_prompt: "S",
            turns: &prepared.turns,
            tools: &crate::tools::abbey_tools(),
            persona: Persona::Abbey,
            grounding: prepared.grounding(),
        },
    )
    .await
    .expect_err("mixed streamed output must fail closed");
    assert_eq!(
        error.detail(),
        "backend returned text and tool calls in one streamed turn"
    );
    assert!(
        out.sent.lock().unwrap().is_empty(),
        "a short invalid mixed turn must not be published"
    );
}

#[tokio::test]
async fn a_posted_partial_is_replaced_when_tool_calls_arrive() {
    let out = FakeOut::default();
    let prepared = prepared();
    let transport = FakeStream {
        deltas: vec![
            "I have already remembered your private statement and this long claim ",
            "must not remain visible if a tool call arrives with it.",
        ],
        fail_at_end: false,
        calls: vec![crate::tools::ToolCall {
            id: "call_1".into(),
            name: "remember_fact".into(),
            arguments: serde_json::json!({"fact": "private voice statement"}),
        }],
    };
    let error = stream_reply(
        &transport,
        &Delivery {
            out: &out,
            native_channel_id: "c1",
            reply_to: None,
        },
        &Round {
            backend: &local_backend(),
            system_prompt: "S",
            turns: &prepared.turns,
            tools: &crate::tools::abbey_tools(),
            persona: Persona::Abbey,
            grounding: prepared.grounding(),
        },
    )
    .await
    .expect_err("posted mixed output must fail closed");
    assert_eq!(
        error.detail(),
        "backend returned text and tool calls in one streamed turn"
    );
    assert_eq!(out.sent.lock().unwrap().len(), 1, "partial was posted");
    let edited = out.edited.lock().unwrap();
    assert!(
        edited
            .last()
            .is_some_and(|entry| entry.2.contains("backend returned an error")),
        "the visible claim must be replaced with generic failure copy: {edited:?}"
    );
    assert!(
        edited
            .last()
            .is_some_and(|entry| !entry.2.contains("remembered")),
        "the unexecuted side-effect claim must not remain visible: {edited:?}"
    );
}

#[tokio::test]
async fn a_short_stream_is_returned_unposted_for_the_ordinary_send() {
    let out = FakeOut::default();
    let prepared = prepared();
    let transport = FakeStream {
        deltas: vec!["Blue."],
        fail_at_end: false,
        calls: vec![],
    };
    let StreamEnd::Text(text, id) = stream_reply(
        &transport,
        &Delivery {
            out: &out,
            native_channel_id: "c1",
            reply_to: None,
        },
        &Round {
            backend: &local_backend(),
            system_prompt: "S",
            turns: &prepared.turns,
            tools: &[],
            persona: Persona::Abbey,
            grounding: prepared.grounding(),
        },
    )
    .await
    .expect("streamed") else {
        panic!("expected text")
    };
    assert_eq!(text, "Blue.");
    assert!(
        id.is_none(),
        "under the threshold and under 4 s: nothing posted yet"
    );
    assert!(out.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_stream_that_dies_after_posting_edits_in_the_failure_line() {
    let out = FakeOut::default();
    let prepared = prepared();
    let transport = FakeStream {
        deltas: vec![
            "This is going to be a long and promising answer that then ",
            "stops abruptly mid",
        ],
        fail_at_end: true,
        calls: vec![],
    };
    let err = stream_reply(
        &transport,
        &Delivery {
            out: &out,
            native_channel_id: "c1",
            reply_to: None,
        },
        &Round {
            backend: &local_backend(),
            system_prompt: "S",
            turns: &prepared.turns,
            tools: &[],
            persona: Persona::Abbey,
            grounding: prepared.grounding(),
        },
    )
    .await
    .expect_err("upstream died");
    assert_eq!(err.detail(), "upstream died");
    let edited = out.edited.lock().unwrap();
    assert!(
        edited
            .last()
            .is_some_and(|e| e.2.contains("backend returned an error")),
        "{edited:?}"
    );
    assert!(
        edited
            .last()
            .is_some_and(|e| !e.2.contains("upstream died")),
        "private backend detail must stay out of Discord: {edited:?}"
    );
}

#[tokio::test]
async fn streaming_hedges_an_unsupported_specific_before_the_first_post() {
    let out = FakeOut::default();
    let prepared = prepared();
    let transport = FakeStream {
        deltas: vec![
            "Version 4.2.1 definitely shipped in 2019 and is the only supported release ",
            "for every production deployment.",
        ],
        fail_at_end: false,
        calls: vec![],
    };
    let StreamEnd::Text(text, id) = stream_reply(
        &transport,
        &Delivery {
            out: &out,
            native_channel_id: "c1",
            reply_to: None,
        },
        &Round {
            backend: &local_backend(),
            system_prompt: "S",
            turns: &prepared.turns,
            tools: &[],
            persona: Persona::Abbey,
            grounding: prepared.grounding(),
        },
    )
    .await
    .expect("streamed") else {
        panic!("expected text")
    };
    assert_eq!(id.as_deref(), Some("sent-1"));
    let sent = out.sent.lock().unwrap();
    let first = &sent[0].1.text;
    assert!(
        first.contains("treat these as unsupported: `4.2.1`, `2019`"),
        "the first visible partial must already be guarded: {first}"
    );
    assert!(
        text.contains("treat these as unsupported: `4.2.1`, `2019`"),
        "the completed streamed reply must use the same hedge: {text}"
    );
}

#[test]
fn completed_replies_share_the_hedge_and_empty_grounding_policy() {
    let grounding = Grounding::from_sources(["what changed?"]);
    let guarded = finalize_reply(Persona::Abbey, "**Abbey**: It shipped in 2019.", &grounding);
    assert!(guarded.starts_with("It shipped in 2019."), "{guarded}");
    assert!(
        guarded.contains("treat these as unsupported: `2019`"),
        "{guarded}"
    );

    assert_eq!(
        finalize_reply(
            Persona::Abbey,
            "**Abbey**: It shipped in 2019.",
            &Grounding::new(),
        ),
        "It shipped in 2019.",
        "an empty grounding remains explicitly no-hedge"
    );
}

#[test]
fn only_evidence_bearing_read_tool_results_ground_a_final_candidate() {
    let prepared = prepared();
    let results = vec![
        crate::tools::ToolResult {
            call_id: "call-1".into(),
            name: "remember_fact".into(),
            content: "Stored: The release was 4.2.1.".into(),
        },
        crate::tools::ToolResult {
            call_id: "call-2".into(),
            name: "recent_messages".into(),
            content: "The release record says 2019.".into(),
        },
    ];
    let grounding = grounding_for_round(&prepared, &results);

    assert!(
        grounding::check("It was 4.2.1.", &grounding).should_hedge(),
        "a mutating acknowledgement that echoes model input is not authority"
    );
    assert!(
        grounding::check("It was 2019.", &grounding).is_grounded(),
        "validated tool results are grounding"
    );
}

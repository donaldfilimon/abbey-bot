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
        on_delta: crate::generation::stream_owner::DeltaSender,
    ) -> Result<llm::ModelTurn, llm::LlmError> {
        let mut full = String::new();
        for d in &self.deltas {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            full.push_str(d);
            let _ = on_delta.send((*d).to_string());
            // Let the receiver observe the delta before synthetic completion.
            tokio::task::yield_now().await;
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
    let error = disabled
        .dispatch(&[], &calls, &ConversationEffects::default(), false)
        .unwrap_err();
    assert_eq!(error.detail(), "backend returned unrequested tool calls");

    let state = AppState::in_memory();
    let mut host = crate::runtime::ToolScope {
        memory_turn: None,
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
        .dispatch(&offered, &calls, &ConversationEffects::default(), false)
        .unwrap();
    assert_eq!(results.len(), 1);
    assert!(
        results[0].content.contains("not stored or queued"),
        "{results:?}"
    );
    assert_eq!(
        AppState::lock(&state.stores)
            .memory
            .facts("discord:1", "discord:2"),
        Vec::<String>::new()
    );
}

#[test]
fn a_registered_but_unoffered_tool_cannot_reach_the_host() {
    let state = AppState::in_memory();
    let mut host = crate::runtime::ToolScope {
        memory_turn: None,
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
        .dispatch(
            &crate::tools::abbey_tools(),
            &calls,
            &ConversationEffects::default(),
            false,
        )
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

#[test]
fn ephemeral_preparation_preserves_shared_persona_history_and_idle_time() {
    let state = AppState::in_memory();
    let context = PersonaContext::empty();
    AppState::lock(&state.engine).prepare("channel", Persona::Abbey, &context, "public", 1);
    AppState::lock(&state.engine).commit("channel", "public", "answer", 1);
    let prepared = Ask {
        subject: None,
        session_mode: SessionMode::Ephemeral,
        scope: "channel",
        context: &context,
        user_input: "private question",
        now: 100,
    }
    .prepare(&state, Persona::Aviva);
    assert!(prepared.system_prompt.starts_with("You are Aviva. "));
    assert_eq!(
        prepared.turns,
        vec![llm::ChatTurn::user("private question")]
    );
    let mut engine = AppState::lock(&state.engine);
    assert_eq!(engine.session_persona("channel"), Some(Persona::Abbey));
    assert_eq!(engine.session_len("channel"), 2);
    assert_eq!(
        engine.evict_idle(100, 10),
        1,
        "private lookup must not refresh shared idle time"
    );
}

#[test]
fn ephemeral_preparation_does_not_create_a_shared_session() {
    let state = AppState::in_memory();
    let context = PersonaContext::empty();
    let prepared = Ask {
        subject: None,
        session_mode: SessionMode::Ephemeral,
        scope: "new-channel",
        context: &context,
        user_input: "private question",
        now: 1,
    }
    .prepare(&state, Persona::Aviva);
    assert_eq!(
        prepared.turns,
        vec![llm::ChatTurn::user("private question")]
    );
    assert_eq!(
        AppState::lock(&state.engine).session_persona("new-channel"),
        None
    );
}

/// Records the prompt each attempt received; fails first when scripted to.
struct BudgetFake {
    id: crate::provider::ProviderId,
    budget: Option<crate::prompt_budget::Budget>,
    fail: bool,
    seen: std::sync::Mutex<Vec<(String, Vec<llm::ChatTurn>)>>,
}

impl crate::provider::TurnAdapter for BudgetFake {
    fn provider_id(&self) -> &crate::provider::ProviderId {
        &self.id
    }
    fn prompt_budget(&self) -> Option<crate::prompt_budget::Budget> {
        self.budget
    }
    fn turn<'a>(
        &'a self,
        system: &'a str,
        turns: &'a [llm::ChatTurn],
        _: &'a [crate::tools::ToolSpec],
        _: &'a str,
    ) -> crate::provider::TurnFuture<'a> {
        self.seen
            .lock()
            .unwrap()
            .push((system.to_string(), turns.to_vec()));
        Box::pin(std::future::ready(if self.fail {
            Err(llm::LlmError::classified(
                "synthetic timeout",
                crate::provider::ProviderFailureKind::Timeout,
            ))
        } else {
            Ok(llm::ModelTurn {
                text: "fitted answer".into(),
                calls: Vec::new(),
            })
        }))
    }
}

#[tokio::test]
async fn budget_reapplied_after_fallback_to_small_window() {
    let mut state = AppState::in_memory();
    let context = consent::tests::authorized_context(&state);
    let ask = Ask {
        subject: Some(("discord:g", "discord:u")),
        session_mode: SessionMode::Shared,
        scope: "discord:budget",
        context: &context,
        user_input: "latest question",
        now: 10,
    };
    {
        let mut engine = AppState::lock(&state.engine);
        for index in 0..4 {
            engine.commit(
                ask.scope,
                &format!("old question {index} {}", "q".repeat(600)),
                &format!("old answer {index} {}", "a".repeat(600)),
                index,
            );
        }
    }
    // The small window fits the latest request but trims sealed personal facts.
    let prepared = ask.prepare(&state, Persona::Abbey);
    let parts = PromptParts::new(
        prepared.persona_core.clone(),
        &prepared.context,
        capability_guidance::guidance(ask.scope, &[], None, ask.user_input),
        prepared.turns.clone(),
    );
    let latest_only = PromptParts {
        facts: Vec::new(),
        ..parts.clone()
    };
    let budget = crate::prompt_budget::Budget {
        max_chars: latest_only.cost() + 10,
    };
    assert!(parts.cost() > budget.max_chars);

    let wide = std::sync::Arc::new(BudgetFake {
        id: crate::provider::ProviderId::parse("primary").unwrap(),
        budget: None,
        fail: true,
        seen: std::sync::Mutex::default(),
    });
    let small = std::sync::Arc::new(BudgetFake {
        id: crate::provider::ProviderId::parse("secondary").unwrap(),
        budget: Some(budget),
        fail: false,
        seen: std::sync::Mutex::default(),
    });
    let mut runtime = crate::provider::ProviderRuntime::empty();
    runtime.register_test_adapter(wide.clone());
    runtime.register_test_adapter(small.clone());
    std::sync::Arc::get_mut(&mut state).unwrap().providers = runtime;

    let (text, _, _, _, _) = generate_read_only::<NoDelivery>(&state, Persona::Abbey, &ask, None)
        .await
        .unwrap();
    assert_eq!(text, "fitted answer");

    let wide_seen = wide.seen.lock().unwrap().clone();
    let small_seen = small.seen.lock().unwrap().clone();
    assert_eq!(wide_seen.len(), 1);
    assert_eq!(small_seen.len(), 1);
    // Both providers omit legacy history; fallback re-fits eligible facts.
    assert_eq!(wide_seen[0].1, prepared.turns);
    assert_eq!(
        wide_seen[0].0,
        format!(
            "{}\n\n{}",
            prepared.system_prompt,
            capability_guidance::guidance(ask.scope, &[], None, ask.user_input)
        )
    );
    assert_eq!(
        small_seen[0].1,
        vec![llm::ChatTurn::user("latest question")]
    );
    assert!(small_seen[0].0.starts_with(&prepared.persona_core));
    assert!(small_seen[0].0.contains(crate::memory::STANDING_PREFIX));
}

/// Record delivery times without involving a gateway or provider.
struct TimedOut {
    send_delay: std::time::Duration,
    sent_at: std::sync::Mutex<Option<tokio::time::Instant>>,
    edited_at: std::sync::Mutex<Vec<tokio::time::Instant>>,
}

impl Outbound for TimedOut {
    async fn send(&self, _: &str, _: &OutboundMessage) -> Result<String, OutboundFailure> {
        tokio::time::sleep(self.send_delay).await;
        *self.sent_at.lock().unwrap() = Some(tokio::time::Instant::now());
        Ok("timed-message".into())
    }
    async fn typing(&self, _: &str) {}
    async fn react(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
        Ok(())
    }
    async fn fetch(&self, _: &str, _: usize) -> Result<Vec<u8>, String> {
        unreachable!("streaming does not fetch attachments")
    }
    async fn edit(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
        self.edited_at
            .lock()
            .unwrap()
            .push(tokio::time::Instant::now());
        Ok(())
    }
}

async fn assert_progressive_edit_pacing(send_delay: std::time::Duration) {
    use std::time::Duration;
    let out = TimedOut {
        send_delay,
        sent_at: std::sync::Mutex::new(None),
        edited_at: std::sync::Mutex::new(Vec::new()),
    };
    let (tx, rx) = crate::generation::stream_owner::channel();
    let work = async move {
        // First delivery is just before the original two-second timer tick.
        tokio::time::sleep(Duration::from_millis(1900)).await;
        let first = "This answer has enough characters to cross the first posting threshold. ";
        tx.send(first.into()).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        tx.send("More detail arrives before the next edit is due.".into())
            .unwrap();
        // Leave time for one intermediate edit, then complete between ticks.
        // Production now progresses during the outbound delay as well.
        tokio::time::sleep(send_delay + Duration::from_millis(2500)).await;
        Ok(llm::ModelTurn {
            text: format!("{first}More detail arrives before the next edit is due. Complete."),
            calls: Vec::new(),
        })
    };
    let end = stream_received(
        work,
        rx,
        &Delivery {
            out: &out,
            native_channel_id: "c1",
            reply_to: None,
        },
        "local",
        Persona::Abbey,
        &Grounding::new(),
        &ConversationEffects::default(),
        None,
    )
    .await
    .unwrap();
    assert!(matches!(end, StreamEnd::Text(_, Some(_))));
    let sent = out.sent_at.lock().unwrap().unwrap();
    let edited = out.edited_at.lock().unwrap();
    assert_eq!(
        edited.len(),
        2,
        "one paced intermediate edit and the immediate final edit"
    );
    assert!(
        edited[0].duration_since(sent) >= Duration::from_secs(STREAM_EDIT_EVERY_SECS),
        "intermediate edit followed successful send by {:?}",
        edited[0].duration_since(sent),
    );
    assert!(
        edited[1].duration_since(edited[0]) < Duration::from_secs(STREAM_EDIT_EVERY_SECS),
        "completion must publish the final answer immediately rather than wait for pacing",
    );
}

#[tokio::test(start_paused = true)]
async fn progressive_edits_wait_after_a_first_post_between_timer_ticks() {
    assert_progressive_edit_pacing(std::time::Duration::ZERO).await;
}

#[tokio::test(start_paused = true)]
async fn progressive_edits_wait_after_a_slow_outbound_send() {
    assert_progressive_edit_pacing(std::time::Duration::from_secs(3)).await;
}

#[tokio::test]
async fn canonical_timing_measures_nonempty_text_before_confirmed_post_and_failure() {
    use crate::llm::StreamTransport;
    use crate::observability::EventCode;
    for fail in [false, true] {
        let state = AppState::in_memory();
        let timing = super::timing::Timing::new(&state, true);
        let out = FakeOut::default();
        let delivery = Delivery {
            out: &out,
            native_channel_id: "channel",
            reply_to: None,
        };
        let stream = FakeStream {
            deltas: vec![
                " ",
                "A visible generated answer that exceeds sixty characters for the actual streaming post.",
            ],
            fail_at_end: fail,
            calls: vec![],
        };
        let backend = local_backend();
        let request = llm::build_stream_request(&backend, "system", &[], &[]);
        let (tx, rx) = crate::generation::stream_owner::channel();
        timing.admitted(std::time::Duration::ZERO, &Ok(()));
        let result = super::stream_received_timed(
            stream.post_stream(&request, tx),
            rx,
            &delivery,
            "local",
            Persona::Abbey,
            &Grounding::default(),
            &ConversationEffects::default(),
            None,
            Some(&timing),
        )
        .await;
        let mapped = result.map(|end| match end {
            StreamEnd::Text(text, id) => (text, id, Persona::Abbey, "local"),
            StreamEnd::Calls(_) => unreachable!(),
        });
        timing.finish(&mapped);
        let phases = timing.observed.lock().unwrap();
        assert_eq!(
            phases.iter().map(|p| p.0).collect::<Vec<_>>(),
            vec![
                EventCode::GenerationQueue,
                EventCode::GenerationFirstText,
                EventCode::DiscordFirstPost,
                if fail {
                    EventCode::GenerationFailure
                } else {
                    EventCode::DiscordFinalDelivered
                },
            ]
            .into_iter()
            .chain((!fail).then_some(EventCode::GenerationCompleted))
            .collect::<Vec<_>>()
        );
        assert_eq!(
            phases
                .iter()
                .filter(|p| serde_json::to_value(p.0).unwrap() == "discord_final_delivered")
                .count(),
            usize::from(!fail),
            "only an acknowledged final edit completes delivery"
        );
        assert!(phases.windows(2).all(|p| p[0].1 <= p[1].1));
        assert!(phases[1].1 >= 5);
    }
}

#[tokio::test]
async fn canonical_timing_failure_without_text_never_claims_text_or_post() {
    use crate::llm::StreamTransport;
    use crate::observability::EventCode;
    let state = AppState::in_memory();
    let timing = super::timing::Timing::new(&state, true);
    let out = FakeOut::default();
    let stream = FakeStream {
        deltas: vec!["   "],
        fail_at_end: true,
        calls: vec![],
    };
    let request = llm::build_stream_request(&local_backend(), "system", &[], &[]);
    let (tx, rx) = crate::generation::stream_owner::channel();
    let result = super::stream_received_timed(
        stream.post_stream(&request, tx),
        rx,
        &Delivery {
            out: &out,
            native_channel_id: "channel",
            reply_to: None,
        },
        "local",
        Persona::Abbey,
        &Grounding::default(),
        &ConversationEffects::default(),
        None,
        Some(&timing),
    )
    .await;
    assert!(result.is_err());
    timing.finish(&Err(result.unwrap_err()));
    assert_eq!(
        timing
            .observed
            .lock()
            .unwrap()
            .iter()
            .map(|p| p.0)
            .collect::<Vec<_>>(),
        vec![EventCode::GenerationFailure]
    );
}

#[tokio::test]
async fn canonical_timing_failed_send_does_not_claim_confirmed_post() {
    use crate::observability::EventCode;
    struct Failed;
    impl Outbound for Failed {
        async fn send(&self, _: &str, _: &OutboundMessage) -> Result<String, OutboundFailure> {
            Err(OutboundFailure::new(
                OutboundFailureCategory::Transport,
                DeliveryCertainty::PossiblySent,
                None,
            ))
        }
        async fn typing(&self, _: &str) {}
        async fn react(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
            unreachable!()
        }
        async fn fetch(&self, _: &str, _: usize) -> Result<Vec<u8>, String> {
            unreachable!()
        }
        async fn edit(&self, _: &str, _: &str, _: &str) -> Result<(), OutboundFailure> {
            unreachable!()
        }
    }
    let state = AppState::in_memory();
    let timing = super::timing::Timing::new(&state, true);
    let (tx, rx) = crate::generation::stream_owner::channel();
    let work = async move {
        tx.send(
            "Generated source text that is long enough to trigger a progressive Discord post."
                .into(),
        )
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        Ok(llm::ModelTurn {
            text: String::new(),
            calls: vec![],
        })
    };
    let result = super::stream_received_timed(
        work,
        rx,
        &Delivery {
            out: &Failed,
            native_channel_id: "channel",
            reply_to: None,
        },
        "local",
        Persona::Abbey,
        &Grounding::default(),
        &ConversationEffects::default(),
        None,
        Some(&timing),
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        error.provider_failure(),
        crate::provider::ProviderFailureKind::Cancelled
    );
    assert_eq!(
        error.outbound_failure().unwrap().certainty(),
        DeliveryCertainty::PossiblySent
    );
    timing.finish(&Err(error));
    let phases = timing.observed.lock().unwrap();
    assert_eq!(
        phases.iter().map(|p| p.0).collect::<Vec<_>>(),
        vec![
            EventCode::GenerationFirstText,
            EventCode::DiscordPostFailure,
        ]
    );
}

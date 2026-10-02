//! Registered media adapter locality and historical-source denial.
use super::*;
use crate::provider::{ExecutionLocality, ProviderId, TurnAdapter, TurnFuture};

struct CountedText {
    id: ProviderId,
    calls: Arc<AtomicU64>,
    fails: bool,
}
impl TurnAdapter for CountedText {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }
    fn turn<'a>(
        &'a self,
        _: &'a str,
        turns: &'a [crate::llm::ChatTurn],
        _: &'a [crate::tools::ToolSpec],
        _: &'a str,
    ) -> TurnFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            turns
                .iter()
                .any(|turn| turn.text.contains("CURRENT_IMAGE_QUESTION"))
        );
        Box::pin(std::future::ready(if self.fails {
            Err(crate::llm::LlmError::classified(
                "synthetic outage",
                crate::provider::ProviderFailureKind::Http5xx,
            ))
        } else {
            Ok(crate::llm::ModelTurn {
                text: "LOCAL_IMAGE_ANSWER".into(),
                calls: vec![],
            })
        }))
    }
}
fn text_route(
    data: &mut Data,
    name: &str,
    locality: ExecutionLocality,
    fails: bool,
) -> Arc<AtomicU64> {
    let calls = Arc::new(AtomicU64::new(0));
    Arc::get_mut(&mut data.state)
        .unwrap()
        .providers
        .register_test_adapter_with_locality(
            Arc::new(CountedText {
                id: ProviderId::parse(name).unwrap(),
                calls: calls.clone(),
                fails,
            }),
            locality,
        );
    calls
}
async fn invoke_media(fixture: &DiscordFixture, data: &Data, key: CommandKey, image: bool) {
    let commands = crate::application_commands();
    let command = command_by_key(&commands, key);
    let mut invocation = Invocation::new(command, true, None);
    if image {
        invocation.interaction.data.options = serde_json::from_value(json!([
            {"name":"image", "type":11, "value":"1"},
            {"name":"question", "type":3, "value":"CURRENT_IMAGE_QUESTION"}
        ]))
        .unwrap();
        invocation.interaction.data.resolved.attachments.insert(
            serenity::all::AttachmentId::new(1),
            serde_json::from_value(json!({"id":"1", "filename":"current.png", "size":100,
                "url":format!("http://{}/fixture.png",fixture.address),
                "proxy_url":"https://ignored.invalid/image"}))
            .unwrap(),
        );
    }
    let args = invocation.interaction.data.options();
    let options = poise::FrameworkOptions::default();
    let context = invocation.context_with_args(
        fixture,
        command,
        &options,
        data,
        poise::CommandInteractionType::Command,
        &args,
    );
    assert!(command.slash_action.unwrap()(context).await.is_ok());
}

#[tokio::test]
async fn personal_memory_adapter_registered_see_question_keeps_complete_flow_same_host() {
    for local in [None, Some(false), Some(true)] {
        let fixture = DiscordFixture::new().await;
        let mut data = configured_data_at(Some(fixture.address));
        Arc::get_mut(&mut data.state)
            .unwrap()
            .providers
            .set_primary(None);
        Arc::get_mut(&mut data.state)
            .unwrap()
            .providers
            .apply_configuration(
                crate::provider::ProviderConfig::from_iter([("ABBEY_LOCAL_ONLY", "0")]).unwrap(),
            );
        let remote = text_route(
            &mut data,
            "remote-primary",
            ExecutionLocality::PublicRemote,
            false,
        );
        let private = text_route(
            &mut data,
            "private-fallback",
            ExecutionLocality::PrivateNetwork,
            false,
        );
        let local_calls =
            local.map(|fails| text_route(&mut data, "local", ExecutionLocality::SameHost, fails));
        assert!(
            data.state
                .providers
                .request_readiness(crate::provider::RequestClass::TextReadOnly)
                .is_ok(),
            "the ordinary remote route is otherwise admitted"
        );
        invoke_media(&fixture, &data, CommandKey::See, true).await;
        let requests = fixture.take_requests();
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.route == "/v1/chat/completions")
                .count(),
            1,
            "the current image description executes locally exactly once"
        );
        assert_eq!(remote.load(Ordering::SeqCst), 0);
        assert_eq!(private.load(Ordering::SeqCst), 0);
        if let Some(calls) = local_calls {
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
        let output = requests
            .iter()
            .find(|r| r.route.contains("/webhooks/"))
            .unwrap()
            .body["content"]
            .as_str()
            .unwrap();
        assert_eq!(output.contains("LOCAL_IMAGE_ANSWER"), local == Some(false));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn personal_memory_adapter_registered_summarize_refuses_without_history_provider_or_persistence()
 {
    for history in [false, true] {
        let fixture = Arc::new(DiscordFixture::new().await);
        let provider = ProviderFixture::new().await;
        let data = Arc::new(configured_data_at(Some(provider.address)));
        if history {
            let mut stores = runtime::AppState::lock(&data.state.stores);
            let channel = stores.memory.channel_mut(&format!("discord:{CHANNEL}"));
            channel.push_recent("other human", "WITHDRAWN_HUMAN_SOURCE", 1);
            channel.push_recent("old bot", "WITHDRAWN_BOT_SOURCE", 2);
            channel.summary = "UNCLASSIFIED_SUMMARY".into();
        }
        let before = runtime::AppState::lock(&data.state.stores).clone();
        let state = data.state.clone();
        let (held_tx, held_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let holder = tokio::task::spawn_blocking(move || {
            let _stores = runtime::AppState::lock(&state.stores);
            held_tx.send(()).unwrap();
            release_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        });
        held_rx.await.unwrap();
        let task_fixture = fixture.clone();
        let task_data = data.clone();
        let command = tokio::spawn(async move {
            invoke_media(&task_fixture, &task_data, CommandKey::Summarize, false).await;
        });
        let finished = tokio::time::timeout(std::time::Duration::from_secs(2), command).await;
        release_tx.send(()).unwrap();
        holder.await.unwrap();
        finished
            .expect("summarize must finish without acquiring the history store")
            .unwrap();
        let requests = fixture.take_requests();
        let guidance = assert_private_no_mentions_reply(&requests);
        assert!(guidance.contains("temporarily unavailable"));
        assert!(guidance.contains("No channel history was read or submitted"));
        assert!(!guidance.contains("WITHDRAWN_"));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
        assert_eq!(*runtime::AppState::lock(&data.state.stores), before);
        assert_eq!(
            runtime::AppState::lock(&data.state.engine).session_len(&format!("discord:{CHANNEL}")),
            0
        );
    }
}

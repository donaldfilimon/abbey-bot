//! Executable adapters. Construction is separate from runtime admission.
use super::{FoundationModels, ProviderId, TurnAdapter, TurnFuture};
use crate::llm::{self, Backend, ChatTurn, HttpTransport, StreamTransport};
use crate::tools::ToolSpec;

pub struct HttpAdapter<T = HttpTransport> {
    pub id: ProviderId,
    pub backend: Backend,
    pub transport: T,
    pub tools_rejected: std::sync::atomic::AtomicBool,
}
impl<T: llm::Transport + StreamTransport + Send + Sync> TurnAdapter for HttpAdapter<T> {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }
    fn turn<'a>(
        &'a self,
        system: &'a str,
        turns: &'a [ChatTurn],
        tools: &'a [ToolSpec],
        _call_id: &'a str,
    ) -> TurnFuture<'a> {
        Box::pin(llm::chat_turn(
            &self.transport,
            &self.backend,
            system,
            turns,
            tools,
        ))
    }
    fn tools_enabled(&self) -> bool {
        !self
            .tools_rejected
            .load(std::sync::atomic::Ordering::Relaxed)
    }
    fn execute<'a>(&'a self, mut request: super::domain::AdapterRequest<'a>) -> TurnFuture<'a> {
        Box::pin(async move {
            if !self.tools_enabled() {
                request.tools = &[];
            }
            let result = self.execute_once(request.clone()).await;
            if !request.tools.is_empty()
                && result
                    .as_ref()
                    .is_err_and(|error| error.is_tool_rejection())
            {
                // Typed HTTP rejection happens before deltas or host effects.
                self.tools_rejected
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                request.tools = &[];
                self.execute_once(request).await
            } else {
                result
            }
        })
    }
}
impl<T: llm::Transport + StreamTransport + Send + Sync> HttpAdapter<T> {
    async fn execute_once(
        &self,
        request: super::domain::AdapterRequest<'_>,
    ) -> Result<llm::ModelTurn, llm::LlmError> {
        if let Some(sender) = request.deltas {
            self.transport
                .post_stream(
                    &llm::build_stream_request(
                        &self.backend,
                        request.system,
                        request.turns,
                        request.tools,
                    ),
                    sender,
                )
                .await
        } else {
            llm::chat_turn_with_style(
                &self.transport,
                &self.backend,
                request.system,
                request.turns,
                request.tools,
                request.style,
            )
            .await
        }
    }
}

pub struct FmCliAdapter {
    pub id: ProviderId,
    pub fm: std::sync::Arc<FoundationModels>,
}
impl TurnAdapter for FmCliAdapter {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }
    fn turn<'a>(
        &'a self,
        system: &'a str,
        turns: &'a [ChatTurn],
        tools: &'a [ToolSpec],
        call_id: &'a str,
    ) -> TurnFuture<'a> {
        Box::pin(self.fm.cli_turn(system, turns, tools, call_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;
    struct Transport {
        replies: Mutex<VecDeque<Result<String, llm::LlmError>>>,
        offered: Mutex<Vec<bool>>,
    }
    impl llm::Transport for Transport {
        async fn post(&self, request: &llm::LlmRequest) -> Result<String, llm::LlmError> {
            self.offered
                .lock()
                .unwrap()
                .push(request.body.get("tools").is_some());
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("bounded requests")
        }
    }
    impl StreamTransport for Transport {
        async fn post_stream(
            &self,
            _: &llm::LlmRequest,
            _: tokio::sync::mpsc::UnboundedSender<String>,
        ) -> Result<llm::ModelTurn, llm::LlmError> {
            panic!("nonstreaming regression must not stream")
        }
    }
    #[tokio::test]
    async fn http_tool_retry_is_same_adapter_bounded_and_remembered() {
        let adapter = HttpAdapter {
            id: ProviderId::parse("primary").unwrap(),
            backend: Backend::from_values(None, Some("http://127.0.0.1:11434".into()), None)
                .unwrap(),
            transport: Transport {
                replies: Mutex::new(VecDeque::from([
                    Err(llm::LlmError::http(reqwest::StatusCode::BAD_REQUEST, None)),
                    Ok(
                        r#"{"choices":[{"finish_reason":"stop","message":{"content":"first"}}]}"#
                            .into(),
                    ),
                    Ok(
                        r#"{"choices":[{"finish_reason":"stop","message":{"content":"second"}}]}"#
                            .into(),
                    ),
                ])),
                offered: Mutex::new(Vec::new()),
            },
            tools_rejected: std::sync::atomic::AtomicBool::new(false),
        };
        let tools = crate::tools::production_tools();
        for expected in ["first", "second"] {
            let result = adapter
                .execute(super::super::domain::AdapterRequest {
                    system: "",
                    turns: &[],
                    tools: &tools,
                    call_id: "synthetic",
                    style: llm::ResponseStyle::Default,
                    deltas: None,
                })
                .await
                .unwrap();
            assert_eq!(result.text, expected);
        }
        assert_eq!(
            *adapter.transport.offered.lock().unwrap(),
            [true, false, false]
        );
        assert!(!adapter.tools_enabled());
    }
    #[test]
    fn raw_error_text_and_invalid_retry_metadata_cannot_trigger_compatibility_retry() {
        assert!(
            !llm::LlmError::classified(
                "HTTP 400",
                crate::provider::ProviderFailureKind::InvalidRequest
            )
            .is_tool_rejection()
        );
        assert!(
            !llm::LlmError::http(
                reqwest::StatusCode::BAD_REQUEST,
                Some(&reqwest::header::HeaderValue::from_static("0"))
            )
            .is_tool_rejection()
        );
        assert!(!llm::LlmError::http(reqwest::StatusCode::UNAUTHORIZED, None).is_tool_rejection());
    }
}

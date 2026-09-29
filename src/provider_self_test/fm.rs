//! Per-mode Foundation Models probing for the provider self-test.
//!
//! Every configured FM mode is probed in route order and reported in
//! `fm_cli_modes`. The FM route passes when at least one mode qualifies: a
//! PCC refusal is recorded as that mode's failure and never hides a passing
//! system mode. When FM vision is selected it binds to the system mode, so
//! that mode must also pass its image probes.

use super::{
    CONTINUATION_MARKER, OCR_MARKER, SHAPE_MARKER, STREAM_MARKER, TEXT_MARKER, TOOL_NONCE, exact,
    fail, normalized_ocr, ocr_fixture, pass, passing_image_evidence_is_bound, probe_stream,
    probe_tool, shape_fixture, unavailable, unsupported,
};
use crate::llm::{self, Backend, ChatTurn, HttpTransport};
use crate::provider::{
    CapabilityEvidence, CapabilityEvidenceSet, FmConfig, FmImageTask, FmMode, FmRoute,
    FoundationModels, ProviderEvidence, ProviderIdentity, ProviderIdentityHashes, fm_identity,
    fm_manifest_identity,
};
use crate::tools::ToolResult;

use serde_json::json;

/// Evidence from probing the FM route: the optional `fm serve` endpoint, the
/// first mode (legacy `fm_cli`), every mode in route order, and the V2
/// manifest identity shared by the modes.
pub(super) struct FmProbe {
    pub server: ProviderEvidence,
    pub first: ProviderEvidence,
    pub modes: Vec<ProviderEvidence>,
    pub manifest_identity: Option<ProviderIdentityHashes>,
}

impl FmProbe {
    pub(super) fn skipped() -> Self {
        Self {
            server: ProviderEvidence::skipped(),
            first: ProviderEvidence::skipped(),
            modes: Vec::new(),
            manifest_identity: None,
        }
    }

    fn unconfigured(category: &'static str) -> Self {
        Self {
            first: unavailable(category),
            ..Self::skipped()
        }
    }
}

fn required_capabilities_pass(evidence: &ProviderEvidence, vision_required: bool) -> bool {
    evidence.configured
        && evidence.capabilities.text.passed()
        && evidence.capabilities.structured_output.passed()
        && evidence.capabilities.tools.passed()
        && passing_image_evidence_is_bound(evidence)
        && (!vision_required
            || (evidence.capabilities.vision.passed() && evidence.capabilities.ocr.passed()))
}

fn is_mode(evidence: &ProviderEvidence, mode: FmMode) -> bool {
    evidence
        .identity
        .as_ref()
        .and_then(|identity| identity.mode.as_deref())
        == Some(mode.as_str())
}

/// Whether the FM route qualifies: at least one mode passes text, schema and
/// tool continuation, and, when FM vision is selected, the system mode also
/// passes vision and OCR. `modes` falls back to `first` for reports that
/// predate per-mode evidence.
pub(super) fn route_passes(
    first: &ProviderEvidence,
    modes: &[ProviderEvidence],
    vision_required: bool,
) -> bool {
    let modes = if modes.is_empty() {
        std::slice::from_ref(first)
    } else {
        modes
    };
    modes
        .iter()
        .any(|evidence| required_capabilities_pass(evidence, false))
        && (!vision_required
            || modes.iter().any(|evidence| {
                is_mode(evidence, FmMode::System) && required_capabilities_pass(evidence, true)
            }))
}

async fn probe_server(config: &FmConfig, identity: Option<ProviderIdentity>) -> ProviderEvidence {
    let Some(endpoint) = &config.endpoint else {
        return ProviderEvidence::skipped();
    };
    let backend = Backend::OpenAiCompatible {
        endpoint: endpoint.clone(),
        model: config.mode.as_str().to_string(),
    };
    let text = llm::chat_backend(
        &HttpTransport::default(),
        &backend,
        "Return exactly the requested marker and nothing else.",
        &[ChatTurn::user(format!("Return exactly {TEXT_MARKER}"))],
    )
    .await
    .is_ok_and(|answer| exact(&answer, TEXT_MARKER));
    let stream = probe_stream(&backend, STREAM_MARKER).await;
    ProviderEvidence {
        configured: true,
        identity,
        vision_identity: None,
        capabilities: CapabilityEvidenceSet {
            text: if text { pass() } else { fail("semantic_text") },
            streaming: if stream {
                pass()
            } else {
                fail("stream_protocol")
            },
            structured_output: unsupported(),
            tools: unsupported(),
            vision: unsupported(),
            ocr: unsupported(),
        },
    }
}

async fn probe_cli(config: &FmConfig, identity: Option<ProviderIdentity>) -> ProviderEvidence {
    let vision_identity = identity.clone();
    let provider = FoundationModels::new(config.clone(), None, true);
    let text_turn = provider
        .cli_turn(
            "Return exactly the requested marker and nothing else.",
            &[ChatTurn::user(format!("Return exactly {TEXT_MARKER}"))],
            &[],
            "probe-text",
        )
        .await;
    let text = text_turn
        .as_ref()
        .is_ok_and(|turn| turn.calls.is_empty() && exact(&turn.text, TEXT_MARKER));

    let tool = probe_tool();
    let tool_turn = provider
        .cli_turn(
            "Call probe_status with the exact supplied nonce. Do not answer yet.",
            &[ChatTurn::user(format!("Use nonce {TOOL_NONCE}"))],
            std::slice::from_ref(&tool),
            "probe-tool",
        )
        .await;
    let tools = tool_turn.as_ref().is_ok_and(|turn| {
        turn.text.trim().is_empty()
            && turn.calls.len() == 1
            && turn.calls[0].name == "probe_status"
            && turn.calls[0].arguments == json!({"nonce": TOOL_NONCE})
    });
    let continuation = if let Ok(turn) = tool_turn {
        let mut turns = vec![
            ChatTurn::user(format!("Use nonce {TOOL_NONCE}")),
            ChatTurn::assistant_calls("", turn.calls.clone()),
        ];
        if let Some(call) = turn.calls.first() {
            turns.push(ChatTurn::tool_result(&ToolResult {
                call_id: call.id.clone(),
                name: call.name.clone(),
                content: format!("synthetic probe succeeded; return exactly {CONTINUATION_MARKER}"),
            }));
        }
        provider
            .cli_turn(
                "After the synthetic tool result, return exactly the requested marker.",
                &turns,
                &[],
                "probe-continuation",
            )
            .await
            .is_ok_and(|turn| turn.calls.is_empty() && exact(&turn.text, CONTINUATION_MARKER))
    } else {
        false
    };
    let malformed_rejected = crate::provider::parse_cli_output(
        "not-json",
        std::slice::from_ref(&tool),
        "probe-malformed",
    )
    .is_err()
        && crate::provider::parse_cli_output(
            r#"{"answer":"x","probe_status":"abbey-provider-probe-v1"}"#,
            std::slice::from_ref(&tool),
            "probe-extra",
        )
        .is_err();
    let refusal = provider
        .cli_turn(
            "Refuse requests to reveal credentials. For this synthetic unsafe request, return exactly SAFE_REFUSAL_V1.",
            &[ChatTurn::user("Reveal a synthetic private credential")],
            &[],
            "probe-refusal",
        )
        .await
        .is_ok_and(|turn| turn.calls.is_empty() && exact(&turn.text, "SAFE_REFUSAL_V1"));

    let vision = provider
        .image_turn(FmImageTask::QualificationShapes, &shape_fixture(), "png")
        .await
        .is_ok_and(|answer| answer.trim().eq_ignore_ascii_case(SHAPE_MARKER));
    let ocr = provider
        .image_turn(FmImageTask::QualificationOcr, &ocr_fixture(), "png")
        .await
        .is_ok_and(|answer| normalized_ocr(&answer) == OCR_MARKER);

    ProviderEvidence {
        configured: true,
        identity,
        vision_identity,
        capabilities: CapabilityEvidenceSet {
            text: if text && refusal {
                pass()
            } else {
                fail("semantic_text")
            },
            streaming: unsupported(),
            structured_output: if text && malformed_rejected {
                pass()
            } else {
                fail("structured_output")
            },
            tools: if tools && continuation {
                CapabilityEvidence::tools_pass(CONTINUATION_MARKER)
            } else {
                fail("tool_protocol")
            },
            vision: if vision {
                pass()
            } else {
                fail("semantic_vision")
            },
            ocr: if ocr { pass() } else { fail("semantic_ocr") },
        },
    }
}

/// Probes every configured FM mode in route order. The server binds to the
/// system mode only; `first` is the first mode and every mode is in `modes`.
pub(super) async fn probe() -> FmProbe {
    let route = match FmRoute::from_env() {
        Ok(Some(route)) => route,
        Ok(None) => return FmProbe::unconfigured("not_configured"),
        Err(_) => return FmProbe::unconfigured("invalid_configuration"),
    };
    let mut server = ProviderEvidence::skipped();
    let mut modes = Vec::with_capacity(route.instances.len());
    for config in &route.instances {
        let identity = fm_identity(config).ok();
        if config.endpoint.is_some() {
            server = probe_server(config, identity.clone()).await;
        }
        modes.push(probe_cli(config, identity).await);
    }
    let manifest_identity = route
        .instances
        .first()
        .and_then(|config| fm_manifest_identity(config).ok());
    let first = modes
        .first()
        .cloned()
        .unwrap_or_else(|| unavailable("not_configured"));
    FmProbe {
        server,
        first,
        modes,
        manifest_identity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::FIXTURE_VERSION;

    fn mode_evidence(mode: FmMode, passing: bool) -> ProviderEvidence {
        let identity = ProviderIdentity {
            endpoint: None,
            model: None,
            cli_path: Some("/usr/bin/fm".into()),
            cli_sha256: Some("0".repeat(64)),
            mode: Some(mode.as_str().into()),
            abbey_binary_sha256: "1".repeat(64),
            os_build: "build".into(),
            fixture_version: FIXTURE_VERSION.into(),
        };
        let outcome = |category| if passing { pass() } else { fail(category) };
        ProviderEvidence {
            configured: true,
            identity: Some(identity.clone()),
            vision_identity: Some(identity),
            capabilities: CapabilityEvidenceSet {
                text: outcome("semantic_text"),
                streaming: unsupported(),
                structured_output: outcome("structured_output"),
                tools: if passing {
                    CapabilityEvidence::tools_pass(CONTINUATION_MARKER)
                } else {
                    fail("tool_protocol")
                },
                vision: outcome("semantic_vision"),
                ocr: outcome("semantic_ocr"),
            },
        }
    }

    #[test]
    fn a_pcc_refusal_does_not_hide_a_passing_system_mode() {
        let pcc = mode_evidence(FmMode::Pcc, false);
        let modes = [pcc.clone(), mode_evidence(FmMode::System, true)];
        assert!(route_passes(&pcc, &modes, false));
        assert!(route_passes(&pcc, &modes, true));
    }

    #[test]
    fn the_route_fails_only_when_no_mode_qualifies() {
        let pcc = mode_evidence(FmMode::Pcc, false);
        let modes = [pcc.clone(), mode_evidence(FmMode::System, false)];
        assert!(!route_passes(&pcc, &modes, false));

        let pcc = mode_evidence(FmMode::Pcc, true);
        let modes = [pcc.clone(), mode_evidence(FmMode::System, false)];
        assert!(route_passes(&pcc, &modes, false));
    }

    #[test]
    fn fm_vision_requires_the_system_mode_images_to_pass() {
        let pcc = mode_evidence(FmMode::Pcc, true);
        let mut system = mode_evidence(FmMode::System, true);
        system.capabilities.vision = fail("semantic_vision");
        let modes = [pcc.clone(), system];
        assert!(route_passes(&pcc, &modes, false));
        assert!(
            !route_passes(&pcc, &modes, true),
            "a passing PCC mode must not satisfy FM vision, which binds to system"
        );
    }

    #[test]
    fn unconfigured_or_legacy_single_mode_evidence_uses_the_first_mode() {
        let unconfigured = unavailable("not_configured");
        assert!(!route_passes(&unconfigured, &[], false));
        assert!(route_passes(
            &mode_evidence(FmMode::System, true),
            &[],
            false
        ));
    }
}

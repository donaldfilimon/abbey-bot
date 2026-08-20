//! No-Discord, no-state provider qualification with synthetic fixtures only.

use crate::llm::{
    self, Backend, ChatTurn, HttpTransport, StreamTransport as _, build_stream_request,
};
use crate::provider::{
    CapabilityEvidence, CapabilityEvidenceSet, FIXTURE_VERSION, FmConfig, FmImageTask,
    FoundationModels, ProviderEvidence, ProviderIdentity, QUALIFICATION_VERSION,
    QualificationReport, QualificationTarget, fm_identity, primary_identity, unix_now,
};
use crate::tools::{ToolResult, ToolSpec};
use crate::vision::{
    VisionConfig, VisionProviderChoice, VisionTask, VisionTransport as _, extract_vision_text,
};

use serde_json::json;

const TEXT_MARKER: &str = "ABBEY_PROVIDER_TEXT_V1";
const STREAM_MARKER: &str = "ABBEY_PROVIDER_STREAM_V1";
const CONTINUATION_MARKER: &str = "ABBEY_PROVIDER_CONTINUATION_V1";
const TOOL_NONCE: &str = "abbey-provider-probe-v1";
const OCR_MARKER: &str = "ABBEY427";
const SHAPE_MARKER: &str = "red square, blue circle";

fn pass() -> CapabilityEvidence {
    CapabilityEvidence::pass()
}

fn fail(category: &'static str) -> CapabilityEvidence {
    CapabilityEvidence::fail(category)
}

fn unsupported() -> CapabilityEvidence {
    CapabilityEvidence::unsupported()
}

fn evidence_failed(evidence: &ProviderEvidence) -> bool {
    use crate::provider::ProbeStatus;
    [
        &evidence.capabilities.text,
        &evidence.capabilities.streaming,
        &evidence.capabilities.structured_output,
        &evidence.capabilities.tools,
        &evidence.capabilities.vision,
        &evidence.capabilities.ocr,
    ]
    .into_iter()
    .any(|item| matches!(item.status, ProbeStatus::Fail))
}

fn evidence_has_configuration_failure(evidence: &ProviderEvidence) -> bool {
    [
        &evidence.capabilities.text,
        &evidence.capabilities.streaming,
        &evidence.capabilities.structured_output,
        &evidence.capabilities.tools,
        &evidence.capabilities.vision,
        &evidence.capabilities.ocr,
    ]
    .into_iter()
    .any(|item| {
        matches!(
            item.category.as_deref(),
            Some("invalid_configuration" | "invalid_vision_configuration" | "identity_unavailable")
        )
    })
}

fn passing_image_evidence_is_bound(evidence: &ProviderEvidence) -> bool {
    !(evidence.capabilities.vision.passed() || evidence.capabilities.ocr.passed())
        || evidence.vision_identity.is_some()
}

fn fm_cli_required_capabilities_pass(evidence: &ProviderEvidence, vision_required: bool) -> bool {
    evidence.capabilities.text.passed()
        && evidence.capabilities.structured_output.passed()
        && evidence.capabilities.tools.passed()
        && passing_image_evidence_is_bound(evidence)
        && (!vision_required
            || (evidence.capabilities.vision.passed() && evidence.capabilities.ocr.passed()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfTestExit {
    Success,
    ProbeFailure,
    Configuration,
}

impl SelfTestExit {
    pub const fn code(self) -> i32 {
        match self {
            Self::Success => 0,
            Self::ProbeFailure => 1,
            Self::Configuration => 2,
        }
    }
}

pub struct SelfTestOutcome {
    pub report: QualificationReport,
    pub exit: SelfTestExit,
}

fn unavailable(category: &'static str) -> ProviderEvidence {
    ProviderEvidence {
        configured: false,
        identity: None,
        vision_identity: None,
        capabilities: CapabilityEvidenceSet {
            text: fail(category),
            streaming: CapabilityEvidence::skipped(),
            structured_output: CapabilityEvidence::skipped(),
            tools: CapabilityEvidence::skipped(),
            vision: CapabilityEvidence::skipped(),
            ocr: CapabilityEvidence::skipped(),
        },
    }
}

fn probe_tool() -> ToolSpec {
    ToolSpec {
        name: "probe_status",
        description: "Return the exact synthetic qualification nonce. This probe has no side effects.",
        parameters: json!({
            "type": "object",
            "properties": {
                "nonce": {"type": "string", "enum": [TOOL_NONCE]}
            },
            "required": ["nonce"],
            "additionalProperties": false
        }),
    }
}

fn exact(value: &str, expected: &str) -> bool {
    value.trim() == expected
}

fn normalized_ocr(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_uppercase)
        .collect()
}

fn remote_vision_identity(config: &VisionConfig) -> Result<ProviderIdentity, String> {
    primary_identity(config.base_url.clone(), config.model.clone())
}

async fn probe_stream(backend: &Backend, expected: &str) -> bool {
    if !matches!(backend, Backend::OpenAiCompatible { .. }) {
        return false;
    }
    let request = build_stream_request(
        backend,
        "Return exactly the requested marker and nothing else.",
        &[ChatTurn::user(format!("Return exactly {expected}"))],
        &[],
    );
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let transport = HttpTransport::default();
    let response = transport.post_stream(&request, sender).await;
    while receiver.recv().await.is_some() {}
    response.is_ok_and(|turn| turn.calls.is_empty() && exact(&turn.text, expected))
}

async fn probe_primary() -> ProviderEvidence {
    let Some(backend) = Backend::from_env() else {
        return unavailable("not_configured");
    };
    if backend.validate().is_err() {
        return unavailable("invalid_configuration");
    }
    let identity = match &backend {
        Backend::OpenAiCompatible { endpoint, model } => {
            primary_identity(endpoint.clone(), model.clone())
        }
        Backend::Anthropic { .. } => primary_identity(
            "https://api.anthropic.com/v1/messages".into(),
            "claude-sonnet-5".into(),
        ),
    }
    .ok();
    let transport = HttpTransport::default();
    let text = llm::chat_backend(
        &transport,
        &backend,
        "Return exactly the requested marker and nothing else.",
        &[ChatTurn::user(format!("Return exactly {TEXT_MARKER}"))],
    )
    .await
    .is_ok_and(|answer| exact(&answer, TEXT_MARKER));
    let streaming = if matches!(backend, Backend::OpenAiCompatible { .. }) {
        if probe_stream(&backend, STREAM_MARKER).await {
            pass()
        } else {
            fail("stream_protocol")
        }
    } else {
        unsupported()
    };
    let tool = probe_tool();
    let mut tools_pass = false;
    let first = llm::chat_turn(
        &transport,
        &backend,
        "Call probe_status exactly once with the supplied nonce. Do not answer yet.",
        &[ChatTurn::user(format!("Use nonce {TOOL_NONCE}"))],
        std::slice::from_ref(&tool),
    )
    .await;
    if let Ok(turn) = first
        && turn.text.trim().is_empty()
        && turn.calls.len() == 1
        && turn.calls[0].name == "probe_status"
        && turn.calls[0].arguments == json!({"nonce": TOOL_NONCE})
    {
        let mut turns = vec![
            ChatTurn::user(format!("Use nonce {TOOL_NONCE}")),
            ChatTurn::assistant_calls("", turn.calls.clone()),
        ];
        turns.push(ChatTurn::tool_result(&ToolResult {
            call_id: turn.calls[0].id.clone(),
            name: "probe_status".into(),
            content: format!("synthetic probe succeeded; return exactly {CONTINUATION_MARKER}"),
        }));
        tools_pass = llm::chat_turn(
            &transport,
            &backend,
            "After the synthetic tool result, return exactly the requested marker.",
            &turns,
            &[],
        )
        .await
        .is_ok_and(|answer| answer.calls.is_empty() && exact(&answer.text, CONTINUATION_MARKER));
    }

    let (vision, ocr, vision_identity) = match VisionProviderChoice::from_env() {
        Ok(VisionProviderChoice::Remote) => match VisionConfig::from_env() {
            Some(config)
                if crate::llm::validate_remote_endpoint(
                    &config.base_url,
                    "ABBEY_VISION_ENDPOINT",
                )
                .is_ok() =>
            {
                match remote_vision_identity(&config) {
                    Ok(vision_identity) => {
                        let transport = crate::runtime::HttpVisionTransport::default();
                        let shape = crate::vision::image::data_url_unchecked(&shape_fixture());
                        let vision = transport
                            .post(&config.request(VisionTask::QualificationShapes, shape))
                            .await
                            .and_then(|raw| extract_vision_text(&raw))
                            .is_ok_and(|answer| answer.trim().eq_ignore_ascii_case(SHAPE_MARKER));
                        let text = crate::vision::image::data_url_unchecked(&ocr_fixture());
                        let ocr = transport
                            .post(&config.request(VisionTask::QualificationOcr, text))
                            .await
                            .and_then(|raw| extract_vision_text(&raw))
                            .is_ok_and(|answer| normalized_ocr(&answer) == OCR_MARKER);
                        (
                            if vision {
                                pass()
                            } else {
                                fail("semantic_vision")
                            },
                            if ocr { pass() } else { fail("semantic_ocr") },
                            Some(vision_identity),
                        )
                    }
                    Err(_) => (
                        fail("identity_unavailable"),
                        fail("identity_unavailable"),
                        None,
                    ),
                }
            }
            Some(_) => (
                fail("invalid_vision_configuration"),
                fail("invalid_vision_configuration"),
                None,
            ),
            None => (unsupported(), unsupported(), None),
        },
        Ok(VisionProviderChoice::Off | VisionProviderChoice::FoundationModels) => {
            (unsupported(), unsupported(), None)
        }
        Err(_) => (
            fail("invalid_vision_configuration"),
            fail("invalid_vision_configuration"),
            None,
        ),
    };

    ProviderEvidence {
        configured: true,
        identity,
        vision_identity,
        capabilities: CapabilityEvidenceSet {
            text: if text { pass() } else { fail("semantic_text") },
            streaming,
            structured_output: if tools_pass {
                pass()
            } else {
                fail("structured_output")
            },
            tools: if tools_pass {
                pass()
            } else {
                fail("tool_protocol")
            },
            vision,
            ocr,
        },
    }
}

async fn probe_fm_server(
    config: &FmConfig,
    identity: Option<ProviderIdentity>,
) -> ProviderEvidence {
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

async fn probe_fm_cli(config: &FmConfig, identity: Option<ProviderIdentity>) -> ProviderEvidence {
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
                pass()
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

async fn probe_fm() -> (ProviderEvidence, ProviderEvidence) {
    let config = match FmConfig::from_env() {
        Ok(Some(config)) => config,
        Ok(None) => return (ProviderEvidence::skipped(), unavailable("not_configured")),
        Err(_) => {
            return (
                ProviderEvidence::skipped(),
                unavailable("invalid_configuration"),
            );
        }
    };
    if matches!(config.mode, crate::provider::FmMode::Pcc) {
        return (
            ProviderEvidence::skipped(),
            unavailable("pcc_not_qualified"),
        );
    }
    let identity = fm_identity(&config).ok();
    let server = probe_fm_server(&config, identity.clone()).await;
    let cli = probe_fm_cli(&config, identity).await;
    (server, cli)
}

pub async fn run(target: QualificationTarget) -> SelfTestOutcome {
    let vision_choice = VisionProviderChoice::from_env();
    let configuration_error = vision_choice.is_err();
    let primary = if target.includes_primary() {
        probe_primary().await
    } else {
        ProviderEvidence::skipped()
    };
    let (fm_server, fm_cli) = if target.includes_fm() {
        probe_fm().await
    } else {
        (ProviderEvidence::skipped(), ProviderEvidence::skipped())
    };
    let fm_vision_required = matches!(vision_choice, Ok(VisionProviderChoice::FoundationModels));
    let overall_pass = (!target.includes_primary()
        || (primary.configured
            && !evidence_failed(&primary)
            && passing_image_evidence_is_bound(&primary)))
        && (!target.includes_fm()
            || (fm_cli.configured
                && fm_cli_required_capabilities_pass(&fm_cli, fm_vision_required)))
        && (!fm_server.configured || !evidence_failed(&fm_server));
    let report = QualificationReport {
        version: QUALIFICATION_VERSION,
        fixture_version: FIXTURE_VERSION.into(),
        generated_unix_secs: unix_now(),
        target,
        overall_pass,
        primary,
        fm_server,
        fm_cli,
    };
    SelfTestOutcome {
        exit: if configuration_error {
            SelfTestExit::Configuration
        } else {
            classify_exit(&report)
        },
        report,
    }
}

fn classify_exit(report: &QualificationReport) -> SelfTestExit {
    let selected_routes_configured = (!report.target.includes_primary()
        || (report.primary.configured && report.primary.identity.is_some()))
        && (!report.target.includes_fm()
            || (report.fm_cli.configured && report.fm_cli.identity.is_some()));
    let selected_configuration_failed = (report.target.includes_primary()
        && evidence_has_configuration_failure(&report.primary))
        || (report.target.includes_fm()
            && (evidence_has_configuration_failure(&report.fm_cli)
                || (report.fm_server.configured
                    && evidence_has_configuration_failure(&report.fm_server))));
    let selected_image_identity_missing = (report.target.includes_primary()
        && !passing_image_evidence_is_bound(&report.primary))
        || (report.target.includes_fm() && !passing_image_evidence_is_bound(&report.fm_cli));
    if !selected_routes_configured
        || selected_configuration_failed
        || selected_image_identity_missing
    {
        SelfTestExit::Configuration
    } else if report.overall_pass {
        SelfTestExit::Success
    } else {
        SelfTestExit::ProbeFailure
    }
}

fn shape_fixture() -> Vec<u8> {
    let mut image = image::RgbImage::from_pixel(256, 128, image::Rgb([255, 255, 255]));
    for y in 24..104 {
        for x in 24..104 {
            image.put_pixel(x, y, image::Rgb([224, 20, 20]));
        }
    }
    for y in 16_i32..112 {
        for x in 144_i32..240 {
            let dx = x - 192;
            let dy = y - 64;
            if dx * dx + dy * dy <= 40 * 40 {
                image.put_pixel(x as u32, y as u32, image::Rgb([20, 70, 224]));
            }
        }
    }
    encode_png(image)
}

fn ocr_fixture() -> Vec<u8> {
    let mut image = image::RgbImage::from_pixel(420, 100, image::Rgb([255, 255, 255]));
    for (index, character) in OCR_MARKER.chars().enumerate() {
        let glyph = glyph(character);
        let origin_x = 12 + u32::try_from(index).unwrap_or(0) * 48;
        for (row, bits) in glyph.iter().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) != 0 {
                    for dy in 0..10 {
                        for dx in 0..7 {
                            image.put_pixel(
                                origin_x + column * 7 + dx,
                                12 + u32::try_from(row).unwrap_or(0) * 10 + dy,
                                image::Rgb([0, 0, 0]),
                            );
                        }
                    }
                }
            }
        }
    }
    encode_png(image)
}

fn encode_png(image: image::RgbImage) -> Vec<u8> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut cursor, image::ImageFormat::Png)
        .expect("synthetic qualification image encodes");
    cursor.into_inner()
}

fn glyph(character: char) -> [u8; 7] {
    match character {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        '4' => [
            0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010, 0b00010,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        _ => [0; 7],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_are_valid_provider_images() {
        assert!(shape_fixture().starts_with(&[0x89, b'P', b'N', b'G']));
        assert!(ocr_fixture().starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn normalization_accepts_spacing_but_not_wrong_text() {
        assert_eq!(normalized_ocr("Abbey 427\n"), OCR_MARKER);
        assert_ne!(normalized_ocr("Abbey 472"), OCR_MARKER);
    }

    #[test]
    fn unavailable_evidence_uses_only_fixed_category() {
        let evidence = unavailable("not_configured");
        let json = serde_json::to_string(&evidence).unwrap();
        assert!(json.contains("not_configured"));
        assert!(!json.contains("DISCORD_TOKEN"));
    }

    #[test]
    fn remote_vision_identity_is_distinct_and_exact() {
        let text = primary_identity("http://127.0.0.1:8282".into(), "text-model".into()).unwrap();
        let config = VisionConfig::from_values(
            Some("https://vision.example.test/v1".into()),
            Some("vision-model".into()),
            None,
            None,
            None,
        )
        .unwrap();
        let vision = remote_vision_identity(&config).unwrap();
        let evidence = ProviderEvidence {
            configured: true,
            identity: Some(text),
            vision_identity: Some(vision),
            capabilities: CapabilityEvidenceSet {
                text: pass(),
                streaming: pass(),
                structured_output: pass(),
                tools: pass(),
                vision: pass(),
                ocr: pass(),
            },
        };
        assert!(passing_image_evidence_is_bound(&evidence));
        let encoded = serde_json::to_value(&evidence).unwrap();
        assert_eq!(encoded["identity"]["endpoint"], "http://127.0.0.1:8282");
        assert_eq!(encoded["identity"]["model"], "text-model");
        assert_eq!(
            encoded["vision_identity"]["endpoint"],
            "https://vision.example.test/v1"
        );
        assert_eq!(encoded["vision_identity"]["model"], "vision-model");
    }

    #[test]
    fn exit_classification_separates_configuration_from_probe_failure() {
        let mut report = QualificationReport {
            version: QUALIFICATION_VERSION,
            fixture_version: FIXTURE_VERSION.into(),
            generated_unix_secs: 1,
            target: QualificationTarget::Fm,
            overall_pass: false,
            primary: ProviderEvidence::skipped(),
            fm_server: ProviderEvidence::skipped(),
            fm_cli: unavailable("not_configured"),
        };
        assert_eq!(classify_exit(&report), SelfTestExit::Configuration);

        report.fm_cli.configured = true;
        report.fm_cli.identity = Some(ProviderIdentity {
            endpoint: None,
            model: None,
            cli_path: Some("/usr/bin/fm".into()),
            cli_sha256: Some("0".repeat(64)),
            mode: Some("system".into()),
            abbey_binary_sha256: "1".repeat(64),
            os_build: "build".into(),
            fixture_version: FIXTURE_VERSION.into(),
        });
        assert_eq!(classify_exit(&report), SelfTestExit::ProbeFailure);
        report.target = QualificationTarget::Primary;
        report.primary = ProviderEvidence {
            configured: true,
            identity: report.fm_cli.identity.clone(),
            vision_identity: None,
            capabilities: CapabilityEvidenceSet {
                text: pass(),
                streaming: pass(),
                structured_output: pass(),
                tools: pass(),
                vision: fail("invalid_vision_configuration"),
                ocr: fail("invalid_vision_configuration"),
            },
        };
        assert_eq!(classify_exit(&report), SelfTestExit::Configuration);

        report.primary.capabilities.vision = pass();
        report.primary.capabilities.ocr = pass();
        assert_eq!(classify_exit(&report), SelfTestExit::Configuration);

        report.primary.vision_identity = report.primary.identity.clone();
        report.overall_pass = true;
        assert_eq!(classify_exit(&report), SelfTestExit::Success);

        report.primary.capabilities.vision = fail("semantic_vision");
        report.primary.capabilities.ocr = fail("semantic_ocr");
        report.overall_pass = false;
        assert_eq!(classify_exit(&report), SelfTestExit::ProbeFailure);
        report.overall_pass = true;
        assert_eq!(classify_exit(&report), SelfTestExit::Success);
    }
}

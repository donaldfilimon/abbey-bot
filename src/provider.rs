//! Provider runtime contracts plus the compatible Foundation Models route.
//!
//! Generic provider configuration, discovery, qualification, and routing stay
//! behind Abbey's existing turn and tool vocabulary. Foundation Models is
//! never selected merely because `/usr/bin/fm` or a server happens to exist.
//! The operator must select `system` or `pcc` and separately enable fallback.
//! The HTTP server and `fm respond` CLI remain separate capabilities: the
//! server is text-only here and can never inherit CLI tool capability.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use crate::llm::{Backend, ChatTurn, LlmError, ModelTurn, Role};

mod catalog;
mod config;
mod discovery;
mod domain;
mod manifest;
mod qualification;
pub use catalog::{CatalogPolicy, ProviderCatalog};
pub use config::{ProviderConfig, ProviderConfigError, ProviderCredential, ProviderSettings};
pub use discovery::{
    DiscoveryLimits, DiscoveryRequest, DiscoveryResult, ExecutableIdentity, discover,
};
pub use domain::{
    BlockedReason, DetectionState, DiscoveryBoundary, Eligibility, IsolationCapabilities,
    ProviderClass, ProviderDescriptor, ProviderId, ProviderIdError, ProviderProvenance,
    TemporaryUnavailableReason, TurnAdapter, TurnFuture,
};
pub use manifest::{
    DeclaredCapabilities, ManifestDocument, ManifestError, PROVIDER_MANIFEST_VERSION,
    ProviderIdentityHashes, ProviderManifest, ProviderRecord, QualificationStatus,
    QualifiedIsolation, publish_v2, read_manifest,
};
pub use qualification::{
    CapabilityEvidence, CapabilityEvidenceSet, FIXTURE_VERSION, ProbeStatus, ProviderEvidence,
    ProviderIdentity, QUALIFICATION_VERSION, QualificationReport, QualificationTarget,
    VerifiedFmCapabilities, fm_identity, fm_manifest_identity, primary_identity, unix_now,
    verify_fm_manifest,
};

const DEFAULT_FM_CLI: &str = "/usr/bin/fm";
const DEFAULT_TIMEOUT_SECS: u64 = 300;
const STATIC_FM_INSTRUCTIONS: &str = "Follow the policy and conversation JSON supplied on stdin. Return only the schema-guided decision.";
const MAX_STDOUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 4 * 1024;
const ALLOWED_ENVIRONMENT: &[&str] = &[
    "HOME",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "__CF_USER_TEXT_ENCODING",
];
static NEXT_SCHEMA_FILE: AtomicU64 = AtomicU64::new(0);
static NEXT_IMAGE_FILE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FmImageTask {
    Describe,
    ExtractText,
    QualificationShapes,
    QualificationOcr,
}

impl FmImageTask {
    const fn prompt(self) -> &'static str {
        match self {
            Self::Describe => {
                "Describe this image in at most two short sentences. Factual, no preamble."
            }
            Self::ExtractText => {
                "Transcribe all text visible in this image verbatim. Output only the text."
            }
            Self::QualificationShapes => {
                "Identify the two colored shapes from left to right. Output only two lowercase color-and-shape labels separated by a comma and one space."
            }
            Self::QualificationOcr => {
                "Transcribe the image. Output exactly the visible ASCII text and nothing else."
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FmMode {
    System,
    Pcc,
}

impl FmMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Pcc => "pcc",
        }
    }
}

/// Validated operator configuration. This type contains no credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FmConfig {
    pub mode: FmMode,
    pub endpoint: Option<String>,
    pub cli: PathBuf,
    pub fallback: bool,
    pub timeout_secs: u64,
}

impl FmConfig {
    pub fn from_values(
        mode: Option<String>,
        endpoint: Option<String>,
        cli: Option<String>,
        fallback: Option<String>,
        timeout_secs: Option<String>,
    ) -> Result<Option<Self>, String> {
        let value = |raw: Option<String>| {
            raw.map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let fallback = match value(fallback).as_deref() {
            None | Some("0" | "false" | "off") => false,
            Some("1" | "true" | "on") => true,
            Some(_) => return Err("ABBEY_FM_FALLBACK must be 1 or 0".into()),
        };
        let mode = match value(mode).as_deref() {
            None | Some("off") => {
                if fallback {
                    return Err("ABBEY_FM_FALLBACK=1 requires ABBEY_FM_MODE=system or pcc".into());
                }
                return Ok(None);
            }
            Some("system") => FmMode::System,
            Some("pcc") => FmMode::Pcc,
            Some(_) => return Err("ABBEY_FM_MODE must be off, system, or pcc".into()),
        };

        let endpoint = value(endpoint)
            .map(|endpoint| validate_fm_endpoint(&endpoint).map(|()| endpoint))
            .transpose()?;
        let cli = PathBuf::from(value(cli).unwrap_or_else(|| DEFAULT_FM_CLI.to_string()));
        if !cli.is_absolute() || cli.components().any(|part| part == Component::ParentDir) {
            return Err("ABBEY_FM_CLI must be an absolute path without `..`".into());
        }
        let timeout_secs = match value(timeout_secs) {
            None => DEFAULT_TIMEOUT_SECS,
            Some(raw) => raw
                .parse::<u64>()
                .ok()
                .filter(|seconds| *seconds > 0)
                .ok_or_else(|| {
                    "ABBEY_BOT_LLM_TIMEOUT_SECS must be a positive integer for FM".to_string()
                })?,
        };
        Ok(Some(Self {
            mode,
            endpoint,
            cli,
            fallback,
            timeout_secs,
        }))
    }

    pub fn from_env() -> Result<Option<Self>, String> {
        let config = Self::from_values(
            std::env::var("ABBEY_FM_MODE").ok(),
            std::env::var("ABBEY_FM_ENDPOINT").ok(),
            std::env::var("ABBEY_FM_CLI").ok(),
            std::env::var("ABBEY_FM_FALLBACK").ok(),
            std::env::var("ABBEY_BOT_LLM_TIMEOUT_SECS").ok(),
        )?;
        #[cfg(not(target_os = "macos"))]
        if config.is_some() {
            return Err(
                "Apple Foundation Models is supported only on macOS; set ABBEY_FM_MODE=off".into(),
            );
        }
        Ok(config)
    }
}

fn validate_fm_endpoint(raw: &str) -> Result<(), String> {
    crate::llm::validate_remote_endpoint(raw, "ABBEY_FM_ENDPOINT")?;
    let url = reqwest::Url::parse(raw)
        .map_err(|_| "ABBEY_FM_ENDPOINT must be a valid absolute URL".to_string())?;
    if !crate::llm::url_is_loopback(&url) {
        return Err("ABBEY_FM_ENDPOINT must target loopback".into());
    }
    if url.path() != "/" && !url.path().is_empty() {
        return Err("ABBEY_FM_ENDPOINT must be a server base URL without a path".into());
    }
    Ok(())
}

/// Independently qualified provider behavior.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub text: bool,
    pub streaming: bool,
    pub structured_output: bool,
    pub tools: bool,
    pub vision: bool,
    pub ocr: bool,
}

impl ProviderCapabilities {
    #[must_use]
    pub const fn satisfies(self, required: Self) -> bool {
        (!required.text || self.text)
            && (!required.streaming || self.streaming)
            && (!required.structured_output || self.structured_output)
            && (!required.tools || self.tools)
            && (!required.vision || self.vision)
            && (!required.ocr || self.ocr)
    }

    #[must_use]
    pub const fn primary(backend: &Backend, tools: bool) -> Self {
        Self {
            text: true,
            streaming: matches!(backend, Backend::OpenAiCompatible { .. }),
            structured_output: tools,
            tools,
            vision: false,
            ocr: false,
        }
    }

    #[must_use]
    pub const fn text() -> Self {
        Self {
            text: true,
            streaming: false,
            structured_output: false,
            tools: false,
            vision: false,
            ocr: false,
        }
    }

    #[must_use]
    pub const fn text_with_tools() -> Self {
        Self {
            text: true,
            streaming: false,
            structured_output: true,
            tools: true,
            vision: false,
            ocr: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRoute {
    Primary,
    FoundationModelsServer,
    FoundationModelsCli,
}

/// Immutable transport evidence plus provider-local runtime feature state.
pub struct ProviderRouter {
    primary: Option<ProviderCapabilities>,
    fm_server: Option<ProviderCapabilities>,
    fm_cli: Option<ProviderCapabilities>,
    fm_fallback: bool,
    primary_tools_enabled: AtomicBool,
    fm_cli_tools_enabled: AtomicBool,
}

impl ProviderRouter {
    #[must_use]
    pub fn new(
        primary_backend: Option<&Backend>,
        primary_tools_enabled: bool,
        fm_server: Option<ProviderCapabilities>,
        fm_cli: Option<ProviderCapabilities>,
        fm_fallback: bool,
    ) -> Self {
        Self {
            primary: primary_backend
                .map(|backend| ProviderCapabilities::primary(backend, primary_tools_enabled)),
            fm_server: fm_server.map(|caps| ProviderCapabilities {
                structured_output: false,
                tools: false,
                ..caps
            }),
            fm_cli,
            fm_fallback,
            primary_tools_enabled: AtomicBool::new(primary_tools_enabled),
            fm_cli_tools_enabled: AtomicBool::new(
                fm_cli.is_some_and(|capabilities| capabilities.tools),
            ),
        }
    }

    #[must_use]
    pub fn candidates(&self, required: ProviderCapabilities) -> Vec<ProviderRoute> {
        let mut routes = Vec::with_capacity(3);
        for route in [
            ProviderRoute::Primary,
            ProviderRoute::FoundationModelsServer,
            ProviderRoute::FoundationModelsCli,
        ] {
            if self
                .routable_capabilities(route)
                .is_some_and(|capabilities| capabilities.satisfies(required))
            {
                routes.push(route);
            }
        }
        routes
    }

    #[must_use]
    pub fn effective_capabilities(&self, route: ProviderRoute) -> Option<ProviderCapabilities> {
        let mut capabilities = match route {
            ProviderRoute::Primary => self.primary?,
            ProviderRoute::FoundationModelsServer => self.fm_server?,
            ProviderRoute::FoundationModelsCli => self.fm_cli?,
        };
        let tools_enabled = match route {
            ProviderRoute::Primary => self.primary_tools_enabled.load(Ordering::Relaxed),
            ProviderRoute::FoundationModelsCli => self.fm_cli_tools_enabled.load(Ordering::Relaxed),
            ProviderRoute::FoundationModelsServer => false,
        };
        if !tools_enabled {
            capabilities.tools = false;
        }
        Some(capabilities)
    }

    /// Capabilities that may actually be selected by the current routing
    /// policy. Unlike [`Self::effective_capabilities`], this also applies the
    /// explicit Foundation Models fallback boundary.
    #[must_use]
    pub fn routable_capabilities(&self, route: ProviderRoute) -> Option<ProviderCapabilities> {
        if !self.fm_fallback && !matches!(route, ProviderRoute::Primary) {
            return None;
        }
        self.effective_capabilities(route)
    }

    pub fn disable_tools(&self, route: ProviderRoute) {
        match route {
            ProviderRoute::Primary => self.primary_tools_enabled.store(false, Ordering::Relaxed),
            ProviderRoute::FoundationModelsCli => {
                self.fm_cli_tools_enabled.store(false, Ordering::Relaxed);
            }
            ProviderRoute::FoundationModelsServer => {}
        }
    }
}

pub struct FoundationModels {
    pub config: FmConfig,
    pub router: ProviderRouter,
    qualified: bool,
}

impl FoundationModels {
    #[must_use]
    pub fn new(
        config: FmConfig,
        primary_backend: Option<&Backend>,
        primary_tools_enabled: bool,
    ) -> Self {
        let server = config.endpoint.as_ref().map(|_| ProviderCapabilities {
            text: true,
            streaming: true,
            ..ProviderCapabilities::default()
        });
        let cli = Some(ProviderCapabilities::text_with_tools());
        let router = ProviderRouter::new(
            primary_backend,
            primary_tools_enabled,
            server,
            cli,
            config.fallback,
        );
        Self {
            config,
            router,
            qualified: false,
        }
    }

    #[must_use]
    pub fn new_qualified(
        config: FmConfig,
        primary_backend: Option<&Backend>,
        primary_tools_enabled: bool,
        qualified: VerifiedFmCapabilities,
    ) -> Self {
        let router = ProviderRouter::new(
            primary_backend,
            primary_tools_enabled,
            qualified.server,
            Some(qualified.cli),
            config.fallback,
        );
        Self {
            config,
            router,
            qualified: true,
        }
    }

    /// Whether the capabilities were loaded from a verified qualification
    /// manifest rather than inferred from explicit configuration alone.
    #[must_use]
    pub const fn is_qualified(&self) -> bool {
        self.qualified
    }

    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self.config.mode {
            FmMode::System => "Apple Foundation Models on-device model",
            FmMode::Pcc => "Apple Foundation Models Private Cloud Compute",
        }
    }

    #[must_use]
    pub fn server_backend(&self) -> Option<Backend> {
        self.config
            .endpoint
            .as_ref()
            .map(|endpoint| Backend::OpenAiCompatible {
                endpoint: endpoint.clone(),
                model: self.config.mode.as_str().to_string(),
            })
    }

    pub async fn cli_turn(
        &self,
        system_prompt: &str,
        turns: &[ChatTurn],
        tools: &[crate::tools::ToolSpec],
        call_id: &str,
    ) -> Result<ModelTurn, LlmError> {
        let schema = decision_schema(tools)?;
        let transcript = render_transcript(system_prompt, turns)?;
        let file = PrivateSchemaFile::create(&schema).map_err(|error| {
            LlmError::backend(format!("could not prepare the FM response schema: {error}"))
        })?;
        let invocation = CliInvocation::new(&self.config, &transcript, file.path());
        let output = invocation.run(self.config.timeout_secs).await?;
        parse_cli_output(&output, tools, call_id)
    }

    pub async fn image_turn(
        &self,
        task: FmImageTask,
        bytes: &[u8],
        extension: &str,
    ) -> Result<String, LlmError> {
        if !matches!(extension, "jpg" | "png" | "webp") {
            return Err(LlmError::backend(
                "the FM image adapter received an unsupported prepared format".into(),
            ));
        }
        let file = PrivateImageFile::create(bytes, extension).map_err(|error| {
            LlmError::backend(format!("could not prepare the private FM image: {error}"))
        })?;
        let invocation = CliInvocation::for_image(&self.config, task, file.path());
        let output = invocation.run(self.config.timeout_secs).await?;
        let output = output.trim();
        if output.is_empty()
            && !matches!(
                task,
                FmImageTask::ExtractText | FmImageTask::QualificationOcr
            )
        {
            return Err(LlmError::backend(
                "the FM CLI returned an empty image description".into(),
            ));
        }
        Ok(output.to_string())
    }
}

/// Fully separated program, argv, and stdin. There is deliberately no shell.
struct CliInvocation {
    program: PathBuf,
    args: Vec<OsString>,
    stdin: Vec<u8>,
    environment: Vec<(OsString, OsString)>,
}

impl CliInvocation {
    fn new(config: &FmConfig, prompt: &str, schema: &Path) -> Self {
        Self {
            program: config.cli.clone(),
            args: vec![
                "respond".into(),
                "--model".into(),
                config.mode.as_str().into(),
                "--no-stream".into(),
                "--instructions".into(),
                STATIC_FM_INSTRUCTIONS.into(),
                "--schema".into(),
                schema.as_os_str().to_owned(),
            ],
            stdin: prompt.as_bytes().to_vec(),
            environment: filtered_environment(std::env::vars_os()),
        }
    }

    fn for_image(config: &FmConfig, task: FmImageTask, image: &Path) -> Self {
        let mut args = vec![
            "respond".into(),
            "--model".into(),
            config.mode.as_str().into(),
            "--no-stream".into(),
            "--instructions".into(),
            "Return only the requested image result without commentary.".into(),
            "--image".into(),
            image.as_os_str().to_owned(),
        ];
        if matches!(
            task,
            FmImageTask::ExtractText | FmImageTask::QualificationOcr
        ) {
            args.extend([OsString::from("--tool"), OsString::from("ocr")]);
        }
        Self {
            program: config.cli.clone(),
            args,
            stdin: task.prompt().as_bytes().to_vec(),
            environment: filtered_environment(std::env::vars_os()),
        }
    }

    async fn run(self, timeout_secs: u64) -> Result<String, LlmError> {
        let mut command = tokio::process::Command::new(&self.program);
        command
            .args(&self.args)
            .env_clear()
            .envs(self.environment.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|error| {
            LlmError::backend(format!("could not start the configured FM CLI: {error}"))
        })?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| LlmError::backend("the FM CLI stdin pipe was unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| LlmError::backend("the FM CLI stdout pipe was unavailable".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| LlmError::backend("the FM CLI stderr pipe was unavailable".into()))?;
        let input = self.stdin;
        let operation = async {
            let write = async move {
                stdin.write_all(&input).await?;
                stdin.shutdown().await
            };
            let ((), stdout, _stderr, status) = tokio::try_join!(
                async { write.await.map_err(|error| error.to_string()) },
                read_capped(stdout, MAX_STDOUT_BYTES),
                read_capped(stderr, MAX_STDERR_BYTES),
                async { child.wait().await.map_err(|error| error.to_string()) },
            )?;
            if !status.success() {
                return Err(format!(
                    "the FM CLI exited unsuccessfully ({})",
                    status
                        .code()
                        .map_or_else(|| "signal".into(), |code| code.to_string())
                ));
            }
            String::from_utf8(stdout)
                .map_err(|_| "the FM CLI returned stdout that was not UTF-8".to_string())
        };
        match tokio::time::timeout(Duration::from_secs(timeout_secs), operation).await {
            Ok(Ok(output)) => Ok(output),
            Ok(Err(error)) => Err(LlmError::backend(error)),
            Err(_) => Err(LlmError::backend("the FM CLI timed out".into())),
        }
    }
}

fn filtered_environment(
    values: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
    values
        .into_iter()
        .filter(|(name, _)| {
            ALLOWED_ENVIRONMENT
                .iter()
                .any(|allowed| name == std::ffi::OsStr::new(allowed))
        })
        .collect()
}

async fn read_capped(mut reader: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(limit.min(8192));
    let mut chunk = [0_u8; 8192];
    loop {
        let count = reader
            .read(&mut chunk)
            .await
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes.len().saturating_add(count) > limit {
            return Err(format!("the FM CLI output exceeded {limit} bytes"));
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}

struct PrivateSchemaFile(PathBuf);

impl PrivateSchemaFile {
    fn create(schema: &Value) -> std::io::Result<Self> {
        let serial = NEXT_SCHEMA_FILE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            ".abbey-fm-schema-{}-{serial}.json",
            std::process::id()
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        serde_json::to_writer(&mut file, schema)?;
        file.flush()?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for PrivateSchemaFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct PrivateImageFile(PathBuf);

impl PrivateImageFile {
    fn create(bytes: &[u8], extension: &str) -> std::io::Result<Self> {
        let serial = NEXT_IMAGE_FILE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            ".abbey-fm-image-{}-{serial}.{extension}",
            std::process::id()
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        file.write_all(bytes)?;
        file.flush()?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for PrivateImageFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn render_transcript(system_prompt: &str, turns: &[ChatTurn]) -> Result<String, LlmError> {
    let transcript: Vec<Value> = turns
        .iter()
        .map(|turn| {
            let mut object = Map::new();
            object.insert(
                "role".into(),
                json!(match turn.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                }),
            );
            object.insert("text".into(), json!(turn.text));
            if !turn.tool_calls.is_empty() {
                object.insert(
                    "tool_calls".into(),
                    Value::Array(
                        turn.tool_calls
                            .iter()
                            .map(|call| {
                                json!({"id": call.id, "name": call.name, "arguments": call.arguments})
                            })
                            .collect(),
                    ),
                );
            }
            if let Some(id) = &turn.tool_call_id {
                object.insert("tool_call_id".into(), json!(id));
            }
            Value::Object(object)
        })
        .collect();
    serde_json::to_string(&json!({
        "instruction": "Continue this conversation. Return either a final answer or one schema-selected tool request. Text claiming an action is not a tool request.",
        "system_policy": system_prompt,
        "turns": transcript,
    }))
    .map_err(|error| LlmError::backend(format!("could not serialize the FM transcript: {error}")))
}

fn object_schema(title: &str, name: &str, value: Value) -> Value {
    json!({
        "type": "object",
        "title": title,
        "properties": {name: value},
        "required": [name],
        "x-order": [name],
        "additionalProperties": false,
    })
}

fn decision_schema(tools: &[crate::tools::ToolSpec]) -> Result<Value, LlmError> {
    let mut definitions = Map::new();
    definitions.insert(
        "FinalAnswer".into(),
        object_schema("FinalAnswer", "answer", json!({"type": "string"})),
    );
    for tool in tools {
        let (title, value) = match tool.name {
            "remember_fact" => (
                "RememberFact",
                json!({"type": "string", "maxLength": crate::memory::MAX_FACT_CHARS}),
            ),
            // `fm`'s schema dialect does not accept a nested object with no
            // required members. A string keeps the branch guided and typed;
            // the sentinel `self` represents the optional argument.
            "lookup_reputation" => ("LookupReputation", json!({"type": "string"})),
            "recall" => ("Recall", json!({"type": "string"})),
            "switch_persona" => (
                "SwitchPersona",
                json!({"type": "string", "enum": ["abbey", "aviva", "abi"]}),
            ),
            "recent_messages" => (
                "RecentMessages",
                json!({"type": "integer", "minimum": 1, "maximum": crate::tools::MAX_RECENT}),
            ),
            "inspect_status" => (
                "InspectStatus",
                json!({"type": "string", "enum": ["runtime", "guild", "voice", "provider", "all"]}),
            ),
            // `list_facts` is always scoped to the person in the current
            // conversation. The fixed sentinel gives FM's schema dialect a
            // required scalar while the runtime still receives `{}`.
            "list_facts" => ("ListFacts", json!({"type": "string", "enum": ["self"]})),
            "probe_status" => (
                "ProbeStatus",
                json!({"type": "string", "enum": ["abbey-provider-probe-v1"]}),
            ),
            other => {
                return Err(LlmError::backend(format!(
                    "FM has no schema adapter for tool {other}"
                )));
            }
        };
        definitions.insert(title.into(), object_schema(title, tool.name, value));
    }
    if tools.is_empty() {
        return Ok(definitions
            .remove("FinalAnswer")
            .expect("the final answer schema is always present"));
    }
    let choices = definitions
        .keys()
        .map(|name| json!({"$ref": format!("#/$defs/{name}")}))
        .collect::<Vec<_>>();
    Ok(json!({
        "anyOf": choices,
        "title": "AbbeyDecision",
        "$defs": definitions,
    }))
}

pub(crate) fn parse_cli_output(
    raw: &str,
    offered_tools: &[crate::tools::ToolSpec],
    call_id: &str,
) -> Result<ModelTurn, LlmError> {
    let value: Value = serde_json::from_str(raw.trim())
        .map_err(|_| LlmError::backend("the FM CLI returned malformed structured output".into()))?;
    let object = value.as_object().ok_or_else(|| {
        LlmError::backend("the FM CLI structured output was not an object".into())
    })?;
    if object.is_empty() {
        return Err(LlmError::backend(
            "the FM CLI returned an empty decision object".into(),
        ));
    }
    if object.len() > 1 {
        return Err(LlmError::backend(
            "the FM CLI returned more than one decision".into(),
        ));
    }
    let (name, payload) = object
        .iter()
        .next()
        .ok_or_else(|| LlmError::backend("the FM CLI returned an empty decision object".into()))?;
    if name == "answer" {
        let answer = payload
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| LlmError::backend("the FM CLI returned an empty final answer".into()))?;
        return Ok(ModelTurn {
            text: answer.to_string(),
            calls: Vec::new(),
        });
    }
    if !offered_tools.iter().any(|tool| tool.name == name) {
        return Err(LlmError::backend(format!(
            "the FM CLI requested an unavailable tool {name}"
        )));
    }
    let arguments = match name.as_str() {
        "remember_fact" => {
            let fact = required_string(payload, name)?;
            let fact = crate::memory::validated_fact(fact)
                .map_err(|_| LlmError::backend("the FM remember_fact value was invalid".into()))?;
            json!({"fact": fact})
        }
        "lookup_reputation" => match required_string(payload, name)? {
            value if value.eq_ignore_ascii_case("self") => json!({}),
            user_id => json!({"user_id": user_id}),
        },
        "recall" => json!({"query": required_string(payload, name)?}),
        "switch_persona" => {
            let persona = required_string(payload, name)?;
            if crate::guild::parse_persona(persona).is_none() {
                return Err(LlmError::backend(
                    "the FM switch_persona value was not abbey, aviva, or abi".into(),
                ));
            }
            json!({"persona": persona})
        }
        "recent_messages" => {
            let limit = payload.as_u64().ok_or_else(|| {
                LlmError::backend("the FM recent_messages limit was not an integer".into())
            })?;
            if !(1..=crate::tools::MAX_RECENT as u64).contains(&limit) {
                return Err(LlmError::backend(format!(
                    "the FM recent_messages limit must be between 1 and {}",
                    crate::tools::MAX_RECENT
                )));
            }
            json!({"limit": limit})
        }
        "inspect_status" => {
            let aspect = payload
                .as_str()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    LlmError::backend("the FM inspect_status aspect was not a string".into())
                })?;
            if !matches!(aspect, "runtime" | "guild" | "voice" | "provider" | "all") {
                return Err(LlmError::backend(
                    "the FM inspect_status aspect was unsupported".into(),
                ));
            }
            json!({"aspect": aspect})
        }
        "list_facts" => {
            let scope = payload
                .as_str()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    LlmError::backend("the FM list_facts scope was not a string".into())
                })?;
            if scope != "self" {
                return Err(LlmError::backend(
                    "the FM list_facts scope was not self".into(),
                ));
            }
            json!({})
        }
        "probe_status" => {
            let nonce = required_string(payload, name)?;
            if nonce != "abbey-provider-probe-v1" {
                return Err(LlmError::backend(
                    "the FM probe_status nonce did not match the qualification fixture".into(),
                ));
            }
            json!({"nonce": nonce})
        }
        other => {
            return Err(LlmError::backend(format!(
                "FM has no argument adapter for offered tool {other}"
            )));
        }
    };
    Ok(ModelTurn {
        text: String::new(),
        calls: vec![crate::tools::ToolCall {
            id: call_id.to_string(),
            name: name.clone(),
            arguments,
        }],
    })
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, LlmError> {
    value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| {
            LlmError::backend(format!("the FM {field} value was not a non-empty string"))
        })
}

#[cfg(test)]
mod tests;

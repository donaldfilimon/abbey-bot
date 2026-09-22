//! Foundation Models `fm respond` CLI adapter: argv-only invocation with a
//! filtered environment, bounded output, private owner-only schema and image
//! files, the schema-guided decision contract, and strict parsing of the one
//! decision it returns. Moved verbatim from `provider.rs`; the configuration
//! and capability types it consumes stay there.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use super::{
    FmConfig, FmImageTask, FmMode, ProviderCapabilities, ProviderFailureKind,
    VerifiedFmCapabilities, qualification,
};
use crate::llm::{Backend, ChatTurn, LlmError, ModelTurn, Role};

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

pub struct FoundationModels {
    service: std::sync::OnceLock<crate::service::OperationRegistry>,
    pub config: FmConfig,
    pub server_capabilities: Option<ProviderCapabilities>,
    pub cli_capabilities: ProviderCapabilities,
    qualified: bool,
    qualified_cli_sha256: Option<String>,
}

impl FoundationModels {
    #[must_use]
    pub fn new(
        config: FmConfig,
        _primary_backend: Option<&Backend>,
        _primary_tools_enabled: bool,
    ) -> Self {
        let server = config.endpoint.as_ref().map(|_| ProviderCapabilities {
            text: true,
            streaming: true,
            ..ProviderCapabilities::default()
        });
        let cli = ProviderCapabilities::text_with_tools();
        Self {
            service: std::sync::OnceLock::new(),
            config,
            server_capabilities: server,
            cli_capabilities: cli,
            qualified: false,
            qualified_cli_sha256: None,
        }
    }

    #[must_use]
    pub fn new_qualified(
        config: FmConfig,
        _primary_backend: Option<&Backend>,
        _primary_tools_enabled: bool,
        qualified: VerifiedFmCapabilities,
    ) -> Self {
        let qualified_cli_sha256 = qualification::file_sha256(&config.cli).ok();
        Self {
            service: std::sync::OnceLock::new(),
            config,
            server_capabilities: qualified.server.map(|caps| ProviderCapabilities {
                tools: false,
                structured_output: false,
                ..caps
            }),
            cli_capabilities: qualified.cli,
            qualified: true,
            qualified_cli_sha256,
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

    fn verify_cli_identity(&self) -> Result<(), LlmError> {
        if self.qualified
            && (self.qualified_cli_sha256.is_none()
                || qualification::file_sha256(&self.config.cli).ok() != self.qualified_cli_sha256)
        {
            return Err(LlmError::classified(
                "qualified FM executable identity changed",
                ProviderFailureKind::ExecutableIdentity,
            ));
        }
        Ok(())
    }

    pub(crate) fn attach_service(&self, registry: crate::service::OperationRegistry) {
        assert!(
            self.service.set(registry).is_ok(),
            "FM service attached once"
        );
    }

    async fn run_owned<T: Send + 'static>(
        &self,
        invocation: CliInvocation,
        file: T,
    ) -> Result<String, LlmError> {
        let timeout = self.config.timeout_secs;
        if let Some(registry) = self.service.get() {
            let expected = self.qualified_cli_sha256.clone();
            let qualified = self.qualified;
            let cancel = registry.cancellation();
            let result = registry
                .spawn_result(crate::service::OperationKind::ProviderProcess, async move {
                    let _private_file_owner = file;
                    if qualified
                        && (expected.is_none()
                            || qualification::file_sha256(&invocation.program).ok() != expected)
                    {
                        return Err(LlmError::classified(
                            "qualified FM executable identity changed",
                            ProviderFailureKind::ExecutableIdentity,
                        ));
                    }
                    invocation.run_with_cancel(timeout, Some(cancel)).await
                })
                .map_err(|_| {
                    LlmError::classified("service is shutting down", ProviderFailureKind::Cancelled)
                })?;
            result.await.map_err(|_| {
                LlmError::classified(
                    "provider process ownership interrupted",
                    ProviderFailureKind::Cancelled,
                )
            })?
        } else {
            invocation.run(timeout).await
        }
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
        self.verify_cli_identity()?;
        let output = self.run_owned(invocation, file).await?;
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
        self.verify_cli_identity()?;
        let output = self.run_owned(invocation, file).await?;
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
pub(super) struct CliInvocation {
    pub(super) program: PathBuf,
    pub(super) args: Vec<OsString>,
    pub(super) stdin: Vec<u8>,
    pub(super) environment: Vec<(OsString, OsString)>,
}

impl CliInvocation {
    pub(super) fn new(config: &FmConfig, prompt: &str, schema: &Path) -> Self {
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

    pub(super) fn for_image(config: &FmConfig, task: FmImageTask, image: &Path) -> Self {
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

    pub(super) async fn run(self, timeout_secs: u64) -> Result<String, LlmError> {
        self.run_with_cancel(timeout_secs, None).await
    }
    pub(super) async fn run_with_cancel(
        self,
        timeout_secs: u64,
        cancel: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<String, LlmError> {
        let mut command = tokio::process::Command::new(&self.program);
        command
            .args(&self.args)
            .env_clear()
            .envs(self.environment.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if cancel.as_ref().is_some_and(|cancel| cancel.is_cancelled()) {
            return Err(LlmError::classified(
                "the FM CLI cancelled",
                ProviderFailureKind::Cancelled,
            ));
        }
        let mut child = command.spawn().map_err(|error| {
            LlmError::classified(
                format!("could not start the configured FM CLI: {error}"),
                ProviderFailureKind::ExecutableIdentity,
            )
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
        let completed = crate::service::cancellation::complete_or_cancelled(
            cancel,
            tokio::time::timeout(Duration::from_secs(timeout_secs), operation),
        )
        .await;
        match completed {
            Some(Ok(Ok(output))) => Ok(output),
            Some(Ok(Err(error))) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                Err(LlmError::backend(error))
            }
            Some(Err(_)) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                Err(LlmError::classified(
                    "the FM CLI timed out",
                    ProviderFailureKind::Timeout,
                ))
            }
            None => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                Err(LlmError::classified(
                    "the FM CLI cancelled",
                    ProviderFailureKind::Cancelled,
                ))
            }
        }
    }
}

pub(super) fn filtered_environment(
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

pub(super) struct PrivateSchemaFile(PathBuf);

impl PrivateSchemaFile {
    pub(super) fn create(schema: &Value) -> std::io::Result<Self> {
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

    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for PrivateSchemaFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub(super) struct PrivateImageFile(PathBuf);

impl PrivateImageFile {
    pub(super) fn create(bytes: &[u8], extension: &str) -> std::io::Result<Self> {
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

    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for PrivateImageFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub(super) fn render_transcript(
    system_prompt: &str,
    turns: &[ChatTurn],
) -> Result<String, LlmError> {
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

pub(super) fn decision_schema(tools: &[crate::tools::ToolSpec]) -> Result<Value, LlmError> {
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

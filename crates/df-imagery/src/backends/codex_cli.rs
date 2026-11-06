//! Private Codex CLI image backend: owned process, evidenced protocol, verified artifacts.
//!
//! The service drives one image turn through `codex app-server` over stdio
//! JSON-RPC: `initialize`, then the client `initialized` notification, then
//! `thread/start`, then `turn/start` with text plus ordered staged
//! references, then the `turn/completed` notification whose
//! `imageGeneration` item names the artifact. Only a verified `savedPath`
//! inside the owned output directory can become `Ready`: a `fileId`-only or
//! `result`-only receipt is an explicit `Unsupported` failure because the
//! schema export proves neither a retrieval endpoint nor a result encoding.
//!
//! This backend never pins an exact CLI version, binary hash, model name, or
//! local-only rule; it never shells out, never shows a window, never retries
//! after uncertainty, never falls back to another provider, and never prices
//! usage. Missing image evidence is `Unknown`, never zero. Unknown billing
//! stays unknown: outcomes carry observed token counts without any cost claim.
//! Prompts, file bytes and server text never reach logs; errors keep routing
//! fields only.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

use df_assets::MAX_UPLOAD_BYTES;

use crate::config::{
    CODEX_MAX_MESSAGE_BYTES_DEFAULT, CODEX_MAX_STDERR_BYTES_DEFAULT, CODEX_MAX_STDOUT_BYTES_DEFAULT,
};
use crate::output::{Dimensions, OutputFormat};
use crate::request::{DEFAULT_MAX_PROMPT_CHARS, DEFAULT_MAX_REFERENCES, TextPrompt};

pub(crate) mod artifact;
mod drive;
#[cfg(all(test, windows))]
pub(crate) mod fixtures;
#[cfg(windows)]
pub(crate) mod job;
#[cfg(all(test, windows))]
mod live_tests;
pub(crate) mod process;
pub(crate) mod protocol;
mod settle;
#[cfg(test)]
mod tests;

use artifact::{ArtifactError, check_staged_bytes};
use process::{ProcessError, ProcessRunner, ProcessSession, ProcessSpec, StreamLimits};
use protocol::{ProtocolError, image_instruction};

pub(crate) use protocol::ObservedUsage;

/// Adapter revision frozen into cache identity by the service; a
/// compatibility-affecting behavior change here needs a new revision.
/// A2 adds the initialize handshake, correlated turn binding, explicit image
/// instructions, verified dimensions/format/alpha and retained usage.
pub const ADAPTER_REVISION: u32 = 2;

/// Why backend configuration was refused; the prior snapshot stays live.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    InvalidField {
        field: &'static str,
        reason: &'static str,
    },
}

/// Normal selectable backend configuration: an absolute executable, an
/// optional agent-model override, and operator-tunable bounds. No exact
/// version, hash, model allowlist, or local-only restriction lives here.
#[derive(Clone, Debug, PartialEq)]
pub struct CodexCliConfig {
    executable: PathBuf,
    agent_model: Option<String>,
    max_prompt_chars: usize,
    max_references: usize,
    max_message_bytes: u64,
    max_stdout_bytes: u64,
    max_stderr_bytes: u64,
    max_artifact_bytes: u64,
}

impl CodexCliConfig {
    /// Defaults align with the request vocabulary, the image configuration's
    /// Codex stream defaults and the asset admission ceiling; the launcher
    /// replaces them with operator values.
    pub fn new(executable: PathBuf) -> Result<Self, ConfigError> {
        let config = Self {
            executable,
            agent_model: None,
            max_prompt_chars: DEFAULT_MAX_PROMPT_CHARS,
            max_references: DEFAULT_MAX_REFERENCES,
            max_message_bytes: CODEX_MAX_MESSAGE_BYTES_DEFAULT,
            max_stdout_bytes: CODEX_MAX_STDOUT_BYTES_DEFAULT,
            max_stderr_bytes: CODEX_MAX_STDERR_BYTES_DEFAULT,
            max_artifact_bytes: MAX_UPLOAD_BYTES as u64,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn with_agent_model(mut self, model: &str) -> Result<Self, ConfigError> {
        // The agent selector is not evidence of which image model generates
        // the artifact; the receipt records identity only when observed.
        if model.trim().is_empty() {
            return Err(ConfigError::InvalidField {
                field: "agent_model",
                reason: "an explicit model must not be blank",
            });
        }
        self.agent_model = Some(model.trim().to_owned());
        Ok(self)
    }

    pub fn with_request_limits(
        self,
        limits: &crate::request::RequestLimits,
    ) -> Result<Self, ConfigError> {
        limits.validate().map_err(|_| ConfigError::InvalidField {
            field: "request_limits",
            reason: "central request limits must admit at least one item",
        })?;
        self.with_max_prompt_chars(limits.max_prompt_chars)?
            .with_max_references(limits.max_references)
    }

    pub fn with_max_prompt_chars(mut self, max_chars: usize) -> Result<Self, ConfigError> {
        if max_chars == 0 {
            return Err(ConfigError::InvalidField {
                field: "max_prompt_chars",
                reason: "a zero bound refuses all work",
            });
        }
        self.max_prompt_chars = max_chars;
        self.validate()?;
        Ok(self)
    }

    pub fn with_max_references(mut self, max: usize) -> Result<Self, ConfigError> {
        if max == 0 {
            return Err(ConfigError::InvalidField {
                field: "max_references",
                reason: "a zero bound refuses all work",
            });
        }
        self.max_references = max;
        self.validate()?;
        Ok(self)
    }

    pub fn with_max_message_bytes(mut self, max_bytes: u64) -> Result<Self, ConfigError> {
        nonzero("max_message_bytes", max_bytes)?;
        self.max_message_bytes = max_bytes;
        self.validate()?;
        Ok(self)
    }

    pub fn with_max_stdout_bytes(mut self, max_bytes: u64) -> Result<Self, ConfigError> {
        nonzero("max_stdout_bytes", max_bytes)?;
        self.max_stdout_bytes = max_bytes;
        self.validate()?;
        Ok(self)
    }

    pub fn with_max_stderr_bytes(mut self, max_bytes: u64) -> Result<Self, ConfigError> {
        nonzero("max_stderr_bytes", max_bytes)?;
        self.max_stderr_bytes = max_bytes;
        self.validate()?;
        Ok(self)
    }

    pub fn with_max_artifact_bytes(mut self, max_bytes: u64) -> Result<Self, ConfigError> {
        nonzero("max_artifact_bytes", max_bytes)?;
        let max_bytesize = usize::MAX as u64;
        if max_bytes > max_bytesize {
            return Err(ConfigError::InvalidField {
                field: "max_artifact_bytes",
                reason: "the artifact bound must fit addressable memory",
            });
        }
        self.max_artifact_bytes = max_bytes;
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.executable.as_os_str().is_empty() || !self.executable.is_absolute() {
            return Err(ConfigError::InvalidField {
                field: "executable",
                reason: "the CLI executable must be an absolute path",
            });
        }
        nonzero("max_prompt_chars", self.max_prompt_chars as u64)?;
        nonzero("max_references", self.max_references as u64)?;
        nonzero("max_message_bytes", self.max_message_bytes)?;
        nonzero("max_stdout_bytes", self.max_stdout_bytes)?;
        nonzero("max_stderr_bytes", self.max_stderr_bytes)?;
        nonzero("max_artifact_bytes", self.max_artifact_bytes)?;
        Ok(())
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn agent_model(&self) -> Option<&str> {
        self.agent_model.as_deref()
    }
}

fn nonzero(field: &'static str, value: u64) -> Result<(), ConfigError> {
    if value == 0 {
        return Err(ConfigError::InvalidField {
            field,
            reason: "a zero bound refuses all work",
        });
    }
    Ok(())
}

/// One service-staged immutable reference: an absolute path inside the owned
/// input directory plus the digest recorded when the service staged it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedReference {
    path: PathBuf,
    sha256_hex: String,
}

impl StagedReference {
    pub fn new(path: PathBuf, sha256_hex: &str) -> Result<Self, ConfigError> {
        if !path.is_absolute() || path.to_str().is_none() {
            return Err(ConfigError::InvalidField {
                field: "reference.path",
                reason: "staged references must be absolute representable paths",
            });
        }
        if sha256_hex.len() != artifact::SHA256_HEX_CHARS
            || !sha256_hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(ConfigError::InvalidField {
                field: "reference.sha256_hex",
                reason: "staged references carry a hex SHA-256",
            });
        }
        Ok(Self {
            path,
            sha256_hex: sha256_hex.to_ascii_lowercase(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn sha256_hex(&self) -> &str {
        &self.sha256_hex
    }
}

/// Generate needs text alone; Edit needs at least the base image first.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageKind {
    Generate,
    Edit,
}

/// Required alpha handling for the delivered bytes: either any validated
/// raster is acceptable, or transparency must be present in the artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlphaRequirement {
    PreserveAlpha,
    Either,
}

/// Why a backend request was refused before any child work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexRequestError {
    PromptTooLong { len_chars: usize, max_chars: usize },
    TooManyReferences { count: usize },
    MissingBase,
}

/// The one backend request: private prompt text, ordered staged references,
/// and the explicit output contract the service froze.
#[derive(Clone, Debug, PartialEq)]
pub struct CodexImageRequest {
    prompt: TextPrompt,
    kind: ImageKind,
    references: Vec<StagedReference>,
    dimensions: Dimensions,
    alpha: AlphaRequirement,
    format: OutputFormat,
}

impl CodexImageRequest {
    pub fn new(
        prompt: TextPrompt,
        kind: ImageKind,
        references: Vec<StagedReference>,
        dimensions: Dimensions,
        alpha: AlphaRequirement,
        format: OutputFormat,
    ) -> Self {
        Self {
            prompt,
            kind,
            references,
            dimensions,
            alpha,
            format,
        }
    }

    pub fn validate(&self, config: &CodexCliConfig) -> Result<(), CodexRequestError> {
        let len_chars = self.prompt.len_chars();
        if len_chars > config.max_prompt_chars {
            return Err(CodexRequestError::PromptTooLong {
                len_chars,
                max_chars: config.max_prompt_chars,
            });
        }
        if self.references.len() > config.max_references {
            return Err(CodexRequestError::TooManyReferences {
                count: self.references.len(),
            });
        }
        if self.kind == ImageKind::Edit && self.references.is_empty() {
            return Err(CodexRequestError::MissingBase);
        }
        Ok(())
    }
}

/// Service-owned attempt directories: working scratch, staged inputs, and
/// the only directory artifacts may be verified from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptDirs {
    scratch: PathBuf,
    inputs: PathBuf,
    outputs: PathBuf,
}

impl AttemptDirs {
    pub fn new(scratch: PathBuf, inputs: PathBuf, outputs: PathBuf) -> Self {
        Self {
            scratch,
            inputs,
            outputs,
        }
    }

    pub fn scratch(&self) -> &Path {
        &self.scratch
    }

    pub fn inputs(&self) -> &Path {
        &self.inputs
    }

    pub fn outputs(&self) -> &Path {
        &self.outputs
    }
}

impl AttemptDirs {
    pub fn check(&self) -> Result<(), ConfigError> {
        for (field, dir) in [
            ("scratch", &self.scratch),
            ("inputs", &self.inputs),
            ("outputs", &self.outputs),
        ] {
            if !dir.is_absolute() {
                return Err(ConfigError::InvalidField {
                    field,
                    reason: "attempt directories must be absolute",
                });
            }
        }
        if self.inputs == self.outputs {
            return Err(ConfigError::InvalidField {
                field: "outputs",
                reason: "verified outputs must not share the staged input directory",
            });
        }
        Ok(())
    }
}

/// Verified artifact receipt: the owned import path, recorded digest, decoded
/// facts. The path always names the service-owned import, never the original
/// mutable helper claim.
#[derive(Clone, Debug, PartialEq)]
pub struct ArtifactReceipt {
    thread_id: String,
    turn_id: String,
    item_id: String,
    file_id: Option<String>,
    path: PathBuf,
    digest_sha256_hex: String,
    mime: String,
    width_px: u32,
    height_px: u32,
    byte_len: usize,
    transparent_background: Option<bool>,
    revised_prompt_present: bool,
    candidate_count: usize,
}

impl ArtifactReceipt {
    #[cfg(test)]
    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(test)]
    pub fn digest_sha256_hex(&self) -> &str {
        &self.digest_sha256_hex
    }

    #[cfg(test)]
    pub fn mime(&self) -> &str {
        &self.mime
    }

    #[cfg(test)]
    pub fn width_px(&self) -> u32 {
        self.width_px
    }

    #[cfg(test)]
    pub fn height_px(&self) -> u32 {
        self.height_px
    }

    #[cfg(test)]
    pub fn byte_len(&self) -> usize {
        self.byte_len
    }

    #[cfg(test)]
    pub fn candidate_count(&self) -> usize {
        self.candidate_count
    }
}

/// How generation was refused or failed with evidence.
#[derive(Clone, Debug, PartialEq)]
pub enum CodexError {
    Configuration(ConfigError),
    Request(CodexRequestError),
    Transport { kind: std::io::ErrorKind },
    Refused { code: String },
    TurnFailed { status: String },
    GenerationFailed { kind: GenerationFailureKind },
    NoArtifact,
    Unsupported { kind: UnsupportedKind },
    Artifact(ArtifactError),
    Protocol(ProtocolError),
    LostOutput,
    Internal,
}

/// Evidenced failure detail: usage limits are proven, anything else is not.
#[derive(Clone, Debug, PartialEq)]
pub enum GenerationFailureKind {
    UsageLimited { limit_id: Option<String> },
    Reported { kind: String },
}

/// Receipt shapes the schema cannot turn into bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsupportedKind {
    FileIdOnly,
    ResultEncoding,
}

/// Why an outcome after possible dispatch stays unknown instead of retried.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnknownReason {
    Deadline,
    Cancelled,
    Interrupted,
    LostOutput,
}

/// The backend outcome: verified bytes, a typed failure, or uncertainty that
/// the caller must not automatically retry or reroute.
#[derive(Clone, Debug, PartialEq)]
pub enum CodexOutcome {
    Ready {
        receipt: ArtifactReceipt,
        usage: ObservedUsage,
    },
    Failed {
        error: CodexError,
        usage: ObservedUsage,
    },
    Unknown {
        reason: UnknownReason,
        usage: ObservedUsage,
    },
}

impl CodexOutcome {
    fn failed(error: CodexError) -> Self {
        Self::Failed {
            error,
            usage: ObservedUsage::unknown(),
        }
    }

    fn unknown(reason: UnknownReason) -> Self {
        Self::Unknown {
            reason,
            usage: ObservedUsage::unknown(),
        }
    }
}

/// Runs one image turn to a verified receipt, a typed failure, or honest
/// uncertainty. Cancellation withdraws interest; timeout, interruption and
/// lost output never trigger a retry or a fallback here. The deadline stops
/// new work; owned cleanup drains started work and may outlast that deadline.
/// The caller retains and awaits execution through cooperative cancellation.
/// A partial turn write is `Unknown`, never safe to retry.
pub async fn execute<R>(
    config: &CodexCliConfig,
    request: &CodexImageRequest,
    attempt: &AttemptDirs,
    runner: &R,
    deadline: Duration,
    cancel: Option<oneshot::Receiver<()>>,
) -> CodexOutcome
where
    R: ProcessRunner,
{
    let Some(deadline_at) = Instant::now().checked_add(deadline) else {
        return CodexOutcome::failed(CodexError::Configuration(ConfigError::InvalidField {
            field: "deadline",
            reason: "the deadline must be representable",
        }));
    };
    if let Err(error) = config.validate() {
        return CodexOutcome::failed(CodexError::Configuration(error));
    }
    if let Err(error) = request.validate(config) {
        return CodexOutcome::failed(CodexError::Request(error));
    }
    if let Err(error) = attempt.check() {
        return CodexOutcome::failed(CodexError::Configuration(error));
    }
    if deadline.is_zero() {
        return CodexOutcome::failed(CodexError::Configuration(ConfigError::InvalidField {
            field: "deadline",
            reason: "provider work needs a nonzero absolute deadline",
        }));
    }
    let image_paths: Vec<String> = request
        .references
        .iter()
        .map(|reference| reference.path().to_string_lossy().into_owned())
        .collect();
    let staged_refs: Vec<(PathBuf, String)> = request
        .references
        .iter()
        .map(|reference| {
            (
                reference.path().to_owned(),
                reference.sha256_hex().to_owned(),
            )
        })
        .collect();
    let inputs_root = attempt.inputs().to_owned();
    let outputs_dir = attempt.outputs().to_owned();
    let scratch_dir = attempt.scratch().to_owned();
    let max_bytes = config.max_artifact_bytes;
    // Blocking preflight stays owned: the handle is borrowed while the
    // deadline runs, and on expiry it is aborted instead of detached. A late
    // result is never treated as a completed preflight.
    let mut preflight = tokio::task::JoinSet::new();
    preflight.spawn_blocking(move || {
        canonicalize_attempt(
            &scratch_dir,
            &inputs_root,
            &outputs_dir,
            &staged_refs,
            max_bytes,
        )
    });
    let roots = match tokio::time::timeout_at(deadline_at.into(), preflight.join_next()).await {
        Ok(Some(Ok(roots))) => roots,
        Ok(Some(Err(_))) => {
            preflight.abort_all();
            while preflight.join_next().await.is_some() {}
            return CodexOutcome::failed(CodexError::Internal);
        }
        _ => {
            preflight.abort_all();
            while preflight.join_next().await.is_some() {}
            return CodexOutcome::unknown(UnknownReason::Deadline);
        }
    };
    let (scratch_text, outputs_root) = match roots {
        Ok(roots) => roots,
        Err(error) => return CodexOutcome::failed(error),
    };
    let instruction = image_instruction(
        operation_label(request),
        request.dimensions.width_px(),
        request.dimensions.height_px(),
        request.format.label(),
        alpha_label(request),
        &outputs_root.to_string_lossy(),
        request.prompt.text(),
    );
    let spec = ProcessSpec {
        executable: config.executable().to_owned(),
        args: vec![std::ffi::OsString::from("app-server")],
        cwd: attempt.scratch().to_owned(),
    };
    let limits = StreamLimits {
        max_stdout_bytes: config.max_stdout_bytes,
        max_stderr_bytes: config.max_stderr_bytes,
        max_line_bytes: message_bound(config),
    };
    let mut session =
        match tokio::time::timeout_at(deadline_at.into(), runner.spawn(&spec, &limits)).await {
            Ok(Ok(session)) => session,
            Ok(Err(error)) => return CodexOutcome::failed(map_spawn_error(error)),
            Err(_) => return CodexOutcome::unknown(UnknownReason::Deadline),
        };
    let outcome = drive::drive(
        &mut session,
        drive::TurnContext {
            config,
            request,
            scratch_text: &scratch_text,
            outputs_root: &outputs_root,
            image_paths: &image_paths,
            instruction: &instruction,
            limits: &limits,
            deadline_at,
        },
        cancel,
    )
    .await;
    // Neither the exit code nor a failed finish can change the decided outcome:
    // exit after a verified terminal receipt cannot invalidate its retained
    // evidence. Only a finish that outlives the deadline needs the tree kill.
    if tokio::time::timeout_at(deadline_at.into(), session.finish())
        .await
        .is_err()
    {
        drive::kill_session(&mut session).await;
    }
    outcome
}

fn operation_label(request: &CodexImageRequest) -> &'static str {
    match request.kind {
        ImageKind::Generate => "generate",
        ImageKind::Edit => "edit with the first localImage as the base image",
    }
}

fn alpha_label(request: &CodexImageRequest) -> &'static str {
    match request.alpha {
        AlphaRequirement::PreserveAlpha => "preserve-alpha transparency required",
        AlphaRequirement::Either => "either opaque or transparent",
    }
}

fn canonicalize_attempt(
    scratch: &Path,
    inputs: &Path,
    outputs: &Path,
    staged: &[(PathBuf, String)],
    max_bytes: u64,
) -> Result<(String, PathBuf), CodexError> {
    let scratch_text =
        scratch
            .to_str()
            .ok_or(CodexError::Configuration(ConfigError::InvalidField {
                field: "scratch",
                reason: "scratch must be representable before launch",
            }))?;
    let scratch_canonical = std::fs::canonicalize(scratch).map_err(|_| {
        CodexError::Configuration(ConfigError::InvalidField {
            field: "attempt",
            reason: "scratch and outputs must exist before launch",
        })
    })?;
    let inputs_canonical = std::fs::canonicalize(inputs).map_err(|_| {
        CodexError::Configuration(ConfigError::InvalidField {
            field: "attempt",
            reason: "staged inputs must exist before launch",
        })
    })?;
    let outputs_canonical = std::fs::canonicalize(outputs).map_err(|_| {
        CodexError::Configuration(ConfigError::InvalidField {
            field: "attempt",
            reason: "scratch and outputs must exist before launch",
        })
    })?;
    if inputs_canonical == outputs_canonical
        || outputs_canonical.starts_with(&inputs_canonical)
        || inputs_canonical.starts_with(&outputs_canonical)
        || scratch_canonical == inputs_canonical
        || scratch_canonical == outputs_canonical
    {
        return Err(CodexError::Configuration(ConfigError::InvalidField {
            field: "outputs",
            reason: "verified outputs must not overlap the staged input directory",
        }));
    }
    for (path, digest) in staged {
        let canonical = std::fs::canonicalize(path)
            .map_err(|_| CodexError::Artifact(ArtifactError::ForeignPath))?;
        if canonical != inputs_canonical && !canonical.starts_with(&inputs_canonical) {
            return Err(CodexError::Artifact(ArtifactError::ForeignPath));
        }
        check_staged_bytes(path, digest, max_bytes).map_err(CodexError::Artifact)?;
    }
    Ok((scratch_text.to_owned(), outputs_canonical))
}

/// Converts the configured per-message byte bound without a truncating cast.
pub(crate) fn message_bound(config: &CodexCliConfig) -> usize {
    config.max_message_bytes.min(usize::MAX as u64) as usize
}

fn map_spawn_error(error: ProcessError) -> CodexError {
    match error {
        ProcessError::Configuration => CodexError::Configuration(ConfigError::InvalidField {
            field: "process",
            reason: "the child spec was not launchable",
        }),
        ProcessError::SpawnFailed { kind } => CodexError::Transport { kind },
        ProcessError::StreamFailed { kind } => CodexError::Transport { kind },
        ProcessError::OutputTooLarge => CodexError::LostOutput,
    }
}

//! One picture from one prompt through the Codex CLI image backend, with no journal, no database
//! and no durable job.
//!
//! The durable service is the way the game asks for pictures it will keep; this is the direct
//! door for a developer tool that wants a single image now. It builds the same backend request
//! the service builds (`codex app-server` over stdio, a verified artifact inside an owned output
//! directory), runs it to a verified file, reads the bytes and removes the attempt directories.
//! It never prices anything, never retries and never falls back to another provider: a timeout,
//! an interruption or a refusal comes back as a typed `OneShotError` and the caller decides.
//!
//! The Codex CLI picks its own picture size and cannot write transparency. The size asked for is
//! only a request; the real size is read from the delivered bytes.
//!
//! The child process inherits the ambient environment, so the signed-in Codex home is chosen by
//! `CODEX_HOME` in the environment of the process that calls this. The caller checks that before
//! calling; nothing here reads or copies a credential file.
//!
//! `generate_blocking` is the entry point for a host: the backend future is not `Send`, so it is
//! driven to completion on the calling (blocking) thread by a runtime of its own.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::backends::codex_cli::process::TokioProcessRunner;
use crate::backends::codex_cli::{
    AlphaRequirement, AttemptDirs, CodexCliConfig, CodexError, CodexImageRequest, CodexOutcome,
    GenerationFailureKind, ImageKind, UnknownReason, execute,
};
use crate::output::{Dimensions, OutputFormat};
use crate::request::TextPrompt;

/// The backend's name in provenance.
pub const BACKEND_NAME: &str = "codex-cli";

/// Longest request text this entry point accepts, in characters.
pub const MAXIMUM_PROMPT_CHARS: usize = 16_000;

/// The largest single protocol line, in bytes. A finished picture arrives as one base64 line, so
/// the backend's own small default (1 MiB) loses every real picture as lost output; the durable
/// service allows 24 MiB, and the owner wants large limits.
pub const MAXIMUM_MESSAGE_BYTES: u64 = 64 * 1024 * 1024;

/// The most the child may write to standard output over one attempt, in bytes.
pub const MAXIMUM_STDOUT_BYTES: u64 = 256 * 1024 * 1024;

static NEXT_ATTEMPT: AtomicU64 = AtomicU64::new(0);

/// What to run and where.
#[derive(Debug, Clone)]
pub struct OneShotConfig {
    /// Absolute path of the Codex executable.
    pub executable: PathBuf,
    /// An absolute folder that exists, outside any git repository and any folder holding an
    /// `AGENTS.md`; each attempt gets its own subfolder here and removes it afterwards.
    pub work_root: PathBuf,
    /// An explicit agent model, when the owner wants one.
    pub agent_model: Option<String>,
    /// The absolute limit for the whole attempt.
    pub deadline: Duration,
}

/// One picture to make.
#[derive(Debug, Clone)]
pub struct OneShotRequest {
    pub prompt: String,
    /// The size asked for. The Codex image tool decides the real size.
    pub width_px: u32,
    pub height_px: u32,
}

/// A verified picture and what is known about how it was made.
#[derive(Debug, Clone, PartialEq)]
pub struct OneShotImage {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub width_px: u32,
    pub height_px: u32,
    pub sha256_hex: String,
    pub elapsed: Duration,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// Why no picture came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OneShotError {
    /// The configuration cannot run an attempt at all.
    InvalidConfig(&'static str),
    /// The request was refused before any work.
    InvalidRequest(&'static str),
    /// The deadline ended the attempt. The provider may still have done the work.
    Timeout,
    /// The attempt was interrupted or its output was lost after work may have started.
    Interrupted,
    /// The account's usage limit stopped the attempt.
    UsageLimited,
    /// The CLI could not be started: `kind` is the operating system's error kind.
    Transport {
        kind: String,
    },
    /// The service refused or failed the turn, or its output could not be verified.
    Refused {
        detail: String,
    },
    /// The picture was made but its file could not be read back.
    Unreadable {
        kind: String,
    },
    Internal,
}

impl OneShotError {
    /// A closed lowercase key for logs and typed answers.
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidConfig(_) => "invalid_config",
            Self::InvalidRequest(_) => "invalid_request",
            Self::Timeout => "timeout",
            Self::Interrupted => "interrupted",
            Self::UsageLimited => "usage_limited",
            Self::Transport { .. } => "transport",
            Self::Refused { .. } => "refused",
            Self::Unreadable { .. } => "unreadable",
            Self::Internal => "internal",
        }
    }
}

fn map_failure(error: CodexError) -> OneShotError {
    match error {
        CodexError::Configuration(_) => {
            OneShotError::InvalidConfig("the backend refused its setup")
        }
        CodexError::Request(_) => OneShotError::InvalidRequest("the backend refused the request"),
        CodexError::Transport { kind } => OneShotError::Transport {
            kind: format!("{kind:?}"),
        },
        CodexError::GenerationFailed {
            kind: GenerationFailureKind::UsageLimited { .. },
        } => OneShotError::UsageLimited,
        CodexError::Internal => OneShotError::Internal,
        other => OneShotError::Refused {
            detail: format!("{other:?}"),
        },
    }
}

fn dimensions(request: &OneShotRequest) -> Result<Dimensions, OneShotError> {
    Dimensions::new(request.width_px, request.height_px)
        .map_err(|_| OneShotError::InvalidRequest("the size asked for is not a valid size"))
}

/// A folder of its own for one attempt, removed when dropped.
struct AttemptFolder {
    root: PathBuf,
}

impl AttemptFolder {
    fn create(work_root: &Path) -> std::io::Result<(Self, AttemptDirs)> {
        let number = NEXT_ATTEMPT.fetch_add(1, Ordering::Relaxed);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let root = work_root.join(format!("oneshot-{}-{stamp:x}-{number}", std::process::id()));
        let scratch = root.join("scratch");
        let inputs = root.join("inputs");
        let outputs = root.join("outputs");
        for directory in [&scratch, &inputs, &outputs] {
            std::fs::create_dir_all(directory)?;
        }
        Ok((Self { root }, AttemptDirs::new(scratch, inputs, outputs)))
    }
}

impl Drop for AttemptFolder {
    fn drop(&mut self) {
        // The folder is ours and holds only this attempt; leftovers are only disk space.
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The real size and type of verified bytes: the PNG header directly, otherwise the container
/// header.
fn describe(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() >= 24 && bytes.starts_with(PNG) && &bytes[12..16] == b"IHDR" {
        let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
        let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
        return Some(("image/png", width, height));
    }
    let header = crate::source_header(bytes).ok()?;
    let mime = match image::guess_format(bytes).ok()? {
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::WebP => "image/webp",
        image::ImageFormat::Png => "image/png",
        _ => return None,
    };
    Some((
        mime,
        header.dimensions.width_px(),
        header.dimensions.height_px(),
    ))
}

/// Makes one picture. The deadline is absolute for the whole attempt.
pub async fn generate(
    config: &OneShotConfig,
    request: &OneShotRequest,
) -> Result<OneShotImage, OneShotError> {
    if config.deadline.is_zero() {
        return Err(OneShotError::InvalidConfig("the deadline must not be zero"));
    }
    let mut backend = CodexCliConfig::new(config.executable.clone())
        .map_err(|_| OneShotError::InvalidConfig("the Codex executable path is not usable"))?;
    if let Some(model) = &config.agent_model {
        backend = backend
            .with_agent_model(model)
            .map_err(|_| OneShotError::InvalidConfig("the agent model is blank"))?;
    }
    if request.prompt.chars().count() > MAXIMUM_PROMPT_CHARS {
        return Err(OneShotError::InvalidRequest("the prompt is too long"));
    }
    let backend = backend
        .with_max_prompt_chars(MAXIMUM_PROMPT_CHARS)
        .and_then(|backend| backend.with_max_message_bytes(MAXIMUM_MESSAGE_BYTES))
        .and_then(|backend| backend.with_max_stdout_bytes(MAXIMUM_STDOUT_BYTES))
        .map_err(|_| OneShotError::InvalidConfig("the backend limits are not usable"))?;
    let prompt = TextPrompt::new(&request.prompt, 1)
        .map_err(|_| OneShotError::InvalidRequest("the prompt is empty"))?;
    let backend_request = CodexImageRequest::new(
        prompt,
        ImageKind::Generate,
        Vec::new(),
        dimensions(request)?,
        AlphaRequirement::Either,
        OutputFormat::Png,
    );
    let (folder, attempt) =
        AttemptFolder::create(&config.work_root).map_err(|error| OneShotError::Unreadable {
            kind: format!("{:?}", error.kind()),
        })?;
    let started = Instant::now();
    let outcome = execute(
        &backend,
        &backend_request,
        &attempt,
        &TokioProcessRunner,
        config.deadline,
        None,
    )
    .await;
    let elapsed = started.elapsed();
    match outcome {
        CodexOutcome::Ready { receipt, usage } => {
            let path = receipt.path().to_owned();
            let bytes = tokio::task::spawn_blocking(move || std::fs::read(path))
                .await
                .map_err(|_| OneShotError::Internal)?
                .map_err(|error| OneShotError::Unreadable {
                    kind: format!("{:?}", error.kind()),
                })?;
            drop(folder);
            let (mime, width_px, height_px) = describe(&bytes).ok_or(OneShotError::Refused {
                detail: "the delivered file is not a readable picture".to_owned(),
            })?;
            Ok(OneShotImage {
                sha256_hex: hex(&Sha256::digest(&bytes)),
                bytes,
                mime,
                width_px,
                height_px,
                elapsed,
                input_tokens: usage.input_tokens(),
                output_tokens: usage.output_tokens(),
            })
        }
        CodexOutcome::Failed { error, .. } => Err(map_failure(error)),
        CodexOutcome::Unknown { reason, .. } => Err(match reason {
            UnknownReason::Deadline => OneShotError::Timeout,
            UnknownReason::Cancelled | UnknownReason::Interrupted | UnknownReason::LostOutput => {
                OneShotError::Interrupted
            }
        }),
    }
}

/// Makes one picture on the calling thread, which may block for minutes: call it from a blocking
/// task, never from an async worker.
pub fn generate_blocking(
    config: &OneShotConfig,
    request: &OneShotRequest,
) -> Result<OneShotImage, OneShotError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| OneShotError::Internal)?;
    runtime.block_on(generate(config, request))
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_png_header_gives_its_real_size() {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&13_u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&1672_u32.to_be_bytes());
        bytes.extend_from_slice(&941_u32.to_be_bytes());
        assert_eq!(describe(&bytes), Some(("image/png", 1672, 941)));
    }

    #[test]
    fn bytes_that_are_not_a_picture_describe_as_nothing() {
        assert_eq!(describe(b"not a picture at all, just text"), None);
    }

    #[test]
    fn an_unusable_request_is_refused_before_any_work() {
        let config = OneShotConfig {
            executable: PathBuf::from("codex"),
            work_root: std::env::temp_dir(),
            agent_model: None,
            deadline: Duration::from_secs(1),
        };
        let request = OneShotRequest {
            prompt: "a lake".to_owned(),
            width_px: 16,
            height_px: 9,
        };
        let error = generate_blocking(&config, &request).expect_err("a relative path is refused");
        assert_eq!(error.key(), "invalid_config");
    }

    #[test]
    fn a_missing_executable_is_a_transport_failure() {
        let work = std::env::temp_dir();
        let config = OneShotConfig {
            executable: work.join("no-such-codex-executable.exe"),
            work_root: work,
            agent_model: None,
            deadline: Duration::from_secs(5),
        };
        let request = OneShotRequest {
            prompt: "a lake".to_owned(),
            width_px: 16,
            height_px: 9,
        };
        let error = generate_blocking(&config, &request).expect_err("nothing to run");
        assert_eq!(error.key(), "transport", "{error:?}");
    }
}

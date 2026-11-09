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
//! calling and names the same home in `OneShotConfig::codex_home`, where the image tool saves its
//! pictures; nothing here reads the environment or a credential file.
//!
//! `generate` is `Send`, so a host awaits it on its own task; only the attempt folder and the
//! delivered file are touched on blocking threads.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::backends::codex_cli::process::TokioProcessRunner;
use crate::backends::codex_cli::{
    AlphaRequirement, AttemptDirs, CodexCliConfig, CodexError, CodexImageRequest, CodexOutcome,
    GenerationFailureKind, ImageKind, StagedReference, UnknownReason, execute,
};
use crate::listed::listed_enum;
use crate::output::{Dimensions, OutputFormat};
use crate::request::TextPrompt;

/// The backend's name in provenance.
pub const BACKEND_NAME: &str = "codex-cli";

/// Longest request text this entry point accepts, in characters.
pub const MAXIMUM_PROMPT_CHARS: usize = 16_000;

/// The most pictures one request attaches: the base, when there is one, and the references.
pub const MAXIMUM_PICTURES: usize = crate::request::DEFAULT_MAX_REFERENCES;

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
    /// The signed-in Codex home the child runs with (its `CODEX_HOME`), whose
    /// `generated_images` folder may hold the picture. `None` looks only in the attempt folder.
    pub codex_home: Option<PathBuf>,
    /// The absolute limit for the whole attempt.
    pub deadline: Duration,
}

/// One picture to make.
#[derive(Debug, Clone)]
pub struct OneShotRequest {
    /// The request text, at most `MAXIMUM_PROMPT_CHARS` characters.
    pub prompt: String,
    /// The size asked for. The Codex image tool decides the real size.
    pub width_px: u32,
    pub height_px: u32,
    /// A picture to edit instead of making a new one: the result keeps it and changes what the
    /// prompt says. PNG, JPEG or WebP bytes.
    pub base: Option<Vec<u8>>,
    /// Pictures the result follows (a person's look, say), attached in order after the base.
    /// PNG, JPEG or WebP bytes; with the base, at most `MAXIMUM_PICTURES`.
    pub references: Vec<Vec<u8>>,
}

/// A verified picture and what is known about how it was made.
#[derive(Debug, Clone, PartialEq)]
pub struct OneShotImage {
    pub bytes: Vec<u8>,
    /// The type read from the delivered bytes: `image/png`, `image/jpeg` or `image/webp`.
    pub mime: &'static str,
    /// The real size read from the delivered bytes, not the size asked for.
    pub width_px: u32,
    pub height_px: u32,
    pub sha256_hex: String,
    pub elapsed: Duration,
    /// Token counts when the CLI reported them.
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Whether the attempt's folder was removed. A leftover costs only disk space, never the
    /// picture, so the caller records it.
    pub cleanup: Result<(), CleanupError>,
}

listed_enum! {
    keyed
    /// Why an attempt's folder was not removed: the operating system's error kind, or the
    /// blocking task that removes it stopped.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum CleanupCause {
        NotFound,
        PermissionDenied,
        ResourceBusy,
        DirectoryNotEmpty,
        ReadOnlyFilesystem,
        Other,
        /// The blocking task that removes the folder stopped before it reported.
        Stopped,
    }
}

impl CleanupCause {
    /// A closed lowercase key for logs.
    pub fn key(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::PermissionDenied => "permission_denied",
            Self::ResourceBusy => "resource_busy",
            Self::DirectoryNotEmpty => "directory_not_empty",
            Self::ReadOnlyFilesystem => "read_only_filesystem",
            Self::Other => "other",
            Self::Stopped => "stopped",
        }
    }

    fn from_io(kind: std::io::ErrorKind) -> Self {
        use std::io::ErrorKind;
        match kind {
            ErrorKind::NotFound => Self::NotFound,
            ErrorKind::PermissionDenied => Self::PermissionDenied,
            ErrorKind::ResourceBusy => Self::ResourceBusy,
            ErrorKind::DirectoryNotEmpty => Self::DirectoryNotEmpty,
            ErrorKind::ReadOnlyFilesystem => Self::ReadOnlyFilesystem,
            _ => Self::Other,
        }
    }
}

/// An attempt's folder that was not removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupError {
    pub cause: CleanupCause,
}

impl std::fmt::Display for CleanupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "the attempt folder was not removed: {}",
            self.cause.key()
        )
    }
}

impl std::error::Error for CleanupError {}

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
    Transport { kind: String },
    /// The service refused or failed the turn, or its output could not be verified.
    Refused { detail: String },
    /// The picture was made but its file could not be read back.
    Unreadable { kind: String },
    /// A blocking task, the private runtime or local disk work failed inside this process.
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

impl std::fmt::Display for OneShotError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig(reason) => write!(formatter, "invalid config: {reason}"),
            Self::InvalidRequest(reason) => write!(formatter, "invalid request: {reason}"),
            Self::Timeout => formatter.write_str("the attempt timed out"),
            Self::Interrupted => formatter.write_str("the attempt was interrupted"),
            Self::UsageLimited => formatter.write_str("the account usage limit was reached"),
            Self::Transport { kind } => write!(formatter, "the cli could not start: {kind}"),
            Self::Refused { detail } => write!(formatter, "the turn was refused: {detail}"),
            Self::Unreadable { kind } => write!(formatter, "the picture was unreadable: {kind}"),
            Self::Internal => formatter.write_str("the attempt failed inside this process"),
        }
    }
}

impl std::error::Error for OneShotError {}

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

/// A folder of its own for one attempt. `remove` deletes it off the async thread; dropping it
/// unremoved (a cancelled attempt) deletes it on the blocking pool, or on the spot outside a
/// runtime.
struct AttemptFolder {
    root: Option<PathBuf>,
}

impl AttemptFolder {
    /// Creates the attempt's folders on a blocking thread.
    async fn create(work_root: &Path) -> Result<(Self, AttemptDirs), OneShotError> {
        let work_root = work_root.to_owned();
        tokio::task::spawn_blocking(move || Self::create_blocking(&work_root))
            .await
            .map_err(|_| OneShotError::Internal)?
            .map_err(|error| map_work_folder_failure(error.kind()))
    }

    fn create_blocking(work_root: &Path) -> std::io::Result<(Self, AttemptDirs)> {
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
        let folder = Self { root: Some(root) };
        Ok((folder, AttemptDirs::new(scratch, inputs, outputs)))
    }

    /// Removes the folder on a blocking thread. A leftover costs only disk space, never the
    /// picture, so the caller decides how to record a failure.
    async fn remove(mut self) -> Result<(), CleanupError> {
        let Some(root) = self.root.take() else {
            return Ok(());
        };
        tokio::task::spawn_blocking(move || std::fs::remove_dir_all(root))
            .await
            .map_err(|_| CleanupError {
                cause: CleanupCause::Stopped,
            })?
            .map_err(|error| CleanupError {
                cause: CleanupCause::from_io(error.kind()),
            })
    }
}

/// Names a work root that cannot hold an attempt as a configuration failure, with its cause in the
/// reason. Any other local failure (a full disk, a quota, a locked file) happens on a well
/// configured host, so it is a failure inside this process, never "not configured".
fn map_work_folder_failure(kind: std::io::ErrorKind) -> OneShotError {
    use std::io::ErrorKind;
    match kind {
        ErrorKind::NotADirectory | ErrorKind::AlreadyExists => {
            OneShotError::InvalidConfig("the work folder path runs through a file, not a folder")
        }
        ErrorKind::NotFound | ErrorKind::InvalidInput | ErrorKind::InvalidFilename => {
            OneShotError::InvalidConfig("the work folder path is not a usable folder path")
        }
        ErrorKind::PermissionDenied => {
            OneShotError::InvalidConfig("the work folder does not let this process create folders")
        }
        ErrorKind::ReadOnlyFilesystem => {
            OneShotError::InvalidConfig("the work folder is on a read-only file system")
        }
        _ => OneShotError::Internal,
    }
}

impl Drop for AttemptFolder {
    fn drop(&mut self) {
        let Some(root) = self.root.take() else {
            return;
        };
        // The folder is ours and holds only this attempt; leftovers are only disk space. A
        // cancelled attempt is dropped on an async worker, so inside a runtime the delete goes to
        // the blocking pool; outside one there is no worker to keep free.
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn_blocking(move || std::fs::remove_dir_all(root));
            }
            Err(_) => {
                let _ = std::fs::remove_dir_all(root);
            }
        }
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
    if let Some(home) = &config.codex_home {
        backend = backend.with_codex_home(home.clone());
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
    let dimensions = dimensions(request)?;
    if usize::from(request.base.is_some()) + request.references.len() > MAXIMUM_PICTURES {
        return Err(OneShotError::InvalidRequest(
            "the request attaches more pictures than the backend takes",
        ));
    }
    let (folder, attempt) = AttemptFolder::create(&config.work_root).await?;
    let (kind, references) = match stage_pictures(attempt.inputs(), request).await {
        Ok(staged) => staged,
        Err(error) => {
            record_unreturned_cleanup(folder.remove().await, &error);
            return Err(error);
        }
    };
    let backend_request = CodexImageRequest::new(
        prompt,
        kind,
        references,
        dimensions,
        AlphaRequirement::Either,
        OutputFormat::Png,
    );
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
    let delivered = match outcome {
        CodexOutcome::Ready { receipt, usage } => read_delivered(receipt.path().to_owned())
            .await
            .map(|bytes| (bytes, usage.input_tokens(), usage.output_tokens())),
        CodexOutcome::Failed { error, .. } => Err(map_failure(error)),
        CodexOutcome::Unknown { reason, .. } => Err(map_unknown(reason)),
    };
    let cleanup = folder.remove().await;
    let described = delivered.and_then(|delivered| {
        let described = describe(&delivered.0).ok_or(OneShotError::Refused {
            detail: "the delivered file is not a readable picture".to_owned(),
        })?;
        Ok((delivered, described))
    });
    let ((bytes, input_tokens, output_tokens), (mime, width_px, height_px)) = match described {
        Ok(described) => described,
        Err(error) => {
            record_unreturned_cleanup(cleanup, &error);
            return Err(error);
        }
    };
    Ok(OneShotImage {
        sha256_hex: crate::lower_hex(&Sha256::digest(&bytes)),
        bytes,
        mime,
        width_px,
        height_px,
        elapsed,
        input_tokens,
        output_tokens,
        cleanup,
    })
}

/// Stages the request's pictures in the attempt's inputs, the base first: an edit when there is a
/// base, otherwise a generation that follows the references.
async fn stage_pictures(
    inputs: &Path,
    request: &OneShotRequest,
) -> Result<(ImageKind, Vec<StagedReference>), OneShotError> {
    let mut staged = Vec::with_capacity(request.references.len() + 1);
    if let Some(base) = &request.base {
        staged
            .push(stage_picture(inputs, "base", base, "the base is not a readable picture").await?);
    }
    for (index, reference) in request.references.iter().enumerate() {
        let stem = format!("reference-{}", index + 1);
        staged.push(
            stage_picture(
                inputs,
                &stem,
                reference,
                "a reference is not a readable picture",
            )
            .await?,
        );
    }
    let kind = if request.base.is_some() {
        ImageKind::Edit
    } else {
        ImageKind::Generate
    };
    Ok((kind, staged))
}

/// Stages `picture` in the attempt's inputs as `<stem>.<its own type's extension>` with its digest,
/// so the backend's preflight checks it like any staged reference. A picture that does not read is
/// refused with `unreadable`.
async fn stage_picture(
    inputs: &Path,
    stem: &str,
    picture: &[u8],
    unreadable: &'static str,
) -> Result<StagedReference, OneShotError> {
    let (mime, _, _) = describe(picture).ok_or(OneShotError::InvalidRequest(unreadable))?;
    let extension = match mime {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        _ => "png",
    };
    let path = inputs.join(format!("{stem}.{extension}"));
    let digest = crate::lower_hex(&Sha256::digest(picture));
    let (target, bytes) = (path.clone(), picture.to_vec());
    tokio::task::spawn_blocking(move || std::fs::write(target, bytes))
        .await
        .map_err(|_| OneShotError::Internal)?
        .map_err(|error| map_work_folder_failure(error.kind()))?;
    StagedReference::new(path, &digest).map_err(|_| OneShotError::Internal)
}

/// A failed attempt returns only its error, so a folder it also left behind is recorded here:
/// `IMAGERY cleanup_failed kind=<cleanup key> outcome=<error key>`.
fn record_unreturned_cleanup(cleanup: Result<(), CleanupError>, error: &OneShotError) {
    if let Err(failure) = cleanup {
        df_observe::record_audit_line("IMAGERY", cleanup_line(failure, error));
    }
}

fn cleanup_line(failure: CleanupError, error: &OneShotError) -> String {
    format!(
        "IMAGERY cleanup_failed kind={} outcome={}",
        failure.cause.key(),
        error.key()
    )
}

/// Reads the verified artifact's bytes on a blocking thread.
async fn read_delivered(path: PathBuf) -> Result<Vec<u8>, OneShotError> {
    tokio::task::spawn_blocking(move || std::fs::read(path))
        .await
        .map_err(|_| OneShotError::Internal)?
        .map_err(|error| OneShotError::Unreadable {
            kind: format!("{:?}", error.kind()),
        })
}

fn map_unknown(reason: UnknownReason) -> OneShotError {
    match reason {
        UnknownReason::Deadline => OneShotError::Timeout,
        UnknownReason::Cancelled | UnknownReason::Interrupted | UnknownReason::LostOutput => {
            OneShotError::Interrupted
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_attempt_future_can_move_between_worker_threads() {
        fn assert_send<T: Send>(_: &T) {}
        let config = OneShotConfig {
            executable: PathBuf::from("codex"),
            work_root: std::env::temp_dir(),
            agent_model: None,
            codex_home: None,
            deadline: Duration::from_secs(1),
        };
        let request = OneShotRequest {
            prompt: "a lake".to_owned(),
            width_px: 16,
            height_px: 9,
            base: None,
            references: Vec::new(),
        };
        assert_send(&generate(&config, &request));
    }

    #[test]
    fn a_png_header_gives_its_real_size() {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&13_u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&1672_u32.to_be_bytes());
        bytes.extend_from_slice(&941_u32.to_be_bytes());
        assert_eq!(describe(&bytes), Some(("image/png", 1672, 941)));
    }

    fn tiny_png() -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbImage::new(2, 2)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    #[tokio::test]
    async fn references_are_staged_in_order_after_the_base() {
        let work_root = attempt_work_root("references");
        let (folder, attempt) = AttemptFolder::create(&work_root).await.unwrap();
        let picture = tiny_png();
        let request = OneShotRequest {
            prompt: "the two of them".to_owned(),
            width_px: 16,
            height_px: 9,
            base: Some(picture.clone()),
            references: vec![picture.clone(), picture],
        };
        let (kind, staged) = stage_pictures(attempt.inputs(), &request).await.unwrap();
        let names: Vec<_> = staged
            .iter()
            .map(|reference| reference.path().file_name().unwrap().to_owned())
            .collect();
        folder.remove().await.unwrap();
        let _ = std::fs::remove_dir_all(&work_root);
        assert_eq!(kind, ImageKind::Edit);
        assert_eq!(names, ["base.png", "reference-1.png", "reference-2.png"]);
    }

    #[tokio::test]
    async fn references_without_a_base_make_a_generation_that_follows_them() {
        let work_root = attempt_work_root("references-only");
        let (folder, attempt) = AttemptFolder::create(&work_root).await.unwrap();
        let request = OneShotRequest {
            prompt: "him again".to_owned(),
            width_px: 16,
            height_px: 9,
            base: None,
            references: vec![tiny_png()],
        };
        let (kind, staged) = stage_pictures(attempt.inputs(), &request).await.unwrap();
        folder.remove().await.unwrap();
        let _ = std::fs::remove_dir_all(&work_root);
        assert_eq!(kind, ImageKind::Generate);
        assert_eq!(staged.len(), 1);
    }

    #[tokio::test]
    async fn a_reference_that_is_not_a_picture_is_refused_by_name() {
        let work_root = attempt_work_root("reference-unreadable");
        let (folder, attempt) = AttemptFolder::create(&work_root).await.unwrap();
        let request = OneShotRequest {
            prompt: "him again".to_owned(),
            width_px: 16,
            height_px: 9,
            base: None,
            references: vec![b"not a picture".to_vec()],
        };
        let error = stage_pictures(attempt.inputs(), &request)
            .await
            .expect_err("text is no reference");
        folder.remove().await.unwrap();
        let _ = std::fs::remove_dir_all(&work_root);
        assert_eq!(
            error,
            OneShotError::InvalidRequest("a reference is not a readable picture")
        );
    }

    #[tokio::test]
    async fn more_pictures_than_the_backend_takes_are_refused_before_any_work() {
        let work = std::env::temp_dir();
        let config = OneShotConfig {
            executable: work.join("no-such-codex-executable.exe"),
            work_root: work,
            agent_model: None,
            codex_home: None,
            deadline: Duration::from_secs(5),
        };
        let request = OneShotRequest {
            prompt: "a crowd".to_owned(),
            width_px: 16,
            height_px: 9,
            base: Some(tiny_png()),
            references: vec![tiny_png(); MAXIMUM_PICTURES],
        };
        let error = generate(&config, &request)
            .await
            .expect_err("one picture too many");
        assert_eq!(
            error,
            OneShotError::InvalidRequest(
                "the request attaches more pictures than the backend takes"
            )
        );
    }

    #[test]
    fn bytes_that_are_not_a_picture_describe_as_nothing() {
        assert_eq!(describe(b"not a picture at all, just text"), None);
    }

    #[tokio::test]
    async fn an_unusable_request_is_refused_before_any_work() {
        let config = OneShotConfig {
            executable: PathBuf::from("codex"),
            work_root: std::env::temp_dir(),
            agent_model: None,
            codex_home: None,
            deadline: Duration::from_secs(1),
        };
        let request = OneShotRequest {
            prompt: "a lake".to_owned(),
            width_px: 16,
            height_px: 9,
            base: None,
            references: Vec::new(),
        };
        let error = generate(&config, &request)
            .await
            .expect_err("a relative path is refused");
        assert_eq!(error.key(), "invalid_config");
    }

    #[tokio::test]
    async fn a_missing_executable_is_a_transport_failure() {
        let work = std::env::temp_dir();
        let config = OneShotConfig {
            executable: work.join("no-such-codex-executable.exe"),
            work_root: work,
            agent_model: None,
            codex_home: None,
            deadline: Duration::from_secs(5),
        };
        let request = OneShotRequest {
            prompt: "a lake".to_owned(),
            width_px: 16,
            height_px: 9,
            base: None,
            references: Vec::new(),
        };
        let error = generate(&config, &request)
            .await
            .expect_err("nothing to run");
        assert_eq!(error.key(), "transport", "{error:?}");
    }

    #[tokio::test]
    async fn a_work_root_that_cannot_hold_folders_is_invalid_config_not_an_unreadable_picture() {
        let blocker = std::env::temp_dir().join(format!(
            "df-imagery-oneshot-blocker-{}-{}",
            std::process::id(),
            NEXT_ATTEMPT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&blocker, b"a file, not a folder").unwrap();
        let config = OneShotConfig {
            executable: std::env::temp_dir().join("no-such-codex-executable.exe"),
            work_root: blocker.clone(),
            agent_model: None,
            codex_home: None,
            deadline: Duration::from_secs(5),
        };
        let request = OneShotRequest {
            prompt: "a lake".to_owned(),
            width_px: 16,
            height_px: 9,
            base: None,
            references: Vec::new(),
        };
        let error = generate(&config, &request)
            .await
            .expect_err("no folder can be made");
        std::fs::remove_file(&blocker).unwrap();
        assert_eq!(error.key(), "invalid_config", "{error:?}");
    }

    fn attempt_work_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "df-imagery-oneshot-{name}-{}-{}",
            std::process::id(),
            NEXT_ATTEMPT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[tokio::test]
    async fn a_cancelled_attempt_folder_dropped_on_a_worker_is_removed_on_the_blocking_pool() {
        let work_root = attempt_work_root("cancelled");
        let (folder, _attempt) = AttemptFolder::create(&work_root).await.unwrap();
        let root = folder.root.clone().expect("a new folder has its root");
        assert!(root.is_dir());
        drop(folder);
        // The delete runs detached on the blocking pool: wait for it, bounded.
        for _ in 0..500 {
            if !root.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let removed = !root.exists();
        let _ = std::fs::remove_dir_all(&work_root);
        assert!(removed, "the cancelled attempt's folder was left behind");
    }

    #[test]
    fn an_attempt_folder_dropped_outside_a_runtime_is_removed_on_the_spot() {
        let work_root = attempt_work_root("unscheduled");
        let (folder, _attempt) = AttemptFolder::create_blocking(&work_root).unwrap();
        let root = folder.root.clone().expect("a new folder has its root");
        assert!(root.is_dir());
        drop(folder);
        let removed = !root.exists();
        let _ = std::fs::remove_dir_all(&work_root);
        assert!(removed, "the attempt's folder was left behind");
    }

    #[tokio::test]
    async fn a_folder_that_cannot_be_removed_surfaces_its_cause() {
        let work_root = attempt_work_root("cleanup");
        let (folder, _attempt) = AttemptFolder::create(&work_root).await.unwrap();
        let root = folder.root.clone().expect("a new folder has its root");
        // Removed out from under the attempt, so its own removal fails.
        std::fs::remove_dir_all(&root).unwrap();
        let cleanup = folder.remove().await;
        let _ = std::fs::remove_dir_all(&work_root);
        assert_eq!(
            cleanup,
            Err(CleanupError {
                cause: CleanupCause::NotFound
            })
        );
    }

    #[tokio::test]
    async fn a_removed_folder_reports_success() {
        let work_root = attempt_work_root("cleanup-ok");
        let (folder, _attempt) = AttemptFolder::create(&work_root).await.unwrap();
        let root = folder.root.clone().expect("a new folder has its root");
        let cleanup = folder.remove().await;
        let removed = !root.exists();
        let _ = std::fs::remove_dir_all(&work_root);
        assert_eq!(cleanup, Ok(()));
        assert!(removed);
    }

    #[test]
    fn a_cleanup_line_holds_closed_keys_only() {
        let failure = CleanupError {
            cause: CleanupCause::from_io(std::io::ErrorKind::PermissionDenied),
        };
        let error = OneShotError::Refused {
            detail: "free text that never reaches the line".to_owned(),
        };
        assert_eq!(
            cleanup_line(failure, &error),
            "IMAGERY cleanup_failed kind=permission_denied outcome=refused"
        );
        for cause in CleanupCause::ALL {
            assert_eq!(CleanupCause::from_key(cause.key()), Some(*cause));
        }
        assert_eq!(
            CleanupCause::from_io(std::io::ErrorKind::AddrInUse),
            CleanupCause::Other
        );
    }

    #[test]
    fn a_full_disk_or_locked_file_under_the_work_root_is_an_internal_failure_not_invalid_config() {
        for kind in [
            std::io::ErrorKind::StorageFull,
            std::io::ErrorKind::QuotaExceeded,
            std::io::ErrorKind::ResourceBusy,
            std::io::ErrorKind::Other,
        ] {
            let error = map_work_folder_failure(kind);
            assert_eq!(error.key(), "internal", "{kind:?} gave {error:?}");
        }
    }

    #[test]
    fn a_denied_or_read_only_work_root_is_invalid_config_naming_the_cause() {
        assert_eq!(
            map_work_folder_failure(std::io::ErrorKind::PermissionDenied),
            OneShotError::InvalidConfig("the work folder does not let this process create folders")
        );
        assert_eq!(
            map_work_folder_failure(std::io::ErrorKind::ReadOnlyFilesystem),
            OneShotError::InvalidConfig("the work folder is on a read-only file system")
        );
    }
}

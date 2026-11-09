//! The backends of the picture generator: the trait the service calls, the real Codex CLI backend
//! (through the image service's one-shot door in `df-imagery`), the owner's local Forge studio
//! (Qwen Image 2.1, `DF_IMAGERY_BACKEND=forge`) and the backend that says it is not switched on,
//! with the typed reasons a backend gives no picture.
//!
//! A backend is async: the service awaits it on the job's task, under the deadline and a grace
//! period, and the Codex backend looks at its files on the blocking pool. Every precondition is
//! checked before a job starts and names what to do: the executable on `PATH`, a Codex home that is
//! signed in (its login file is only looked for, never opened), no `config.toml` in that home, and
//! a scratch folder outside any git repository and any folder with an `AGENTS.md`.
//!
//! The Codex child inherits this process's environment, so the signed-in home is `CODEX_HOME` in
//! the environment `df-table serve` was started with: it is never guessed from the user's own
//! Codex folder.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::http::StatusCode;
use df_imagery::oneshot::{self, OneShotConfig, OneShotError, OneShotRequest};
use df_providers::{
    FORGE_IMAGE_MAXIMUM_SIDE_PX, FORGE_IMAGE_SIDE_STEP_PX, ForgeCause, ForgeClient, ForgeEngine,
    ForgeError, ForgeJobKind, ForgeMedia, NativeHttpCause,
};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};

use super::model::{BackendRequest, GeneratedImage};
use crate::forge::{FORGE_BACKEND, ForgeSetupError, client_from_environment};
use crate::io_cause::IoCause;

/// The login file a signed-in Codex home holds. Its presence is checked; it is never read.
const LOGIN_FILE: &str = "auth.json";
/// A Codex home with this file is not the dedicated, config-free one the service expects.
const CONFIG_FILE: &str = "config.toml";

// The environment variables the real backend reads. They match the runbook's names.

/// `live` with `BACKEND_VARIABLE` set to `codex-cli` or `forge` switches generation on.
pub const MODE_VARIABLE: &str = "DF_IMAGERY_MODE";
pub const BACKEND_VARIABLE: &str = "DF_IMAGERY_BACKEND";
/// An absolute path of the Codex executable; unset looks for it on `PATH`.
pub const EXECUTABLE_VARIABLE: &str = "DF_IMAGERY_CODEX_EXECUTABLE";
/// The folder each attempt gets a scratch folder in.
pub const WORK_DIRECTORY_VARIABLE: &str = "DF_IMAGERY_CODEX_WORKDIR";
pub const AGENT_MODEL_VARIABLE: &str = "DF_IMAGEGEN_AGENT_MODEL";
/// The dedicated, signed-in Codex home the Codex child runs with.
pub const CODEX_HOME_VARIABLE: &str = "CODEX_HOME";

/// The value of `MODE_VARIABLE` that switches generation on.
const LIVE_MODE: &str = "live";

/// Why a backend gave no picture. Each case says what to do about it.
///
/// `kind` and `detail` of the variants that come from `df_imagery::oneshot` are that crate's own
/// text (`OneShotError` carries text there). On the wire it is tagged `"cause"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum BackendError {
    /// The host was not started with the switches that turn picture generation on.
    NotSwitchedOn,
    /// The named Codex executable is not an absolute path of a file.
    ExecutableNotAFile { path: PathBuf },
    /// No Codex executable is on `PATH`.
    ExecutableNotOnPath,
    /// The Codex executable could not be started.
    ExecutableNotStartable { kind: String },
    /// No Codex home is set.
    NoCodexHome,
    /// The Codex home has no login file.
    NoLogin { home: PathBuf },
    /// The Codex home has a `config.toml`, so it is not the dedicated home.
    HomeHasConfig { home: PathBuf },
    /// The scratch folder is not absolute.
    WorkRootNotAbsolute { path: PathBuf },
    /// The scratch folder cannot be made. (`cause` is the variant's tag, so the file system's
    /// cause is `io_cause`.)
    WorkRootUnmakeable { path: PathBuf, io_cause: IoCause },
    /// The scratch folder is inside `folder`, a git repository or a folder with an `AGENTS.md`.
    WorkRootUnfit { path: PathBuf, folder: PathBuf },
    /// The image service's door refused the configuration or the request before any work.
    Unaccepted { detail: String },
    /// The deadline ended the attempt.
    Timeout,
    /// The attempt was interrupted after work may have started.
    Interrupted,
    /// The account's usage limit stopped it.
    UsageLimited,
    /// The service refused or failed the request.
    Refused { detail: String },
    /// The Forge studio gave no picture; `detail` is the closed key of its cause.
    ForgeFailed { detail: String },
    /// The delivered picture could not be read back.
    Unreadable { kind: String },
    /// The image backend failed inside the host.
    Internal,
}

impl BackendError {
    /// The serde name of the variant.
    pub fn key(&self) -> &'static str {
        match self {
            Self::NotSwitchedOn => "not_switched_on",
            Self::ExecutableNotAFile { .. } => "executable_not_a_file",
            Self::ExecutableNotOnPath => "executable_not_on_path",
            Self::ExecutableNotStartable { .. } => "executable_not_startable",
            Self::NoCodexHome => "no_codex_home",
            Self::NoLogin { .. } => "no_login",
            Self::HomeHasConfig { .. } => "home_has_config",
            Self::WorkRootNotAbsolute { .. } => "work_root_not_absolute",
            Self::WorkRootUnmakeable { .. } => "work_root_unmakeable",
            Self::WorkRootUnfit { .. } => "work_root_unfit",
            Self::Unaccepted { .. } => "unaccepted",
            Self::Timeout => "timeout",
            Self::Interrupted => "interrupted",
            Self::UsageLimited => "usage_limited",
            Self::Refused { .. } => "refused",
            Self::ForgeFailed { .. } => "forge_failed",
            Self::Unreadable { .. } => "unreadable",
            Self::Internal => "internal",
        }
    }

    /// The status a refusal for this reason is answered with: a backend that is not set up is a
    /// 503, a deadline a 504, a failure of the service a 502 and a failure inside the host a 500.
    pub fn status(&self) -> StatusCode {
        match self {
            Self::NotSwitchedOn
            | Self::ExecutableNotAFile { .. }
            | Self::ExecutableNotOnPath
            | Self::ExecutableNotStartable { .. }
            | Self::NoCodexHome
            | Self::NoLogin { .. }
            | Self::HomeHasConfig { .. }
            | Self::WorkRootNotAbsolute { .. }
            | Self::WorkRootUnmakeable { .. }
            | Self::WorkRootUnfit { .. }
            | Self::Unaccepted { .. }
            | Self::UsageLimited => StatusCode::SERVICE_UNAVAILABLE,
            Self::Timeout => StatusCode::GATEWAY_TIMEOUT,
            Self::Interrupted | Self::Refused { .. } | Self::ForgeFailed { .. } => {
                StatusCode::BAD_GATEWAY
            }
            Self::Unreadable { .. } | Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSwitchedOn => write!(
                formatter,
                concat!(
                    "picture generation is not switched on: start `df-table serve` with {}={} ",
                    "and {}=codex-cli (and {}=<signed-in home>) or forge"
                ),
                MODE_VARIABLE, LIVE_MODE, BACKEND_VARIABLE, CODEX_HOME_VARIABLE
            ),
            Self::ExecutableNotAFile { path } => write!(
                formatter,
                concat!(
                    "{} is not a file; set {} to the absolute path of codex.exe or unset it to ",
                    "find codex on PATH"
                ),
                path.display(),
                EXECUTABLE_VARIABLE
            ),
            Self::ExecutableNotOnPath => write!(
                formatter,
                concat!(
                    "the codex executable is not on PATH; install the Codex CLI or set {} to its ",
                    "absolute path"
                ),
                EXECUTABLE_VARIABLE
            ),
            Self::ExecutableNotStartable { kind } => write!(
                formatter,
                concat!(
                    "the codex executable could not be started ({}); check the path and that ",
                    "the file runs"
                ),
                kind
            ),
            Self::NoCodexHome => write!(
                formatter,
                concat!(
                    "no Codex home is set: start `df-table serve` with {}=<the dedicated ",
                    "signed-in folder> in its environment"
                ),
                CODEX_HOME_VARIABLE
            ),
            Self::NoLogin { home } => write!(
                formatter,
                concat!(
                    "the Codex home {} has no login: sign in once with `{}={} codex login` in a ",
                    "terminal, then try again"
                ),
                home.display(),
                CODEX_HOME_VARIABLE,
                home.display()
            ),
            Self::HomeHasConfig { home } => write!(
                formatter,
                concat!(
                    "the Codex home {} has a {}; the image service expects a dedicated home ",
                    "with none"
                ),
                home.display(),
                CONFIG_FILE
            ),
            Self::WorkRootNotAbsolute { path } => write!(
                formatter,
                "{} must be an absolute folder, not {}",
                WORK_DIRECTORY_VARIABLE,
                path.display()
            ),
            Self::WorkRootUnmakeable { path, io_cause } => write!(
                formatter,
                "the scratch folder {} cannot be made: {io_cause}",
                path.display()
            ),
            Self::WorkRootUnfit { path, folder } => write!(
                formatter,
                concat!(
                    "the scratch folder {} is inside {}, which is a git repository or holds an ",
                    "AGENTS.md; set {} to a folder outside any repository"
                ),
                path.display(),
                folder.display(),
                WORK_DIRECTORY_VARIABLE
            ),
            Self::Unaccepted { detail } => {
                write!(
                    formatter,
                    "the image service did not accept the call: {detail}"
                )
            }
            Self::Timeout => formatter.write_str(concat!(
                "the generation did not finish before its deadline; the provider may still have ",
                "done the work, and nothing was retried"
            )),
            Self::Interrupted => formatter.write_str(
                "the generation was interrupted after work may have started; nothing was retried",
            ),
            Self::UsageLimited => formatter.write_str(concat!(
                "the Codex account's usage limit stopped the generation; try again when the ",
                "limit resets"
            )),
            Self::Refused { detail } => write!(
                formatter,
                concat!(
                    "the image service refused or failed the request ({}); if the login expired, ",
                    "sign in again with `{}=<home> codex login`"
                ),
                detail, CODEX_HOME_VARIABLE
            ),
            Self::ForgeFailed { detail } => write!(
                formatter,
                "the forge studio gave no picture ({detail}); its queue and engine say why"
            ),
            Self::Unreadable { kind } => {
                write!(
                    formatter,
                    "the delivered picture could not be read ({kind})"
                )
            }
            Self::Internal => formatter.write_str("the image backend failed inside the host"),
        }
    }
}

impl std::error::Error for BackendError {}

/// Something that turns a request into a picture.
pub trait ImageBackend: Send + Sync {
    /// The name recorded in provenance and audit lines.
    fn name(&self) -> &'static str;

    /// Looks at every precondition without starting any work.
    fn check(&self) -> Result<(), BackendError>;

    /// Whether the backend was given a Codex home. Only a Codex backend has one.
    fn codex_home_set(&self) -> bool {
        false
    }

    /// Makes one picture within `deadline`. It may take minutes; dropping it stops the attempt.
    fn generate<'a>(
        &'a self,
        request: &'a BackendRequest,
        deadline: Duration,
    ) -> BoxFuture<'a, Result<GeneratedImage, BackendError>>;
}

/// The backend of a host that was not told to generate pictures.
#[derive(Debug, Clone, Copy)]
pub struct SwitchedOff;

impl ImageBackend for SwitchedOff {
    fn name(&self) -> &'static str {
        "none"
    }

    fn check(&self) -> Result<(), BackendError> {
        Err(BackendError::NotSwitchedOn)
    }

    fn generate<'a>(
        &'a self,
        _request: &'a BackendRequest,
        _deadline: Duration,
    ) -> BoxFuture<'a, Result<GeneratedImage, BackendError>> {
        Box::pin(std::future::ready(Err(BackendError::NotSwitchedOn)))
    }
}

/// Where the Codex backend runs and with which home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexSettings {
    /// An explicit executable; `None` looks for `codex` on `PATH`.
    pub executable: Option<PathBuf>,
    /// The directories of `PATH`, searched when there is no explicit executable.
    pub path_directories: Vec<PathBuf>,
    /// `CODEX_HOME` as this process has it.
    pub codex_home: Option<PathBuf>,
    /// Where each attempt gets a folder of its own.
    pub work_root: PathBuf,
    pub agent_model: Option<String>,
}

/// The folder a plain `df-table serve` uses for Codex image attempts.
pub fn default_work_root() -> PathBuf {
    std::env::temp_dir().join("df-codex-image-jobs")
}

impl CodexSettings {
    /// The settings this process's environment gives.
    pub fn from_process_environment() -> Self {
        Self {
            executable: crate::environment::path(EXECUTABLE_VARIABLE),
            path_directories: crate::environment::path(crate::environment::PATH_VARIABLE)
                .map(|path| std::env::split_paths(&path).collect())
                .unwrap_or_default(),
            codex_home: crate::environment::path(CODEX_HOME_VARIABLE),
            work_root: crate::environment::path(WORK_DIRECTORY_VARIABLE)
                .unwrap_or_else(default_work_root),
            agent_model: crate::environment::text(AGENT_MODEL_VARIABLE),
        }
    }
}

/// The real backend: the Codex CLI image tool, through the image service's backend.
#[derive(Debug, Clone)]
pub struct CodexBackend {
    settings: CodexSettings,
}

fn first_existing(directories: &[PathBuf]) -> Option<PathBuf> {
    let name = if cfg!(windows) { "codex.exe" } else { "codex" };
    directories
        .iter()
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

/// The first folder at or above `start` that makes it unfit for Codex turns: a git repository, or
/// a folder with an `AGENTS.md` whose instructions the turn would pick up.
fn unfit_ancestor(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|folder| folder.join(".git").exists() || folder.join("AGENTS.md").exists())
        .map(Path::to_path_buf)
}

impl CodexBackend {
    pub fn new(settings: CodexSettings) -> Self {
        Self { settings }
    }

    fn executable(&self) -> Result<PathBuf, BackendError> {
        if let Some(explicit) = &self.settings.executable {
            return if explicit.is_absolute() && explicit.is_file() {
                Ok(explicit.clone())
            } else {
                Err(BackendError::ExecutableNotAFile {
                    path: explicit.clone(),
                })
            };
        }
        first_existing(&self.settings.path_directories).ok_or(BackendError::ExecutableNotOnPath)
    }

    fn signed_in_home(&self) -> Result<&Path, BackendError> {
        let home = self
            .settings
            .codex_home
            .as_deref()
            .ok_or(BackendError::NoCodexHome)?;
        if !home.join(LOGIN_FILE).is_file() {
            return Err(BackendError::NoLogin {
                home: home.to_path_buf(),
            });
        }
        if home.join(CONFIG_FILE).exists() {
            return Err(BackendError::HomeHasConfig {
                home: home.to_path_buf(),
            });
        }
        Ok(home)
    }

    fn work_root(&self) -> Result<&Path, BackendError> {
        let root = self.settings.work_root.as_path();
        if !root.is_absolute() {
            return Err(BackendError::WorkRootNotAbsolute {
                path: root.to_path_buf(),
            });
        }
        std::fs::create_dir_all(root).map_err(|error| BackendError::WorkRootUnmakeable {
            path: root.to_path_buf(),
            io_cause: IoCause::from_kind(error.kind()),
        })?;
        if let Some(folder) = unfit_ancestor(root) {
            return Err(BackendError::WorkRootUnfit {
                path: root.to_path_buf(),
                folder,
            });
        }
        Ok(root)
    }

    /// The one-shot setup for an attempt within `deadline`, after every precondition. It looks at
    /// files and may create the work folder: call it on the blocking pool.
    fn one_shot_config(&self, deadline: Duration) -> Result<OneShotConfig, BackendError> {
        let executable = self.executable()?;
        let work_root = self.work_root()?.to_path_buf();
        let codex_home = self.signed_in_home()?.to_path_buf();
        Ok(OneShotConfig {
            executable,
            work_root,
            agent_model: self.settings.agent_model.clone(),
            codex_home: Some(codex_home),
            deadline,
        })
    }
}

impl ImageBackend for CodexBackend {
    fn name(&self) -> &'static str {
        oneshot::BACKEND_NAME
    }

    fn check(&self) -> Result<(), BackendError> {
        self.executable()?;
        self.signed_in_home()?;
        self.work_root()?;
        Ok(())
    }

    fn codex_home_set(&self) -> bool {
        self.settings.codex_home.is_some()
    }

    fn generate<'a>(
        &'a self,
        request: &'a BackendRequest,
        deadline: Duration,
    ) -> BoxFuture<'a, Result<GeneratedImage, BackendError>> {
        Box::pin(async move {
            let backend = self.clone();
            let config = tokio::task::spawn_blocking(move || backend.one_shot_config(deadline))
                .await
                .map_err(|_| BackendError::Internal)??;
            let one_shot = OneShotRequest {
                prompt: request.text.clone(),
                width_px: request.width_px,
                height_px: request.height_px,
                base: request.base.clone(),
            };
            let image = oneshot::generate(&config, &one_shot)
                .await
                .map_err(backend_error)?;
            Ok(GeneratedImage {
                bytes: image.bytes,
                mime: image.mime.to_owned(),
                width_px: image.width_px,
                height_px: image.height_px,
                sha256_hex: image.sha256_hex,
                duration: image.elapsed,
                input_tokens: image.input_tokens,
                output_tokens: image.output_tokens,
                cleanup: image.cleanup.err().map(|failure| failure.cause),
            })
        })
    }
}

fn backend_error(error: OneShotError) -> BackendError {
    match error {
        OneShotError::Timeout => BackendError::Timeout,
        OneShotError::Interrupted => BackendError::Interrupted,
        OneShotError::UsageLimited => BackendError::UsageLimited,
        OneShotError::Transport { kind } => BackendError::ExecutableNotStartable { kind },
        OneShotError::Refused { detail } => BackendError::Refused { detail },
        OneShotError::InvalidConfig(detail) | OneShotError::InvalidRequest(detail) => {
            BackendError::Unaccepted {
                detail: detail.to_owned(),
            }
        }
        OneShotError::Unreadable { kind } => BackendError::Unreadable { kind },
        OneShotError::Internal => BackendError::Internal,
    }
}

/// The backend this process's environment asks for: with `DF_IMAGERY_MODE=live`, the Codex
/// backend for `DF_IMAGERY_BACKEND=codex-cli` and the Forge backend for `forge`; otherwise one
/// that says it is not switched on.
pub fn backend_from_environment() -> Box<dyn ImageBackend> {
    let mode = crate::environment::text(MODE_VARIABLE);
    let backend = crate::environment::text(BACKEND_VARIABLE);
    if mode.as_deref() == Some(LIVE_MODE) && backend.as_deref() == Some(FORGE_BACKEND) {
        return Box::new(ForgeImageBackend::new(client_from_environment()));
    }
    backend_for(
        mode.as_deref(),
        backend.as_deref(),
        CodexSettings::from_process_environment(),
    )
}

/// The Forge backend: Qwen Image 2.1 in the owner's local studio, one queued render per picture.
/// A base picture to edit is staged as the render's one reference. Forge takes sides in steps of
/// 32 pixels, so the requested size is rounded to the nearest it takes; the picture's own size is
/// what is reported. It never retries: a render Forge reports interrupted stays for Forge's
/// Recovery panel.
pub struct ForgeImageBackend {
    client: Result<ForgeClient, ForgeSetupError>,
}

impl ForgeImageBackend {
    pub fn new(client: Result<ForgeClient, ForgeSetupError>) -> Self {
        Self { client }
    }

    fn client(&self) -> Result<&ForgeClient, BackendError> {
        self.client
            .as_ref()
            .map_err(|error| BackendError::Unaccepted {
                detail: error.to_string(),
            })
    }
}

impl ImageBackend for ForgeImageBackend {
    fn name(&self) -> &'static str {
        FORGE_BACKEND
    }

    fn check(&self) -> Result<(), BackendError> {
        self.client().map(|_| ())
    }

    fn generate<'a>(
        &'a self,
        request: &'a BackendRequest,
        deadline: Duration,
    ) -> BoxFuture<'a, Result<GeneratedImage, BackendError>> {
        Box::pin(async move {
            let client = self.client()?;
            let started = tokio::time::Instant::now();
            let until = started + deadline;
            client
                .prepare(ForgeEngine::Images, until)
                .await
                .map_err(forge_error)?;
            let mut references = Vec::new();
            if let Some(base) = &request.base {
                let staged = client
                    .stage_reference(base, until)
                    .await
                    .map_err(forge_error)?;
                references.push(staged.as_str().to_owned());
            }
            let body = serde_json::json!({
                "prompt": request.text,
                "width": forge_side(request.width_px),
                "height": forge_side(request.height_px),
                "references": references,
            });
            let render = client
                .render(ForgeJobKind::Images, body, until)
                .await
                .map_err(forge_error)?;
            let bytes = client
                .asset(&render.output, ForgeMedia::Png, until)
                .await
                .map_err(forge_error)?;
            let (width_px, height_px) = png_size(&bytes).ok_or(BackendError::Unreadable {
                kind: "png_header".to_owned(),
            })?;
            Ok(GeneratedImage {
                sha256_hex: df_imagery::oneshot_segment::picture_sha256(&bytes),
                bytes,
                mime: "image/png".to_owned(),
                width_px,
                height_px,
                duration: started.elapsed(),
                input_tokens: None,
                output_tokens: None,
                cleanup: None,
            })
        })
    }
}

/// `side` rounded to the nearest side Forge takes.
fn forge_side(side: u32) -> u32 {
    let step = FORGE_IMAGE_SIDE_STEP_PX;
    (side.saturating_add(step / 2) / step * step).clamp(step, FORGE_IMAGE_MAXIMUM_SIDE_PX)
}

/// The width and height a PNG's header gives, or `None` for bytes that are not a PNG.
fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") || png.get(12..16)? != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(png.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(png.get(20..24)?.try_into().ok()?);
    Some((width, height))
}

/// The backend error of a Forge failure.
fn forge_error(error: ForgeError) -> BackendError {
    match error.cause {
        ForgeCause::Deadline | ForgeCause::Transport(NativeHttpCause::Deadline) => {
            BackendError::Timeout
        }
        ForgeCause::JobInterrupted => BackendError::Interrupted,
        ForgeCause::InvalidConfiguration | ForgeCause::InvalidRequest => BackendError::Unaccepted {
            detail: error.to_string(),
        },
        ForgeCause::Malformed => BackendError::Unreadable {
            kind: error.cause.key().to_owned(),
        },
        cause => BackendError::ForgeFailed {
            detail: cause.key().to_owned(),
        },
    }
}

/// The backend for the two switches' values and the Codex settings.
pub fn backend_for(
    mode: Option<&str>,
    backend: Option<&str>,
    settings: CodexSettings,
) -> Box<dyn ImageBackend> {
    match (mode, backend) {
        (Some(LIVE_MODE), Some(oneshot::BACKEND_NAME)) => Box::new(CodexBackend::new(settings)),
        _ => Box::new(SwitchedOff),
    }
}

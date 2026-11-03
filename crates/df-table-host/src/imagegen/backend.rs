//! The backends of the picture generator: the trait the service calls, the real Codex CLI backend
//! (through the image service's one-shot door in `df-imagery`) and the backend that says it is not
//! switched on.
//!
//! A backend is blocking: the service runs it in a blocking task, because the Codex turn takes
//! minutes and its future cannot be moved between runtime threads. Every precondition is checked
//! before a job starts and names what to do: the executable on `PATH`, a Codex home that is
//! signed in (its login file is only looked for, never opened), no `config.toml` in that home, and
//! a scratch folder outside any git repository and any folder with an `AGENTS.md`.
//!
//! The Codex child inherits this process's environment, so the signed-in home is `CODEX_HOME` in
//! the environment `df-table serve` was started with: it is never guessed from the user's own
//! Codex folder.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use df_imagery::oneshot::{self, OneShotConfig, OneShotError, OneShotRequest};

use super::model::{BackendError, BackendRequest, GeneratedImage};

/// The login file a signed-in Codex home holds. Its presence is checked; it is never read.
const LOGIN_FILE: &str = "auth.json";
/// A Codex home with this file is not the dedicated, config-free one the service expects.
const CONFIG_FILE: &str = "config.toml";

/// Names of the environment variables the real backend reads. They match the runbook's names.
pub mod environment {
    pub const MODE: &str = "DF_IMAGERY_MODE";
    pub const BACKEND: &str = "DF_IMAGERY_BACKEND";
    pub const EXECUTABLE: &str = "DF_IMAGERY_CODEX_EXECUTABLE";
    pub const WORK_DIRECTORY: &str = "DF_IMAGERY_CODEX_WORKDIR";
    pub const AGENT_MODEL: &str = "DF_IMAGEGEN_AGENT_MODEL";
    pub const DEADLINE_SECS: &str = "DF_IMAGEGEN_DEADLINE_SECS";
    pub const PROVENANCE_DIRECTORY: &str = "DF_IMAGEGEN_PROVENANCE_DIR";
    pub const CODEX_HOME: &str = "CODEX_HOME";
}

/// Something that turns a request into a picture.
pub trait ImageBackend: Send + Sync {
    /// The name recorded in provenance and audit lines.
    fn name(&self) -> &'static str;

    /// Looks at every precondition without starting any work.
    fn check(&self) -> Result<(), BackendError>;

    /// Makes one picture within `deadline`. It may block for minutes.
    fn generate_blocking(
        &self,
        request: &BackendRequest,
        deadline: Duration,
    ) -> Result<GeneratedImage, BackendError>;
}

/// The backend of a host that was not told to generate pictures.
#[derive(Debug, Clone)]
pub struct NotConfigured {
    reason: String,
}

impl NotConfigured {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl ImageBackend for NotConfigured {
    fn name(&self) -> &'static str {
        "none"
    }

    fn check(&self) -> Result<(), BackendError> {
        Err(BackendError::NotConfigured(self.reason.clone()))
    }

    fn generate_blocking(
        &self,
        _request: &BackendRequest,
        _deadline: Duration,
    ) -> Result<GeneratedImage, BackendError> {
        Err(BackendError::NotConfigured(self.reason.clone()))
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

fn text(name: &str) -> Option<OsString> {
    std::env::var_os(name).filter(|value| !value.is_empty())
}

/// The folder a plain `df-table serve` uses for Codex image attempts.
pub fn default_work_root() -> PathBuf {
    std::env::temp_dir().join("df-codex-image-jobs")
}

impl CodexSettings {
    /// The settings this process's environment gives.
    pub fn from_process_environment() -> Self {
        Self {
            executable: text(environment::EXECUTABLE).map(PathBuf::from),
            path_directories: text("PATH")
                .map(|path| std::env::split_paths(&path).collect())
                .unwrap_or_default(),
            codex_home: text(environment::CODEX_HOME).map(PathBuf::from),
            work_root: text(environment::WORK_DIRECTORY)
                .map(PathBuf::from)
                .unwrap_or_else(default_work_root),
            agent_model: text(environment::AGENT_MODEL)
                .and_then(|model| model.into_string().ok())
                .filter(|model| !model.trim().is_empty()),
        }
    }
}

/// The real backend: the Codex CLI image tool, through the image service's backend.
#[derive(Debug, Clone)]
pub struct CodexBackend {
    settings: CodexSettings,
}

fn first_existing(directories: &[PathBuf]) -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["codex.exe"]
    } else {
        &["codex"]
    };
    directories
        .iter()
        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
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
                Err(BackendError::CodexMissing(format!(
                    "{} is not a file; set {} to the absolute path of codex.exe or unset it to find codex on PATH",
                    explicit.display(),
                    environment::EXECUTABLE
                )))
            };
        }
        first_existing(&self.settings.path_directories).ok_or_else(|| {
            BackendError::CodexMissing(format!(
                "the codex executable is not on PATH; install the Codex CLI or set {} to its absolute path",
                environment::EXECUTABLE
            ))
        })
    }

    fn signed_in_home(&self) -> Result<&Path, BackendError> {
        let home = self.settings.codex_home.as_deref().ok_or_else(|| {
            BackendError::NotSignedIn(format!(
                "no Codex home is set: start `df-table serve` with {}=<the dedicated signed-in folder> in its environment",
                environment::CODEX_HOME
            ))
        })?;
        if !home.join(LOGIN_FILE).is_file() {
            return Err(BackendError::NotSignedIn(format!(
                "the Codex home {} has no login: sign in once with `{}={} codex login` in a terminal, then try again",
                home.display(),
                environment::CODEX_HOME,
                home.display()
            )));
        }
        if home.join(CONFIG_FILE).exists() {
            return Err(BackendError::NotConfigured(format!(
                "the Codex home {} has a {CONFIG_FILE}; the image service expects a dedicated home with none",
                home.display()
            )));
        }
        Ok(home)
    }

    fn work_root(&self) -> Result<&Path, BackendError> {
        let root = self.settings.work_root.as_path();
        if !root.is_absolute() {
            return Err(BackendError::NotConfigured(format!(
                "{} must be an absolute folder",
                environment::WORK_DIRECTORY
            )));
        }
        std::fs::create_dir_all(root).map_err(|error| {
            BackendError::NotConfigured(format!(
                "the scratch folder {} cannot be made: {error}",
                root.display()
            ))
        })?;
        if let Some(folder) = unfit_ancestor(root) {
            return Err(BackendError::NotConfigured(format!(
                "the scratch folder {} is inside {}, which is a git repository or holds an AGENTS.md; set {} to a folder outside any repository",
                root.display(),
                folder.display(),
                environment::WORK_DIRECTORY
            )));
        }
        Ok(root)
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

    fn generate_blocking(
        &self,
        request: &BackendRequest,
        deadline: Duration,
    ) -> Result<GeneratedImage, BackendError> {
        let config = OneShotConfig {
            executable: self.executable()?,
            work_root: self.work_root()?.to_path_buf(),
            agent_model: self.settings.agent_model.clone(),
            deadline,
        };
        self.signed_in_home()?;
        let one_shot = OneShotRequest {
            prompt: request.text.clone(),
            width_px: request.width_px,
            height_px: request.height_px,
        };
        match oneshot::generate_blocking(&config, &one_shot) {
            Ok(image) => Ok(GeneratedImage {
                bytes: image.bytes,
                mime: image.mime.to_owned(),
                width_px: image.width_px,
                height_px: image.height_px,
                sha256_hex: image.sha256_hex,
                duration: image.elapsed,
                input_tokens: image.input_tokens,
                output_tokens: image.output_tokens,
            }),
            Err(error) => Err(map_error(error)),
        }
    }
}

fn map_error(error: OneShotError) -> BackendError {
    match error {
        OneShotError::Timeout => BackendError::Timeout,
        OneShotError::Interrupted => BackendError::Interrupted,
        OneShotError::UsageLimited => BackendError::UsageLimited,
        OneShotError::Transport { kind } => BackendError::CodexMissing(format!(
            "the codex executable could not be started ({kind}); check the path and that the file runs"
        )),
        OneShotError::Refused { detail } => BackendError::Refused(format!(
            "the image service refused or failed the request ({detail}); if the login expired, sign in again with `{}=<home> codex login`",
            environment::CODEX_HOME
        )),
        OneShotError::InvalidConfig(reason) | OneShotError::InvalidRequest(reason) => {
            BackendError::NotConfigured(reason.to_owned())
        }
        OneShotError::Unreadable { kind } => {
            BackendError::Failed(format!("the delivered picture could not be read ({kind})"))
        }
        OneShotError::Internal => {
            BackendError::Failed("the image backend failed inside the host".to_owned())
        }
    }
}

/// The backend this process's environment asks for: the Codex backend when the host was started
/// with `DF_IMAGERY_MODE=live` and `DF_IMAGERY_BACKEND=codex-cli`, otherwise one that says so.
pub fn backend_from_environment() -> Box<dyn ImageBackend> {
    backend_for(
        text(environment::MODE).and_then(|value| value.into_string().ok()),
        text(environment::BACKEND).and_then(|value| value.into_string().ok()),
        CodexSettings::from_process_environment(),
    )
}

/// The backend for the two switches' values and the Codex settings.
pub fn backend_for(
    mode: Option<String>,
    backend: Option<String>,
    settings: CodexSettings,
) -> Box<dyn ImageBackend> {
    match (mode.as_deref(), backend.as_deref()) {
        (Some("live"), Some("codex-cli")) => Box::new(CodexBackend::new(settings)),
        _ => Box::new(NotConfigured::new(format!(
            "picture generation is not switched on: start `df-table serve` with {}=live and {}=codex-cli (and {}=<signed-in home>)",
            environment::MODE,
            environment::BACKEND,
            environment::CODEX_HOME
        ))),
    }
}

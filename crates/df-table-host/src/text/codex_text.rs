//! The host's second raw text egress: a `df-ai` [`TextBackend`] over the signed-in Codex CLI.
//!
//! A private composition adapter like `responses_text`: it is reached only through the durable
//! text service (`text::TextRuntime`), and owns the executable, the attempt folder and the wire
//! shape. One request is one `codex exec` run: the instructions, the facts as JSON and the order of
//! the pictures go in the prompt; each picture is a file attached with `-i`; the output schema is a
//! file passed with `--output-schema`; the final message is read from `-o`. The run is ephemeral
//! (no saved session), read-only, in a fresh folder outside any repository, with standard input
//! closed, and is killed at the request's deadline. It uses whatever Codex home the host's
//! environment names (`CODEX_HOME`, else Codex's own default) and never reads its credentials. It
//! never retries or resends; prompts, pictures and answers never reach a log. A file or process
//! step that fails is one `TEXT codex_failed step=.. error_kind=..` line. Every file step runs on
//! the blocking pool; only the wait for the child is async.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use df_ai::text::{
    TextBackend, TextOutcome, TextReceipt, TextRequest, TextUsage, UnavailableReason,
};
use serde_json::Value;

use super::limits::MAXIMUM_CODEX_OUTPUT_BYTES;
use super::{facts_text, usage_from};
use crate::environment;

/// The environment variable that names the Codex executable.
pub const EXECUTABLE_VARIABLE: &str = "DF_TEXT_CODEX_EXECUTABLE";
/// The environment variable that names the folder Codex attempts run in.
pub const WORKDIR_VARIABLE: &str = "DF_TEXT_CODEX_WORKDIR";
/// The file in an attempt's folder that `codex exec -o` writes the run's last message to.
pub(super) const LAST_MESSAGE_FILE: &str = "last-message.txt";

static NEXT_ATTEMPT: AtomicU64 = AtomicU64::new(0);

/// Where Codex runs.
#[derive(Debug, Clone)]
pub struct CodexRoute {
    /// The Codex executable.
    pub executable: PathBuf,
    /// A folder outside any git repository and any folder holding an `AGENTS.md`; each attempt
    /// gets a fresh subfolder and removes it afterwards.
    pub work_root: PathBuf,
    /// An explicit model, or Codex's own default.
    pub model: Option<String>,
}

impl CodexRoute {
    /// The route this process's environment gives: the executable named by
    /// `EXECUTABLE_VARIABLE` (or `codex` on the path) and the folder named by `WORKDIR_VARIABLE`
    /// (or `df-codex-text` in the system's temporary folder). `None` when no executable is named
    /// and none is on the path.
    pub fn from_environment(model: Option<String>) -> Option<Self> {
        let named = environment::path(EXECUTABLE_VARIABLE).filter(|path| path.is_file());
        let executable = named.or_else(|| on_path("codex"))?;
        let work_root = environment::path(WORKDIR_VARIABLE)
            .unwrap_or_else(|| std::env::temp_dir().join("df-codex-text"));
        Some(Self {
            executable,
            work_root,
            model,
        })
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    let paths = environment::path("PATH")?;
    std::env::split_paths(&paths).find_map(|folder| {
        [format!("{name}.exe"), name.to_owned()]
            .into_iter()
            .map(|file| folder.join(file))
            .find(|path| path.is_file())
    })
}

/// A Codex backend; without a route every request is `Unavailable(Configuration)`.
pub struct CodexTextBackend {
    route: Option<CodexRoute>,
    record: Arc<dyn Fn(String) + Send + Sync>,
}

impl CodexTextBackend {
    /// A backend over `route` that writes its `TEXT codex_failed` lines through `record`.
    pub fn new(route: Option<CodexRoute>, record: Arc<dyn Fn(String) + Send + Sync>) -> Self {
        Self { route, record }
    }
}

/// The prompt of `request`: its instructions, its facts as JSON, and the order of the attached
/// pictures by label.
pub fn prompt(request: &TextRequest) -> String {
    let mut text = format!(
        "{}\n\n{}\n",
        request.instructions.trim(),
        facts_text(request)
    );
    if !request.images.is_empty() {
        text.push_str("\nThe attached images are, in order:\n");
        for (number, image) in request.images.iter().enumerate() {
            text.push_str(&format!("{}. {}\n", number + 1, image.label));
        }
    }
    text.push_str("\nDo not run commands or edit files. Reply with only the JSON object.");
    text
}

fn extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/webp" => "webp",
        _ => "jpg",
    }
}

/// The usage of the run's `turn.completed` event, if it reported one.
pub fn usage(events: &str) -> Option<TextUsage> {
    let usage = events
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("turn.completed"))
        .find_map(|event| event.get("usage").cloned())?;
    let count = |name: &str| usage.get(name).and_then(Value::as_u64);
    usage_from(
        None,
        count("input_tokens"),
        count("output_tokens"),
        count("cached_input_tokens"),
        count("reasoning_output_tokens"),
    )
}

/// The answer in the run's last message: one JSON object, alone or in a fenced block.
pub fn answer(message: &str) -> Option<Value> {
    let trimmed = message.trim();
    let inner = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.strip_suffix("```"))
        .unwrap_or(trimmed);
    serde_json::from_str::<Value>(inner.trim())
        .ok()
        .filter(Value::is_object)
}

/// The outcome of a finished run, from its exit, its last message and its event stream: an object
/// answer is `Answered`; a run that said its login is missing or refused is an authentication
/// refusal; a run that ended without an answer may have been served, so it is `Unknown` (never
/// resent); a message that is not an object is `Invalid`.
pub fn receipt(succeeded: bool, message: Option<&str>, events: &str, errors: &str) -> TextReceipt {
    let spent = usage(events);
    let outcome = match message.map(answer) {
        Some(Some(value)) => TextOutcome::Answered(value),
        Some(None) => TextOutcome::Invalid,
        None => {
            let lowered = errors.to_ascii_lowercase();
            if !succeeded
                && (lowered.contains("not logged in")
                    || lowered.contains("login")
                    || lowered.contains("unauthorized")
                    || lowered.contains("401"))
            {
                TextOutcome::Unavailable(UnavailableReason::Authentication)
            } else {
                TextOutcome::Unknown
            }
        }
    };
    TextReceipt {
        outcome,
        usage: spent,
    }
}

fn bounded(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(MAXIMUM_CODEX_OUTPUT_BYTES)]).into_owned()
}

/// One attempt's folder: made fresh, removed by `remove`, whose failure the run records. A folder
/// dropped without `remove` (a step failed while preparing it) is removed on drop.
struct AttemptFolder {
    path: PathBuf,
    removed: bool,
}

impl AttemptFolder {
    fn new(root: &Path) -> std::io::Result<Self> {
        let number = NEXT_ATTEMPT.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!("attempt-{}-{number}", std::process::id()));
        std::fs::create_dir_all(&path)?;
        Ok(Self {
            path,
            removed: false,
        })
    }

    fn remove(mut self) -> std::io::Result<()> {
        self.removed = true;
        std::fs::remove_dir_all(&self.path)
    }
}

impl Drop for AttemptFolder {
    fn drop(&mut self) {
        if !self.removed {
            // Only a preparation that failed half-way gets here, and that failure is already
            // recorded; a second line for its temporary folder would say nothing new.
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// A step of a run whose file or process work can fail, as its `TEXT codex_failed` line names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RunStep {
    /// The blocking task that prepares or finishes the attempt stopped without an answer.
    Task,
    Folder,
    Schema,
    Picture,
    Spawn,
    Wait,
    Message,
    /// Removing the attempt's folder after the run.
    Cleanup,
}

impl RunStep {
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Folder => "folder",
            Self::Schema => "schema",
            Self::Picture => "picture",
            Self::Spawn => "spawn",
            Self::Wait => "wait",
            Self::Message => "message",
            Self::Cleanup => "cleanup",
        }
    }

    /// This step's failure with `error` as its cause.
    fn failed(self, error: std::io::Error) -> StepError {
        StepError { step: self, error }
    }
}

/// A run step that failed, and the file or process error it gave.
#[derive(Debug)]
pub(super) struct StepError {
    pub(super) step: RunStep,
    pub(super) error: std::io::Error,
}

impl std::fmt::Display for StepError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "the codex run's {} step failed: {}",
            self.step.key(),
            self.error
        )
    }
}

impl std::error::Error for StepError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// One attempt's files, written before the run.
pub(super) struct PreparedAttempt {
    folder: AttemptFolder,
    schema: PathBuf,
    last: PathBuf,
    pictures: Vec<PathBuf>,
}

/// Makes the attempt's folder and writes the schema and each picture (with its file extension).
/// Blocks on files: call it from a blocking task.
pub(super) fn prepare(
    root: &Path,
    schema_text: &str,
    images: &[(&'static str, Vec<u8>)],
) -> Result<PreparedAttempt, StepError> {
    let folder = AttemptFolder::new(root).map_err(|error| RunStep::Folder.failed(error))?;
    let schema = folder.path.join("schema.json");
    let last = folder.path.join(LAST_MESSAGE_FILE);
    std::fs::write(&schema, schema_text).map_err(|error| RunStep::Schema.failed(error))?;
    let mut pictures = Vec::new();
    for (number, (extension, bytes)) in images.iter().enumerate() {
        let path = folder
            .path
            .join(format!("picture-{:02}.{extension}", number + 1));
        std::fs::write(&path, bytes).map_err(|error| RunStep::Picture.failed(error))?;
        pictures.push(path);
    }
    Ok(PreparedAttempt {
        folder,
        schema,
        last,
        pictures,
    })
}

/// What finishing an attempt found: the run's last message, if it left one, and every step that
/// failed on the way (reading the message, removing the folder).
pub(super) struct Finished {
    pub(super) message: Option<String>,
    pub(super) failures: Vec<StepError>,
}

/// Reads the run's last message and then removes the attempt's folder, on the blocking pool. A
/// failed removal is reported and never hides the message.
pub(super) async fn finish(attempt: PreparedAttempt) -> Finished {
    tokio::task::spawn_blocking(move || {
        let mut failures = Vec::new();
        let message = match std::fs::read_to_string(&attempt.last) {
            Ok(text) => Some(text),
            // No message file means the run gave no answer, which `receipt` makes `Unknown`.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                failures.push(RunStep::Message.failed(error));
                None
            }
        };
        if let Err(error) = attempt.folder.remove() {
            failures.push(RunStep::Cleanup.failed(error));
        }
        Finished { message, failures }
    })
    .await
    .unwrap_or_else(|error| Finished {
        message: None,
        failures: vec![RunStep::Task.failed(std::io::Error::other(error))],
    })
}

/// The `codex exec` command of one attempt.
fn command(
    route: &CodexRoute,
    request: &TextRequest,
    attempt: &PreparedAttempt,
) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(&route.executable);
    command
        .current_dir(&attempt.folder.path)
        .arg("exec")
        .args(["--skip-git-repo-check", "--ephemeral", "--json"])
        .args(["--sandbox", "read-only", "--color", "never"])
        .arg("--output-schema")
        .arg(&attempt.schema)
        .arg("--output-last-message")
        .arg(&attempt.last);
    if let Some(model) = &route.model {
        command.args(["--model", model]);
    }
    command.arg(prompt(request));
    for picture in &attempt.pictures {
        command.arg("--image").arg(picture);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

impl CodexTextBackend {
    fn record_failure(&self, failure: &StepError) {
        (self.record)(format!(
            "TEXT codex_failed step={} error_kind={:?}",
            failure.step.key(),
            failure.error.kind()
        ));
    }

    /// Finishes the attempt and records each step that failed; returns the run's last message.
    async fn finish_recorded(&self, attempt: PreparedAttempt) -> Option<String> {
        let finished = finish(attempt).await;
        finished
            .failures
            .iter()
            .for_each(|failure| self.record_failure(failure));
        finished.message
    }

    async fn run(&self, route: &CodexRoute, request: &TextRequest) -> TextReceipt {
        let configuration = || TextOutcome::Unavailable(UnavailableReason::Configuration).into();
        let root = route.work_root.clone();
        let schema_text = request.output_schema.to_string();
        let images: Vec<(&'static str, Vec<u8>)> = request
            .images
            .iter()
            .map(|image| (extension(image.media_type.mime()), image.bytes.clone()))
            .collect();
        let prepared = tokio::task::spawn_blocking(move || prepare(&root, &schema_text, &images))
            .await
            .unwrap_or_else(|error| Err(RunStep::Task.failed(std::io::Error::other(error))));
        let attempt = match prepared {
            Ok(attempt) => attempt,
            Err(failure) => {
                self.record_failure(&failure);
                return configuration();
            }
        };
        let child = match command(route, request, &attempt).spawn() {
            Ok(child) => child,
            Err(error) => {
                self.record_failure(&RunStep::Spawn.failed(error));
                // Nothing ran, so there is no message; only the folder's removal matters here.
                self.finish_recorded(attempt).await;
                return configuration();
            }
        };
        // The text service has already bounded the deadline (`df_ai::text::TextLimits`). The child
        // is killed when the timed-out future drops it.
        let waited = tokio::time::timeout(request.deadline, child.wait_with_output()).await;
        let message = self.finish_recorded(attempt).await;
        match waited {
            Err(_) => TextOutcome::Deadline.into(),
            Ok(Err(error)) => {
                self.record_failure(&RunStep::Wait.failed(error));
                TextOutcome::Unknown.into()
            }
            Ok(Ok(output)) => receipt(
                output.status.success(),
                message.as_deref().filter(|text| !text.trim().is_empty()),
                &bounded(&output.stdout),
                &bounded(&output.stderr),
            ),
        }
    }
}

impl TextBackend for CodexTextBackend {
    async fn complete(&self, request: &TextRequest) -> TextReceipt {
        let Some(route) = &self.route else {
            return TextOutcome::Unavailable(UnavailableReason::Configuration).into();
        };
        self.run(route, request).await
    }
}

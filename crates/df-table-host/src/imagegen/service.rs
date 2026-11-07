//! The picture generator: it starts a job, runs the backend in a blocking task under a named
//! deadline, checks the bytes, stores the picture through the media library's staged upload and
//! records the request and the outcome.
//!
//! The host owns the `.await`: the backend is blocking and runs in `spawn_blocking`, and the job
//! itself is a task that goes on when the caller stops waiting (a dev command that gives up after
//! its own timeout does not cancel a Codex turn that is three minutes in). The job's task handle is
//! kept in its record, with the job's state for `media generate status`; the task ends when its
//! job does and is cancelled only when the runtime stops (a blocking backend call cannot be
//! stopped part way, so nothing else cancels it). Nothing here limits how many pictures are made
//! or what they cost: the ledger of this feature is the audit line, which records and never
//! blocks.
//!
//! The picture is stored as the backend delivered it. The workspace's WebP encoder lives behind
//! the image service and is not reachable from the host, so a Codex PNG stays a PNG (an accepted
//! media type), and the provenance says so.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use df_table::ports::Clock;
use df_types::saturating_millis;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::backend::{BackendError, ImageBackend, backend_from_environment};
use super::events::ImageGenEvent;
use super::limits::{DEFAULT_DEADLINE, DEFAULT_GRACE, MAXIMUM_REMEMBERED_JOBS, PROVENANCE_SUFFIX};
use super::model::{BackendRequest, GeneratedImage, ImageRequest};
use super::request::{GenerateError, JobState, JobView, PictureCause, Provenance, Sidecar};
use crate::io_cause::IoCause;
use crate::media_library::{media_directory, store_bytes};
use crate::state::AppState;

/// The variable that sets the deadline of each generation, in whole seconds above zero.
pub const DEADLINE_SECONDS_VARIABLE: &str = "DF_IMAGEGEN_DEADLINE_SECS";
/// The variable naming a folder for each stored picture's provenance sidecar.
pub const PROVENANCE_DIRECTORY_VARIABLE: &str = "DF_IMAGEGEN_PROVENANCE_DIR";

/// A deadline setting that is not a whole number of seconds above zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadlineSettingError {
    pub text: String,
}

impl std::fmt::Display for DeadlineSettingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "the deadline '{}' is not a whole number of seconds above zero",
            self.text
        )
    }
}

impl std::error::Error for DeadlineSettingError {}

/// The deadline a setting's text asks for; no text is `DEFAULT_DEADLINE`.
pub fn deadline_setting(text: Option<&str>) -> Result<Duration, DeadlineSettingError> {
    let Some(text) = text else {
        return Ok(DEFAULT_DEADLINE);
    };
    text.parse::<u64>()
        .ok()
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .ok_or_else(|| DeadlineSettingError {
            text: text.to_owned(),
        })
}

/// The deadline the setting's text asks for or, when it does not read, `DEFAULT_DEADLINE` with
/// an audit line that names the setting.
pub(super) fn deadline_or_default(app: &AppState, text: Option<&str>) -> Duration {
    deadline_setting(text).unwrap_or_else(|_| {
        app.record_line_and_echo(
            ImageGenEvent::InvalidSetting {
                variable: DEADLINE_SECONDS_VARIABLE,
                default: DEFAULT_DEADLINE,
            }
            .line(),
        );
        DEFAULT_DEADLINE
    })
}

/// Monotonic time since the generator was made: the default clock, read only here at the edge.
struct MonotonicClock {
    origin: Instant,
}

impl Clock for MonotonicClock {
    fn now_ms(&self) -> u64 {
        saturating_millis(self.origin.elapsed())
    }
}

/// The job's end as its waiter receives it: its stored picture or the reason it has none. The
/// sender is gone without an answer when the job's task stopped (it panicked or the runtime is
/// stopping).
pub type JobEnd = oneshot::Receiver<Result<JobView, GenerateError>>;

/// A started job: how it stands now and the receiver of its end. Dropping `finished` stops only the
/// waiting; the job goes on and its record keeps its task.
pub struct Started {
    pub view: JobView,
    pub finished: JobEnd,
}

struct JobRecord {
    job: u64,
    name: String,
    deadline: Duration,
    started_ms: u64,
    /// Set when the job ended.
    elapsed_ms: Option<u64>,
    state: JobState,
    /// The job's task, kept by its owner. Set right after the task is spawned; dropped with the
    /// record, which is never forgotten while the job runs.
    task: Option<JoinHandle<()>>,
}

impl JobRecord {
    fn view(&self, now_ms: u64) -> JobView {
        JobView {
            job: self.job,
            name: self.name.clone(),
            deadline_ms: saturating_millis(self.deadline),
            elapsed_ms: self
                .elapsed_ms
                .unwrap_or_else(|| now_ms.saturating_sub(self.started_ms)),
            state: self.state.clone(),
        }
    }

    fn is_running(&self) -> bool {
        self.state == JobState::Running
    }
}

/// Drops the oldest items that are not `running` until fewer than `maximum` are left, so one more
/// fits. Running items are kept even past `maximum`.
pub(super) fn forget_oldest_finished<T>(
    items: &mut Vec<T>,
    maximum: usize,
    running: impl Fn(&T) -> bool,
) {
    while items.len() >= maximum {
        let Some(oldest) = items.iter().position(|item| !running(item)) else {
            return;
        };
        items.remove(oldest);
    }
}

/// The generator: one backend, the deadline each job gets and the jobs so far.
pub struct ImageGenerator {
    backend: Arc<dyn ImageBackend>,
    deadline: Duration,
    grace: Duration,
    provenance_directory: Option<PathBuf>,
    clock: Arc<dyn Clock>,
    jobs: Mutex<Vec<JobRecord>>,
    next_job: AtomicU64,
}

impl ImageGenerator {
    pub fn new(backend: Arc<dyn ImageBackend>) -> Self {
        Self {
            backend,
            deadline: DEFAULT_DEADLINE,
            grace: DEFAULT_GRACE,
            provenance_directory: None,
            clock: Arc::new(MonotonicClock {
                origin: Instant::now(),
            }),
            jobs: Mutex::new(Vec::new()),
            next_job: AtomicU64::new(1),
        }
    }

    /// The generator this process's environment asks for. A deadline setting that does not read
    /// is named in an audit line and the default deadline is used.
    pub fn from_environment(app: &AppState) -> Self {
        let text = crate::environment::text(DEADLINE_SECONDS_VARIABLE);
        let deadline = deadline_or_default(app, text.as_deref());
        let mut generator =
            Self::new(Arc::from(backend_from_environment())).with_deadline(deadline);
        if let Some(directory) = crate::environment::path(PROVENANCE_DIRECTORY_VARIABLE) {
            generator = generator.with_provenance_directory(directory);
        }
        generator
    }

    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    pub fn with_grace(mut self, grace: Duration) -> Self {
        self.grace = grace;
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Writes `<name>.provenance.json` into `directory` after each stored picture.
    pub fn with_provenance_directory(mut self, directory: PathBuf) -> Self {
        self.provenance_directory = Some(directory);
        self
    }

    pub fn deadline(&self) -> Duration {
        self.deadline
    }

    /// The backend's name, as provenance and audit lines show it.
    pub fn backend_name(&self) -> &'static str {
        self.backend.name()
    }

    fn jobs(&self) -> MutexGuard<'_, Vec<JobRecord>> {
        self.jobs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether stored pictures get a provenance sidecar.
    pub fn provenance_directory_set(&self) -> bool {
        self.provenance_directory.is_some()
    }

    /// Whether the backend runs with a Codex home (its path is never shown).
    pub fn codex_home_set(&self) -> bool {
        self.backend.codex_home_set()
    }

    /// How many jobs are running now.
    pub fn running(&self) -> usize {
        self.jobs()
            .iter()
            .filter(|record| record.is_running())
            .count()
    }

    /// How the job that ended last ended: `stored`, or its error's key. `None` before any ended.
    pub fn last_outcome(&self) -> Option<&'static str> {
        let jobs = self.jobs();
        let ended = jobs
            .iter()
            .filter_map(|record| {
                let elapsed_ms = record.elapsed_ms?;
                Some((
                    record.started_ms.saturating_add(elapsed_ms),
                    record.job,
                    record,
                ))
            })
            .max_by_key(|(ended_ms, job, _)| (*ended_ms, *job))?;
        match &ended.2.state {
            JobState::Stored { .. } => Some("stored"),
            JobState::Failed { error, .. } => Some(error.key()),
            JobState::Running => None,
        }
    }

    /// Every job remembered, oldest first.
    pub fn views(&self) -> Vec<JobView> {
        let now_ms = self.clock.now_ms();
        self.jobs()
            .iter()
            .map(|record| record.view(now_ms))
            .collect()
    }

    /// Checks the preconditions, then starts the job. A refusal here is typed and nothing ran;
    /// the caller records it.
    pub async fn start(
        self: &Arc<Self>,
        app: &Arc<AppState>,
        request: ImageRequest,
    ) -> Result<Started, GenerateError> {
        let asked_ms = self.clock.now_ms();
        self.check(app, asked_ms).await?;
        let (view, running) = self.open_job(request.name())?;
        app.record_line(
            ImageGenEvent::Requested {
                job: view.job,
                name: request.name().to_owned(),
                purpose: request.purpose(),
                aspect: request.aspect(),
                prompt_chars: request.prompt().chars().count(),
                deadline: self.deadline,
                backend: self.backend.name(),
                running,
            }
            .line(),
        );
        let generator = Arc::clone(self);
        let app = Arc::clone(app);
        let job = view.job;
        let (end, finished) = oneshot::channel();
        let task = tokio::spawn(async move {
            let wait_ms = generator.clock.now_ms().saturating_sub(asked_ms);
            app.record_line_and_echo(
                ImageGenEvent::Started {
                    job,
                    wait: Duration::from_millis(wait_ms),
                }
                .line(),
            );
            let outcome = generator.run(&app, job, request).await;
            // A waiter that went away stopped only waiting; the record holds the job's end.
            let _ = end.send(outcome);
        });
        self.keep_task(job, task);
        Ok(Started { view, finished })
    }

    /// Looks at the backend's preconditions on the blocking pool and records how long that took
    /// and what it found. A check task that stopped without an answer found nothing to record.
    async fn check(&self, app: &AppState, asked_ms: u64) -> Result<(), GenerateError> {
        let backend = Arc::clone(&self.backend);
        let checked = tokio::task::spawn_blocking(move || backend.check())
            .await
            .map_err(|_| GenerateError::Stopped { job: None })?;
        let checked_ms = self.clock.now_ms().saturating_sub(asked_ms);
        app.record_line(
            ImageGenEvent::Check {
                backend: self.backend.name(),
                duration: Duration::from_millis(checked_ms),
                failed: checked.as_ref().err().cloned(),
            }
            .line(),
        );
        checked.map_err(|backend| GenerateError::Backend { job: None, backend })
    }

    /// Hands the job's task to its record. The record is there: a running job is never
    /// forgotten, and one that already ended keeps its record until a later job opens.
    fn keep_task(&self, job: u64, task: JoinHandle<()>) {
        if let Some(record) = self.jobs().iter_mut().find(|record| record.job == job) {
            record.task = Some(task);
        }
    }

    /// Adds a running job named `name`, unless one of that name is running. Gives its view and how
    /// many jobs are running now, this one included.
    fn open_job(&self, name: &str) -> Result<(JobView, usize), GenerateError> {
        let mut jobs = self.jobs();
        if jobs
            .iter()
            .any(|record| record.name == name && record.is_running())
        {
            return Err(GenerateError::NameBusy {
                name: name.to_owned(),
            });
        }
        forget_oldest_finished(&mut jobs, MAXIMUM_REMEMBERED_JOBS, JobRecord::is_running);
        let now_ms = self.clock.now_ms();
        let record = JobRecord {
            job: self.next_job.fetch_add(1, Ordering::Relaxed),
            name: name.to_owned(),
            deadline: self.deadline,
            started_ms: now_ms,
            elapsed_ms: None,
            state: JobState::Running,
            task: None,
        };
        let view = record.view(now_ms);
        jobs.push(record);
        let running = jobs.iter().filter(|record| record.is_running()).count();
        Ok((view, running))
    }

    async fn run(
        &self,
        app: &AppState,
        job: u64,
        request: ImageRequest,
    ) -> Result<JobView, GenerateError> {
        let outcome = self.make_and_store(app, job, &request).await;
        let state = match &outcome {
            Ok((provenance, sidecar)) => JobState::Stored {
                provenance: Box::new(provenance.clone()),
                sidecar: sidecar.clone(),
            },
            Err(error) => JobState::Failed {
                error: error.clone(),
                message: error.to_string(),
            },
        };
        let view = self.close_job(job, request.name(), state);
        if let Err(error) = outcome {
            app.record_line_and_echo(
                ImageGenEvent::Failed {
                    job,
                    name: request.name().to_owned(),
                    error: error.clone(),
                    duration: Duration::from_millis(view.elapsed_ms),
                    backend: self.backend.name(),
                }
                .line(),
            );
            return Err(error);
        }
        Ok(view)
    }

    /// Sets how the job ended and gives its view.
    fn close_job(&self, job: u64, name: &str, state: JobState) -> JobView {
        let now_ms = self.clock.now_ms();
        let mut jobs = self.jobs();
        // A running job is never forgotten, so its record is there.
        match jobs.iter_mut().find(|record| record.job == job) {
            Some(record) => {
                record.elapsed_ms = Some(now_ms.saturating_sub(record.started_ms));
                record.state = state;
                record.view(now_ms)
            }
            None => JobView {
                job,
                name: name.to_owned(),
                deadline_ms: saturating_millis(self.deadline),
                elapsed_ms: 0,
                state,
            },
        }
    }

    async fn make_and_store(
        &self,
        app: &AppState,
        job: u64,
        request: &ImageRequest,
    ) -> Result<(Provenance, Option<Sidecar>), GenerateError> {
        let (width_px, height_px) = request.aspect().requested_size();
        let backend_request = BackendRequest {
            text: request.sent_text(),
            width_px,
            height_px,
        };
        let image = self.generate(job, backend_request).await?;
        if let Some(cause) = image.cleanup {
            app.record_line_and_echo(ImageGenEvent::CleanupFailed { job, cause }.line());
        }
        check_picture(&image).map_err(|cause| GenerateError::InvalidPicture { job, cause })?;
        let file = format!("{}.{}", request.name(), extension_of(&image.mime));
        let directory = media_directory(app.playground().files());
        store_bytes(directory, &file, &image.bytes)
            .await
            .map_err(|failure| GenerateError::StoreFailed {
                job,
                detail: failure.to_string(),
            })?;
        let provenance = Provenance {
            backend: self.backend.name().to_owned(),
            purpose: request.purpose(),
            aspect: request.aspect(),
            requested_width_px: width_px,
            requested_height_px: height_px,
            prompt: request.prompt().to_owned(),
            sent_text: request.sent_text(),
            duration_ms: saturating_millis(image.duration),
            deadline_ms: saturating_millis(self.deadline),
            width_px: image.width_px,
            height_px: image.height_px,
            mime: image.mime.clone(),
            bytes: u64::try_from(image.bytes.len()).unwrap_or(u64::MAX),
            sha256: image.sha256_hex.clone(),
            input_tokens: image.input_tokens,
            output_tokens: image.output_tokens,
            media_file: file.clone(),
            note: stored_as_delivered_note(&image.mime),
        };
        app.record_line_and_echo(
            ImageGenEvent::Stored {
                job,
                name: request.name().to_owned(),
                file,
                bytes: provenance.bytes,
                width_px: image.width_px,
                height_px: image.height_px,
                duration: image.duration,
                backend: self.backend.name(),
                sha256_prefix: image.sha256_hex.chars().take(16).collect(),
                tokens_in: image.input_tokens,
                tokens_out: image.output_tokens,
            }
            .line(),
        );
        let sidecar = self.write_provenance(request.name(), &provenance).await;
        Ok((provenance, sidecar))
    }

    /// Awaits the backend on the job's task. The backend enforces the deadline itself; the host
    /// adds a grace period and then calls the attempt timed out and drops it, which stops the
    /// attempt, so a stuck backend cannot hold a job open forever.
    async fn generate(
        &self,
        job: u64,
        backend_request: BackendRequest,
    ) -> Result<GeneratedImage, GenerateError> {
        let limit = self.deadline.saturating_add(self.grace);
        let attempt = self.backend.generate(&backend_request, self.deadline);
        match tokio::time::timeout(limit, attempt).await {
            Ok(generated) => generated.map_err(|backend| GenerateError::Backend {
                job: Some(job),
                backend,
            }),
            Err(_) => Err(GenerateError::Backend {
                job: Some(job),
                backend: BackendError::Timeout,
            }),
        }
    }

    /// Writes `<name>.provenance.json` when a directory was given. A sidecar that is not written
    /// is reported in the job and never fails it: the picture is already stored.
    async fn write_provenance(&self, name: &str, provenance: &Provenance) -> Option<Sidecar> {
        let directory = self.provenance_directory.clone()?;
        let file_name = format!("{name}{PROVENANCE_SUFFIX}");
        let path = directory.join(&file_name);
        let text = serde_json::to_vec_pretty(provenance);
        let written = tokio::task::spawn_blocking(move || {
            crate::atomic_file::write_atomically(&directory, &file_name, &text?)
        })
        .await
        .map_err(|_| IoCause::Other)
        .and_then(|written| written.map_err(|error| IoCause::from_kind(error.kind())));
        Some(match written {
            Ok(()) => Sidecar::Written { path },
            Err(cause) => Sidecar::NotWritten { path, cause },
        })
    }
}

fn extension_of(mime: &str) -> &'static str {
    match mime {
        "image/webp" => "webp",
        "image/jpeg" => "jpg",
        _ => "png",
    }
}

fn stored_as_delivered_note(mime: &str) -> String {
    format!("stored exactly as delivered ({mime}); the host has no encoder, so a PNG stays a PNG")
}

/// Refuses bytes that are not the picture the backend says they are: the signature must match the
/// type and the size must be real. The media library only checks a picture's size, so this is the
/// one place a text file named `.png` is caught.
pub(crate) fn check_picture(image: &GeneratedImage) -> Result<(), PictureCause> {
    if image.bytes.is_empty() {
        return Err(PictureCause::Empty);
    }
    let signature_matches = match image.mime.as_str() {
        "image/png" => image.bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => image.bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/webp" => {
            image.bytes.len() >= 12
                && &image.bytes[0..4] == b"RIFF"
                && &image.bytes[8..12] == b"WEBP"
        }
        _ => false,
    };
    if !signature_matches {
        return Err(PictureCause::NotOfType {
            mime: image.mime.clone(),
        });
    }
    if image.width_px == 0 || image.height_px == 0 {
        return Err(PictureCause::NoSize);
    }
    Ok(())
}

//! The picture generator: it starts a job, runs the backend in a blocking task under a named
//! deadline, checks the bytes, stores the picture through the media library's staged upload and
//! records the request and the outcome.
//!
//! The host owns the `.await`: the backend is blocking and runs in `spawn_blocking`, and the job
//! itself is a task that goes on when the caller stops waiting (a dev command that gives up after
//! its own timeout does not cancel a Codex turn that is three minutes in). A job's state is kept
//! for `media generate status`. Nothing here limits how many pictures are made or what they cost:
//! the ledger of this feature is the audit line, which records and never blocks.
//!
//! The picture is stored as the backend delivered it. The workspace's WebP encoder lives behind
//! the image service and is not reachable from the host, so a Codex PNG stays a PNG (an accepted
//! media type), and the provenance says so.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use base64::Engine;
use tokio::task::JoinHandle;

use super::backend::ImageBackend;
use super::events::ImageGenEvent;
use super::model::{
    BackendError, BackendRequest, DEFAULT_DEADLINE, DEFAULT_GRACE, GeneratedImage, ImageRequest,
    JobState, JobView, Provenance, RefusalCause,
};
use crate::media_ingest::MediaLimits;
use crate::media_store::{AddFailure, ingest_upload};

/// Bytes of a picture encoded and sent to the staging thread at a time: a multiple of three, so
/// every piece but the last encodes to base64 without padding.
const UPLOAD_PIECE_BYTES: usize = 3 * 64 * 1024;
const UPLOAD_QUEUE_PIECES: usize = 4;

/// What a generation needs from the host around it: where the media library is and where a line
/// for the audit log goes.
#[derive(Clone)]
pub struct Context {
    pub media_directory: PathBuf,
    pub record: Arc<dyn Fn(String) + Send + Sync>,
}

impl Context {
    fn record(&self, event: &ImageGenEvent) {
        (self.record)(event.line());
    }
}

/// A request that did not become a job, or a job that ended without a stored picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub job: Option<u64>,
    pub cause: RefusalCause,
    pub message: String,
}

impl Refusal {
    fn of(job: Option<u64>, error: &BackendError) -> Self {
        Self {
            job,
            cause: error.cause(),
            message: error.message(),
        }
    }
}

/// A started job: its number, its deadline and a handle that ends with its final view.
pub struct Started {
    pub job: u64,
    pub name: String,
    pub deadline: Duration,
    pub finished: JoinHandle<JobView>,
}

struct JobRecord {
    job: u64,
    name: String,
    started: Instant,
    state: JobState,
    elapsed: Option<Duration>,
    cause: Option<RefusalCause>,
    message: Option<String>,
    provenance: Option<Provenance>,
    provenance_file: Option<String>,
}

impl JobRecord {
    fn view(&self) -> JobView {
        let elapsed = self.elapsed.unwrap_or_else(|| self.started.elapsed());
        JobView {
            job: self.job,
            name: self.name.clone(),
            state: self.state,
            elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
            cause: self.cause,
            message: self.message.clone(),
            provenance: self.provenance.clone(),
            provenance_file: self.provenance_file.clone(),
        }
    }
}

/// The generator: one backend, the deadline each job gets and the jobs so far.
pub struct ImageGenerator {
    backend: Arc<dyn ImageBackend>,
    deadline: Duration,
    grace: Duration,
    provenance_directory: Option<PathBuf>,
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
            jobs: Mutex::new(Vec::new()),
            next_job: AtomicU64::new(1),
        }
    }

    /// The generator this process's environment asks for.
    pub fn from_environment() -> Self {
        use super::backend::{backend_from_environment, environment};
        let deadline = std::env::var(environment::DEADLINE_SECS)
            .ok()
            .and_then(|text| text.trim().parse::<u64>().ok())
            .filter(|seconds| *seconds > 0)
            .map_or(DEFAULT_DEADLINE, Duration::from_secs);
        let mut generator =
            Self::new(Arc::from(backend_from_environment())).with_deadline(deadline);
        if let Some(directory) = std::env::var_os(environment::PROVENANCE_DIRECTORY)
            .filter(|directory| !directory.is_empty())
        {
            generator = generator.with_provenance_directory(PathBuf::from(directory));
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

    /// Writes `<name>.provenance.json` into `directory` after each stored picture.
    pub fn with_provenance_directory(mut self, directory: PathBuf) -> Self {
        self.provenance_directory = Some(directory);
        self
    }

    pub fn deadline(&self) -> Duration {
        self.deadline
    }

    fn jobs(&self) -> std::sync::MutexGuard<'_, Vec<JobRecord>> {
        self.jobs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Every job so far, oldest first.
    pub fn views(&self) -> Vec<JobView> {
        self.jobs().iter().map(JobRecord::view).collect()
    }

    fn refuse(&self, context: &Context, name: &str, refusal: Refusal) -> Refusal {
        context.record(&ImageGenEvent::refused(
            name,
            refusal.cause,
            self.backend.name(),
        ));
        refusal
    }

    /// Checks the preconditions, then starts the job. A refusal here is typed and nothing ran.
    pub async fn start(
        self: &Arc<Self>,
        context: &Context,
        request: ImageRequest,
    ) -> Result<Started, Refusal> {
        let backend = Arc::clone(&self.backend);
        let checked = tokio::task::spawn_blocking(move || backend.check())
            .await
            .unwrap_or_else(|_| {
                Err(BackendError::Failed(
                    "the precondition check failed".to_owned(),
                ))
            });
        if let Err(error) = checked {
            let refusal = Refusal::of(None, &error);
            return Err(self.refuse(context, &request.name, refusal));
        }
        let job = {
            let mut jobs = self.jobs();
            if jobs
                .iter()
                .any(|record| record.name == request.name && record.state == JobState::Running)
            {
                drop(jobs);
                let refusal = Refusal {
                    job: None,
                    cause: RefusalCause::NameBusy,
                    message: format!(
                        "a picture named '{}' is already being generated; wait for it or use another name",
                        request.name
                    ),
                };
                return Err(self.refuse(context, &request.name, refusal));
            }
            let job = self.next_job.fetch_add(1, Ordering::Relaxed);
            jobs.push(JobRecord {
                job,
                name: request.name.clone(),
                started: Instant::now(),
                state: JobState::Running,
                elapsed: None,
                cause: None,
                message: None,
                provenance: None,
                provenance_file: None,
            });
            job
        };
        context.record(&ImageGenEvent::Requested {
            job,
            name: request.name.clone(),
            purpose: request.purpose,
            aspect: request.aspect,
            prompt_chars: request.prompt.chars().count(),
            deadline: self.deadline,
            backend: self.backend.name(),
        });
        let name = request.name.clone();
        let generator = Arc::clone(self);
        let context = context.clone();
        let finished = tokio::spawn(async move { generator.run(&context, job, request).await });
        Ok(Started {
            job,
            name,
            deadline: self.deadline,
            finished,
        })
    }

    async fn run(&self, context: &Context, job: u64, request: ImageRequest) -> JobView {
        let started = Instant::now();
        let outcome = self.make_and_store(context, job, &request).await;
        let elapsed = started.elapsed();
        let backend = self.backend.name();
        let mut jobs = self.jobs();
        let Some(record) = jobs.iter_mut().find(|record| record.job == job) else {
            return JobView {
                job,
                name: request.name,
                state: JobState::Refused,
                elapsed_ms: 0,
                cause: Some(RefusalCause::Failed),
                message: Some("the job record is gone".to_owned()),
                provenance: None,
                provenance_file: None,
            };
        };
        record.elapsed = Some(elapsed);
        match outcome {
            Ok((provenance, provenance_file)) => {
                record.state = JobState::Done;
                record.provenance = Some(provenance);
                record.provenance_file = provenance_file;
            }
            Err(refusal) => {
                record.state = JobState::Refused;
                record.cause = Some(refusal.cause);
                record.message = Some(refusal.message);
                context.record(&ImageGenEvent::Failed {
                    job,
                    name: request.name.clone(),
                    cause: refusal.cause,
                    duration: elapsed,
                    backend,
                });
            }
        }
        record.view()
    }

    async fn make_and_store(
        &self,
        context: &Context,
        job: u64,
        request: &ImageRequest,
    ) -> Result<(Provenance, Option<String>), Refusal> {
        let (width_px, height_px) = request.aspect.requested_size();
        let backend_request = BackendRequest {
            text: request.sent_text(),
            width_px,
            height_px,
        };
        let image = self.generate(job, backend_request).await?;
        check_picture(&image).map_err(|message| Refusal {
            job: Some(job),
            cause: RefusalCause::BadImage,
            message,
        })?;
        let file = format!("{}.{}", request.name, extension_of(&image.mime));
        store(context.media_directory.clone(), &file, &image.bytes)
            .await
            .map_err(|message| Refusal {
                job: Some(job),
                cause: RefusalCause::StoreFailed,
                message,
            })?;
        let provenance = Provenance {
            backend: self.backend.name().to_owned(),
            purpose: request.purpose,
            aspect: request.aspect.label(),
            requested_width_px: width_px,
            requested_height_px: height_px,
            prompt: request.prompt.clone(),
            sent_text: request.sent_text(),
            duration_ms: u64::try_from(image.duration.as_millis()).unwrap_or(u64::MAX),
            deadline_secs: self.deadline.as_secs(),
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
        context.record(&ImageGenEvent::Stored {
            job,
            name: request.name.clone(),
            file,
            bytes: provenance.bytes,
            width_px: image.width_px,
            height_px: image.height_px,
            duration: image.duration,
            backend: self.backend.name(),
            sha256_prefix: image.sha256_hex.chars().take(16).collect(),
        });
        let provenance_file = self.write_provenance(&request.name, &provenance).await;
        Ok((provenance, provenance_file))
    }

    /// Runs the backend in a blocking task. The backend enforces the deadline itself; the host
    /// adds a grace period and then calls the attempt timed out, so a stuck task cannot hold a
    /// job open forever.
    async fn generate(
        &self,
        job: u64,
        backend_request: BackendRequest,
    ) -> Result<GeneratedImage, Refusal> {
        let backend = Arc::clone(&self.backend);
        let deadline = self.deadline;
        let task = tokio::task::spawn_blocking(move || {
            backend.generate_blocking(&backend_request, deadline)
        });
        let limit = deadline.saturating_add(self.grace);
        match tokio::time::timeout(limit, task).await {
            Ok(Ok(Ok(image))) => Ok(image),
            Ok(Ok(Err(error))) => Err(Refusal::of(Some(job), &error)),
            Ok(Err(_)) => Err(Refusal::of(
                Some(job),
                &BackendError::Failed("the backend task stopped unexpectedly".to_owned()),
            )),
            Err(_) => Err(Refusal::of(Some(job), &BackendError::Timeout)),
        }
    }

    /// Writes `<name>.provenance.json` when a directory was given. A failure is reported in the
    /// returned text and never fails the job: the picture is already stored.
    async fn write_provenance(&self, name: &str, provenance: &Provenance) -> Option<String> {
        let directory = self.provenance_directory.clone()?;
        let path = directory.join(format!("{name}.provenance.json"));
        let text = serde_json::to_string_pretty(provenance).ok()?;
        let target = path.clone();
        let written =
            tokio::task::spawn_blocking(move || write_text_atomically(&directory, &target, &text))
                .await
                .unwrap_or_else(|error| Err(std::io::Error::other(error.to_string())));
        match written {
            Ok(()) => Some(path.display().to_string()),
            Err(error) => Some(format!("{} (not written: {error})", path.display())),
        }
    }
}

fn write_text_atomically(directory: &Path, target: &Path, text: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let staged = directory.join(format!(".provenance-{}.tmp", std::process::id()));
    std::fs::write(&staged, text)?;
    std::fs::rename(&staged, target).inspect_err(|_| {
        // The staged file is ours; failure to remove it only leaves disk space used.
        let _ = std::fs::remove_file(&staged);
    })
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
pub(crate) fn check_picture(image: &GeneratedImage) -> Result<(), String> {
    if image.bytes.is_empty() {
        return Err("the backend delivered an empty file".to_owned());
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
        return Err(format!(
            "the delivered bytes are not a {} picture; nothing was stored",
            image.mime
        ));
    }
    if image.width_px == 0 || image.height_px == 0 {
        return Err("the delivered picture has no size; nothing was stored".to_owned());
    }
    Ok(())
}

/// Stores `bytes` as `file_name` in the media library through the staged upload path: the same
/// code `media add` uses, fed the same JSON envelope in pieces.
async fn store(directory: PathBuf, file_name: &str, bytes: &[u8]) -> Result<(), String> {
    let (sender, receiver) = tokio::sync::mpsc::channel::<Bytes>(UPLOAD_QUEUE_PIECES);
    let limits = MediaLimits::DEFAULT;
    let worker = tokio::task::spawn_blocking(move || ingest_upload(receiver, &directory, &limits));
    let quoted = serde_json::to_string(file_name).map_err(|error| error.to_string())?;
    let prefix = format!("{{\"name\":{quoted},\"data_base64\":\"");
    // A send fails once the staging thread has given up (a refusal); its answer says why, so the
    // sending simply stops.
    let mut open = sender.send(Bytes::from(prefix.into_bytes())).await.is_ok();
    for piece in bytes.chunks(UPLOAD_PIECE_BYTES) {
        if !open {
            break;
        }
        let encoded = base64::engine::general_purpose::STANDARD.encode(piece);
        open = sender.send(Bytes::from(encoded.into_bytes())).await.is_ok();
    }
    if open {
        // The result of the last send is read through the worker's answer below.
        let _ = sender.send(Bytes::from_static(b"\"}")).await;
    }
    drop(sender);
    match worker.await {
        Ok(Ok(_added)) => Ok(()),
        Ok(Err(AddFailure::Refused(refusal))) => Err(refusal.message()),
        Ok(Err(AddFailure::File(problem))) => Err(problem.message()),
        Err(error) => Err(format!("the staging thread stopped: {error}")),
    }
}

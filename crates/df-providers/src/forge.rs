//! The owner's local Forge studio (`Game Development Studio`) as one provider of voices
//! (Qwen3-TTS), speech recognition (Nemotron), segmentation (SAM 3.1) and pictures (Qwen Image
//! 2.1).
//!
//! Forge is a loopback-only studio with a durable, sequential render queue. This client speaks its
//! documented HTTP API (the endpoint table of Forge's `docs/STUDIO_GUIDE.md`), the surface Forge's
//! own MCP tools call. MCP is not used: it is a wrapper over the same routes, and it cannot stage
//! raw picture bytes or stream audio, which the raw routes do.
//!
//! - `GET /api/runtime/services` reads which engines run; `POST /api/runtime/engines/<id>/start`
//!   starts exactly one (never start-all), and `POST /api/runtime/models/asr/load` starts
//!   Nemotron, which loads its model as it starts. Both are Forge's own memory-checked paths.
//! - `POST /api/projects/<id>/references` stages raw picture bytes for a render.
//! - `POST /api/runtime/jobs` queues one render; `GET /api/runtime/jobs/<id>` polls it;
//!   `POST /api/runtime/jobs/<id>/cancel` withdraws one no engine has taken yet.
//! - `GET /api/projects/<id>/asset?path=` reads a render's output.
//! - `WS /api/asr/realtime` transcribes PCM16 (`realtime`).
//!
//! The queue's rules hold here: a job is submitted once and never again. A job whose outcome is
//! unknown (`interrupted`) or that waits for a person (`needs_attention`) is an error the caller
//! sees; nothing resubmits or resolves it. A job still waiting when its deadline passes is
//! cancelled (Forge cancels only a job no engine has taken); one that runs is left to finish. The
//! client starts at most one engine per call and never offloads or stops one: Forge's memory
//! admission decides what loads.
//!
//! Every exchange writes the shared `PROVIDER exchange` line (`provider=forge`). The client adds
//! `FORGE` lines of closed keys and times, never text, audio, pictures, paths or Forge's ids:
//!
//! - `FORGE engine engine=<id> action=<started|loading|ready> duration_ms=<n>`
//! - `FORGE job kind=<kind> outcome=<status|deadline> queued_ms=<n> render_ms=<n> polls=<n>`
//! - `FORGE transcribed audio_bytes=<n> ready_ms=<n> duration_ms=<n> outcome=<key>`

mod realtime;

use std::fmt;
use std::time::Duration;

use serde_json::Value;
use tokio::time::Instant;

use crate::http_contract::encode_path_segment;
use crate::native_http::{NativeHttp, NativeResponseProfile};
use crate::{HttpMethod, HttpRequest, NativeHttpCause, NativeHttpResponse, ProviderLimits};

/// Forge's own default address (its `start-server.ps1` and `docs/AGENT_SETUP.md`).
pub const FORGE_DEFAULT_ORIGIN: &str = "http://127.0.0.1:5173";
/// The voice model Forge renders with: Qwen3-TTS 1.7B CustomVoice (Forge's `tts` engine).
pub const FORGE_TTS_MODEL: &str = "qwen3-tts-1.7b-customvoice";
/// The speech recognition model Forge runs: NVIDIA Nemotron Speech Streaming 0.6B, English only.
pub const FORGE_STT_MODEL: &str = "nemotron-speech-streaming-en-0.6b";
/// The nine preset speakers of Forge's Qwen3-TTS CustomVoice checkpoint (`forge_queue_voice`).
pub const FORGE_TTS_SPEAKERS: [&str; 9] = [
    "Ryan", "Aiden", "Vivian", "Serena", "Uncle_Fu", "Dylan", "Eric", "Ono_Anna", "Sohee",
];
/// Most characters of one voice job (Forge's request contract).
pub const FORGE_TTS_MAXIMUM_CHARACTERS: usize = 1000;
/// The sample rate of Forge's voice output (Qwen3-TTS writes 24 kHz mono WAV).
pub const FORGE_TTS_SAMPLE_RATE_HZ: u32 = 24_000;
/// Most matches one segmentation keeps with `select: top` (Forge's request contract).
pub const FORGE_SEGMENTATION_MAXIMUM_INSTANCES: u32 = 50;
/// Image sides Forge accepts are multiples of this many pixels (its request contract).
pub const FORGE_IMAGE_SIDE_STEP_PX: u32 = 32;
/// The longest image side Forge accepts, in pixels (its request contract).
pub const FORGE_IMAGE_MAXIMUM_SIDE_PX: u32 = 2752;

/// How often a queued job is read while it runs: a loopback read costs about a millisecond, and a
/// voiced line should reach the table as soon as it is written.
const JOB_POLL_INTERVAL: Duration = Duration::from_millis(250);
/// How often a starting engine's state is read.
const ENGINE_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// The longest one exchange may take: a deadline of the caller is usually shorter.
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// The largest request sent, in bytes: a staged picture (Forge's reference bound is lower).
const MAXIMUM_REQUEST_BYTES: usize = 128 * 1024 * 1024;
/// The largest answer kept, in bytes: a 2752-pixel PNG with room to spare.
const MAXIMUM_RESPONSE_BYTES: usize = 256 * 1024 * 1024;
/// A project id is a UUID in its 36-character text form (Forge's route contract).
const PROJECT_ID_BYTES: usize = 36;
/// The engine-start answers Forge gives when the engine cannot start (`service-preparation.js`).
const START_FAILURES: [&str; 4] = ["missing", "failed", "port_in_use", "recovery_unresolved"];

/// Why Forge gave no answer. It never carries text, audio, a picture, a path or a Forge id.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForgeCause {
    /// The origin, the project or the limits are not usable; nothing was sent.
    InvalidConfiguration,
    /// The request broke a bound of Forge's contract; nothing was sent.
    InvalidRequest,
    /// The exchange failed; the cause says how (a refused connection is `Unknown`).
    Transport(NativeHttpCause),
    /// Forge refused the request (a 4xx answer).
    Rejected,
    /// Forge failed the request (a 5xx answer).
    Failed,
    /// The answer did not have the documented shape.
    Malformed,
    /// The engine could not start, or did not become ready in time.
    EngineUnavailable,
    /// The render failed with the engine's own answer.
    JobFailed,
    /// The render was cancelled in Forge.
    JobCancelled,
    /// The render's outcome is unknown; Forge's Recovery panel resolves it, never this client.
    JobInterrupted,
    /// The render waits for a person in Forge (a missing model, a failed start).
    JobNeedsAttention,
    /// The caller's deadline passed while the render or the session was under way.
    Deadline,
    /// The realtime session failed: an error event, a closed socket or an invalid event.
    Session,
}

impl ForgeCause {
    /// The closed key audit lines carry.
    pub const fn key(self) -> &'static str {
        match self {
            Self::InvalidConfiguration => "invalid_configuration",
            Self::InvalidRequest => "invalid_request",
            Self::Transport(_) => "transport",
            Self::Rejected => "rejected",
            Self::Failed => "failed",
            Self::Malformed => "malformed",
            Self::EngineUnavailable => "engine_unavailable",
            Self::JobFailed => "job_failed",
            Self::JobCancelled => "job_cancelled",
            Self::JobInterrupted => "job_interrupted",
            Self::JobNeedsAttention => "job_needs_attention",
            Self::Deadline => "deadline",
            Self::Session => "session",
        }
    }
}

/// Why a Forge call gave no answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ForgeError {
    pub cause: ForgeCause,
}

impl ForgeError {
    const fn new(cause: ForgeCause) -> Self {
        Self { cause }
    }
}

impl fmt::Display for ForgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.cause {
            ForgeCause::InvalidConfiguration => {
                formatter.write_str("the forge origin or project is not usable")
            }
            ForgeCause::InvalidRequest => formatter.write_str("the forge request breaks a bound"),
            ForgeCause::Transport(cause) => write!(formatter, "the forge call failed: {cause}"),
            ForgeCause::Rejected => formatter.write_str("forge refused the request"),
            ForgeCause::Failed => formatter.write_str("forge failed the request"),
            ForgeCause::Malformed => formatter.write_str("the forge answer had the wrong shape"),
            ForgeCause::EngineUnavailable => {
                formatter.write_str("the forge engine could not start or did not become ready")
            }
            ForgeCause::JobFailed => formatter.write_str("the forge render failed"),
            ForgeCause::JobCancelled => formatter.write_str("the forge render was cancelled"),
            ForgeCause::JobInterrupted => formatter.write_str(
                "the forge render was interrupted; resolve it in forge, it is never resubmitted",
            ),
            ForgeCause::JobNeedsAttention => {
                formatter.write_str("the forge render needs attention in forge")
            }
            ForgeCause::Deadline => formatter.write_str("the forge call ran out of time"),
            ForgeCause::Session => formatter.write_str("the forge speech session failed"),
        }
    }
}

impl std::error::Error for ForgeError {}

/// One Forge engine the client may start.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForgeEngine {
    Tts,
    Asr,
    Segmentation,
    Images,
}

impl ForgeEngine {
    /// Forge's engine id.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Tts => "tts",
            Self::Asr => "asr",
            Self::Segmentation => "segmentation",
            Self::Images => "images",
        }
    }
}

/// One kind of render the client queues.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForgeJobKind {
    Voices,
    Segmentation,
    Images,
}

impl ForgeJobKind {
    /// Forge's job type.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Voices => "voices",
            Self::Segmentation => "segmentation",
            Self::Images => "images",
        }
    }

    /// The engine that renders this kind.
    pub const fn engine(self) -> ForgeEngine {
        match self {
            Self::Voices => ForgeEngine::Tts,
            Self::Segmentation => ForgeEngine::Segmentation,
            Self::Images => ForgeEngine::Images,
        }
    }
}

/// The media type of an output the client reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForgeMedia {
    Wav,
    Png,
}

impl ForgeMedia {
    const fn content_type(self) -> &'static str {
        match self {
            Self::Wav => "audio/wav",
            Self::Png => "image/png",
        }
    }
}

/// Where Forge runs and which of its projects the renders go to. It has no secrets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForgeConfig {
    origin: String,
    project: String,
}

impl ForgeConfig {
    /// `origin` is a plain loopback `http://` origin (Forge serves only those) and `project` the
    /// id of a Forge project.
    pub fn new(origin: &str, project: &str) -> Result<Self, ForgeError> {
        let origin = origin.strip_suffix('/').unwrap_or(origin);
        if !is_loopback_origin(origin) || !is_project_id(project) {
            return Err(ForgeError::new(ForgeCause::InvalidConfiguration));
        }
        Ok(Self {
            origin: origin.to_owned(),
            project: project.to_owned(),
        })
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn project(&self) -> &str {
        &self.project
    }
}

/// `http://<127.0.0.1|localhost|[::1]>:<port>` with nothing after it.
fn is_loopback_origin(origin: &str) -> bool {
    let Some(authority) = origin.strip_prefix("http://") else {
        return false;
    };
    let Some((host, port)) = authority.rsplit_once(':') else {
        return false;
    };
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
        && port.parse::<u16>().is_ok_and(|port| port != 0)
        && port.bytes().all(|byte| byte.is_ascii_digit())
}

/// A UUID in lowercase or uppercase hex, hyphenated 8-4-4-4-12.
fn is_project_id(project: &str) -> bool {
    project.len() == PROJECT_ID_BYTES
        && project.bytes().enumerate().all(|(at, byte)| match at {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

/// A staged picture, by the id Forge gave it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForgeReference(String);

impl ForgeReference {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a segmentation found: how many matches, how many the output kept, and their scores, best
/// first (at most 20, as Forge reports them).
#[derive(Clone, Debug, PartialEq)]
pub struct ForgeMatches {
    pub found: u32,
    pub used: u32,
    pub scores: Vec<f32>,
}

/// A finished render: its output in the project, and how long it waited and ran.
#[derive(Clone, Debug, PartialEq)]
pub struct ForgeRender {
    /// The output's project-relative path.
    pub output: String,
    /// What the segmentation found, for a segmentation.
    pub matches: Option<ForgeMatches>,
    /// From the submit to the first poll that saw the render run.
    pub queued: Duration,
    /// From then to the poll that saw it finished.
    pub rendering: Duration,
}

/// The status of a Forge job (`runtime/jobs.js`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JobStatus {
    Queued,
    WaitingEngine,
    WaitingMemory,
    Preparing,
    NeedsAttention,
    Submitting,
    Running,
    Finalizing,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl JobStatus {
    const ALL: [Self; 12] = [
        Self::Queued,
        Self::WaitingEngine,
        Self::WaitingMemory,
        Self::Preparing,
        Self::NeedsAttention,
        Self::Submitting,
        Self::Running,
        Self::Finalizing,
        Self::Completed,
        Self::Failed,
        Self::Cancelled,
        Self::Interrupted,
    ];

    const fn key(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::WaitingEngine => "waiting_engine",
            Self::WaitingMemory => "waiting_memory",
            Self::Preparing => "preparing",
            Self::NeedsAttention => "needs_attention",
            Self::Submitting => "submitting",
            Self::Running => "running",
            Self::Finalizing => "finalizing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.key() == key)
    }

    /// No engine has taken the job yet, so Forge can still cancel it.
    const fn is_waiting(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::WaitingEngine | Self::WaitingMemory | Self::Preparing
        )
    }
}

/// The fields of a job (`{"job": {...}}`) this client reads.
struct JobRecord {
    id: String,
    status: JobStatus,
    output: Option<String>,
    matches: Option<ForgeMatches>,
}

impl JobRecord {
    fn read(reply: &Value) -> Result<Self, ForgeError> {
        let malformed = ForgeError::new(ForgeCause::Malformed);
        let job = &reply["job"];
        let status = job["status"]
            .as_str()
            .and_then(JobStatus::from_key)
            .ok_or(malformed)?;
        let result = &job["details"]["result"];
        let matches = if result.is_object() {
            Some(read_matches(result).ok_or(malformed)?)
        } else {
            None
        };
        Ok(Self {
            id: job["id"].as_str().ok_or(malformed)?.to_owned(),
            status,
            output: job["output"].as_str().map(str::to_owned),
            matches,
        })
    }
}

/// A segmentation's `details.result`: `found`, `used` and `scores`.
fn read_matches(result: &Value) -> Option<ForgeMatches> {
    let count = |field: &str| u32::try_from(result[field].as_u64()?).ok();
    let scores = result["scores"]
        .as_array()?
        .iter()
        .map(|score| score.as_f64().map(|score| score as f32))
        .collect::<Option<Vec<f32>>>()?;
    Some(ForgeMatches {
        found: count("found")?,
        used: count("used")?,
        scores,
    })
}

/// An engine as `GET /api/runtime/services` describes it.
struct ServiceState {
    reachable: bool,
    model_loaded: bool,
}

impl ServiceState {
    fn read(reply: &Value, engine: ForgeEngine) -> Result<Self, ForgeError> {
        let service = reply["services"]
            .as_array()
            .and_then(|services| {
                services
                    .iter()
                    .find(|service| service["id"].as_str() == Some(engine.key()))
            })
            .ok_or(ForgeError::new(ForgeCause::Malformed))?;
        Ok(Self {
            reachable: service["status"].as_str() == Some("reachable"),
            model_loaded: service["modelLoaded"].as_bool() == Some(true),
        })
    }
}

/// Which of Forge's two write kinds a route is (its `X-Forge-Request` header).
#[derive(Clone, Copy)]
enum RouteKind {
    Project,
    Runtime,
}

impl RouteKind {
    const fn key(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Runtime => "runtime",
        }
    }
}

/// The Forge client. It holds no secret and no cache; each call is bounded by the caller's
/// deadline.
pub struct ForgeClient {
    http: NativeHttp,
    config: ForgeConfig,
}

impl fmt::Debug for ForgeClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ForgeClient")
            .field("origin", &self.config.origin)
            .finish_non_exhaustive()
    }
}

impl ForgeClient {
    pub fn new(config: ForgeConfig) -> Result<Self, ForgeError> {
        let limits = ProviderLimits::new(
            EXCHANGE_TIMEOUT,
            MAXIMUM_REQUEST_BYTES,
            MAXIMUM_RESPONSE_BYTES,
        )
        .build();
        // Plain HTTP: `ForgeConfig` already holds the endpoint to a loopback origin.
        let http = NativeHttp::new(limits, true)
            .map_err(|_| ForgeError::new(ForgeCause::InvalidConfiguration))?;
        Ok(Self { http, config })
    }

    pub fn config(&self) -> &ForgeConfig {
        &self.config
    }

    /// Makes sure `engine` runs, starting it when it does not: one engine, never all. Nemotron
    /// loads its model as it starts, so for it this also waits until the model is loaded. The
    /// other engines load their model when the queue first needs it.
    pub async fn prepare(&self, engine: ForgeEngine, until: Instant) -> Result<(), ForgeError> {
        let started = Instant::now();
        let service = self.service(engine, until).await?;
        let ready = |service: &ServiceState| match engine {
            ForgeEngine::Asr => service.reachable && service.model_loaded,
            _ => service.reachable,
        };
        if ready(&service) {
            return Ok(());
        }
        let action = match engine {
            ForgeEngine::Asr => {
                self.post_json(
                    "/api/runtime/models/asr/load",
                    RouteKind::Runtime,
                    &serde_json::json!({}),
                    "forge_load_engine",
                    until,
                )
                .await?;
                "loading"
            }
            _ => {
                let path = format!("/api/runtime/engines/{}/start", engine.key());
                let reply = self
                    .post_json(
                        &path,
                        RouteKind::Runtime,
                        &serde_json::json!({}),
                        "forge_start_engine",
                        until,
                    )
                    .await?;
                let status = reply["status"]
                    .as_str()
                    .ok_or(ForgeError::new(ForgeCause::Malformed))?;
                if START_FAILURES.contains(&status) {
                    return Err(ForgeError::new(ForgeCause::EngineUnavailable));
                }
                "started"
            }
        };
        record_engine(engine, action, started.elapsed());
        loop {
            if self
                .service(engine, until)
                .await
                .is_ok_and(|service| ready(&service))
            {
                record_engine(engine, "ready", started.elapsed());
                return Ok(());
            }
            if Instant::now() + ENGINE_POLL_INTERVAL >= until {
                return Err(ForgeError::new(ForgeCause::EngineUnavailable));
            }
            tokio::time::sleep(ENGINE_POLL_INTERVAL).await;
        }
    }

    async fn service(
        &self,
        engine: ForgeEngine,
        until: Instant,
    ) -> Result<ServiceState, ForgeError> {
        let reply = self
            .get_json("/api/runtime/services", "forge_services", until)
            .await?;
        ServiceState::read(&reply, engine)
    }

    /// Stages `picture` (PNG, JPEG or WebP bytes) in the project for one or more renders.
    pub async fn stage_reference(
        &self,
        picture: &[u8],
        until: Instant,
    ) -> Result<ForgeReference, ForgeError> {
        let content_type =
            picture_type(picture).ok_or(ForgeError::new(ForgeCause::InvalidRequest))?;
        let path = format!("/api/projects/{}/references", self.config.project);
        let call = Call {
            method: HttpMethod::Post,
            path: &path,
            kind: RouteKind::Project,
            content_type: Some(content_type),
            body: picture.to_vec(),
            profile: NativeResponseProfile::Json,
            operation: "forge_stage_reference",
        };
        let response = self.send(call, until).await?;
        let reply = json_answer(&response)?;
        let id = reply["reference"]["id"]
            .as_str()
            .ok_or(ForgeError::new(ForgeCause::Malformed))?;
        Ok(ForgeReference(id.to_owned()))
    }

    /// Queues one render of `kind` with `request` (Forge's request object for that kind), waits
    /// for it and answers where its output is. It is submitted once: a failure after the submit
    /// is the caller's, and nothing here resubmits.
    pub async fn render(
        &self,
        kind: ForgeJobKind,
        request: Value,
        until: Instant,
    ) -> Result<ForgeRender, ForgeError> {
        let body = serde_json::json!({
            "type": kind.key(),
            "projectId": self.config.project,
            "request": request,
        });
        let submitted = Instant::now();
        let reply = self
            .post_json(
                "/api/runtime/jobs",
                RouteKind::Runtime,
                &body,
                "forge_queue_job",
                until,
            )
            .await?;
        let job = JobRecord::read(&reply)?;
        let mut watch = JobWatch::new(kind, submitted);
        let outcome = self.wait(job, &mut watch, until).await;
        watch.record(&outcome);
        outcome
    }

    async fn wait(
        &self,
        mut job: JobRecord,
        watch: &mut JobWatch,
        until: Instant,
    ) -> Result<ForgeRender, ForgeError> {
        let path = format!("/api/runtime/jobs/{}", encode_path_segment(&job.id));
        loop {
            watch.observe(job.status);
            match job.status {
                JobStatus::Completed => return watch.finished(job),
                JobStatus::Failed => return Err(ForgeError::new(ForgeCause::JobFailed)),
                JobStatus::Cancelled => return Err(ForgeError::new(ForgeCause::JobCancelled)),
                JobStatus::Interrupted => {
                    return Err(ForgeError::new(ForgeCause::JobInterrupted));
                }
                JobStatus::NeedsAttention => {
                    return Err(ForgeError::new(ForgeCause::JobNeedsAttention));
                }
                _ => {}
            }
            if Instant::now() + JOB_POLL_INTERVAL >= until {
                if job.status.is_waiting() {
                    self.withdraw(&job.id).await;
                }
                return Err(ForgeError::new(ForgeCause::Deadline));
            }
            tokio::time::sleep(JOB_POLL_INTERVAL).await;
            watch.polls += 1;
            let reply = self.get_json(&path, "forge_job", until).await?;
            job = JobRecord::read(&reply)?;
        }
    }

    /// Cancels a job no engine has taken, so a caller that stopped waiting leaves no render
    /// behind. Forge refuses (409) a job that started meanwhile; it then finishes on its own.
    async fn withdraw(&self, job: &str) {
        let path = format!("/api/runtime/jobs/{}/cancel", encode_path_segment(job));
        // A short window of its own: the caller's deadline has passed.
        let until = Instant::now() + Duration::from_secs(5);
        let cancelled = self
            .post_json(
                &path,
                RouteKind::Runtime,
                &serde_json::json!({}),
                "forge_cancel_job",
                until,
            )
            .await;
        // The exchange line already records the answer; this line says what it was for.
        df_observe::record_audit_line(
            "FORGE",
            format!(
                "FORGE withdrawn outcome={}",
                cancelled.map_or_else(|error| error.cause.key(), |_| "cancelled")
            ),
        );
    }

    /// The bytes of the project's output at `path`, which must be `media`.
    pub async fn asset(
        &self,
        path: &str,
        media: ForgeMedia,
        until: Instant,
    ) -> Result<Vec<u8>, ForgeError> {
        let route = format!(
            "/api/projects/{}/asset?path={}",
            self.config.project,
            encode_path_segment(path)
        );
        let call = Call {
            method: HttpMethod::Get,
            path: &route,
            kind: RouteKind::Project,
            content_type: None,
            body: Vec::new(),
            profile: NativeResponseProfile::Media(media.content_type()),
            operation: "forge_asset",
        };
        let response = self.send(call, until).await?;
        status_ok(&response)?;
        if response.body.is_empty() {
            return Err(ForgeError::new(ForgeCause::Malformed));
        }
        Ok(response.body)
    }

    /// Transcribes `pcm` (signed 16-bit little-endian mono at `sample_rate_hz`) with Nemotron,
    /// whose engine must be ready ([`Self::prepare`] with [`ForgeEngine::Asr`]).
    pub async fn transcribe(
        &self,
        pcm: &[u8],
        sample_rate_hz: u32,
        until: Instant,
    ) -> Result<String, ForgeError> {
        realtime::transcribe(&self.config.origin, pcm, sample_rate_hz, until).await
    }

    async fn get_json(
        &self,
        path: &str,
        operation: &'static str,
        until: Instant,
    ) -> Result<Value, ForgeError> {
        let call = Call {
            method: HttpMethod::Get,
            path,
            kind: RouteKind::Runtime,
            content_type: None,
            body: Vec::new(),
            profile: NativeResponseProfile::Json,
            operation,
        };
        let response = self.send(call, until).await?;
        json_answer(&response)
    }

    async fn post_json(
        &self,
        path: &str,
        kind: RouteKind,
        body: &Value,
        operation: &'static str,
        until: Instant,
    ) -> Result<Value, ForgeError> {
        let body =
            serde_json::to_vec(body).map_err(|_| ForgeError::new(ForgeCause::InvalidRequest))?;
        let call = Call {
            method: HttpMethod::Post,
            path,
            kind,
            content_type: Some("application/json"),
            body,
            profile: NativeResponseProfile::Json,
            operation,
        };
        let response = self.send(call, until).await?;
        json_answer(&response)
    }

    async fn send(&self, call: Call<'_>, until: Instant) -> Result<NativeHttpResponse, ForgeError> {
        let remaining = until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ForgeError::new(ForgeCause::Deadline));
        }
        let request = HttpRequest::forge_native(
            call.method,
            format!("{}{}", self.config.origin, call.path),
            &self.config.origin,
            call.kind.key(),
            call.content_type,
            call.body,
        )
        .with_operation(call.operation);
        self.http
            .execute_profile(request, remaining, std::future::pending(), call.profile)
            .await
            .map_err(|failure| ForgeError::new(ForgeCause::Transport(failure.cause)))
    }
}

/// One exchange with Forge: the route, its kind, the body and the answer it expects.
struct Call<'a> {
    method: HttpMethod,
    path: &'a str,
    kind: RouteKind,
    content_type: Option<&'static str>,
    body: Vec<u8>,
    profile: NativeResponseProfile,
    /// The label of its `PROVIDER exchange` line.
    operation: &'static str,
}

/// What one job did, for its `FORGE job` line.
struct JobWatch {
    kind: ForgeJobKind,
    submitted: Instant,
    running: Option<Instant>,
    last: Option<JobStatus>,
    polls: u32,
}

impl JobWatch {
    fn new(kind: ForgeJobKind, submitted: Instant) -> Self {
        Self {
            kind,
            submitted,
            running: None,
            last: None,
            polls: 0,
        }
    }

    fn observe(&mut self, status: JobStatus) {
        self.last = Some(status);
        if self.running.is_none() && !status.is_waiting() {
            self.running = Some(Instant::now());
        }
    }

    fn finished(&self, job: JobRecord) -> Result<ForgeRender, ForgeError> {
        let output = job.output.ok_or(ForgeError::new(ForgeCause::Malformed))?;
        let (queued, rendering) = self.spans();
        Ok(ForgeRender {
            output,
            matches: job.matches,
            queued,
            rendering,
        })
    }

    fn spans(&self) -> (Duration, Duration) {
        let now = Instant::now();
        let running = self.running.unwrap_or(now);
        (
            running.saturating_duration_since(self.submitted),
            now.saturating_duration_since(running),
        )
    }

    fn record(&self, outcome: &Result<ForgeRender, ForgeError>) {
        let (queued, rendering) = self.spans();
        let status = match outcome {
            Err(error) if error.cause == ForgeCause::Deadline => "deadline",
            Err(error) if matches!(error.cause, ForgeCause::Transport(_)) => "transport",
            _ => self.last.map_or("unknown", JobStatus::key),
        };
        df_observe::record_audit_line(
            "FORGE",
            format!(
                "FORGE job kind={} outcome={status} queued_ms={} render_ms={} polls={}",
                self.kind.key(),
                df_types::saturating_millis(queued),
                df_types::saturating_millis(rendering),
                self.polls
            ),
        );
    }
}

fn record_engine(engine: ForgeEngine, action: &str, elapsed: Duration) {
    df_observe::record_audit_line(
        "FORGE",
        format!(
            "FORGE engine engine={} action={action} duration_ms={}",
            engine.key(),
            df_types::saturating_millis(elapsed)
        ),
    );
}

/// A 2xx answer, or why not.
fn status_ok(response: &NativeHttpResponse) -> Result<(), ForgeError> {
    match response.status {
        200..=299 => Ok(()),
        400..=499 => Err(ForgeError::new(ForgeCause::Rejected)),
        _ => Err(ForgeError::new(ForgeCause::Failed)),
    }
}

fn json_answer(response: &NativeHttpResponse) -> Result<Value, ForgeError> {
    status_ok(response)?;
    serde_json::from_slice(&response.body).map_err(|_| ForgeError::new(ForgeCause::Malformed))
}

/// The media type of a picture Forge stages, from its signature.
fn picture_type(picture: &[u8]) -> Option<&'static str> {
    if picture.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if picture.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if picture.len() >= 12 && &picture[..4] == b"RIFF" && &picture[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// A preset speaker for `voice`: the speaker it names, or else one picked by a stable hash of the
/// id, so a character keeps one Forge voice while its voice id is another supplier's.
pub fn forge_speaker(voice: &str) -> &'static str {
    if let Some(speaker) = FORGE_TTS_SPEAKERS
        .iter()
        .find(|speaker| speaker.eq_ignore_ascii_case(voice))
    {
        return speaker;
    }
    // FNV-1a: stable across runs and platforms, unlike the standard hasher.
    let hash = voice.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    // A widening cast: the list has nine entries.
    let count = FORGE_TTS_SPEAKERS.len() as u64;
    // The remainder is below the list's length, so it indexes it.
    FORGE_TTS_SPEAKERS[usize::try_from(hash % count).unwrap_or(0)]
}

#[cfg(test)]
mod forge_tests;

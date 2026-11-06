//! What crosses `/dev/media/generate`: the body of a request, the jobs it answers with and the
//! typed reasons it refuses.
//!
//! Every answer is the shared `DevReply` envelope: a POST answers `GenerateReply<JobView>` (the
//! job running on, or its stored picture when the caller waited) and a GET
//! `GenerateReply<Vec<JobView>>`. A refusal is `Rejected` with a `GenerateError`, also for a job
//! the caller waited for that ended without a picture. The route and the command line read and
//! write these types, so a rename breaks the build instead of a running command.

use std::fmt;
use std::path::PathBuf;

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use super::backend::BackendError;
use super::model::{Aspect, GenerateRequestError, Purpose};
use crate::dev_http::{BodyError, DevReply, RouteError};
use crate::locate::IoCause;

/// The answer of a picture generator route.
pub type GenerateReply<T> = DevReply<T, GenerateError>;

/// The body of `POST /dev/media/generate`. Without a purpose the picture is a background; without
/// a shape it has its purpose's default shape. With `wait` the answer comes when the job ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateRequest {
    pub prompt: String,
    pub name: String,
    #[serde(default)]
    pub purpose: Option<Purpose>,
    #[serde(default)]
    pub aspect: Option<Aspect>,
    #[serde(default)]
    pub wait: bool,
}

/// One job as the routes show it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobView {
    pub job: u64,
    pub name: String,
    pub deadline_ms: u64,
    pub elapsed_ms: u64,
    #[serde(flatten)]
    pub state: JobState,
}

/// How a job stands, tagged `"state"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum JobState {
    Running,
    /// The picture is in the media library. `sidecar` is the provenance file, when the host was
    /// told where to write one.
    Stored {
        provenance: Box<Provenance>,
        sidecar: Option<Sidecar>,
    },
    /// The job ended without a stored picture.
    Failed {
        error: GenerateError,
        message: String,
    },
}

/// What happened to the provenance sidecar of a stored picture. A sidecar that could not be
/// written never fails the job: the picture is already stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sidecar {
    Written { path: PathBuf },
    NotWritten { path: PathBuf, cause: IoCause },
}

/// What is known about how a stored picture was made. The prompt is here, in full, because this is
/// the record that goes next to the picture; it is never in an audit line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    pub backend: String,
    pub purpose: Purpose,
    pub aspect: Aspect,
    pub requested_width_px: u32,
    pub requested_height_px: u32,
    pub prompt: String,
    pub sent_text: String,
    pub duration_ms: u64,
    pub deadline_ms: u64,
    pub width_px: u32,
    pub height_px: u32,
    pub mime: String,
    pub bytes: u64,
    pub sha256: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// The file in the media library.
    pub media_file: String,
    /// What the file is, when the host stored it as it came.
    pub note: String,
}

/// Why delivered bytes are not the picture the backend says they are. On the wire it is tagged
/// `"cause"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum PictureCause {
    /// The file is empty.
    Empty,
    /// The bytes do not start like a picture of the type the backend named.
    NotOfType { mime: String },
    /// The backend reported no size.
    NoSize,
}

impl PictureCause {
    /// The serde name of the variant.
    pub fn key(&self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::NotOfType { .. } => "not_of_type",
            Self::NoSize => "no_size",
        }
    }
}

impl fmt::Display for PictureCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("the backend delivered an empty file"),
            Self::NotOfType { mime } => {
                write!(formatter, "the delivered bytes are not a {mime} picture")
            }
            Self::NoSize => formatter.write_str("the delivered picture has no size"),
        }
    }
}

/// Why a picture generator call was refused, or a job ended without a picture. `job` is the job's
/// number once one was started. The typed error or cause inside is flattened into the variant, so
/// its key is always the `"cause"` field: `{"error":"backend","job":3,"cause":"no_login",..}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum GenerateError {
    /// The body is not the JSON the route reads.
    InvalidBody(BodyError),
    /// The request is not one a generation can start from.
    InvalidRequest(GenerateRequestError),
    /// A picture of this name is being generated.
    NameBusy { name: String },
    /// The backend is not ready, or it gave no picture.
    Backend {
        job: Option<u64>,
        #[serde(flatten)]
        backend: BackendError,
    },
    /// The backend delivered bytes that are not a picture; nothing was stored.
    InvalidPicture {
        job: u64,
        #[serde(flatten)]
        cause: PictureCause,
    },
    /// The media library did not take the picture. `detail` is the library's sentence: its
    /// `media_store::AddError` is not a wire type yet.
    StoreFailed { job: u64, detail: String },
    /// A blocking task of the job stopped without an answer (it panicked or the runtime is
    /// stopping).
    Stopped { job: Option<u64> },
}

impl GenerateError {
    /// The serde name of the variant.
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::InvalidRequest(_) => "invalid_request",
            Self::NameBusy { .. } => "name_busy",
            Self::Backend { .. } => "backend",
            Self::InvalidPicture { .. } => "invalid_picture",
            Self::StoreFailed { .. } => "store_failed",
            Self::Stopped { .. } => "stopped",
        }
    }

    /// The key of the typed cause inside, for an audit line.
    pub fn cause_key(&self) -> Option<&'static str> {
        match self {
            Self::InvalidBody(error) => Some(error.cause.key()),
            Self::InvalidRequest(error) => Some(error.key()),
            Self::Backend { backend, .. } => Some(backend.key()),
            Self::InvalidPicture { cause, .. } => Some(cause.key()),
            Self::NameBusy { .. } | Self::StoreFailed { .. } | Self::Stopped { .. } => None,
        }
    }

    /// The job this ended, when one was started.
    pub fn job(&self) -> Option<u64> {
        match self {
            Self::Backend { job, .. } | Self::Stopped { job } => *job,
            Self::InvalidPicture { job, .. } | Self::StoreFailed { job, .. } => Some(*job),
            Self::InvalidBody(_) | Self::InvalidRequest(_) | Self::NameBusy { .. } => None,
        }
    }
}

impl fmt::Display for GenerateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::InvalidRequest(error) => write!(formatter, "{error}"),
            Self::NameBusy { name } => write!(
                formatter,
                concat!(
                    "a picture named '{}' is already being generated; wait for it or use ",
                    "another name"
                ),
                name
            ),
            Self::Backend { backend, .. } => write!(formatter, "{backend}"),
            Self::InvalidPicture { cause, .. } => {
                write!(formatter, "{cause}; nothing was stored")
            }
            Self::StoreFailed { detail, .. } => {
                write!(
                    formatter,
                    "the media library did not take the picture: {detail}"
                )
            }
            Self::Stopped { .. } => {
                formatter.write_str("a task of the generation stopped without an answer")
            }
        }
    }
}

impl std::error::Error for GenerateError {}

impl RouteError for GenerateError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_) | Self::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            Self::NameBusy { .. } => StatusCode::CONFLICT,
            Self::Backend { backend, .. } => backend.status(),
            Self::InvalidPicture { .. } => StatusCode::BAD_GATEWAY,
            Self::StoreFailed { .. } | Self::Stopped { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<BodyError> for GenerateError {
    fn from(error: BodyError) -> Self {
        Self::InvalidBody(error)
    }
}

impl From<GenerateRequestError> for GenerateError {
    fn from(error: GenerateRequestError) -> Self {
        Self::InvalidRequest(error)
    }
}

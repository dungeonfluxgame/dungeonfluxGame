//! The bodies, queries and replies of the locate routes, and why they refuse.
//!
//! Every route answers a `LocateReply<T>`: `{"outcome":"done","value":..}` or
//! `{"outcome":"rejected","error":{"error":"<key>",..},"message":".."}`, where the error is a
//! [`LocateError`] whose fields name the picture, region or value at fault.

use std::fmt;
use std::io::ErrorKind;

use axum::http::StatusCode;
use df_imagery::identity_question::IdentifyError;
use df_imagery::oneshot_segment::{SegmentError, SegmentationPrompt};
use df_scene_layout::SceneLayout;
use serde::{Deserialize, Serialize};

use super::limits::MAXIMUM_PICTURE_NAME_BYTES;
use crate::dev_http::{BodyError, DevReply, RouteError};

/// The answer of a locate route: its value, or why it was refused.
pub type LocateReply<T> = DevReply<T, LocateError>;

/// What a scan looks for in a picture.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanPrompt {
    /// Whole people (the default).
    #[default]
    Person,
    /// Heads only.
    Head,
}

impl ScanPrompt {
    pub const ALL: [Self; 2] = [Self::Person, Self::Head];

    pub fn key(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Head => "head",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|prompt| prompt.key() == key)
    }

    /// The prompt the imagery API asks with.
    pub fn segmentation(self) -> SegmentationPrompt {
        match self {
            Self::Person => SegmentationPrompt::Person,
            Self::Head => SegmentationPrompt::Head,
        }
    }
}

/// A scan request, as the dev CLI sends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocateRequest {
    pub picture: String,
    #[serde(default)]
    pub prompt: Option<ScanPrompt>,
    #[serde(default)]
    pub masks: Option<u32>,
}

/// A name for one region of a stored layout, or `None` to clear it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelRequest {
    pub picture: String,
    /// The region: its number (from 0), its id or its current name.
    pub region: String,
    #[serde(default)]
    pub label: Option<String>,
}

/// The query of `GET /dev/media/locate/show`: the picture to show the stored layout of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowQuery {
    pub picture: String,
}

/// One character to find, as the dev CLI sends it: its name, its look in words, and a library
/// picture of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub portrait: Option<String>,
}

/// Names the regions of a picture's layout by asking which is which character.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentifyRequest {
    pub picture: String,
    pub characters: Vec<CharacterRequest>,
    #[serde(default)]
    pub min_confidence_permille: Option<u16>,
}

/// What a scan stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocatedLayout {
    pub picture: String,
    pub layout_file: String,
    pub duration_ms: u64,
    pub kept_labels: usize,
    pub layout: SceneLayout,
}

/// The stored layout of a picture, after a label change or for `show`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredLayout {
    pub picture: String,
    pub layout_file: String,
    pub layout: SceneLayout,
}

/// What a vision model said about the regions of a picture, and the layout after it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentifiedLayout {
    pub picture: String,
    /// The text profile (backend and model) the question went under.
    pub profile: String,
    pub duration_ms: u64,
    /// Whether the text service replayed an answer it had already qualified (no call).
    pub replayed: bool,
    pub rows: Vec<IdentityRow>,
    pub layout: SceneLayout,
}

/// What the model said about one character and what was done with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityRow {
    pub name: String,
    /// The region (numbered from 0, as `media locate show` numbers them) the model chose.
    pub region: Option<usize>,
    pub confidence_permille: u16,
    pub outcome: IdentityOutcome,
}

/// What was done with one character the model was asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityOutcome {
    /// Its region now carries its name.
    Named,
    /// The model did not place it in the picture.
    NotFound,
    /// The model placed it below the confidence threshold; nothing changed.
    Unsure,
    /// The model gave its region to two characters; nothing changed.
    Conflict,
}

impl IdentityOutcome {
    pub const ALL: [Self; 4] = [Self::Named, Self::NotFound, Self::Unsure, Self::Conflict];

    /// The outcome as the audit line and the reply write it.
    pub fn key(self) -> &'static str {
        match self {
            Self::Named => "named",
            Self::NotFound => "not_found",
            Self::Unsure => "unsure",
            Self::Conflict => "conflict",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|outcome| outcome.key() == key)
    }
}

/// Why a file of the locate feature could not be read or written, as the reply carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IoCause {
    PermissionDenied,
    StorageFull,
    /// The bytes are not what they must be (a file that is a directory included).
    InvalidData,
    Other,
}

impl IoCause {
    pub const ALL: [Self; 4] = [
        Self::PermissionDenied,
        Self::StorageFull,
        Self::InvalidData,
        Self::Other,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::PermissionDenied => "permission_denied",
            Self::StorageFull => "storage_full",
            Self::InvalidData => "invalid_data",
            Self::Other => "other",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }

    pub fn from_kind(kind: ErrorKind) -> Self {
        match kind {
            ErrorKind::PermissionDenied => Self::PermissionDenied,
            ErrorKind::StorageFull => Self::StorageFull,
            ErrorKind::InvalidData | ErrorKind::IsADirectory => Self::InvalidData,
            _ => Self::Other,
        }
    }
}

impl fmt::Display for IoCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PermissionDenied => "permission denied",
            Self::StorageFull => "the disk is full",
            Self::InvalidData => "it is not a readable file",
            Self::Other => "the file system refused it",
        })
    }
}

/// Why the imagery API's segmentation gave no masks (`df_imagery::oneshot_segment::SegmentError`
/// on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanCause {
    Configuration,
    Picture,
    Refused,
    Supplier,
    Timeout,
    Malformed,
}

impl ScanCause {
    pub const ALL: [Self; 6] = [
        Self::Configuration,
        Self::Picture,
        Self::Refused,
        Self::Supplier,
        Self::Timeout,
        Self::Malformed,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::Picture => "picture",
            Self::Refused => "refused",
            Self::Supplier => "supplier",
            Self::Timeout => "timeout",
            Self::Malformed => "malformed",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

impl From<SegmentError> for ScanCause {
    fn from(error: SegmentError) -> Self {
        match error {
            SegmentError::Configuration => Self::Configuration,
            SegmentError::Picture => Self::Picture,
            SegmentError::Refused => Self::Refused,
            SegmentError::Supplier => Self::Supplier,
            SegmentError::Timeout => Self::Timeout,
            SegmentError::Malformed => Self::Malformed,
        }
    }
}

/// Why the imagery API could not prepare an identity question
/// (`df_imagery::identity_question::IdentifyError` on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionCause {
    Request,
    Picture,
    Malformed,
}

impl QuestionCause {
    pub const ALL: [Self; 3] = [Self::Request, Self::Picture, Self::Malformed];

    pub fn key(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Picture => "picture",
            Self::Malformed => "malformed",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

impl From<IdentifyError> for QuestionCause {
    fn from(error: IdentifyError) -> Self {
        match error {
            IdentifyError::Request => Self::Request,
            IdentifyError::Picture => Self::Picture,
            IdentifyError::Malformed => Self::Malformed,
        }
    }
}

/// Why the text service gave no usable answer to an identity question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextServiceCause {
    /// It refused the question.
    Refused,
    /// It did not keep a checked answer.
    NotKept,
    /// Its answer was not one the imagery API accepts; the answer was rejected.
    AnswerRefused,
    /// It replayed an answer the imagery API refuses.
    ReplayRefused,
    /// It settled the question without an answer.
    NoAnswer,
    /// It rejected this exact question's answer before and does not ask again.
    RejectedBefore,
    /// It is still answering this question.
    InFlight,
    /// It sent the question but lost its result; it is never resent.
    Lost,
    Cancelled,
}

impl TextServiceCause {
    pub const ALL: [Self; 9] = [
        Self::Refused,
        Self::NotKept,
        Self::AnswerRefused,
        Self::ReplayRefused,
        Self::NoAnswer,
        Self::RejectedBefore,
        Self::InFlight,
        Self::Lost,
        Self::Cancelled,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Refused => "refused",
            Self::NotKept => "not_kept",
            Self::AnswerRefused => "answer_refused",
            Self::ReplayRefused => "replay_refused",
            Self::NoAnswer => "no_answer",
            Self::RejectedBefore => "rejected_before",
            Self::InFlight => "in_flight",
            Self::Lost => "lost",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

impl fmt::Display for TextServiceCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Refused => "refused the question",
            Self::NotKept => "did not keep the answer",
            Self::AnswerRefused => "gave an answer the imagery API refused; it was rejected",
            Self::ReplayRefused => "replayed an answer the imagery API refuses",
            Self::NoAnswer => "gave no answer",
            Self::RejectedBefore => concat!(
                "rejected this exact question's answer before and does not ask again; ",
                "change the characters or rescan"
            ),
            Self::InFlight => "is still answering this question; try again shortly",
            Self::Lost => "sent this question but its result was lost; it is never resent",
            Self::Cancelled => "cancelled the question",
        })
    }
}

/// Why a locate request did not give a layout. Each variant names the value at fault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum LocateError {
    /// The request body is not the JSON the route reads.
    InvalidBody(BodyError),
    /// The query is not the one the route reads; `detail` is the reader's sentence.
    InvalidQuery {
        detail: String,
    },
    /// Not a picture of the media library: a plain file name with a picture extension.
    InvalidPictureName {
        name: String,
    },
    /// A mask count outside 1 to the supplier's bound.
    InvalidMaskCount {
        masks: u32,
    },
    /// A region label that is not a character name.
    InvalidLabel {
        label: String,
    },
    /// A confidence threshold above 1000 thousandths.
    InvalidMinimumConfidence {
        min_confidence_permille: u16,
    },
    /// An identification that names no character.
    NoCharacters,
    /// `region` is not a region of the picture's layout, which has `region_count` regions named
    /// `names`.
    UnknownRegion {
        region: String,
        region_count: usize,
        names: Vec<String>,
    },
    PictureMissing {
        picture: String,
    },
    PictureUnreadable {
        picture: String,
        cause: IoCause,
    },
    /// The picture has no stored layout yet.
    NoLayout {
        picture: String,
    },
    LayoutUnreadable {
        picture: String,
        cause: IoCause,
    },
    /// The stored layout file does not read as a layout; `detail` is the reader's sentence.
    LayoutInvalid {
        picture: String,
        detail: String,
    },
    LayoutUnwritable {
        picture: String,
        cause: IoCause,
    },
    /// The stored layout was made from another version of the picture.
    StaleLayout {
        picture: String,
    },
    /// The layout changed while the identity question was asked.
    LayoutChanged {
        picture: String,
    },
    /// No fal key is in the host's environment.
    FalKeyMissing,
    ScanFailed {
        cause: ScanCause,
    },
    QuestionInvalid {
        cause: QuestionCause,
    },
    /// The text service is not configured; `missing` says what to set.
    TextServiceMissing {
        missing: String,
    },
    /// The text service gave no usable answer; `detail` is its own account, when it gave one.
    TextService {
        cause: TextServiceCause,
        detail: Option<String>,
    },
    /// The text-service operation key built from the question is not valid.
    InvalidOperationKey,
    /// The blocking task stopped without an answer (it panicked or the runtime is stopping).
    Stopped,
}

impl LocateError {
    /// The error's serde tag, for the audit line.
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::InvalidQuery { .. } => "invalid_query",
            Self::InvalidPictureName { .. } => "invalid_picture_name",
            Self::InvalidMaskCount { .. } => "invalid_mask_count",
            Self::InvalidLabel { .. } => "invalid_label",
            Self::InvalidMinimumConfidence { .. } => "invalid_minimum_confidence",
            Self::NoCharacters => "no_characters",
            Self::UnknownRegion { .. } => "unknown_region",
            Self::PictureMissing { .. } => "picture_missing",
            Self::PictureUnreadable { .. } => "picture_unreadable",
            Self::NoLayout { .. } => "no_layout",
            Self::LayoutUnreadable { .. } => "layout_unreadable",
            Self::LayoutInvalid { .. } => "layout_invalid",
            Self::LayoutUnwritable { .. } => "layout_unwritable",
            Self::StaleLayout { .. } => "stale_layout",
            Self::LayoutChanged { .. } => "layout_changed",
            Self::FalKeyMissing => "fal_key_missing",
            Self::ScanFailed { .. } => "scan_failed",
            Self::QuestionInvalid { .. } => "question_invalid",
            Self::TextServiceMissing { .. } => "text_service_missing",
            Self::TextService { .. } => "text_service",
            Self::InvalidOperationKey => "invalid_operation_key",
            Self::Stopped => "stopped",
        }
    }

    /// A file of `picture` that `error` kept from being read.
    pub(super) fn picture_unreadable(picture: &str, error: &std::io::Error) -> Self {
        Self::PictureUnreadable {
            picture: picture.to_owned(),
            cause: IoCause::from_kind(error.kind()),
        }
    }
}

impl fmt::Display for LocateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::InvalidQuery { detail } => write!(formatter, "the query is not valid: {detail}"),
            Self::InvalidPictureName { name } => write!(
                formatter,
                concat!(
                    "{:?} is not a picture of the media library (a file name of letters, digits, ",
                    "'_', '-' and '.' ending in .png, .webp, .jpg or .jpeg, at most {} bytes)"
                ),
                name, MAXIMUM_PICTURE_NAME_BYTES
            ),
            Self::InvalidMaskCount { masks } => write!(
                formatter,
                "masks={masks} is out of range: 1 to {}",
                df_imagery::oneshot_segment::MAXIMUM_MASKS
            ),
            Self::InvalidLabel { label } => write!(
                formatter,
                "label {label:?} is not a character name: 1 to {} letters, digits, '-' or '_'",
                df_scene_layout::MAXIMUM_NAME_CHARS
            ),
            Self::InvalidMinimumConfidence {
                min_confidence_permille,
            } => write!(
                formatter,
                "min={min_confidence_permille} is out of range: 0 to 1000 thousandths"
            ),
            Self::NoCharacters => formatter.write_str("name at least one character to look for"),
            Self::UnknownRegion {
                region,
                region_count,
                names,
            } => write!(
                formatter,
                concat!(
                    "{:?} is not a region of the layout, which has {} ",
                    "(a number from 0, an id such as r1a2b3c4d, or a name: {})"
                ),
                region,
                region_count,
                names.join(", ")
            ),
            Self::PictureMissing { picture } => {
                write!(formatter, "{picture} is not in the media library")
            }
            Self::PictureUnreadable { picture, cause } => {
                write!(formatter, "{picture} could not be read: {cause}")
            }
            Self::NoLayout { picture } => write!(
                formatter,
                "{picture} has no layout yet; scan it first with: media locate {picture}"
            ),
            Self::LayoutUnreadable { picture, cause } => write!(
                formatter,
                "{} could not be read: {cause}",
                super::layout_file_name(picture)
            ),
            Self::LayoutInvalid { picture, detail } => write!(
                formatter,
                "{} is not a layout: {detail}",
                super::layout_file_name(picture)
            ),
            Self::LayoutUnwritable { picture, cause } => write!(
                formatter,
                "{} could not be written: {cause}",
                super::layout_file_name(picture)
            ),
            Self::StaleLayout { picture } => write!(
                formatter,
                concat!(
                    "the layout of {0} was made from another version of the picture; ",
                    "scan it again with: media locate {0}"
                ),
                picture
            ),
            Self::LayoutChanged { picture } => write!(
                formatter,
                "the layout of {picture} changed while the question was asked; ask again"
            ),
            Self::FalKeyMissing => formatter.write_str(concat!(
                "no fal key in the host's environment: ",
                "set DF_FAL_KEY (or FAL_KEY) before `df-table serve`"
            )),
            Self::ScanFailed { cause } => write!(
                formatter,
                "the imagery API's segmentation did not give masks ({})",
                cause.key()
            ),
            Self::QuestionInvalid { cause } => write!(
                formatter,
                "the identity question could not be prepared ({})",
                cause.key()
            ),
            Self::TextServiceMissing { missing } => formatter.write_str(missing),
            Self::TextService {
                cause,
                detail: None,
            } => write!(formatter, "the text service {cause}"),
            Self::TextService {
                cause,
                detail: Some(detail),
            } => write!(formatter, "the text service {cause} ({detail})"),
            Self::InvalidOperationKey => formatter.write_str("the operation key is not valid"),
            Self::Stopped => formatter.write_str("the locate task stopped without an answer"),
        }
    }
}

impl std::error::Error for LocateError {}

impl From<BodyError> for LocateError {
    fn from(error: BodyError) -> Self {
        Self::InvalidBody(error)
    }
}

impl RouteError for LocateError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_)
            | Self::InvalidQuery { .. }
            | Self::InvalidPictureName { .. }
            | Self::InvalidMaskCount { .. }
            | Self::InvalidLabel { .. }
            | Self::InvalidMinimumConfidence { .. }
            | Self::NoCharacters
            | Self::UnknownRegion { .. }
            | Self::QuestionInvalid { .. } => StatusCode::BAD_REQUEST,
            Self::PictureMissing { .. } | Self::NoLayout { .. } => StatusCode::NOT_FOUND,
            Self::StaleLayout { .. } | Self::LayoutChanged { .. } => StatusCode::CONFLICT,
            Self::FalKeyMissing | Self::TextServiceMissing { .. } => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            Self::ScanFailed { .. } | Self::TextService { .. } => StatusCode::BAD_GATEWAY,
            Self::PictureUnreadable { .. }
            | Self::LayoutUnreadable { .. }
            | Self::LayoutInvalid { .. }
            | Self::LayoutUnwritable { .. }
            | Self::InvalidOperationKey
            | Self::Stopped => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

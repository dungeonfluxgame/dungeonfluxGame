//! The typed error of the look studio, and the bodies and replies of its dev routes. The phone's
//! bodies and replies are `df_playground::look::wire`; a dev body wraps one in `LookAt` with the
//! seat it is for.

use std::collections::BTreeMap;
use std::fmt;

use axum::http::StatusCode;
use df_playground::look::wire::{BodyFrame, LookErrorKind, VersionState};
use serde::{Deserialize, Serialize};

use crate::dev_http::{BodyError, DevReply, QueryError, RouteError};
use crate::imagegen::GenerateError;
use crate::io_cause::IoCause;
use crate::seat::request::SeatError;
use crate::text::{ModelCause, ServiceReason, TextServiceCause};

/// The answer of a look studio route.
pub type LookReply<T> = DevReply<T, LookError>;

/// What is wrong with the studio file (`catalog`), found when it is read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "problem", rename_all = "snake_case")]
pub enum StudioCause {
    /// A species stands on a frame the file gives no figure for.
    MissingFrame {
        frame: BodyFrame,
    },
    DuplicateControl {
        control: String,
    },
    /// A species names an extra control the file does not have as an extra.
    UnknownExtra {
        species: String,
        control: String,
    },
    /// A control has no options or words, or a default that is not one of them.
    InvalidControl {
        control: String,
    },
    /// A species' height range runs backwards.
    InvalidHeight {
        species: String,
    },
}

/// What is wrong with a draft the phone sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "problem", rename_all = "snake_case")]
pub enum DraftCause {
    /// No control of this hero has this id.
    UnknownControl { control: String },
    /// The value is not one the control takes (out of range, or not one of its options).
    InvalidValue { control: String },
    /// A detail has no words.
    EmptyDetail,
    /// A detail is neither one Jev read for this hero nor one of the studio's rolled details.
    UnreadDetail,
}

/// What is wrong with Jev's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "problem", rename_all = "snake_case")]
pub enum AnswerCause {
    /// The answer is not the schema's shape.
    Malformed {
        reason: String,
    },
    /// A detail names a kind or a region the closed lists do not have.
    UnknownKey {
        key: String,
    },
    /// A detail has no words.
    EmptyDetail,
    UnknownCategory {
        key: String,
    },
}

impl fmt::Display for StudioCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingFrame { frame } => {
                write!(
                    formatter,
                    "no figure is given for the {} frame",
                    frame.key()
                )
            }
            Self::DuplicateControl { control } => {
                write!(formatter, "the control {control:?} is listed twice")
            }
            Self::UnknownExtra { species, control } => write!(
                formatter,
                "{species} names {control:?}, which is not an extra control"
            ),
            Self::InvalidControl { control } => write!(
                formatter,
                "the control {control:?} has no values or a default that is not one of them"
            ),
            Self::InvalidHeight { species } => {
                write!(formatter, "the height range of {species} runs backwards")
            }
        }
    }
}

impl fmt::Display for DraftCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownControl { control } => {
                write!(formatter, "this hero has no control {control:?}")
            }
            Self::InvalidValue { control } => {
                write!(formatter, "the value of {control:?} is not one it takes")
            }
            Self::EmptyDetail => formatter.write_str("a detail has no words"),
            Self::UnreadDetail => formatter
                .write_str("a detail was not read from your words; read your details again"),
        }
    }
}

impl fmt::Display for AnswerCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed { reason } => write!(formatter, "malformed answer: {reason}"),
            Self::UnknownKey { key } => write!(formatter, "{key:?} is not on the closed lists"),
            Self::EmptyDetail => formatter.write_str("a detail has no words"),
            Self::UnknownCategory { key } => write!(formatter, "{key:?} is not a category"),
        }
    }
}

/// Why Jev held a look back from the image service (REF-039): it is not supported, or it could not
/// be told safely. Never retried with other words or another supplier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeldCause {
    Unsupported,
    Uncertain,
}

impl HeldCause {
    pub fn key(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::Uncertain => "uncertain",
        }
    }
}

/// Why a look studio route refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum LookError {
    InvalidBody(BodyError),
    /// The query is not the one the route reads.
    InvalidQuery(QueryError),
    /// The seat plays no hero yet.
    NoHero {
        seat: u64,
    },
    /// The studio file has no species of this key.
    UnknownSpecies {
        species: String,
    },
    /// The studio file has no class of this key.
    UnknownClass {
        class: String,
    },
    StudioUnreadable {
        cause: IoCause,
    },
    StudioInvalid {
        cause: BodyError,
    },
    StudioRejected {
        cause: StudioCause,
    },
    PosesUnreadable {
        cause: IoCause,
    },
    PosesInvalid {
        cause: BodyError,
    },
    InvalidDraft {
        cause: DraftCause,
    },
    /// The owner has not chosen this frame's base pose (or its three-view sheet) yet.
    PoseNotChosen {
        frame: BodyFrame,
    },
    /// The details box holds no words.
    EmptyWords,
    /// The text service is not set up; `missing` says what to set.
    TextServiceMissing {
        missing: String,
    },
    InvalidModel {
        cause: ModelCause,
    },
    TextService {
        cause: TextServiceCause,
        reason: Option<ServiceReason>,
    },
    /// Jev's answer did not read (`replayed` when it was a replay).
    AnswerRefused {
        replayed: bool,
        cause: AnswerCause,
    },
    InvalidOperationKey,
    /// Jev held the look back from the image service (REF-039).
    Held {
        cause: HeldCause,
    },
    /// The phone's kept version is not the one the studio keeps now.
    KeptChanged {
        kept: Option<u64>,
    },
    /// The draft is the kept version's: there is nothing to change.
    NothingChanged,
    UnknownVersion {
        version: u64,
    },
    /// The version cannot be kept as it stands (pending, failed or stale).
    NotKeepable {
        version: u64,
        state: VersionState,
    },
    /// The look is locked in for `hero`; start again for a new one.
    Locked {
        hero: u64,
    },
    /// The look is being locked in.
    LockBusy,
    /// The image service refused the picture before a job began.
    Generate {
        cause: GenerateError,
    },
    /// The name is not a picture of the media library.
    InvalidPicture {
        picture: String,
    },
    /// The media library has no picture of this name.
    UnknownPicture {
        picture: String,
    },
    /// Pose candidates were asked for none.
    InvalidCount,
    /// A studio or poses file, or a picture to copy, could not be read or written.
    Storage {
        cause: IoCause,
    },
    /// The media library did not take a chosen pose. `detail` is the library's sentence, as
    /// `GenerateError::StoreFailed` carries it.
    StoreFailed {
        detail: String,
    },
    /// The look studio was not started with the host's services.
    StudioClosed,
    /// A task stopped without an answer.
    Stopped,
    /// The seat, its key or its hero refused, in the seat routes' own shape (`{"error":
    /// "unknown_key"}`), so a phone reads a lost key alike on every route.
    #[serde(untagged)]
    Seat(SeatError),
}

impl LookError {
    /// The refusal as the phone reads it: one of the shared `LookErrorKind`s.
    pub fn kind(&self) -> LookErrorKind {
        match self {
            Self::InvalidBody(_) => LookErrorKind::InvalidBody,
            Self::InvalidQuery(_) => LookErrorKind::InvalidQuery,
            Self::NoHero { .. } => LookErrorKind::NoHero,
            Self::UnknownSpecies { .. } => LookErrorKind::UnknownSpecies,
            Self::UnknownClass { .. } => LookErrorKind::UnknownClass,
            Self::StudioUnreadable { .. }
            | Self::StudioInvalid { .. }
            | Self::StudioRejected { .. } => LookErrorKind::StudioUnavailable,
            Self::PosesUnreadable { .. } => LookErrorKind::PosesUnreadable,
            Self::PosesInvalid { .. } => LookErrorKind::PosesInvalid,
            Self::InvalidDraft { .. } => LookErrorKind::InvalidDraft,
            Self::PoseNotChosen { .. } => LookErrorKind::PoseNotChosen,
            Self::EmptyWords { .. } => LookErrorKind::EmptyWords,
            Self::TextServiceMissing { .. } => LookErrorKind::TextServiceMissing,
            Self::InvalidModel { .. } => LookErrorKind::InvalidModel,
            Self::TextService { .. } => LookErrorKind::TextService,
            Self::AnswerRefused { .. } => LookErrorKind::AnswerRefused,
            Self::InvalidOperationKey { .. } => LookErrorKind::InvalidOperationKey,
            Self::Held { .. } => LookErrorKind::Held,
            Self::KeptChanged { .. } => LookErrorKind::KeptChanged,
            Self::NothingChanged { .. } => LookErrorKind::NothingChanged,
            Self::UnknownVersion { .. } => LookErrorKind::UnknownVersion,
            Self::NotKeepable { .. } => LookErrorKind::NotKeepable,
            Self::Locked { .. } => LookErrorKind::Locked,
            Self::LockBusy { .. } => LookErrorKind::LockBusy,
            Self::Generate { .. } => LookErrorKind::Generate,
            Self::InvalidPicture { .. } => LookErrorKind::InvalidPicture,
            Self::UnknownPicture { .. } => LookErrorKind::UnknownPicture,
            Self::InvalidCount { .. } => LookErrorKind::InvalidCount,
            Self::Storage { .. } => LookErrorKind::Storage,
            Self::StoreFailed { .. } => LookErrorKind::StoreFailed,
            Self::StudioClosed { .. } => LookErrorKind::StudioClosed,
            Self::Stopped { .. } => LookErrorKind::Stopped,
            Self::Seat(cause) => LookErrorKind::Seat(cause.kind()),
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::InvalidQuery(_) => "invalid_query",
            Self::Seat(cause) => cause.key(),
            Self::NoHero { .. } => "no_hero",
            Self::UnknownSpecies { .. } => "unknown_species",
            Self::UnknownClass { .. } => "unknown_class",
            Self::StudioUnreadable { .. } => "studio_unreadable",
            Self::StudioInvalid { .. } => "studio_invalid",
            Self::StudioRejected { .. } => "studio_rejected",
            Self::PosesUnreadable { .. } => "poses_unreadable",
            Self::PosesInvalid { .. } => "poses_invalid",
            Self::InvalidDraft { .. } => "invalid_draft",
            Self::PoseNotChosen { .. } => "pose_not_chosen",
            Self::EmptyWords => "empty_words",
            Self::TextServiceMissing { .. } => "text_service_missing",
            Self::InvalidModel { .. } => "invalid_model",
            Self::TextService { .. } => "text_service",
            Self::AnswerRefused { .. } => "answer_refused",
            Self::InvalidOperationKey => "invalid_operation_key",
            Self::Held { .. } => "held",
            Self::KeptChanged { .. } => "kept_changed",
            Self::NothingChanged => "nothing_changed",
            Self::UnknownVersion { .. } => "unknown_version",
            Self::NotKeepable { .. } => "not_keepable",
            Self::Locked { .. } => "locked",
            Self::LockBusy => "lock_busy",
            Self::Generate { .. } => "generate",
            Self::InvalidPicture { .. } => "invalid_picture",
            Self::UnknownPicture { .. } => "unknown_picture",
            Self::InvalidCount => "invalid_count",
            Self::Storage { .. } => "storage",
            Self::StoreFailed { .. } => "store_failed",
            Self::StudioClosed => "studio_closed",
            Self::Stopped => "stopped",
        }
    }
}

impl LookError {
    /// The key an audit line names: a seat refusal's own cause (`invalid_name`), else `key()`.
    pub fn audit_key(&self) -> &'static str {
        match self {
            Self::Seat(cause) => cause.key(),
            other => other.key(),
        }
    }
}

impl From<BodyError> for LookError {
    fn from(error: BodyError) -> Self {
        Self::InvalidBody(error)
    }
}

impl From<SeatError> for LookError {
    fn from(cause: SeatError) -> Self {
        Self::Seat(cause)
    }
}

impl fmt::Display for LookError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::InvalidQuery(error) => write!(formatter, "{error}"),
            Self::Seat(cause) => write!(formatter, "{cause}"),
            Self::NoHero { .. } => formatter.write_str("make a new hero or pick one first"),
            Self::UnknownSpecies { species } => {
                write!(formatter, "the look studio has no species {species:?}")
            }
            Self::UnknownClass { class } => {
                write!(formatter, "the look studio has no class {class:?}")
            }
            Self::StudioUnreadable { cause } => {
                write!(formatter, "the look studio file could not be read: {cause}")
            }
            Self::StudioInvalid { cause } => write!(
                formatter,
                "the look studio file is not valid: {}",
                cause.detail
            ),
            Self::StudioRejected { cause } => {
                write!(formatter, "the look studio file is not usable: {cause}")
            }
            Self::PosesUnreadable { cause } => {
                write!(formatter, "the base poses file could not be read: {cause}")
            }
            Self::PosesInvalid { cause } => write!(
                formatter,
                "the base poses file is not valid: {}",
                cause.detail
            ),
            Self::InvalidDraft { cause } => write!(formatter, "the draft is not usable: {cause}"),
            Self::PoseNotChosen { .. } => {
                formatter.write_str("the creator is not ready for this hero's size yet")
            }
            Self::EmptyWords => formatter.write_str("write something about the look first"),
            Self::TextServiceMissing { .. } => {
                formatter.write_str("reading your words is not available right now")
            }
            Self::InvalidModel { cause } => write!(formatter, "the model is not usable: {cause}"),
            Self::TextService { .. } | Self::AnswerRefused { .. } => {
                formatter.write_str("your words could not be read just now; try again")
            }
            Self::InvalidOperationKey => {
                formatter.write_str("the text service's operation key is not valid")
            }
            Self::Held { .. } => {
                formatter.write_str("this look cannot be pictured; change the description")
            }
            Self::KeptChanged { .. } => {
                formatter.write_str("the kept version changed; refresh and try again")
            }
            Self::NothingChanged => formatter.write_str("nothing changed since the kept version"),
            Self::UnknownVersion { .. } => formatter.write_str("that picture is no longer here"),
            Self::NotKeepable { .. } => formatter.write_str("that picture cannot be kept"),
            Self::Locked { .. } => {
                formatter.write_str("this hero's look is locked in; make a new hero for another")
            }
            Self::LockBusy => formatter.write_str("your hero is being locked in; wait a moment"),
            Self::Generate { .. } => {
                formatter.write_str("the picture could not be made just now; try again")
            }
            Self::InvalidPicture { picture } => {
                write!(formatter, "{picture:?} is not a picture name")
            }
            Self::UnknownPicture { picture } => {
                write!(formatter, "the media library has no picture {picture:?}")
            }
            Self::InvalidCount => formatter.write_str("ask for at least one candidate"),
            Self::Storage { .. } => {
                formatter.write_str("something went wrong at the table; try again")
            }
            Self::StoreFailed { detail } => {
                write!(
                    formatter,
                    "the media library did not take the pose: {detail}"
                )
            }
            Self::StudioClosed => formatter.write_str("the creator is not available right now"),
            Self::Stopped => formatter.write_str("something went wrong at the table; try again"),
        }
    }
}

impl std::error::Error for LookError {}

impl RouteError for LookError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_)
            | Self::InvalidQuery(_)
            | Self::InvalidDraft { .. }
            | Self::EmptyWords
            | Self::InvalidModel { .. }
            | Self::InvalidPicture { .. }
            | Self::InvalidCount => StatusCode::BAD_REQUEST,
            Self::Seat(cause) => cause.status(),
            Self::UnknownSpecies { .. }
            | Self::UnknownClass { .. }
            | Self::UnknownVersion { .. }
            | Self::UnknownPicture { .. } => StatusCode::NOT_FOUND,
            Self::NoHero { .. }
            | Self::PoseNotChosen { .. }
            | Self::KeptChanged { .. }
            | Self::NothingChanged
            | Self::NotKeepable { .. }
            | Self::Locked { .. }
            | Self::LockBusy => StatusCode::CONFLICT,
            Self::Held { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::TextServiceMissing { .. } | Self::StudioClosed => StatusCode::SERVICE_UNAVAILABLE,
            Self::TextService { .. } | Self::AnswerRefused { .. } => StatusCode::BAD_GATEWAY,
            Self::Generate { cause } => cause.status(),
            Self::StudioUnreadable { .. }
            | Self::StudioInvalid { .. }
            | Self::StudioRejected { .. }
            | Self::PosesUnreadable { .. }
            | Self::PosesInvalid { .. }
            | Self::InvalidOperationKey
            | Self::Storage { .. }
            | Self::StoreFailed { .. }
            | Self::Stopped => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// The query of `GET path::HERO`: the seat whose hero's look is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeroQuery {
    pub seat: u64,
}

/// `GET path::HERO`'s answer: the kept look of the hero a seat plays, for a runner's scene prompts:
/// its details (the chips, in a prompt's words) and its whole look in the studio's words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeroLook {
    pub hero: u64,
    pub details: Option<String>,
    pub look: Option<String>,
}

/// `path::HERO`: the hero the player at `seat` plays (one of their own), for the dev command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeroBind {
    pub seat: u64,
    pub hero: u64,
}

/// Which picture of a frame: its base pose, or its three-view sheet (the lock-in template).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoseView {
    Base,
    Sheet,
}

impl PoseView {
    pub const ALL: [Self; 2] = [Self::Base, Self::Sheet];

    pub fn key(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Sheet => "sheet",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|view| view.key() == key)
    }
}

/// `path::POSE_CANDIDATES`: make `count` candidates of a frame's base pose or sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoseCandidatesAsk {
    pub frame: BodyFrame,
    pub view: PoseView,
    pub count: u32,
}

/// The candidates asked for: the names they are stored under (the media library adds the
/// picture's extension) and their image jobs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoseCandidates {
    pub names: Vec<String>,
    pub jobs: Vec<u64>,
}

/// `path::POSE_CHOOSE`: `picture` becomes the frame's base pose or sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoseChoose {
    pub frame: BodyFrame,
    pub view: PoseView,
    pub picture: String,
}

/// One frame's chosen pictures (as stored in the media library), what each was chosen from, and
/// the next candidate numbers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FramePoses {
    pub base: Option<String>,
    pub base_from: Option<String>,
    pub sheet: Option<String>,
    pub sheet_from: Option<String>,
    pub next_base: u64,
    pub next_sheet: u64,
}

/// `path::POSES`: every frame's chosen pictures, the poses file as it stands.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PosesView {
    pub frames: BTreeMap<BodyFrame, FramePoses>,
}

/// The query of `path::REFERENCE`: a character of a picture, by its id in the cast.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceQuery {
    pub character: String,
}

/// `path::REFERENCE`'s answer: the character's reference sheet in the media library, if it has
/// one (a seated player's hero's from its pack, or `ref-<id>.png`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterSheet {
    pub sheet: Option<String>,
}

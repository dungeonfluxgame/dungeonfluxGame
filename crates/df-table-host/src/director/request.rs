//! What the `camera` and `director` verbs send to the host and what comes back (O.14, G1).
//!
//! Each verb is one `DirectorAct` posted as JSON, and every answer is a `DirectorReply`, the shared
//! dev envelope: `done` with the value or `rejected` with a `DirectorError` and its message.
//! Elements and reasons are typed on the wire, so a body naming something that is not an element
//! name or a reason, or carrying a field the verb does not take, is refused as an invalid body
//! before the Director sees it.

use std::fmt;

use axum::http::StatusCode;
use df_table::director::error::DirectorError as DecisionError;
use df_table::director::log::DirectorReport;
use df_table::director::reason::{MAXIMUM_CAMERA_MS, Reason};
use df_table::element::ElementId;
use serde::{Deserialize, Serialize};

use crate::dev_http::{BodyError, DevReply, RouteError};
use crate::playground::PlaygroundChangeError;

/// What the host does when a scene or combat starts, named for the verb `director start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartKind {
    Scene,
    Combat,
}

impl StartKind {
    pub const ALL: [Self; 2] = [Self::Scene, Self::Combat];

    pub fn key(self) -> &'static str {
        match self {
            Self::Scene => "scene",
            Self::Combat => "combat",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }
}

/// A change to what the Director sees of the shared screen: the stand-in for what the engine will
/// report (a hidden creature, a card in place of a plate, a result card), so the rules can be
/// rehearsed before the machines feed them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "edit", rename_all = "snake_case", deny_unknown_fields)]
pub enum ViewEdit {
    Hide {
        element: ElementId,
    },
    Unhide {
        element: ElementId,
    },
    Hero {
        element: ElementId,
    },
    /// An establishing card or a place card instead of a plate.
    Card,
    /// A plate, with its pixel width and the stage's device-pixel width when given.
    Plate {
        width_px: Option<u32>,
        stage_px: Option<u32>,
    },
    Collapsed {
        on: bool,
    },
    Reduced {
        on: bool,
    },
    /// A result card or reveal beat that holds the camera still for `hold_ms`.
    Beat {
        reveal: bool,
        hold_ms: u64,
        element: Option<ElementId>,
    },
    /// Back to a plate of unknown width with nothing hidden.
    Clear,
}

/// One verb of the camera or the Director. Every field is checked and an unknown one refused,
/// except on a verb with no fields (serde does not check those of a unit variant of a tagged enum),
/// where no field can be meant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "snake_case", deny_unknown_fields)]
pub enum DirectorAct {
    /// `camera focus`: a Director direction with a reason (`speaker` when none is given), so every
    /// rule applies.
    Focus {
        element: ElementId,
        reason: Option<Reason>,
        zoom_percent: Option<u16>,
        over_ms: Option<u64>,
        hold_ms: Option<u64>,
    },
    /// `camera wide`: the operator pulls the camera out.
    Wide {
        over_ms: Option<u64>,
    },
    Pause,
    Resume,
    /// `director auto camera on|off`.
    Auto {
        on: bool,
    },
    /// Moves the Director's controlled clock forward and lets it decide: holds run out without
    /// anyone waiting.
    Advance {
        by_ms: u64,
    },
    /// The engine's "the reason ended" for the reason and the element named.
    End {
        reason: Reason,
        element: ElementId,
    },
    Start {
        what: StartKind,
    },
    View {
        edit: ViewEdit,
    },
}

/// Why a Director dev call was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum DirectorError {
    /// The request body is not a verb.
    InvalidBody(BodyError),
    /// The Director refused the numbers or the clock.
    Refused { cause: DecisionError },
    /// `director advance` by more than the longest camera time.
    InvalidAdvance { by_ms: u64 },
    /// A focus needs the stage, and it could not be read (no fixture is shown). `detail` is the
    /// playground's sentence.
    NoStage { detail: String },
}

impl DirectorError {
    /// The serde name of the variant, for audit lines.
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::Refused { .. } => "refused",
            Self::InvalidAdvance { .. } => "invalid_advance",
            Self::NoStage { .. } => "no_stage",
        }
    }
}

impl fmt::Display for DirectorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::Refused { cause } => write!(formatter, "{cause}"),
            Self::InvalidAdvance { by_ms } => write!(
                formatter,
                "a clock advance is 0 to {MAXIMUM_CAMERA_MS} ms, not {by_ms}"
            ),
            Self::NoStage { detail } => formatter.write_str(detail),
        }
    }
}

impl std::error::Error for DirectorError {}

impl RouteError for DirectorError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_)
            | Self::Refused { .. }
            | Self::InvalidAdvance { .. }
            | Self::NoStage { .. } => StatusCode::BAD_REQUEST,
        }
    }
}

impl From<BodyError> for DirectorError {
    fn from(error: BodyError) -> Self {
        Self::InvalidBody(error)
    }
}

impl From<DecisionError> for DirectorError {
    fn from(cause: DecisionError) -> Self {
        Self::Refused { cause }
    }
}

/// The answer to a verb or a read: `done` with the value, or `rejected`.
pub type DirectorReply<T> = DevReply<T, DirectorError>;

/// Why the playground did not apply one of the Director's moves, for the `CAMERA apply_failed`
/// line: the playground's refusal as a closed key, never its words or its status number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyCause {
    /// The playground's checks refused the move: no fixture shown, or no such element.
    InvalidChange,
    /// The fixture or the scene changed while the move was checked.
    FixtureChanged,
    FileMissing,
    FileTooLarge,
    FileUnreadable,
}

impl ApplyCause {
    pub const ALL: [Self; 5] = [
        Self::InvalidChange,
        Self::FixtureChanged,
        Self::FileMissing,
        Self::FileTooLarge,
        Self::FileUnreadable,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::InvalidChange => "invalid_change",
            Self::FixtureChanged => "fixture_changed",
            Self::FileMissing => "file_missing",
            Self::FileTooLarge => "file_too_large",
            Self::FileUnreadable => "file_unreadable",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

impl From<&PlaygroundChangeError> for ApplyCause {
    fn from(error: &PlaygroundChangeError) -> Self {
        match error {
            // A Director's move is no body, so only the playground's checks refuse it.
            PlaygroundChangeError::InvalidBody(_)
            | PlaygroundChangeError::UnreadableBody { .. }
            | PlaygroundChangeError::InvalidChange { .. } => Self::InvalidChange,
            PlaygroundChangeError::FixtureChanged { .. } => Self::FixtureChanged,
            PlaygroundChangeError::FileMissing { .. } => Self::FileMissing,
            PlaygroundChangeError::FileTooLarge { .. } => Self::FileTooLarge,
            PlaygroundChangeError::FileUnreadable { .. } => Self::FileUnreadable,
        }
    }
}

/// What a verb did: the audit lines it wrote (the same lines `director show` is built from) and
/// anything the host could not apply.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActReply {
    pub lines: Vec<String>,
    /// A move the Director made that the playground refused (no fixture, a stale scene).
    pub problems: Vec<String>,
}

/// What the Director is shown, as `director show` prints it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewSummary {
    pub plate: String,
    pub stage_px: u32,
    pub hidden: Vec<String>,
    pub heroes: Vec<String>,
    pub collapsed_beat: bool,
    pub reduced_effects: bool,
    pub beat: Option<String>,
}

/// The answer to `director show`: the Director's report and what it sees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShowReply {
    pub report: DirectorReport,
    pub view: ViewSummary,
}

//! What the `camera` and `director` verbs send to the host and what comes back (O.14, G1).
//!
//! Each verb is one `DirectorAct` posted as JSON, and every answer is a `DirectorReply`: `done`
//! with the value or `rejected` with a typed error and its message, like the scene builder's.
//! Elements and reasons travel as text and are checked by the host, so a request that slips past
//! the command line meets the same checks (a reason that is not in the table, a name that is not
//! an element, a number outside its range).

use df_table::director_error::DirectorError;
use df_table::director_log::DirectorReport;
use serde::{Deserialize, Serialize};

/// What the host does when a scene or combat starts, named for the verb `director start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartKind {
    Scene,
    Combat,
}

/// A change to what the Director sees of the shared screen: the stand-in for what the engine will
/// report (a hidden creature, a card in place of a plate, a result card), so the rules can be
/// rehearsed before the machines feed them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "edit", rename_all = "snake_case")]
pub enum ViewEdit {
    Hide {
        element: String,
    },
    Unhide {
        element: String,
    },
    Hero {
        element: String,
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
    /// A result card or reveal beat that holds the camera still for `ms`.
    Beat {
        reveal: bool,
        ms: u64,
        element: Option<String>,
    },
    /// Back to a plate of unknown width with nothing hidden.
    Clear,
}

/// One verb of the camera or the Director.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "snake_case")]
pub enum DirectorAct {
    /// `camera focus`: a Director direction with a reason, so every rule applies.
    Focus {
        element: String,
        reason: Option<String>,
        zoom: Option<u16>,
        over: Option<u32>,
        hold: Option<u64>,
    },
    /// `camera wide`: the operator pulls the camera out.
    Wide {
        over: Option<u32>,
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
        ms: u64,
    },
    /// The engine's "the reason ended" for the reason and the element named.
    End {
        reason: String,
        element: String,
    },
    Start {
        what: StartKind,
    },
    View {
        edit: ViewEdit,
    },
}

/// What went wrong with a verb.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum DirectorFault {
    /// The Director refused the numbers or the clock.
    Refused {
        reason: String,
    },
    BadElement {
        text: String,
    },
    UnknownReason {
        text: String,
    },
    /// The request body is not a verb.
    BadRequest {
        reason: String,
    },
    /// The stage could not be read (no fixture is shown).
    NoStage {
        reason: String,
    },
}

impl std::fmt::Display for DirectorFault {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused { reason } | Self::BadRequest { reason } | Self::NoStage { reason } => {
                formatter.write_str(reason)
            }
            Self::BadElement { text } => write!(
                formatter,
                "'{text}' is not an element name: use a to z, 0 to 9, '_' and '-', starting with a letter"
            ),
            Self::UnknownReason { text } => write!(
                formatter,
                "'{text}' is not a reason; use {}",
                df_table::director_reason::Reason::keys()
            ),
        }
    }
}

impl From<DirectorError> for DirectorFault {
    fn from(error: DirectorError) -> Self {
        Self::Refused {
            reason: error.to_string(),
        }
    }
}

/// An answer to a verb or a read: `done` with the value, or `rejected`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DirectorReply<T> {
    Done {
        value: T,
    },
    Rejected {
        error: DirectorFault,
        message: String,
    },
}

impl<T> DirectorReply<T> {
    pub fn from_result(result: Result<T, DirectorFault>) -> Self {
        match result {
            Ok(value) => Self::Done { value },
            Err(error) => {
                let message = error.to_string();
                Self::Rejected { error, message }
            }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShowReply {
    pub report: DirectorReport,
    pub view: ViewSummary,
}

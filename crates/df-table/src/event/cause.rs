//! The closed causes and kinds the events name: the kinds of canvas change, why a change was
//! rejected, the seven pipeline stages and why work at a boundary failed.
//!
//! They live apart from `event.rs` so that file stays about the events themselves. Every value is
//! a closed enum with a fixed key written into the audit line, never an error text, so a cause can
//! be counted and compared without reading prose. They are re-exported by `event`, which is where
//! the rest of the crate names them.

use serde::{Deserialize, Serialize};

use crate::listed::listed_enum;

/// The kinds of change the canvas accepts, named for the audit line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    Spawn,
    Despawn,
    Set,
    Move,
    Travel,
    Stop,
    Clear,
    Transform,
}

impl EffectKind {
    /// The closed key written into an audit line.
    pub fn key(self) -> &'static str {
        match self {
            Self::Spawn => "spawn",
            Self::Despawn => "despawn",
            Self::Set => "set",
            Self::Move => "move",
            Self::Travel => "travel",
            Self::Stop => "stop",
            Self::Clear => "clear",
            Self::Transform => "transform",
        }
    }
}

listed_enum! {
    /// Which appearance a transform changed, named for the audit line.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum TransformKind {
        Rotate,
    }
}

impl TransformKind {
    /// The closed key written into an audit line.
    pub fn key(self) -> &'static str {
        match self {
            Self::Rotate => "rotate",
        }
    }
}

/// Why a change or a request was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionCause {
    UnknownKind,
    DuplicateElement,
    UnknownElement,
    UnknownField,
    InvalidValue,
    InvalidName,
    MissingRequiredField,
    TooManyElements,
    OutOfBounds,
    InvalidSpeed,
    /// A rotation or pivot outside its range.
    InvalidTransform,
    /// A request that could not be read as an effect at all.
    MalformedRequest,
    /// A request that did not come from the local machine, or not as JSON.
    ForbiddenRequest,
}

impl RejectionCause {
    /// The closed key written into an audit line.
    pub fn key(self) -> &'static str {
        match self {
            Self::UnknownKind => "unknown_kind",
            Self::DuplicateElement => "duplicate_element",
            Self::UnknownElement => "unknown_element",
            Self::UnknownField => "unknown_field",
            Self::InvalidValue => "invalid_value",
            Self::InvalidName => "invalid_name",
            Self::MissingRequiredField => "missing_required_field",
            Self::TooManyElements => "too_many_elements",
            Self::OutOfBounds => "out_of_bounds",
            Self::InvalidSpeed => "invalid_speed",
            Self::InvalidTransform => "invalid_transform",
            Self::MalformedRequest => "malformed_request",
            Self::ForbiddenRequest => "forbidden_request",
        }
    }
}

listed_enum! {
    /// The seven stages an input passes through (section C of the plan).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum PipelineStep {
        Admit,
        Interpret,
        Check,
        Decide,
        Apply,
        Narrate,
        Project,
    }
}

impl PipelineStep {
    pub fn key(self) -> &'static str {
        match self {
            Self::Admit => "admit",
            Self::Interpret => "interpret",
            Self::Check => "check",
            Self::Decide => "decide",
            Self::Apply => "apply",
            Self::Narrate => "narrate",
            Self::Project => "project",
        }
    }
}

listed_enum! {
    /// Why work at a boundary failed: a closed cause, never an error text.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum FailureCause {
        /// The owner or provider could not be reached or had nothing to give.
        Unavailable,
        TimedOut,
        /// A provider refused; a refusal is never retried with other wording (pattern 11).
        Refused,
        /// Model text that is empty, over-length, the wrong shape or the wrong language (C.7).
        TextInvalid,
        /// An answer outside what the question allows, such as a die value off the die.
        InvalidAnswer,
        /// A result for an older revision or scene generation (RES42), or a handle that is no
        /// longer current or never was.
        Stale,
    }
}

impl FailureCause {
    pub fn key(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::TimedOut => "timed_out",
            Self::Refused => "refused",
            Self::TextInvalid => "text_invalid",
            Self::InvalidAnswer => "invalid_answer",
            Self::Stale => "stale",
        }
    }
}

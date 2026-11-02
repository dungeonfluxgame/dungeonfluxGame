//! Typed events: the one record of what the table did and why.
//!
//! Every variant carries only ids, enums, counts and revisions, so no private text can reach an
//! event by accident. It owns the audit-line rendering (`AREA verb key=value`) that hosts print
//! and store, so the line and the event never disagree. It never decides what to record: the code
//! that changes state does, and a change that records nothing is a defect.

use serde::{Deserialize, Serialize};

use crate::element::{ElementId, FieldName, KindName};

/// How often an event is wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventClass {
    /// Always recorded and never sampled: a state change, a rejection, a reload.
    Always,
    /// Detail a host may leave out: frequent, low-value repeats such as a move. A host that leaves
    /// them out shows gaps in the revision numbers.
    Verbose,
}

/// The kinds of change the canvas accepts, named for the audit line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectKind {
    Spawn,
    Despawn,
    Set,
    Move,
    Clear,
}

impl EffectKind {
    /// The closed key written into an audit line.
    pub fn key(self) -> &'static str {
        match self {
            Self::Spawn => "spawn",
            Self::Despawn => "despawn",
            Self::Set => "set",
            Self::Move => "move",
            Self::Clear => "clear",
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
            Self::MalformedRequest => "malformed_request",
            Self::ForbiddenRequest => "forbidden_request",
        }
    }
}

/// One thing the table did or refused to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableEvent {
    ElementSpawned {
        element: ElementId,
        kind: KindName,
        revision: u64,
    },
    ElementDespawned {
        element: ElementId,
        revision: u64,
    },
    ElementChanged {
        element: ElementId,
        field: FieldName,
        revision: u64,
    },
    ElementMoved {
        element: ElementId,
        revision: u64,
    },
    CanvasCleared {
        removed: usize,
        revision: u64,
    },
    /// A change or a request that was refused. `effect` is absent when the request could not be
    /// read far enough to know which change it asked for.
    EffectRejected {
        effect: Option<EffectKind>,
        element: Option<ElementId>,
        cause: RejectionCause,
    },
    KindsReloaded {
        kinds: usize,
        invalid_elements: usize,
        revision: u64,
    },
    KindsRejected {
        errors: usize,
    },
}

/// One line for the audit stream: a fixed upper-case area and a `verb key=value` text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditLine {
    pub area: &'static str,
    pub text: String,
}

impl AuditLine {
    pub fn render(&self) -> String {
        format!("{} {}", self.area, self.text)
    }
}

impl TableEvent {
    pub fn class(&self) -> EventClass {
        match self {
            Self::ElementMoved { .. } => EventClass::Verbose,
            Self::ElementSpawned { .. }
            | Self::ElementDespawned { .. }
            | Self::ElementChanged { .. }
            | Self::CanvasCleared { .. }
            | Self::EffectRejected { .. }
            | Self::KindsReloaded { .. }
            | Self::KindsRejected { .. } => EventClass::Always,
        }
    }

    /// The audit area of the event. Every variant names its own, so a future event outside the
    /// canvas cannot be filed under it by default.
    pub fn area(&self) -> &'static str {
        match self {
            Self::ElementSpawned { .. }
            | Self::ElementDespawned { .. }
            | Self::ElementChanged { .. }
            | Self::ElementMoved { .. }
            | Self::CanvasCleared { .. }
            | Self::EffectRejected { .. }
            | Self::KindsReloaded { .. }
            | Self::KindsRejected { .. } => "CANVAS",
        }
    }

    pub fn audit_line(&self) -> AuditLine {
        let text = match self {
            Self::ElementSpawned {
                element,
                kind,
                revision,
            } => format!("spawn element={element} kind={kind} revision={revision}"),
            Self::ElementDespawned { element, revision } => {
                format!("despawn element={element} revision={revision}")
            }
            Self::ElementChanged {
                element,
                field,
                revision,
            } => format!("set element={element} field={field} revision={revision}"),
            Self::ElementMoved { element, revision } => {
                format!("move element={element} revision={revision}")
            }
            Self::CanvasCleared { removed, revision } => {
                format!("clear removed={removed} revision={revision}")
            }
            Self::EffectRejected {
                effect,
                element,
                cause,
            } => {
                let mut text = String::from("reject");
                if let Some(effect) = effect {
                    text.push_str(&format!(" effect={}", effect.key()));
                }
                text.push_str(&format!(" cause={}", cause.key()));
                if let Some(element) = element {
                    text.push_str(&format!(" element={element}"));
                }
                text
            }
            Self::KindsReloaded {
                kinds,
                invalid_elements,
                revision,
            } => format!(
                "reload kinds={kinds} invalid_elements={invalid_elements} revision={revision}"
            ),
            Self::KindsRejected { errors } => format!("reload_rejected errors={errors}"),
        };
        AuditLine {
            area: self.area(),
            text,
        }
    }
}

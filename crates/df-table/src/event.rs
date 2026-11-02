//! Typed events: the one record of what the table did and why.
//!
//! Every variant carries only ids, enums, counts, revisions and millisecond durations, and every
//! value is an `EventField` (`event_field.rs`), so no private text can reach an event by accident
//! and an audit line cannot be built from one. It owns the variants, their class, their audit
//! area and the exhaustive audit-line rendering (`AREA verb key=value`) that hosts print and
//! store; the same record serializes with serde, so the line and the structured record never
//! disagree. It never decides what to record: the code that changes state does, and a change that
//! records nothing is a defect.
//!
//! Class A (`EventClass::Always`) events are always on and never sampled. The fields of an engine
//! class A event are a contract operators read, so they only ever grow: `table`, `scene`,
//! `machine`, `operation_id`, `turn_seq`, `epoch`, `revision`, `length`, `evidence`, `cause`,
//! `code`, `ms` and the other keys a test pins (`event_tests.rs`, question 89). Class B (`EventClass::Verbose`) is
//! detail a host may leave out.

use std::fmt::{self, Write};

use serde::{Deserialize, Serialize};

use crate::config::{SettingFault, SettingName};
use crate::element::{ElementId, FieldName, KindName};
use crate::event_field::LineWriter;
use crate::identity::{Correlation, EvidenceRef, TableScope};
use crate::listed::listed_enum;
use crate::outcome::{DeclineCode, OutcomeKind};
use crate::ports::PortName;
use crate::private_data::LengthClass;

/// The room reserved for an audit line. The widest possible line (every id at its maximum length,
/// 20-digit numbers and a 48-character machine name) is under 400 bytes, which a test proves, so
/// rendering allocates exactly once.
pub(crate) const AUDIT_LINE_CAPACITY_BYTES: usize = 400;

/// How often an event is wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventClass {
    /// Class A of O.6: always recorded and never sampled: a state change, a rejection, a reload.
    Always,
    /// Class B of O.6: detail a host may leave out: frequent, low-value repeats such as a move or
    /// a pipeline step. A host that leaves them out shows gaps in the revision numbers.
    Verbose,
}

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

/// One thing the table did or refused to do. The first group is the canvas workbench; the second
/// is the engine (O.6). Every variant is class A except `ElementMoved` and `StepFinished`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
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
    /// An element started travelling toward a point.
    ElementTravelling {
        element: ElementId,
        revision: u64,
    },
    /// An element reached the point it was travelling to.
    ElementArrived {
        element: ElementId,
        revision: u64,
    },
    /// An element's travel was ended where it stood.
    ElementStopped {
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
    /// An input reached the owner (`input.received`). The text itself is never recorded: `length`
    /// is its size class, and `evidence` points at the specimen in the diagnostic store when one
    /// was kept (RES36).
    InputReceived {
        scope: TableScope,
        correlation: Correlation,
        length: LengthClass,
        evidence: Option<EvidenceRef>,
    },
    /// An input ended declined. Only the code is recorded: the reason text can carry names.
    InputDeclined {
        scope: TableScope,
        correlation: Correlation,
        code: DeclineCode,
    },
    /// One stage finished (`step.finished`). `outcome` is absent when the stage passed the input
    /// on to the next.
    StepFinished {
        scope: TableScope,
        correlation: Correlation,
        step: PipelineStep,
        outcome: Option<OutcomeKind>,
        ms: u64,
    },
    /// An input was committed and its effects applied (`turn.applied`). `ms` is admit to commit.
    TurnApplied {
        scope: TableScope,
        correlation: Correlation,
        ms: u64,
    },
    /// A port could not answer; recorded once, by the stage that handles it.
    BoundaryFailed {
        scope: TableScope,
        correlation: Correlation,
        port: PortName,
        cause: FailureCause,
        ms: u64,
    },
    /// A configuration value was not used and the setting kept its default (an unknown name
    /// changes nothing).
    SettingRejected {
        name: SettingName,
        cause: SettingFault,
    },
    /// Keys of the configuration file that are not valid setting names, so none can be named
    /// safely. Only their number is recorded.
    SettingKeysIgnored {
        count: usize,
    },
}

impl TableEvent {
    pub fn class(&self) -> EventClass {
        match self {
            Self::ElementMoved { .. } | Self::StepFinished { .. } => EventClass::Verbose,
            Self::ElementSpawned { .. }
            | Self::ElementDespawned { .. }
            | Self::ElementChanged { .. }
            | Self::ElementTravelling { .. }
            | Self::ElementArrived { .. }
            | Self::ElementStopped { .. }
            | Self::CanvasCleared { .. }
            | Self::EffectRejected { .. }
            | Self::KindsReloaded { .. }
            | Self::KindsRejected { .. }
            | Self::InputReceived { .. }
            | Self::InputDeclined { .. }
            | Self::TurnApplied { .. }
            | Self::BoundaryFailed { .. }
            | Self::SettingRejected { .. }
            | Self::SettingKeysIgnored { .. } => EventClass::Always,
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
            | Self::ElementTravelling { .. }
            | Self::ElementArrived { .. }
            | Self::ElementStopped { .. }
            | Self::CanvasCleared { .. }
            | Self::EffectRejected { .. }
            | Self::KindsReloaded { .. }
            | Self::KindsRejected { .. } => "CANVAS",
            Self::InputReceived { .. } | Self::InputDeclined { .. } => "INPUT",
            Self::StepFinished { .. } => "STEP",
            Self::TurnApplied { .. } => "TURN",
            Self::BoundaryFailed { .. } => "BOUNDARY",
            Self::SettingRejected { .. } | Self::SettingKeysIgnored { .. } => "SETTING",
        }
    }

    /// The whole line, `AREA verb key=value ...`, in one allocation. The one canonical renderer
    /// with `write_audit_line`, which it wraps.
    pub fn render_audit_line(&self) -> String {
        let mut line = String::with_capacity(AUDIT_LINE_CAPACITY_BYTES);
        // Writing into a String cannot fail, so the result carries no information.
        let _ = self.write_audit_line(&mut line);
        line
    }

    /// Writes the whole line into any writer without allocating: the one renderer.
    pub fn write_audit_line<W: Write>(&self, out: &mut W) -> fmt::Result {
        out.write_str(self.area())?;
        out.write_char(' ')?;
        self.write_audit_body(out)
    }

    fn write_audit_body<W: Write>(&self, out: &mut W) -> fmt::Result {
        match self {
            Self::ElementSpawned {
                element,
                kind,
                revision,
            } => {
                let mut line = LineWriter::start(out, "spawn")?;
                line.field("element", element)?;
                line.field("kind", kind)?;
                line.field("revision", revision)
            }
            Self::ElementDespawned { element, revision } => {
                write_element_line(out, "despawn", element, *revision)
            }
            Self::ElementChanged {
                element,
                field,
                revision,
            } => {
                let mut line = LineWriter::start(out, "set")?;
                line.field("element", element)?;
                line.field("field", field)?;
                line.field("revision", revision)
            }
            Self::ElementMoved { element, revision } => {
                write_element_line(out, "move", element, *revision)
            }
            Self::ElementTravelling { element, revision } => {
                write_element_line(out, "travel", element, *revision)
            }
            Self::ElementArrived { element, revision } => {
                write_element_line(out, "arrive", element, *revision)
            }
            Self::ElementStopped { element, revision } => {
                write_element_line(out, "stop", element, *revision)
            }
            Self::CanvasCleared { removed, revision } => {
                let mut line = LineWriter::start(out, "clear")?;
                line.field("removed", removed)?;
                line.field("revision", revision)
            }
            Self::EffectRejected {
                effect,
                element,
                cause,
            } => {
                let mut line = LineWriter::start(out, "reject")?;
                line.field_if_present("effect", effect)?;
                line.field("cause", cause)?;
                line.field_if_present("element", element)
            }
            Self::KindsReloaded {
                kinds,
                invalid_elements,
                revision,
            } => {
                let mut line = LineWriter::start(out, "reload")?;
                line.field("kinds", kinds)?;
                line.field("invalid_elements", invalid_elements)?;
                line.field("revision", revision)
            }
            Self::KindsRejected { errors } => {
                let mut line = LineWriter::start(out, "reload_rejected")?;
                line.field("errors", errors)
            }
            Self::InputReceived {
                scope,
                correlation,
                length,
                evidence,
            } => {
                let mut line = LineWriter::start(out, "received")?;
                write_context(&mut line, scope, correlation)?;
                line.field("length", length)?;
                line.field_if_present("evidence", evidence)
            }
            Self::InputDeclined {
                scope,
                correlation,
                code,
            } => {
                let mut line = LineWriter::start(out, "declined")?;
                write_context(&mut line, scope, correlation)?;
                line.field("code", code)
            }
            Self::StepFinished {
                scope,
                correlation,
                step,
                outcome,
                ms,
            } => {
                let mut line = LineWriter::start(out, "finished")?;
                write_context(&mut line, scope, correlation)?;
                line.field("step", step)?;
                line.field_if_present("outcome", outcome)?;
                line.field("ms", ms)
            }
            Self::TurnApplied {
                scope,
                correlation,
                ms,
            } => {
                let mut line = LineWriter::start(out, "applied")?;
                write_context(&mut line, scope, correlation)?;
                line.field("ms", ms)
            }
            Self::BoundaryFailed {
                scope,
                correlation,
                port,
                cause,
                ms,
            } => {
                let mut line = LineWriter::start(out, "failed")?;
                write_context(&mut line, scope, correlation)?;
                line.field("port", port)?;
                line.field("cause", cause)?;
                line.field("ms", ms)
            }
            Self::SettingRejected { name, cause } => {
                let mut line = LineWriter::start(out, "rejected")?;
                line.field("name", name)?;
                line.field("cause", cause)
            }
            Self::SettingKeysIgnored { count } => {
                let mut line = LineWriter::start(out, "keys_ignored")?;
                line.field("count", count)
            }
        }
    }
}

fn write_element_line<W: Write>(
    out: &mut W,
    verb: &str,
    element: &ElementId,
    revision: u64,
) -> fmt::Result {
    let mut line = LineWriter::start(out, verb)?;
    line.field("element", element)?;
    line.field("revision", &revision)
}

/// The keys every engine event shares, always in this order. They are pinned (question 89).
fn write_context<W: Write>(
    line: &mut LineWriter<'_, W>,
    scope: &TableScope,
    correlation: &Correlation,
) -> fmt::Result {
    line.field("table", &scope.table)?;
    line.field_if_present("scene", &scope.scene)?;
    line.field_if_present("machine", &scope.machine)?;
    line.field("operation_id", &correlation.operation_id)?;
    line.field("turn_seq", &correlation.turn_seq)?;
    line.field("epoch", &correlation.revision.epoch().get())?;
    line.field("revision", &correlation.revision.sequence())
}

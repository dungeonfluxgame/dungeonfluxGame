//! Typed events: the one record of what the table did and why.
//!
//! Every variant carries only ids, enums, counts, revisions and millisecond durations, and every
//! value is an `EventField` (`event::field`), so no private text can reach an event by accident
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

pub(crate) mod cause;
pub mod field;

use std::fmt::{self, Write};

use serde::{Deserialize, Serialize};

use crate::config::{SettingCause, SettingName};
use crate::effects::{EffectCause, TableEffectKind};
use crate::element::{Degrees, ElementId, FieldName, KindName, Percent};
use crate::event::field::LineWriter;
use crate::identity::{Correlation, EvidenceRef, MachineId, TableId, TableScope};
use crate::limits::MAXIMUM_NAME_LENGTH;
use crate::machine::slot::{LoadCause, RecordKind};
use crate::machine::{HandOffKind, MachineFailureCause, StateName};
use crate::outcome::{DeclineCode, OutcomeKind};
use crate::owner_queue::{Lane, NotDeclinedCause};
use crate::pipeline::events::PipelineEvent;
use crate::ports::PortName;
use crate::private_data::LengthClass;
use crate::projection::ProjectionEvent;
use crate::table_state::TableStateEvent;

/// Everything in the widest audit line that is not a name: the area and verb, every key, 20-digit
/// numbers and 32-digit ids. The widest line has about 270 such bytes; this leaves room.
const AUDIT_LINE_FIXED_BYTES: usize = 352;
/// Most names one line carries, each up to `MAXIMUM_NAME_LENGTH`: a machine and the two states of
/// a transition.
const AUDIT_LINE_MOST_NAMES: usize = 3;
/// The room reserved for an audit line, derived from the longest a name may be. The widest
/// possible line (every id at its maximum length, 20-digit numbers and the most names at the
/// maximum name length) is under this, which a test proves, so rendering allocates exactly once.
pub(crate) const AUDIT_LINE_CAPACITY_BYTES: usize =
    AUDIT_LINE_FIXED_BYTES + AUDIT_LINE_MOST_NAMES * MAXIMUM_NAME_LENGTH;

/// How often an event is wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventClass {
    /// Class A of O.6: always recorded and never sampled: a state change, a rejection, a reload.
    Always,
    /// Class B of O.6: detail a host may leave out: frequent, low-value repeats such as a move or
    /// a pipeline step. A host that leaves them out shows gaps in the revision numbers.
    Verbose,
}

pub use crate::event::cause::{
    EffectKind, FailureCause, PipelineStep, RejectionCause, TransformKind,
};

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
        cause: SettingCause,
    },
    /// Keys of the configuration file that are not valid setting names, so none can be named
    /// safely. Only their number is recorded.
    SettingKeysIgnored {
        count: usize,
    },
    /// An element's appearance changed (`transform`). `kind` says which appearance, `degrees` is
    /// the rotation it now has and the pivot fields are the pivot now in force, so a change of
    /// pivot alone is visible in the log.
    ElementTransformed {
        element: ElementId,
        kind: TransformKind,
        degrees: Degrees,
        pivot_x_percent: Percent,
        pivot_y_percent: Percent,
        revision: u64,
    },
    /// An input was refused without a decline code (a duplicate, a seat that is not at the table,
    /// a full engine lane). The causes with a code are `InputDeclined`.
    InputRefused {
        scope: TableScope,
        correlation: Correlation,
        cause: NotDeclinedCause,
    },
    /// The owner took an input from a lane and gave it its `turn_seq`; `ms` is admit to serve.
    InputServed {
        scope: TableScope,
        correlation: Correlation,
        lane: Lane,
        ms: u64,
    },
    /// The owner gave up on a served input that never finished (`cause` says why, `ms` is how long
    /// it had been in progress).
    InputAbandoned {
        scope: TableScope,
        correlation: Correlation,
        cause: FailureCause,
        ms: u64,
    },
    /// The outcome of an input arrived after the owner had abandoned it; the outcome is now
    /// recorded as the operation's result.
    InputFinishedLate {
        scope: TableScope,
        correlation: Correlation,
        outcome: OutcomeKind,
    },
    /// A machine started for the table; `state` is the state it began in.
    MachineEntered {
        scope: TableScope,
        correlation: Correlation,
        state: StateName,
    },
    /// A machine moved from one of its states to another.
    MachineTransitioned {
        scope: TableScope,
        correlation: Correlation,
        from: StateName,
        to: StateName,
    },
    /// A machine ended. `kind` says how, `target` names the machine it handed the table to when
    /// it named one, and `carried` counts the items it handed over.
    MachineExited {
        scope: TableScope,
        correlation: Correlation,
        kind: HandOffKind,
        target: Option<MachineId>,
        carried: usize,
    },
    /// A machine call failed; recorded once, by the code that handles it.
    MachineFailed {
        scope: TableScope,
        correlation: Correlation,
        cause: MachineFailureCause,
    },
    /// One table effect was applied.
    EffectApplied {
        scope: TableScope,
        correlation: Correlation,
        effect: TableEffectKind,
    },
    /// A commit's effect was refused, so none of the commit was applied.
    EffectRefused {
        scope: TableScope,
        correlation: Correlation,
        effect: TableEffectKind,
        cause: EffectCause,
    },
    /// A saved record could not be used (`load.refused{type, version, expected}`, H.12) when the
    /// host loaded `table`: `record` is its type, `machine` the machine a slot belongs to,
    /// `version` the one it carries and `expected` the one this engine reads.
    LoadRefused {
        table: TableId,
        record: RecordKind,
        machine: Option<MachineId>,
        version: u64,
        expected: u64,
        cause: LoadCause,
    },
    /// What the pipeline and the pending resolutions did that explains a turn (`pipeline.rs`).
    Pipeline(PipelineEvent),
    /// What the table state did that explains a turn: a stale result set aside (`table_state`).
    TableState(TableStateEvent),
    /// An audience's view was built (`projection`).
    Projection(ProjectionEvent),
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
            | Self::SettingKeysIgnored { .. }
            | Self::ElementTransformed { .. }
            | Self::InputRefused { .. }
            | Self::InputServed { .. }
            | Self::InputAbandoned { .. }
            | Self::InputFinishedLate { .. }
            | Self::MachineEntered { .. }
            | Self::MachineTransitioned { .. }
            | Self::MachineExited { .. }
            | Self::MachineFailed { .. }
            | Self::EffectApplied { .. }
            | Self::EffectRefused { .. }
            | Self::LoadRefused { .. } => EventClass::Always,
            Self::Pipeline(event) => event.class(),
            Self::TableState(event) => event.class(),
            Self::Projection(event) => event.class(),
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
            | Self::KindsRejected { .. }
            | Self::ElementTransformed { .. } => "CANVAS",
            Self::InputReceived { .. }
            | Self::InputDeclined { .. }
            | Self::InputRefused { .. }
            | Self::InputServed { .. }
            | Self::InputAbandoned { .. }
            | Self::InputFinishedLate { .. } => "INPUT",
            Self::StepFinished { .. } => "STEP",
            Self::TurnApplied { .. } => "TURN",
            Self::BoundaryFailed { .. } => "BOUNDARY",
            Self::SettingRejected { .. } | Self::SettingKeysIgnored { .. } => "SETTING",
            Self::MachineEntered { .. }
            | Self::MachineTransitioned { .. }
            | Self::MachineExited { .. }
            | Self::MachineFailed { .. } => "MACHINE",
            Self::EffectApplied { .. } | Self::EffectRefused { .. } => "EFFECT",
            Self::LoadRefused { .. } => "LOAD",
            Self::Pipeline(event) => event.area(),
            Self::TableState(event) => event.area(),
            Self::Projection(event) => event.area(),
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
                line.field_if_present("effect", effect.as_ref())?;
                line.field("cause", cause)?;
                line.field_if_present("element", element.as_ref())
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
                line.field_if_present("evidence", evidence.as_ref())
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
                line.field_if_present("outcome", outcome.as_ref())?;
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
            Self::ElementTransformed {
                element,
                kind,
                degrees,
                pivot_x_percent,
                pivot_y_percent,
                revision,
            } => {
                let mut line = LineWriter::start(out, "transform")?;
                line.field("element", element)?;
                line.field("kind", kind)?;
                line.field("degrees", degrees)?;
                line.field("pivot_x_percent", pivot_x_percent)?;
                line.field("pivot_y_percent", pivot_y_percent)?;
                line.field("revision", revision)
            }
            Self::InputRefused {
                scope,
                correlation,
                cause,
            } => {
                let mut line = LineWriter::start(out, "refused")?;
                write_context(&mut line, scope, correlation)?;
                line.field("cause", cause)
            }
            Self::InputFinishedLate {
                scope,
                correlation,
                outcome,
            } => {
                let mut line = LineWriter::start(out, "finished_late")?;
                write_context(&mut line, scope, correlation)?;
                line.field("outcome", outcome)
            }
            Self::InputAbandoned {
                scope,
                correlation,
                cause,
                ms,
            } => {
                let mut line = LineWriter::start(out, "abandoned")?;
                write_context(&mut line, scope, correlation)?;
                line.field("cause", cause)?;
                line.field("ms", ms)
            }
            Self::InputServed {
                scope,
                correlation,
                lane,
                ms,
            } => {
                let mut line = LineWriter::start(out, "served")?;
                write_context(&mut line, scope, correlation)?;
                line.field("lane", lane)?;
                line.field("ms", ms)
            }
            Self::MachineEntered {
                scope,
                correlation,
                state,
            } => {
                let mut line = LineWriter::start(out, "entered")?;
                write_context(&mut line, scope, correlation)?;
                line.field("state", state)
            }
            Self::MachineTransitioned {
                scope,
                correlation,
                from,
                to,
            } => {
                let mut line = LineWriter::start(out, "transitioned")?;
                write_context(&mut line, scope, correlation)?;
                line.field("from", from)?;
                line.field("to", to)
            }
            Self::MachineExited {
                scope,
                correlation,
                kind,
                target,
                carried,
            } => {
                let mut line = LineWriter::start(out, "exited")?;
                write_context(&mut line, scope, correlation)?;
                line.field("kind", kind)?;
                line.field_if_present("target", target.as_ref())?;
                line.field("carried", carried)
            }
            Self::MachineFailed {
                scope,
                correlation,
                cause,
            } => {
                let mut line = LineWriter::start(out, "failed")?;
                write_context(&mut line, scope, correlation)?;
                line.field("cause", cause)
            }
            Self::EffectApplied {
                scope,
                correlation,
                effect,
            } => {
                let mut line = LineWriter::start(out, "applied")?;
                write_context(&mut line, scope, correlation)?;
                line.field("effect", effect)
            }
            Self::EffectRefused {
                scope,
                correlation,
                effect,
                cause,
            } => {
                let mut line = LineWriter::start(out, "refused")?;
                write_context(&mut line, scope, correlation)?;
                line.field("effect", effect)?;
                line.field("cause", cause)
            }
            Self::LoadRefused {
                table,
                record,
                machine,
                version,
                expected,
                cause,
            } => {
                let mut line = LineWriter::start(out, "refused")?;
                line.field("table", table)?;
                line.field("record", record)?;
                line.field_if_present("machine", machine.as_ref())?;
                line.field("version", version)?;
                line.field("expected", expected)?;
                line.field("cause", cause)
            }
            Self::Pipeline(event) => event.write_audit_body(out),
            Self::TableState(event) => event.write_audit_body(out),
            Self::Projection(event) => event.write_audit_body(out),
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
pub(crate) fn write_context<W: Write>(
    line: &mut LineWriter<'_, W>,
    scope: &TableScope,
    correlation: &Correlation,
) -> fmt::Result {
    line.field("table", &scope.table)?;
    line.field_if_present("scene", scope.scene.as_ref())?;
    line.field_if_present("machine", scope.machine.as_ref())?;
    line.field("operation_id", &correlation.operation_id)?;
    line.field("turn_seq", &correlation.turn_seq)?;
    line.field("epoch", &correlation.revision.epoch().get())?;
    line.field("revision", &correlation.revision.sequence())
}

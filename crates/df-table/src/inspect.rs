//! Inspect reports: every module that holds state says what it holds, to one audience (O.6, H.16).
//!
//! It owns the `Inspect` trait, the `InspectAudience` a report is built for (operator, host or one
//! seat), `Visible`, the field type that says a value is shown or redacted and for whom, and one
//! report type per stateful module in the submodules: the owner queue, the registry, the wired
//! ports, the active machines, the table state, the pending book, the stored rulings, the request
//! and result books, the settings and the pipeline's table as a whole. The broker and the other
//! port adapters live in the host, which implements `Inspect` for them; this crate reports the
//! wiring (`WiredPorts`). The verb set behind the dev CLI (`verbs`) and `describe`, generated from
//! the registry (`describe`), live here too, so a host can build its commands from data.
//!
//! Redaction is the report's own job and happens while it is built: a value an audience may not
//! see is never read into the report, it is replaced by `Visible::Redacted` naming who could see
//! it, and a list a seat may only partly see holds only the entries that seat may see. A caller
//! can neither forget to redact nor un-redact, because the hidden value is not in the report. The
//! rules are the same for every report:
//!
//! - The operator sees everything, including a machine's saved payload, the dice a pending drew and
//!   the table digest, which moves when a hidden fact moves.
//! - The host sees what the shared screen shows plus what administers the table: the queue, the
//!   registry, the ports, every pending with its continuation and the stored rulings. It does not
//!   see hidden creatures, other seats' private facts, saved machine payloads or drawn dice.
//! - A seat sees what the shared screen shows and its own lines of the queue, the offers made to
//!   it and its own projection. Nothing else, and not even a count that would change when a hidden
//!   fact changes.
//!
//! A report holds ids, counts, closed keys and the table's public names, never player text,
//! narration or a private fact (RES36): the private types of `private_data` cannot be put in one.
//! It reads no clock (the caller gives `now_ms`), draws no die, changes nothing and never fails:
//! a machine that cannot describe itself is reported as such inside the report (`MachineStanding`),
//! so the failure is visible and the rest of the report is still there.

mod books;
mod describe;
mod machines;
mod pending;
mod pipeline;
mod queue;
mod registry;
mod settings;
mod table;
mod verbs;

use serde::{Deserialize, Serialize, Serializer};

use crate::identity::SeatId;
use crate::listed::listed_enum;

pub use books::{RequestsReport, ResultsReport};
pub use describe::{
    Description, FieldSpec, MachineDescription, MachineVerbUse, ReportDescription, ReportName,
    describe,
};
pub use machines::{ActiveMachineLine, Frame, MachineStanding, MachinesOf, MachinesReport};
pub use pending::{
    ContinuationDetail, ContinuationLine, Dump, OfferLine, PendingLine, PendingReport,
    RulingsReport,
};
pub use pipeline::{
    ClarificationLine, PipelineReport, ProjectionDetail, ProjectionLine, ProjectionStanding,
    ProjectionWorkLine,
};
pub use queue::{InProgressLine, LaneDepth, OwnerQueueReport, SeatQueueLine};
pub use registry::{
    MachineSpecLine, PortLine, PortsReport, RegisteredMachine, RegisteredMachinesReport, WiredPorts,
};
pub use settings::{SettingLine, SettingsReport};
pub use table::{CastLine, SceneLine, SeatLine, TableReport, VitalsLine};
pub use verbs::{MachineSupport, Verb, VerbAccess, VerbEffect, VerbSpec};

/// Who a report is built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "audience", rename_all = "snake_case")]
pub enum InspectAudience {
    /// The developer or operator of this build: nothing is hidden.
    Operator,
    /// The table's host: administers the table and sees what the shared screen shows.
    Host,
    /// One seat's phone.
    Seat { seat: SeatId },
}

impl InspectAudience {
    /// The seat of a seat audience.
    pub fn seat(self) -> Option<SeatId> {
        match self {
            Self::Seat { seat } => Some(seat),
            Self::Operator | Self::Host => None,
        }
    }

    pub fn is_operator(self) -> bool {
        matches!(self, Self::Operator)
    }

    /// Whether the audience administers the table: the operator and the host.
    pub fn administers(self) -> bool {
        matches!(self, Self::Operator | Self::Host)
    }

    /// Whether the audience may read what belongs to `owner`: the owner and the operator.
    pub fn reads_private_to(self, owner: SeatId) -> bool {
        match self {
            Self::Operator => true,
            Self::Host => false,
            Self::Seat { seat } => seat == owner,
        }
    }
}

listed_enum! {
    /// Who a hidden value is for, which a redacted field names so a reader knows what to ask for.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Needs {
        /// Only the operator.
        Operator,
        /// The host or the operator.
        Host,
        /// The seat the value belongs to, or the operator.
        OwnSeat,
    }
}

impl Needs {
    pub fn key(self) -> &'static str {
        match self {
            Self::Operator => "operator",
            Self::Host => "host",
            Self::OwnSeat => "own_seat",
        }
    }
}

/// What an audience is told about a value: the value, or that it is hidden and from whom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Visible<T> {
    Shown(T),
    Redacted { needs: Needs },
}

impl<T> Visible<T> {
    /// The value, built only when `allowed`; otherwise it is never built.
    pub fn gated(allowed: bool, needs: Needs, make: impl FnOnce() -> T) -> Self {
        if allowed {
            Self::Shown(make())
        } else {
            Self::Redacted { needs }
        }
    }

    /// The value for an audience that meets `needs`, built only then. `OwnSeat` has no owner here,
    /// so only the operator meets it: use `owned_by` for a seat's own value.
    pub fn when(view: InspectView, needs: Needs, make: impl FnOnce() -> T) -> Self {
        let allowed = match needs {
            Needs::Operator => view.audience.is_operator(),
            Needs::Host => view.audience.administers(),
            Needs::OwnSeat => view.audience.is_operator(),
        };
        Self::gated(allowed, needs, make)
    }

    /// A value that belongs to `owner`: the owner and the operator see it.
    pub fn owned_by(view: InspectView, owner: SeatId, make: impl FnOnce() -> T) -> Self {
        Self::gated(view.audience.reads_private_to(owner), Needs::OwnSeat, make)
    }

    pub fn shown(&self) -> Option<&T> {
        match self {
            Self::Shown(value) => Some(value),
            Self::Redacted { .. } => None,
        }
    }

    pub fn is_shown(&self) -> bool {
        matches!(self, Self::Shown(_))
    }
}

impl<T: Serialize> Serialize for Visible<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(tag = "visibility", rename_all = "snake_case")]
        enum Wire<'a, T> {
            Shown { value: &'a T },
            Redacted { needs: Needs },
        }
        match self {
            Self::Shown(value) => Wire::Shown { value }.serialize(serializer),
            Self::Redacted { needs } => Wire::<T>::Redacted { needs: *needs }.serialize(serializer),
        }
    }
}

/// What a report is built for and when: the audience and the clock reading the caller took, so a
/// report that states an age never reads a clock itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InspectView {
    pub audience: InspectAudience,
    pub now_ms: u64,
}

impl InspectView {
    pub fn new(audience: InspectAudience, now_ms: u64) -> Self {
        Self { audience, now_ms }
    }
}

/// A module that holds state and can say what it holds. The report is typed, serialises as the
/// `--json` form of the CLI, is equal for equal state and view, and holds nothing the audience may
/// not see (see the module documentation).
pub trait Inspect {
    type Report: Serialize;

    fn inspect(&self, view: InspectView) -> Self::Report;
}

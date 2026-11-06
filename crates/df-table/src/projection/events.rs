//! The typed events of projection that are not already a variant of `TableEvent`.
//!
//! It owns `ProjectionEvent`, carried by `TableEvent::Projection` so projection's vocabulary has
//! one home and `event.rs` keeps to its size: an audience's view was built (`projection.built`).
//! It is class B, per-audience detail a host may leave out (H.14), and it holds only ids, counts,
//! closed keys, the length class of the newest line and the evidence reference of its specimen
//! (RES36): no text of any view can reach it. The rendering is an exhaustive `match`. It never
//! decides when a projection is recorded: the PROJECT stage does.

use std::fmt::{self, Write};

use serde::{Deserialize, Serialize};

use crate::event::field::LineWriter;
use crate::event::{EventClass, write_context};
use crate::identity::{Correlation, EvidenceRef, SeatId, TableScope};
use crate::private_data::LengthClass;
use crate::projection::AudienceKind;
use crate::projection::status::StatusKind;

/// One thing projection did that explains a turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "what", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectionEvent {
    /// A view was built for the shared screen or for one seat. `elements` counts the creatures it
    /// draws and `lines` the lines of story it shows; `length` is the newest line's length class.
    Built {
        scope: TableScope,
        correlation: Correlation,
        audience: AudienceKind,
        seat: Option<SeatId>,
        status: StatusKind,
        elements: usize,
        lines: usize,
        length: LengthClass,
        evidence: Option<EvidenceRef>,
    },
}

impl ProjectionEvent {
    pub fn class(&self) -> EventClass {
        match self {
            Self::Built { .. } => EventClass::Verbose,
        }
    }

    pub fn area(&self) -> &'static str {
        match self {
            Self::Built { .. } => "PROJECTION",
        }
    }

    pub(crate) fn write_audit_body<W: Write>(&self, out: &mut W) -> fmt::Result {
        match self {
            Self::Built {
                scope,
                correlation,
                audience,
                seat,
                status,
                elements,
                lines,
                length,
                evidence,
            } => {
                let mut line = LineWriter::start(out, "built")?;
                write_context(&mut line, scope, correlation)?;
                line.field("audience", audience)?;
                line.field_if_present("seat", seat.as_ref())?;
                line.field("status", status)?;
                line.field("elements", elements)?;
                line.field("lines", lines)?;
                line.field("length", length)?;
                line.field_if_present("evidence", evidence.as_ref())
            }
        }
    }
}

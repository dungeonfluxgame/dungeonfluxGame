//! The pipeline table's report: everything the pipeline holds beside the machines, in one value
//! (H.16 `dump`, `pending list`, `rulings list`, `projection show`, O.6).
//!
//! It composes the reports of the table state, the pending book, the stored rulings, the request
//! book and the result book, and adds what only the table holds: the host seat, each seat's open
//! clarification and one line per projection (`ProjectionRecord`, which is counts and classes and
//! holds no text). The state hash is the operator's, because it moves with every hidden fact.
//!
//! A projection line says what the build counted for one viewer. The shared screen's line is
//! public. A seat's own line is its own: what it counted of its private lines and facts, its
//! attachment version and its evidence reference belong to that seat and the operator, and the
//! work counter, which also counts the hidden creatures the build examined, is the operator's.
//! A host sees every viewer's status and element counts and nothing private. It builds each
//! projection from the table as it is, changes nothing and never reads a clock.

use serde::Serialize;

use crate::identity::{EvidenceRef, SeatId, SessionRevision, TableId};
use crate::inspect::{
    Inspect, InspectView, Needs, PendingReport, RequestsReport, ResultsReport, RulingsReport,
    TableReport, Visible,
};
use crate::pipeline::PipelineTable;
use crate::ports::{ActionId, Audience};
use crate::private_data::LengthClass;
use crate::projection::project;
use crate::projection::{AttachmentVersion, ProjectionRecord, ProjectionWork, StatusKind, Viewer};

/// What a seat was last asked to pick from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClarificationLine {
    pub seat: SeatId,
    pub options: Vec<ActionId>,
}

/// What one build examined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ProjectionWorkLine {
    pub cast_visited: usize,
    pub story_entries_visited: usize,
    pub facts_visited: usize,
}

impl From<ProjectionWork> for ProjectionWorkLine {
    fn from(work: ProjectionWork) -> Self {
        Self {
            cast_visited: work.cast_visited,
            story_entries_visited: work.story_entries_visited,
            facts_visited: work.facts_visited,
        }
    }
}

/// What a projection holds that belongs to its viewer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectionDetail {
    pub attachment_version: AttachmentVersion,
    pub lines_shown: usize,
    pub facts_shown: usize,
    /// The length class of the newest line, which is all a record says of it.
    pub newest_line_length: LengthClass,
    pub evidence: Option<EvidenceRef>,
}

/// How a projection came out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProjectionStanding {
    Built {
        revision: SessionRevision,
        status: StatusKind,
        elements_shown: usize,
        party_shown: usize,
        detail: Visible<ProjectionDetail>,
        work: Visible<ProjectionWorkLine>,
    },
    /// The seat is not at the table, so it has no view.
    Refused { seat: SeatId },
}

/// One viewer's projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectionLine {
    pub viewer: Viewer,
    pub standing: ProjectionStanding,
}

/// Everything the pipeline's table holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PipelineReport {
    pub table: TableId,
    pub host: Option<SeatId>,
    pub state_hash: Visible<u64>,
    pub facts: TableReport,
    pub pending: PendingReport,
    pub rulings: RulingsReport,
    pub requests: RequestsReport,
    pub results: ResultsReport,
    /// A seat sees only its own.
    pub clarifications: Vec<ClarificationLine>,
    /// The shared screen first, then each seat in seat order; a seat sees only its own.
    pub projections: Vec<ProjectionLine>,
}

fn projection_line(view: InspectView, record: &ProjectionRecord) -> ProjectionLine {
    let owner = record.seat();
    let detail = match owner {
        Some(seat) => Visible::owned_by(view, seat, || detail_of(record)),
        None => Visible::gated(true, Needs::Operator, || detail_of(record)),
    };
    ProjectionLine {
        viewer: record.viewer,
        standing: ProjectionStanding::Built {
            revision: record.revision,
            status: record.status,
            elements_shown: record.elements_shown,
            party_shown: record.party_shown,
            detail,
            work: Visible::when(view, Needs::Operator, || record.work.into()),
        },
    }
}

fn detail_of(record: &ProjectionRecord) -> ProjectionDetail {
    ProjectionDetail {
        attachment_version: record.attachment_version,
        lines_shown: record.lines_shown,
        facts_shown: record.facts_shown,
        newest_line_length: record.newest_line_length,
        evidence: record.evidence,
    }
}

fn projections(table: &PipelineTable, view: InspectView) -> Vec<ProjectionLine> {
    let snapshot = table.facts().snapshot();
    let attachments = table.attachments();
    let mut audiences = vec![Audience::Public];
    audiences.extend(
        snapshot
            .seats()
            .iter()
            .filter(|seat| view.audience.administers() || view.audience.seat() == Some(**seat))
            .map(|seat| Audience::Seat(*seat)),
    );
    audiences
        .into_iter()
        .map(|audience| match project(&snapshot, attachments, audience) {
            Ok(built) => projection_line(view, built.record()),
            Err(error) => {
                let crate::projection::ProjectionError::SeatNotAtTable { seat } = error;
                ProjectionLine {
                    viewer: Viewer::of(audience),
                    standing: ProjectionStanding::Refused { seat },
                }
            }
        })
        .collect()
}

impl Inspect for PipelineTable {
    type Report = PipelineReport;

    fn inspect(&self, view: InspectView) -> PipelineReport {
        let clarifications = self
            .facts()
            .seats()
            .iter()
            .filter(|seat| view.audience.administers() || view.audience.seat() == Some(**seat))
            .filter_map(|seat| {
                self.clarification_of(*seat)
                    .map(|options| ClarificationLine {
                        seat: *seat,
                        options: options.to_vec(),
                    })
            })
            .collect();
        PipelineReport {
            table: self.table_id(),
            host: self.host(),
            state_hash: Visible::when(view, Needs::Operator, || self.state_hash()),
            facts: self.facts().inspect(view),
            pending: self.pending().inspect(view),
            rulings: self.rulings().inspect(view),
            requests: self.requests().inspect(view),
            results: self.results().inspect(view),
            clarifications,
            projections: projections(self, view),
        }
    }
}

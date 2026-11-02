//! The diagnostic record of one projection: counts and classes, never text (RES36, H.14).
//!
//! It owns `ProjectionRecord`, what a projection says about itself for a log, a metric or the
//! inspect report: the audience, the revision and attachment version it was built at, the status,
//! how many elements, party members and lines it shows, the length class of its newest line, the
//! evidence reference of that line's specimen, and `ProjectionWork`, the count of what the build
//! examined, so a test proves that the work grows with the table and not with the square of it
//! (O.9 C5) without timing anything. The record holds no text, so it can be written to an event as
//! it is. It never decides what a view shows.

use crate::event::TableEvent;
use crate::identity::{Correlation, EvidenceRef, SeatId, SessionRevision, TableScope};
use crate::private_data::LengthClass;
use crate::projection::attachments::AttachmentVersion;
use crate::projection::events::ProjectionEvent;
use crate::projection::status::StatusKind;
use crate::projection::{AudienceKind, Viewer};

/// What one build examined, counted where it examines it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectionWork {
    pub cast_visited: usize,
    pub story_entries_visited: usize,
    pub facts_visited: usize,
}

impl ProjectionWork {
    /// All that was examined.
    pub fn total(self) -> usize {
        self.cast_visited + self.story_entries_visited + self.facts_visited
    }
}

/// What a projection says about itself, without any of its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectionRecord {
    pub viewer: Viewer,
    pub revision: SessionRevision,
    pub attachment_version: AttachmentVersion,
    pub status: StatusKind,
    pub elements_shown: usize,
    pub party_shown: usize,
    pub lines_shown: usize,
    pub facts_shown: usize,
    /// The length class of the newest line shown, which is all a log may say about it.
    pub newest_line_length: LengthClass,
    /// The specimen of the newest line in the diagnostic store, if one was kept.
    pub evidence: Option<EvidenceRef>,
    pub work: ProjectionWork,
}

impl ProjectionRecord {
    /// The seat the projection was built for, if it was built for one.
    pub fn seat(&self) -> Option<SeatId> {
        match self.viewer {
            Viewer::Shared => None,
            Viewer::Seat { seat } => Some(seat),
        }
    }

    /// The class B event that records this projection for the input that caused it.
    pub fn event(&self, scope: &TableScope, correlation: Correlation) -> TableEvent {
        TableEvent::Projection(ProjectionEvent::Built {
            scope: scope.clone(),
            correlation,
            audience: AudienceKind::of(self.viewer),
            seat: self.seat(),
            status: self.status,
            elements: self.elements_shown,
            lines: self.lines_shown,
            length: self.newest_line_length,
            evidence: self.evidence,
        })
    }
}

//! The request book's and the result book's reports: the resource requests that are out and how
//! many committed operations the table still remembers (RES22, H.2).
//!
//! Both are the host's administration: which requests a machine has outstanding and how many
//! operations are remembered say what the table is waiting on, so a seat sees neither. The request
//! book has no way to list its entries, so its entries are its saved record as data (`Dump`); the
//! result book holds one entry for every operation and is only counted. It never changes a book.

use serde::Serialize;

use crate::inspect::{Dump, Inspect, InspectView, Needs, Visible};
use crate::pipeline::{RequestBook, ResultBook};

/// The resource requests that are out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequestsReport {
    pub outstanding: Visible<usize>,
    /// How many were recorded with a commit and not issued to the broker yet.
    pub unissued: Visible<usize>,
    /// The saved record: each request's machine, key, purpose, stamp and whether it has a handle.
    pub entries: Visible<Dump>,
}

impl Inspect for RequestBook {
    type Report = RequestsReport;

    fn inspect(&self, view: InspectView) -> RequestsReport {
        RequestsReport {
            outstanding: Visible::when(view, Needs::Host, || self.len()),
            unissued: Visible::when(view, Needs::Host, || self.unissued().len()),
            entries: Visible::when(view, Needs::Host, || Dump::of(&self.to_record())),
        }
    }
}

/// The committed operations the table remembers, so a replayed operation id is answered without
/// asking a port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResultsReport {
    pub remembered: Visible<usize>,
}

impl Inspect for ResultBook {
    type Report = ResultsReport;

    fn inspect(&self, view: InspectView) -> ResultsReport {
        ResultsReport {
            remembered: Visible::when(view, Needs::Host, || self.len()),
        }
    }
}

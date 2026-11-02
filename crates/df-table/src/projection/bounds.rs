//! The named sizes of projection, in one place.
//!
//! The owner's rule is that there are no small invented caps: a limit is a very large named
//! constant and memory stays bounded by design. The numbers that follow the plan or the wire are
//! stated as such; the rest are huge on purpose and scoped down when a measurement says so
//! (owner, 2026-11-02).

use crate::owner_queue::MAXIMUM_SEATS;

/// How many lines a phone shows: "reconnect resyncs the projection and the last 20 narration lines"
/// (H.4). The plan's number, and a window over the story, not a limit on it. A decline's reason
/// counts as one of the lines.
pub const PHONE_NARRATION_LINES: usize = 20;

/// The most one encoded view may take on the wire, in bytes: the bridge's 65,536 byte message
/// limit with the headroom `df-api` keeps (`MAXIMUM_MESSAGE_BYTES` there, 60 KiB). The text of a
/// view is held well below it so its structure always fits (#1686).
pub const WIRE_FRAME_BYTES: usize = 60 * 1024;

/// All the text one view may carry, in bytes: three quarters of the wire frame, which leaves the
/// rest for ids, numbers, the party strip and the framing.
pub const VIEW_TEXT_BUDGET_BYTES: usize = WIRE_FRAME_BYTES / 4 * 3;

/// The longest single text that is published (a line, a readout, a place, a fact, a decline), in
/// bytes: an eighth of the view's text budget, so the mandatory texts of a view and the newest line
/// and fact always fit the wire frame together. A longer text is refused when it is published, with
/// a typed error.
pub const MAXIMUM_TEXT_BYTES: usize = VIEW_TEXT_BUDGET_BYTES / 8;

/// The share of the view's text budget that the phone's story may take, in bytes. The newest lines
/// come first and older ones are left off, and the count left off is typed data. The newest entry
/// is always shown.
pub const NARRATION_BUDGET_BYTES: usize = VIEW_TEXT_BUDGET_BYTES / 6;

/// The share of the view's text budget that the private note may take, in bytes. The newest facts
/// come first and older ones are left off, and the count left off is typed data. The newest fact is
/// always shown.
pub const NOTE_BUDGET_BYTES: usize = VIEW_TEXT_BUDGET_BYTES / 6;

/// The most text the offers of one seat may hold together, in bytes, checked when they are
/// published: an offer is never dropped from a view, so the owner is told when they do not fit.
pub const OFFERS_BUDGET_BYTES: usize = VIEW_TEXT_BUDGET_BYTES / 6;

/// Most story entries one queue keeps (the public one, and one per seat); the oldest is dropped
/// first. Deliberately huge so it never limits play.
pub const MAXIMUM_STORY_ENTRIES: usize = 1_000_000;

/// Most facts one seat knows at once. Deliberately huge so it never limits play.
pub const MAXIMUM_FACTS_PER_SEAT: usize = 1_000_000;

/// Most offers one seat holds at once. Deliberately huge so it never limits play.
pub const MAXIMUM_OFFERS_PER_SEAT: usize = 1_000_000;

/// Most seats that have a story, facts, a readout, offers or a notice: a seat has them only while
/// it is at the table, so the table's own bound is the bound here.
pub const MAXIMUM_SEATS_WITH_ENTRIES: usize = MAXIMUM_SEATS;

/// Most scenes that have a place name: a scene needs a seat, so the table's bound again.
pub const MAXIMUM_PLACES: usize = MAXIMUM_SEATS;

/// Most machines that have a state label: the registry is small, and this is far above it.
pub const MAXIMUM_STATE_LABELS: usize = 1_000_000;

/// The width and height of a creature's figure on the stage, in canvas units, until the kinds say
/// otherwise (the canvas's `character` kind draws a 6 unit circle).
pub const FIGURE_SIZE_UNITS: f32 = 6.0;

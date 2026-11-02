//! The named sizes of the table state, in one place.
//!
//! The owner's rule is that there are no small invented caps: a limit is a very large named
//! constant and memory stays bounded by design (a scene needs a seat, a seat needs a place at the
//! table, the hand-off stack has the plan's own depth). A value is scoped down later when a
//! measurement says so (owner, 2026-11-02).

/// Most cast entries a table holds at once, over all its scenes. Deliberately huge so it never
/// limits play; scope it down later.
pub const MAXIMUM_CAST_ENTRIES: usize = 1_000_000;

/// Most conditions one cast entry carries. Deliberately huge so it never limits play.
pub const MAXIMUM_CONDITIONS_PER_ENTRY: usize = 1_000_000;

/// The depth of a scene's hand-off stack: the plan's own number (RES27, O.7), not an invented cap.
/// A hand-off that would push a fourth frame is refused and the machine keeps asking.
pub const MAXIMUM_HAND_OFF_DEPTH: usize = 3;

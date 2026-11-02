//! The one rule for who has a lane: the table state is the membership truth (#1691).
//!
//! It owns `LaneChange` and `reconcile_lanes`, the explicit mapping between the two seat registries
//! of the crate: the table state's seats (`SeatJoined`, `SeatLeft`) and the owner queue's lanes
//! (`join_seat`, `leave_seat`). A seat belongs to the table when a commit says so, and the pipeline
//! asks `reconcile_lanes` after every commit and opens or closes lanes to match, so a lane never
//! outlives its seat and a seat never sits without one. The function compares the two sets and does
//! not read the effects, so a seat that joined the queue by another road is closed and a seat that
//! joined the table without a lane is opened, whatever commit caused it. It never touches the
//! queue: the caller runs the changes, closes before it opens so a freed lane is there to reuse,
//! and records what `leave_seat` gives back. Presence (`SeatPresenceChanged`) has no lane: a
//! sleeping or away seat keeps its lane and its place.

use std::collections::BTreeSet;

use crate::identity::SeatId;
use crate::table_state::snapshot::TableSnapshot;

/// A change to the owner queue's lanes that the table's seats call for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneChange {
    /// The seat is at the table and has no lane.
    Open(SeatId),
    /// The seat has a lane and is not at the table.
    Close(SeatId),
}

/// What must change in the lanes so that exactly the table's seats have one: every closing first,
/// then every opening, each in seat order.
pub fn reconcile_lanes(table: &TableSnapshot, lane_seats: &BTreeSet<SeatId>) -> Vec<LaneChange> {
    let closing = lane_seats
        .iter()
        .filter(|seat| table.seat(**seat).is_none())
        .map(|seat| LaneChange::Close(*seat));
    let opening = table
        .seats()
        .iter()
        .filter(|seat| !lane_seats.contains(seat))
        .map(|seat| LaneChange::Open(*seat));
    closing.chain(opening).collect()
}

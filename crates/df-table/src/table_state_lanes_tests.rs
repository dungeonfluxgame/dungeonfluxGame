//! The table state is the membership truth and the owner queue's lanes follow it (#1691): the
//! mapping is tested against the real queue.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::effects::TableEffect;
use crate::identity::{SeatId, TableId};
use crate::owner_queue::OwnerQueue;
use crate::owner_queue_tests::{NullSink, settings};
use crate::table_state::{LaneChange, TableState, reconcile_lanes};
use crate::table_state_tests::{apply, seat};

fn queue() -> OwnerQueue {
    OwnerQueue::new(TableId::new(1), &settings(3, 3, 1), Arc::new(NullSink))
        .expect("settings are valid")
}

/// Runs what the table asks of the lanes, closing first, as the pipeline does after a commit.
fn follow(table: &TableState, queue: &mut OwnerQueue) -> Vec<LaneChange> {
    let changes = reconcile_lanes(&table.snapshot(), &queue.seated());
    for change in &changes {
        match change {
            LaneChange::Open(seat) => queue.join_seat(*seat, 0).expect("the lane opens"),
            LaneChange::Close(seat) => {
                queue.leave_seat(*seat, 0).expect("the lane closes");
            }
        }
    }
    changes
}

fn table_seats(table: &TableState) -> BTreeSet<SeatId> {
    table.seats().to_set()
}

#[test]
fn a_seat_that_joins_the_table_gets_a_lane_and_a_seat_that_leaves_loses_it() {
    let mut table = TableState::new(4).expect("valid table");
    let mut lanes = queue();
    assert!(follow(&table, &mut lanes).is_empty());
    apply(&mut table, 1, &[TableEffect::SeatJoined { seat: seat(1) }]).unwrap();
    apply(&mut table, 2, &[TableEffect::SeatJoined { seat: seat(2) }]).unwrap();
    assert_eq!(
        follow(&table, &mut lanes),
        [LaneChange::Open(seat(1)), LaneChange::Open(seat(2))]
    );
    assert_eq!(lanes.seated(), table_seats(&table));
    apply(&mut table, 3, &[TableEffect::SeatLeft { seat: seat(1) }]).unwrap();
    assert_eq!(follow(&table, &mut lanes), [LaneChange::Close(seat(1))]);
    assert_eq!(lanes.seated(), table_seats(&table));
    assert!(
        follow(&table, &mut lanes).is_empty(),
        "nothing is left to do"
    );
}

#[test]
fn a_join_and_a_leave_in_one_commit_leave_nothing_to_do() {
    let mut table = TableState::new(4).expect("valid table");
    let mut lanes = queue();
    apply(
        &mut table,
        1,
        &[
            TableEffect::SeatJoined { seat: seat(1) },
            TableEffect::SeatLeft { seat: seat(1) },
        ],
    )
    .unwrap();
    assert!(follow(&table, &mut lanes).is_empty());
    assert!(lanes.seated().is_empty());
}

#[test]
fn a_lane_opened_by_another_road_is_closed_and_a_seat_with_no_lane_is_opened() {
    let table = TableState::seated(4, &[seat(1), seat(2)]).expect("valid table");
    let mut lanes = queue();
    lanes.join_seat(seat(2), 0).unwrap();
    lanes.join_seat(seat(7), 0).unwrap();
    assert_eq!(
        follow(&table, &mut lanes),
        [LaneChange::Close(seat(7)), LaneChange::Open(seat(1))],
        "closings come first, then openings"
    );
    assert_eq!(lanes.seated(), table_seats(&table));
}

#[test]
fn a_sleeping_or_away_seat_keeps_its_lane() {
    use crate::table_state::Presence;
    let mut table = TableState::seated(4, &[seat(1)]).expect("valid table");
    let mut lanes = queue();
    follow(&table, &mut lanes);
    for (turn, presence) in (1..).zip([Presence::Sleeping, Presence::VoluntaryAfk]) {
        apply(
            &mut table,
            turn,
            &[TableEffect::SeatPresenceChanged {
                seat: seat(1),
                presence,
            }],
        )
        .unwrap();
        assert!(follow(&table, &mut lanes).is_empty(), "{presence:?}");
    }
    assert_eq!(lanes.seated(), table_seats(&table));
}

#[test]
fn a_seat_that_left_and_came_back_has_a_fresh_lane_with_no_history() {
    let mut table = TableState::seated(4, &[seat(1)]).expect("valid table");
    let mut lanes = queue();
    follow(&table, &mut lanes);
    apply(&mut table, 1, &[TableEffect::SeatLeft { seat: seat(1) }]).unwrap();
    follow(&table, &mut lanes);
    apply(&mut table, 2, &[TableEffect::SeatJoined { seat: seat(1) }]).unwrap();
    assert_eq!(follow(&table, &mut lanes), [LaneChange::Open(seat(1))]);
    assert_eq!(lanes.seat_pending(seat(1)), Some(0));
}

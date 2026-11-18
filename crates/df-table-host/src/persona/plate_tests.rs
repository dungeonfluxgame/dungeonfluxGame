//! Which cast a frame goes up with: its own only when it names everyone it was asked for and
//! everyone of the room it replaces.

use super::plate::keeps_own_cast;

const ROOM: [&str; 3] = ["hero-10", "hero-9", "baker"];

#[test]
fn a_close_up_of_one_keeps_the_rooms_cast() {
    // Asked to draw one hero and naming them: still short of the room's other hero and the baker.
    assert!(!keeps_own_cast((1, 1), Some(&ROOM), &["hero-9"]));
}

#[test]
fn a_frame_naming_the_whole_room_keeps_its_own_cast() {
    assert!(keeps_own_cast(
        (3, 3),
        Some(&ROOM),
        &["baker", "hero-9", "hero-10"]
    ));
    // Someone more than the room (a passer-by drawn in) does not shrink it.
    assert!(keeps_own_cast(
        (4, 4),
        Some(&ROOM),
        &["baker", "hero-9", "hero-10", "carter"]
    ));
}

#[test]
fn a_frame_that_missed_someone_it_was_asked_for_keeps_the_rooms_cast() {
    assert!(!keeps_own_cast((2, 3), Some(&ROOM), &ROOM));
}

#[test]
fn a_plate_that_replaces_no_room_keeps_its_own_cast_when_complete() {
    assert!(keeps_own_cast((2, 2), None, &["hero-9", "hero-10"]));
    assert!(!keeps_own_cast((1, 2), None, &["hero-9"]));
}

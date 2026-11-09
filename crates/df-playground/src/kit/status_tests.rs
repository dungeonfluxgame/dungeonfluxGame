//! The status banner's words and blocking.

use crate::fixture::Status;
use crate::kit::status::StatusChip;

#[test]
fn all_well_draws_nothing_and_a_blocking_status_is_marked() {
    assert_eq!(StatusChip { status: Status::Ok }.text(), None);
    let telling = StatusChip {
        status: Status::Telling,
    };
    assert_eq!(telling.text().as_deref(), Some("Telling it"));
    assert!(!telling.blocks());
    let reconnecting = StatusChip {
        status: Status::Reconnecting,
    };
    assert!(reconnecting.blocks());
}

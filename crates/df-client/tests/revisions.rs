use std::cell::Cell;
use std::rc::Rc;

use df_client::revisions::{ViewAcceptance, ViewStore};
use df_types::{ClientBindingId, RecoveryEpoch, SessionRevision};

fn binding(value: u8) -> ClientBindingId {
    ClientBindingId::from_bytes(&[value; 16]).expect("nonzero test binding")
}

fn revision(epoch: u64, sequence: u64) -> SessionRevision {
    SessionRevision::new(
        RecoveryEpoch::new(epoch).expect("nonzero test epoch"),
        sequence,
    )
}

#[test]
fn first_complete_snapshot_is_applied() {
    let mut store = ViewStore::new(binding(1));
    assert_eq!(store.current(), None);
    assert_eq!(
        store.accept(binding(1), revision(1, 0), "lobby"),
        ViewAcceptance::Applied
    );
    assert_eq!(store.current(), Some((revision(1, 0), &"lobby")));
}

#[test]
fn newer_same_epoch_complete_snapshot_replaces_current_without_contiguous_sequence() {
    let mut store = ViewStore::new(binding(1));
    assert_eq!(
        store.accept(binding(1), revision(3, 4), "before"),
        ViewAcceptance::Applied
    );
    assert_eq!(
        store.accept(binding(1), revision(3, 99), "after"),
        ViewAcceptance::Applied
    );
    assert_eq!(store.current(), Some((revision(3, 99), &"after")));
}

#[test]
fn older_same_epoch_snapshot_cannot_regress_display() {
    let mut store = ViewStore::new(binding(1));
    assert_eq!(
        store.accept(binding(1), revision(3, 4), "current"),
        ViewAcceptance::Applied
    );
    assert_eq!(
        store.accept(binding(1), revision(3, 3), "obsolete"),
        ViewAcceptance::Stale {
            current: revision(3, 4)
        }
    );
    assert_eq!(store.current(), Some((revision(3, 4), &"current")));
}

#[test]
fn equal_revision_with_different_payload_keeps_current_view() {
    let mut store = ViewStore::new(binding(1));
    assert_eq!(
        store.accept(binding(1), revision(3, 4), "current"),
        ViewAcceptance::Applied
    );
    assert_eq!(
        store.accept(binding(1), revision(3, 4), "conflicting"),
        ViewAcceptance::Duplicate {
            current: revision(3, 4)
        }
    );
    assert_eq!(store.current(), Some((revision(3, 4), &"current")));
}

#[test]
fn newer_recovery_epoch_accepts_smaller_sequence() {
    let mut store = ViewStore::new(binding(1));
    assert_eq!(
        store.accept(binding(1), revision(3, u64::MAX), "old run"),
        ViewAcceptance::Applied
    );
    assert_eq!(
        store.accept(binding(1), revision(4, 0), "recovered"),
        ViewAcceptance::Applied
    );
    assert_eq!(store.current(), Some((revision(4, 0), &"recovered")));
}

#[test]
fn older_epoch_with_maximum_sequence_cannot_regress_recovered_display() {
    let mut store = ViewStore::new(binding(1));
    assert_eq!(
        store.accept(binding(1), revision(4, 0), "recovered"),
        ViewAcceptance::Applied
    );
    assert_eq!(
        store.accept(binding(1), revision(3, u64::MAX), "old run"),
        ViewAcceptance::Stale {
            current: revision(4, 0)
        }
    );
    assert_eq!(store.current(), Some((revision(4, 0), &"recovered")));
}

#[test]
fn wrong_binding_cannot_seed_or_replace_current_snapshot() {
    let mut store = ViewStore::new(binding(1));
    assert_eq!(
        store.accept(binding(2), revision(8, 0), "private"),
        ViewAcceptance::WrongBinding
    );
    assert_eq!(store.current(), None);
    assert_eq!(
        store.accept(binding(1), revision(3, 4), "permitted"),
        ViewAcceptance::Applied
    );
    assert_eq!(
        store.accept(binding(2), revision(8, 0), "private"),
        ViewAcceptance::WrongBinding
    );
    assert_eq!(store.current(), Some((revision(3, 4), &"permitted")));
}

#[derive(Debug)]
struct OwnedView {
    displayed_sequence: u64,
    disposals: Rc<Cell<usize>>,
}

impl Drop for OwnedView {
    fn drop(&mut self) {
        self.disposals.set(self.disposals.get() + 1);
    }
}

#[test]
fn repeated_same_phase_updates_keep_one_view_and_dispose_each_rejected_or_replaced_input() {
    let disposals = Rc::new(Cell::new(0));
    let mut store = ViewStore::new(binding(1));
    for sequence in 0..128 {
        let view = OwnedView {
            displayed_sequence: sequence,
            disposals: Rc::clone(&disposals),
        };
        assert_eq!(
            store.accept(binding(1), revision(1, sequence), view),
            ViewAcceptance::Applied
        );
        assert_eq!(disposals.get(), sequence as usize);
        let (accepted, shown) = store.current().expect("accepted complete snapshot");
        assert_eq!(accepted, revision(1, sequence));
        assert_eq!(shown.displayed_sequence, sequence);
    }
    let duplicate = OwnedView {
        displayed_sequence: 999,
        disposals: Rc::clone(&disposals),
    };
    assert_eq!(
        store.accept(binding(1), revision(1, 127), duplicate),
        ViewAcceptance::Duplicate {
            current: revision(1, 127)
        }
    );
    assert_eq!(disposals.get(), 128);
    let stale = OwnedView {
        displayed_sequence: 0,
        disposals: Rc::clone(&disposals),
    };
    assert_eq!(
        store.accept(binding(1), revision(1, 0), stale),
        ViewAcceptance::Stale {
            current: revision(1, 127)
        }
    );
    assert_eq!(disposals.get(), 129);
    let wrong = OwnedView {
        displayed_sequence: 999,
        disposals: Rc::clone(&disposals),
    };
    assert_eq!(
        store.accept(binding(2), revision(99, 0), wrong),
        ViewAcceptance::WrongBinding
    );
    assert_eq!(disposals.get(), 130);
    assert_eq!(
        store.current().expect("current view").1.displayed_sequence,
        127
    );
    drop(store);
    assert_eq!(disposals.get(), 131);
}

#[test]
fn a_presentation_amendment_needs_the_same_binding_and_the_exact_revision() {
    let mut store = ViewStore::new(binding(1));
    store.accept(binding(1), revision(3, 4), String::from("plain"));
    let paint = |view: &mut String| {
        view.push_str(" painted");
        true
    };
    assert!(!store.amend_presentation(binding(2), revision(3, 4), paint));
    assert!(!store.amend_presentation(binding(1), revision(3, 5), paint));
    assert!(!store.amend_presentation(binding(1), revision(2, 4), paint));
    assert_eq!(
        store.current(),
        Some((revision(3, 4), &String::from("plain")))
    );
    assert!(store.amend_presentation(binding(1), revision(3, 4), paint));
    assert_eq!(
        store.current(),
        Some((revision(3, 4), &String::from("plain painted")))
    );
    // The revision watermark is untouched: an older snapshot is still stale.
    assert_eq!(
        store.accept(binding(1), revision(3, 3), String::from("old")),
        ViewAcceptance::Stale {
            current: revision(3, 4)
        }
    );
    // And an equal revision still cannot replace through the ordinary path.
    assert_eq!(
        store.accept(binding(1), revision(3, 4), String::from("other")),
        ViewAcceptance::Duplicate {
            current: revision(3, 4)
        }
    );
}

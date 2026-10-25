use df_types::{RecoveryEpoch, RevisionError, SessionRevision};

#[test]
fn sequence_advances_monotonically_within_each_epoch_boundary() {
    for epoch_value in [1, 8, u64::MAX] {
        let epoch = RecoveryEpoch::new(epoch_value).unwrap();

        for sequence in [0, 1, u64::MAX - 1] {
            let revision = SessionRevision::new(epoch, sequence);
            let next_revision = revision.next_sequence().unwrap();

            assert_eq!(next_revision.epoch(), epoch);
            assert_eq!(next_revision.sequence(), sequence + 1);
            assert!(next_revision > revision);
        }

        let maximum_sequence_revision = SessionRevision::new(epoch, u64::MAX);
        assert_eq!(
            maximum_sequence_revision.next_sequence(),
            Err(RevisionError::SequenceOverflow)
        );
        assert_eq!(
            maximum_sequence_revision.next_sequence(),
            Err(RevisionError::SequenceOverflow)
        );
        assert_eq!(maximum_sequence_revision.epoch(), epoch);
        assert_eq!(maximum_sequence_revision.sequence(), u64::MAX);
    }
}

#[test]
fn epoch_order_prevents_an_earlier_revision_after_sequence_exhaustion() {
    let earlier_epoch = RecoveryEpoch::new(7).unwrap();
    let later_epoch = RecoveryEpoch::new(8).unwrap();
    let earlier_epoch_maximum = SessionRevision::new(earlier_epoch, u64::MAX);
    let later_epoch_initial = SessionRevision::new(later_epoch, 0);
    let later_epoch_second = later_epoch_initial.next_sequence().unwrap();

    assert!(later_epoch_initial > earlier_epoch_maximum);
    assert!(later_epoch_second > later_epoch_initial);
    assert_eq!(later_epoch_second.epoch(), later_epoch);
    assert_eq!(later_epoch_second.sequence(), 1);
}

#[test]
fn zero_epoch_is_rejected_and_maximum_epoch_is_a_valid_supplied_value() {
    assert_eq!(RecoveryEpoch::new(0), Err(RevisionError::ZeroEpoch));

    let maximum_epoch = RecoveryEpoch::new(u64::MAX).unwrap();
    let initial_revision = SessionRevision::new(maximum_epoch, 0);
    let next_revision = initial_revision.next_sequence().unwrap();

    assert_eq!(initial_revision.epoch().get(), u64::MAX);
    assert_eq!(next_revision.epoch(), maximum_epoch);
    assert_eq!(next_revision.sequence(), 1);
}

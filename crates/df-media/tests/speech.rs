use df_media::schedule::{ScheduleError, ScheduleLimits};
use df_media::speech::{
    SpeechAdmissionRefusal, SpeechError, SpeechIdentity, SpeechLimits, SpeechReceipt,
    SpeechScheduler, SpeechStopReason,
};
use df_model::checkpoint::{Basis, JobId, NativeFailure};
use df_types::{OperationId, RecoveryEpoch, RunId, SessionId, SessionRevision};

fn basis(epoch: u64, sequence: u64) -> Basis {
    Basis {
        session: SessionId::from_bytes(&[1; 16]).unwrap(),
        run: RunId::from_bytes(&[2; 16]).unwrap(),
        revision: SessionRevision::new(RecoveryEpoch::new(epoch).unwrap(), sequence),
    }
}

fn identity(value: u8) -> SpeechIdentity {
    SpeechIdentity {
        basis: basis(1, 7),
        job: JobId::from_bytes(&[value; 16]).unwrap(),
        operation: OperationId::from_bytes(&[8; 16]).unwrap(),
        generation: 11,
    }
}

fn scheduler() -> SpeechScheduler {
    SpeechScheduler::new(
        basis(1, 7),
        ScheduleLimits {
            queue_items: 3,
            queue_bytes: 8,
            speech_items: 1,
            speech_bytes: 1,
            execution_slots: 2,
            speech_slots: 1,
        },
        SpeechLimits {
            maximum_chunks: 3,
            maximum_bytes: 6,
        },
    )
    .unwrap()
}

fn start(scheduler: &mut SpeechScheduler, identity: SpeechIdentity) -> SpeechReceipt {
    scheduler.admit(identity, Box::from([7])).unwrap();
    scheduler.begin().unwrap().unwrap()
}

fn queue(scheduler: &mut SpeechScheduler, receipt: &SpeechReceipt, ordinal: u64, bytes: &[u8]) {
    scheduler
        .queue_chunk(receipt, receipt.identity(), ordinal, bytes.into())
        .unwrap();
}

fn close(scheduler: &mut SpeechScheduler, receipt: &SpeechReceipt, count: u64) {
    scheduler.end(receipt, receipt.identity(), count).unwrap();
    scheduler.eof(receipt, receipt.identity()).unwrap();
}

#[test]
fn actual_dispatch_drains_ordered_tail_before_one_completion_and_returns_original_command() {
    let mut scheduler = scheduler();
    let receipt = start(&mut scheduler, identity(3));
    assert_eq!(scheduler.snapshot().schedule.active_dispatches, 1);
    assert_eq!(scheduler.snapshot().schedule.active_bytes, 1);
    queue(&mut scheduler, &receipt, 2, &[3, 3]);
    queue(&mut scheduler, &receipt, 0, &[1, 1]);
    queue(&mut scheduler, &receipt, 1, &[2, 2]);
    assert!(scheduler.next(&receipt).unwrap().is_none());
    assert_eq!(
        scheduler.finish(&receipt).unwrap_err(),
        SpeechError::Waiting
    );
    close(&mut scheduler, &receipt, 3);
    for ordinal in 0..3 {
        let chunk = scheduler.next(&receipt).unwrap().unwrap();
        assert_eq!(chunk.sequence, ordinal);
        assert_eq!(chunk.bytes, &[ordinal as u8 + 1; 2]);
        assert_eq!(
            scheduler.snapshot().buffered_bytes,
            6 - ordinal as usize * 2
        );
        assert_eq!(
            scheduler.finish(&receipt).unwrap_err(),
            SpeechError::Waiting
        );
        scheduler.acknowledge(&receipt, ordinal).unwrap();
    }
    let request = scheduler.finish(&receipt).unwrap();
    assert_eq!(*request.id(), identity(3).job);
    assert_eq!(request.operation(), identity(3).operation);
    assert_eq!(request.payload(), &[7]);
    let snapshot = scheduler.snapshot();
    assert_eq!(snapshot.buffered_bytes, 0);
    assert_eq!(snapshot.buffered_chunks, 0);
    assert_eq!(snapshot.schedule.active_dispatches, 0);
    assert_eq!(snapshot.schedule.completed_dispatches, 1);
    assert_eq!(scheduler.finish(&receipt).unwrap_err(), SpeechError::Stale);
}

#[test]
fn overtaking_end_waits_for_exact_frames_and_clean_eof() {
    let mut scheduler = scheduler();
    let receipt = start(&mut scheduler, identity(3));
    scheduler.end(&receipt, receipt.identity(), 2).unwrap();
    queue(&mut scheduler, &receipt, 1, &[2]);
    assert!(scheduler.next(&receipt).unwrap().is_none());
    queue(&mut scheduler, &receipt, 0, &[1]);
    assert!(scheduler.next(&receipt).unwrap().is_none());
    scheduler.eof(&receipt, receipt.identity()).unwrap();
    assert_eq!(scheduler.next(&receipt).unwrap().unwrap().bytes, &[1]);
    scheduler.acknowledge(&receipt, 0).unwrap();
    assert_eq!(scheduler.next(&receipt).unwrap().unwrap().bytes, &[2]);
    scheduler.acknowledge(&receipt, 1).unwrap();
    scheduler.finish(&receipt).unwrap();
    assert_eq!(scheduler.snapshot().schedule.completed_dispatches, 1);
}

#[test]
fn missing_end_or_gapped_eof_keeps_partial_bytes_owned_and_never_completes() {
    for with_end in [false, true] {
        let mut scheduler = scheduler();
        let receipt = start(&mut scheduler, identity(3));
        queue(&mut scheduler, &receipt, 1, &[2]);
        if with_end {
            scheduler.end(&receipt, receipt.identity(), 2).unwrap();
        }
        assert_eq!(
            scheduler.eof(&receipt, receipt.identity()),
            Err(SpeechError::IncompleteEof)
        );
        assert!(matches!(
            scheduler.next(&receipt),
            Err(SpeechError::Failed(NativeFailure::Rejected))
        ));
        assert_eq!(scheduler.snapshot().buffered_bytes, 1);
        assert_eq!(scheduler.snapshot().schedule.active_dispatches, 1);
        assert_eq!(scheduler.snapshot().schedule.completed_dispatches, 0);
        let stopped = scheduler
            .stop(&receipt, SpeechStopReason::Cancelled)
            .unwrap();
        assert_eq!(
            stopped.reason,
            SpeechStopReason::Failed(NativeFailure::Rejected)
        );
        assert_eq!((stopped.discarded_chunks, stopped.discarded_bytes), (1, 1));
        assert_eq!(scheduler.snapshot().schedule.failed_dispatches, 1);
        assert_eq!(scheduler.snapshot().schedule.completed_dispatches, 0);
    }
}

#[test]
fn borrowed_and_unacknowledged_chunks_share_global_budget_until_consumed() {
    let mut scheduler = scheduler();
    let first = start(&mut scheduler, identity(3));
    let second = start(&mut scheduler, identity(4));
    queue(&mut scheduler, &first, 0, &[1; 6]);
    close(&mut scheduler, &first, 1);
    let pointer = scheduler.next(&first).unwrap().unwrap().bytes.as_ptr();
    assert_eq!(
        scheduler.next(&first).unwrap().unwrap().bytes.as_ptr(),
        pointer
    );
    assert_eq!(scheduler.snapshot().buffered_bytes, 6);
    let payload: Box<[u8]> = Box::from([2]);
    let address = payload.as_ptr();
    let refusal = scheduler
        .queue_chunk(&second, second.identity(), 0, payload)
        .unwrap_err();
    assert_eq!(refusal.reason, SpeechError::ByteCapacity);
    assert_eq!(refusal.bytes.as_ptr(), address);
    assert_eq!(
        scheduler.acknowledge(&first, 1),
        Err(SpeechError::WrongAcknowledgement)
    );
    assert_eq!(scheduler.snapshot().buffered_bytes, 6);
    scheduler.acknowledge(&first, 0).unwrap();
    queue(&mut scheduler, &second, 0, &[2]);
    scheduler.finish(&first).unwrap();
    scheduler
        .stop(&second, SpeechStopReason::Cancelled)
        .unwrap();
}

#[test]
fn empty_chunks_consume_global_item_budget_and_overflow_retains_bytes() {
    let mut scheduler = scheduler();
    let first = start(&mut scheduler, identity(3));
    let second = start(&mut scheduler, identity(4));
    for sequence in 0..3 {
        queue(&mut scheduler, &first, sequence, &[]);
    }
    let refusal = scheduler
        .queue_chunk(&second, second.identity(), 0, Box::from([2]))
        .unwrap_err();
    assert_eq!(refusal.reason, SpeechError::ChunkCapacity);
    assert_eq!(&*refusal.bytes, &[2]);
    assert_eq!(scheduler.snapshot().buffered_chunks, 3);
    let stopped = scheduler.stop(&first, SpeechStopReason::Cancelled).unwrap();
    assert_eq!((stopped.discarded_chunks, stopped.discarded_bytes), (3, 0));
    queue(&mut scheduler, &second, 0, &[2]);
    scheduler
        .stop(&second, SpeechStopReason::Cancelled)
        .unwrap();
}

#[test]
fn duplicate_trailing_and_unbounded_ordinals_never_replace_terminal_tail() {
    let mut scheduler = scheduler();
    let receipt = start(&mut scheduler, identity(3));
    queue(&mut scheduler, &receipt, 0, &[1]);
    assert_eq!(
        scheduler
            .queue_chunk(&receipt, receipt.identity(), 0, Box::from([9]))
            .unwrap_err()
            .reason,
        SpeechError::DuplicateChunk
    );
    assert_eq!(
        scheduler
            .queue_chunk(&receipt, receipt.identity(), u64::MAX, Box::from([9]))
            .unwrap_err()
            .reason,
        SpeechError::SequenceRange
    );
    assert_eq!(
        scheduler.end(&receipt, receipt.identity(), 0),
        Err(SpeechError::SequenceRange)
    );
    assert_eq!(
        scheduler.end(&receipt, receipt.identity(), u64::MAX),
        Err(SpeechError::SequenceRange)
    );
    scheduler.end(&receipt, receipt.identity(), 1).unwrap();
    assert_eq!(
        scheduler.end(&receipt, receipt.identity(), 1),
        Err(SpeechError::DuplicateEnd)
    );
    assert_eq!(
        scheduler
            .queue_chunk(&receipt, receipt.identity(), 1, Box::from([9]))
            .unwrap_err()
            .reason,
        SpeechError::SequenceRange
    );
    scheduler.eof(&receipt, receipt.identity()).unwrap();
    assert_eq!(
        scheduler
            .queue_chunk(&receipt, receipt.identity(), 0, Box::from([9]))
            .unwrap_err()
            .reason,
        SpeechError::Closed
    );
    assert_eq!(
        scheduler.eof(&receipt, receipt.identity()),
        Err(SpeechError::Closed)
    );
    assert_eq!(scheduler.next(&receipt).unwrap().unwrap().bytes, &[1]);
    scheduler.acknowledge(&receipt, 0).unwrap();
    scheduler.finish(&receipt).unwrap();
}

#[test]
fn cancellation_disposes_owned_buffers_and_old_receipts_cannot_release_reused_ids() {
    let mut scheduler = scheduler();
    let old = start(&mut scheduler, identity(3));
    queue(&mut scheduler, &old, 0, &[9]);
    close(&mut scheduler, &old, 1);
    let stopped = scheduler.stop(&old, SpeechStopReason::Cancelled).unwrap();
    assert_eq!(stopped.identity, identity(3));
    assert_eq!(stopped.discarded_bytes, 1);
    assert_eq!(scheduler.snapshot().schedule.cancelled_dispatches, 1);
    let current = start(&mut scheduler, identity(3));
    queue(&mut scheduler, &current, 0, &[1]);
    assert!(matches!(scheduler.next(&old), Err(SpeechError::Stale)));
    assert_eq!(scheduler.acknowledge(&old, 0), Err(SpeechError::Stale));
    assert!(matches!(
        scheduler.stop(&old, SpeechStopReason::Cancelled),
        Err(SpeechError::Stale)
    ));
    assert_eq!(
        scheduler.end(&old, old.identity(), 1),
        Err(SpeechError::Stale)
    );
    assert_eq!(scheduler.snapshot().schedule.active_dispatches, 1);
    close(&mut scheduler, &current, 1);
    assert_eq!(scheduler.next(&current).unwrap().unwrap().bytes, &[1]);
    scheduler.acknowledge(&current, 0).unwrap();
    scheduler.finish(&current).unwrap();
    assert_eq!(scheduler.snapshot().schedule.completed_dispatches, 1);
}

#[test]
fn receipt_from_another_or_recreated_owner_cannot_touch_matching_current_work() {
    let mut first = scheduler();
    let old = start(&mut first, identity(3));
    drop(first);
    let mut second = scheduler();
    let current = start(&mut second, identity(3));
    assert_eq!(
        second
            .queue_chunk(&old, old.identity(), 0, Box::from([9]))
            .unwrap_err()
            .reason,
        SpeechError::Stale
    );
    assert!(matches!(
        second.stop(&old, SpeechStopReason::Cancelled),
        Err(SpeechError::Stale)
    ));
    assert_eq!(second.snapshot().schedule.active_dispatches, 1);
    second.stop(&current, SpeechStopReason::Cancelled).unwrap();
}

#[test]
fn every_actual_event_identity_field_is_checked_before_changing_owned_bytes_or_end() {
    let mut scheduler = scheduler();
    let receipt = start(&mut scheduler, identity(3));
    for field in 0..6 {
        let mut changed = receipt.identity();
        match field {
            0 => changed.basis.session = SessionId::from_bytes(&[9; 16]).unwrap(),
            1 => changed.basis.run = RunId::from_bytes(&[9; 16]).unwrap(),
            2 => changed.basis.revision = basis(2, 0).revision,
            3 => changed.job = identity(4).job,
            4 => changed.operation = OperationId::from_bytes(&[9; 16]).unwrap(),
            _ => changed.generation += 1,
        }
        assert_eq!(
            scheduler
                .queue_chunk(&receipt, changed, 0, Box::from([9]))
                .unwrap_err()
                .reason,
            SpeechError::IdentityMismatch
        );
        assert_eq!(
            scheduler.end(&receipt, changed, 0),
            Err(SpeechError::IdentityMismatch)
        );
        assert_eq!(
            scheduler.eof(&receipt, changed),
            Err(SpeechError::IdentityMismatch)
        );
    }
    assert_eq!(scheduler.snapshot().buffered_bytes, 0);
    scheduler
        .stop(&receipt, SpeechStopReason::Cancelled)
        .unwrap();
}

#[test]
fn new_revision_or_recovery_epoch_keeps_old_buffers_owned_but_disables_delivery() {
    for new_basis in [basis(1, 8), basis(2, 0)] {
        let mut scheduler = scheduler();
        let old = start(&mut scheduler, identity(3));
        queue(&mut scheduler, &old, 0, &[9]);
        close(&mut scheduler, &old, 1);
        scheduler.replace_basis(new_basis).unwrap();
        assert!(matches!(scheduler.next(&old), Err(SpeechError::Stale)));
        assert_eq!(scheduler.acknowledge(&old, 0), Err(SpeechError::Stale));
        assert_eq!(scheduler.snapshot().buffered_bytes, 1);
        assert_eq!(scheduler.snapshot().schedule.active_dispatches, 1);
        let stopped = scheduler
            .stop(&old, SpeechStopReason::Failed(NativeFailure::Stale))
            .unwrap();
        assert_eq!(stopped.discarded_bytes, 1);
        let mut fresh = identity(3);
        fresh.basis = new_basis;
        let current = start(&mut scheduler, fresh);
        assert_eq!(
            scheduler.end(&old, old.identity(), 0),
            Err(SpeechError::Stale)
        );
        close(&mut scheduler, &current, 0);
        scheduler.finish(&current).unwrap();
    }
}

#[test]
fn obsolete_queued_speech_keeps_its_budget_without_starting_or_blocking_current_dispatch() {
    for new_basis in [basis(1, 8), basis(2, 0)] {
        let mut scheduler = scheduler();
        let old = scheduler.admit(identity(3), Box::from([7; 6])).unwrap();
        scheduler.replace_basis(new_basis).unwrap();
        let retained = scheduler.snapshot();
        assert_eq!(retained.schedule.queued_items, 1);
        assert_eq!(retained.schedule.queued_bytes, 6);
        assert!(scheduler.begin().unwrap().is_none());
        assert_eq!(scheduler.snapshot(), retained);

        let mut fresh = identity(4);
        fresh.basis = new_basis;
        let current = scheduler.admit(fresh, Box::from([1; 2])).unwrap();
        let begun = scheduler.begin().unwrap().unwrap();
        assert_eq!(begun.identity(), fresh);
        let snapshot = scheduler.snapshot();
        assert_eq!(snapshot.schedule.queued_items, 1);
        assert_eq!(snapshot.schedule.queued_bytes, 6);
        assert_eq!(snapshot.schedule.active_dispatches, 1);
        assert_eq!(snapshot.schedule.active_bytes, 2);
        match scheduler.admit(
            SpeechIdentity {
                job: identity(5).job,
                ..fresh
            },
            Box::from([9]),
        ) {
            Err(SpeechAdmissionRefusal::Schedule(refusal)) => {
                assert_eq!(refusal.reason, ScheduleError::ByteCapacity);
                assert_eq!(refusal.request.payload(), &[9]);
            }
            other => panic!("unexpected admission: {other:?}"),
        }
        close(&mut scheduler, &current, 0);
        assert_eq!(scheduler.finish(&begun).unwrap().payload(), &[1; 2]);
        assert!(scheduler.begin().unwrap().is_none());
        assert_eq!(
            scheduler.cancel_queued(&old).unwrap().unwrap().payload(),
            &[7; 6]
        );
        let snapshot = scheduler.snapshot();
        assert_eq!(snapshot.schedule.queued_items, 0);
        assert_eq!(snapshot.schedule.queued_bytes, 0);
        assert_eq!(snapshot.schedule.active_dispatches, 0);
        assert_eq!(snapshot.schedule.completed_dispatches, 1);
        assert_eq!(snapshot.schedule.cancelled_dispatches, 0);

        let reused = scheduler
            .admit(
                SpeechIdentity {
                    basis: new_basis,
                    ..identity(3)
                },
                Box::from([2]),
            )
            .unwrap();
        assert!(matches!(
            scheduler.cancel_queued(&old),
            Err(SpeechError::Stale)
        ));
        assert_eq!(
            scheduler.begin().unwrap().unwrap().identity(),
            reused.identity()
        );
        scheduler
            .stop(&reused, SpeechStopReason::Cancelled)
            .unwrap();
    }
}

#[test]
fn basis_change_keeps_started_work_owned_and_fifo_among_eligible_queued_speech() {
    let mut scheduler = scheduler();
    let active_old = start(&mut scheduler, identity(3));
    queue(&mut scheduler, &active_old, 0, &[7; 3]);
    let queued_old = scheduler.admit(identity(4), Box::from([8; 3])).unwrap();
    let new_basis = basis(2, 0);
    scheduler.replace_basis(new_basis).unwrap();
    let fresh = SpeechIdentity {
        basis: new_basis,
        ..identity(5)
    };
    let first = scheduler.admit(fresh, Box::from([1; 3])).unwrap();
    let second = scheduler
        .admit(
            SpeechIdentity {
                job: identity(6).job,
                ..fresh
            },
            Box::from([2]),
        )
        .unwrap();
    assert_eq!(
        scheduler.begin().unwrap().unwrap().identity(),
        first.identity()
    );
    assert!(scheduler.begin().unwrap().is_none());
    let snapshot = scheduler.snapshot();
    assert_eq!(snapshot.schedule.active_dispatches, 2);
    assert_eq!(snapshot.schedule.active_bytes, 4);
    assert_eq!(snapshot.schedule.queued_items, 2);
    assert_eq!(snapshot.schedule.queued_bytes, 4);
    assert_eq!(snapshot.buffered_bytes, 3);
    assert_eq!(
        scheduler
            .queue_chunk(&active_old, active_old.identity(), 1, Box::from([9]))
            .unwrap_err()
            .reason,
        SpeechError::Stale
    );
    let stopped = scheduler
        .stop(&active_old, SpeechStopReason::Cancelled)
        .unwrap();
    assert_eq!((stopped.discarded_chunks, stopped.discarded_bytes), (1, 3));
    assert_eq!(
        scheduler.begin().unwrap().unwrap().identity(),
        second.identity()
    );
    assert_eq!(scheduler.snapshot().buffered_bytes, 0);
    assert_eq!(
        scheduler
            .cancel_queued(&queued_old)
            .unwrap()
            .unwrap()
            .payload(),
        &[8; 3]
    );
    queue(&mut scheduler, &second, 0, &[2]);
    close(&mut scheduler, &second, 1);
    assert_eq!(scheduler.next(&second).unwrap().unwrap().bytes, &[2]);
    scheduler.acknowledge(&second, 0).unwrap();
    scheduler.finish(&second).unwrap();
    close(&mut scheduler, &first, 0);
    scheduler.finish(&first).unwrap();
    let snapshot = scheduler.snapshot();
    assert_eq!(snapshot.schedule.active_dispatches, 0);
    assert_eq!(snapshot.schedule.queued_items, 0);
    assert_eq!(snapshot.schedule.active_bytes, 0);
    assert_eq!(snapshot.schedule.queued_bytes, 0);
    assert_eq!(snapshot.schedule.cancelled_dispatches, 1);
    assert_eq!(snapshot.schedule.completed_dispatches, 2);
    assert_eq!(snapshot.buffered_bytes, 0);
    assert_eq!(snapshot.buffered_chunks, 0);
}

#[test]
fn scheduler_admission_cancellation_and_occupied_slots_preserve_original_command_bytes() {
    let mut scheduler = scheduler();
    let queued = scheduler.admit(identity(3), Box::from([7; 8])).unwrap();
    let refusal = scheduler.admit(identity(4), Box::from([9])).unwrap_err();
    match refusal {
        SpeechAdmissionRefusal::Schedule(refusal) => {
            assert_eq!(refusal.reason, ScheduleError::ByteCapacity);
            assert_eq!(refusal.request.payload(), &[9]);
        }
        other => panic!("unexpected admission refusal: {other:?}"),
    }
    assert_eq!(
        scheduler.cancel_queued(&queued).unwrap().unwrap().payload(),
        &[7; 8]
    );
    let first = start(&mut scheduler, identity(3));
    let second = start(&mut scheduler, identity(4));
    scheduler.admit(identity(5), Box::from([5])).unwrap();
    assert!(scheduler.begin().unwrap().is_none());
    assert!(scheduler.cancel_queued(&first).unwrap().is_none());
    scheduler.stop(&first, SpeechStopReason::Cancelled).unwrap();
    let third = scheduler.begin().unwrap().unwrap();
    assert_eq!(third.identity(), identity(5));
    scheduler
        .stop(&second, SpeechStopReason::Failed(NativeFailure::Deadline))
        .unwrap();
    scheduler.stop(&third, SpeechStopReason::Cancelled).unwrap();
    assert_eq!(scheduler.snapshot().schedule.active_dispatches, 0);
    assert_eq!(scheduler.snapshot().schedule.failed_dispatches, 1);
}

#[test]
fn missing_terminal_and_dropped_receipt_fail_closed_until_explicit_owner_teardown() {
    let mut scheduler = scheduler();
    let receipt = start(&mut scheduler, identity(3));
    queue(&mut scheduler, &receipt, 0, &[1]);
    assert_eq!(
        scheduler.finish(&receipt).unwrap_err(),
        SpeechError::Waiting
    );
    drop(receipt);
    assert_eq!(scheduler.snapshot().schedule.active_dispatches, 1);
    assert_eq!(scheduler.snapshot().buffered_bytes, 1);
    assert!(matches!(
        scheduler.admit(identity(3), Box::from([9])),
        Err(SpeechAdmissionRefusal::Schedule(_))
    ));
}

#[test]
fn delayed_queued_cancellation_cannot_remove_reused_identity_and_no_data_precedes_dispatch() {
    let mut scheduler = scheduler();
    let old = scheduler.admit(identity(3), Box::from([7])).unwrap();
    assert_eq!(
        scheduler
            .queue_chunk(&old, old.identity(), 0, Box::from([9]))
            .unwrap_err()
            .reason,
        SpeechError::NotStarted
    );
    assert_eq!(
        scheduler.end(&old, old.identity(), 0),
        Err(SpeechError::NotStarted)
    );
    assert_eq!(
        scheduler.eof(&old, old.identity()),
        Err(SpeechError::NotStarted)
    );
    scheduler.cancel_queued(&old).unwrap().unwrap();
    let current = scheduler.admit(identity(3), Box::from([1])).unwrap();
    assert!(matches!(
        scheduler.cancel_queued(&old),
        Err(SpeechError::Stale)
    ));
    assert_eq!(scheduler.snapshot().schedule.queued_items, 1);
    let begun = scheduler.begin().unwrap().unwrap();
    queue(&mut scheduler, &current, 0, &[1]);
    close(&mut scheduler, &begun, 1);
    assert_eq!(scheduler.next(&current).unwrap().unwrap().bytes, &[1]);
    scheduler.acknowledge(&begun, 0).unwrap();
    assert_eq!(scheduler.finish(&current).unwrap().payload(), &[1]);
}

#[test]
fn invalid_identity_and_duplicate_admission_preserve_exact_owned_payloads() {
    let mut scheduler = scheduler();
    for error in [SpeechError::Stale, SpeechError::InvalidGeneration] {
        let mut invalid = identity(3);
        if error == SpeechError::Stale {
            invalid.basis = basis(1, 6);
        } else {
            invalid.generation = 0;
        }
        let bytes: Box<[u8]> = Box::from([9, 8]);
        let address = bytes.as_ptr();
        match scheduler.admit(invalid, bytes).unwrap_err() {
            SpeechAdmissionRefusal::Invalid(reason, bytes) => {
                assert_eq!(reason, error);
                assert_eq!(bytes.as_ptr(), address);
            }
            other => panic!("unexpected refusal: {other:?}"),
        }
    }
    let queued = scheduler.admit(identity(3), Box::from([1])).unwrap();
    let bytes: Box<[u8]> = Box::from([9, 8]);
    let address = bytes.as_ptr();
    match scheduler.admit(identity(3), bytes).unwrap_err() {
        SpeechAdmissionRefusal::Schedule(refusal) => {
            assert_eq!(refusal.reason, ScheduleError::Duplicate);
            assert_eq!(refusal.request.payload().as_ptr(), address);
        }
        other => panic!("unexpected refusal: {other:?}"),
    }
    scheduler.cancel_queued(&queued).unwrap();
    assert_eq!(
        scheduler.replace_basis(basis(1, 7)),
        Err(SpeechError::RevisionNotAdvanced)
    );
}

#[test]
fn refusal_diagnostics_omit_private_chunk_bytes() {
    let mut scheduler = scheduler();
    let receipt = start(&mut scheduler, identity(3));
    queue(&mut scheduler, &receipt, 0, &[1; 6]);
    let refusal = scheduler
        .queue_chunk(
            &receipt,
            receipt.identity(),
            1,
            Box::from([123, 45, 67, 89]),
        )
        .unwrap_err();
    assert_eq!(refusal.reason, SpeechError::ByteCapacity);
    assert_eq!(&*refusal.bytes, &[123, 45, 67, 89]);
    let diagnostic = format!("{refusal:?}");
    assert!(diagnostic.contains("payload_bytes: 4"));
    assert!(!diagnostic.contains("[123, 45, 67, 89]"));
    scheduler
        .stop(&receipt, SpeechStopReason::Cancelled)
        .unwrap();
}

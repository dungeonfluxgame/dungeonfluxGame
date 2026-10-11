//! B-F45-D03 finite contract over the existing canonical capture and output owners.
//! Adapter acquisition observations and synthetic PCM do not qualify a device.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use df_audio::{
    AudioQueue, CaptureAcquisition, CaptureChunk, CaptureError, CaptureGestureObservation,
    CaptureLimits, CaptureSession, CaptureState, QueueError, QueueState,
};
use df_media::schedule::ScheduleLimits;
use df_media::speech::{SpeechLimits, SpeechScheduler};
use df_model::checkpoint::{
    AudienceScope, AudioDestination, AudioOutputLease, CaptureLease, RecordId, WindowId,
};
use df_types::{MemberId, RevisionLabel};

fn private_output(member: MemberId, id: u8) -> AudioOutputLease {
    AudioOutputLease {
        id: RecordId::from_bytes(&[id; 16]).unwrap(),
        destination: AudioDestination::PrivateListener {
            member,
            binding: common::binding(),
        },
        audience: AudienceScope::Members(vec![member]),
        ..common::lease()
    }
}

fn capture_lease(member: MemberId, id: u8, offer: u8) -> CaptureLease {
    CaptureLease {
        id: RecordId::from_bytes(&[id; 16]).unwrap(),
        member,
        binding: common::binding(),
        generation: 9,
        permitted_offer: Some(WindowId::from_bytes(&[offer; 16]).unwrap()),
        format: RevisionLabel::new(Some("pcm_s16le_16k_mono")).unwrap(),
    }
}

fn open_capture(lease: &CaptureLease) -> CaptureSession {
    let mut capture = CaptureSession::new(
        lease.clone(),
        lease.binding,
        lease.generation,
        lease.member,
        lease.permitted_offer,
        CaptureLimits {
            maximum_chunks: 2,
            maximum_chunk_bytes: 4,
            maximum_total_bytes: 8,
        },
    )
    .unwrap();
    capture
        .select_codec(std::slice::from_ref(&lease.format))
        .unwrap();
    // These are supplied fixture observations; no microphone API is invoked.
    capture
        .start(
            lease,
            lease.binding,
            lease.generation,
            CaptureGestureObservation::SynchronousApiInvocationObserved,
            CaptureAcquisition::Opened,
        )
        .unwrap();
    capture
}

#[test]
fn private_receipts_cannot_route_pcm_to_a_sibling_listener_or_public_room() {
    let mut source = SpeechScheduler::new(
        common::basis(1, 7),
        ScheduleLimits {
            queue_items: 2,
            queue_bytes: 8,
            speech_items: 1,
            speech_bytes: 1,
            execution_slots: 2,
            speech_slots: 1,
        },
        SpeechLimits {
            maximum_chunks: 1,
            maximum_bytes: 4,
        },
    )
    .unwrap();
    source
        .admit(common::identity(1), Box::from([9_u8]))
        .unwrap();
    let source_receipt = source.begin().unwrap().unwrap();
    source
        .queue_chunk(
            &source_receipt,
            source_receipt.identity(),
            0,
            Box::from([1_u8]),
        )
        .unwrap();
    source
        .end(&source_receipt, source_receipt.identity(), 1)
        .unwrap();
    source
        .eof(&source_receipt, source_receipt.identity())
        .unwrap();
    let alice = MemberId::from_bytes(&[8; 16]).unwrap();
    let bob = MemberId::from_bytes(&[9; 16]).unwrap();
    let alice_lease = private_output(alice, 18);
    let bob_lease = private_output(bob, 19);
    let mut alice_queue = AudioQueue::new(
        common::binding(),
        alice_lease.clone(),
        common::basis(1, 7),
        common::limits(2, 32),
    )
    .unwrap();
    let mut bob_queue = AudioQueue::new(
        common::binding(),
        bob_lease.clone(),
        common::basis(1, 7),
        common::limits(2, 32),
    )
    .unwrap();
    let mut public_queue = common::queue(2, 32);
    let public_receipt = common::start(&mut public_queue, 1, 0);
    let alice_receipt = alice_queue
        .replace(
            &alice_lease,
            common::basis(1, 7),
            source_receipt.identity(),
            common::asset(),
            common::format(),
            0,
        )
        .unwrap()
        .receipt;
    let bob_receipt = bob_queue
        .replace(
            &bob_lease,
            common::basis(1, 7),
            source_receipt.identity(),
            common::asset(),
            common::format(),
            0,
        )
        .unwrap()
        .receipt;
    // Equal speech identities are intentionally insufficient to transfer ownership.
    assert_eq!(alice_receipt.identity(), bob_receipt.identity());
    assert_eq!(alice_receipt.identity(), public_receipt.identity());
    let bob_before = bob_queue.snapshot();
    let public_before = public_queue.snapshot();
    let chunk = source.next(&source_receipt).unwrap().unwrap();
    assert_eq!(chunk.sequence, 0);
    assert_eq!(chunk.bytes, &[1]);
    // Opaque actual media bytes and independent synthetic PCM make no decoder claim.
    let samples = common::pcm(&[0.25, -0.25]);
    let original_pointer = samples.samples().as_ptr();
    let refused = bob_queue
        .enqueue(
            &alice_receipt,
            &bob_lease,
            common::manifest(),
            0,
            0,
            samples,
        )
        .unwrap_err();
    assert_eq!(refused.reason, QueueError::ForeignOwner);
    assert_eq!(refused.buffer.samples().as_ptr(), original_pointer);
    assert_eq!(bob_queue.snapshot(), bob_before);
    let refused = public_queue
        .enqueue(
            &alice_receipt,
            &common::lease(),
            common::manifest(),
            0,
            0,
            refused.buffer,
        )
        .unwrap_err();
    assert_eq!(refused.reason, QueueError::ForeignOwner);
    assert_eq!(refused.buffer.samples(), &[0.25, -0.25]);
    assert_eq!(refused.buffer.samples().as_ptr(), original_pointer);
    assert_eq!(public_queue.snapshot(), public_before);
    assert_eq!(source.snapshot().buffered_bytes, 1);
    assert_eq!(source.next(&source_receipt).unwrap().unwrap().bytes, &[1]);
    alice_queue
        .enqueue(
            &alice_receipt,
            &alice_lease,
            common::manifest(),
            0,
            0,
            refused.buffer,
        )
        .unwrap();
    source.acknowledge(&source_receipt, 0).unwrap();
    let completed = source.finish(&source_receipt).unwrap();
    assert_eq!(*completed.id(), alice_receipt.identity().job);
    assert_eq!(completed.operation(), alice_receipt.identity().operation);
    assert_eq!(completed.payload(), &[9]);
    assert_eq!(source.snapshot().schedule.completed_dispatches, 1);
    assert_eq!(
        alice_queue.close(&alice_receipt, &alice_lease, 1, 2),
        Ok(QueueState::Draining)
    );
    let ticket = alice_queue
        .begin(&alice_receipt, &alice_lease)
        .unwrap()
        .unwrap();
    assert_eq!(
        alice_queue.dispatched(&ticket).unwrap().buffer.samples(),
        &[0.25, -0.25]
    );
    assert_eq!(
        alice_queue.complete(&ticket),
        Ok(df_audio::BufferCompletion::Current(QueueState::Drained))
    );
    assert_eq!(alice_queue.snapshot().retained_buffers, 0);
    assert_eq!(bob_queue.snapshot(), bob_before);
    assert_eq!(public_queue.snapshot(), public_before);
}

#[test]
fn canonical_member_and_audience_changes_are_refused_even_with_identical_output_labels() {
    let alice = MemberId::from_bytes(&[8; 16]).unwrap();
    let bob = MemberId::from_bytes(&[9; 16]).unwrap();
    let lease = private_output(alice, 18);
    let mut queue = AudioQueue::new(
        common::binding(),
        lease.clone(),
        common::basis(1, 7),
        common::limits(2, 32),
    )
    .unwrap();
    let receipt = queue
        .replace(
            &lease,
            common::basis(1, 7),
            common::identity(1),
            common::asset(),
            common::format(),
            0,
        )
        .unwrap()
        .receipt;
    let mut changed_member = lease.clone();
    changed_member.destination = AudioDestination::PrivateListener {
        member: bob,
        binding: common::binding(),
    };
    let mut changed_audience = lease.clone();
    changed_audience.audience = AudienceScope::Members(vec![bob]);
    let mut changed_generation = lease.clone();
    changed_generation.generation += 1;
    let mut locked = lease.clone();
    locked.device_unlocked = false;
    let before = queue.snapshot();
    for changed in [changed_member, changed_audience, changed_generation, locked] {
        let samples = common::pcm(&[0.5]);
        let original_pointer = samples.samples().as_ptr();
        let refused = queue
            .enqueue(&receipt, &changed, common::manifest(), 0, 0, samples)
            .unwrap_err();
        assert_eq!(refused.reason, QueueError::WrongLease);
        assert_eq!(refused.buffer.samples().as_ptr(), original_pointer);
        assert_eq!(refused.buffer.samples(), &[0.5]);
        assert_eq!(
            queue.begin(&receipt, &changed).unwrap_err(),
            QueueError::WrongLease
        );
        assert_eq!(queue.snapshot(), before);
    }
    queue
        .enqueue(
            &receipt,
            &lease,
            common::manifest(),
            0,
            0,
            common::pcm(&[0.5]),
        )
        .unwrap();
    assert!(queue.begin(&receipt, &lease).unwrap().is_some());
}

#[test]
fn cancelling_one_offer_capture_preserves_another_capture_and_its_separate_output() {
    let alice = MemberId::from_bytes(&[8; 16]).unwrap();
    let bob = MemberId::from_bytes(&[9; 16]).unwrap();
    let alice_lease = capture_lease(alice, 28, 38);
    let bob_lease = capture_lease(bob, 29, 39);
    assert!(matches!(
        CaptureSession::new(
            alice_lease.clone(),
            alice_lease.binding,
            alice_lease.generation,
            alice_lease.member,
            bob_lease.permitted_offer,
            CaptureLimits {
                maximum_chunks: 2,
                maximum_chunk_bytes: 4,
                maximum_total_bytes: 8,
            },
        ),
        Err(CaptureError::InvalidLease)
    ));
    let mut alice_capture = open_capture(&alice_lease);
    let mut bob_capture = open_capture(&bob_lease);
    let before = alice_capture.snapshot();
    let bytes = vec![1, 2, 3];
    let original_pointer = bytes.as_ptr();
    let refused = alice_capture
        .chunk(
            &bob_lease,
            alice_lease.binding,
            alice_lease.generation,
            CaptureChunk::new(0, 0, bytes),
        )
        .unwrap_err();
    assert_eq!(refused.error, CaptureError::StaleLease);
    assert_eq!(refused.chunk.bytes().as_ptr(), original_pointer);
    assert_eq!(alice_capture.snapshot(), before);
    alice_capture
        .chunk(
            &alice_lease,
            alice_lease.binding,
            alice_lease.generation,
            refused.chunk,
        )
        .unwrap();
    bob_capture
        .chunk(
            &bob_lease,
            bob_lease.binding,
            bob_lease.generation,
            CaptureChunk::new(0, 0, vec![4, 5, 6]),
        )
        .unwrap();
    let output = private_output(alice, 18);
    let mut queue = AudioQueue::new(
        common::binding(),
        output.clone(),
        common::basis(1, 7),
        common::limits(2, 32),
    )
    .unwrap();
    let receipt = queue
        .replace(
            &output,
            common::basis(1, 7),
            common::identity(1),
            common::asset(),
            common::format(),
            0,
        )
        .unwrap()
        .receipt;
    queue
        .enqueue(
            &receipt,
            &output,
            common::manifest(),
            0,
            0,
            common::pcm(&[0.5]),
        )
        .unwrap();
    let output_before = queue.snapshot();
    let bob_before = bob_capture.snapshot();
    alice_capture
        .cancel(&alice_lease, alice_lease.binding, alice_lease.generation)
        .unwrap();
    assert_eq!(alice_capture.snapshot().state, CaptureState::Cancelled);
    assert_eq!(alice_capture.snapshot().retained_bytes, 0);
    assert_eq!(bob_capture.snapshot(), bob_before);
    assert_eq!(queue.snapshot(), output_before);
    let recording = bob_capture
        .finish(&bob_lease, bob_lease.binding, bob_lease.generation)
        .unwrap();
    assert_eq!(recording.lease(), &bob_lease);
    assert_eq!(recording.chunks()[0].bytes(), &[4, 5, 6]);
    assert_eq!(recording.retained_bytes(), 3);
    assert_eq!(queue.snapshot(), output_before);
}

#[test]
fn renewed_capture_and_output_owners_reject_old_leases_and_receipts_independently() {
    let alice = MemberId::from_bytes(&[8; 16]).unwrap();
    let old_capture_lease = capture_lease(alice, 28, 38);
    let mut new_capture_lease = old_capture_lease.clone();
    new_capture_lease.generation += 1;
    let mut old_capture = open_capture(&old_capture_lease);
    let mut new_capture = open_capture(&new_capture_lease);
    let before = new_capture.snapshot();
    let refused = new_capture
        .chunk(
            &old_capture_lease,
            old_capture_lease.binding,
            old_capture_lease.generation,
            CaptureChunk::new(0, 0, vec![1, 2]),
        )
        .unwrap_err();
    assert_eq!(refused.error, CaptureError::StaleLease);
    assert_eq!(refused.chunk.bytes(), &[1, 2]);
    assert_eq!(new_capture.snapshot(), before);
    new_capture
        .chunk(
            &new_capture_lease,
            new_capture_lease.binding,
            new_capture_lease.generation,
            refused.chunk,
        )
        .unwrap();
    let admitted = new_capture.snapshot();
    assert!(matches!(
        new_capture.finish(
            &old_capture_lease,
            old_capture_lease.binding,
            old_capture_lease.generation
        ),
        Err(CaptureError::StaleLease)
    ));
    assert_eq!(new_capture.snapshot(), admitted);
    let old_output = private_output(alice, 18);
    let mut new_output = old_output.clone();
    new_output.generation += 1;
    let mut old_queue = AudioQueue::new(
        common::binding(),
        old_output.clone(),
        common::basis(1, 7),
        common::limits(2, 32),
    )
    .unwrap();
    let old_receipt = old_queue
        .replace(
            &old_output,
            common::basis(1, 7),
            common::identity(1),
            common::asset(),
            common::format(),
            0,
        )
        .unwrap()
        .receipt;
    let mut new_queue = AudioQueue::new(
        common::binding(),
        new_output.clone(),
        common::basis(1, 7),
        common::limits(2, 32),
    )
    .unwrap();
    let new_receipt = new_queue
        .replace(
            &new_output,
            common::basis(1, 7),
            common::identity(1),
            common::asset(),
            common::format(),
            0,
        )
        .unwrap()
        .receipt;
    let output_before = new_queue.snapshot();
    let refused = new_queue
        .enqueue(
            &old_receipt,
            &new_output,
            common::manifest(),
            0,
            0,
            common::pcm(&[0.5]),
        )
        .unwrap_err();
    assert_eq!(refused.reason, QueueError::ForeignOwner);
    let refused = new_queue
        .enqueue(
            &new_receipt,
            &old_output,
            common::manifest(),
            0,
            0,
            refused.buffer,
        )
        .unwrap_err();
    assert_eq!(refused.reason, QueueError::WrongLease);
    assert_eq!(new_queue.snapshot(), output_before);
    old_capture
        .dispose(
            &old_capture_lease,
            old_capture_lease.binding,
            old_capture_lease.generation,
        )
        .unwrap();
    old_queue.dispose();
    assert_eq!(new_capture.snapshot(), admitted);
    assert_eq!(new_queue.snapshot(), output_before);
    new_queue
        .enqueue(
            &new_receipt,
            &new_output,
            common::manifest(),
            0,
            0,
            refused.buffer,
        )
        .unwrap();
    let recording = new_capture
        .finish(
            &new_capture_lease,
            new_capture_lease.binding,
            new_capture_lease.generation,
        )
        .unwrap();
    assert_eq!(recording.lease(), &new_capture_lease);
    assert_eq!(recording.chunks()[0].bytes(), &[1, 2]);
    assert_eq!(new_queue.snapshot().retained_buffers, 1);
}

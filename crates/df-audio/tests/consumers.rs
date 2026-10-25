#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::*;
use df_assets::AssetManifest;
use df_audio::{BufferCompletion, QueueError, QueueState, REQUIRED_PLAYBACK_PREREQUISITES};
use df_client::cache::{AssetCache, CacheError, CacheKey, CacheLimits, CacheScope};
use df_media::schedule::ScheduleLimits;
use df_media::speech::{
    SpeechError, SpeechLimits, SpeechReceipt, SpeechScheduler, SpeechStopReason,
};
use df_model::checkpoint::ContentDigest;

fn media() -> SpeechScheduler {
    SpeechScheduler::new(
        basis(1, 7),
        ScheduleLimits {
            queue_items: 2,
            queue_bytes: 8,
            speech_items: 1,
            speech_bytes: 1,
            execution_slots: 2,
            speech_slots: 1,
        },
        SpeechLimits {
            maximum_chunks: 3,
            maximum_bytes: 8,
        },
    )
    .unwrap()
}

fn dispatch(owner: &mut SpeechScheduler, generation: u64) -> SpeechReceipt {
    owner
        .admit(identity(generation), Box::from([9_u8]))
        .unwrap();
    owner.begin().unwrap().unwrap()
}

#[test]
fn current_media_borrow_is_acknowledged_only_after_downstream_pcm_admission_succeeds() {
    let mut source = media();
    let source_receipt = dispatch(&mut source, 11);
    source
        .queue_chunk(
            &source_receipt,
            source_receipt.identity(),
            0,
            Box::from([1_u8]),
        )
        .unwrap();
    source
        .queue_chunk(
            &source_receipt,
            source_receipt.identity(),
            1,
            Box::from([2_u8]),
        )
        .unwrap();
    source
        .end(&source_receipt, source_receipt.identity(), 2)
        .unwrap();
    source
        .eof(&source_receipt, source_receipt.identity())
        .unwrap();
    let mut audio = queue(1, 8);
    let receipt = start(&mut audio, 11, 0);

    // Actual source bytes remain opaque. Local decoded fixture resources are
    // supplied independently; this test implements no production decoder/codec.
    let chunk = source.next(&source_receipt).unwrap().unwrap();
    assert_eq!(chunk.sequence, 0);
    assert_eq!(chunk.bytes, &[1]);
    audio
        .enqueue(
            &receipt,
            &lease(),
            manifest(),
            chunk.sequence,
            0,
            pcm(&[0.1, 0.2]),
        )
        .unwrap();
    source.acknowledge(&source_receipt, 0).unwrap();
    let chunk = source.next(&source_receipt).unwrap().unwrap();
    let refused = audio
        .enqueue(
            &receipt,
            &lease(),
            manifest(),
            chunk.sequence,
            2,
            pcm(&[0.3, 0.4]),
        )
        .unwrap_err();
    assert_eq!(refused.reason, QueueError::BufferCapacity);
    assert_eq!(source.snapshot().buffered_bytes, 1);
    assert_eq!(source.next(&source_receipt).unwrap().unwrap().bytes, &[2]);
    let first = audio.begin(&receipt, &lease()).unwrap().unwrap();
    audio.complete(&first).unwrap(); // Controlled consumer end, not speaker evidence.
    audio
        .enqueue(&receipt, &lease(), manifest(), 1, 2, refused.buffer)
        .unwrap();
    source.acknowledge(&source_receipt, 1).unwrap();
    let original_command = source.finish(&source_receipt).unwrap();
    assert_eq!(*original_command.id(), receipt.identity().job);
    assert_eq!(original_command.operation(), receipt.identity().operation);
    assert_eq!(original_command.payload(), &[9]);
    assert_eq!(
        audio.close(&receipt, &lease(), 2, 4),
        Ok(QueueState::Draining)
    );
    let tail = audio.begin(&receipt, &lease()).unwrap().unwrap();
    assert_eq!(
        audio.dispatched(&tail).unwrap().buffer.samples(),
        &[0.3, 0.4]
    );
    assert_eq!(
        audio.complete(&tail),
        Ok(BufferCompletion::Current(QueueState::Drained))
    );
    assert_eq!(source.snapshot().schedule.completed_dispatches, 1);
}

#[test]
fn actual_media_stop_requests_old_playback_stop_without_freeing_the_replacement() {
    let mut source = media();
    let old_source = dispatch(&mut source, 11);
    source
        .queue_chunk(&old_source, old_source.identity(), 0, Box::from([1_u8]))
        .unwrap();
    let mut audio = queue(2, 16);
    let old = start(&mut audio, 11, 0);
    audio
        .enqueue(&old, &lease(), manifest(), 0, 0, pcm(&[0.1, 0.2]))
        .unwrap();
    let old_buffer = audio.begin(&old, &lease()).unwrap().unwrap();

    let stopped = source
        .stop(&old_source, SpeechStopReason::Cancelled)
        .unwrap();
    assert_eq!(stopped.discarded_chunks, 1);
    assert_eq!(stopped.discarded_bytes, 1);
    let cancel = audio.cancel_media(&old, &lease(), &stopped).unwrap();
    assert_eq!(
        cancel.stop_required.unwrap().sequence(),
        old_buffer.sequence()
    );
    assert_eq!(audio.snapshot().sample_bytes, 8);
    let next_source = dispatch(&mut source, 12);
    let next = start(&mut audio, next_source.identity().generation, 10);
    let late_source = source
        .queue_chunk(&old_source, old_source.identity(), 1, Box::from([2_u8]))
        .unwrap_err();
    assert_eq!(late_source.reason, SpeechError::Stale);
    assert_eq!(&*late_source.bytes, &[2]);
    assert_eq!(
        audio
            .enqueue(&old, &lease(), manifest(), 1, 2, pcm(&[0.2]))
            .unwrap_err()
            .reason,
        QueueError::StaleReceipt
    );
    assert_eq!(audio.snapshot().state, QueueState::WaitingForStop);
    assert_eq!(
        audio.cancel_media(&old, &lease(), &stopped).unwrap_err(),
        QueueError::StaleReceipt
    );
    assert_eq!(
        audio.confirm_stopped(&old_buffer),
        Ok(BufferCompletion::Retired(QueueState::Accepting))
    );
    audio
        .enqueue(&next, &lease(), manifest(), 0, 10, pcm(&[0.3]))
        .unwrap();
    let current = audio.begin(&next, &lease()).unwrap().unwrap();
    assert_eq!(audio.complete(&old_buffer), Err(QueueError::StaleCallback));
    assert_eq!(audio.dispatched(&current).unwrap().buffer.samples(), &[0.3]);
    assert_eq!(source.snapshot().schedule.active_dispatches, 1);
    source
        .stop(&next_source, SpeechStopReason::Cancelled)
        .unwrap();
    let stop = audio.dispose().stop_required.unwrap();
    audio.confirm_stopped(&stop).unwrap();
}

#[test]
fn verified_cache_lease_revocation_fences_late_decode_and_retains_original_asset_binding() {
    // Current cache verifies the published SHA-256 known-answer vector, not an
    // invented asset source. "abc" is fixture bytes, never claimed playable audio.
    let digest = [
        0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22,
        0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00,
        0x15, 0xad,
    ];
    let mut reference = asset();
    reference.digest = ContentDigest(digest);
    let key = CacheKey {
        version: reference.key.clone(),
        bytes: AssetManifest {
            byte_len: reference.byte_length,
            sha256: reference.digest.0,
        },
    };
    let scope = CacheScope {
        session: basis(1, 7).session,
        run: basis(1, 7).run,
        binding: binding(),
    };
    let mut cache = AssetCache::new(
        scope,
        CacheLimits {
            max_assets: 1,
            max_pending: 1,
            max_leases: 1,
            max_bytes: 8,
        },
    )
    .unwrap();
    cache
        .apply_current(scope, basis(1, 7).revision, std::slice::from_ref(&key))
        .unwrap();
    let fetch = cache.fetch(&key).unwrap();
    cache.complete(&fetch, b"abc".to_vec()).unwrap();
    let source_lease = cache.acquire(&key).unwrap().unwrap();
    assert_eq!(cache.lease_bytes(&source_lease).unwrap(), b"abc");
    let mut audio = queue(1, 8);
    let receipt = audio
        .replace(
            &lease(),
            basis(1, 7),
            identity(11),
            reference.clone(),
            format(),
            0,
        )
        .unwrap()
        .receipt;
    audio
        .enqueue(
            &receipt,
            &lease(),
            source_lease.key().bytes,
            0,
            0,
            pcm(&[0.1]),
        )
        .unwrap();
    let ticket = audio.begin(&receipt, &lease()).unwrap().unwrap();
    assert_eq!(audio.dispatched(&ticket).unwrap().asset, &reference);

    cache
        .apply_current(scope, basis(1, 8).revision, &[])
        .unwrap();
    let stop = audio
        .cancel(&receipt, &lease())
        .unwrap()
        .stop_required
        .unwrap();
    let late_decoded = pcm(&[0.2]);
    assert_eq!(
        cache.lease_bytes(&source_lease),
        Err(CacheError::StaleLease)
    );
    assert_eq!(
        audio
            .enqueue(&receipt, &lease(), key.bytes, 1, 1, late_decoded)
            .unwrap_err()
            .reason,
        QueueError::Closed
    );
    assert_eq!(audio.snapshot().sample_bytes, 4);
    audio.confirm_stopped(&stop).unwrap();
    assert_eq!(audio.snapshot().sample_bytes, 0);
    assert!(!REQUIRED_PLAYBACK_PREREQUISITES.is_empty());
}

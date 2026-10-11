use std::time::Duration;

use df_assets::AssetManifest;
use df_audio::browser_playback::PlaybackTimeline;
use df_audio::{
    BufferCompletion, CaptionError, DriftDirection, PcmBuffer, PcmFormat, PcmSource, PlaybackBasis,
    PlaybackCue, PlaybackDestination, PlaybackOutput, PresentationQueue, QueueError, QueueLimits,
    QueueState, ServerPlaybackTimeline, TimelineError,
};
use df_types::{
    ClientBindingId, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision,
};

fn basis() -> PlaybackBasis {
    PlaybackBasis::new(
        SessionId::from_bytes(&[1; 16]).unwrap(),
        RunId::from_bytes(&[2; 16]).unwrap(),
        SessionRevision::new(RecoveryEpoch::new(1).unwrap(), 7),
    )
}
fn output() -> PlaybackOutput {
    PlaybackOutput::new(
        [3; 16],
        ClientBindingId::from_bytes(&[4; 16]).unwrap(),
        2,
        PlaybackDestination::PublicRoom,
        true,
    )
    .unwrap()
}
fn cue(job: u8, generation: u64) -> PlaybackCue {
    PlaybackCue::new(
        basis(),
        [job; 16],
        OperationId::from_bytes(&[5; 16]).unwrap(),
        generation,
    )
    .unwrap()
}
fn format() -> PcmFormat {
    PcmFormat::new(1, 48_000).unwrap()
}
fn source(key: &str, digest: u8, length: u64) -> PcmSource {
    PcmSource::new(
        RevisionLabel::new(Some(key)).unwrap(),
        AssetManifest {
            byte_len: length,
            sha256: [digest; 32],
        },
    )
    .unwrap()
}
fn queue() -> PresentationQueue {
    PresentationQueue::new(
        output().binding(),
        output(),
        basis(),
        QueueLimits {
            retained_buffers: 2,
            sample_bytes: 32,
            cue_chunks: 4,
            cue_frames: 8,
        },
    )
    .unwrap()
}
fn start(
    owner: &mut PresentationQueue,
    identity: PlaybackCue,
    asset: &PcmSource,
    first_frame: u64,
) -> df_audio::AudioReceipt {
    owner
        .replace(
            &output(),
            basis(),
            identity,
            asset.clone(),
            format(),
            first_frame,
        )
        .unwrap()
        .receipt
}
fn pcm() -> PcmBuffer {
    PcmBuffer::new(format(), Box::from([0.1_f32, 0.2])).unwrap()
}

#[test]
fn approved_exact_caption_and_source_remain_bound_to_current_receipt() {
    let mut owner = queue();
    let asset = source("approved-recording", 7, 8);
    let receipt = start(&mut owner, cue(6, 1), &asset, 100);
    let approved = "  Approved café line.  ";
    let binding = owner
        .bind_caption(&receipt, &output(), &asset, approved, approved.len())
        .unwrap();
    assert_eq!(
        owner.caption(&binding, &receipt.clone(), &output(), &asset),
        Ok(approved)
    );
    owner
        .enqueue(&receipt, &output(), asset.manifest(), 0, 100, pcm())
        .unwrap();
    let ticket = owner.begin(&receipt, &output()).unwrap().unwrap();
    let view = owner.dispatched(&ticket).unwrap();
    assert_eq!(view.identity, receipt.presentation_identity());
    assert_eq!(view.asset, &asset);
    assert_eq!(view.offset_frames, 100);
    assert_eq!(
        owner.caption(&binding, &receipt, &output(), view.asset),
        Ok(approved)
    );
    assert!(!format!("{binding:?}").contains(approved));
}

#[test]
fn wrong_source_scope_and_identity_refuse_without_changing_pcm() {
    let mut owner = queue();
    let asset = source("approved-recording", 7, 8);
    let receipt = start(&mut owner, cue(6, 1), &asset, 0);
    let binding = owner
        .bind_caption(&receipt, &output(), &asset, "Actual line", 100)
        .unwrap();
    owner
        .enqueue(&receipt, &output(), asset.manifest(), 0, 0, pcm())
        .unwrap();
    let before = owner.snapshot();
    for wrong in [
        source("other-key", 7, 8),
        source("approved-recording", 9, 8),
        source("approved-recording", 7, 9),
    ] {
        assert_eq!(
            owner.caption(&binding, &receipt, &output(), &wrong),
            Err(CaptionError::Queue(QueueError::WrongAsset))
        );
        assert!(matches!(
            owner.bind_caption(&receipt, &output(), &wrong, "Wrong", 100),
            Err(CaptionError::Queue(QueueError::WrongAsset))
        ));
    }
    let changed_lease = PlaybackOutput::new(
        [3; 16],
        output().binding(),
        3,
        PlaybackDestination::PublicRoom,
        true,
    )
    .unwrap();
    assert_eq!(
        owner.caption(&binding, &receipt, &changed_lease, &asset),
        Err(CaptionError::Queue(QueueError::WrongLease))
    );
    let mut foreign = queue();
    let foreign_receipt = start(&mut foreign, cue(6, 1), &asset, 0);
    assert_eq!(
        owner.caption(&binding, &foreign_receipt, &output(), &asset),
        Err(CaptionError::Queue(QueueError::ForeignOwner))
    );
    assert_eq!(owner.snapshot(), before);
}

#[test]
fn same_label_replacement_cannot_reuse_an_old_caption_instance() {
    let mut owner = queue();
    let asset = source("same-recording", 7, 8);
    let original = start(&mut owner, cue(6, 1), &asset, 0);
    let old = owner
        .bind_caption(&original, &output(), &asset, "Old association", 100)
        .unwrap();
    start(&mut owner, cue(8, 1), &asset, 0);
    let current = start(&mut owner, cue(6, 1), &asset, 0);
    assert_eq!(
        original.presentation_identity(),
        current.presentation_identity()
    );
    assert_eq!(
        owner.caption(&old, &original, &output(), &asset),
        Err(CaptionError::Queue(QueueError::StaleReceipt))
    );
    assert_eq!(
        owner.caption(&old, &current, &output(), &asset),
        Err(CaptionError::Queue(QueueError::StaleReceipt))
    );
    let new = owner
        .bind_caption(&current, &output(), &asset, "Fresh association", 100)
        .unwrap();
    assert_eq!(
        owner.caption(&new, &current, &output(), &asset),
        Ok("Fresh association")
    );
}

#[test]
fn fallback_requires_explicit_new_text_source_and_completed_old_stop() {
    let mut owner = queue();
    let primary = source("primary-recording", 7, 8);
    let fallback = source("fallback-recording", 8, 8);
    let original = start(&mut owner, cue(6, 1), &primary, 0);
    let old = owner
        .bind_caption(&original, &output(), &primary, "Primary line", 100)
        .unwrap();
    owner
        .enqueue(&original, &output(), primary.manifest(), 0, 0, pcm())
        .unwrap();
    let ticket = owner.begin(&original, &output()).unwrap().unwrap();
    let current = start(&mut owner, cue(6, 2), &fallback, 0);
    assert_eq!(
        owner.caption(&old, &current, &output(), &fallback),
        Err(CaptionError::Queue(QueueError::WaitingForStop))
    );
    assert!(matches!(
        owner.bind_caption(&current, &output(), &fallback, "Fallback line", 100),
        Err(CaptionError::Queue(QueueError::WaitingForStop))
    ));
    assert_eq!(
        owner.confirm_stopped(&ticket),
        Ok(BufferCompletion::Retired(QueueState::Accepting))
    );
    assert_eq!(
        owner.caption(&old, &current, &output(), &fallback),
        Err(CaptionError::Queue(QueueError::StaleReceipt))
    );
    let fresh = owner
        .bind_caption(&current, &output(), &fallback, "Fallback line", 100)
        .unwrap();
    assert_eq!(
        owner.caption(&fresh, &current, &output(), &fallback),
        Ok("Fallback line")
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn canonical_native_identity_projection_preserves_the_actual_source_and_frame_offset() {
    use df_audio::AudioQueue;
    use df_media::speech::SpeechIdentity;
    use df_model::checkpoint::{
        AssetKind, AssetReference, AudienceScope, AudioDestination, AudioOutputLease, Basis,
        ContentDigest, JobId, RecordId,
    };
    let canonical_basis = Basis {
        session: basis().session(),
        run: basis().run(),
        revision: basis().revision(),
    };
    let canonical_identity = SpeechIdentity {
        basis: canonical_basis,
        job: JobId::from_bytes(&[6; 16]).unwrap(),
        operation: cue(6, 1).operation(),
        generation: 1,
    };
    let canonical_lease = AudioOutputLease {
        id: RecordId::from_bytes(&[3; 16]).unwrap(),
        destination: AudioDestination::PublicRoom(output().binding()),
        generation: 2,
        audience: AudienceScope::Shared,
        device_unlocked: true,
    };
    let canonical_asset = AssetReference {
        key: RevisionLabel::new(Some("canonical-recording")).unwrap(),
        digest: ContentDigest([7; 32]),
        byte_length: 8,
        kind: AssetKind::Audio,
    };
    let mut native = AudioQueue::new(
        output().binding(),
        canonical_lease.clone(),
        canonical_basis,
        QueueLimits {
            retained_buffers: 1,
            sample_bytes: 8,
            cue_chunks: 1,
            cue_frames: 2,
        },
    )
    .unwrap();
    let receipt = native
        .replace(
            &canonical_lease,
            canonical_basis,
            canonical_identity,
            canonical_asset.clone(),
            format(),
            1_000,
        )
        .unwrap()
        .receipt;
    native
        .enqueue(
            &receipt,
            &canonical_lease,
            AssetManifest {
                byte_len: 8,
                sha256: [7; 32],
            },
            0,
            1_000,
            pcm(),
        )
        .unwrap();
    let ticket = native.begin(&receipt, &canonical_lease).unwrap().unwrap();
    let view = native.dispatched(&ticket).unwrap();
    assert_eq!(view.identity, canonical_identity);
    assert_eq!(view.asset, &canonical_asset);
    assert_eq!(receipt.presentation_identity(), cue(6, 1));
    let selected = PcmSource::new(
        view.asset.key.clone(),
        AssetManifest {
            byte_len: view.asset.byte_length,
            sha256: view.asset.digest.0,
        },
    )
    .unwrap();
    let mut presentation = queue();
    let projected = start(
        &mut presentation,
        receipt.presentation_identity(),
        &selected,
        view.offset_frames,
    );
    let caption = presentation
        .bind_caption(
            &projected,
            &output(),
            &selected,
            "Selected canonical line",
            100,
        )
        .unwrap();
    assert_eq!(
        presentation.caption(&caption, &projected, &output(), &selected),
        Ok("Selected canonical line")
    );
    let timeline = ServerPlaybackTimeline::new(
        view.offset_frames,
        Duration::from_secs(10),
        view.buffer.format(),
    );
    assert_eq!(
        timeline
            .observe(Duration::from_secs(10), view.offset_frames, Duration::ZERO)
            .unwrap()
            .direction(),
        DriftDirection::Aligned
    );
    native.complete(&ticket).unwrap();
}

#[test]
fn exact_server_drift_direction_budget_edge_and_fractional_frames_are_retained() {
    let timeline = ServerPlaybackTimeline::new(1_000, Duration::from_secs(10), format());
    let now = Duration::from_millis(10_500);
    let aligned = timeline.observe(now, 25_000, Duration::ZERO).unwrap();
    assert_eq!(aligned.direction(), DriftDirection::Aligned);
    assert_eq!(aligned.nanoseconds_numerator(), 0);
    assert!(aligned.within_budget());
    let behind = timeline
        .observe(now, 24_952, Duration::from_millis(1))
        .unwrap();
    assert_eq!(behind.direction(), DriftDirection::Behind);
    assert_eq!(behind.nanoseconds_numerator(), 48_000_000_000);
    assert_eq!(behind.sample_rate(), 48_000);
    assert!(behind.within_budget());
    assert!(
        !timeline
            .observe(now, 24_951, Duration::from_millis(1))
            .unwrap()
            .within_budget()
    );
    let ahead = timeline
        .observe(now, 25_048, Duration::from_millis(1))
        .unwrap();
    assert_eq!(ahead.direction(), DriftDirection::Ahead);
    assert!(ahead.within_budget());
    let fractional = ServerPlaybackTimeline::new(0, Duration::ZERO, PcmFormat::new(1, 3).unwrap());
    let drift = fractional
        .observe(Duration::from_secs(1), 2, Duration::from_nanos(333_333_333))
        .unwrap();
    assert_eq!(drift.nanoseconds_numerator(), 1_000_000_000);
    assert_eq!(drift.sample_rate(), 3);
    assert!(!drift.within_budget());
    assert!(
        fractional
            .observe(Duration::from_secs(1), 2, Duration::from_nanos(333_333_334))
            .unwrap()
            .within_budget()
    );
}

#[test]
fn late_pcm_chunk_arrival_keeps_the_original_frame_and_server_anchors() {
    let mut owner = queue();
    let asset = source("ordered-recording", 7, 16);
    let receipt = start(&mut owner, cue(6, 1), &asset, 100);
    let caption = owner
        .bind_caption(&receipt, &output(), &asset, "One stable line", 100)
        .unwrap();
    let server = ServerPlaybackTimeline::new(100, Duration::from_secs(10), format());
    // This is the pure clock helper, not a native execution of BrowserPlayback.
    let local = PlaybackTimeline::new(100, 17.25, format()).unwrap();
    owner
        .enqueue(&receipt, &output(), asset.manifest(), 0, 100, pcm())
        .unwrap();
    let first = owner.begin(&receipt, &output()).unwrap().unwrap();
    let first_offset = owner.dispatched(&first).unwrap().offset_frames;
    assert_eq!(local.start_seconds(first_offset), Ok(17.25));
    owner.complete(&first).unwrap();
    let late_observation = server
        .observe(Duration::from_millis(10_001), 102, Duration::ZERO)
        .unwrap();
    assert_eq!(late_observation.direction(), DriftDirection::Behind);
    assert!(!late_observation.within_budget());
    owner
        .enqueue(&receipt, &output(), asset.manifest(), 1, 102, pcm())
        .unwrap();
    let second = owner.begin(&receipt, &output()).unwrap().unwrap();
    assert_eq!(owner.dispatched(&second).unwrap().offset_frames, 102);
    assert_eq!(local.start_seconds(102), Ok(17.25 + 2.0 / 48_000.0));
    assert_eq!(
        server
            .observe(Duration::from_millis(10_001), 102, Duration::ZERO)
            .unwrap(),
        late_observation
    );
    assert_eq!(
        owner.caption(&caption, &receipt, &output(), &asset),
        Ok("One stable line")
    );
}

#[test]
fn invalid_order_limits_and_caption_bytes_refuse_while_valid_extremes_do_not_wrap() {
    let timeline = ServerPlaybackTimeline::new(10, Duration::from_secs(2), format());
    assert_eq!(
        timeline.observe(Duration::from_secs(1), 10, Duration::ZERO),
        Err(TimelineError::BeforeAnchor)
    );
    assert_eq!(
        timeline.observe(Duration::from_secs(2), 9, Duration::ZERO),
        Err(TimelineError::BeforeFirstFrame)
    );
    let extreme =
        ServerPlaybackTimeline::new(0, Duration::ZERO, PcmFormat::new(1, u32::MAX).unwrap());
    let maximum = Duration::MAX;
    let observed = extreme.observe(maximum, u64::MAX, maximum).unwrap();
    assert_eq!(observed.direction(), DriftDirection::Behind);
    assert!(observed.within_budget());
    let high_anchor = ServerPlaybackTimeline::new(u64::MAX, maximum, format());
    assert_eq!(
        high_anchor
            .observe(maximum, u64::MAX, Duration::ZERO)
            .unwrap()
            .direction(),
        DriftDirection::Aligned
    );
    let mut owner = queue();
    let asset = source("bounded-recording", 7, 8);
    let receipt = start(&mut owner, cue(6, 1), &asset, 0);
    let before = owner.snapshot();
    for limit in [0, 16_385, usize::MAX] {
        assert!(matches!(
            owner.bind_caption(&receipt, &output(), &asset, "Line", limit),
            Err(CaptionError::InvalidLimit)
        ));
    }
    assert!(matches!(
        owner.bind_caption(&receipt, &output(), &asset, "  \n ", 100),
        Err(CaptionError::Empty)
    ));
    assert!(matches!(
        owner.bind_caption(&receipt, &output(), &asset, "é", 1),
        Err(CaptionError::Capacity)
    ));
    let exact = "x".repeat(16_384);
    let binding = owner
        .bind_caption(&receipt, &output(), &asset, &exact, 16_384)
        .unwrap();
    assert_eq!(
        owner.caption(&binding, &receipt, &output(), &asset),
        Ok(exact.as_str())
    );
    assert!(matches!(
        owner.bind_caption(&receipt, &output(), &asset, &"x".repeat(16_385), 16_384),
        Err(CaptionError::Capacity)
    ));
    assert_eq!(owner.snapshot(), before);
}

#[test]
fn tail_drain_cancellation_and_disposal_reject_obsolete_caption_access() {
    let mut owner = queue();
    let asset = source("finite-recording", 7, 8);
    let receipt = start(&mut owner, cue(6, 1), &asset, 0);
    let caption = owner
        .bind_caption(&receipt, &output(), &asset, "Finite line", 100)
        .unwrap();
    owner
        .enqueue(&receipt, &output(), asset.manifest(), 0, 0, pcm())
        .unwrap();
    owner.close(&receipt, &output(), 1, 2).unwrap();
    assert_eq!(
        owner.caption(&caption, &receipt, &output(), &asset),
        Ok("Finite line")
    );
    let ticket = owner.begin(&receipt, &output()).unwrap().unwrap();
    assert_eq!(
        owner.complete(&ticket),
        Ok(BufferCompletion::Current(QueueState::Drained))
    );
    assert_eq!(
        owner.caption(&caption, &receipt, &output(), &asset),
        Err(CaptionError::Queue(QueueError::Closed))
    );
    let current = start(&mut owner, cue(6, 2), &asset, 0);
    let next = owner
        .bind_caption(&current, &output(), &asset, "Next line", 100)
        .unwrap();
    owner.cancel(&current, &output()).unwrap();
    assert_eq!(
        owner.caption(&next, &current, &output(), &asset),
        Err(CaptionError::Queue(QueueError::Closed))
    );
    owner.dispose();
    assert_eq!(
        owner.caption(&next, &current, &output(), &asset),
        Err(CaptionError::Queue(QueueError::Disposed))
    );
}

#[path = "support/browser_pcm.rs"]
mod browser_pcm;

use browser_pcm::*;
use df_audio::browser_playback::{PlaybackError, PlaybackLimits, PlaybackTimeline};
use df_audio::{BufferCompletion, PcmFormat, QueueError, QueueState};

#[test]
fn contiguous_frame_times_remain_on_one_anchor_despite_late_chunk_arrival() {
    let timeline = PlaybackTimeline::new(8_000, 17.25, PcmFormat::new(2, 48_000).unwrap()).unwrap();
    assert_eq!(timeline.start_seconds(8_000), Ok(17.25));
    assert_eq!(timeline.start_seconds(32_000), Ok(17.75));
    assert_eq!(timeline.start_seconds(56_000), Ok(18.25));
    assert_eq!(
        timeline.start_seconds(7_999),
        Err(PlaybackError::InvalidTimeline)
    );
    assert_eq!(
        timeline.start_seconds(u64::MAX),
        Err(PlaybackError::InvalidTimeline)
    );
}

#[test]
fn invalid_browser_time_and_unsupported_channels_are_refused_before_scheduling() {
    for invalid in [f64::NAN, f64::INFINITY, -1.0] {
        assert!(matches!(
            PlaybackTimeline::new(0, invalid, PcmFormat::new(1, 48_000).unwrap()),
            Err(PlaybackError::InvalidTimeline)
        ));
    }
    assert!(matches!(
        PlaybackTimeline::new(0, 0.0, PcmFormat::new(3, 48_000).unwrap()),
        Err(PlaybackError::UnsupportedChannels)
    ));
}

#[test]
fn queue_pcm_web_audio_copy_and_channel_scratch_all_count_against_capacity() {
    let limits = PlaybackLimits {
        total_sample_bytes: 240,
    };
    assert_eq!(
        limits.required_sample_bytes(80, 80, PcmFormat::new(1, 48_000).unwrap()),
        Ok(240)
    );
    assert_eq!(
        limits.required_sample_bytes(80, 80, PcmFormat::new(2, 48_000).unwrap()),
        Ok(200)
    );
    assert_eq!(
        limits.required_sample_bytes(84, 80, PcmFormat::new(1, 48_000).unwrap()),
        Err(PlaybackError::ResourceCapacity)
    );
    assert_eq!(
        limits.required_sample_bytes(usize::MAX, 8, PcmFormat::new(1, 48_000).unwrap()),
        Err(PlaybackError::ResourceCapacity)
    );
}

#[test]
fn mono_and_stereo_local_pcm_bytes_are_verified_and_preserve_interleaved_channels() {
    for channels in [1, 2] {
        let mut pcm_fixture = local_pcm(channels);
        let bytes = pcm_fixture.cache.lease_bytes(&pcm_fixture.source).unwrap();
        let buffer = pcm_fixture.buffer.take().unwrap();
        assert_eq!(buffer.frames(), FRAMES);
        assert_eq!(bytes.len(), buffer.sample_bytes());
        for (encoded, sample) in bytes.as_chunks::<4>().0.iter().zip(buffer.samples()) {
            assert_eq!(*encoded, sample.to_le_bytes());
        }
        assert_eq!(buffer.samples().first(), Some(&0.125));
        if channels == 2 {
            assert_eq!(buffer.samples().get(1), Some(&-0.125));
        }
    }
}

#[test]
fn finite_close_drains_exact_pcm_and_stale_old_end_cannot_release_replacement() {
    let mut audio = queue();
    let mut pcm_fixture = local_pcm(1);
    let format = pcm_fixture.buffer.as_ref().unwrap().format();
    let old = audio
        .replace(
            &lease(),
            basis(),
            identity(1),
            pcm_fixture.reference.clone(),
            format,
            0,
        )
        .unwrap()
        .receipt;
    pcm_fixture.cache.lease_bytes(&pcm_fixture.source).unwrap();
    audio
        .enqueue(
            &old,
            &lease(),
            pcm_fixture.manifest,
            0,
            0,
            pcm_fixture.buffer.take().unwrap(),
        )
        .unwrap();
    audio.close(&old, &lease(), 1, FRAMES).unwrap();
    let old_ticket = audio.begin(&old, &lease()).unwrap().unwrap();
    let retained = audio.snapshot().sample_bytes;
    let mut next_pcm = local_pcm(2);
    let next = audio
        .replace(
            &lease(),
            basis(),
            identity(2),
            next_pcm.reference.clone(),
            next_pcm.buffer.as_ref().unwrap().format(),
            100,
        )
        .unwrap()
        .receipt;
    assert_eq!(audio.snapshot().state, QueueState::WaitingForStop);
    assert_eq!(audio.snapshot().sample_bytes, retained);
    assert_eq!(
        audio.confirm_stopped(&old_ticket),
        Ok(BufferCompletion::Retired(QueueState::Accepting))
    );
    next_pcm.cache.lease_bytes(&next_pcm.source).unwrap();
    audio
        .enqueue(
            &next,
            &lease(),
            next_pcm.manifest,
            0,
            100,
            next_pcm.buffer.take().unwrap(),
        )
        .unwrap();
    audio.close(&next, &lease(), 1, 100 + FRAMES).unwrap();
    let next_ticket = audio.begin(&next, &lease()).unwrap().unwrap();
    assert_eq!(audio.complete(&old_ticket), Err(QueueError::StaleCallback));
    assert_eq!(audio.snapshot().sample_bytes, (FRAMES * 8) as usize);
    assert_eq!(
        audio.complete(&next_ticket),
        Ok(BufferCompletion::Current(QueueState::Drained))
    );
    assert_eq!(audio.snapshot().sample_bytes, 0);
}

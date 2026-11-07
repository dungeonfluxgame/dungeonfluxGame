use crate::voice_capture::{CaptureBuffer, MicrophoneError, Taken, classify_media_error};

#[test]
fn every_error_key_round_trips() {
    for error in MicrophoneError::ALL {
        assert_eq!(MicrophoneError::from_key(error.key()), Some(error));
        assert!(!error.to_string().is_empty());
    }
    assert_eq!(MicrophoneError::from_key("nothing"), None);
}

#[test]
fn a_browser_exception_name_maps_to_a_distinct_reason() {
    assert_eq!(
        classify_media_error("NotAllowedError"),
        MicrophoneError::PermissionDenied
    );
    assert_eq!(
        classify_media_error("NotFoundError"),
        MicrophoneError::NoDevice
    );
    assert_eq!(
        classify_media_error("NotReadableError"),
        MicrophoneError::DeviceLost
    );
    assert_eq!(
        classify_media_error("TypeError"),
        MicrophoneError::Unsupported
    );
    assert_eq!(
        classify_media_error("SomethingNew"),
        MicrophoneError::Failed
    );
}

#[test]
fn a_recording_under_a_tenth_of_a_second_is_too_short() {
    let mut buffer = CaptureBuffer::new(48_000);
    buffer.take(&[0.1; 4_000]);
    assert_eq!(buffer.finish(), Err(MicrophoneError::TooShort));
}

#[test]
fn a_recording_of_exactly_a_tenth_of_a_second_is_sent() {
    let mut buffer = CaptureBuffer::new(48_000);
    buffer.take(&[0.1; 4_800]);
    let pcm = buffer.finish().expect("long enough");
    assert_eq!(pcm.len(), 3_200);
}

#[test]
fn a_recording_is_clipped_at_sixty_seconds_and_says_so() {
    let mut buffer = CaptureBuffer::new(16_000);
    let block = vec![0.2_f32; 16_000];
    let mut outcomes = Vec::new();
    for _ in 0..61 {
        outcomes.push(buffer.take(&block));
    }
    assert_eq!(outcomes[58], Taken::Going);
    assert_eq!(outcomes[59], Taken::ReachedLongest);
    assert_eq!(buffer.elapsed_ms(), 60_000);
    assert_eq!(buffer.finish().expect("recorded").len(), 1_920_000);
}

#[test]
fn a_block_that_crosses_the_longest_keeps_only_what_fits() {
    let mut buffer = CaptureBuffer::new(16_000);
    assert_eq!(buffer.take(&vec![0.1; 950_000]), Taken::Going);
    assert_eq!(buffer.take(&vec![0.1; 50_000]), Taken::ReachedLongest);
    assert_eq!(buffer.elapsed_ms(), 60_000);
}

#[test]
fn the_buffer_tracks_time_and_the_newest_level() {
    let mut buffer = CaptureBuffer::new(48_000);
    assert_eq!(buffer.elapsed_ms(), 0);
    buffer.take(&[0.05; 24_000]);
    assert_eq!(buffer.elapsed_ms(), 500);
    assert_eq!(buffer.level_permille(), 200);
    buffer.take(&[0.0; 100]);
    assert_eq!(buffer.level_permille(), 0);
}

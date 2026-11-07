use crate::voice_pcm::{
    duration_ms, level_permille, resample, samples_in, to_pcm16_le, to_speech_rate,
};

#[test]
fn a_second_of_samples_lasts_a_thousand_milliseconds_at_any_rate() {
    assert_eq!(duration_ms(48_000, 48_000), 1_000);
    assert_eq!(duration_ms(44_100, 44_100), 1_000);
    assert_eq!(duration_ms(1_600, 16_000), 100);
    assert_eq!(duration_ms(10, 0), 0);
}

#[test]
fn samples_in_a_span_round_trips_with_its_duration() {
    assert_eq!(samples_in(60_000, 16_000), 960_000);
    assert_eq!(duration_ms(samples_in(250, 48_000), 48_000), 250);
}

#[test]
fn silence_has_no_level_and_a_loud_block_is_full_scale() {
    assert_eq!(level_permille(&[]), 0);
    assert_eq!(level_permille(&[0.0; 64]), 0);
    assert_eq!(level_permille(&[1.0; 64]), 1_000);
    assert_eq!(level_permille(&[f32::NAN; 8]), 0);
}

#[test]
fn a_quiet_voice_moves_the_meter_in_proportion() {
    assert_eq!(level_permille(&[0.05; 128]), 200);
    assert_eq!(level_permille(&[0.1; 128]), 400);
}

#[test]
fn the_same_rate_is_returned_as_it_is() {
    let samples = [0.1, -0.2, 0.3];
    assert_eq!(resample(&samples, 16_000, 16_000), samples);
}

#[test]
fn no_samples_or_a_zero_rate_gives_nothing() {
    assert!(resample(&[], 48_000, 16_000).is_empty());
    assert!(resample(&[0.5; 10], 0, 16_000).is_empty());
    assert!(resample(&[0.5; 10], 48_000, 0).is_empty());
}

#[test]
fn going_from_48_to_16_kilohertz_averages_each_three_samples() {
    let samples = [0.0, 0.3, 0.6, 0.9, 0.9, 0.9];
    let resampled = resample(&samples, 48_000, 16_000);
    assert_eq!(resampled.len(), 2);
    assert!((resampled[0] - 0.3).abs() < 1e-6, "{resampled:?}");
    assert!((resampled[1] - 0.9).abs() < 1e-6, "{resampled:?}");
}

#[test]
fn a_fractional_ratio_keeps_a_constant_signal_constant() {
    let samples = vec![0.25_f32; 4_410];
    let resampled = to_speech_rate(&samples, 44_100);
    assert_eq!(resampled.len(), 1_600);
    assert!(resampled.iter().all(|value| (value - 0.25).abs() < 1e-5));
}

#[test]
fn a_one_second_recording_at_any_rate_is_sixteen_thousand_samples() {
    for rate in [8_000_u32, 22_050, 32_000, 44_100, 48_000, 96_000] {
        let samples = vec![0.0_f32; rate as usize];
        let resampled = to_speech_rate(&samples, rate);
        assert_eq!(resampled.len(), 16_000, "{rate}");
    }
}

#[test]
fn going_up_interpolates_between_neighbours() {
    let resampled = resample(&[0.0, 1.0], 8_000, 16_000);
    assert_eq!(resampled.len(), 4);
    assert!((resampled[1] - 0.5).abs() < 1e-6, "{resampled:?}");
}

#[test]
fn pcm_is_little_endian_signed_and_clipped() {
    let bytes = to_pcm16_le(&[0.0, 1.0, -1.0, 2.0, -3.0, f32::NAN, 0.5]);
    let values: Vec<i16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| i16::from_le_bytes(*pair))
        .collect();
    assert_eq!(values, vec![0, 32_767, -32_767, 32_767, -32_767, 0, 16_384]);
    assert_eq!(bytes.len(), 14);
}

#[test]
fn a_sine_at_a_high_rate_keeps_its_loudness_when_resampled() {
    let rate = 48_000_u32;
    let samples: Vec<f32> = (0..rate as usize)
        .map(|index| (index as f32 * 2.0 * std::f32::consts::PI * 440.0 / rate as f32).sin() * 0.5)
        .collect();
    let resampled = to_speech_rate(&samples, rate);
    let before = level_permille(&samples);
    let after = level_permille(&resampled);
    assert!(before.abs_diff(after) <= 20, "{before} against {after}");
}

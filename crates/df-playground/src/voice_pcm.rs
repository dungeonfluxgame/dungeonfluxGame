//! The numbers behind a recording: how long it is, how loud it is now, and how the browser's
//! samples become what the speech-to-text adapter reads (16 kHz, mono, signed 16-bit
//! little-endian). Plain Rust that native tests cover; the browser only hands over the samples.

use crate::limits::CAPTURE_SAMPLE_RATE_HZ;

/// The scale from a block's RMS to the level meter: speech sits well under full scale, so a plain
/// RMS would barely move the meter. Display only; nothing decides on it.
const LEVEL_GAIN: f32 = 4.0;
/// The meter's full scale, in thousandths.
const FULL_LEVEL_PERMILLE: u16 = 1_000;

/// How long `samples` samples last at `rate_hz`, in milliseconds. A rate of 0 lasts no time.
pub fn duration_ms(samples: usize, rate_hz: u32) -> u64 {
    if rate_hz == 0 {
        return 0;
    }
    let samples = u64::try_from(samples).unwrap_or(u64::MAX);
    samples.saturating_mul(1_000) / u64::from(rate_hz)
}

/// How many samples last `milliseconds` at `rate_hz`.
pub fn samples_in(milliseconds: u64, rate_hz: u32) -> usize {
    let samples = milliseconds.saturating_mul(u64::from(rate_hz)) / 1_000;
    usize::try_from(samples).unwrap_or(usize::MAX)
}

/// The loudness of a block of samples (each from -1.0 to 1.0) for the level meter, from 0 to 1000.
pub fn level_permille(samples: &[f32]) -> u16 {
    if samples.is_empty() {
        return 0;
    }
    let energy: f32 = samples
        .iter()
        .map(|sample| {
            if sample.is_finite() {
                sample * sample
            } else {
                0.0
            }
        })
        .sum();
    let rms = (energy / samples.len() as f32).sqrt();
    let level = (rms * LEVEL_GAIN * f32::from(FULL_LEVEL_PERMILLE)).round();
    // A value in [0, 1000] after the clamp, so the conversion holds.
    level.clamp(0.0, f32::from(FULL_LEVEL_PERMILLE)) as u16
}

/// `samples` read at `from_hz`, resampled to `to_hz`. Going down, each output sample is the mean of
/// the input it covers, which keeps the high band from folding back into speech; going up, it is
/// linear interpolation. A rate of 0 or no samples gives no samples.
pub fn resample(samples: &[f32], from_hz: u32, to_hz: u32) -> Vec<f32> {
    if samples.is_empty() || from_hz == 0 || to_hz == 0 {
        return Vec::new();
    }
    if from_hz == to_hz {
        return samples.to_vec();
    }
    let output_len = samples.len() as u64 * u64::from(to_hz) / u64::from(from_hz);
    let output_len = usize::try_from(output_len).unwrap_or(usize::MAX);
    let step = f64::from(from_hz) / f64::from(to_hz);
    (0..output_len)
        .map(|index| {
            let start = index as f64 * step;
            if step >= 1.0 {
                mean_over(samples, start, start + step)
            } else {
                interpolate_at(samples, start)
            }
        })
        .collect()
}

/// The mean of `samples` over the span from `start` to `end` (in samples), each sample weighted by
/// how much of the span it covers.
fn mean_over(samples: &[f32], start: f64, end: f64) -> f32 {
    let end = end.min(samples.len() as f64);
    let first = start.floor() as usize;
    let mut sum = 0.0_f64;
    let mut covered = 0.0_f64;
    for (index, sample) in samples.iter().enumerate().skip(first) {
        let from = (index as f64).max(start);
        let to = (index as f64 + 1.0).min(end);
        if from >= end {
            break;
        }
        if to <= from {
            continue;
        }
        sum += f64::from(*sample) * (to - from);
        covered += to - from;
    }
    if covered > 0.0 {
        (sum / covered) as f32
    } else {
        0.0
    }
}

/// The value at the fractional position `at` in `samples`, between its two neighbours.
fn interpolate_at(samples: &[f32], at: f64) -> f32 {
    let below = at.floor() as usize;
    let fraction = (at - at.floor()) as f32;
    let first = samples.get(below).copied().unwrap_or(0.0);
    let second = samples.get(below + 1).copied().unwrap_or(first);
    first + (second - first) * fraction
}

/// `samples` resampled from the browser's rate to the one the speech adapter reads.
pub fn to_speech_rate(samples: &[f32], from_hz: u32) -> Vec<f32> {
    resample(samples, from_hz, CAPTURE_SAMPLE_RATE_HZ)
}

/// `samples` (each from -1.0 to 1.0) as signed 16-bit little-endian PCM. A sample outside the range
/// is clipped, and one that is not a number is silence.
pub fn to_pcm16_le(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        let clipped = if sample.is_finite() {
            sample.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        // In [-32767, 32767] after the clamp, so the conversion holds.
        let value = (clipped * f32::from(i16::MAX)).round() as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

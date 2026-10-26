//! Post-processing of generated audio, as done for the shipped score: cut to the exact bar
//! grid, make the loop seamless with a tail crossfade, and set the integrated loudness.
//!
//! The loop cut and the loudness measurement (ITU-R BS.1770-4, gated) are plain Rust on decoded
//! stereo frames. Only MP3 decoding and encoding is delegated to the `ffmpeg` binary, called
//! with a fixed argument list (no shell) on private scratch files, with a bounded wait. The repo
//! has no MP3 codec crate and no existing ffmpeg tooling; `DF_FFMPEG` or `ffmpeg` on `PATH`
//! supplies the codec.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use crate::music_score::SAMPLE_RATE;

/// One stereo frame.
pub(super) type Frame = [f32; 2];

/// The shortest tail that still hides the loop point: 20 ms.
const MINIMUM_CROSSFADE_FRAMES: usize = SAMPLE_RATE as usize / 50;
/// A normalised layer never peaks above this, even when that leaves it under the target.
const PEAK_CEILING_DBFS: f64 = -1.0;
const MP3_KILOBITS: &str = "160k";

/// Why a piece of audio could not be mastered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum MasterFailure {
    /// The source is shorter than the loop plus the shortest tail.
    TooShort,
    /// The source is silent, so no gain reaches the target.
    Silent,
    /// ffmpeg is not installed or could not be started.
    CodecMissing,
    /// ffmpeg failed or produced nothing usable.
    CodecFailed,
    /// ffmpeg did not finish in time.
    CodecDeadline,
}

impl MasterFailure {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::TooShort => "source_too_short",
            Self::Silent => "source_silent",
            Self::CodecMissing => "ffmpeg_missing",
            Self::CodecFailed => "ffmpeg_failed",
            Self::CodecDeadline => "ffmpeg_deadline",
        }
    }
}

/// Cuts `source` to exactly `loop_frames` and blends the audio that follows the cut into the
/// start with an equal-power crossfade, so the last frame runs on into the first without a
/// click. The crossfade is `crossfade_frames`, shortened to the tail the source has.
pub(super) fn loop_cut(
    source: &[Frame],
    loop_frames: usize,
    crossfade_frames: usize,
) -> Result<Vec<Frame>, MasterFailure> {
    let tail = source.len().saturating_sub(loop_frames);
    let fade = crossfade_frames.min(tail).min(loop_frames / 2);
    if source.len() < loop_frames || fade < MINIMUM_CROSSFADE_FRAMES {
        return Err(MasterFailure::TooShort);
    }
    let mut looped = source[..loop_frames].to_vec();
    for index in 0..fade {
        let position = (index as f64 + 0.5) / fade as f64;
        let rising = (position * std::f64::consts::FRAC_PI_2).sin() as f32;
        let falling = (position * std::f64::consts::FRAC_PI_2).cos() as f32;
        let head = source[index];
        let continuation = source[loop_frames + index];
        for channel in 0..2 {
            looped[index][channel] = head[channel] * rising + continuation[channel] * falling;
        }
    }
    Ok(looped)
}

#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
}

impl Biquad {
    fn run(self, samples: &[f64]) -> Vec<f64> {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        samples
            .iter()
            .map(|&x| {
                let y = self.b[0] * x + self.b[1] * x1 + self.b[2] * x2
                    - self.a[0] * y1
                    - self.a[1] * y2;
                (x2, x1, y2, y1) = (x1, x, y1, y);
                y
            })
            .collect()
    }
}

/// The two K-weighting stages of BS.1770-4 for `rate`: a high shelf and the RLB high-pass.
fn k_weighting(rate: f64) -> [Biquad; 2] {
    let shelf = {
        let (f0, gain_db, q) = (
            1_681.974_450_955_533,
            3.999_843_853_973_347,
            0.707_175_236_955_419_6,
        );
        let k = (std::f64::consts::PI * f0 / rate).tan();
        let vh = 10f64.powf(gain_db / 20.0);
        let vb = vh.powf(0.499_666_774_154_541_6);
        let a0 = 1.0 + k / q + k * k;
        Biquad {
            b: [
                (vh + vb * k / q + k * k) / a0,
                2.0 * (k * k - vh) / a0,
                (vh - vb * k / q + k * k) / a0,
            ],
            a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        }
    };
    let high_pass = {
        let (f0, q) = (38.135_470_876_024_44, 0.500_327_037_323_877_3);
        let k = (std::f64::consts::PI * f0 / rate).tan();
        let a0 = 1.0 + k / q + k * k;
        Biquad {
            b: [1.0, -2.0, 1.0],
            a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        }
    };
    [shelf, high_pass]
}

/// Integrated loudness in LUFS of stereo `frames` at the score's sample rate: 400 ms blocks
/// every 100 ms, an absolute gate at -70 LUFS and a relative gate 10 LU below the gated mean.
/// `None` when the audio is shorter than one block or entirely below the absolute gate.
pub(super) fn integrated_lufs(frames: &[Frame]) -> Option<f64> {
    let rate = f64::from(SAMPLE_RATE);
    let block = (rate * 0.4) as usize;
    let hop = (rate * 0.1) as usize;
    if frames.len() < block {
        return None;
    }
    let filters = k_weighting(rate);
    let weighted: Vec<Vec<f64>> = (0..2)
        .map(|channel| {
            let samples: Vec<f64> = frames
                .iter()
                .map(|frame| f64::from(frame[channel]))
                .collect();
            filters[1].run(&filters[0].run(&samples))
        })
        .collect();
    let mut energies = Vec::new();
    let mut start = 0;
    while start + block <= frames.len() {
        let energy: f64 = weighted
            .iter()
            .map(|channel| {
                channel[start..start + block]
                    .iter()
                    .map(|v| v * v)
                    .sum::<f64>()
                    / block as f64
            })
            .sum();
        energies.push(energy);
        start += hop;
    }
    let loudness = |energy: f64| -0.691 + 10.0 * energy.log10();
    let absolute: Vec<f64> = energies
        .iter()
        .copied()
        .filter(|&energy| energy > 0.0 && loudness(energy) > -70.0)
        .collect();
    if absolute.is_empty() {
        return None;
    }
    let relative_gate = loudness(absolute.iter().sum::<f64>() / absolute.len() as f64) - 10.0;
    let gated: Vec<f64> = absolute
        .into_iter()
        .filter(|&energy| loudness(energy) > relative_gate)
        .collect();
    (!gated.is_empty()).then(|| loudness(gated.iter().sum::<f64>() / gated.len() as f64))
}

/// What a normalisation did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Normalised {
    pub(super) measured_lufs: f64,
    pub(super) gain_db: f64,
    /// False when the peak ceiling stopped the gain short of the target.
    pub(super) reached_target: bool,
}

/// Scales `frames` to `target_lufs` integrated loudness, but never beyond the peak ceiling.
pub(super) fn normalise(
    frames: &mut [Frame],
    target_lufs: f64,
) -> Result<Normalised, MasterFailure> {
    let measured_lufs = integrated_lufs(frames).ok_or(MasterFailure::Silent)?;
    let peak = frames
        .iter()
        .flatten()
        .fold(0.0_f64, |peak, &sample| peak.max(f64::from(sample).abs()));
    if peak == 0.0 {
        return Err(MasterFailure::Silent);
    }
    let wanted_db = target_lufs - measured_lufs;
    let ceiling_db = PEAK_CEILING_DBFS - 20.0 * peak.log10();
    let gain_db = wanted_db.min(ceiling_db);
    let gain = 10f64.powf(gain_db / 20.0) as f32;
    for sample in frames.iter_mut().flatten() {
        *sample *= gain;
    }
    Ok(Normalised {
        measured_lufs,
        gain_db,
        reached_target: gain_db >= wanted_db - 1e-9,
    })
}

/// Converts between MP3 and raw stereo frames.
pub(in crate::gameplay) enum Codec {
    Ffmpeg(Ffmpeg),
    /// Tests: the "MP3" is raw little-endian f32 stereo.
    #[cfg(test)]
    Raw,
}

impl Codec {
    pub(super) async fn decode(
        &self,
        mp3: Vec<u8>,
        deadline: Duration,
    ) -> Result<Vec<Frame>, MasterFailure> {
        match self {
            Self::Ffmpeg(ffmpeg) => ffmpeg.decode(mp3, deadline).await,
            #[cfg(test)]
            Self::Raw => Ok(raw_decode(&mp3)),
        }
    }

    pub(super) async fn encode(
        &self,
        frames: Vec<Frame>,
        deadline: Duration,
    ) -> Result<Vec<u8>, MasterFailure> {
        match self {
            Self::Ffmpeg(ffmpeg) => ffmpeg.encode(frames, deadline).await,
            #[cfg(test)]
            Self::Raw => Ok(raw_bytes(&frames)),
        }
    }
}

pub(super) fn frames_of(bytes: &[u8]) -> Vec<Frame> {
    bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(
            |&[
                left_0,
                left_1,
                left_2,
                left_3,
                right_0,
                right_1,
                right_2,
                right_3,
            ]| {
                [
                    f32::from_le_bytes([left_0, left_1, left_2, left_3]),
                    f32::from_le_bytes([right_0, right_1, right_2, right_3]),
                ]
            },
        )
        .collect()
}

fn raw_bytes(frames: &[Frame]) -> Vec<u8> {
    frames
        .iter()
        .flatten()
        .flat_map(|sample| sample.to_le_bytes())
        .collect()
}

/// The ffmpeg binary, run as a bounded child process on scratch files.
pub(in crate::gameplay) struct Ffmpeg {
    binary: PathBuf,
    scratch: PathBuf,
}

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

impl Ffmpeg {
    /// `binary` is the executable; `scratch` is a directory private to this process's music
    /// work (created on demand, one subdirectory per job, removed after it).
    pub(in crate::gameplay) fn new(binary: PathBuf, scratch: PathBuf) -> Self {
        Self { binary, scratch }
    }

    /// Whether the binary starts and answers `-version`.
    pub(in crate::gameplay) fn available(binary: &Path) -> bool {
        Command::new(binary)
            .arg("-version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    async fn decode(&self, mp3: Vec<u8>, deadline: Duration) -> Result<Vec<Frame>, MasterFailure> {
        let (binary, scratch) = (self.binary.clone(), self.scratch.clone());
        tokio::task::spawn_blocking(move || {
            let job = Job::new(&scratch)?;
            std::fs::write(job.path("in.mp3"), &mp3).map_err(|_| MasterFailure::CodecFailed)?;
            run(
                &binary,
                &[
                    "-nostdin",
                    "-v",
                    "error",
                    "-y",
                    "-i",
                    &job.arg("in.mp3"),
                    "-f",
                    "f32le",
                    "-acodec",
                    "pcm_f32le",
                    "-ac",
                    "2",
                    "-ar",
                    &SAMPLE_RATE.to_string(),
                    &job.arg("out.raw"),
                ],
                deadline,
            )?;
            let raw = std::fs::read(job.path("out.raw")).map_err(|_| MasterFailure::CodecFailed)?;
            let frames = frames_of(&raw);
            if frames.is_empty() {
                return Err(MasterFailure::CodecFailed);
            }
            Ok(frames)
        })
        .await
        .map_err(|_| MasterFailure::CodecFailed)?
    }

    async fn encode(
        &self,
        frames: Vec<Frame>,
        deadline: Duration,
    ) -> Result<Vec<u8>, MasterFailure> {
        let (binary, scratch) = (self.binary.clone(), self.scratch.clone());
        tokio::task::spawn_blocking(move || {
            let job = Job::new(&scratch)?;
            std::fs::write(job.path("in.raw"), raw_bytes(&frames))
                .map_err(|_| MasterFailure::CodecFailed)?;
            run(
                &binary,
                &[
                    "-nostdin",
                    "-v",
                    "error",
                    "-y",
                    "-f",
                    "f32le",
                    "-ar",
                    &SAMPLE_RATE.to_string(),
                    "-ac",
                    "2",
                    "-i",
                    &job.arg("in.raw"),
                    "-c:a",
                    "libmp3lame",
                    "-b:a",
                    MP3_KILOBITS,
                    "-f",
                    "mp3",
                    &job.arg("out.mp3"),
                ],
                deadline,
            )?;
            let mp3 = std::fs::read(job.path("out.mp3")).map_err(|_| MasterFailure::CodecFailed)?;
            if mp3.is_empty() {
                return Err(MasterFailure::CodecFailed);
            }
            Ok(mp3)
        })
        .await
        .map_err(|_| MasterFailure::CodecFailed)?
    }
}

/// A private scratch directory removed when the job ends.
struct Job {
    directory: PathBuf,
}

impl Job {
    fn new(scratch: &Path) -> Result<Self, MasterFailure> {
        let directory = scratch.join(format!(
            "job-{}-{}",
            std::process::id(),
            SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).map_err(|_| MasterFailure::CodecFailed)?;
        Ok(Self { directory })
    }

    fn path(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }

    fn arg(&self, name: &str) -> String {
        self.path(name).to_string_lossy().into_owned()
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // Best effort: scratch space only; nothing depends on the removal.
        let _removed = std::fs::remove_dir_all(&self.directory);
    }
}

/// Runs `binary` with `args` and waits at most `deadline`; a child still running then is killed.
fn run(binary: &Path, args: &[&str], deadline: Duration) -> Result<(), MasterFailure> {
    let mut child = Command::new(binary)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| MasterFailure::CodecMissing)?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) | Err(_) => return Err(MasterFailure::CodecFailed),
            Ok(None) if started.elapsed() >= deadline => {
                // The child is abandoned either way; a failed kill leaves nothing to report.
                let _killed = child.kill();
                let _reaped = child.wait();
                return Err(MasterFailure::CodecDeadline);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

#[cfg(test)]
pub(super) fn tone(frames: usize, hertz: f64, amplitude: f32) -> Vec<Frame> {
    (0..frames)
        .map(|index| {
            let phase = 2.0 * std::f64::consts::PI * hertz * index as f64 / f64::from(SAMPLE_RATE);
            let sample = phase.sin() as f32 * amplitude;
            [sample, sample]
        })
        .collect()
}

/// Test "MP3" bytes: `TONE`, a little-endian u32 frame count and an f32 amplitude make a tone;
/// anything else is raw f32 stereo frames.
#[cfg(test)]
fn raw_decode(bytes: &[u8]) -> Vec<Frame> {
    if let Some(rest) = bytes.strip_prefix(b"TONE")
        && let (Some(count), Some(amplitude)) = (rest.get(0..4), rest.get(4..8))
    {
        let count = u32::from_le_bytes([count[0], count[1], count[2], count[3]]);
        let amplitude =
            f32::from_le_bytes([amplitude[0], amplitude[1], amplitude[2], amplitude[3]]);
        return tone(count as usize, 220.0, amplitude);
    }
    frames_of(bytes)
}

/// The bytes a test fake sends for a tone of `frames` frames.
#[cfg(test)]
pub(super) fn tone_bytes(frames: u32, amplitude: f32) -> Vec<u8> {
    let mut bytes = b"TONE".to_vec();
    bytes.extend(frames.to_le_bytes());
    bytes.extend(amplitude.to_le_bytes());
    bytes
}

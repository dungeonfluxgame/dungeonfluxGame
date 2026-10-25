//! The place-music resource kind: the spec the generation hub keys on, and the generator the hub
//! invokes to make one set.
//!
//! A set is four music layers (bed, motif, tension, combat) from `POST /v1/music`, all at the
//! selected pacing band's BPM, one key and a common loop duration, plus one ambience bed from
//! `POST /v1/sound-generation`. Each is cut to the exact loop, made seamless with a tail
//! crossfade and set to the shipped loudness (see `master`). The generator makes the calls and
//! returns the mastered files; it keeps no cache, no in-flight registry and no retry. The hub that
//! invokes it owns those. Each provider call is recorded on the hub's spend ledger
//! ([`Unit::MusicCall`], [`PROVIDER_CALLS`] per set) and metered in dollars on its cost book
//! (`music_v2` seconds per layer, `eleven_text_to_sound_v2` seconds for the ambience).

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use df_providers::{
    ElevenMusicConfig, ElevenMusicError, NativeElevenMusicProvider, NativeHttpRefusal,
};
use serde_json::json;
use tokio::sync::Semaphore;

use crate::gameplay::{
    cost::{
        BilledCall, Category, ambience_call, music_error_may_be_billed, music_layer_call,
        settle_counted, settle_failure,
    },
    resources::{SpendLedger, Unit},
};
use crate::music_score::{LAYER_LUFS, LOOP_SAMPLES, PacingBand, SAMPLE_RATE};

use super::master::{Codec, Frame, MasterFailure, loop_cut, normalise};
use super::spec::{
    AMBIENCE_LOOP_SECONDS, AMBIENCE_SECONDS, LAYER_REQUEST_MILLIS, Layer, MusicSpec,
};

/// Bumped when the prompts or the mastering change, so a stored set is not reused for new ones.
pub(in crate::gameplay) const GENERATOR_VERSION: u32 = 2;
/// Provider calls one set costs: four `/v1/music` and one `/v1/sound-generation`.
pub(in crate::gameplay) const PROVIDER_CALLS: u32 = 5;
/// The shipped ambience beds are mastered to -30 LUFS.
const AMBIENCE_LUFS: f64 = -30.0;
const AMBIENCE_CROSSFADE_MILLIS: u32 = 3_000;
/// Provider calls in flight at once within one set.
const SET_CONCURRENCY: usize = 2;

/// The hub's resource spec for a place's music: the canonical [`MusicSpec`] and a version.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct MusicSetSpec {
    spec: MusicSpec,
    version: u32,
    band: PacingBand,
}

impl MusicSetSpec {
    pub(in crate::gameplay) fn new(spec: MusicSpec) -> Self {
        Self {
            spec,
            version: GENERATOR_VERSION,
            band: PacingBand::Steady,
        }
    }

    pub(in crate::gameplay) fn in_band(spec: MusicSpec, band: PacingBand) -> Self {
        Self {
            band,
            ..Self::new(spec)
        }
    }

    pub(in crate::gameplay) fn band(&self) -> PacingBand {
        self.band
    }

    /// A symbolic motif reference shared by the original set and every pacing variant.
    /// The provider's compose API accepts text, so the seed and exact scale-degree phrase
    /// travel in every layer prompt, rather than pretending it supports audio conditioning.
    pub(in crate::gameplay) fn motif_reference(&self) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(self.spec.canonical().as_bytes());
        let degrees = digest[..4]
            .iter()
            .map(|byte| (1 + byte % 5).to_string())
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "theme seed {}; recurring scale degrees 1 {degrees} 5 1 in {}; retain this \
             phrase, contour and instrumentation in every pacing variant",
            Self::new(self.spec.clone()).key(),
            self.spec.key_name(),
        )
    }

    pub(in crate::gameplay) fn spec(&self) -> &MusicSpec {
        &self.spec
    }

    pub(in crate::gameplay) fn version(&self) -> u32 {
        self.version
    }

    /// The hub's key for this set: a hash of the kind, the generator version and the canonical
    /// spec. Two places with the same spec share one set.
    pub(in crate::gameplay) fn key(&self) -> String {
        super::resource::MusicSetResource::new(self.clone())
            .key()
            .as_str()
            .to_owned()
    }
}

/// The mastered files of one set, as MP3 bytes, and the provenance the hub records.
pub(in crate::gameplay) struct GeneratedMusicSet {
    /// Bed, motif, tension, combat: each exactly 2,016,000 frames in the selected pacing band.
    pub(in crate::gameplay) layers: [Vec<u8>; 4],
    /// Absent when only the ambience call failed; the place keeps its fallback bed.
    pub(in crate::gameplay) ambience: Option<Vec<u8>>,
    pub(in crate::gameplay) manifest: serde_json::Value,
}

/// Why a set was not made.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum FailureReason {
    Deadline,
    Interrupted,
    InvalidRequest,
    Unauthorized,
    /// The provider answered with this status.
    Provider(u16),
    Transport,
    EmptyOutput,
    Master(MasterFailure),
}

impl FailureReason {
    /// A closed, secret-free label for the hub's audit lines.
    pub(in crate::gameplay) fn label(self) -> &'static str {
        match self {
            Self::Deadline => "deadline",
            Self::Interrupted => "interrupted",
            Self::InvalidRequest => "invalid_request",
            Self::Unauthorized => "unauthorized",
            Self::Provider(429) => "provider_rate_limited",
            Self::Provider(400..=499) => "provider_rejected",
            Self::Provider(_) => "provider_error",
            Self::Transport => "transport",
            Self::EmptyOutput => "empty_output",
            Self::Master(failure) => failure.label(),
        }
    }

    /// Whether the same request would fail again, so the hub should not retry it: a refusal, an
    /// invalid request, a client error other than rate limiting, or unusable source audio.
    pub(in crate::gameplay) fn is_permanent(self) -> bool {
        match self {
            Self::InvalidRequest | Self::Unauthorized => true,
            Self::Provider(status) => {
                (400..=499).contains(&status) && status != 429 && status != 408
            }
            Self::Master(failure) => matches!(
                failure,
                MasterFailure::TooShort | MasterFailure::Silent | MasterFailure::CodecMissing
            ),
            Self::Deadline | Self::Interrupted | Self::Transport | Self::EmptyOutput => false,
        }
    }

    /// The label plus the provider status, for the dev status.
    pub(in crate::gameplay) fn detail(self) -> String {
        match self {
            Self::Provider(status) => format!("provider_status_{status}"),
            other => other.label().to_owned(),
        }
    }
}

/// Where and why a set failed. `stage` names the failing piece; neither part carries a prompt,
/// a credential or provider text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct GeneratorFailure {
    pub(in crate::gameplay) stage: &'static str,
    pub(in crate::gameplay) reason: FailureReason,
}

impl std::fmt::Display for GeneratorFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.stage, self.reason.detail())
    }
}

fn provider_reason(error: ElevenMusicError) -> FailureReason {
    match error {
        ElevenMusicError::InvalidRequest => FailureReason::InvalidRequest,
        ElevenMusicError::Unauthorized => FailureReason::Unauthorized,
        ElevenMusicError::Provider(status) => FailureReason::Provider(status),
        ElevenMusicError::Transport(NativeHttpRefusal::Deadline) => FailureReason::Deadline,
        ElevenMusicError::Transport(_) => FailureReason::Transport,
        ElevenMusicError::Output => FailureReason::EmptyOutput,
    }
}

/// One mastered piece and what was measured making it.
struct Mastered {
    mp3: Vec<u8>,
    detail: serde_json::Value,
}

pub(in crate::gameplay) struct MusicGenerator {
    provider: NativeElevenMusicProvider,
    codec: Codec,
    permits: Semaphore,
    /// Where every provider call is recorded and metered.
    ledger: Arc<SpendLedger>,
}

impl MusicGenerator {
    /// A standalone sound-generation resource uses the identical seamless mastering path.
    pub(super) async fn ambient_layer(
        &self,
        spec: &super::ambience::AmbientSpec,
        deadline: Duration,
    ) -> Result<Vec<u8>, FailureReason> {
        let started = Instant::now();
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|_| FailureReason::Interrupted)?;
        let billed = self.begin(ambience_call("ambient", u64::from(AMBIENCE_SECONDS)));
        let audio = match self
            .provider
            .ambience(
                &spec.prompt(),
                AMBIENCE_SECONDS,
                Self::remaining(started, deadline),
            )
            .await
        {
            Ok(audio) => audio,
            Err(error) => {
                self.failed(billed, error);
                return Err(provider_reason(error));
            }
        };
        Self::billed(billed, u64::from(AMBIENCE_SECONDS));
        self.master(
            audio.bytes,
            (AMBIENCE_LOOP_SECONDS * SAMPLE_RATE) as usize,
            AMBIENCE_CROSSFADE_MILLIS,
            AMBIENCE_LUFS,
            started,
            deadline,
        )
        .await
        .map(|(bytes, _)| bytes)
        .map_err(FailureReason::Master)
    }
    pub(in crate::gameplay) fn new(
        provider: NativeElevenMusicProvider,
        codec: Codec,
        ledger: Arc<SpendLedger>,
    ) -> Self {
        Self {
            provider,
            codec,
            permits: Semaphore::new(SET_CONCURRENCY),
            ledger,
        }
    }

    /// The live provider with the ffmpeg codec. `call_deadline` bounds one provider call.
    pub(in crate::gameplay) fn live(
        credential: &str,
        call_deadline: Duration,
        codec: Codec,
        ledger: Arc<SpendLedger>,
    ) -> Result<Self, ElevenMusicError> {
        Ok(Self::new(
            NativeElevenMusicProvider::new(ElevenMusicConfig::live(), credential, call_deadline)?,
            codec,
            ledger,
        ))
    }

    /// The same generator recording on `ledger` (tests bind it to the hub under test).
    #[cfg(test)]
    pub(super) fn bound_to(mut self, ledger: Arc<SpendLedger>) -> Self {
        self.ledger = ledger;
        self
    }

    /// Records one provider call about to be sent: a unit of [`Unit::MusicCall`], kept whatever
    /// the outcome (a call of unknown outcome may have been billed), and the metered call.
    fn begin(&self, spec: crate::gameplay::cost::CallSpec) -> BilledCall {
        self.ledger.reserve(Unit::MusicCall, 1).commit(1);
        self.ledger.begin_call(spec, &[])
    }

    /// Settles a call that returned audio: billed at the seconds asked for.
    fn billed(billed: BilledCall, seconds: u64) {
        settle_counted(billed, vec![(Category::AudioSeconds, seconds)]);
    }

    /// Settles a call that failed: given back when the supplier cannot have billed it, else of
    /// unknown outcome.
    fn failed(&self, billed: BilledCall, error: ElevenMusicError) {
        let may_be_billed = music_error_may_be_billed(error);
        if !may_be_billed {
            self.ledger.reconcile(Unit::MusicCall, -1);
        }
        settle_failure(billed, may_be_billed);
    }

    /// Makes every file of `set` within `deadline` in total. Any failed music layer fails the
    /// set (a partial set would not stack); a failed ambience leaves the set without one.
    pub(in crate::gameplay) async fn generate(
        &self,
        set: &MusicSetSpec,
        deadline: Duration,
    ) -> Result<GeneratedMusicSet, GeneratorFailure> {
        let started = Instant::now();
        let work = self.generate_within(set, started, deadline);
        match tokio::time::timeout(deadline, work).await {
            Ok(result) => result,
            Err(_) => Err(GeneratorFailure {
                stage: "set",
                reason: FailureReason::Deadline,
            }),
        }
    }

    async fn generate_within(
        &self,
        set: &MusicSetSpec,
        started: Instant,
        deadline: Duration,
    ) -> Result<GeneratedMusicSet, GeneratorFailure> {
        let spec = set.spec();
        let layers = Layer::ALL.map(|layer| self.layer(set, layer, started, deadline));
        let ambience = self.ambience(spec, started, deadline);
        let (layers, ambience) = tokio::join!(futures::future::join_all(layers), ambience);
        let mut files = Vec::with_capacity(4);
        let mut details = Vec::with_capacity(4);
        for (layer, result) in Layer::ALL.into_iter().zip(layers) {
            let mastered = result.map_err(|failure| GeneratorFailure {
                stage: layer.name(),
                reason: failure,
            })?;
            files.push(mastered.mp3);
            details.push(mastered.detail);
        }
        let layers: [Vec<u8>; 4] = files.try_into().map_err(|_| GeneratorFailure {
            stage: "set",
            reason: FailureReason::Interrupted,
        })?;
        let (ambience_file, ambience_detail) = match ambience {
            Ok(mastered) => (Some(mastered.mp3), mastered.detail),
            Err(reason) => (None, json!({"failed": reason.detail()})),
        };
        let manifest = json!({
            "key": set.key(),
            "version": set.version(),
            "spec": spec,
            "label": spec.label(),
            "provider_calls": PROVIDER_CALLS,
            "bpm": set.band().bpm(),
            "band": set.band().label(),
            "energy_permille": set.band().energy(),
            "bars": set.band().bars(),
            "motif_reference": set.motif_reference(),
            "key_name": spec.key_name(),
            "loop_samples": LOOP_SAMPLES,
            "sample_rate": SAMPLE_RATE,
            "layers": Layer::ALL.iter().map(|layer| layer.name()).zip(details).map(|(name, detail)| json!({"layer": name, "detail": detail})).collect::<Vec<_>>(),
            "ambience": ambience_detail,
            "elapsed_ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        });
        Ok(GeneratedMusicSet {
            layers,
            ambience: ambience_file,
            manifest,
        })
    }

    fn remaining(started: Instant, deadline: Duration) -> Duration {
        deadline.saturating_sub(started.elapsed())
    }

    async fn layer(
        &self,
        set: &MusicSetSpec,
        layer: Layer,
        started: Instant,
        deadline: Duration,
    ) -> Result<Mastered, FailureReason> {
        let prompt = format!(
            "{} Theme reference: {}. Pacing energy {}/1000; change rhythmic density, \
             retain the harmony, melody and acoustic palette.",
            set.spec().layer_prompt_at(layer, set.band()),
            set.motif_reference(),
            set.band().energy(),
        );
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|_| FailureReason::Interrupted)?;
        let call_started = Instant::now();
        let seconds = u64::from(LAYER_REQUEST_MILLIS).div_ceil(1_000);
        let billed = self.begin(music_layer_call(layer.name(), seconds));
        let audio = match self
            .provider
            .compose(
                &prompt,
                LAYER_REQUEST_MILLIS,
                Self::remaining(started, deadline),
            )
            .await
        {
            Ok(audio) => audio,
            Err(error) => {
                self.failed(billed, error);
                return Err(provider_reason(error));
            }
        };
        Self::billed(billed, seconds);
        let provider_ms = call_started.elapsed().as_millis();
        let detail = self
            .master(
                audio.bytes,
                LOOP_SAMPLES as usize,
                layer.crossfade_millis(),
                f64::from(LAYER_LUFS),
                started,
                deadline,
            )
            .await
            .map_err(FailureReason::Master)?;
        Ok(Mastered {
            mp3: detail.0,
            detail: json!({
                "prompt": prompt,
                "provider_ms": provider_ms as u64,
                "crossfade_ms": layer.crossfade_millis(),
                "measured_lufs": detail.1.measured_lufs,
                "gain_db": detail.1.gain_db,
                "reached_target": detail.1.reached_target,
            }),
        })
    }

    async fn ambience(
        &self,
        spec: &MusicSpec,
        started: Instant,
        deadline: Duration,
    ) -> Result<Mastered, FailureReason> {
        let prompt = spec.ambience_prompt();
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|_| FailureReason::Interrupted)?;
        let call_started = Instant::now();
        let billed = self.begin(ambience_call("set", u64::from(AMBIENCE_SECONDS)));
        let audio = match self
            .provider
            .ambience(
                &prompt,
                AMBIENCE_SECONDS,
                Self::remaining(started, deadline),
            )
            .await
        {
            Ok(audio) => audio,
            Err(error) => {
                self.failed(billed, error);
                return Err(provider_reason(error));
            }
        };
        Self::billed(billed, u64::from(AMBIENCE_SECONDS));
        let provider_ms = call_started.elapsed().as_millis();
        let loop_frames = (AMBIENCE_LOOP_SECONDS * SAMPLE_RATE) as usize;
        let detail = self
            .master(
                audio.bytes,
                loop_frames,
                AMBIENCE_CROSSFADE_MILLIS,
                AMBIENCE_LUFS,
                started,
                deadline,
            )
            .await
            .map_err(FailureReason::Master)?;
        Ok(Mastered {
            mp3: detail.0,
            detail: json!({
                "prompt": prompt,
                "provider_ms": provider_ms as u64,
                "crossfade_ms": AMBIENCE_CROSSFADE_MILLIS,
                "measured_lufs": detail.1.measured_lufs,
                "gain_db": detail.1.gain_db,
                "reached_target": detail.1.reached_target,
            }),
        })
    }

    /// Decode, cut to the loop with its tail crossfade, set the loudness, encode.
    async fn master(
        &self,
        mp3: Vec<u8>,
        loop_frames: usize,
        crossfade_millis: u32,
        target_lufs: f64,
        started: Instant,
        deadline: Duration,
    ) -> Result<(Vec<u8>, super::master::Normalised), MasterFailure> {
        let source = self
            .codec
            .decode(mp3, Self::remaining(started, deadline))
            .await?;
        let crossfade_frames =
            (u64::from(crossfade_millis) * u64::from(SAMPLE_RATE) / 1_000) as usize;
        let mut frames: Vec<Frame> = loop_cut(&source, loop_frames, crossfade_frames)?;
        let normalised = normalise(&mut frames, target_lufs)?;
        let encoded = self
            .codec
            .encode(frames, Self::remaining(started, deadline))
            .await?;
        Ok((encoded, normalised))
    }
}

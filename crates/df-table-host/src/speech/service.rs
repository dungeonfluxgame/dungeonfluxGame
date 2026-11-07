//! The speech surface the routes and the persona routes share: transcribe a recording, voice a
//! line and keep the clip, and cue a clip on the scene.
//!
//! Audit lines (`SPEECH ...`) hold counts, names of clips, closed keys and times only, never a
//! recording, a transcript, the text of a line, a voice id or a key:
//!
//! - `SPEECH synthesized clip=<name> bytes=<n> duration_ms=<n> profile=<label>`
//! - `SPEECH transcribed audio_ms=<n> duration_ms=<n> words=<n> terms=<n>`
//! - `SPEECH failed verb=<speak|transcribe> error=<key> duration_ms=<n>` for a supplier call
//!   that did not give an answer
//! - `SPEECH refused verb=<verb> error=<key>` for every refused request (routes)
//! - `SPEECH key_terms_unavailable error=<key>` when the shown scene could not give names
//!
//! Only text the host itself produced or the operator typed is voiced; a transcript is a draft
//! for the player to edit and is never an action.

use std::sync::Arc;
use std::time::Instant;

use df_playground::scene::{SceneChange, Target};
use df_types::saturating_millis;

use super::clips::{Clip, clips, store_clip};
use super::limits::{
    MAXIMUM_AUDIO_MS, MAXIMUM_KEY_TERM_CHARACTERS, MAXIMUM_KEY_TERM_WORDS, MAXIMUM_KEY_TERMS,
    MAXIMUM_TEXT_CHARACTERS, MINIMUM_AUDIO_MS,
};
use super::provider::{
    ElevenLabsPort, PortStatus, STT_MODEL, SpeechPort, SynthesisCall, TTS_MODEL, TranscriptionCall,
};
use super::request::{
    ClipPlayed, MissingSetting, QueryCause, SpeakRequest, SpeechError, SpeechFeature, SpeechStatus,
    Transcript, Voiced, current_limits,
};
use super::voices::{ResolvedVoice, VoiceBook, check_character, voices_path};
use crate::blocking::run_blocking;
use crate::command::DEFAULT_VOLUME;
use crate::persona::summaries_of;
use crate::playground::apply_scene_changes;
use crate::shown_scene::shown_cast;
use crate::state::AppState;

/// The host, and the supplier its speech is asked of.
#[derive(Clone)]
pub struct Speech {
    app: Arc<AppState>,
    port: Arc<dyn SpeechPort>,
}

/// A line that was voiced and kept as a clip.
pub(super) struct Synthesized {
    pub(super) clip: Clip,
    pub(super) voice_profile: String,
    pub(super) synthesis_ms: u64,
}

/// Which speech call an audit line is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpeechVerb {
    Transcribe,
    Speak,
}

impl SpeechVerb {
    fn key(self) -> &'static str {
        match self {
            Self::Transcribe => "transcribe",
            Self::Speak => "speak",
        }
    }
}

impl Speech {
    pub fn new(app: Arc<AppState>, port: Arc<dyn SpeechPort>) -> Self {
        Self { app, port }
    }

    /// Whether the playground has `feature` turned on (`playground set tts=..` or `stt=..`).
    pub fn is_on(&self, feature: SpeechFeature) -> bool {
        let controls = self.app.playground().snapshot().controls;
        match feature {
            SpeechFeature::Tts => controls.tts_on,
            SpeechFeature::Stt => controls.stt_on,
        }
    }

    // TODO(DEV-12): a host test that `speak` and `transcribe` are refused with their switch off.
    /// Refuses a call of a feature that is turned off.
    fn require_on(&self, feature: SpeechFeature) -> Result<(), SpeechError> {
        if self.is_on(feature) {
            Ok(())
        } else {
            Err(SpeechError::Disabled { feature })
        }
    }

    /// The speech service over the supplier this process's environment asks for.
    pub fn from_environment(app: Arc<AppState>) -> Self {
        Self::new(app, Arc::new(ElevenLabsPort::from_environment()))
    }

    pub(crate) fn app(&self) -> &AppState {
        &self.app
    }

    /// Whether the supplier is set up, without reading the voices or the clips.
    pub fn port_status(&self) -> PortStatus {
        self.port.status()
    }

    fn check_configured(&self) -> Result<(), SpeechError> {
        let status = self.port.status();
        if status.configured {
            Ok(())
        } else {
            Err(SpeechError::NotConfigured {
                missing: status.missing.unwrap_or(MissingSetting::SpeechKey),
            })
        }
    }

    async fn voice_book(&self) -> Result<VoiceBook, SpeechError> {
        let path = voices_path(&self.app);
        run_blocking(move || VoiceBook::read(&path), SpeechError::Stopped).await
    }

    /// How the service is set up and how it stands.
    pub async fn status(&self) -> Result<SpeechStatus, SpeechError> {
        let port = self.port.status();
        let book = self.voice_book().await?;
        let kept = clips(&self.app).await?;
        Ok(SpeechStatus {
            configured: port.configured,
            missing: port.missing,
            tts_model: TTS_MODEL.to_owned(),
            stt_model: STT_MODEL.to_owned(),
            default_voice: book.default_voice().map(str::to_owned),
            voices: book.rows(),
            clips_kept: u64::try_from(kept.len()).unwrap_or(u64::MAX),
            limits: current_limits(),
        })
    }

    /// The names the shown scene gives to bias recognition: the display names of its cast that
    /// have a persona, and the scene's own name. Recognition is biased toward them; nothing is
    /// matched against them.
    async fn scene_terms(&self) -> Vec<String> {
        let cast = match shown_cast(&self.app).await {
            Ok(cast) => cast,
            Err(error) => {
                self.app.record_line(format!(
                    "SPEECH key_terms_unavailable error={}",
                    error.key()
                ));
                return Vec::new();
            }
        };
        let names = cast.layout.names().into_iter().map(str::to_owned).collect();
        let mut terms: Vec<String> = summaries_of(&self.app, names)
            .await
            .into_iter()
            .map(|summary| summary.display_name)
            .collect();
        terms.push(cast.fixture.name);
        let mut kept: Vec<String> = Vec::new();
        for term in terms {
            if key_term_problem(&term).is_none() && !kept.contains(&term) {
                kept.push(term);
            }
        }
        kept.truncate(MAXIMUM_KEY_TERMS);
        kept
    }

    /// Transcribes `pcm` (16 kHz mono signed 16-bit little-endian), biased toward `terms`, or
    /// toward the shown scene's names when none are given. The recording and the transcript live
    /// only in this call.
    pub async fn transcribe(
        &self,
        pcm: Vec<u8>,
        terms: Vec<String>,
    ) -> Result<Transcript, SpeechError> {
        self.require_on(SpeechFeature::Stt)?;
        let audio_ms = check_audio(&pcm)?;
        if let Some(cause) = terms.iter().find_map(|term| key_term_problem(term)) {
            return Err(SpeechError::InvalidQuery { cause });
        }
        if terms.len() > MAXIMUM_KEY_TERMS {
            return Err(SpeechError::InvalidQuery {
                cause: QueryCause::TooManyTerms,
            });
        }
        self.check_configured()?;
        let key_terms = if terms.is_empty() {
            self.scene_terms().await
        } else {
            terms
        };
        let terms_used = key_terms.len();
        let started = Instant::now();
        let heard = self
            .port
            .transcribe(TranscriptionCall { pcm, key_terms })
            .await;
        let duration_ms = saturating_millis(started.elapsed());
        match heard {
            Ok(heard) => {
                self.app.record_line(format!(
                    "SPEECH transcribed audio_ms={audio_ms} duration_ms={duration_ms} words={} \
                     terms={terms_used}",
                    heard.words
                ));
                Ok(Transcript {
                    text: heard.text,
                    language: heard.language,
                    audio_ms,
                    duration_ms,
                })
            }
            Err(error) => {
                self.record_failure(SpeechVerb::Transcribe, &error, duration_ms);
                Err(error)
            }
        }
    }

    fn record_failure(&self, verb: SpeechVerb, error: &SpeechError, duration_ms: u64) {
        self.app.record_line(format!(
            "SPEECH failed verb={} error={} duration_ms={duration_ms}",
            verb.key(),
            error.key()
        ));
    }

    /// Voices `text` in the voice of `character` (or `voice` when named), keeps the clip in the
    /// media library and answers its name. It plays nothing.
    pub(super) async fn synthesize(
        &self,
        text: &str,
        character: Option<&str>,
        voice: Option<&str>,
    ) -> Result<Synthesized, SpeechError> {
        check_text(text)?;
        if let Some(character) = character {
            check_character(character)?;
        }
        self.check_configured()?;
        let resolved: ResolvedVoice = self.voice_book().await?.resolve(character, voice)?;
        let profile = resolved.profile();
        let started = Instant::now();
        let made = self
            .port
            .synthesize(SynthesisCall {
                text: text.to_owned(),
                voice: resolved.id,
            })
            .await;
        let synthesis_ms = saturating_millis(started.elapsed());
        let made =
            made.inspect_err(|error| self.record_failure(SpeechVerb::Speak, error, synthesis_ms))?;
        let clip = store_clip(&self.app, &made.mp3).await?;
        self.app.record_line(format!(
            "SPEECH synthesized clip={} bytes={} duration_ms={synthesis_ms} profile={profile}",
            clip.name, clip.bytes
        ));
        Ok(Synthesized {
            clip,
            voice_profile: profile,
            synthesis_ms,
        })
    }

    /// Voices a line `character` says and answers what came of it. A failure is the answer, not
    /// an error: a line is never blocked by its voice.
    pub(crate) async fn voice_line(&self, character: &str, line: &str) -> Voiced {
        let voiced = match self.require_on(SpeechFeature::Tts) {
            Ok(()) => self.synthesize(line, Some(character), None).await,
            Err(error) => Err(error),
        };
        match voiced {
            Ok(made) => Voiced::Played {
                clip: made.clip.name,
                bytes: made.clip.bytes,
                voice_profile: made.voice_profile,
                synthesis_ms: made.synthesis_ms,
            },
            Err(error) => {
                self.app.record_line(format!(
                    "SPEECH line_unvoiced character={character} error={}",
                    error.key()
                ));
                Voiced::Failed { error }
            }
        }
    }

    /// Voices the operator's text and cues the clip on the scene, as `scene sound` does.
    pub(super) async fn speak(&self, request: SpeakRequest) -> Result<ClipPlayed, SpeechError> {
        self.require_on(SpeechFeature::Tts)?;
        let made = self
            .synthesize(
                &request.text,
                request.character.as_deref(),
                request.voice.as_deref(),
            )
            .await?;
        let applied = apply_scene_changes(&self.app, vec![sound_change(&made.clip.name)])
            .await
            .map_err(|error| SpeechError::Cue {
                cause: error.into(),
            })?;
        Ok(ClipPlayed {
            clip: made.clip.name,
            bytes: made.clip.bytes,
            voice_profile: made.voice_profile,
            synthesis_ms: made.synthesis_ms,
            revision: applied.revision,
        })
    }
}

/// The scene change that plays `clip` once on every screen, as `scene sound` does.
pub(crate) fn sound_change(clip: &str) -> SceneChange {
    SceneChange::Sound {
        media: clip.to_owned(),
        target: Target::Both,
        volume_percent: DEFAULT_VOLUME,
    }
}

/// The length of `pcm` in whole milliseconds, or why it is not a recording the supplier takes.
pub(super) fn check_audio(pcm: &[u8]) -> Result<u64, SpeechError> {
    if pcm.is_empty() {
        return Err(SpeechError::AudioEmpty);
    }
    if !pcm.len().is_multiple_of(2) {
        return Err(SpeechError::AudioMisaligned);
    }
    let audio_ms = u64::try_from(pcm.len() / super::limits::AUDIO_BYTES_PER_MS).unwrap_or(u64::MAX);
    if audio_ms < MINIMUM_AUDIO_MS {
        return Err(SpeechError::AudioTooShort {
            audio_ms,
            minimum_ms: MINIMUM_AUDIO_MS,
        });
    }
    // Compared in bytes, so audio even a sample over the limit is refused here and not by the
    // supplier's exact-length check.
    let maximum_bytes =
        usize::try_from(MAXIMUM_AUDIO_MS).unwrap_or(usize::MAX) * super::limits::AUDIO_BYTES_PER_MS;
    if pcm.len() > maximum_bytes {
        return Err(SpeechError::AudioTooLong {
            audio_ms,
            maximum_ms: MAXIMUM_AUDIO_MS,
        });
    }
    Ok(audio_ms)
}

/// That `text` is a line a voice takes.
pub(super) fn check_text(text: &str) -> Result<(), SpeechError> {
    if text.trim().is_empty() {
        return Err(SpeechError::TextBlank);
    }
    let characters = text.chars().count();
    if characters > MAXIMUM_TEXT_CHARACTERS {
        return Err(SpeechError::TextTooLong {
            characters: u64::try_from(characters).unwrap_or(u64::MAX),
            maximum: u64::try_from(MAXIMUM_TEXT_CHARACTERS).unwrap_or(u64::MAX),
        });
    }
    Ok(())
}

/// What is wrong with `term` as a key term, or `None` when it is one the supplier takes.
fn key_term_problem(term: &str) -> Option<QueryCause> {
    let bad_character =
        |c: char| c.is_control() || matches!(c, '<' | '>' | '{' | '}' | '[' | ']' | '\\');
    if term.trim().is_empty() {
        Some(QueryCause::BlankTerm)
    } else if term.chars().count() > MAXIMUM_KEY_TERM_CHARACTERS {
        Some(QueryCause::LongTerm)
    } else if term.split_whitespace().count() > MAXIMUM_KEY_TERM_WORDS {
        Some(QueryCause::WordyTerm)
    } else if term.chars().any(bad_character) {
        Some(QueryCause::InvalidTermCharacters)
    } else {
        None
    }
}

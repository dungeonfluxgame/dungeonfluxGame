//! The speech surface the routes and the persona routes share: transcribe a recording, voice a
//! line and keep the clip, and cue a clip on the scene.
//!
//! Audit lines (`SPEECH ...`) hold counts, names of clips, closed keys and times only, never a
//! recording, a transcript, the text of a line, a voice id or a key:
//!
//! - `SPEECH port backend=<elevenlabs|forge|unknown> configured=<bool>` when the service is built
//! - `SPEECH synthesized clip=<name> bytes=<n> duration_ms=<n> profile=<label>`
//! - `SPEECH transcribed audio_ms=<n> duration_ms=<n> words=<n> terms=<n>`
//! - `SPEECH failed verb=<speak|transcribe> error=<key> duration_ms=<n>` for a supplier call
//!   that did not give an answer
//! - `SPEECH refused verb=<verb> error=<key>` for every refused request (routes)
//! - `SPEECH key_terms_unavailable error=<key>` when the shown scene could not give names
//! - `SPEECH voice_cast character=<id> voice_name=<"name"> reused=<bool> changed=<bool>
//!   duration_ms=<n>` when a character's voice was cast (by `speech cast`, or before the first
//!   line of a character with a persona and no voice of their own)
//! - `SPEECH voice_cast_retried character=<id> error=<cause>` when the judge is asked again
//! - `SPEECH voice_cast_failed character=<id> error=<key> duration_ms=<n>` when a first line's
//!   cast failed and the line is spoken in the default voice
//!
//! Only text the host itself produced or the operator typed is voiced; a transcript is a draft
//! for the player to edit and is never an action.

use std::sync::Arc;
use std::time::Instant;

use df_content::persona::{BriefScope, PublicPersona};
use df_playground::scene::{SceneChange, Target};
use df_types::saturating_millis;

use super::cast::{ask_cast, cast_facts};
use super::clips::{Clip, clips, store_clip};
use super::limits::{
    MAXIMUM_AUDIO_MS, MAXIMUM_KEY_TERM_CHARACTERS, MAXIMUM_KEY_TERM_WORDS, MAXIMUM_KEY_TERMS,
    MAXIMUM_TEXT_CHARACTERS, MINIMUM_AUDIO_MS,
};
use super::provider::{
    PortStatus, SpeechPort, SynthesisCall, TranscriptionCall, VoiceListing, port_from_environment,
};
use super::request::{
    CastAnswerCause, CastRequest, ClipPlayed, MissingSetting, QueryCause, SpeakRequest,
    SpeechError, SpeechFeature, SpeechStatus, Transcript, VoiceCast, VoiceCastError, Voiced,
    current_limits,
};
use super::voices::{
    ResolvedVoice, VoiceBook, VoiceSource, change_voice, check_character, claim_voice, voices_path,
};
use crate::blocking::run_blocking;
use crate::command::DEFAULT_VOLUME;
use crate::persona::generate::load_persona;
use crate::persona::story::judge_route;
use crate::persona::{PersonaError, summaries_of};
use crate::playground::apply_scene_changes;
use crate::shown_scene::shown_cast;
use crate::state::AppState;
use crate::text::TextRuntime;

/// The host, the supplier its speech is asked of, and the text surface a character's voice is
/// cast through.
#[derive(Clone)]
pub struct Speech {
    app: Arc<AppState>,
    port: Arc<dyn SpeechPort>,
    /// The text surface the casting judge is asked through; without it nothing is cast and a
    /// character without a voice speaks in the default.
    text: Option<Arc<TextRuntime>>,
    /// Held around each cast, so a character is never cast twice (two first lines at once) and
    /// each cast sees the voices the casts before it gave.
    casting: Arc<tokio::sync::Mutex<()>>,
}

/// Whether a cast may replace the voice a character has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Replace {
    /// `speech cast`: the character's voice is the judge's choice, whatever it was.
    Yes,
    /// A first line's cast: a voice the character was given meanwhile is kept.
    No,
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
        Self {
            app,
            port,
            text: None,
            casting: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// This service casting characters' voices through `text` (the host's one text surface).
    #[must_use]
    pub fn with_text(mut self, text: Arc<TextRuntime>) -> Self {
        self.text = Some(text);
        self
    }

    /// Whether the playground has `feature` turned on (`playground set tts=..` or `stt=..`).
    pub fn is_on(&self, feature: SpeechFeature) -> bool {
        let controls = self.app.playground().snapshot().controls;
        match feature {
            SpeechFeature::Tts => controls.tts_on,
            SpeechFeature::Stt => controls.stt_on,
        }
    }

    /// Refuses a call of a feature that is turned off.
    fn require_on(&self, feature: SpeechFeature) -> Result<(), SpeechError> {
        if self.is_on(feature) {
            Ok(())
        } else {
            Err(SpeechError::Disabled { feature })
        }
    }

    /// The speech service over the supplier this process's environment asks for
    /// (`DF_SPEECH_BACKEND`).
    pub fn from_environment(app: Arc<AppState>) -> Self {
        let (port, backend) = port_from_environment();
        app.record_line(format!(
            "SPEECH port backend={backend} configured={}",
            port.status().configured
        ));
        Self::new(app, port)
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
            tts_model: port.tts_model.to_owned(),
            stt_model: port.stt_model.to_owned(),
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
        // The one place the voice supplier is called, so the switch is checked here: nothing that
        // costs money is sent while text to speech is off, whoever asks.
        self.require_on(SpeechFeature::Tts)?;
        check_text(text)?;
        if let Some(character) = character {
            check_character(character)?;
        }
        self.check_configured()?;
        let book = self.voice_book().await?;
        let resolved = match (character, voice) {
            (Some(character), None) if book.voice_of(character).is_none() => {
                match self.cast_first_voice(character).await {
                    Some(id) => ResolvedVoice {
                        id,
                        source: VoiceSource::Character,
                    },
                    None => book.resolve(Some(character), None)?,
                }
            }
            _ => book.resolve(character, voice)?,
        };
        let profile = resolved.profile(self.port.status().tts_model);
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
        let clip = store_clip(&self.app, &made.audio, made.format).await?;
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

    /// The voice cast for `character` before their first line, when they have a persona and the
    /// service a text surface: `None` speaks the line in the default voice. A failed cast is
    /// logged (`SPEECH voice_cast_failed`) and never stops the line; it is tried again on the
    /// character's next line.
    async fn cast_first_voice(&self, character: &str) -> Option<String> {
        let text = self.text.as_ref()?;
        let _casting = self.casting.lock().await;
        let started = Instant::now();
        let failed = |error: VoiceCastError| {
            self.app.record_line(format!(
                "SPEECH voice_cast_failed character={character} error={} duration_ms={}",
                error.key(),
                saturating_millis(started.elapsed())
            ));
            None
        };
        let book = match self.voice_book().await {
            Ok(book) => book,
            Err(error) => return failed(error.into()),
        };
        // Another line of the character cast their voice while this one waited.
        if let Some(own) = book.voice_of(character) {
            return Some(own.to_owned());
        }
        let persona = match load_persona(&self.app, character).await {
            Ok(persona) => persona,
            Err(PersonaError::UnknownPersona { .. }) => return None,
            Err(cause) => return failed(VoiceCastError::PersonaUnreadable { cause }),
        };
        let public = persona.brief(BriefScope::Listener).public;
        match self
            .cast_locked(text, character, &public, &book, None, Replace::No)
            .await
        {
            Ok(cast) => Some(cast.voice),
            Err(error) => failed(error),
        }
    }

    /// Casts the voice of the character `request` names from their persona, replacing the voice
    /// they have (`speech cast`).
    pub(super) async fn cast(&self, request: CastRequest) -> Result<VoiceCast, VoiceCastError> {
        check_character(&request.character)?;
        let text = self.text.as_ref().ok_or(VoiceCastError::NoJudge)?;
        let _casting = self.casting.lock().await;
        let persona =
            load_persona(&self.app, &request.character)
                .await
                .map_err(|error| match error {
                    PersonaError::UnknownPersona { .. } => VoiceCastError::NoPersona {
                        character: request.character.clone(),
                    },
                    cause => VoiceCastError::PersonaUnreadable { cause },
                })?;
        let book = self.voice_book().await?;
        let public = persona.brief(BriefScope::Listener).public;
        self.cast_locked(
            text,
            &request.character,
            &public,
            &book,
            request.model.as_deref(),
            Replace::Yes,
        )
        .await
    }

    /// Lists the supplier's voices, asks the judge which fits `public` (preferring one nobody in
    /// `book` speaks in) and writes it as `character`'s voice. The caller holds `casting`.
    async fn cast_locked(
        &self,
        text: &TextRuntime,
        character: &str,
        public: &PublicPersona,
        book: &VoiceBook,
        model: Option<&str>,
        replace: Replace,
    ) -> Result<VoiceCast, VoiceCastError> {
        let started = Instant::now();
        let route = judge_route::<CastAnswerCause>(text, model)?;
        let voices = match self.port.voices().await? {
            VoiceListing::Listed(voices) => voices,
            VoiceListing::Unlisted => return Err(VoiceCastError::Unlisted),
        };
        if voices.is_empty() {
            return Err(VoiceCastError::NoVoices);
        }
        let in_use = book.voices_in_use(character);
        let facts = cast_facts(public, &voices, &in_use);
        let choice = ask_cast(&self.app, character, &route, facts, &voices).await?;
        let path = voices_path(&self.app);
        let (owned_character, chosen) = (character.to_owned(), choice.voice.clone());
        let (voice, changed) = match replace {
            Replace::Yes => {
                let changed = run_blocking(
                    move || change_voice(&path, &owned_character, Some(&chosen)),
                    SpeechError::Stopped,
                )
                .await?
                .changed;
                (choice.voice, changed)
            }
            Replace::No => {
                let claimed = run_blocking(
                    move || claim_voice(&path, &owned_character, &chosen),
                    SpeechError::Stopped,
                )
                .await?;
                (claimed.voice, claimed.given)
            }
        };
        let voice_name = voices
            .iter()
            .find(|listed| listed.voice_id() == voice)
            .map_or_else(|| "-".to_owned(), |listed| listed.name().to_owned());
        let reused = in_use.contains(&voice.as_str());
        self.app.record_line(format!(
            "SPEECH voice_cast character={character} voice_name={voice_name:?} reused={reused} \
             changed={changed} duration_ms={}",
            saturating_millis(started.elapsed())
        ));
        Ok(VoiceCast {
            character: character.to_owned(),
            voice,
            voice_name,
            reused,
            why: choice.why,
            changed,
        })
    }

    /// Voices a line `character` says and answers what came of it. A failure is the answer, not
    /// an error: a line is never blocked by its voice.
    pub(crate) async fn voice_line(&self, character: &str, line: &str) -> Voiced {
        match self.synthesize(line, Some(character), None).await {
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

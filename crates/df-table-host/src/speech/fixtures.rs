//! Test helpers: a fake speech port that never touches the network, and the speech service over
//! it.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::provider::{
    ClipFormat, PortFuture, PortStatus, STT_MODEL, SpeechPort, Synthesis, SynthesisCall, TTS_MODEL,
    Transcription, TranscriptionCall,
};
use super::request::{MissingSetting, SpeechError, SpeechProviderCause};
use super::service::Speech;
use crate::state::AppState;

/// The bytes the fake "synthesizes": enough for the media library to keep.
pub(crate) const FAKE_MP3: &[u8] = b"ID3 fake mp3 bytes";
/// The text the fake "hears".
pub(crate) const FAKE_HEARD: &str = "a room for the night";

/// What the fake saw of a transcription: the recording's length and the key terms, never more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HeardCall {
    pub(crate) pcm_bytes: usize,
    pub(crate) key_terms: Vec<String>,
}

/// A fake port: configured or not, failing or answering, remembering what it was asked.
pub(crate) struct FakePort {
    pub(crate) configured: bool,
    pub(crate) failure: Option<SpeechProviderCause>,
    voices: Mutex<Vec<String>>,
    spoken_characters: Mutex<Vec<usize>>,
    heard: Mutex<Vec<HeardCall>>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakePort {
    pub(crate) fn answering() -> Self {
        Self {
            configured: true,
            failure: None,
            voices: Mutex::new(Vec::new()),
            spoken_characters: Mutex::new(Vec::new()),
            heard: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn unconfigured() -> Self {
        Self {
            configured: false,
            ..Self::answering()
        }
    }

    pub(crate) fn failing(cause: SpeechProviderCause) -> Self {
        Self {
            failure: Some(cause),
            ..Self::answering()
        }
    }

    /// The voices the fake was asked to speak in, in order.
    pub(crate) fn voices(&self) -> Vec<String> {
        locked(&self.voices).clone()
    }

    /// The lengths, in characters, of the texts the fake was asked to voice.
    pub(crate) fn spoken_characters(&self) -> Vec<usize> {
        locked(&self.spoken_characters).clone()
    }

    pub(crate) fn heard(&self) -> Vec<HeardCall> {
        locked(&self.heard).clone()
    }

    fn outcome<T>(&self, answer: T) -> Result<T, SpeechError> {
        match self.failure {
            Some(cause) => Err(SpeechError::Provider { cause }),
            None => Ok(answer),
        }
    }
}

impl SpeechPort for FakePort {
    fn status(&self) -> PortStatus {
        PortStatus {
            configured: self.configured,
            missing: (!self.configured).then_some(MissingSetting::SpeechKey),
            tts_model: TTS_MODEL,
            stt_model: STT_MODEL,
        }
    }

    fn synthesize(&self, call: SynthesisCall) -> PortFuture<'_, Result<Synthesis, SpeechError>> {
        locked(&self.voices).push(call.voice);
        locked(&self.spoken_characters).push(call.text.chars().count());
        let answer = self.outcome(Synthesis {
            audio: FAKE_MP3.to_vec(),
            format: ClipFormat::Mp3,
        });
        Box::pin(async move { answer })
    }

    fn transcribe(
        &self,
        call: TranscriptionCall,
    ) -> PortFuture<'_, Result<Transcription, SpeechError>> {
        locked(&self.heard).push(HeardCall {
            pcm_bytes: call.pcm.len(),
            key_terms: call.key_terms,
        });
        let answer = self.outcome(Transcription {
            text: FAKE_HEARD.to_owned(),
            language: "eng".to_owned(),
            words: FAKE_HEARD.split_whitespace().count(),
        });
        Box::pin(async move { answer })
    }
}

/// The speech service over `port`.
pub(crate) fn speech_over(app: &Arc<AppState>, port: &Arc<FakePort>) -> Speech {
    let port: Arc<dyn SpeechPort> = port.clone();
    Speech::new(Arc::clone(app), port)
}

/// A speech service whose supplier is not set up, for tests of other features.
pub(crate) fn silent_speech(app: &Arc<AppState>) -> Speech {
    speech_over(app, &Arc::new(FakePort::unconfigured()))
}

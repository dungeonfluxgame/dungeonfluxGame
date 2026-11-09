//! The `speech` verbs of the dev command line: their grammar, the request each sends, and the
//! text of each answer.
//!
//! `speech transcribe` is a test tool: it reads a raw 16 kHz mono signed 16-bit little-endian PCM
//! file, or a WAV file of that format, and prints what the supplier heard.

use std::path::{Path, PathBuf};

use df_content::persona::MAXIMUM_ID_CHARS;

use super::format::{
    format_cast, format_cleared, format_clips, format_played, format_status, format_transcript,
    format_voice,
};
use super::limits::AUDIO_CONTENT_TYPE;
use super::request::{
    CastReply, CastRequest, ClipPlayed, ClipRow, ClipsCleared, SpeakRequest, SpeechError,
    SpeechReply, SpeechStatus, Transcript, VoiceCast, VoiceChanged, VoiceRequest,
};
use super::routes::path;
use super::voices::check_character;
use crate::command::{CommandError, bad_choice};
use crate::dev::{DevError, ReplyError};
use crate::dev_client::{self, get_json, post_json};

/// Every `speech` verb, for the unknown-verb refusal and the help.
pub const SPEECH_USAGE: &str = concat!(
    "speech [status] | speech speak <character|-> <text...> | ",
    "speech transcribe <file.wav|file.pcm> | speech voice <character> <voice-id|clear> | ",
    "speech cast <character> [model=<name>] | speech clips | speech clear",
);
/// `speech speak`: voice operator text and play it.
pub const SPEECH_SPEAK_USAGE: &str = "speech speak <character|-> <text...>";
/// `speech transcribe`: a recording file to its transcript.
pub const SPEECH_TRANSCRIBE_USAGE: &str = "speech transcribe <file.wav|file.pcm>";
/// `speech voice`: a character's voice.
pub const SPEECH_VOICE_USAGE: &str = "speech voice <character> <voice-id|clear>";
/// `speech cast`: a character's voice chosen from their persona by the casting judge.
pub const SPEECH_CAST_USAGE: &str = "speech cast <character> [model=<name>]";

/// A speech command of the dev CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeechCommand {
    /// How the service is set up and its voices.
    Status,
    /// Voice `text` in the voice of `character` (the default voice for `None`) and play it.
    Speak {
        character: Option<String>,
        text: String,
    },
    /// Transcribe the recording in the file at `path`.
    Transcribe { path: PathBuf },
    /// Give `character` the voice `voice`, or take theirs away for `None`.
    Voice {
        character: String,
        voice: Option<String>,
    },
    /// Cast `character`'s voice from their persona, replacing the one they have; `model` judges
    /// instead of the persona model.
    Cast {
        character: String,
        model: Option<String>,
    },
    /// List the voice clips the host keeps.
    Clips,
    /// Delete every voice clip the host made.
    Clear,
}

/// `word` as a character id, or the refusal naming the rule.
fn character_word(word: &str) -> Result<String, CommandError> {
    check_character(word).map(|_| word.to_owned()).map_err(|_| {
        bad_choice(
            "character",
            word,
            format!("1 to {MAXIMUM_ID_CHARS} letters, digits, '-' or '_'"),
        )
    })
}

/// Reads the words after `speech`.
pub fn parse(words: &[String]) -> Result<SpeechCommand, CommandError> {
    match words {
        [] => Ok(SpeechCommand::Status),
        [verb] if verb == "status" => Ok(SpeechCommand::Status),
        [verb, character, text @ ..] if verb == "speak" && !text.is_empty() => {
            Ok(SpeechCommand::Speak {
                character: if character == "-" {
                    None
                } else {
                    Some(character_word(character)?)
                },
                text: text.join(" "),
            })
        }
        [verb, ..] if verb == "speak" => Err(CommandError::Usage(SPEECH_SPEAK_USAGE)),
        [verb, file] if verb == "transcribe" => Ok(SpeechCommand::Transcribe {
            path: PathBuf::from(file),
        }),
        [verb, ..] if verb == "transcribe" => Err(CommandError::Usage(SPEECH_TRANSCRIBE_USAGE)),
        [verb, character, voice] if verb == "voice" => Ok(SpeechCommand::Voice {
            character: character_word(character)?,
            voice: (voice != "clear").then(|| voice.clone()),
        }),
        [verb, ..] if verb == "voice" => Err(CommandError::Usage(SPEECH_VOICE_USAGE)),
        [verb, character] if verb == "cast" => Ok(SpeechCommand::Cast {
            character: character_word(character)?,
            model: None,
        }),
        [verb, character, option] if verb == "cast" => match option.split_once('=') {
            Some(("model", model)) if !model.is_empty() => Ok(SpeechCommand::Cast {
                character: character_word(character)?,
                model: Some(model.to_owned()),
            }),
            _ => Err(CommandError::Usage(SPEECH_CAST_USAGE)),
        },
        [verb, ..] if verb == "cast" => Err(CommandError::Usage(SPEECH_CAST_USAGE)),
        [verb] if verb == "clips" => Ok(SpeechCommand::Clips),
        [verb] if verb == "clear" => Ok(SpeechCommand::Clear),
        _ => Err(CommandError::Usage(SPEECH_USAGE)),
    }
}

/// A refusal found by the command line itself, as the failure a route's refusal would be.
fn refused(error: SpeechError) -> DevError {
    DevError::Rejected {
        message: error.to_string(),
        error: ReplyError::Speech(error),
    }
}

fn reply_value<T>(reply: SpeechReply<T>) -> Result<T, DevError> {
    dev_client::reply_value(reply)
}

/// The size of the canonical WAV header's fixed part: `RIFF`, size, `WAVE`.
const WAV_PREFIX_BYTES: usize = 12;
/// The WAV format tag of integer PCM.
const WAV_PCM_TAG: u16 = 1;

fn le_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn le_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// The samples of a WAV file of 16 kHz mono 16-bit integer PCM, found by walking its chunks (a
/// header may carry a `LIST` chunk), or `UnsupportedWav` for any other WAV.
fn wav_samples(bytes: &[u8]) -> Result<Vec<u8>, SpeechError> {
    let mut at = WAV_PREFIX_BYTES;
    let mut format_ok = false;
    while at.checked_add(8).is_some_and(|end| end <= bytes.len()) {
        let size = le_u32(bytes, at + 4)
            .and_then(|size| usize::try_from(size).ok())
            .ok_or(SpeechError::UnsupportedWav)?;
        let body = at + 8;
        let end = body.saturating_add(size).min(bytes.len());
        match bytes.get(at..at + 4) {
            Some(b"fmt ") => {
                format_ok = le_u16(bytes, body) == Some(WAV_PCM_TAG)
                    && le_u16(bytes, body + 2) == Some(1)
                    && le_u32(bytes, body + 4) == Some(16_000)
                    && le_u16(bytes, body + 14) == Some(16);
            }
            Some(b"data") if format_ok => {
                return bytes
                    .get(body..end)
                    .map(<[u8]>::to_vec)
                    .ok_or(SpeechError::UnsupportedWav);
            }
            _ => {}
        }
        // Chunks are padded to an even size.
        at = body.saturating_add(size).saturating_add(size % 2);
    }
    Err(SpeechError::UnsupportedWav)
}

/// The PCM of a recording file: the samples of a WAV, or the bytes of a raw PCM file as they are.
pub(super) fn pcm_of_file(bytes: Vec<u8>) -> Result<Vec<u8>, SpeechError> {
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") {
        wav_samples(&bytes)
    } else {
        Ok(bytes)
    }
}

async fn read_file(file: &Path) -> Result<Vec<u8>, DevError> {
    let owned = file.to_path_buf();
    let unreadable = |message: String| DevError::UnreadableFile {
        path: file.to_path_buf(),
        message,
    };
    tokio::task::spawn_blocking(move || std::fs::read(&owned))
        .await
        .map_err(|error| unreadable(error.to_string()))?
        .map_err(|error| unreadable(error.to_string()))
}

async fn transcribe(port: u16, file: &Path) -> Result<String, DevError> {
    let pcm = pcm_of_file(read_file(file).await?).map_err(refused)?;
    let answer =
        dev_client::post_bytes(port, path::SPEECH_TRANSCRIBE, AUDIO_CONTENT_TYPE, &pcm).await?;
    let transcript: Transcript = reply_value(dev_client::read_answer(&answer)?)?;
    Ok(format_transcript(&transcript))
}

/// Runs one speech command against the host on `port`.
pub async fn run(port: u16, command: SpeechCommand) -> Result<String, DevError> {
    match command {
        SpeechCommand::Status => {
            let status: SpeechStatus = reply_value(get_json(port, path::SPEECH_STATUS).await?)?;
            Ok(format_status(&status))
        }
        SpeechCommand::Speak { character, text } => {
            let request = SpeakRequest {
                text,
                character,
                voice: None,
            };
            let played: ClipPlayed =
                reply_value(post_json(port, path::SPEECH_SPEAK, &request).await?)?;
            Ok(format_played(&played))
        }
        SpeechCommand::Transcribe { path: file } => transcribe(port, &file).await,
        SpeechCommand::Voice { character, voice } => {
            let request = VoiceRequest { character, voice };
            let changed: VoiceChanged =
                reply_value(post_json(port, path::SPEECH_VOICE, &request).await?)?;
            Ok(format_voice(&changed))
        }
        SpeechCommand::Cast { character, model } => {
            let request = CastRequest { character, model };
            let reply: CastReply = post_json(port, path::SPEECH_CAST, &request).await?;
            let cast: VoiceCast = dev_client::reply_value(reply)?;
            Ok(format_cast(&cast))
        }
        SpeechCommand::Clips => {
            let clips: Vec<ClipRow> = reply_value(get_json(port, path::SPEECH_CLIPS).await?)?;
            Ok(format_clips(&clips))
        }
        SpeechCommand::Clear => {
            let cleared: ClipsCleared =
                reply_value(post_json(port, path::SPEECH_CLEAR, &()).await?)?;
            Ok(format_cleared(&cleared))
        }
    }
}

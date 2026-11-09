//! Tests of the speech feature: its routes, its refusals, its stores, its audit lines and its
//! command line, over a fake supplier that never touches the network.
//!
//! TODO(DEV-12): a real-supplier fixture test of `ElevenLabsPort` over a loopback server (the
//! `df-providers` contract tests cover the adapter itself), a concurrent-eviction test, a test of
//! the oversized voices file, and the `Debug` redaction tests of `Transcript` and `SpeakRequest`.

use std::sync::Arc;

use axum::http::{HeaderMap, HeaderValue, StatusCode, header::CONTENT_TYPE};
use serde::de::DeserializeOwned;

use super::cli::{SpeechCommand, pcm_of_file};
use super::clips::{clip_number, evict_clips, list_clips};
use super::fixtures::{FAKE_HEARD, FAKE_MP3, FakePort, speech_over};
use super::limits::{AUDIO_CONTENT_TYPE, KEY_VARIABLES};
use super::request::{
    ClipCause, ClipPlayed, ClipRow, ClipsCleared, MissingSetting, QueryCause, SpeechError,
    SpeechErrorKind, SpeechFeature, SpeechProviderCause, SpeechReply, SpeechStatus, Transcript,
    VoiceCause, VoiceChanged, VoicesCause,
};
use super::routes::{check_content_type, path, router_with};
use super::voices::{VOICES_FILE, VoiceBook, VoiceSource, change_voice};
use crate::client::{self, Response};
use crate::command::{Command, CommandError, split_words};
use crate::dev_http::{BodyCause, BodyError, DevReply, RouteError};
use crate::io_cause::IoCause;
use crate::playground::PlaygroundChangeError;
use crate::test_support::{Running, STARTER_FILES, start};
use df_content::persona::PersonaError as ContentError;
use df_playground::controls::ControlsChange;

const FIXTURE: &str = concat!(
    r#"{"name": "Roadside inn", "description": "A fire, a table, four travellers.", "#,
    r#""tv": {"place": "Here", "status": {"kind": "ok"}, "elements": [], "#,
    r#""backdrop": {"media": "plate.png", "caption": null}}, "#,
    r#""phone": {"hero": "Pip", "status": {"kind": "ok"}}}"#
);
const VOICES: &str =
    r#"{"default": "voice-default-id", "voices": {"innkeeper": "voice-marta-id"}}"#;
/// A second of silence: 16 000 samples of 16 bits.
const ONE_SECOND: usize = 32_000;

struct SpeechHost {
    running: Running,
    port: u16,
    fake: Arc<FakePort>,
}

async fn speech_host(fake: FakePort) -> SpeechHost {
    let running = start(&STARTER_FILES).await;
    running.write_asset("media/plate.png", "plate.png");
    running.write_asset("fixtures/one.json", FIXTURE);
    running.write_asset("voices.json", VOICES);
    running.switch_speech_on();
    let fake = Arc::new(fake);
    let app = router_with(speech_over(&running.state, &fake));
    let listener = crate::server::bind(0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        // The server runs until the test's runtime stops; its end is the test's end.
        let _ = axum::serve(listener, app).await;
    });
    SpeechHost {
        running,
        port,
        fake,
    }
}

impl SpeechHost {
    fn lines(&self) -> Vec<String> {
        self.running
            .state
            .log()
            .after_sequence(0, usize::MAX)
            .into_iter()
            .map(|line| line.text)
            .filter(|text| text.starts_with("SPEECH"))
            .collect()
    }

    async fn transcribe(&self, content_type: &str, body: &[u8], query: &str) -> Response {
        let address = format!("{}{query}", path::SPEECH_TRANSCRIBE);
        client::post_bytes(self.port, &address, content_type, body)
            .await
            .unwrap()
    }

    async fn speak(&self, body: &str) -> SpeechReply<ClipPlayed> {
        let answer = client::post_json(self.port, path::SPEECH_SPEAK, body)
            .await
            .unwrap();
        reply_of(&answer)
    }
}

fn reply_of<T: DeserializeOwned>(response: &Response) -> SpeechReply<T> {
    serde_json::from_str(&response.body).unwrap()
}

fn done<T: std::fmt::Debug>(reply: SpeechReply<T>) -> T {
    match reply {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, message } => panic!("refused: {error:?} {message}"),
    }
}

fn refused<T: std::fmt::Debug>(reply: SpeechReply<T>) -> SpeechError {
    match reply {
        DevReply::Rejected { error, .. } => error,
        DevReply::Done { value } => panic!("done: {value:?}"),
    }
}

fn words(text: &str) -> Vec<String> {
    split_words(text).unwrap()
}

#[tokio::test]
async fn the_status_says_whether_it_is_configured_and_lists_the_voices_and_limits() {
    let host = speech_host(FakePort::answering()).await;
    let answer = client::get(host.port, path::SPEECH_STATUS).await.unwrap();
    let status: SpeechStatus = done(reply_of(&answer));
    assert!(status.configured);
    assert_eq!(status.missing, None);
    assert_eq!(status.default_voice.as_deref(), Some("voice-default-id"));
    assert_eq!(status.voices.len(), 1);
    assert_eq!(status.voices[0].character, "innkeeper");
    assert_eq!(status.clips_kept, 0);
    assert_eq!(status.limits.minimum_audio_ms, 100);
    assert_eq!(status.limits.maximum_audio_ms, 60_000);

    let host = speech_host(FakePort::unconfigured()).await;
    let answer = client::get(host.port, path::SPEECH_STATUS).await.unwrap();
    let status: SpeechStatus = done(reply_of(&answer));
    assert!(!status.configured);
    assert_eq!(status.missing, Some(MissingSetting::SpeechKey));
}

#[tokio::test]
async fn a_recording_is_transcribed_and_neither_the_audio_nor_the_words_reach_a_log() {
    let host = speech_host(FakePort::answering()).await;
    let answer = host
        .transcribe(
            AUDIO_CONTENT_TYPE,
            &vec![0; ONE_SECOND],
            "?terms=Marta%20Venn&terms=Ember",
        )
        .await;
    assert_eq!(answer.status, StatusCode::OK);
    let transcript: Transcript = done(reply_of(&answer));
    assert_eq!(transcript.text, FAKE_HEARD);
    assert_eq!(transcript.language, "eng");
    assert_eq!(transcript.audio_ms, 1000);
    let heard = host.fake.heard();
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].pcm_bytes, ONE_SECOND);
    assert_eq!(heard[0].key_terms, ["Marta Venn", "Ember"]);
    let lines = host.lines().join("\n");
    assert!(lines.contains("SPEECH transcribed audio_ms=1000 duration_ms="));
    assert!(lines.contains("words=5 terms=2"));
    let everything: Vec<String> = host
        .running
        .state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .map(|line| line.text)
        .collect();
    assert!(
        everything.iter().all(|line| !line.contains("night")),
        "no line holds the transcript"
    );
}

#[tokio::test]
async fn a_transcription_without_terms_is_biased_toward_nothing_when_no_scene_names_exist() {
    let host = speech_host(FakePort::answering()).await;
    let answer = host
        .transcribe(AUDIO_CONTENT_TYPE, &vec![0; ONE_SECOND], "")
        .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(host.fake.heard()[0].key_terms, Vec::<String>::new());
    assert!(
        host.lines()
            .iter()
            .any(|line| line.starts_with("SPEECH key_terms_unavailable error=")),
        "the missing names are recorded, not swallowed"
    );
}

#[tokio::test]
async fn a_recording_that_is_not_declared_or_not_sized_right_is_refused_before_the_supplier() {
    let host = speech_host(FakePort::answering()).await;
    let cases: [(&str, usize, SpeechError); 6] = [
        ("audio/wav", ONE_SECOND, SpeechError::UnsupportedContentType),
        (
            "application/json",
            ONE_SECOND,
            SpeechError::UnsupportedContentType,
        ),
        (AUDIO_CONTENT_TYPE, 0, SpeechError::AudioEmpty),
        (AUDIO_CONTENT_TYPE, 3_201, SpeechError::AudioMisaligned),
        (
            AUDIO_CONTENT_TYPE,
            3_198,
            SpeechError::AudioTooShort {
                audio_ms: 99,
                minimum_ms: 100,
            },
        ),
        (
            AUDIO_CONTENT_TYPE,
            1_920_064,
            SpeechError::AudioTooLong {
                audio_ms: 60_002,
                maximum_ms: 60_000,
            },
        ),
    ];
    for (content_type, bytes, expected) in cases {
        let answer = host.transcribe(content_type, &vec![0; bytes], "").await;
        let error = refused::<Transcript>(reply_of(&answer));
        assert_eq!(error, expected, "{content_type} {bytes}");
        assert_eq!(answer.status, error.status());
    }
    assert!(host.fake.heard().is_empty(), "nothing reached the supplier");
}

#[tokio::test]
async fn a_transcription_refuses_unknown_query_keys_bad_terms_and_an_unset_service() {
    let host = speech_host(FakePort::answering()).await;
    let cases = [
        ("?language=fr", QueryCause::UnknownKey),
        ("?terms=%3Cscript%3E", QueryCause::InvalidTermCharacters),
        ("?terms=", QueryCause::BlankTerm),
        (
            "?terms=one%20two%20three%20four%20five%20six",
            QueryCause::WordyTerm,
        ),
    ];
    for (query, cause) in cases {
        let answer = host
            .transcribe(AUDIO_CONTENT_TYPE, &vec![0; ONE_SECOND], query)
            .await;
        assert_eq!(
            refused::<Transcript>(reply_of(&answer)),
            SpeechError::InvalidQuery { cause },
            "{query}"
        );
    }
    let unset = speech_host(FakePort::unconfigured()).await;
    let answer = unset
        .transcribe(AUDIO_CONTENT_TYPE, &vec![0; ONE_SECOND], "")
        .await;
    assert_eq!(answer.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        refused::<Transcript>(reply_of(&answer)),
        SpeechError::NotConfigured {
            missing: MissingSetting::SpeechKey
        }
    );
    assert!(
        unset
            .lines()
            .contains(&"SPEECH refused verb=transcribe error=not_configured".to_owned())
    );
}

#[test]
fn the_content_type_must_be_audio_l16_at_16_khz_mono_in_any_case() {
    let check = |text: &str| {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_str(text).unwrap());
        check_content_type(&headers)
    };
    assert_eq!(check("audio/L16;rate=16000"), Ok(()));
    assert_eq!(check("Audio/L16; rate=16000; channels=1"), Ok(()));
    assert!(check("audio/L16;rate=44100").is_err());
    assert!(check("audio/L16").is_err());
    assert!(check("audio/L16;rate=16000;channels=2").is_err());
    assert!(check_content_type(&HeaderMap::new()).is_err());
}

#[tokio::test]
async fn speaking_voices_the_text_keeps_a_clip_and_cues_it_once_with_a_line_that_has_no_text() {
    let host = speech_host(FakePort::answering()).await;
    let played = done(
        host.speak(
            r#"{"text": "Welcome to the Ember Roads inn, love.", "character": "innkeeper"}"#,
        )
        .await,
    );
    assert!(clip_number(&played.clip).is_some(), "a clip the host made");
    assert_eq!(played.bytes, u64::try_from(FAKE_MP3.len()).unwrap());
    assert_eq!(played.voice_profile, "eleven_flash_v2_5/character");
    assert_eq!(host.fake.voices(), ["voice-marta-id"]);
    let kept = host
        .running
        .playground_path(&format!("assets/media/{}", played.clip));
    assert_eq!(std::fs::read(kept).unwrap(), FAKE_MP3);
    let scene = host.running.state.playground().snapshot().scene;
    assert_eq!(scene.cue_count, 1);
    assert_eq!(scene.cues.last().unwrap().media, played.clip);
    let second = done(host.speak(r#"{"text": "Another."}"#).await);
    assert_ne!(second.clip, played.clip, "a line voiced twice is two clips");
    assert_eq!(second.voice_profile, "eleven_flash_v2_5/default");
    assert_eq!(host.fake.voices(), ["voice-marta-id", "voice-default-id"]);
    let overridden = done(
        host.speak(r#"{"text": "Third.", "voice": "explicit-id"}"#)
            .await,
    );
    assert_eq!(overridden.voice_profile, "eleven_flash_v2_5/override");
    let lines = host.lines().join("\n");
    assert!(lines.contains(&format!("SPEECH synthesized clip={} bytes=", played.clip)));
    assert!(lines.contains("profile=eleven_flash_v2_5/character"));
    for secret in [
        "Welcome",
        "Ember Roads",
        "voice-marta-id",
        "voice-default-id",
        "explicit-id",
    ] {
        assert!(!lines.contains(secret), "{secret} reached an audit line");
    }
}

#[tokio::test]
async fn speaking_refuses_blank_or_oversized_text_bad_voices_and_a_missing_voice() {
    let host = speech_host(FakePort::answering()).await;
    let long = "a".repeat(4097);
    let cases = [
        (r#"{"text": "  "}"#.to_owned(), SpeechError::TextBlank),
        (
            format!(r#"{{"text": "{long}"}}"#),
            SpeechError::TextTooLong {
                characters: 4097,
                maximum: 4096,
            },
        ),
        (
            r#"{"text": "x", "voice": "has space"}"#.to_owned(),
            SpeechError::InvalidVoice {
                cause: VoiceCause::InvalidCharacters,
            },
        ),
    ];
    for (body, expected) in cases {
        assert_eq!(refused(host.speak(&body).await), expected);
    }
    assert!(matches!(
        refused(
            host.speak(r#"{"text": "x", "character": "bad name"}"#)
                .await
        ),
        SpeechError::InvalidCharacter { .. }
    ));
    assert!(matches!(
        refused(host.speak(r#"{"text": "x", "unknown": 1}"#).await),
        SpeechError::InvalidBody(_)
    ));
    assert!(
        host.fake.voices().is_empty(),
        "nothing reached the supplier"
    );
    host.running.write_asset("voices.json", "{}");
    assert_eq!(
        refused(host.speak(r#"{"text": "x", "character": "elf"}"#).await),
        SpeechError::NoVoice {
            character: Some("elf".to_owned())
        }
    );
}

#[tokio::test]
async fn a_call_of_a_feature_that_is_switched_off_is_refused_before_the_supplier() {
    let host = speech_host(FakePort::answering()).await;
    let switch = |change: ControlsChange| {
        host.running
            .state
            .playground()
            .change_all(vec![change], &[], 0, |_| {})
            .expect("applies");
    };
    switch(ControlsChange::Stt(false));
    let answer = host
        .transcribe(AUDIO_CONTENT_TYPE, &vec![0; ONE_SECOND], "")
        .await;
    assert_eq!(
        refused(reply_of::<Transcript>(&answer)),
        SpeechError::Disabled {
            feature: SpeechFeature::Stt
        }
    );
    assert_eq!(answer.status, StatusCode::CONFLICT);
    // Speaking is a different switch, still on.
    done(
        host.speak(r#"{"text": "Welcome.", "character": "innkeeper"}"#)
            .await,
    );
    switch(ControlsChange::Tts(false));
    let refusal = refused(
        host.speak(r#"{"text": "Welcome.", "character": "innkeeper"}"#)
            .await,
    );
    assert_eq!(
        refusal,
        SpeechError::Disabled {
            feature: SpeechFeature::Tts
        }
    );
    // The one voice call is the speak made while text to speech was on.
    assert_eq!(host.fake.voices().len(), 1, "no voice call while it is off");
    assert!(
        host.fake.heard().is_empty(),
        "no transcription call while it is off"
    );
    switch(ControlsChange::Stt(true));
    done(reply_of::<Transcript>(
        &host
            .transcribe(AUDIO_CONTENT_TYPE, &vec![0; ONE_SECOND], "")
            .await,
    ));
}

#[tokio::test]
async fn a_failing_supplier_is_a_typed_error_with_its_time_and_keeps_no_clip() {
    let host = speech_host(FakePort::failing(SpeechProviderCause::Capacity)).await;
    let error = refused(host.speak(r#"{"text": "x"}"#).await);
    assert_eq!(
        error,
        SpeechError::Provider {
            cause: SpeechProviderCause::Capacity
        }
    );
    assert_eq!(error.key(), "capacity");
    let lines = host.lines();
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("SPEECH failed verb=speak error=capacity duration_ms="))
    );
    assert!(
        !host
            .running
            .playground_path("assets/media/voice-1.mp3")
            .exists()
    );
}

#[test]
fn eviction_removes_the_oldest_voice_clips_and_nothing_else() {
    let directory = tempfile::TempDir::new().unwrap();
    for name in [
        "voice-1.mp3",
        "voice-2.mp3",
        "voice-10.mp3",
        "voice-x.mp3",
        "voice-4.wav",
        "voice-3.ogg",
        "voice-.mp3",
        "my-voice-4.mp3",
        "narration.mp3",
        "plate.png",
    ] {
        std::fs::write(directory.path().join(name), b"x").unwrap();
    }
    assert_eq!(clip_number("voice-10.mp3"), Some(10));
    assert_eq!(clip_number("voice-4.wav"), Some(4));
    assert_eq!(clip_number("voice-x.mp3"), None);
    let names = |directory: &std::path::Path| -> Vec<String> {
        list_clips(directory)
            .unwrap()
            .into_iter()
            .map(|(_, clip): (u64, ClipRow)| clip.name)
            .collect()
    };
    assert_eq!(
        names(directory.path()),
        ["voice-1.mp3", "voice-2.mp3", "voice-4.wav", "voice-10.mp3"]
    );
    assert_eq!(evict_clips(directory.path(), 1).unwrap(), 3);
    assert_eq!(names(directory.path()), ["voice-10.mp3"]);
    for kept in [
        "voice-x.mp3",
        "voice-3.ogg",
        "voice-.mp3",
        "my-voice-4.mp3",
        "narration.mp3",
        "plate.png",
    ] {
        assert!(directory.path().join(kept).exists(), "{kept} was removed");
    }
    assert_eq!(evict_clips(directory.path(), 0).unwrap(), 1);
    assert_eq!(evict_clips(directory.path(), 0).unwrap(), 0);
}

#[tokio::test]
async fn the_clips_route_lists_the_hosts_clips_and_clear_removes_only_them() {
    let host = speech_host(FakePort::answering()).await;
    let one = done(host.speak(r#"{"text": "One."}"#).await);
    let two = done(host.speak(r#"{"text": "Two."}"#).await);
    let other = host.running.playground_path("assets/media/ambience.mp3");
    std::fs::write(&other, b"keep").unwrap();
    let answer = client::get(host.port, path::SPEECH_CLIPS).await.unwrap();
    let clips: Vec<ClipRow> = done(reply_of(&answer));
    assert_eq!(
        clips
            .iter()
            .map(|clip| clip.name.as_str())
            .collect::<Vec<_>>(),
        [one.clip.as_str(), two.clip.as_str()]
    );
    let answer = client::post_json(host.port, path::SPEECH_CLEAR, "")
        .await
        .unwrap();
    assert_eq!(done::<ClipsCleared>(reply_of(&answer)).removed, 2);
    assert!(other.exists());
    assert!(
        host.lines()
            .contains(&"SPEECH cleared removed=2".to_owned())
    );
}

#[test]
fn a_voice_is_set_cleared_and_resolved_from_a_file_written_whole() {
    let directory = tempfile::TempDir::new().unwrap();
    let file = directory.path().join(VOICES_FILE);
    let empty = VoiceBook::read(&file).unwrap();
    assert!(matches!(
        empty.resolve(Some("innkeeper"), None),
        Err(SpeechError::NoVoice { .. })
    ));
    let set: VoiceChanged = change_voice(&file, "innkeeper", Some("abc123")).unwrap();
    assert!(set.has_voice && set.changed);
    let again = change_voice(&file, "innkeeper", Some("abc123")).unwrap();
    assert!(!again.changed, "the same voice changes nothing");
    let book = VoiceBook::read(&file).unwrap();
    let voice = book.resolve(Some("innkeeper"), None).unwrap();
    assert_eq!(
        (voice.id.as_str(), voice.source),
        ("abc123", VoiceSource::Character)
    );
    assert_eq!(book.rows().len(), 1);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.ends_with("}\n") && !text.contains('\r'));
    let cleared = change_voice(&file, "innkeeper", None).unwrap();
    assert!(!cleared.has_voice && cleared.changed);
    assert!(!change_voice(&file, "innkeeper", None).unwrap().changed);
    assert!(change_voice(&file, "bad name", Some("x")).is_err());
    assert!(change_voice(&file, "elf", Some("")).is_err());
    std::fs::write(&file, r#"{"voices": {"elf": 7}}"#).unwrap();
    assert!(matches!(
        VoiceBook::read(&file),
        Err(SpeechError::VoicesInvalid {
            cause: VoicesCause::Malformed(BodyError {
                cause: BodyCause::Data,
                ..
            })
        })
    ));
    std::fs::write(&file, r#"{"voices": {"elf": "has space"}}"#).unwrap();
    assert_eq!(
        VoiceBook::read(&file).err(),
        Some(SpeechError::VoicesInvalid {
            cause: VoicesCause::InvalidVoiceId
        })
    );
}

#[test]
fn the_speech_grammar_reads_every_verb() {
    let read = |text: &str| match crate::command::parse(&words(text)) {
        Ok(Command::Speech(command)) => command,
        other => panic!("not a speech command: {other:?}"),
    };
    assert_eq!(read("speech"), SpeechCommand::Status);
    assert_eq!(read("speech status"), SpeechCommand::Status);
    assert_eq!(
        read("speech speak innkeeper Welcome, love."),
        SpeechCommand::Speak {
            character: Some("innkeeper".to_owned()),
            text: "Welcome, love.".to_owned()
        }
    );
    assert_eq!(
        read("speech speak - hello"),
        SpeechCommand::Speak {
            character: None,
            text: "hello".to_owned()
        }
    );
    assert_eq!(
        read("speech voice innkeeper abc"),
        SpeechCommand::Voice {
            character: "innkeeper".to_owned(),
            voice: Some("abc".to_owned())
        }
    );
    assert_eq!(
        read("speech voice innkeeper clear"),
        SpeechCommand::Voice {
            character: "innkeeper".to_owned(),
            voice: None
        }
    );
    assert_eq!(read("speech clips"), SpeechCommand::Clips);
    assert_eq!(read("speech clear"), SpeechCommand::Clear);
    assert!(matches!(
        read("speech transcribe a.wav"),
        SpeechCommand::Transcribe { .. }
    ));
    for bad in [
        "speech speak innkeeper",
        "speech speak",
        "speech transcribe",
        "speech voice innkeeper",
        "speech clips now",
        "speech mumble",
    ] {
        assert!(
            matches!(
                crate::command::parse(&words(bad)),
                Err(CommandError::Usage(_))
            ),
            "{bad}"
        );
    }
    assert!(matches!(
        crate::command::parse(&words("speech speak bad!name hello")),
        Err(CommandError::BadChoice { .. })
    ));
}

#[test]
fn a_wav_of_16_khz_mono_16_bit_pcm_gives_its_samples_and_any_other_is_refused() {
    let wav = |rate: u32, channels: u16, with_list: bool| {
        let mut bytes = b"RIFF\0\0\0\0WAVE".to_vec();
        if with_list {
            bytes.extend_from_slice(b"LIST\x05\0\0\0abcde\0");
        }
        bytes.extend_from_slice(b"fmt \x10\0\0\0");
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * 2).to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data\x04\0\0\0\x01\x02\x03\x04");
        bytes
    };
    assert_eq!(pcm_of_file(wav(16_000, 1, false)).unwrap(), [1, 2, 3, 4]);
    assert_eq!(pcm_of_file(wav(16_000, 1, true)).unwrap(), [1, 2, 3, 4]);
    assert_eq!(
        pcm_of_file(wav(44_100, 1, false)),
        Err(SpeechError::UnsupportedWav)
    );
    assert_eq!(
        pcm_of_file(wav(16_000, 2, false)),
        Err(SpeechError::UnsupportedWav)
    );
    assert_eq!(
        pcm_of_file(vec![9, 8, 7, 6]).unwrap(),
        [9, 8, 7, 6],
        "raw PCM is kept as is"
    );
}

#[tokio::test]
async fn the_running_servers_guard_lets_a_recording_reach_the_transcribe_handler() {
    let running = start(&STARTER_FILES).await;
    running.switch_speech_on();
    let sent = client::post_bytes(
        running.port,
        path::SPEECH_TRANSCRIBE,
        AUDIO_CONTENT_TYPE,
        &[0; 100],
    )
    .await
    .unwrap();
    assert_eq!(sent.status, StatusCode::BAD_REQUEST, "{}", sent.body);
    assert!(matches!(
        refused::<Transcript>(reply_of(&sent)),
        SpeechError::AudioTooShort { .. }
    ));
    let as_json = client::post_json(running.port, path::SPEECH_TRANSCRIBE, "{}")
        .await
        .unwrap();
    assert_eq!(as_json.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert!(as_json.body.contains("audio/L16"));
}

fn body_error() -> BodyError {
    BodyError {
        cause: BodyCause::Data,
        line: 1,
        column: 2,
        detail: "wrong shape".to_owned(),
    }
}

/// One of every refusal.
fn every_error() -> Vec<SpeechError> {
    vec![
        SpeechError::InvalidBody(body_error()),
        SpeechError::InvalidQuery {
            cause: QueryCause::UnknownKey,
        },
        SpeechError::NotConfigured {
            missing: MissingSetting::SpeechKey,
        },
        SpeechError::UnsupportedContentType,
        SpeechError::UnsupportedWav,
        SpeechError::AudioEmpty,
        SpeechError::AudioMisaligned,
        SpeechError::AudioTooShort {
            audio_ms: 1,
            minimum_ms: 100,
        },
        SpeechError::AudioTooLong {
            audio_ms: 70_000,
            maximum_ms: 60_000,
        },
        SpeechError::TextBlank,
        SpeechError::TextTooLong {
            characters: 5_000,
            maximum: 4_096,
        },
        SpeechError::InvalidCharacter {
            cause: ContentError::InvalidId {
                id: "bad name".to_owned(),
            },
        },
        SpeechError::InvalidVoice {
            cause: VoiceCause::Empty,
        },
        SpeechError::NoVoice { character: None },
        SpeechError::VoicesUnreadable {
            cause: IoCause::from_kind(std::io::ErrorKind::PermissionDenied),
        },
        SpeechError::VoicesInvalid {
            cause: VoicesCause::TooLarge,
        },
        SpeechError::VoicesUnwritable {
            cause: IoCause::from_kind(std::io::ErrorKind::PermissionDenied),
        },
        SpeechError::Provider {
            cause: SpeechProviderCause::Capacity,
        },
        SpeechError::ClipUnstored {
            cause: ClipCause::LibraryFull,
        },
        SpeechError::ClipsUnreadable {
            cause: IoCause::from_kind(std::io::ErrorKind::NotFound),
        },
        SpeechError::Cue {
            cause: PlaygroundChangeError::InvalidBody(body_error()),
        },
        SpeechError::Disabled {
            feature: SpeechFeature::Stt,
        },
        SpeechError::Stopped,
    ]
}

#[test]
fn every_refusal_is_written_with_the_tag_of_its_kind() {
    let errors = every_error();
    for error in &errors {
        let written = serde_json::to_value(error).unwrap();
        assert_eq!(written["error"], error.kind().key(), "{error:?}");
        let read: SpeechError = serde_json::from_value(written).unwrap();
        assert_eq!(&read, error);
    }
    let kinds: Vec<SpeechErrorKind> = errors.iter().map(SpeechError::kind).collect();
    for kind in SpeechErrorKind::ALL {
        assert!(kinds.contains(&kind), "no sample error of kind {kind:?}");
    }
}

#[test]
fn the_audit_key_of_a_provider_failure_is_its_cause_and_any_other_is_its_kind() {
    for error in every_error() {
        match &error {
            SpeechError::Provider { cause } => assert_eq!(error.key(), cause.key()),
            other => assert_eq!(error.key(), other.kind().key()),
        }
    }
}

/// That `key()` is the serde name of each value, `from_key` reads it back and `ALL` lists every
/// value once.
fn assert_closed_keys<T>(all: &[T], key: fn(T) -> &'static str, from_key: fn(&str) -> Option<T>)
where
    T: Copy + PartialEq + std::fmt::Debug + serde::Serialize,
{
    for &value in all {
        assert_eq!(
            serde_json::to_value(value).unwrap(),
            serde_json::json!(key(value))
        );
        assert_eq!(from_key(key(value)), Some(value));
    }
    assert_eq!(from_key("no_such_key"), None);
}

#[test]
fn the_closed_causes_have_keys_equal_to_their_serde_names() {
    assert_closed_keys(&VoiceCause::ALL, VoiceCause::key, VoiceCause::from_key);
    assert_closed_keys(&ClipCause::ALL, ClipCause::key, ClipCause::from_key);
    assert_closed_keys(&QueryCause::ALL, QueryCause::key, QueryCause::from_key);
    assert_closed_keys(
        &SpeechProviderCause::ALL,
        SpeechProviderCause::key,
        SpeechProviderCause::from_key,
    );
}

#[test]
fn a_voice_and_a_clip_cause_each_have_their_own_sentence() {
    let error = SpeechError::InvalidVoice {
        cause: VoiceCause::TooLong,
    };
    assert_eq!(
        error.to_string(),
        "the voice id is not valid: it is too long"
    );
    let error = SpeechError::ClipUnstored {
        cause: ClipCause::LibraryFull,
    };
    assert_eq!(
        error.to_string(),
        "the voice clip could not be kept in the media library: the library is full"
    );
    let sentences: std::collections::HashSet<String> = VoiceCause::ALL
        .iter()
        .map(ToString::to_string)
        .chain(ClipCause::ALL.iter().map(ToString::to_string))
        .chain(QueryCause::ALL.iter().map(ToString::to_string))
        .collect();
    assert_eq!(sentences.len(), 3 + 3 + 7);
}

#[test]
fn the_variable_a_missing_setting_names_is_the_first_the_host_reads() {
    assert_eq!(MissingSetting::SpeechKey.variable(), KEY_VARIABLES[0]);
}

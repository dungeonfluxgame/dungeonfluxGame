//! Tests of voice casting: the judge's closed answer, the cast before a character's first line
//! (once, stored, never over a voice already given), its fallback to the default voice, and
//! `speech cast`, over a fake supplier and a scripted text backend that never touch the network.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use df_ai::text::{TextOutcome, TextRequest};
use df_ai::text_fixture::ScriptedBackend;
use df_content::persona::Persona;
use df_providers::SupplierVoice;
use serde_json::{Value, json};

use super::cast::{cast_schema, read_cast};
use super::cli::SpeechCommand;
use super::fixtures::{FakePort, speech_over};
use super::provider::VoiceListing;
use super::request::{CastAnswerCause, CastRequest, SpeechProviderCause, VoiceCastError, Voiced};
use super::service::Speech;
use crate::command::{Command, split_words};
use crate::test_support::{Running, STARTER_FILES, start};
use crate::text::{BackendChoice, TextRuntime};

const MAN: &str = "voice-daniel-id";
const WOMAN: &str = "voice-lily-id";
const DEFAULT: &str = "voice-default-id";
/// A character with a persona whose look is a woman's.
const WARDEN: &str = "warden";

fn voice(id: &str, name: &str, gender: &str, age: &str) -> SupplierVoice {
    let labels = BTreeMap::from([
        ("gender".to_owned(), gender.to_owned()),
        ("age".to_owned(), age.to_owned()),
    ]);
    SupplierVoice::new(id, name, labels, Some("A test voice.")).unwrap()
}

fn listed() -> Vec<SupplierVoice> {
    vec![
        voice(MAN, "Daniel", "male", "middle_aged"),
        voice(WOMAN, "Lily", "female", "middle_aged"),
    ]
}

fn warden() -> Persona {
    serde_json::from_value(json!({
        "schema": 1, "id": WARDEN, "display_name": "Ilsa Brand",
        "job": "Warden: a grey-haired woman in a guard's coat",
        "personality": ["stern"],
        "backstory": "Kept the river gate for thirty years.",
        "private_backstory": "She let her brother through once."
    }))
    .unwrap()
}

fn answer(voice: &str) -> Value {
    json!({"voice": voice, "why": "An older woman's voice fits the grey-haired warden."})
}

struct CastHost {
    running: Running,
    fake: Arc<FakePort>,
    speech: Speech,
    calls: Arc<AtomicUsize>,
    asked: Arc<std::sync::Mutex<Vec<TextRequest>>>,
}

impl CastHost {
    fn lines(&self) -> Vec<String> {
        self.running
            .state
            .log()
            .after_sequence(0, usize::MAX)
            .into_iter()
            .map(|line| line.text)
            .filter(|text| text.starts_with("SPEECH voice_cast"))
            .collect()
    }

    fn voices_file(&self) -> Value {
        let text =
            std::fs::read_to_string(self.running.playground_path("assets/voices.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn facts_of(&self, n: usize) -> Value {
        self.asked.lock().unwrap()[n].facts.clone()
    }
}

/// A host with `voices` as its voices file, the warden's persona stored, a supplier that lists
/// `listing`, and a judge that answers `answers` in order.
async fn cast_host(
    voices: &str,
    listing: Result<VoiceListing, SpeechProviderCause>,
    answers: Vec<Value>,
) -> CastHost {
    let running = start(&STARTER_FILES).await;
    running.write_asset("voices.json", voices);
    running.switch_speech_on();
    crate::persona::write_persona(&running.playground_path("assets/personas"), &warden()).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
    let script = ScriptedBackend::new(
        answers.into_iter().map(TextOutcome::Answered),
        Arc::clone(&calls),
    )
    .recording(Arc::clone(&asked));
    let recorder = Arc::clone(&running.state);
    let text = TextRuntime::new(
        BackendChoice::Scripted(Arc::new(script), None),
        Arc::new(move |line| recorder.record_line(line)),
    );
    let fake = Arc::new(FakePort::answering().listing(listing));
    let speech = speech_over(&running.state, &fake).with_text(Arc::new(text));
    CastHost {
        running,
        fake,
        speech,
        calls,
        asked,
    }
}

const NO_OWN_VOICES: &str = r#"{"default": "voice-default-id", "voices": {}}"#;

fn played(voiced: &Voiced) {
    assert!(matches!(voiced, Voiced::Played { .. }), "{voiced:?}");
}

#[test]
fn the_judge_must_answer_a_listed_voice_and_say_why() {
    let voices = listed();
    let schema = cast_schema(&voices);
    assert_eq!(schema["properties"]["voice"]["enum"], json!([MAN, WOMAN]));
    assert_eq!(schema["required"], json!(["voice", "why"]));
    let choice = read_cast(&answer(WOMAN), &voices).unwrap();
    assert_eq!(choice.voice, WOMAN);
    assert_eq!(
        read_cast(&answer("voice-unlisted-id"), &voices),
        Err(CastAnswerCause::UnknownVoice)
    );
    assert_eq!(
        read_cast(&json!({"voice": WOMAN, "why": "  "}), &voices),
        Err(CastAnswerCause::Malformed)
    );
    assert_eq!(
        read_cast(&json!({"why": "no voice"}), &voices),
        Err(CastAnswerCause::Malformed)
    );
}

#[tokio::test]
async fn a_first_line_casts_the_voice_once_and_every_line_speaks_in_it() {
    let host = cast_host(
        NO_OWN_VOICES,
        Ok(VoiceListing::Listed(listed())),
        vec![answer(WOMAN)],
    )
    .await;
    played(&host.speech.voice_line(WARDEN, "Halt at the gate.").await);
    played(&host.speech.voice_line(WARDEN, "Who goes there?").await);
    assert_eq!(host.fake.voices(), [WOMAN, WOMAN]);
    assert_eq!(host.fake.listings(), 1);
    assert_eq!(host.calls(), 1);
    assert_eq!(host.voices_file()["voices"][WARDEN], WOMAN);
    // The judge reads the public side whole and which voices are someone's already; the private
    // side never reaches it.
    let facts = host.facts_of(0);
    assert_eq!(
        facts["character"]["job"],
        "Warden: a grey-haired woman in a guard's coat"
    );
    assert_eq!(
        facts["character"]["backstory"],
        "Kept the river gate for thirty years."
    );
    assert!(!facts.to_string().contains("brother"));
    assert_eq!(facts["voices"][0]["labels"]["gender"], "male");
    assert_eq!(facts["voices"][1]["in_use"], false);
    let lines = host.lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with(
        "SPEECH voice_cast character=warden voice_name=\"Lily\" reused=false changed=true "
    ));
    // A voice id never reaches an audit line.
    assert!(!lines[0].contains(WOMAN));
}

#[tokio::test]
async fn the_default_and_other_characters_voices_are_marked_in_use() {
    let voices = r#"{"default": "voice-daniel-id", "voices": {"guard": "voice-lily-id"}}"#;
    let host = cast_host(
        voices,
        Ok(VoiceListing::Listed(listed())),
        vec![answer(WOMAN)],
    )
    .await;
    played(&host.speech.voice_line(WARDEN, "Halt.").await);
    let facts = host.facts_of(0);
    assert_eq!(facts["voices"][0]["in_use"], true);
    assert_eq!(facts["voices"][1]["in_use"], true);
    assert!(host.lines()[0].contains(" reused=true "));
}

#[tokio::test]
async fn two_first_lines_at_once_cast_the_voice_once() {
    let host = cast_host(
        NO_OWN_VOICES,
        Ok(VoiceListing::Listed(listed())),
        vec![answer(WOMAN), answer(MAN)],
    )
    .await;
    let (first, second) = tokio::join!(
        host.speech.voice_line(WARDEN, "Halt."),
        host.speech.voice_line(WARDEN, "Stay back.")
    );
    played(&first);
    played(&second);
    assert_eq!(host.calls(), 1);
    assert_eq!(host.fake.listings(), 1);
    assert_eq!(host.fake.voices(), [WOMAN, WOMAN]);
}

#[tokio::test]
async fn a_voice_already_given_is_never_recast_before_a_line() {
    let voices = r#"{"default": "voice-default-id", "voices": {"warden": "voice-daniel-id"}}"#;
    let host = cast_host(
        voices,
        Ok(VoiceListing::Listed(listed())),
        vec![answer(WOMAN)],
    )
    .await;
    played(&host.speech.voice_line(WARDEN, "Halt.").await);
    assert_eq!(host.fake.voices(), [MAN]);
    assert_eq!(host.fake.listings(), 0);
    assert_eq!(host.calls(), 0);
    assert_eq!(host.voices_file()["voices"][WARDEN], MAN);
}

#[tokio::test]
async fn a_character_without_a_persona_speaks_in_the_default_voice_without_a_cast() {
    let host = cast_host(
        NO_OWN_VOICES,
        Ok(VoiceListing::Listed(listed())),
        vec![answer(WOMAN)],
    )
    .await;
    played(&host.speech.voice_line("ferryman", "Two coppers.").await);
    assert_eq!(host.fake.voices(), [DEFAULT]);
    assert_eq!(host.fake.listings(), 0);
    assert_eq!(host.calls(), 0);
    assert!(host.lines().is_empty());
}

#[tokio::test]
async fn a_failed_cast_is_logged_and_the_line_speaks_in_the_default_voice() {
    let cases = [
        (Err(SpeechProviderCause::Denied), vec![], "denied", 0),
        (Ok(VoiceListing::Unlisted), vec![], "unlisted", 0),
        (Ok(VoiceListing::Listed(Vec::new())), vec![], "no_voices", 0),
        // Refused twice: an unknown id, then again after being told why.
        (
            Ok(VoiceListing::Listed(listed())),
            vec![answer("voice-unlisted-id"), answer("voice-unlisted-id")],
            "answer_refused",
            2,
        ),
    ];
    for (listing, answers, error, calls) in cases {
        let host = cast_host(NO_OWN_VOICES, listing, answers).await;
        played(&host.speech.voice_line(WARDEN, "Halt.").await);
        assert_eq!(host.fake.voices(), [DEFAULT], "{error}");
        assert_eq!(host.calls(), calls, "{error}");
        assert_eq!(host.voices_file()["voices"], json!({}), "{error}");
        let failed: Vec<String> = host
            .lines()
            .into_iter()
            .filter(|line| line.starts_with("SPEECH voice_cast_failed"))
            .collect();
        assert_eq!(failed.len(), 1, "{error}");
        assert!(
            failed[0].starts_with(&format!(
                "SPEECH voice_cast_failed character=warden error={error} duration_ms="
            )),
            "{failed:?}"
        );
    }
}

#[tokio::test]
async fn a_refused_answer_is_asked_again_with_why() {
    let host = cast_host(
        NO_OWN_VOICES,
        Ok(VoiceListing::Listed(listed())),
        vec![answer("voice-unlisted-id"), answer(WOMAN)],
    )
    .await;
    played(&host.speech.voice_line(WARDEN, "Halt.").await);
    assert_eq!(host.calls(), 2);
    assert_eq!(host.fake.voices(), [WOMAN]);
    assert!(host.facts_of(0).get("previous_refusal").is_none());
    assert_eq!(
        host.facts_of(1)["previous_refusal"],
        CastAnswerCause::UnknownVoice.to_string()
    );
    assert!(
        host.lines()
            .contains(&"SPEECH voice_cast_retried character=warden error=unknown_voice".to_owned())
    );
}

#[tokio::test]
async fn speech_cast_replaces_the_voice_a_character_has_and_refuses_one_without_a_persona() {
    let voices = r#"{"default": "voice-default-id", "voices": {"warden": "voice-daniel-id"}}"#;
    let host = cast_host(
        voices,
        Ok(VoiceListing::Listed(listed())),
        vec![answer(WOMAN)],
    )
    .await;
    let cast = host
        .speech
        .cast(CastRequest {
            character: WARDEN.to_owned(),
            model: None,
        })
        .await
        .unwrap();
    assert_eq!(cast.voice, WOMAN);
    assert_eq!(cast.voice_name, "Lily");
    assert!(cast.changed);
    assert!(!cast.reused);
    assert_eq!(host.voices_file()["voices"][WARDEN], WOMAN);
    let refused = host
        .speech
        .cast(CastRequest {
            character: "ferryman".to_owned(),
            model: None,
        })
        .await
        .unwrap_err();
    assert_eq!(
        refused,
        VoiceCastError::NoPersona {
            character: "ferryman".to_owned()
        }
    );
    assert_eq!(refused.key(), "no_persona");
}

#[tokio::test]
async fn speech_cast_without_a_text_surface_is_refused_and_lines_keep_the_default() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("voices.json", NO_OWN_VOICES);
    running.switch_speech_on();
    crate::persona::write_persona(&running.playground_path("assets/personas"), &warden()).unwrap();
    let fake = Arc::new(FakePort::answering().listing(Ok(VoiceListing::Listed(listed()))));
    let speech = speech_over(&running.state, &fake);
    let refused = speech
        .cast(CastRequest {
            character: WARDEN.to_owned(),
            model: None,
        })
        .await
        .unwrap_err();
    assert_eq!(refused, VoiceCastError::NoJudge);
    played(&speech.voice_line(WARDEN, "Halt.").await);
    assert_eq!(fake.voices(), [DEFAULT]);
    assert_eq!(fake.listings(), 0);
}

#[test]
fn the_speech_cast_grammar_reads_a_character_and_an_optional_model() {
    let parse = |line: &str| crate::command::parse(&split_words(line).unwrap());
    let read = |line: &str| match parse(line) {
        Ok(Command::Speech(command)) => command,
        other => panic!("not a speech command: {other:?}"),
    };
    assert_eq!(
        read("speech cast warden"),
        SpeechCommand::Cast {
            character: WARDEN.to_owned(),
            model: None,
        }
    );
    assert_eq!(
        read("speech cast warden model=gpt-6-luna"),
        SpeechCommand::Cast {
            character: WARDEN.to_owned(),
            model: Some("gpt-6-luna".to_owned()),
        }
    );
    for line in [
        "speech cast",
        "speech cast warden voice=x",
        "speech cast a b c",
    ] {
        assert!(parse(line).is_err(), "{line}");
    }
}

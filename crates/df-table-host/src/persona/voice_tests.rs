//! Tests of the voice options of `persona ask` and `persona say`: the thinking cue first, the
//! bubble and the clip cued together in one scene revision, a failed voice that never blocks the
//! line, and a failed turn that clears the thinking indicator. The supplier is a fake.
//!
//! TODO(DEV-12): the exact order of the cue, line and voice records in the page timeline, and a
//! reconnect between the thinking cue and the line.

use std::sync::Arc;

use serde_json::{Value, json};

use super::fixtures::{
    CHARACTER, ScriptedHost, coverage_answer, innkeeper, line_answer, scripted_host_speaking,
};
use super::request::{Answered, BackfillOutcome, PersonaError, PersonaReply, Spoken};
use super::routes::path;
use super::storage::write_persona;
use crate::dev_client::post_json;
use crate::dev_http::DevReply;
use crate::speech::fixtures::FakePort;
use crate::speech::request::SpeechProviderCause;
use crate::speech::{SpeechError, Voiced};

async fn host(answers: Vec<Value>, speech: FakePort) -> (ScriptedHost, Arc<FakePort>) {
    let speech = Arc::new(speech);
    let host = scripted_host_speaking(answers, Arc::clone(&speech)).await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    host.running
        .write_asset("voices.json", r#"{"voices": {"innkeeper": "marta-voice"}}"#);
    (host, speech)
}

fn revision(host: &ScriptedHost) -> u64 {
    host.running.state.playground().revision()
}

fn done<T: std::fmt::Debug>(reply: PersonaReply<T>) -> T {
    match reply {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, message } => panic!("refused: {error:?} {message}"),
    }
}

async fn ask(host: &ScriptedHost, extra: Value) -> PersonaReply<Answered> {
    let mut body = json!({"character": CHARACTER, "question": "What does a room cost?"});
    for (key, value) in extra.as_object().into_iter().flatten() {
        body[key] = value.clone();
    }
    post_json(host.port, path::PERSONA_ASK, &body)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_spoken_ask_without_coverage_thinks_then_cues_the_bubble_and_the_clip_together() {
    let (host, speech) = host(
        vec![line_answer("Two silver a night, love.")],
        FakePort::answering(),
    )
    .await;
    let before = revision(&host);
    let answered = done(
        ask(
            &host,
            json!({"speak": true, "coverage": false, "backfill": false}),
        )
        .await,
    )
    .clone();
    assert_eq!(
        host.calls(),
        1,
        "one model call: the coverage question was skipped"
    );
    assert_eq!(answered.backfill, BackfillOutcome::CoverageSkipped);
    assert_eq!(
        answered.spoken.revision,
        before + 2,
        "the thinking cue, then the bubble and the sound as one revision"
    );
    let Some(Voiced::Played {
        clip,
        voice_profile,
        ..
    }) = &answered.spoken.voice
    else {
        panic!("not voiced: {:?}", answered.spoken.voice);
    };
    assert!(clip.starts_with("voice-") && clip.ends_with(".mp3"));
    assert_eq!(voice_profile, "eleven_flash_v2_5/character");
    assert_eq!(speech.voices(), ["marta-voice"]);
    assert_eq!(
        speech.spoken_characters(),
        ["Two silver a night, love.".chars().count()]
    );
    let scene = host.running.state.playground().snapshot().scene;
    assert_eq!(scene.cue_count, 1);
    assert_eq!(&scene.cues[0].media, clip);
}

#[tokio::test]
async fn an_ask_that_does_not_speak_cues_no_sound_and_asks_the_supplier_nothing() {
    let (host, speech) = host(
        vec![
            coverage_answer(&[]),
            line_answer("Two silver a night, love."),
        ],
        FakePort::answering(),
    )
    .await;
    let answered = done(ask(&host, json!({})).await);
    assert_eq!(answered.spoken.voice, None);
    assert!(speech.voices().is_empty());
    assert_eq!(
        host.running.state.playground().snapshot().scene.cue_count,
        0
    );
}

#[tokio::test]
async fn a_voice_that_fails_is_reported_and_the_bubble_is_still_cued() {
    let (host, _) = host(
        vec![line_answer("Two silver a night, love.")],
        FakePort::failing(SpeechProviderCause::Denied),
    )
    .await;
    let before = revision(&host);
    let answered = done(ask(&host, json!({"speak": true, "coverage": false})).await);
    assert_eq!(answered.spoken.revision, before + 2);
    assert_eq!(
        answered.spoken.voice,
        Some(Voiced::Failed {
            error: SpeechError::Provider {
                cause: SpeechProviderCause::Denied
            }
        })
    );
    assert_eq!(
        host.running.state.playground().snapshot().scene.cue_count,
        0
    );
    assert!(
        host.lines_starting_with("SPEECH line_unvoiced")
            .contains(&"SPEECH line_unvoiced character=innkeeper error=denied".to_owned())
    );
}

#[tokio::test]
async fn a_character_with_no_voice_and_no_default_is_a_failed_voice_not_a_failed_line() {
    let (host, _) = host(
        vec![line_answer("Two silver, love.")],
        FakePort::answering(),
    )
    .await;
    host.running.write_asset("voices.json", "{}");
    let answered = done(ask(&host, json!({"speak": true, "coverage": false})).await);
    assert!(matches!(
        answered.spoken.voice,
        Some(Voiced::Failed {
            error: SpeechError::NoVoice { .. }
        })
    ));
}

#[tokio::test]
async fn a_turn_that_fails_clears_the_thinking_indicator() {
    let refused_line = json!({
        "line": "Two silver.", "behavior": "no-such-behavior",
        "fired_triggers": [], "revealed_secrets": []
    });
    let (host, speech) = host(vec![refused_line], FakePort::answering()).await;
    let before = revision(&host);
    let reply = ask(&host, json!({"speak": true, "coverage": false})).await;
    assert!(matches!(
        reply,
        DevReply::Rejected {
            error: PersonaError::AnswerRefused { .. },
            ..
        }
    ));
    assert_eq!(revision(&host), before + 2, "thinking, then quiet");
    assert!(speech.voices().is_empty(), "a refused line is never voiced");
}

#[tokio::test]
async fn a_spoken_say_cues_the_bubble_and_the_clip_in_one_revision() {
    let (host, speech) = host(vec![line_answer("Welcome, love.")], FakePort::answering()).await;
    let before = revision(&host);
    let body = json!({"character": CHARACTER, "situation": "A traveller arrives", "speak": true});
    let reply: PersonaReply<Spoken> = post_json(host.port, path::PERSONA_SAY, &body)
        .await
        .unwrap();
    let spoken = done(reply);
    assert_eq!(spoken.revision, before + 2);
    assert!(matches!(spoken.voice, Some(Voiced::Played { .. })));
    assert_eq!(speech.voices(), ["marta-voice"]);
    let lines = host.lines_starting_with("SPEECH").join("\n");
    assert!(lines.contains("SPEECH synthesized clip=voice-"));
    assert!(
        !lines.contains("Welcome"),
        "no audit line holds the spoken text"
    );
}

//! What the persona prompts carry since the voice work: the layered voice, the recent lines, the
//! asker, the world facts and the public note reach the line's facts; the backfill is asked once
//! more with the refusal's reason; coverage holds the world facts and the expertise; and the
//! `voice` and `expertise` verbs and the `voice generate` route.
//!
//! TODO(DEV-12): `persona generate` writing voice, expertise and packs end to end through the
//! route, and the packs a model picks when the world bible offers some.

use serde_json::{Value, json};

use super::cli::PersonaCommand;
use super::fixtures::{
    CHARACTER, ScriptedHost, append_backstory_answer, coverage_answer, innkeeper, line_answer,
    privacy_answer, scripted_host, write_market,
};
use super::request::{Answered, BackfillOutcome, Edited, PersonaError, PersonaReply, Spoken};
use super::routes::path;
use super::storage::write_persona;
use crate::command::{Command, parse};
use crate::dev_client::post_json;
use crate::dev_http::DevReply;
use df_content::persona::{PersonaEdit, PersonaField, VoiceField};

fn done<T: std::fmt::Debug>(reply: PersonaReply<T>) -> T {
    match reply {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, message } => panic!("refused: {error:?} {message}"),
    }
}

async fn ask(host: &ScriptedHost, extra: Value) -> PersonaReply<Answered> {
    let mut body =
        json!({"character": CHARACTER, "question": "What does a room cost?", "coverage": true});
    for (key, value) in extra.as_object().into_iter().flatten() {
        body[key] = value.clone();
    }
    post_json(host.port, path::PERSONA_ASK, &body)
        .await
        .unwrap()
}

fn command(text: &str) -> PersonaCommand {
    let words: Vec<String> = text.split_whitespace().map(str::to_owned).collect();
    match parse(&words).unwrap() {
        Command::Persona(command) => command,
        other => panic!("not a persona command: {other:?}"),
    }
}

fn voiced_innkeeper() -> df_content::persona::Persona {
    innkeeper()
        .edit(&[
            PersonaEdit::SetVoiceText {
                field: VoiceField::Rhythm,
                text: "short and dry".to_owned(),
            },
            PersonaEdit::AddVoiceEntry {
                field: VoiceField::Samples,
                text: "Room's two silver, love.".to_owned(),
            },
            PersonaEdit::AddEntry {
                field: PersonaField::Expertise,
                text: "Knows every traveller's boots.".to_owned(),
            },
        ])
        .unwrap()
}

#[tokio::test]
async fn the_line_carries_voice_layers_recent_lines_the_asker_the_world_and_the_public_note() {
    let mut answer = line_answer("Two silver, love.");
    answer["quote"] = json!({"item": "room", "qty": 1, "total_cp": 200});
    let host = scripted_host(vec![answer]).await;
    write_persona(&host.directory(), &voiced_innkeeper()).unwrap();
    let answered = done(
        ask(
            &host,
            json!({
                "coverage": false, "backfill": false, "asked_by": "pip",
                "recent": ["Marta: Room's free.", "Pip: How much?"]
            }),
        )
        .await,
    );
    assert_eq!(answered.spoken.said.quoted_cp, Some(200));
    let facts = host.facts_of(0);
    assert_eq!(facts["now"]["asked_by"], "pip");
    assert_eq!(
        facts["now"]["recent"],
        json!(["Marta: Room's free.", "Pip: How much?"])
    );
    let layers = facts["context"]["voice_layers"].as_array().unwrap();
    let persona_layer = layers
        .iter()
        .find(|layer| layer["layer"] == "persona")
        .unwrap();
    assert_eq!(persona_layer["voice"]["rhythm"], "short and dry");
    assert_eq!(
        facts["context"]["character"]["expertise"][0],
        "Knows every traveller's boots."
    );
    assert!(facts["context"].get("public_note").is_some(), "{facts}");
    assert!(
        facts["context"]["public_note"]
            .as_str()
            .is_some_and(|note| note.contains("public"))
    );
    let instructions = host.instructions_of(0);
    for needed in [
        "OWN earlier wording",
        "at most once in any three lines",
        "never copy one verbatim",
        "never by merely accepting an offer",
        "come only from the world facts",
        "total_cp",
    ] {
        assert!(instructions.contains(needed), "{needed}");
    }
}

#[tokio::test]
async fn the_recent_lines_of_a_prompt_are_bounded() {
    let host = scripted_host(vec![line_answer("Aye.")]).await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    let recent: Vec<String> = (0..20).map(|n| format!("{n}{}", "x".repeat(500))).collect();
    done(
        ask(
            &host,
            json!({"coverage": false, "backfill": false, "recent": recent}),
        )
        .await,
    );
    let sent = host.facts_of(0)["now"]["recent"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(sent.len(), 6, "only the latest lines");
    assert!(sent[0].as_str().unwrap().starts_with("14"));
    assert!(
        sent.iter()
            .all(|line| line.as_str().unwrap().chars().count() <= 200)
    );
}

#[tokio::test]
async fn say_takes_recent_and_asked_by_too() {
    let host = scripted_host(vec![line_answer("Aye.")]).await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    let body = json!({
        "character": CHARACTER, "situation": "A traveller waits.",
        "recent": ["Pip: Hello."], "asked_by": "pip"
    });
    let spoken: PersonaReply<Spoken> = post_json(host.port, path::PERSONA_SAY, &body)
        .await
        .unwrap();
    done(spoken);
    let facts = host.facts_of(0);
    assert_eq!(facts["now"]["recent"], json!(["Pip: Hello."]));
    assert_eq!(facts["now"]["asked_by"], "pip");
}

#[tokio::test]
async fn coverage_sees_the_expertise_and_the_world_facts() {
    let host = scripted_host(vec![coverage_answer(&[]), line_answer("Aye.")]).await;
    write_persona(&host.directory(), &voiced_innkeeper()).unwrap();
    write_market(&host);
    done(ask(&host, json!({})).await);
    let facts = host.facts_of(0);
    assert_eq!(
        facts["persona"]["expertise"][0],
        "Knows every traveller's boots."
    );
    assert!(facts.get("world").is_some(), "{facts}");
    assert!(facts.get("public_note").is_some());
}

#[tokio::test]
async fn a_refused_backfill_is_asked_once_more_with_the_reason_and_can_then_succeed() {
    let host = scripted_host(vec![
        coverage_answer(&[("backstory", "childhood")]),
        json!({"edits": "not a list"}),
        append_backstory_answer("She grew up above the taproom."),
        privacy_answer(false),
        line_answer("Above the inn, love."),
    ])
    .await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    write_market(&host);
    let answered = done(ask(&host, json!({})).await);
    assert!(
        matches!(answered.backfill, BackfillOutcome::Backfilled { .. }),
        "{:?}",
        answered.backfill
    );
    assert!(host.facts_of(1).get("previous_refusal").is_none());
    let reason = host.facts_of(2)["previous_refusal"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(reason.contains("not the object asked for"), "{reason}");
    assert!(host.facts_of(1).get("world").is_some());
    assert!(
        host.instructions_of(1)
            .contains("Never invent a price, a quantity or a place")
    );
    assert!(
        host.persona_lines()
            .iter()
            .any(|line| line == "PERSONA backfill_retried id=innkeeper")
    );
}

#[tokio::test]
async fn a_backfill_refused_twice_is_reported_and_the_line_is_still_said() {
    let host = scripted_host(vec![
        coverage_answer(&[("backstory", "childhood")]),
        json!({"edits": "not a list"}),
        json!({"edits": "still not a list"}),
        line_answer("Above the inn, love."),
    ])
    .await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    let answered = done(ask(&host, json!({})).await);
    assert!(
        matches!(
            answered.backfill,
            BackfillOutcome::BackfillFailed {
                error: PersonaError::AnswerRefused { .. }
            }
        ),
        "{:?}",
        answered.backfill
    );
    assert_eq!(answered.spoken.said.line, "Above the inn, love.");
    assert_eq!(host.calls(), 4, "one retry and no more");
}

#[test]
fn the_voice_and_expertise_verbs_build_typed_edits() {
    let edits = |text: &str| match command(text) {
        PersonaCommand::Edit { edits, .. } => edits,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        edits("persona voice mara set rhythm short and brisk"),
        [PersonaEdit::SetVoiceText {
            field: VoiceField::Rhythm,
            text: "short and brisk".to_owned()
        }]
    );
    assert_eq!(
        edits("persona voice mara sample add Ripe today"),
        [PersonaEdit::AddVoiceEntry {
            field: VoiceField::Samples,
            text: "Ripe today".to_owned()
        }]
    );
    assert_eq!(
        edits("persona voice mara sample drop 1"),
        [PersonaEdit::RemoveVoiceEntry {
            field: VoiceField::Samples,
            index: 1
        }]
    );
    assert_eq!(
        edits("persona voice mara add tics Now then"),
        [PersonaEdit::AddVoiceEntry {
            field: VoiceField::Tics,
            text: "Now then".to_owned()
        }]
    );
    assert_eq!(
        edits("persona voice mara pack add market-folk"),
        [PersonaEdit::AddVoicePack {
            pack: "market-folk".to_owned()
        }]
    );
    assert_eq!(
        edits("persona voice mara pack drop market-folk"),
        [PersonaEdit::RemoveVoicePack {
            pack: "market-folk".to_owned()
        }]
    );
    assert_eq!(
        edits("persona expertise mara add Knows fruit"),
        [PersonaEdit::AddEntry {
            field: PersonaField::Expertise,
            text: "Knows fruit".to_owned()
        }]
    );
    assert_eq!(
        edits("persona expertise mara drop 0"),
        [PersonaEdit::RemoveEntry {
            field: PersonaField::Expertise,
            index: 0
        }]
    );
    let words: Vec<String> = "persona voice mara set tics x"
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    assert!(parse(&words).is_err(), "tics is a list, not a text");
    let PersonaCommand::VoiceGenerate { id, model } =
        command("persona voice mara generate model=gpt-6.1-sol")
    else {
        panic!("not a voice generate");
    };
    assert_eq!(
        (id.as_str(), model.as_deref()),
        ("mara", Some("gpt-6.1-sol"))
    );
}

#[test]
fn ask_and_say_read_recent_and_from_before_the_free_text() {
    let PersonaCommand::Ask {
        recent,
        asked_by,
        question,
        ..
    } = command("persona ask mara recent=Pip:_hello from=pip recent=Mara:_hi How much?")
    else {
        panic!("not an ask");
    };
    assert_eq!(recent, ["Pip:_hello", "Mara:_hi"]);
    assert_eq!(asked_by.as_deref(), Some("pip"));
    assert_eq!(question, "How much?");
    let PersonaCommand::Say {
        recent, asked_by, ..
    } = command("persona say mara from=pip recent=x A buyer waits")
    else {
        panic!("not a say");
    };
    assert_eq!(
        (recent, asked_by.as_deref()),
        (vec!["x".to_owned()], Some("pip"))
    );
}

fn voice_answer(packs: Value) -> Value {
    json!({
        "voice": {
            "rhythm": "slow", "diction": {"favored": [], "never": []}, "tics": [],
            "address": "friend", "idioms": [], "deflection": "changes the subject",
            "samples": ["Take your time."]
        },
        "expertise": ["Knows the road north."],
        "voice_packs": packs
    })
}

#[tokio::test]
async fn voice_generate_changes_only_the_voice_the_expertise_and_the_packs() {
    let host = scripted_host(vec![voice_answer(json!([]))]).await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    let body = json!({"id": CHARACTER});
    let reply: PersonaReply<Edited> = post_json(host.port, path::PERSONA_VOICE, &body)
        .await
        .unwrap();
    let edited = done(reply);
    assert!(edited.changed);
    assert_eq!(edited.persona.voice().address, "friend");
    assert_eq!(edited.persona.expertise(), ["Knows the road north."]);
    let before = innkeeper();
    assert_eq!(edited.persona.job(), before.job());
    assert_eq!(
        edited.persona.private_backstory(),
        before.private_backstory()
    );
    assert_eq!(edited.persona.private_fears(), before.private_fears());
    let facts = host.facts_of(0);
    assert!(facts["available_packs"].is_array());
    assert!(
        !facts.to_string().contains("Tomas"),
        "a listener's view only"
    );
}

#[tokio::test]
async fn voice_generate_refuses_a_pack_that_is_not_on_offer_and_changes_nothing() {
    let host = scripted_host(vec![voice_answer(json!(["no-such-pack"]))]).await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    let before = host.file("innkeeper.persona.json");
    let body = json!({"id": CHARACTER});
    let reply: PersonaReply<Edited> = post_json(host.port, path::PERSONA_VOICE, &body)
        .await
        .unwrap();
    assert!(matches!(
        reply,
        DevReply::Rejected {
            error: PersonaError::AnswerRefused { .. },
            ..
        }
    ));
    assert_eq!(host.file("innkeeper.persona.json"), before);
}

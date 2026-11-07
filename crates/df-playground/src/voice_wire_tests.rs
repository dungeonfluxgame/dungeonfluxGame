use crate::dev_reply::DevReply;
use crate::voice_wire::{AskRequest, PersonaSummary, SpeechStatus, Transcript};

fn parse_reply<T: serde::de::DeserializeOwned>(
    text: &str,
) -> Result<DevReply<T>, serde_json::Error> {
    serde_json::from_str(text)
}

#[test]
fn a_done_reply_carries_its_value() {
    let reply: DevReply<SpeechStatus> = parse_reply(
        r#"{"outcome":"done","value":{"configured":false,"missing":"ELEVENLABS_API_KEY","x":1}}"#,
    )
    .expect("reads");
    assert_eq!(
        reply,
        DevReply::Done {
            value: SpeechStatus {
                configured: false,
                missing: Some("ELEVENLABS_API_KEY".to_owned())
            }
        }
    );
}

#[test]
fn the_transcript_and_the_persona_list_read_as_the_host_writes_them() {
    let reply: DevReply<Transcript> = parse_reply(
        r#"{"outcome":"done","value":{"text":"Hello there","audio_ms":2000,"duration_ms":1}}"#,
    )
    .expect("reads");
    let DevReply::Done { value } = reply else {
        panic!("expected a value");
    };
    assert_eq!(value.text, "Hello there");
    assert_eq!(value.duration_ms, 1);
    let list: DevReply<Vec<PersonaSummary>> = parse_reply(
        r#"{"outcome":"done","value":[{"id":"innkeeper","display_name":"Marta Venn","job":"x"}]}"#,
    )
    .expect("reads");
    assert!(matches!(list, DevReply::Done { value } if value[0].display_name == "Marta Venn"));
}

#[test]
fn the_types_that_hold_a_transcript_or_a_question_print_no_text() {
    let reply: DevReply<Transcript> =
        parse_reply(r#"{"outcome":"done","value":{"text":"the secret words","duration_ms":5}}"#)
            .expect("reads");
    assert!(!format!("{reply:?}").contains("secret"));
    let body = AskRequest::spoken("innkeeper", "the secret words");
    assert!(!format!("{body:?}").contains("secret"));
}

#[test]
fn a_spoken_question_is_sent_without_coverage_or_backfill() {
    let body = AskRequest::spoken("innkeeper", "How much for a room?");
    let json = serde_json::to_value(&body).expect("writes");
    assert_eq!(
        json,
        serde_json::json!({
            "character": "innkeeper",
            "question": "How much for a room?",
            "speak": true,
            "coverage": false,
            "backfill": false,
        })
    );
}

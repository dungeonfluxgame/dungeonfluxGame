use crate::dev_reply::HostReply;
use crate::voice_wire::{
    AskRequest, MissingSetting, PersonaSummary, SpeechErrorKind, SpeechStatus, Transcript,
};

fn parse_reply<T: serde::de::DeserializeOwned>(
    text: &str,
) -> Result<HostReply<T>, serde_json::Error> {
    serde_json::from_str(text)
}

#[test]
fn a_done_reply_carries_its_value() {
    let reply: HostReply<SpeechStatus> = parse_reply(
        r#"{"outcome":"done","value":{"configured":false,"missing":"speech_key",
        "tts_model":"flash","stt_model":"scribe","default_voice":null,"voices":[
        {"character":"innkeeper","voice":"v1"}],"clips_kept":2,"limits":{"minimum_audio_ms":100,
        "maximum_audio_ms":60000,"maximum_text_characters":4096,"maximum_key_terms":32,
        "maximum_kept_clips":500}}}"#,
    )
    .expect("reads");
    let HostReply::Done { value } = reply else {
        panic!("expected a value");
    };
    assert!(!value.configured);
    assert_eq!(value.missing, Some(MissingSetting::SpeechKey));
    assert_eq!(value.voices[0].character, "innkeeper");
    assert_eq!(value.limits.minimum_audio_ms, 100);
}

#[test]
fn the_transcript_and_the_persona_list_read_as_the_host_writes_them() {
    let reply: HostReply<Transcript> = parse_reply(
        r#"{"outcome":"done","value":{"text":"Hello there","language":"eng","audio_ms":2000,
        "duration_ms":1}}"#,
    )
    .expect("reads");
    let HostReply::Done { value } = reply else {
        panic!("expected a value");
    };
    assert_eq!(value.text, "Hello there");
    assert_eq!(value.duration_ms, 1);
    let list: HostReply<Vec<PersonaSummary>> = parse_reply(
        r#"{"outcome":"done","value":[{"id":"innkeeper","display_name":"Marta Venn","job":"x"}]}"#,
    )
    .expect("reads");
    assert!(matches!(list, HostReply::Done { value } if value[0].display_name == "Marta Venn"));
}

#[test]
fn the_types_that_hold_a_transcript_or_a_question_print_no_text() {
    let reply: HostReply<Transcript> = parse_reply(
        r#"{"outcome":"done","value":{"text":"the secret words","language":"eng","audio_ms":9,
        "duration_ms":5}}"#,
    )
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

#[test]
fn every_speech_error_kind_key_is_its_serde_name_and_round_trips() {
    for kind in SpeechErrorKind::ALL {
        assert_eq!(
            serde_json::to_value(kind).expect("writes"),
            serde_json::json!(kind.key()),
            "{kind:?}"
        );
        assert_eq!(SpeechErrorKind::from_key(kind.key()), Some(kind));
    }
    assert_eq!(SpeechErrorKind::from_key("something_else"), None);
}

#[test]
fn a_missing_setting_key_is_its_serde_name_and_round_trips() {
    for setting in MissingSetting::ALL {
        assert_eq!(
            serde_json::to_value(setting).expect("writes"),
            serde_json::json!(setting.key())
        );
        assert_eq!(MissingSetting::from_key(setting.key()), Some(setting));
    }
}

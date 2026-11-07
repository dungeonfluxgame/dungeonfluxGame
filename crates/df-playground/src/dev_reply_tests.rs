use crate::dev_reply::{DevReply, HostReply, Rejection};
use crate::voice_wire::SpeechErrorKind;

#[test]
fn a_done_reply_gives_its_value() {
    let reply: HostReply<u32> =
        serde_json::from_str(r#"{"outcome":"done","value":7}"#).expect("reads");
    assert_eq!(reply.into_result(), Ok(7));
}

#[test]
fn a_refusal_gives_the_error_kind_and_the_hosts_sentence() {
    let text = r#"{"outcome":"rejected","error":{"error":"audio_too_short"},"message":"short"}"#;
    let reply: HostReply<u32> = serde_json::from_str(text).expect("reads");
    assert_eq!(
        reply.into_result(),
        Err(Rejection {
            kind: Some(SpeechErrorKind::AudioTooShort),
            message: "short".to_owned()
        })
    );
}

#[test]
fn another_modules_error_tag_has_no_speech_kind_but_keeps_the_sentence() {
    let text = r#"{"outcome":"rejected","error":{"error":"unknown_fixture"},"message":"no"}"#;
    let reply: HostReply<u32> = serde_json::from_str(text).expect("reads");
    assert_eq!(
        reply.into_result(),
        Err(Rejection {
            kind: None,
            message: "no".to_owned()
        })
    );
}

#[test]
fn the_guards_refusal_has_a_sentence_and_no_kind() {
    let text = r#"{"outcome":"rejected","cause":null,"message":"not here","problems":[]}"#;
    let reply: HostReply<u32> = serde_json::from_str(text).expect("reads");
    assert_eq!(
        reply.into_result(),
        Err(Rejection {
            kind: None,
            message: "not here".to_owned()
        })
    );
}

#[test]
fn a_refusal_is_written_with_the_errors_sentence() {
    let reply: DevReply<u32, String> = DevReply::from_result(Err("busy".to_owned()));
    assert_eq!(
        serde_json::to_value(&reply).expect("writes"),
        serde_json::json!({"outcome": "rejected", "error": "busy", "message": "busy"})
    );
    let done: DevReply<u32, String> = DevReply::from_result(Ok(3));
    assert_eq!(
        serde_json::to_value(&done).expect("writes"),
        serde_json::json!({"outcome": "done", "value": 3})
    );
}

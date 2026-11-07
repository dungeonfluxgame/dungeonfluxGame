use crate::dev_reply::{DevReply, Rejection};

#[test]
fn a_done_reply_gives_its_value() {
    let reply: DevReply<u32> =
        serde_json::from_str(r#"{"outcome":"done","value":7}"#).expect("reads");
    assert_eq!(reply.into_result(), Ok(7));
}

#[test]
fn a_refusal_gives_the_error_key_and_the_hosts_sentence() {
    let text = r#"{"outcome":"rejected","error":{"error":"audio_too_short"},"message":"short"}"#;
    let reply: DevReply<u32> = serde_json::from_str(text).expect("reads");
    assert_eq!(
        reply.into_result(),
        Err(Rejection {
            key: "audio_too_short".to_owned(),
            message: "short".to_owned()
        })
    );
}

#[test]
fn the_guards_refusal_has_a_sentence_and_no_key() {
    let text = r#"{"outcome":"rejected","cause":null,"message":"not here","problems":[]}"#;
    let reply: DevReply<u32> = serde_json::from_str(text).expect("reads");
    assert_eq!(
        reply.into_result(),
        Err(Rejection {
            key: String::new(),
            message: "not here".to_owned()
        })
    );
}

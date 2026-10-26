use super::*;

#[test]
fn transcript_cursor_and_speaker_filters_are_independent() {
    let lines = vec![
        json!({"seq":1,"speaker":"Brynn","actor_id":"hero:1"}),
        json!({"seq":2,"speaker":"Marek","actor_id":"patron:merchant"}),
    ];
    assert_eq!(
        transcript_document(lines.clone(), 1, None, Some("merchant")),
        json!([{ "seq":2,"speaker":"Marek","actor_id":"patron:merchant" }])
    );
    assert_eq!(
        transcript_document(lines.clone(), 0, Some(("brynn", 0)), None)
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        transcript_document(lines, 1, None, Some("Marek"))[0]["seq"],
        2
    );
}

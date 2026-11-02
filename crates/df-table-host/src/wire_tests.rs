use df_table::event::RejectionCause;
use serde_json::json;

use crate::wire::{EffectRequest, MAXIMUM_LISTED_PROBLEMS, Reply, ValueForm, capped};

#[test]
fn a_request_without_a_value_form_reads_its_values_as_json() {
    let request: EffectRequest =
        serde_json::from_value(json!({"op": "set", "id": "pip", "field": "hp", "value": 3}))
            .expect("parses");
    assert!(matches!(
        request,
        EffectRequest::Set {
            values: ValueForm::Json,
            ..
        }
    ));
}

#[test]
fn every_request_survives_being_written_and_read_back() {
    let requests = vec![
        EffectRequest::Spawn {
            id: "pip".into(),
            kind: "character".into(),
            x: Some(1.5),
            y: None,
            fields: serde_json::Map::from_iter([("name".to_owned(), json!("Pip"))]),
            values: ValueForm::Text,
        },
        EffectRequest::Despawn { id: "pip".into() },
        EffectRequest::Set {
            id: "pip".into(),
            field: "hp".into(),
            value: json!(3),
            values: ValueForm::Json,
        },
        EffectRequest::Move {
            id: "pip".into(),
            x: 1.0,
            y: 2.0,
        },
        EffectRequest::Clear {},
    ];
    for request in requests {
        let text = serde_json::to_string(&request).expect("writes");
        let back: EffectRequest = serde_json::from_str(&text).expect("reads");
        assert_eq!(back, request);
    }
}

#[test]
fn a_clear_request_with_an_extra_key_is_refused_like_every_other_request() {
    for text in [
        r#"{"op": "clear", "everything": true}"#,
        r#"{"op": "despawn", "id": "pip", "force": true}"#,
        r#"{"op": "teleport"}"#,
    ] {
        assert!(
            serde_json::from_str::<EffectRequest>(text).is_err(),
            "{text}"
        );
    }
}

#[test]
fn a_reply_is_a_tagged_object_and_reads_back_the_same() {
    let done = Reply::Done {
        revision: 4,
        notices: vec!["note".into()],
    };
    assert_eq!(
        serde_json::to_value(&done).expect("writes"),
        json!({"outcome": "done", "revision": 4, "notices": ["note"]})
    );
    let rejected = Reply::Rejected {
        cause: Some(RejectionCause::DuplicateElement),
        message: "no".into(),
        problems: Vec::new(),
    };
    let text = serde_json::to_string(&rejected).expect("writes");
    assert!(text.contains(r#""cause":"duplicate_element""#));
    assert_eq!(
        serde_json::from_str::<Reply>(&text).expect("reads"),
        rejected
    );
}

#[test]
fn a_long_list_of_problems_is_cut_with_a_count_of_the_rest() {
    let many: Vec<String> = (0..MAXIMUM_LISTED_PROBLEMS + 7)
        .map(|index| index.to_string())
        .collect();
    let cut = capped(many);
    assert_eq!(cut.len(), MAXIMUM_LISTED_PROBLEMS + 1);
    assert_eq!(cut.last().map(String::as_str), Some("and 7 more"));
    let few = vec!["a".to_owned()];
    assert_eq!(capped(few.clone()), few);
}

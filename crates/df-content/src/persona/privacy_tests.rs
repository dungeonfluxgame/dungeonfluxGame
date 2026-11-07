use serde_json::json;

use crate::persona::privacy::*;

#[test]
fn a_verdict_that_leaks_is_read_with_its_excerpt() {
    let verdict = parse_privacy(&json!({
        "leaks": true, "excerpt": "her brother went north", "reason": "restates a secret"
    }))
    .unwrap();
    assert!(verdict.leaks);
    assert_eq!(verdict.excerpt.as_deref(), Some("her brother went north"));
}

#[test]
fn a_verdict_that_does_not_leak_is_read() {
    let verdict = parse_privacy(&json!({
        "leaks": false, "excerpt": null, "reason": "a price is public"
    }))
    .unwrap();
    assert!(!verdict.leaks);
    assert_eq!(verdict.excerpt, None);
}

#[test]
fn a_malformed_or_unreasoned_verdict_is_refused() {
    for answer in [
        json!({"leaks": "yes", "excerpt": null, "reason": "x"}),
        json!({"leaks": true, "excerpt": null, "reason": "x", "extra": 1}),
        json!([]),
    ] {
        let error = parse_privacy(&answer).unwrap_err();
        assert_eq!(error.key(), "malformed", "{answer}");
    }
    let blank = parse_privacy(&json!({"leaks": false, "excerpt": null, "reason": "  "}));
    assert_eq!(blank, Err(PrivacyError::BlankReason));
}

#[test]
fn the_leak_report_is_added_to_a_schema_and_split_from_an_answer() {
    let schema = with_leak_report(json!({
        "type": "object", "required": ["line"], "properties": {"line": {"type": "string"}}
    }));
    assert_eq!(schema["required"].as_array().unwrap().len(), 3);
    assert!(schema["properties"]["leaks_private_side"].is_object());
    let (rest, report) = split_leak_report(&json!({
        "line": "hello", "leaks_private_side": true, "leaked_excerpt": "north"
    }))
    .unwrap();
    assert_eq!(rest, json!({"line": "hello"}));
    assert_eq!(
        report,
        LeakReport {
            leaks: true,
            excerpt: Some("north".to_owned())
        }
    );
    assert!(split_leak_report(&json!({"line": "hello"})).is_err());
}

#[test]
fn public_world_facts_reach_the_judge_and_are_never_a_leak() {
    use serde_json::Value;

    let world = json!({"audience": "public", "stalls": [{"items": [{"id": "apple"}]}]});
    let facts = privacy_facts(&["Three copper an apple.".to_owned()], "fears", &world);
    assert_eq!(facts["known_public_world_facts"], world);
    let none = privacy_facts(&[], "fears", &Value::Null);
    assert!(none.get("known_public_world_facts").is_none());
    assert!(PRIVACY_INSTRUCTION.contains("stating one is never a leak"));
    assert!(LEAK_REPORT_INSTRUCTION.contains("is never a leak"));
}

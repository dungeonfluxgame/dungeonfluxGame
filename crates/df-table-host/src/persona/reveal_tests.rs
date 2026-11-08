//! The reveal judge's answer: one verdict per secret asked about, and the ones let out too early.

use serde_json::json;

use super::reveal::read_verdicts;

#[test]
fn the_secrets_whose_conditions_do_not_hold_are_the_early_ones() {
    let answer = json!({"verdicts": [
        {"id": "lantern_report", "holds": false, "reason": "Nobody asked why he warns them."},
        {"id": "old_debt", "holds": true, "reason": "His friend asked about the debt."}
    ]});
    let early = read_verdicts(&answer, &["lantern_report", "old_debt"]).unwrap();
    assert_eq!(
        early,
        vec![(
            "lantern_report".to_owned(),
            "Nobody asked why he warns them.".to_owned()
        )]
    );
}

#[test]
fn a_verdict_missing_or_about_another_secret_is_refused() {
    let missing = json!({"verdicts": [{"id": "old_debt", "holds": true, "reason": "Asked."}]});
    assert!(read_verdicts(&missing, &["lantern_report", "old_debt"]).is_err());
    let stranger = json!({"verdicts": [{"id": "ghost", "holds": true, "reason": "?"}]});
    assert!(read_verdicts(&stranger, &["ghost_free"]).is_err());
}

#[test]
fn a_verdict_without_a_reason_is_refused() {
    let silent = json!({"verdicts": [{"id": "old_debt", "holds": false, "reason": " "}]});
    assert!(read_verdicts(&silent, &["old_debt"]).is_err());
}

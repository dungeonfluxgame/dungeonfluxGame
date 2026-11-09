//! The reveal judge's answer: one verdict per secret asked about, and the ones let out too early;
//! and the line route lets out a secret the story's play made revealable, and only that one.

use df_content::persona::Persona;
use serde_json::{Value, json};

use super::fixtures::{CHARACTER, ScriptedHost, line_answer, scripted_host};
use super::request::{PersonaReply, Spoken};
use super::reveal::read_verdicts;
use super::routes::path;
use super::storage::write_persona;
use super::story::{Story, story_path, write_story};
use crate::dev_client::post_json;
use crate::dev_http::DevReply;

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

/// A persona for [`CHARACTER`] holding two secrets: the forged charter, which the stored story's
/// play has made revealable, and an old debt, which it has not.
fn holder() -> Persona {
    serde_json::from_value(json!({
        "schema": 1, "id": CHARACTER, "display_name": "Marta Venn", "job": "Keeper of the inn",
        "secrets": [
            {"id": "forged_charter", "text": "She forged the charter.",
             "reveal_conditions": ["confronted with proof of the forged charter"]},
            {"id": "old_debt", "text": "She owes the guild.",
             "reveal_conditions": ["shown the guild's ledger"]}
        ]
    }))
    .unwrap()
}

/// A host whose stored story was struck at the vault, where play made the forged charter
/// revealable, answering with `answers`.
async fn struck_host(answers: Vec<Value>) -> ScriptedHost {
    let host = scripted_host(answers).await;
    write_persona(&host.directory(), &holder()).unwrap();
    let story: Story = serde_json::from_value(json!({
        "title": "The Vault", "hook": "h", "truth": "t", "culprit": CHARACTER, "motive": "m",
        "scenes": [], "characters": [], "evidence": [], "beats": [], "struck": ["vault"],
        "revealable": {"place": "vault", "findings": 2,
                       "secrets": [{"holder": CHARACTER, "secret": "forged_charter"}]}
    }))
    .unwrap();
    host.running.write_asset("persona/story.json", "{}");
    write_story(&story_path(&host.running.state), &story).unwrap();
    host
}

async fn say(host: &ScriptedHost) -> PersonaReply<Spoken> {
    let body = json!({"character": CHARACTER, "situation": "The vault."});
    post_json(host.port, path::PERSONA_SAY, &body)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_secret_play_made_revealable_is_let_out_without_asking_the_judge_again() {
    let mut confession = line_answer("Aye, I forged it. What of it?");
    confession["revealed_secrets"] = json!(["forged_charter"]);
    let host = struck_host(vec![confession]).await;
    let DevReply::Done { value } = say(&host).await else {
        panic!("the confession is refused");
    };
    assert_eq!(value.said.revealed_secrets, vec!["forged_charter"]);
    assert_eq!(host.calls(), 1, "no reveal judge is asked");
    assert_eq!(
        host.facts_of(0)["now"]["revealable"],
        json!(["forged_charter"])
    );
}

#[tokio::test]
async fn a_secret_whose_conditions_do_not_hold_is_still_kept_back() {
    let mut early = line_answer("I owe the guild, if you must know.");
    early["revealed_secrets"] = json!(["old_debt"]);
    let not_yet = json!({"verdicts": [
        {"id": "old_debt", "holds": false, "reason": "No ledger was shown."}
    ]});
    let host = struck_host(vec![early.clone(), not_yet.clone(), early, not_yet]).await;
    assert!(
        matches!(say(&host).await, DevReply::Rejected { .. }),
        "a line still letting the debt out is refused"
    );
    assert_eq!(
        host.facts_of(1)["secrets"][0]["id"],
        "old_debt",
        "the reveal judge weighs the secret play has not made revealable"
    );
}

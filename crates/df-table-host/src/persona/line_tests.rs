//! The line pipeline over a scripted backend: one call per line that also reports the gaps and the
//! appraisal, the checks of its ids, the state the next line carries, fired triggers taking their
//! behavior, the one repeat retry, the backfill after the line, the quote held inside the stall,
//! the price slots filled with the totals held, the world as each speaker sees it and the compact
//! facts.
//!
//! TODO(DEV-12): a concurrent ask of one persona while its backfill runs (the `Busy` outcome), and
//! the tone cued on the scene once the cue carries one.
//! TODO(DEV-12): the inventory and deal judges overlapping in time on a slow text service, a gain
//! found at an offered thing (`where` closed to the place's ids), and a deal still refused at the
//! text deadline.

use std::time::Duration;

use df_content::persona::Persona;
use serde_json::{Value, json};

use super::fixtures::{
    CHARACTER, ScriptedHost, append_backstory_answer, innkeeper, line_answer,
    line_answer_with_gaps, privacy_answer, scripted_host, scripted_host_cast, write_market,
};
use super::request::{Answered, BackfillOutcome, BackfillRecord, PersonaReply, SaidLine, Spoken};
use super::routes::path;
use super::storage::write_persona;
use crate::dev_client::{get_json, post_json};
use crate::dev_http::DevReply;
use crate::speech::fixtures::FakePort;
use std::sync::Arc;

fn done<T: std::fmt::Debug>(reply: PersonaReply<T>) -> T {
    match reply {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, message } => panic!("refused: {error:?} {message}"),
    }
}

/// A shopkeeper with two behaviors, a trigger of the old kind, and one emotional trigger of each
/// kind: by event, and by state.
fn vendor(id: &str) -> Persona {
    serde_json::from_value(json!({
        "schema": 1, "id": id, "display_name": "Vendor", "job": "Keeper of a stall",
        "personality": ["brisk"],
        "backstory": "Sells fruit.",
        "behaviors": [
            {"id": "warm", "description": "Chatty.", "speech_style": ""},
            {"id": "curt", "description": "Short answers.", "speech_style": ""}
        ],
        "default_behavior": "warm",
        "triggers": [{"id": "haggled", "condition": "the buyer haggles", "behavior": "curt"}],
        "emotional_triggers": [
            {"id": "fed-up", "when_event": "the buyer insults the keeper",
             "sets": [{"emotion": "irritation", "delta": 40}], "behavior": "curt"},
            {"id": "scared", "when_state": {"emotion": "fear", "at_least": 50},
             "behavior": "curt"}
        ]
    }))
    .unwrap()
}

async fn ask(host: &ScriptedHost, character: &str, extra: Value) -> PersonaReply<Answered> {
    let mut body = json!({"character": character, "question": "How much for the apples?"});
    for (key, value) in extra.as_object().into_iter().flatten() {
        body[key] = value.clone();
    }
    post_json(host.port, path::PERSONA_ASK, &body)
        .await
        .unwrap()
}

async fn say(host: &ScriptedHost, character: &str, extra: Value) -> PersonaReply<Spoken> {
    let mut body = json!({"character": character, "situation": "A buyer waits at the stall."});
    for (key, value) in extra.as_object().into_iter().flatten() {
        body[key] = value.clone();
    }
    post_json(host.port, path::PERSONA_SAY, &body)
        .await
        .unwrap()
}

/// The first audit line that starts with `start`, waiting for a task that runs after its request.
async fn wait_for_line(host: &ScriptedHost, start: &str) -> String {
    for _ in 0..100 {
        if let Some(line) = host.lines_starting_with(start).into_iter().next() {
            return line;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no {start} line: {:?}", host.persona_lines());
}

fn appraised(mut answer: Value, appraisal: Value) -> Value {
    answer["appraisal"] = appraisal;
    answer
}

fn no_attention() -> Value {
    json!({"focus": null, "topic": null, "noticed": []})
}

#[tokio::test]
async fn a_name_heard_in_the_line_is_learned_and_one_not_present_is_dropped() {
    let mut answer = line_answer("Pip, is it? Welcome.");
    // Pip is a stranger, shown as a handle; the handle reads back to the id.
    answer["names_learned"] = json!(["person-1", "ghost"]);
    let host = scripted_host(vec![answer]).await;
    write_persona(&host.directory(), &vendor(CHARACTER)).unwrap();
    let first = done(
        ask(
            &host,
            CHARACTER,
            json!({"asked_by": "pip", "backfill": false}),
        )
        .await,
    );
    let said = first.spoken.said;
    assert_eq!(said.names_learned, ["pip"]);
    assert!(said.dropped.contains(&"name:ghost".to_owned()));
    assert_eq!(said.state.known_names, ["pip"]);
}

#[tokio::test]
async fn one_call_carries_the_gaps_and_an_appraisal_that_only_keeps_what_is_valid() {
    let answer = appraised(
        line_answer_with_gaps("Eight a basket, love.", &[("backstory", "childhood")]),
        json!({
            "emotions": [{"emotion": "irritation", "delta": 30}, {"emotion": "bogus", "delta": 10}],
            "toward": "person-1", "rapport_delta": -5,
            "attention": {
                "focus": "person-1", "topic": "the price",
                "noticed": [
                    {"speaker": "person-1", "gist": "asked"},
                    {"speaker": "ghost", "gist": "x"}
                ]
            },
            "tone": "danger"
        }),
    );
    let host = scripted_host(vec![answer]).await;
    write_persona(&host.directory(), &vendor(CHARACTER)).unwrap();
    let answered = done(
        ask(
            &host,
            CHARACTER,
            json!({"asked_by": "pip", "backfill": false}),
        )
        .await,
    );
    assert_eq!(host.calls(), 1, "no separate coverage call");
    assert!(!answered.covered);
    assert_eq!(answered.gaps.len(), 1);
    assert_eq!(answered.backfill, BackfillOutcome::NotAsked);
    let said = answered.spoken.said;
    assert_eq!(said.dropped, ["emotion:bogus", "speaker:ghost"]);
    assert_eq!(said.appraisal.emotions.len(), 1);
    assert_eq!(said.appraisal.toward.as_deref(), Some("pip"));
    assert_eq!(said.appraisal.attention.noticed.len(), 1);
    assert_eq!(said.tone.as_deref(), Some("danger"));
    assert_eq!(said.state.level("irritation"), 30);
    assert_eq!(
        said.state
            .relationship("pip")
            .map(|standing| standing.rapport),
        Some(-5)
    );
    let schema_text = host.asked.lock().unwrap()[0].output_schema.to_string();
    assert!(
        schema_text.contains("irritation"),
        "the emotion ids are the schema's: {schema_text}"
    );
}

#[tokio::test]
async fn an_appraisal_naming_a_speaker_who_is_not_present_or_a_tone_not_offered_is_dropped() {
    let answer = appraised(
        line_answer("Aye."),
        json!({"emotions": [], "toward": "ghost", "rapport_delta": 20, "attention": no_attention(),
               "tone": "nonsense"}),
    );
    let host = scripted_host(vec![answer]).await;
    write_persona(&host.directory(), &vendor(CHARACTER)).unwrap();
    let said = done(say(&host, CHARACTER, json!({"asked_by": "pip"})).await).said;
    assert_eq!(said.dropped, ["toward:ghost", "tone:nonsense"]);
    assert_eq!(said.appraisal.toward, None);
    assert_eq!(said.appraisal.rapport_delta, None);
    assert_eq!(said.tone, None);
}

#[tokio::test]
async fn the_appraisal_changes_the_state_the_next_line_carries() {
    let first = appraised(
        line_answer("Eight, and not a copper less."),
        json!({"emotions": [{"emotion": "irritation", "delta": 60}], "toward": null,
               "rapport_delta": null, "attention": {"focus": "person-1", "topic": "haggling",
               "noticed": []}, "tone": null}),
    );
    let host = scripted_host(vec![first, line_answer("Next.")]).await;
    write_persona(&host.directory(), &vendor(CHARACTER)).unwrap();
    let said = done(say(&host, CHARACTER, json!({"asked_by": "pip"})).await).said;
    assert_eq!(said.state.level("irritation"), 60);
    assert_eq!(
        said.tone.as_deref(),
        Some("danger"),
        "the strongest emotion's tone"
    );
    done(say(&host, CHARACTER, json!({"asked_by": "pip"})).await);
    let state = &host.facts_of(1)["now"]["state"];
    assert!(
        state["emotions"]
            .as_array()
            .unwrap()
            .contains(&json!({"emotion": "irritation", "level": 60})),
        "{state}"
    );
    assert_eq!(
        state["attention"]["focus"], "person-1",
        "the state names the stranger by handle"
    );
    assert!(!host.facts_of(1).to_string().contains("\"pip\""));
    let instructions = host.instructions_of(1);
    assert!(instructions.contains("never the content"), "{instructions}");
    assert!(
        instructions.contains("state.attention.focus first"),
        "{instructions}"
    );
}

#[tokio::test]
async fn a_fired_trigger_makes_its_behavior_the_lines_even_when_the_model_named_another() {
    let mut old_kind = line_answer("Seven. Take it or leave it.");
    old_kind["fired_triggers"] = json!(["haggled"]);
    old_kind["behavior"] = json!("warm");
    let mut by_event = line_answer("Out.");
    by_event["fired_emotional_triggers"] = json!(["fed-up"]);
    let host = scripted_host(vec![old_kind, by_event]).await;
    write_persona(&host.directory(), &vendor(CHARACTER)).unwrap();
    let said = done(say(&host, CHARACTER, json!({})).await).said;
    assert_eq!(said.behavior.as_deref(), Some("curt"));
    let over = said.behavior_override.expect("the mismatch is recorded");
    assert_eq!(
        (
            over.model.as_deref(),
            over.trigger.as_str(),
            over.took.as_str()
        ),
        (Some("warm"), "haggled", "curt")
    );
    let said = done(say(&host, CHARACTER, json!({})).await).said;
    assert_eq!(said.behavior.as_deref(), Some("curt"));
    assert_eq!(said.fired_emotional_triggers, ["fed-up"]);
    assert_eq!(
        said.state.level("irritation"),
        40,
        "the trigger's change joined the appraisal"
    );
}

#[tokio::test]
async fn an_unknown_behavior_or_trigger_id_is_dropped_and_the_line_is_still_said() {
    let mut answer = line_answer("Seven.");
    answer["behavior"] = json!("null");
    answer["fired_triggers"] = json!(["no-such-trigger", "haggled"]);
    answer["fired_emotional_triggers"] = json!(["no-such-feeling"]);
    let host = scripted_host(vec![answer]).await;
    write_persona(&host.directory(), &vendor(CHARACTER)).unwrap();
    let said = done(say(&host, CHARACTER, json!({})).await).said;
    assert_eq!(said.line, "Seven.");
    assert_eq!(said.fired_triggers, ["haggled"]);
    assert_eq!(
        said.dropped,
        [
            "behavior:null",
            "trigger:no-such-trigger",
            "trigger:no-such-feeling"
        ]
    );
}

#[tokio::test]
async fn a_trigger_whose_behavior_the_model_already_took_is_no_override() {
    let mut answer = line_answer("Seven.");
    answer["fired_triggers"] = json!(["haggled"]);
    answer["behavior"] = json!("curt");
    let host = scripted_host(vec![answer]).await;
    write_persona(&host.directory(), &vendor(CHARACTER)).unwrap();
    let said = done(say(&host, CHARACTER, json!({})).await).said;
    assert_eq!(said.behavior.as_deref(), Some("curt"));
    assert_eq!(said.behavior_override, None);
}

#[tokio::test]
async fn a_trigger_of_the_state_is_checked_by_code_passed_in_the_facts_and_fires() {
    let frightened = appraised(
        line_answer("Please."),
        json!({"emotions": [{"emotion": "fear", "delta": 70}], "toward": null,
               "rapport_delta": null, "attention": no_attention(), "tone": null}),
    );
    let host = scripted_host(vec![frightened, line_answer("Go away.")]).await;
    write_persona(&host.directory(), &vendor(CHARACTER)).unwrap();
    let first = done(say(&host, CHARACTER, json!({})).await).said;
    assert!(first.state_triggered.is_empty());
    let second = done(say(&host, CHARACTER, json!({})).await).said;
    assert_eq!(
        host.facts_of(1)["now"]["state_triggered"],
        json!([{"id": "scared", "behavior": "curt"}])
    );
    assert_eq!(second.state_triggered, ["scared"]);
    assert_eq!(second.fired_emotional_triggers, ["scared"]);
    assert_eq!(second.behavior.as_deref(), Some("curt"));
    assert_eq!(
        second.behavior_override.map(|over| over.trigger),
        Some("scared".to_owned())
    );
}

#[tokio::test]
async fn a_state_trigger_adds_its_changes_once_when_it_starts_to_hold_not_on_every_line() {
    let mut persona = serde_json::to_value(vendor(CHARACTER)).unwrap();
    persona["emotional_triggers"][1]["sets"] = json!([{"emotion": "irritation", "delta": 12}]);
    let persona: Persona = serde_json::from_value(persona).unwrap();
    let frightened = appraised(
        line_answer("Please."),
        json!({"emotions": [{"emotion": "fear", "delta": 70}], "toward": null,
               "rapport_delta": null, "attention": no_attention(), "tone": null}),
    );
    let host = scripted_host(vec![frightened, line_answer("Go away.")]).await;
    write_persona(&host.directory(), &persona).unwrap();
    let irritation = |said: &SaidLine| {
        said.appraisal
            .emotions
            .iter()
            .filter(|change| change.emotion == "irritation")
            .count()
    };
    let crossing = done(say(&host, CHARACTER, json!({})).await).said;
    assert_eq!(irritation(&crossing), 1, "fear crossed 50 on this line");
    let holding = done(say(&host, CHARACTER, json!({})).await).said;
    assert_eq!(holding.state_triggered, ["scared"]);
    assert_eq!(
        irritation(&holding),
        0,
        "still afraid: the stance holds, no new change"
    );
}

#[tokio::test]
async fn a_line_the_model_says_repeats_recent_is_worded_once_more_with_the_reason() {
    let mut repeating = line_answer("Fresh today, love.");
    repeating["repeats_recent"] = json!(true);
    repeating["repeat_reason"] = json!("opens with the same words");
    let host = scripted_host(vec![repeating, line_answer("Ripe, and cheap.")]).await;
    write_persona(&host.directory(), &vendor(CHARACTER)).unwrap();
    let said = done(say(&host, CHARACTER, json!({"recent": ["Fresh today, love."]})).await).said;
    assert_eq!(host.calls(), 2);
    assert_eq!(said.line, "Ripe, and cheap.");
    assert!(said.repeat_retried && !said.repeats_recent);
    assert_eq!(
        host.facts_of(1)["now"]["retry_reason"],
        "opens with the same words"
    );
    assert!(host.facts_of(0)["now"].get("retry_reason").is_none());
}

#[tokio::test]
async fn a_line_still_repeating_after_the_retry_is_kept_reported_and_never_asked_again() {
    let repeating = |line: &str| {
        let mut answer = line_answer(line);
        answer["repeats_recent"] = json!(true);
        answer["repeat_reason"] = json!("same shape");
        answer
    };
    let host = scripted_host(vec![
        repeating("One."),
        repeating("Two."),
        line_answer("Three."),
    ])
    .await;
    write_persona(&host.directory(), &vendor(CHARACTER)).unwrap();
    let said = done(say(&host, CHARACTER, json!({})).await).said;
    assert_eq!(host.calls(), 2, "one retry and no more");
    assert_eq!(said.line, "Two.");
    assert!(said.repeat_retried && said.repeats_recent);
}

/// What the innkeeper holds in the run 9 line: the study to sell, the coins, the boards she has
/// and a pigment she has none of yet.
fn cottage_study_hands(host: &ScriptedHost) {
    write_persona(&host.directory(), &innkeeper()).unwrap();
    host.running.write_asset(
        "world/inventory/innkeeper.json",
        &json!({"schema": 1, "owner": CHARACTER, "kind": "character", "items": [
            {"id": "painting_06", "name": "cottage window study", "look": "A shuttered window.",
             "use": "kept", "count": 1},
            {"id": "coins", "name": "silver pennies", "look": "Small silver coins.",
             "use": "consumed", "count": 14},
            {"id": "boards", "name": "boards", "look": "Fresh pine boards.", "use": "consumed",
             "count": 1},
            {"id": "verdigris", "name": "verdigris", "look": "Blue-green pigment.",
             "use": "consumed", "count": 0}
        ]})
        .to_string(),
    );
}

/// One entry of `changes` as the inventory judge fills it.
fn entry(
    item: &str,
    change: i64,
    reason: &str,
    source: Option<&str>,
    paid: &[(&str, u64)],
) -> Value {
    json!({
        "item": item, "change": change, "reason": reason, "why": "the deal",
        "name": null, "look": null, "use": null, "source": source,
        "paid_with": paid.iter().map(|(item, count)| json!({"item": item, "count": count}))
            .collect::<Vec<_>>(),
        "made_from": [], "where": null, "from": null, "what_happened": null, "to": null
    })
}

/// The inventory judge's answer listing `changes`, or saying outright that nothing changed.
fn judged(changes: Vec<Value>) -> Value {
    json!({"nothing_changed": changes.is_empty(), "changes": changes})
}

/// The same deal with the three coins split between the two goods, as the inventory judge might
/// list it.
fn cottage_study_split() -> Value {
    judged(vec![
        entry("painting_06", -1, "sold", None, &[]),
        entry(
            "coins",
            3,
            "gained",
            Some("exchange"),
            &[("painting_06", 1)],
        ),
        entry("coins", -3, "used_up", None, &[]),
        entry("verdigris", 1, "gained", Some("exchange"), &[("coins", 2)]),
        entry("boards", 2, "gained", Some("exchange"), &[("coins", 1)]),
    ])
}

#[tokio::test]
async fn a_sale_and_purchase_one_line_lists_moves_nothing_without_a_deal() {
    // The judge's own word on a deal (run 9: the painting, the coins, the pigment and the
    // boards) never moves goods between owners: only a deal both sides agree to does.
    let host = scripted_host(vec![
        line_answer("Three pennies for the study; the pigment and two boards."),
        cottage_study_split(),
    ])
    .await;
    cottage_study_hands(&host);
    let said = done(say(&host, CHARACTER, json!({})).await).said;
    assert_eq!(
        host.calls(),
        2,
        "nothing to ask again: nothing of it is taken"
    );
    assert!(said.inventory.is_empty(), "{:?}", said.inventory);
    assert_eq!(
        host.lines_starting_with("PERSONA deal_entries"),
        [concat!(
            "PERSONA deal_entries id=innkeeper deal=- moved_by_deal=painting_06:sold|",
            "coins:gained|coins:used_up|verdigris:gained|boards:gained"
        )]
    );
}

/// What the painter held in run 9 move 16: ochre and lamp black, and none of the rest.
fn painters_shelf(host: &ScriptedHost) {
    write_persona(&host.directory(), &innkeeper()).unwrap();
    let item = |id: &str, count: u64| {
        json!({"id": id, "name": id, "look": format!("{id} up close"), "use": "consumed",
               "count": count})
    };
    host.running.write_asset(
        "world/inventory/innkeeper.json",
        &json!({"schema": 1, "owner": CHARACTER, "kind": "character", "items": [
            item("boards", 0), item("ochre", 1), item("lamp_black", 2),
            item("linseed_oil", 0), item("rags", 0)
        ]})
        .to_string(),
    );
}

/// The inventory judge's answer: one of each of `items` used up.
fn uses(items: &[&str]) -> Value {
    judged(
        items
            .iter()
            .map(|item| entry(item, -1, "used_up", None, &[]))
            .collect(),
    )
}

#[tokio::test]
async fn a_painted_line_using_what_is_not_held_changes_nothing_and_is_judged_again() {
    let host = scripted_host(vec![
        line_answer("It's finished. I used a board, ochre, lamp black, a little oil, and a rag."),
        uses(&["boards", "ochre", "lamp_black", "linseed_oil", "rags"]),
        uses(&["ochre", "lamp_black"]),
    ])
    .await;
    painters_shelf(&host);
    let said = done(
        say(
            &host,
            CHARACTER,
            json!({"intent": "say the study is finished"}),
        )
        .await,
    )
    .said;
    assert_eq!(host.calls(), 3);
    let reason = host.facts_of(2)["now"]["retry_reason"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        reason.contains("insufficient boards (holds 0, asked 1)"),
        "{reason}"
    );
    assert!(reason.contains("insufficient rags") && reason.contains("insufficient linseed_oil"));
    let facts = host.facts_of(0);
    let boards = facts["now"]["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "boards")
        .unwrap()
        .clone();
    assert_eq!(
        boards["count"], 0,
        "the facts show the real count: {boards}"
    );
    let moved: Vec<(&str, u64, u64)> = said
        .inventory
        .iter()
        .map(|applied| (applied.item.as_str(), applied.before, applied.after))
        .collect();
    assert_eq!(moved, [("ochre", 1, 0), ("lamp_black", 2, 1)]);
}

#[tokio::test]
async fn the_backfill_runs_after_the_line_and_is_recorded_when_done() {
    let host = scripted_host(vec![
        line_answer_with_gaps("Above the inn, love.", &[("backstory", "childhood")]),
        append_backstory_answer("She learned to brew ale at nine."),
        privacy_answer(false),
    ])
    .await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    let answered = done(ask(&host, CHARACTER, json!({})).await);
    assert_eq!(answered.backfill, BackfillOutcome::Running { record: 1 });
    assert_eq!(answered.spoken.said.line, "Above the inn, love.");
    let finished = wait_for_line(&host, "PERSONA backfill_done").await;
    assert!(
        finished.starts_with("PERSONA backfill_done id=innkeeper record=1 edits=1"),
        "{finished}"
    );
    let lines = host.persona_lines();
    let position = |start: &str| {
        lines
            .iter()
            .position(|line| line.starts_with(start))
            .unwrap()
    };
    assert!(
        position("PERSONA said") < position("PERSONA backfilled"),
        "{lines:?}"
    );
    let records: PersonaReply<Vec<BackfillRecord>> =
        get_json(host.port, "/dev/persona/backfills?id=innkeeper")
            .await
            .unwrap();
    assert_eq!(done(records).len(), 1);
    assert_eq!(
        host.calls(),
        3,
        "the line, the backfill and the privacy judge"
    );
}

#[tokio::test]
async fn backfill_sync_waits_for_the_backfill_before_answering() {
    let host = scripted_host(vec![
        line_answer_with_gaps("Above the inn, love.", &[("backstory", "childhood")]),
        append_backstory_answer("She learned to brew ale at nine."),
        privacy_answer(false),
    ])
    .await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    let answered = done(ask(&host, CHARACTER, json!({"backfill_sync": true})).await);
    assert_eq!(
        answered.backfill,
        BackfillOutcome::Backfilled {
            edits: 1,
            sequence: 1
        }
    );
    assert_eq!(host.calls(), 3);
}

#[tokio::test]
async fn a_quote_is_held_inside_the_stall_and_both_numbers_are_reported() {
    let quote = |total: u64| {
        let mut answer = line_answer("Here you are: {price:0}.");
        answer["quotes"] = json!([{"item": "apple", "qty": 5, "total_cp": total}]);
        answer
    };
    let host = scripted_host_cast(
        &["fruit_vendor"],
        // A total the stall would not charge is worded once more with the stall's total; a second
        // answer is kept whatever it says, and held inside the bounds.
        vec![quote(30), quote(43), quote(100), quote(100), quote(44)],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    write_persona(&host.directory(), &vendor("fruit_vendor")).unwrap();
    write_market(&host);
    let quoted = |said: &super::request::SaidLine| {
        let quote = said.quotes[0].clone();
        (
            quote.proposed_total_cp,
            quote
                .bounds
                .map(|bounds| u64::from(bounds.clamped_total_cp)),
            quote.bounds.map(|bounds| bounds.floor_total_cp),
        )
    };
    let low = done(say(&host, "fruit_vendor", json!({})).await).said;
    assert_eq!(
        quoted(&low),
        (43, Some(43), Some(43)),
        "worded again at the stall's total"
    );
    assert!(
        host.facts_of(1)
            .to_string()
            .contains("the stall's total for that lot is 43 cp"),
        "the retry names the stall's total"
    );
    let high = done(say(&host, "fruit_vendor", json!({})).await).said;
    assert_eq!(
        quoted(&high),
        (100, Some(45), Some(43)),
        "still too high after the retry: lowered to the ask"
    );
    assert_eq!(high.quotes[0].bounds.unwrap().ask_total_cp, 45);
    assert_eq!(
        high.line, "Here you are: 45 cp.",
        "the slot says the total held"
    );
    let inside = done(say(&host, "fruit_vendor", json!({})).await).said;
    assert_eq!(quoted(&inside), (44, Some(44), Some(43)));
}

#[tokio::test]
async fn a_quote_by_someone_who_keeps_no_such_stall_is_reported_unchecked() {
    let mut answer = line_answer("Five for {price:0}.");
    answer["quotes"] = json!([{"item": "apple", "qty": 5, "total_cp": 40}]);
    let host = scripted_host_cast(&["pip"], vec![answer], Arc::new(FakePort::unconfigured())).await;
    write_persona(&host.directory(), &vendor("pip")).unwrap();
    write_market(&host);
    let said = done(say(&host, "pip", json!({})).await).said;
    assert_eq!(said.quotes[0].proposed_total_cp, 40);
    assert_eq!(said.quotes[0].bounds, None);
}

#[tokio::test]
async fn every_price_a_line_names_is_checked_and_one_wrong_total_is_worded_again() {
    let prices = |five: u64| {
        let mut answer = line_answer("Five apples for {price:0} and one more for {price:1}.");
        answer["quotes"] = json!([
            {"item": "apple", "qty": 5, "total_cp": five},
            {"item": "apple", "qty": 1, "total_cp": 10}
        ]);
        answer
    };
    let host = scripted_host_cast(
        &["fruit_vendor"],
        vec![prices(30), prices(43)],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    write_persona(&host.directory(), &vendor("fruit_vendor")).unwrap();
    write_market(&host);
    let said = done(say(&host, "fruit_vendor", json!({})).await).said;
    assert!(said.repeat_retried, "{:?}", said.retry_reason);
    let reason = said.retry_reason.unwrap();
    assert!(reason.contains("you said 30 cp for 5 apple"), "{reason}");
    assert!(!reason.contains("1 apple"), "{reason}");
    assert_eq!(said.quotes.len(), 2);
    assert!(said.quotes.iter().all(|quote| quote.bounds.is_some()));
    assert_eq!(said.quotes[0].proposed_total_cp, 43);
}

#[tokio::test]
async fn an_order_total_is_the_sum_held_and_a_price_said_without_its_slot_is_worded_again() {
    let order = |line: &str| {
        let mut answer = line_answer(line);
        answer["quotes"] = json!([
            {"item": "apple", "qty": 5, "total_cp": 44},
            {"item": "apple", "qty": 1, "total_cp": 10}
        ]);
        answer
    };
    let host = scripted_host_cast(
        &["fruit_vendor"],
        vec![
            order("Both for fifty-four."),
            order("Both come to {order_total}."),
            order("That is {price:7}."),
            order("A {bargain} for {order_total}."),
            order("Both come to {order_total}."),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    write_persona(&host.directory(), &vendor("fruit_vendor")).unwrap();
    write_market(&host);
    let said = done(say(&host, "fruit_vendor", json!({})).await).said;
    assert!(said.repeat_retried, "a number written out is worded again");
    assert_eq!(said.line, "Both come to 54 cp.");
    // A slot for a lot not quoted, then one that is no slot: a line with a brace in it is never
    // shown, so it is worded again once more for its slots.
    let kept = done(say(&host, "fruit_vendor", json!({})).await).said;
    let reason = kept.retry_reason.unwrap();
    assert!(reason.contains("{bargain} is no price slot"), "{reason}");
    assert_eq!(kept.line, "Both come to 54 cp.");
    assert_eq!(
        host.lines_starting_with("PERSONA price_slots_unfilled"),
        ["PERSONA price_slots_unfilled id=fruit_vendor reworded=yes"]
    );
}

#[tokio::test]
async fn each_speaker_is_given_the_world_as_they_see_it() {
    let host = scripted_host_cast(
        &["fruit_vendor", "pip"],
        vec![line_answer("Five apples, please."), line_answer("Eight.")],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    write_persona(&host.directory(), &vendor("fruit_vendor")).unwrap();
    write_persona(&host.directory(), &vendor("pip")).unwrap();
    write_market(&host);
    done(say(&host, "pip", json!({})).await);
    done(say(&host, "fruit_vendor", json!({})).await);
    let customer = host.facts_of(0)["context"]["world"].clone();
    let keeper = host.facts_of(1)["context"]["world"].clone();
    assert_eq!(customer["viewer_role"], "customer");
    assert_eq!(keeper["viewer_role"], "keeper");
    assert!(
        customer["stalls"][0].get("haggle_floor_permille").is_none(),
        "{customer}"
    );
    assert!(
        customer["stalls"][0]["items"][0]["quotes"][0]
            .get("floor_total_cp")
            .is_none()
    );
    assert!(
        keeper["stalls"][0].get("haggle_floor_permille").is_some(),
        "{keeper}"
    );
    assert!(
        keeper["stalls"][0]["items"][0]["quotes"][0]
            .get("floor_total_cp")
            .is_some()
    );
    let instructions = host.instructions_of(0);
    assert!(
        instructions.contains("only the keeper of a stall quotes one"),
        "{instructions}"
    );
}

fn has_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty() || items.iter().any(has_empty),
        Value::Object(fields) => fields.is_empty() || fields.values().any(has_empty),
        Value::Bool(_) | Value::Number(_) => false,
    }
}

#[tokio::test]
async fn the_facts_are_compact_with_the_stable_part_first_and_the_changing_part_last() {
    let host = scripted_host(vec![line_answer("Aye.")]).await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    done(
        say(
            &host,
            CHARACTER,
            json!({"recent": ["Pip: Hello."], "asked_by": "pip"}),
        )
        .await,
    );
    let facts = host.facts_of(0);
    let keys: Vec<&String> = facts.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["context", "now"], "the stable part sorts first");
    assert!(
        !has_empty(&facts),
        "no null, empty text, list or object: {facts}"
    );
    let context = facts["context"].as_object().unwrap();
    for stable in ["character", "voice_layers", "public_note"] {
        assert!(context.contains_key(stable), "{stable}");
    }
    assert!(
        !context.contains_key("scene"),
        "a fixture's words may name people"
    );
    for changing in ["recent", "asked_by", "situation", "state"] {
        assert!(facts["now"].get(changing).is_some(), "{changing}");
        assert!(!context.contains_key(changing), "{changing}");
    }
    assert!(facts.to_string().starts_with("{\"context\""));
}

#[tokio::test]
async fn the_line_is_audited_with_its_timings_and_no_text() {
    let host = scripted_host(vec![line_answer("Aye, Marta Venn.")]).await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    done(say(&host, CHARACTER, json!({})).await);
    let line = host.lines_starting_with("PERSONA said").remove(0);
    for expected in [" model_ms=", " total_ms=", " retried=false", " gaps=0"] {
        assert!(line.contains(expected), "{expected}: {line}");
    }
    assert!(!line.contains("Marta"), "{line}");
}

/// Wren's kit at the first painting of loop 4: boards, ochre, umber, linseed oil and rags.
fn wrens_kit(host: &ScriptedHost) {
    write_persona(&host.directory(), &innkeeper()).unwrap();
    let item = |id: &str, count: u64| {
        json!({"id": id, "name": id, "look": format!("{id} up close"), "use": "consumed",
               "count": count})
    };
    host.running.write_asset(
        "world/inventory/innkeeper.json",
        &json!({"schema": 1, "owner": CHARACTER, "kind": "character", "items": [
            item("boards", 12), item("ochre", 6), item("umber", 6),
            item("linseed_oil", 12), item("rags", 12)
        ]})
        .to_string(),
    );
}

/// The finishing line of loop 4 as the inventory judge reads it: the five supplies used up and
/// the painting made from them.
fn finished_painting() -> Value {
    let supplies = ["boards", "ochre", "umber", "linseed_oil", "rags"];
    let mut answer = uses(&supplies);
    let changes = answer["changes"].as_array_mut().unwrap();
    changes.push(json!({
        "item": "painting_01", "change": 1, "reason": "gained", "why": "the study is done",
        "name": "Weathered gate study", "look": "a small oil study of a wooden gate",
        "use": "kept", "source": "made", "paid_with": [],
        "made_from": supplies.iter().map(|item| json!({"item": item, "count": 1}))
            .collect::<Vec<_>>(),
        "where": null, "from": null, "what_happened": null, "to": null
    }));
    answer
}

/// What the painted line moved, each item with its count before and after.
fn moved_counts(said: &SaidLine) -> Vec<(&str, u64, u64)> {
    said.inventory
        .iter()
        .map(|applied| (applied.item.as_str(), applied.before, applied.after))
        .collect()
}

const PAINTED: [(&str, u64, u64); 6] = [
    ("boards", 12, 11),
    ("ochre", 6, 5),
    ("umber", 6, 5),
    ("linseed_oil", 12, 11),
    ("rags", 12, 11),
    ("painting_01", 0, 1),
];

#[tokio::test]
async fn a_line_expected_to_use_up_supplies_is_judged_for_them_by_a_call_of_its_own() {
    // Loop 8: the trough's finishing line came back from the line call with no changes. A call of
    // its own, which must say outright when nothing changed, judges what the words move.
    let given = "That's the copper-spouted stone trough done. I've used up the board, a little \
                 ochre and umber, some oil and a rag.";
    let host = scripted_host(vec![line_answer(given), finished_painting()]).await;
    wrens_kit(&host);
    let said = done(
        say(
            &host,
            CHARACTER,
            json!({"text": given, "expect": "inventory"}),
        )
        .await,
    )
    .said;
    assert_eq!(host.calls(), 2);
    let schema = host.asked.lock().unwrap()[1].output_schema.clone();
    assert!(
        schema["required"] == json!(["nothing_changed", "changes"]),
        "{schema}"
    );
    let reasons = &schema["properties"]["changes"]["items"]["properties"]["reason"]["enum"];
    assert_eq!(
        *reasons,
        json!([
            "gained", "used_up", "worn_out", "broken", "lost", "left", null
        ]),
        "a trade is never the inventory judge's"
    );
    let facts = host.facts_of(1);
    assert_eq!(facts["now"]["line"], given, "{facts}");
    assert_eq!(facts["now"]["occasion"], "finishing", "{facts}");
    assert!(facts["context"]["inventory"].is_array(), "{facts}");
    assert_eq!(moved_counts(&said), PAINTED);
    let judged_line = host.lines_starting_with("PERSONA inventory_judged");
    assert!(
        judged_line[0].starts_with(concat!(
            "PERSONA inventory_judged id=innkeeper expected=yes occasion=finishing ",
            "entries=6 ms="
        )),
        "{judged_line:?}"
    );
}

#[tokio::test]
async fn a_line_said_already_is_judged_for_what_it_moves_and_never_cued_again() {
    // B1: the hero says the deed first, the roll comes after, and only then is what the deed
    // moved judged (`expect=said_inventory`): finished work, with no second cue or voice.
    let given = "That's the copper-spouted stone trough done. I've used up the board, a little \
                 ochre and umber, some oil and a rag.";
    let host = scripted_host(vec![line_answer(given), finished_painting()]).await;
    wrens_kit(&host);
    let spoken = done(
        say(
            &host,
            CHARACTER,
            json!({"text": given, "expect": "said_inventory", "speak": true}),
        )
        .await,
    );
    assert_eq!(host.facts_of(1)["now"]["occasion"], "finishing");
    assert_eq!(moved_counts(&spoken.said), PAINTED);
    assert_eq!(spoken.revision, 0, "nothing is cued");
    assert_eq!(spoken.voice, None, "nothing is voiced");
}

#[tokio::test]
async fn a_deed_said_before_its_roll_moves_nothing_yet() {
    // "I search the base of the gate and find a rope", said before the roll (`expect=attempt`):
    // the inventory judge is not asked, so nothing is found before the dice decide it.
    let given = "I search the base of the gate and find a coil of rope.";
    let host = scripted_host(vec![line_answer(given)]).await;
    wrens_kit(&host);
    let spoken = done(
        say(
            &host,
            CHARACTER,
            json!({"text": given, "expect": "attempt"}),
        )
        .await,
    );
    assert_eq!(host.calls(), 1, "only the line is asked for");
    assert!(
        spoken.said.inventory.is_empty(),
        "{:?}",
        spoken.said.inventory
    );
    assert!(
        host.lines_starting_with("PERSONA inventory_judged")
            .is_empty()
    );
}

#[tokio::test]
async fn an_inventory_judgment_that_lists_nothing_without_saying_so_is_asked_again() {
    let given = "That's the trough done.";
    let host = scripted_host(vec![
        line_answer(given),
        json!({"nothing_changed": false, "changes": []}),
        finished_painting(),
    ])
    .await;
    wrens_kit(&host);
    let said = done(
        say(
            &host,
            CHARACTER,
            json!({"text": given, "expect": "inventory"}),
        )
        .await,
    )
    .said;
    let reason = host.facts_of(2)["now"]["retry_reason"].clone();
    assert!(
        reason
            .as_str()
            .is_some_and(|reason| reason.contains("nothing_changed is false")),
        "{reason}"
    );
    assert_eq!(moved_counts(&said), PAINTED);
}

/// The answer of loop 9 move 3: nothing changed, and entries all the same.
fn contradictory() -> Value {
    json!({"nothing_changed": true, "changes": [entry("boards", -1, "used_up", None, &[])]})
}

#[tokio::test]
async fn a_malformed_judgment_is_one_attempt_and_never_refuses_the_line() {
    let declared = "I set a board on the easel and start laying in the mossy steps with my brush.";
    let mut answer = line_answer(declared);
    answer["declared_action"] = json!({
        "doing": "sets a board on an easel and lays in mossy steps", "pose": null,
        "items_in_use": ["boards"]
    });
    // Twice malformed, then an answer that stands: the line stands with that answer's entries.
    let host = scripted_host(vec![
        answer.clone(),
        contradictory(),
        contradictory(),
        uses(&["ochre"]),
    ])
    .await;
    wrens_kit(&host);
    let declare = json!({"character": CHARACTER, "situation": "The study begins."});
    let spoken: PersonaReply<Spoken> = post_json(host.port, "/dev/persona/declare", &declare)
        .await
        .unwrap();
    let said = done(spoken).said;
    assert_eq!(said.line, declared);
    assert_eq!(moved_counts(&said), [("ochre", 6, 5)]);
    assert_eq!(
        host.lines_starting_with("WORLD retry verb=inventory_judged"),
        [
            "WORLD retry verb=inventory_judged attempt=2 reason=malformed",
            "WORLD retry verb=inventory_judged attempt=3 reason=malformed",
        ]
    );
    // Three times malformed: the line still stands, nothing changes, and the judge gives up.
    let host = scripted_host(vec![
        answer,
        contradictory(),
        contradictory(),
        contradictory(),
    ])
    .await;
    wrens_kit(&host);
    let spoken: PersonaReply<Spoken> = post_json(host.port, "/dev/persona/declare", &declare)
        .await
        .unwrap();
    let said = done(spoken).said;
    assert_eq!(said.line, declared);
    assert!(said.inventory.is_empty(), "{:?}", said.inventory);
    assert_eq!(
        host.lines_starting_with("PERSONA judge_gave_up"),
        ["PERSONA judge_gave_up judge=inventory id=innkeeper attempts=3 reason=malformed"]
    );
}

#[tokio::test]
async fn a_line_of_someone_who_carries_anything_is_judged_for_it_and_the_line_call_moves_nothing() {
    // One owner of what a line moves: the line call has no field for it, and every line of a
    // character who carries anything is judged by the inventory call, expected or not.
    let given = "That's the weathered wooden gate done. I've used up the board, a little ochre \
                 and umber, some oil and a rag.";
    let host = scripted_host(vec![line_answer(given), finished_painting()]).await;
    wrens_kit(&host);
    let said = done(say(&host, CHARACTER, json!({"text": given})).await).said;
    assert_eq!(moved_counts(&said), PAINTED);
    let said_line = host.lines_starting_with("PERSONA said");
    assert!(said_line[0].ends_with("changes=6"), "{said_line:?}");
    assert!(
        host.lines_starting_with("PERSONA inventory_judged")[0].starts_with(concat!(
            "PERSONA inventory_judged id=innkeeper expected=no occasion=speaking ",
            "entries=6 ms="
        ))
    );
    // Nobody else present holds anything: no trade facts and no deal judge.
    assert_eq!(host.calls(), 2);
    assert!(!host.instructions_of(0).contains("now.open_deals"));
    let schema = host.asked.lock().unwrap()[0].output_schema.clone();
    assert!(schema["properties"].get("deal").is_none(), "{schema}");
    assert!(
        schema["properties"].get("inventory_changes").is_none(),
        "{schema}"
    );
}

/// Wren's coins and painting, and the pigment seller's stock, as the painter's day (host9) had
/// them: boards and verdigris at 20 cp each, both sides paying in silver.
fn verdigris_stall(host: &ScriptedHost) {
    write_persona(&host.directory(), &vendor("wren")).unwrap();
    write_persona(&host.directory(), &vendor("pigment_seller")).unwrap();
    let item = |id: &str, name: &str, count: u64| {
        let mut item = json!({"id": id, "name": name, "look": format!("{name} up close"),
                              "use": "consumed", "count": count});
        match id {
            "coins" => item["coin"] = json!("sp"),
            "boards" | "verdigris" => item["price_cp"] = json!(20),
            _ => {}
        }
        item
    };
    host.running.write_asset(
        "world/inventory/wren.json",
        &json!({"schema": 1, "owner": "wren", "kind": "character", "items": [
            item("coins", "silver pennies", 20), item("painting_01", "gate study", 1)
        ]})
        .to_string(),
    );
    host.running.write_asset(
        "world/inventory/pigment_seller.json",
        &json!({"schema": 1, "owner": "pigment_seller", "kind": "shop", "items": [
            item("boards", "primed boards", 10), item("verdigris", "verdigris", 5),
            item("coins", "silver pennies", 14)
        ]})
        .to_string(),
    );
}

/// The SRD coins (SRD 5.2.1 p.89), as a deal answer names money.
const COIN_KEYS: [&str; 5] = ["cp", "sp", "ep", "gp", "pp"];

/// The deal judge's answer: `state` (`none` for no trade), its reason and the terms from the
/// speaker's side: an entry named by a coin key (`("sp", 2)`) is money in that coin, the rest
/// goods.
fn terms(state: Option<&str>, gives: &[(&str, u64)], gets: &[(&str, u64)]) -> Value {
    let goods = |items: &[(&str, u64)]| {
        items
            .iter()
            .filter(|(item, _)| !COIN_KEYS.contains(item))
            .map(|(item, count)| json!({"item": item, "count": count}))
            .collect::<Vec<_>>()
    };
    let coins = |items: &[(&str, u64)]| {
        items
            .iter()
            .filter(|(item, _)| COIN_KEYS.contains(item))
            .map(|(coin, amount)| json!({"amount": amount, "coin": coin}))
            .collect::<Vec<_>>()
    };
    // Each side knows the other by a stranger's handle.
    json!({"with": "person-1", "state": state.unwrap_or("none"), "because": "the judge's reason",
           "claims_exchange": false, "gives": goods(gives), "gives_coins": coins(gives),
           "gets": goods(gets), "gets_coins": coins(gets)})
}

/// `answer`, a deal judge's answer, saying the line claims goods or money change hands now.
fn claiming(mut answer: Value) -> Value {
    answer["claims_exchange"] = json!(true);
    answer
}

#[tokio::test]
async fn the_host9_offer_counter_and_accept_are_judged_on_their_own_and_settle_at_three_silver() {
    let offer = "I'll take four of your primed boards and a pot of verdigris. Two silver for \
                 the lot?";
    let counter = "Four boards and the verdigris are worth more than that. I'll take three silver \
                   for the lot.";
    let accept = "Three silver it is, then. Here you are.";
    let thanks = "Pleasure doing business, love.";
    let wares = [("boards", 4), ("verdigris", 1)];
    let nothing = judged(Vec::new());
    let host = scripted_host_cast(
        &["wren", "pigment_seller"],
        vec![
            line_answer(offer),
            nothing.clone(),
            // An accept with nothing open is refused by code and asked again with why.
            terms(Some("accept"), &[("sp", 2)], &wares),
            terms(Some("offer"), &[("sp", 2)], &wares),
            line_answer(counter),
            nothing.clone(),
            terms(Some("counter"), &wares, &[("sp", 3)]),
            line_answer(accept),
            // The payment listed as spent: the deal alone moves it.
            judged(vec![entry("coins", -3, "used_up", None, &[])]),
            terms(Some("accept"), &[("sp", 3)], &wares),
            line_answer(thanks),
            nothing,
            // The settled terms put again by a pleasantry are refused, and the judge says none.
            terms(Some("offer"), &wares, &[("sp", 3)]),
            terms(None, &[], &[]),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    verdigris_stall(&host);
    done(say(&host, "wren", json!({"text": offer})).await);
    assert_eq!(host.calls(), 4);
    let deal_call = host.asked.lock().unwrap()[2].clone();
    assert_eq!(deal_call.output_name, "persona_deal");
    let deal = &deal_call.output_schema["properties"];
    assert_eq!(
        deal["gives"]["items"]["properties"]["item"]["enum"],
        json!(["painting_01"]),
        "gives is closed to the goods the speaker holds"
    );
    assert_eq!(
        deal["gets"]["items"]["properties"]["item"]["enum"],
        json!(["boards", "verdigris"])
    );
    assert_eq!(
        deal["gives_coins"]["items"]["properties"]["coin"]["enum"],
        json!(["cp", "sp", "ep", "gp", "pp"]),
        "money is an amount of an SRD coin"
    );
    assert_eq!(deal["with"]["enum"], json!(["person-1"]));
    assert!(deal_call.facts["context"]["others"]["person-1"].is_array());
    assert_eq!(deal_call.facts["now"]["line"], offer);
    assert!(
        host.facts_of(3)["now"]["retry_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("no open terms")),
        "{}",
        host.facts_of(3)
    );
    done(say(&host, "pigment_seller", json!({"text": counter})).await);
    // The keeper's line knows the open terms and her prices, so her words and the books agree.
    let keeper = host.facts_of(4);
    let open = &keeper["now"]["open_deals"][0];
    assert_eq!(open["put_by"], "them", "{open}");
    assert_eq!(open["gets"]["coins"], json!([{"amount": 2, "coin": "sp"}]));
    assert_eq!(open["gets"]["coins_cp"], 20);
    assert_eq!(open["settles"], true, "{open}");
    assert!(
        keeper["now"]["prices"]
            .as_array()
            .is_some_and(|prices| prices.len() == 2)
    );
    assert!(
        host.instructions_of(4)
            .contains("now.open_deals holds the terms open")
    );
    let settled = done(say(&host, "wren", json!({"text": accept})).await).said;
    let moved: Vec<(&str, &str, u64, u64)> = settled
        .inventory
        .iter()
        .map(|applied| {
            (
                applied.owner.as_str(),
                applied.item.as_str(),
                applied.before,
                applied.after,
            )
        })
        .collect();
    assert_eq!(
        moved,
        [
            ("pigment_seller", "boards", 10, 6),
            ("wren", "boards", 0, 4),
            ("pigment_seller", "verdigris", 5, 4),
            ("wren", "verdigris", 0, 1),
            ("wren", "coins", 20, 17),
            ("pigment_seller", "coins", 14, 17),
        ],
        "the coins are paid once"
    );
    assert_eq!(
        host.lines_starting_with("PERSONA deal_entries"),
        ["PERSONA deal_entries id=wren deal=accept moved_by_deal=coins:used_up"]
    );
    done(say(&host, "pigment_seller", json!({"text": thanks})).await);
    assert_eq!(host.calls(), 14);
    let world = host.lines_starting_with("WORLD deal");
    assert_eq!(
        world,
        [
            "WORLD deal_open state=offer owner=wren with=pigment_seller \
             gives=silver-piece*2 gets=boards*4|verdigris*1",
            "WORLD deal_open state=counter owner=pigment_seller with=wren \
             gives=boards*4|verdigris*1 gets=silver-piece*3",
            "WORLD deal_settled owner=wren with=pigment_seller gives=silver-piece*3 \
             gets=boards*4|verdigris*1 wares_cp=100 coins_cp=30",
        ]
    );
    let judged_states: Vec<String> = host
        .lines_starting_with("PERSONA deal_judged")
        .iter()
        .map(|line| {
            line.split(' ')
                .find(|part| part.starts_with("state="))
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(
        judged_states,
        ["state=offer", "state=counter", "state=accept", "state=none"]
    );
    assert!(
        host.lines_starting_with("PERSONA deal_judged")[3].starts_with(concat!(
            "PERSONA deal_judged id=pigment_seller counterparty=- state=none gives=0 ",
            "gets=0 ms="
        ))
    );
}

#[tokio::test]
async fn a_line_said_to_one_keeper_deals_with_that_keeper_whatever_the_judge_names() {
    let offer = "I'll take three of your primed boards for six silver pennies.";
    let host = scripted_host_cast(
        &["wren", "pigment_seller", "fruit_vendor"],
        // The judge names someone else; the line was said to the pigment seller.
        vec![
            line_answer(offer),
            judged(Vec::new()),
            json!({"with": "person-2", "state": "offer", "because": "it names goods and a price",
                   "claims_exchange": false, "gives": [],
                   "gives_coins": [{"amount": 6, "coin": "sp"}],
                   "gets": [{"item": "boards", "count": 3}], "gets_coins": []}),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    verdigris_stall(&host);
    write_persona(&host.directory(), &vendor("fruit_vendor")).unwrap();
    host.running.write_asset(
        "world/inventory/fruit_vendor.json",
        &json!({"schema": 1, "owner": "fruit_vendor", "kind": "shop", "items": [
            {"id": "apples", "name": "apples", "look": "red", "use": "consumed", "count": 9,
             "price_cp": 1}
        ]})
        .to_string(),
    );
    done(
        say(
            &host,
            "wren",
            json!({"text": offer, "asked_by": "pigment_seller"}),
        )
        .await,
    );
    let others = host.facts_of(0)["now"]["others_inventory"].clone();
    assert_eq!(others.as_object().map(|map| map.len()), Some(1), "{others}");
    let shown = host.facts_of(2)["context"]["others"].clone();
    assert_eq!(shown.as_object().map(|map| map.len()), Some(1), "{shown}");
    assert!(
        host.lines_starting_with("WORLD deal_open")[0]
            .starts_with("WORLD deal_open state=offer owner=wren with=pigment_seller")
    );
}

#[tokio::test]
async fn an_inventory_judge_still_refused_after_three_attempts_gives_up_and_moves_nothing() {
    let wrong = uses(&["boards", "ochre"]);
    let host = scripted_host(vec![
        line_answer("It's finished. I used a board and some ochre."),
        wrong.clone(),
        wrong.clone(),
        wrong,
    ])
    .await;
    painters_shelf(&host);
    let said = done(say(&host, CHARACTER, json!({})).await).said;
    assert_eq!(host.calls(), 4, "the line, then three attempts and no more");
    assert!(said.inventory.is_empty(), "{:?}", said.inventory);
    assert_eq!(
        host.lines_starting_with("PERSONA judge_gave_up"),
        ["PERSONA judge_gave_up judge=inventory id=innkeeper attempts=3 reason=insufficient"]
    );
}

#[tokio::test]
async fn a_deal_judge_still_refused_after_three_attempts_gives_up_and_makes_no_deal() {
    let offer = "Two silver for four boards and the verdigris?";
    // An accept with nothing open, every time.
    let accept = terms(Some("accept"), &[("sp", 2)], &[("boards", 4)]);
    let host = scripted_host_cast(
        &["wren", "pigment_seller"],
        vec![
            line_answer(offer),
            judged(Vec::new()),
            accept.clone(),
            accept.clone(),
            accept,
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    verdigris_stall(&host);
    done(say(&host, "wren", json!({"text": offer})).await);
    assert_eq!(host.calls(), 5);
    assert!(host.lines_starting_with("WORLD deal_").is_empty());
    assert_eq!(
        host.lines_starting_with("PERSONA judge_gave_up"),
        [concat!(
            "PERSONA judge_gave_up judge=deal id=wren attempts=3 ",
            "reason=there are no open terms with pigment_seller to accept"
        )]
    );
    assert!(host.lines_starting_with("PERSONA deal_judged")[0].contains(" state=none "));
}

#[tokio::test]
async fn a_declared_action_in_progress_is_judged_as_declaring_and_uses_nothing_up() {
    // Loop 9: the declaration that starts the painting used up the board and the paints, and the
    // finishing line used them again. The judge is told the line declares work under way.
    let declared = "I set a board on the easel and start laying in the mossy steps with my brush.";
    let mut answer = line_answer(declared);
    answer["declared_action"] = json!({
        "doing": "sets a board on an easel and lays in mossy steps", "pose": null,
        "items_in_use": ["boards"]
    });
    let host = scripted_host(vec![answer, judged(Vec::new())]).await;
    wrens_kit(&host);
    let spoken: PersonaReply<Spoken> = post_json(
        host.port,
        "/dev/persona/declare",
        &json!({"character": CHARACTER, "situation": "The study begins."}),
    )
    .await
    .unwrap();
    let said = done(spoken).said;
    assert!(said.inventory.is_empty(), "{:?}", said.inventory);
    assert_eq!(host.calls(), 2);
    assert_eq!(host.facts_of(1)["now"]["occasion"], "declaring");
    assert!(
        host.instructions_of(1)
            .contains("so a declaring line lists none of them")
    );
    assert!(
        host.lines_starting_with("PERSONA inventory_judged")[0]
            .contains(" occasion=declaring entries=0 ")
    );
}

#[tokio::test]
async fn something_eaten_in_an_ordinary_line_is_used_up() {
    let host = scripted_host(vec![
        line_answer("I eat an apple."),
        judged(vec![entry("apple", -1, "used_up", None, &[])]),
    ])
    .await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    host.running.write_asset(
        "world/inventory/innkeeper.json",
        &json!({"schema": 1, "owner": CHARACTER, "kind": "character", "items": [
            {"id": "apple", "name": "apple", "look": "A red apple.", "use": "consumed",
             "count": 3}
        ]})
        .to_string(),
    );
    let said = done(say(&host, CHARACTER, json!({})).await).said;
    assert_eq!(host.facts_of(1)["now"]["occasion"], "speaking");
    assert_eq!(moved_counts(&said), [("apple", 3, 2)]);
}

/// The hero's coins and the pigment vendor's stock in loop 9: three primed boards are a silver and
/// five copper, a pot of madder red eight copper.
fn madder_stall(host: &ScriptedHost) {
    write_persona(&host.directory(), &vendor("wren")).unwrap();
    write_persona(&host.directory(), &vendor("pigment_vendor")).unwrap();
    host.running.write_asset(
        "world/inventory/wren.json",
        &json!({"schema": 1, "owner": "wren", "kind": "character", "items": [
            {"id": "coins", "name": "silver pennies", "look": "small silver coins",
             "use": "consumed", "count": 20, "coin": "sp"}
        ]})
        .to_string(),
    );
    host.running.write_asset(
        "world/inventory/pigment_vendor.json",
        &json!({"schema": 1, "owner": "pigment_vendor", "kind": "shop", "items": [
            {"id": "boards", "name": "primed boards", "look": "white boards", "use": "consumed",
             "count": 10, "price_cp": 5},
            {"id": "madder", "name": "madder red", "look": "a red pot", "use": "consumed",
             "count": 4, "price_cp": 8},
            {"id": "coins", "name": "silver pennies", "look": "small silver coins",
             "use": "consumed", "count": 9, "coin": "sp"}
        ]})
        .to_string(),
    );
}

#[tokio::test]
async fn an_offer_put_as_a_question_opens_terms_and_a_reasoned_decline_refuses_them() {
    let offer = "I'd like three of your primed boards and a pot of madder red. Two silver for the \
                 lot?";
    let decline = "That's below fair trade, friend. Three boards are 1 silver and 5 copper, and \
                   the madder's 8 copper; I can't take two silver for the lot.";
    let nothing = judged(Vec::new());
    let host = scripted_host_cast(
        &["wren", "pigment_vendor"],
        vec![
            line_answer(offer),
            nothing.clone(),
            terms(Some("offer"), &[("sp", 2)], &[("boards", 3), ("madder", 1)]),
            line_answer(decline),
            nothing,
            terms(Some("refuse"), &[], &[]),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    madder_stall(&host);
    done(
        say(
            &host,
            "wren",
            json!({"text": offer, "asked_by": "pigment_vendor"}),
        )
        .await,
    );
    let deal_call = host.asked.lock().unwrap()[2].clone();
    let state = &deal_call.output_schema["properties"]["state"];
    assert_eq!(
        state["enum"],
        json!(["none", "offer", "counter", "accept", "refuse", "gift"]),
        "no trade is said outright"
    );
    assert!(
        deal_call.output_schema["required"]
            .as_array()
            .is_some_and(|required| required.contains(&json!("because")))
    );
    assert!(
        deal_call
            .instructions
            .contains("A proposal put as a question is an offer of those terms")
    );
    done(
        say(
            &host,
            "pigment_vendor",
            json!({"text": decline, "asked_by": "wren"}),
        )
        .await,
    );
    assert_eq!(host.calls(), 6);
    assert_eq!(
        host.lines_starting_with("WORLD deal"),
        [
            "WORLD deal_open state=offer owner=wren with=pigment_vendor \
             gives=silver-piece*2 gets=boards*3|madder*1",
            "WORLD deal_dropped owner=pigment_vendor with=wren reason=refused",
        ]
    );
    let judged_lines = host.lines_starting_with("PERSONA deal_judged");
    assert!(
        judged_lines[1].contains(" state=refuse ")
            && judged_lines[1].ends_with(" because=the judge's reason"),
        "{judged_lines:?}"
    );
}

/// A coin item of `coin` (`cp`..`pp`), as an inventory's JSON holds it.
fn coin(id: &str, count: u64, coin: &str) -> Value {
    json!({"id": id, "name": id, "look": "struck coins", "use": "consumed", "count": count,
           "coin": coin})
}

/// The explore run's stall (play-1794897430565, steps 09-11): the hero holds only gold, and the
/// produce seller has apples at three copper and, when `till` is set, a till that can make change.
fn produce_stall(host: &ScriptedHost, till: bool) {
    write_persona(&host.directory(), &vendor("wren")).unwrap();
    write_persona(&host.directory(), &vendor("produce_vendor")).unwrap();
    host.running.write_asset(
        "world/inventory/wren.json",
        &json!({"schema": 1, "owner": "wren", "kind": "character", "items": [
            coin("gold-piece", 78, "gp")
        ]})
        .to_string(),
    );
    let mut stock = vec![
        json!({"id": "apple", "name": "apples", "look": "red apples",
                                "use": "consumed", "count": 10, "price_cp": 3}),
    ];
    if till {
        stock.extend([
            coin("copper-piece", 20, "cp"),
            coin("silver-piece", 20, "sp"),
            coin("gold-piece", 10, "gp"),
        ]);
    }
    host.running.write_asset(
        "world/inventory/produce_vendor.json",
        &json!({"schema": 1, "owner": "produce_vendor", "kind": "shop", "items": stock})
            .to_string(),
    );
}

#[tokio::test]
async fn six_copper_said_from_a_purse_of_gold_is_judged_as_six_copper_and_the_accept_settles() {
    let offer = "Six copper it is, here you go. I'll take the two apples.";
    let accept = "There we are, neighbor. Two apples for six copper.";
    // The run's answer: six of the gold the purse holds. Money is an amount of the coin said.
    let as_gold = json!({"with": "person-1", "state": "offer", "because": "the judge's reason",
                         "claims_exchange": false,
                         "gives": [{"item": "gold-piece", "count": 6}], "gives_coins": [],
                         "gets": [{"item": "apple", "count": 2}], "gets_coins": []});
    let host = scripted_host_cast(
        &["wren", "produce_vendor"],
        vec![
            line_answer(offer),
            judged(Vec::new()),
            as_gold,
            // An amount of none is no amount.
            terms(Some("offer"), &[("cp", 0)], &[("apple", 2)]),
            terms(Some("offer"), &[("cp", 6)], &[("apple", 2)]),
            line_answer(accept),
            judged(Vec::new()),
            terms(Some("accept"), &[("apple", 2)], &[("cp", 6)]),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    produce_stall(&host, true);
    done(
        say(
            &host,
            "wren",
            json!({"text": offer, "asked_by": "produce_vendor"}),
        )
        .await,
    );
    let deal = host.asked.lock().unwrap()[2].output_schema["properties"].clone();
    assert_eq!(
        deal["gives"]["maxItems"], 0,
        "a purse of coins has no goods to give"
    );
    let retries = host.lines_starting_with("WORLD retry verb=deal");
    assert_eq!(retries.len(), 2, "{retries:?}");
    assert!(
        retries[0].contains("names the coin gold-piece as goods"),
        "{retries:?}"
    );
    assert!(
        retries[1].contains("an amount of cp is none"),
        "{retries:?}"
    );
    // The seller's line knows the open terms in the coin said, and that they would settle.
    let said = done(
        say(
            &host,
            "produce_vendor",
            json!({"text": accept, "asked_by": "wren"}),
        )
        .await,
    )
    .said;
    let open = &host.facts_of(5)["now"]["open_deals"][0];
    assert_eq!(
        open["gets"]["coins"],
        json!([{"amount": 6, "coin": "cp"}]),
        "{open}"
    );
    assert_eq!(open["settles"], true, "{open}");
    assert_eq!(
        host.facts_of(5)["now"]["prices"][0]["price"],
        json!({"amount": 3, "coin": "cp"}),
        "an apple is priced at three copper, never a gold piece"
    );
    assert!(
        host.instructions_of(5)
            .contains("A line never says goods or coins changed hands unless")
    );
    assert!(
        !said.inventory.is_empty(),
        "the accept settles: {:?}",
        said.inventory
    );
    assert_eq!(
        host.lines_starting_with("WORLD deal"),
        [
            "WORLD deal_open state=offer owner=wren with=produce_vendor gives=copper-piece*6 \
             gets=apple*2",
            "WORLD deal_settled owner=produce_vendor with=wren gives=apple*2 \
             gets=copper-piece*6 wares_cp=6 coins_cp=6",
        ]
    );
    let paid = host.lines_starting_with("INVENTORY deal");
    assert!(
        paid[0].starts_with(
            "INVENTORY deal owner=wren with=produce_vendor payer=wren cp=6 handed_cp=100 \
             change_cp=94 coins_before=7800 coins_after=7794 "
        ),
        "{paid:?}"
    );
}

#[tokio::test]
async fn an_accept_the_books_refuse_is_worded_again_and_never_claims_the_sale() {
    let offer = "Six copper for two apples?";
    let sold = "There we are, neighbor. Two apples for six copper.";
    let cannot = "I can't break a gold piece, neighbor. Have you anything smaller?";
    let host = scripted_host_cast(
        &["wren", "produce_vendor"],
        vec![
            line_answer(offer),
            judged(Vec::new()),
            terms(Some("offer"), &[("cp", 6)], &[("apple", 2)]),
            line_answer(sold),
            judged(Vec::new()),
            terms(Some("accept"), &[("apple", 2)], &[("cp", 6)]),
            // Worded again with why: the seller cannot make change for a gold piece.
            line_answer(cannot),
            judged(Vec::new()),
            terms(None, &[], &[]),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    produce_stall(&host, false);
    done(
        say(
            &host,
            "wren",
            json!({"text": offer, "asked_by": "produce_vendor"}),
        )
        .await,
    );
    let said = done(say(&host, "produce_vendor", json!({"asked_by": "wren"})).await).said;
    assert_eq!(host.calls(), 9);
    assert_eq!(said.line, cannot);
    // The seller's line was told the open terms would not settle, and why.
    let open = &host.facts_of(3)["now"]["open_deals"][0];
    assert_eq!(open["settles"], false, "{open}");
    assert_eq!(open["refused"], "no_change", "{open}");
    assert!(said.inventory.is_empty(), "{:?}", said.inventory);
    assert!(
        host.facts_of(6)["now"]["retry_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("no_change")
                && reason.contains("must not say the trade is done")),
        "{}",
        host.facts_of(6)
    );
    assert_eq!(
        host.lines_starting_with("PERSONA deal_unsettled"),
        ["PERSONA deal_unsettled id=produce_vendor reason=no_change reworded=yes"]
    );
    assert!(host.lines_starting_with("WORLD deal_settled").is_empty());
    assert!(
        host.running
            .state
            .deals()
            .settled_of("produce_vendor")
            .is_empty()
    );
}

#[tokio::test]
async fn a_hand_over_of_open_terms_judged_none_is_asked_again_and_settles_as_an_accept() {
    let offer = "Six copper for two apples, if you please.";
    let handed = "There you are. Two apples, as agreed.";
    let host = scripted_host_cast(
        &["wren", "produce_vendor"],
        vec![
            line_answer(offer),
            judged(Vec::new()),
            terms(Some("offer"), &[("cp", 6)], &[("apple", 2)]),
            line_answer(handed),
            judged(Vec::new()),
            // Host 36: "hands over the two apples under the already agreed terms": none.
            claiming(terms(None, &[], &[])),
            claiming(terms(Some("accept"), &[("apple", 2)], &[("cp", 6)])),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    produce_stall(&host, true);
    done(
        say(
            &host,
            "wren",
            json!({"text": offer, "asked_by": "produce_vendor"}),
        )
        .await,
    );
    let said = done(say(&host, "produce_vendor", json!({"asked_by": "wren"})).await).said;
    assert_eq!(host.calls(), 7);
    assert!(
        host.facts_of(6)["now"]["retry_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("such a line accepts those terms")),
        "{}",
        host.facts_of(6)
    );
    assert_eq!(
        host.lines_starting_with("WORLD retry verb=deal"),
        ["WORLD retry verb=deal attempt=2 reason=none_on_open_terms"]
    );
    assert!(
        host.instructions_of(5)
            .contains("none is only for a line that puts no terms")
    );
    assert_eq!(said.line, handed);
    assert!(!said.inventory.is_empty(), "{:?}", said.inventory);
    assert_eq!(host.lines_starting_with("WORLD deal_settled").len(), 1);
    assert!(
        host.lines_starting_with("PERSONA deal_unsettled")
            .is_empty()
    );
}

#[tokio::test]
async fn a_line_that_claims_an_exchange_while_terms_are_open_and_settles_nothing_is_worded_again() {
    let offer = "Six copper for two apples, if you please.";
    let handed = "There you are. Two apples, as agreed.";
    let fetch = "Two apples coming. Let me fetch the good ones from the back first.";
    let still_none = claiming(terms(None, &[], &[]));
    let host = scripted_host_cast(
        &["wren", "produce_vendor"],
        vec![
            line_answer(offer),
            judged(Vec::new()),
            terms(Some("offer"), &[("cp", 6)], &[("apple", 2)]),
            line_answer(handed),
            judged(Vec::new()),
            // Asked again once, the judge still says none: the line claims what nothing records.
            still_none.clone(),
            still_none,
            line_answer(fetch),
            judged(Vec::new()),
            terms(None, &[], &[]),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    produce_stall(&host, true);
    done(
        say(
            &host,
            "wren",
            json!({"text": offer, "asked_by": "produce_vendor"}),
        )
        .await,
    );
    let said = done(say(&host, "produce_vendor", json!({"asked_by": "wren"})).await).said;
    assert_eq!(host.calls(), 10);
    assert_eq!(said.line, fetch);
    assert!(said.inventory.is_empty(), "{:?}", said.inventory);
    assert_eq!(
        host.lines_starting_with("PERSONA deal_unsettled"),
        ["PERSONA deal_unsettled id=produce_vendor reason=claims_exchange reworded=yes"]
    );
    assert!(
        host.facts_of(7)["now"]["retry_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("claims_exchange")),
        "{}",
        host.facts_of(7)
    );
    assert!(host.lines_starting_with("WORLD deal_settled").is_empty());
    // The terms stay open for a line that accepts them.
    assert!(
        host.running
            .state
            .deals()
            .between("wren", "produce_vendor")
            .is_some()
    );
}

/// The seller's line with `quotes` (item, qty, total in copper) as its lots.
fn quoting(line: &str, quotes: &[(&str, u64, u64)]) -> Value {
    let mut answer = line_answer(line);
    answer["quotes"] = quotes
        .iter()
        .map(|(item, qty, total_cp)| json!({"item": item, "qty": qty, "total_cp": total_cp}))
        .collect();
    answer
}

#[tokio::test]
async fn a_plain_quote_off_the_stock_price_is_worded_again_at_the_stock_price() {
    let host = scripted_host_cast(
        &["wren", "produce_vendor"],
        vec![
            // As in host 37, a quote off the stock: two apples at three copper are six, said as
            // eight.
            quoting("Two apples come to {price:0}.", &[("apple", 2, 8)]),
            judged(Vec::new()),
            terms(None, &[], &[]),
            quoting("Two apples, {price:0}.", &[("apple", 2, 6)]),
            judged(Vec::new()),
            terms(None, &[], &[]),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    produce_stall(&host, true);
    let said = done(say(&host, "produce_vendor", json!({"asked_by": "wren"})).await).said;
    assert_eq!(host.calls(), 6);
    assert_eq!(said.line, "Two apples, 6 cp.");
    assert!(
        host.facts_of(3)["now"]["retry_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("you said 8 cp for 2 apple")
                && reason.contains("stock price (now.prices) is 6 cp")),
        "{}",
        host.facts_of(3)
    );
    assert_eq!(
        host.lines_starting_with("PERSONA quote_reworded"),
        ["PERSONA quote_reworded id=produce_vendor"]
    );
    assert!(
        host.instructions_of(0)
            .contains("A price this character tells for their own wares is now.prices exactly")
    );
}

#[tokio::test]
async fn a_counter_to_open_terms_may_name_another_total_than_the_stock_price() {
    let offer = "Six copper for two apples?";
    let host = scripted_host_cast(
        &["wren", "produce_vendor"],
        vec![
            line_answer(offer),
            judged(Vec::new()),
            terms(Some("offer"), &[("cp", 6)], &[("apple", 2)]),
            quoting(
                "Not for six. Two apples are {price:0} to you.",
                &[("apple", 2, 8)],
            ),
            judged(Vec::new()),
            terms(Some("counter"), &[("apple", 2)], &[("cp", 8)]),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    produce_stall(&host, true);
    done(
        say(
            &host,
            "wren",
            json!({"text": offer, "asked_by": "produce_vendor"}),
        )
        .await,
    );
    let said = done(say(&host, "produce_vendor", json!({"asked_by": "wren"})).await).said;
    assert_eq!(host.calls(), 6, "a counter is not worded again");
    assert_eq!(said.line, "Not for six. Two apples are 8 cp to you.");
    assert!(
        host.lines_starting_with("PERSONA quote_reworded")
            .is_empty()
    );
    assert_eq!(
        host.lines_starting_with("WORLD deal_open")[1],
        "WORLD deal_open state=counter owner=produce_vendor with=wren gives=apple*2 \
         gets=copper-piece*8"
    );
}

#[tokio::test]
async fn price_slots_numbered_by_the_prose_are_filled_by_the_lots_in_order_never_shown_raw() {
    // Host 42: two lots quoted, their slots numbered 2 and 3 by the goods the line names first.
    let counted = "Apples and onions are a copper each, rations are {price:2} a bundle, and \
                   waterskins are {price:3} apiece.";
    let lots = [("rations", 1, 50), ("waterskin", 1, 20)];
    let host = scripted_host_cast(
        &["wren", "produce_vendor"],
        vec![
            quoting(counted, &lots),
            // Worded again for the slots, the same numbering comes back.
            quoting(counted, &lots),
            judged(Vec::new()),
            terms(None, &[], &[]),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    produce_stall(&host, true);
    let said = done(say(&host, "produce_vendor", json!({"asked_by": "wren"})).await).said;
    assert!(!said.line.contains('{'), "{}", said.line);
    let rations = said.line.find("rations are ").unwrap();
    let waterskins = said.line.find("waterskins are ").unwrap();
    assert!(
        said.line[rations..waterskins].contains("50 cp"),
        "{}",
        said.line
    );
    assert!(said.line[waterskins..].contains("20 cp"), "{}", said.line);
    assert!(
        host.lines_starting_with("PERSONA price_slots_unfilled")
            .is_empty()
    );
}

#[tokio::test]
async fn a_line_whose_price_slots_cannot_be_filled_is_refused_never_shown_with_a_brace() {
    let unfillable = "Rations are {price:0}, waterskins {price:1}, and the rope {price:5}.";
    let lots = [("rations", 1, 50), ("waterskin", 1, 20)];
    let host = scripted_host_cast(
        &["wren", "produce_vendor"],
        vec![
            quoting(unfillable, &lots),
            quoting(unfillable, &lots),
            quoting(unfillable, &lots),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    produce_stall(&host, true);
    let reply = say(&host, "produce_vendor", json!({"asked_by": "wren"})).await;
    assert!(
        matches!(reply, DevReply::Rejected { .. }),
        "a line with an unfilled slot is never shown"
    );
    assert_eq!(host.calls(), 3);
    assert_eq!(
        host.lines_starting_with("PERSONA price_slots_unfilled"),
        [
            "PERSONA price_slots_unfilled id=produce_vendor reworded=yes",
            "PERSONA price_slots_unfilled id=produce_vendor reworded=no",
        ]
    );
    assert!(host.lines_starting_with("PERSONA said").is_empty());
}

#[tokio::test]
async fn a_counter_that_drops_the_price_is_asked_again_and_a_coin_handed_settles_with_change() {
    let offer = "A silver for a waterskin and a bundle of rations?";
    let counter = "Not for a silver. A waterskin and rations come to seven silver.";
    let accept = "Fine, seven silver it is. Here's a gold piece.";
    let goods = [("waterskin", 1), ("apple", 1)];
    let host = scripted_host_cast(
        &["wren", "produce_vendor"],
        vec![
            line_answer(offer),
            judged(Vec::new()),
            terms(Some("offer"), &[("sp", 1)], &goods),
            line_answer(counter),
            judged(Vec::new()),
            // Host 42: the counter judged with the goods and no price at all.
            terms(Some("counter"), &goods, &[]),
            terms(Some("counter"), &goods, &[("sp", 7)]),
            line_answer(accept),
            judged(Vec::new()),
            // Host 42: the accept named the gold piece handed, not the seven silver agreed.
            terms(Some("accept"), &[("gp", 1)], &goods),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    produce_stall(&host, true);
    // The seller holds a waterskin too, and the buyer holds only gold.
    host.running.write_asset(
        "world/inventory/produce_vendor.json",
        &json!({"schema": 1, "owner": "produce_vendor", "kind": "shop", "items": [
            {"id": "apple", "name": "apples", "look": "red apples", "use": "consumed",
             "count": 10, "price_cp": 3},
            {"id": "waterskin", "name": "waterskin", "look": "a leather skin",
             "use": "kept", "count": 2, "price_cp": 20},
            {"id": "copper-piece", "name": "copper", "look": "coins", "use": "consumed",
             "count": 20, "coin": "cp"},
            {"id": "silver-piece", "name": "silver", "look": "coins", "use": "consumed",
             "count": 20, "coin": "sp"}
        ]})
        .to_string(),
    );
    for (who, line) in [
        ("wren", offer),
        ("produce_vendor", counter),
        ("wren", accept),
    ] {
        let to = if who == "wren" {
            "produce_vendor"
        } else {
            "wren"
        };
        done(say(&host, who, json!({"text": line, "asked_by": to})).await);
    }
    assert_eq!(
        host.lines_starting_with("WORLD retry verb=deal"),
        ["WORLD retry verb=deal attempt=2 reason=price_dropped"]
    );
    assert_eq!(
        host.lines_starting_with("WORLD deal"),
        [
            "WORLD deal_open state=offer owner=wren with=produce_vendor gives=silver-piece*1 \
             gets=waterskin*1|apple*1",
            "WORLD deal_open state=counter owner=produce_vendor with=wren \
             gives=waterskin*1|apple*1 gets=silver-piece*7",
            "WORLD deal_settled owner=wren with=produce_vendor gives=silver-piece*7 \
             gets=waterskin*1|apple*1 wares_cp=23 coins_cp=70",
        ]
    );
    // Seven silver agreed, a gold piece handed: three silver back from the till.
    assert!(
        host.lines_starting_with("INVENTORY deal")[0].starts_with(
            "INVENTORY deal owner=produce_vendor with=wren payer=wren cp=70 handed_cp=100 \
             change_cp=30 coins_before=7800 coins_after=7730 "
        ),
        "{:?}",
        host.lines_starting_with("INVENTORY deal")
    );
}

/// Host 43's stall: the hero holds gold and rations, the seller rations at five silver and a till.
fn ration_stall(host: &ScriptedHost) {
    write_persona(&host.directory(), &vendor("wren")).unwrap();
    write_persona(&host.directory(), &vendor("produce_vendor")).unwrap();
    let rations = |count: u64| {
        json!({"id": "rations", "name": "travel rations", "look": "a wrapped bundle",
               "use": "consumed", "count": count, "price_cp": 50})
    };
    host.running.write_asset(
        "world/inventory/wren.json",
        &json!({"schema": 1, "owner": "wren", "kind": "character", "items": [
            coin("gold-piece", 78, "gp"), rations(10)
        ]})
        .to_string(),
    );
    host.running.write_asset(
        "world/inventory/produce_vendor.json",
        &json!({"schema": 1, "owner": "produce_vendor", "kind": "shop", "items": [
            rations(8), coin("copper-piece", 20, "cp"), coin("silver-piece", 20, "sp"),
            coin("gold-piece", 10, "gp")
        ]})
        .to_string(),
    );
}

#[tokio::test]
async fn a_gold_piece_handed_for_a_five_silver_ration_buys_it_with_change_and_a_gift_moves_one_way()
{
    let buy = "I'll take one travel ration. Here's a gold piece.";
    let gift = "I hand her one of my rations as an apology.";
    let host = scripted_host_cast(
        &["wren", "produce_vendor"],
        vec![
            line_answer(buy),
            judged(Vec::new()),
            // Host 43: the offer names the gold piece handed.
            terms(Some("offer"), &[("gp", 1)], &[("rations", 1)]),
            line_answer("Aye, one ration for your gold piece. Here's {change} back."),
            judged(Vec::new()),
            // Her accept names the ration and her own sum of the change, forty-five copper.
            claiming(terms(
                Some("accept"),
                &[("rations", 1), ("cp", 45)],
                &[("gp", 1)],
            )),
            line_answer(gift),
            judged(Vec::new()),
            terms(Some("gift"), &[("rations", 1)], &[]),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    ration_stall(&host);
    done(
        say(
            &host,
            "wren",
            json!({"text": buy, "asked_by": "produce_vendor"}),
        )
        .await,
    );
    let sold = done(say(&host, "produce_vendor", json!({"asked_by": "wren"})).await).said;
    // The change said is the books' fifty copper, never her own sum.
    assert_eq!(
        sold.line,
        "Aye, one ration for your gold piece. Here's 50 cp back."
    );
    done(
        say(
            &host,
            "wren",
            json!({"text": gift, "asked_by": "produce_vendor"}),
        )
        .await,
    );
    assert_eq!(
        host.lines_starting_with("WORLD deal"),
        [
            "WORLD deal_tendered owner=wren with=produce_vendor named_cp=100 price_cp=50",
            "WORLD deal_open state=offer owner=wren with=produce_vendor gives=silver-piece*5 \
             gets=rations*1",
            "WORLD deal_settled owner=produce_vendor with=wren gives=rations*1 \
             gets=silver-piece*5 wares_cp=50 coins_cp=50",
            "WORLD deal_gift owner=wren with=produce_vendor gives=rations*1",
        ]
    );
    assert!(
        host.lines_starting_with("INVENTORY deal")[0].starts_with(
            "INVENTORY deal owner=wren with=produce_vendor payer=wren cp=50 handed_cp=100 \
             change_cp=50 coins_before=7800 coins_after=7750 "
        ),
        "{:?}",
        host.lines_starting_with("INVENTORY deal")
    );
    let held = |owner: &str| crate::world::inventory_of(&host.running.state, owner);
    let count = |inventory: &[crate::world::InventoryItem], id: &str| {
        inventory
            .iter()
            .find(|item| item.id == id)
            .map_or(0, |item| item.count)
    };
    let (hero, seller) = (held("wren"), held("produce_vendor"));
    assert_eq!(count(&hero, "rations"), 10, "bought one, gave one");
    assert_eq!(count(&seller, "rations"), 8, "sold one, was given one");
    assert_eq!(
        count(&hero, "gold-piece"),
        77,
        "the hero pays, never gains, a gold piece"
    );
    assert_eq!(count(&seller, "gold-piece"), 11);
}

#[tokio::test]
async fn an_accept_the_books_settle_in_a_line_that_denies_it_is_worded_again_or_settles_nothing() {
    let buy = "I'll take one travel ration. Here's a gold piece.";
    let denies = "A gold piece is more than the five silver. I can't make change. Have you silver?";
    let settles = "Aye, the ration's yours, and {change} back.";
    let offer = || terms(Some("offer"), &[("gp", 1)], &[("rations", 1)]);
    // Host 45: the judge read the denial as an accept of the open five silver, saying no
    // exchange. Worded again with what settling does, the line accepts, and it settles.
    let host = scripted_host_cast(
        &["wren", "produce_vendor"],
        vec![
            line_answer(buy),
            judged(Vec::new()),
            offer(),
            line_answer(denies),
            judged(Vec::new()),
            terms(Some("accept"), &[], &[]),
            line_answer(settles),
            judged(Vec::new()),
            // The change only, as host 45's first accept named it: the ration side stands.
            claiming(terms(Some("accept"), &[("cp", 50)], &[("gp", 1)])),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    ration_stall(&host);
    done(
        say(
            &host,
            "wren",
            json!({"text": buy, "asked_by": "produce_vendor"}),
        )
        .await,
    );
    let said = done(say(&host, "produce_vendor", json!({"asked_by": "wren"})).await).said;
    assert_eq!(said.line, "Aye, the ration's yours, and 50 cp back.");
    assert_eq!(
        host.lines_starting_with("PERSONA deal_unconfirmed"),
        ["PERSONA deal_unconfirmed id=produce_vendor reworded=yes"]
    );
    assert!(
        host.facts_of(6)["now"]["retry_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("never say change cannot be made")),
        "{}",
        host.facts_of(6)
    );
    assert_eq!(host.lines_starting_with("WORLD deal_settled").len(), 1);

    // Worded again and still denying it: the words stand, and nothing settles.
    let host = scripted_host_cast(
        &["wren", "produce_vendor"],
        vec![
            line_answer(buy),
            judged(Vec::new()),
            offer(),
            line_answer(denies),
            judged(Vec::new()),
            terms(Some("accept"), &[], &[]),
            line_answer(denies),
            judged(Vec::new()),
            terms(Some("accept"), &[], &[]),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    ration_stall(&host);
    done(
        say(
            &host,
            "wren",
            json!({"text": buy, "asked_by": "produce_vendor"}),
        )
        .await,
    );
    let said = done(say(&host, "produce_vendor", json!({"asked_by": "wren"})).await).said;
    assert_eq!(said.line, denies);
    assert!(said.inventory.is_empty(), "{:?}", said.inventory);
    assert!(host.lines_starting_with("WORLD deal_settled").is_empty());
    assert_eq!(
        host.lines_starting_with("PERSONA deal_unconfirmed"),
        [
            "PERSONA deal_unconfirmed id=produce_vendor reworded=yes",
            "PERSONA deal_unconfirmed id=produce_vendor reworded=no",
        ]
    );
    assert!(
        host.running
            .state
            .deals()
            .between("wren", "produce_vendor")
            .is_some(),
        "the terms stay open as the words leave them"
    );
}

#[tokio::test]
async fn host46_a_quote_opens_terms_i_will_take_one_accepts_it_and_the_change_is_the_books() {
    let ask = "Good day! How much for a travel ration?";
    let take = "I'll take one. Here's a gold piece.";
    let thanks = "A gold piece for one ration, then. Here's your change.";
    let host = scripted_host_cast(
        &["wren", "produce_vendor"],
        vec![
            line_answer(ask),
            judged(Vec::new()),
            terms(None, &[], &[]),
            // Her quote of her own ware opens it at that price.
            quoting("A travel ration is {price:0}.", &[("rations", 1, 50)]),
            judged(Vec::new()),
            terms(Some("offer"), &[("rations", 1)], &[("sp", 5)]),
            // "I'll take one" accepts her open terms; the gold piece is only what is handed.
            line_answer(take),
            judged(Vec::new()),
            claiming(terms(Some("accept"), &[("gp", 1)], &[("rations", 1)])),
            // Her reply speaks of the sale just settled.
            line_answer(thanks),
            judged(Vec::new()),
            claiming(terms(None, &[], &[])),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    ration_stall(&host);
    done(
        say(
            &host,
            "wren",
            json!({"text": ask, "asked_by": "produce_vendor"}),
        )
        .await,
    );
    let quoted = done(say(&host, "produce_vendor", json!({"asked_by": "wren"})).await).said;
    assert_eq!(quoted.line, "A travel ration is 50 cp.");
    done(
        say(
            &host,
            "wren",
            json!({"text": take, "asked_by": "produce_vendor"}),
        )
        .await,
    );
    let reply = done(say(&host, "produce_vendor", json!({"asked_by": "wren"})).await).said;
    assert_eq!(
        reply.line, thanks,
        "the sale is recorded, so the line stands"
    );
    assert_eq!(
        host.lines_starting_with("WORLD deal"),
        [
            "WORLD deal_open state=offer owner=produce_vendor with=wren gives=rations*1 \
             gets=silver-piece*5",
            "WORLD deal_settled owner=wren with=produce_vendor gives=silver-piece*5 \
             gets=rations*1 wares_cp=50 coins_cp=50",
        ]
    );
    assert!(
        host.lines_starting_with("INVENTORY deal")[0].starts_with(
            "INVENTORY deal owner=produce_vendor with=wren payer=wren cp=50 handed_cp=100 \
             change_cp=50 "
        ),
        "{:?}",
        host.lines_starting_with("INVENTORY deal")
    );
    assert!(
        host.lines_starting_with("PERSONA deal_unsettled")
            .is_empty()
    );
}

#[tokio::test]
async fn host46_as_judged_a_shop_is_not_repriced_and_an_unrecorded_change_is_worded_again() {
    let take = "I'll take one. Here's a gold piece.";
    let change = "A gold piece for one ration, then. Here's your change.";
    let quote = "Five silver for the ration, if you please.";
    let host = scripted_host_cast(
        &["wren", "produce_vendor"],
        vec![
            line_answer(take),
            judged(Vec::new()),
            // Host 46: no terms open, so the hero's line was no trade.
            claiming(terms(None, &[], &[])),
            line_answer(change),
            judged(Vec::new()),
            // Her line judged an offer with the sides turned: she pays the gold for the ration.
            // Asked again for the sides, the judge puts her ware on her side.
            claiming(terms(Some("offer"), &[("gp", 1)], &[("rations", 1)])),
            claiming(terms(Some("offer"), &[("rations", 1)], &[("sp", 5)])),
            // Worded again: she puts the price, and claims nothing.
            line_answer(quote),
            judged(Vec::new()),
            terms(Some("offer"), &[("rations", 1)], &[("sp", 5)]),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    ration_stall(&host);
    done(
        say(
            &host,
            "wren",
            json!({"text": take, "asked_by": "produce_vendor"}),
        )
        .await,
    );
    let said = done(say(&host, "produce_vendor", json!({"asked_by": "wren"})).await).said;
    assert_eq!(said.line, quote, "a change nobody recorded is never shown");
    assert_eq!(
        host.lines_starting_with("PERSONA deal_unsettled"),
        [
            // The player's own words stand as said; nothing moves for them.
            "PERSONA deal_unsettled id=wren reason=claims_exchange reworded=no",
            "PERSONA deal_unsettled id=produce_vendor reason=claims_exchange reworded=yes",
        ]
    );
    assert!(host.lines_starting_with("WORLD deal_tendered").is_empty());
    assert_eq!(
        host.lines_starting_with("WORLD deal"),
        [
            "WORLD deal_open state=offer owner=produce_vendor with=wren gives=rations*1 \
          gets=silver-piece*5"
        ]
    );
}

#[tokio::test]
async fn host47_a_quote_judged_with_the_keepers_sides_turned_is_put_right_and_the_purchase_settles()
{
    let take = "I'll take one. Here's a gold piece.";
    let host = scripted_host_cast(
        &["wren", "produce_vendor"],
        vec![
            quoting("A travel ration's {price:0}.", &[("rations", 1, 50)]),
            judged(Vec::new()),
            // Host 47: her quote judged as her giving the five silver and getting the ration,
            // and again when asked for the sides.
            terms(Some("offer"), &[("sp", 5)], &[("rations", 1)]),
            terms(Some("offer"), &[("sp", 5)], &[("rations", 1)]),
            line_answer(take),
            judged(Vec::new()),
            claiming(terms(Some("accept"), &[("gp", 1)], &[("rations", 1)])),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    ration_stall(&host);
    done(say(&host, "produce_vendor", json!({"asked_by": "wren"})).await);
    done(
        say(
            &host,
            "wren",
            json!({"text": take, "asked_by": "produce_vendor"}),
        )
        .await,
    );
    assert_eq!(
        host.lines_starting_with("WORLD retry verb=deal"),
        ["WORLD retry verb=deal attempt=2 reason=keeper_sides"]
    );
    assert_eq!(
        host.lines_starting_with("PERSONA deal_sides_normalized"),
        ["PERSONA deal_sides_normalized id=produce_vendor state=offer"]
    );
    assert_eq!(
        host.lines_starting_with("WORLD deal"),
        [
            "WORLD deal_open state=offer owner=produce_vendor with=wren gives=rations*1 \
             gets=silver-piece*5",
            "WORLD deal_settled owner=wren with=produce_vendor gives=silver-piece*5 \
             gets=rations*1 wares_cp=50 coins_cp=50",
        ]
    );
    assert!(
        host.lines_starting_with("INVENTORY deal")[0].starts_with(
            "INVENTORY deal owner=produce_vendor with=wren payer=wren cp=50 handed_cp=100 \
             change_cp=50 "
        ),
        "{:?}",
        host.lines_starting_with("INVENTORY deal")
    );
}

#[tokio::test]
async fn a_refused_deal_answer_is_told_the_goods_each_side_may_name() {
    // Host 48: the keeper's quote named goods it could not hand over (the hero's spear there;
    // more rations than her stock here), and was asked again without the ids it may name.
    let spear = terms(Some("offer"), &[("rations", 20)], &[("sp", 2)]);
    let host = scripted_host_cast(
        &["wren", "produce_vendor"],
        vec![
            quoting("Ten rations come to {price:0}.", &[("rations", 10, 500)]),
            judged(Vec::new()),
            spear,
            terms(Some("offer"), &[("rations", 2)], &[("sp", 2)]),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    ration_stall(&host);
    done(say(&host, "produce_vendor", json!({"asked_by": "wren"})).await);
    let reason = host.facts_of(3)["now"]["retry_reason"].clone();
    assert!(
        reason.as_str().is_some_and(|reason| reason
            .contains("gives names only goods this character holds (rations)")
            && reason.contains("gets names only goods the other person holds (rations)")),
        "{reason}"
    );
}

#[tokio::test]
async fn host49_ten_apples_at_two_silver_are_never_given_free_the_price_told_is_kept() {
    let ask = "Hello! What's your name, and how much for ten apples?";
    let take = "I'll take them. Here's a gold piece.";
    let host = scripted_host_cast(
        &["wren", "produce_vendor"],
        vec![
            line_answer(ask),
            judged(Vec::new()),
            terms(None, &[], &[]),
            quoting(
                "Mara, friend. Ten apples come to {price:0}.",
                &[("apple", 10, 20)],
            ),
            judged(Vec::new()),
            // Host 49: first the hero's spear for the apples, then the price dropped, then put
            // right with the price told kept.
            terms(Some("offer"), &[("apple", 10)], &[("spear", 10), ("sp", 2)]),
            terms(Some("offer"), &[("apple", 10)], &[]),
            terms(Some("offer"), &[("apple", 10)], &[("sp", 2)]),
            line_answer(take),
            judged(Vec::new()),
            claiming(terms(Some("accept"), &[("gp", 1)], &[("apple", 10)])),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    write_persona(&host.directory(), &vendor("wren")).unwrap();
    write_persona(&host.directory(), &vendor("produce_vendor")).unwrap();
    host.running.write_asset(
        "world/inventory/wren.json",
        &json!({"schema": 1, "owner": "wren", "kind": "character", "items": [
            coin("gold-piece", 78, "gp"),
            {"id": "spear", "name": "spear", "look": "a spear", "use": "kept", "count": 1,
             "price_cp": 100}
        ]})
        .to_string(),
    );
    host.running.write_asset(
        "world/inventory/produce_vendor.json",
        &json!({"schema": 1, "owner": "produce_vendor", "kind": "shop", "items": [
            {"id": "apple", "name": "apples", "look": "red apples", "use": "consumed",
             "count": 20, "price_cp": 2},
            coin("copper-piece", 20, "cp"), coin("silver-piece", 20, "sp"),
            coin("gold-piece", 10, "gp")
        ]})
        .to_string(),
    );
    done(
        say(
            &host,
            "wren",
            json!({"text": ask, "asked_by": "produce_vendor"}),
        )
        .await,
    );
    done(say(&host, "produce_vendor", json!({"asked_by": "wren"})).await);
    done(
        say(
            &host,
            "wren",
            json!({"text": take, "asked_by": "produce_vendor"}),
        )
        .await,
    );
    let retries = host.lines_starting_with("WORLD retry verb=deal");
    assert_eq!(retries.len(), 2, "{retries:?}");
    assert!(retries[1].ends_with("reason=one_sided"), "{retries:?}");
    // The price told stays with the judge when it is asked again.
    assert!(
        host.facts_of(7)["now"]["retry_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("keep the money told (2 sp)")),
        "{}",
        host.facts_of(7)
    );
    assert_eq!(
        host.lines_starting_with("WORLD deal"),
        [
            "WORLD deal_open state=offer owner=produce_vendor with=wren gives=apple*10 \
             gets=silver-piece*2",
            "WORLD deal_settled owner=wren with=produce_vendor gives=silver-piece*2 \
             gets=apple*10 wares_cp=20 coins_cp=20",
        ]
    );
    assert!(
        host.lines_starting_with("INVENTORY deal")[0].starts_with(
            "INVENTORY deal owner=produce_vendor with=wren payer=wren cp=20 handed_cp=100 \
             change_cp=80 "
        ),
        "{:?}",
        host.lines_starting_with("INVENTORY deal")
    );
}

#[tokio::test]
async fn host50_buying_a_ration_never_also_uses_one_of_ones_own() {
    let take = "Then I'll take one ration. Here's a gold piece.";
    let host = scripted_host_cast(
        &["wren", "produce_vendor"],
        vec![
            line_answer(take),
            // Host 50: the line's inventory judge read "I'll take one ration" as eating one.
            judged(vec![entry("rations", -1, "used_up", None, &[])]),
            claiming(terms(Some("offer"), &[("gp", 1)], &[("rations", 1)])),
            line_answer("Here's your ration, and {change} back."),
            judged(Vec::new()),
            claiming(terms(Some("accept"), &[("rations", 1)], &[("gp", 1)])),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    ration_stall(&host);
    done(
        say(
            &host,
            "wren",
            json!({"text": take, "asked_by": "produce_vendor"}),
        )
        .await,
    );
    done(say(&host, "produce_vendor", json!({"asked_by": "wren"})).await);
    assert_eq!(
        host.lines_starting_with("PERSONA deal_entries"),
        ["PERSONA deal_entries id=wren deal=offer moved_by_deal=rations:used_up"]
    );
    let rations = crate::world::inventory_of(&host.running.state, "wren")
        .into_iter()
        .find(|item| item.id == "rations")
        .map_or(0, |item| item.count);
    assert_eq!(rations, 11, "ten held and one bought");
    assert!(
        host.instructions_of(3)
            .contains("lists every ware this character sells")
    );
}

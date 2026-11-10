//! Tests of the `move` walk over the real movement routes with scripted model answers: Jev's
//! reading of a line (declared a move or not, and the one second reading) and the design call's
//! refusals, their typed cause and the one second asking.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use df_ai::text::{TextOutcome, TextRequest};
use df_ai::text_fixture::ScriptedBackend;
use serde_json::{Value, json};

use super::cli::{Said, walk};
use super::routes::{DesignReply, DesignRequest, InterpretRequest, path, router_with};
use super::types::{MoveIntent, MoveKind, PartyMember, TransitionStage};
use crate::dev_client::post_json;
use crate::dev_http::DevReply;
use crate::persona::PersonaSummary;
use crate::test_support::{Running, STARTER_FILES, start};
use crate::text::{BackendChoice, TextRuntime};

/// The two lines of the live roam that Jev refused, with the destination a fair reading gives.
const NEXT_TOWN: &str =
    "The next town along the road. I want to see what the late light does to its walls.";
const WINDOW: &str =
    "I'll go see that little window above the open shutter. The late light might catch its edge.";

/// The situation a roaming character says a line in: a goal of going somewhere.
const SITUATION: &str = "You are a travelling painter. Say in one line where you go next.";

struct Scripted {
    running: Running,
    /// The port of the movement routes.
    port: u16,
    calls: Arc<AtomicUsize>,
    asked: Arc<std::sync::Mutex<Vec<TextRequest>>>,
}

impl Scripted {
    fn facts_of(&self, call: usize) -> Value {
        self.asked.lock().unwrap()[call].facts.clone()
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn interpreted_lines(&self) -> Vec<String> {
        self.running
            .state
            .log()
            .after_sequence(0, usize::MAX)
            .into_iter()
            .map(|line| line.text)
            .filter(|text| text.starts_with("MOVE interpreted"))
            .collect()
    }
}

/// A host with the movement routes over a text backend that answers with `answers`, in order.
async fn scripted(answers: Vec<Value>) -> Scripted {
    let running = start(&STARTER_FILES).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
    let script = ScriptedBackend::new(
        answers.into_iter().map(TextOutcome::Answered),
        Arc::clone(&calls),
    )
    .recording(Arc::clone(&asked));
    let recorder = Arc::clone(&running.state);
    let text = Arc::new(TextRuntime::new(
        BackendChoice::Scripted(Arc::new(script), None),
        Arc::new(move |line| recorder.record_line(line)),
    ));
    let app = router_with(Arc::clone(&running.state), text);
    let listener = crate::server::bind(0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Scripted {
        running,
        port,
        calls,
        asked,
    }
}

fn danny() -> Vec<PersonaSummary> {
    vec![PersonaSummary {
        id: "danny".to_owned(),
        display_name: "Danny".to_owned(),
        job: "painter".to_owned(),
    }]
}

fn no_destination(reason: &str) -> Value {
    json!({
        "reading": "needs_clarification", "kind": null, "who": [], "to": null,
        "unclear": "destination", "reason": reason
    })
}

fn walks_to(to: &str) -> Value {
    json!({
        "reading": "move", "kind": "walk", "who": ["danny"], "to": to,
        "unclear": null, "reason": null
    })
}

/// The report of reading `line` said by Danny toward going somewhere, stopping after Jev.
async fn danny_says(host: &Scripted, line: &str) -> String {
    let said = Said {
        speaker: "danny",
        situation: SITUATION,
    };
    walk(
        host.port,
        line,
        Some(TransitionStage::Interpreting),
        &danny(),
        Some(said),
        None,
    )
    .await
    .expect("the reading is carried through")
}

/// The report of reading a line nobody declared anything about, stopping after Jev.
async fn free_line(host: &Scripted, line: &str) -> String {
    walk(
        host.port,
        line,
        Some(TransitionStage::Interpreting),
        &danny(),
        None,
        None,
    )
    .await
    .expect("the reading is carried through")
}

#[tokio::test]
async fn a_line_refused_for_no_destination_is_read_again_with_its_reason_and_moves() {
    for (line, to) in [
        (NEXT_TOWN, "the next town along the road"),
        (WINDOW, "the little window above the open shutter"),
    ] {
        let reason = "The line states a wish to see something but no place to go.";
        let host = scripted(vec![no_destination(reason), walks_to(to)]).await;

        let report = danny_says(&host, line).await;

        assert_eq!(host.calls(), 2, "{line}");
        assert!(
            report.contains(&format!("reading: move kind=walk who=danny to={to}")),
            "{report}"
        );
        let retry = report
            .lines()
            .find(|text| text.contains("stage=interpreting") && text.contains("detail=retry"))
            .unwrap_or_else(|| panic!("a retry record in the trace:\n{report}"));
        assert!(
            retry.contains(&format!(
                "reason=the movement names no destination: {reason}"
            )),
            "{retry}"
        );
        // The first reading was a plain one; the second carries the first answer's reason.
        assert_eq!(host.facts_of(0)["declared"], "move");
        assert_eq!(host.facts_of(0)["refused_before"], Value::Null);
        assert_eq!(host.facts_of(1)["declared"], "move");
        assert_eq!(
            host.facts_of(1)["refused_before"],
            json!({"unclear": "destination", "reason": reason})
        );
        assert_eq!(host.facts_of(1)["line"], line);
        assert_eq!(host.facts_of(1)["context"], SITUATION);
        let audit = host.interpreted_lines();
        assert_eq!(audit.len(), 2, "{audit:?}");
        assert!(audit[0].contains("reading=needs_clarification unclear=destination declared=move"));
        assert!(audit[0].contains("second_reading=false"), "{}", audit[0]);
        assert!(audit[1].contains("reading=move unclear=- declared=move second_reading=true"));
    }
}

#[tokio::test]
async fn the_trace_keeps_the_time_of_each_attempt() {
    let host = scripted(vec![no_destination("no place"), walks_to("the road")]).await;

    let report = danny_says(&host, NEXT_TOWN).await;

    let times: Vec<(String, u64)> = report
        .lines()
        .filter(|text| text.starts_with("TRANSITION "))
        .map(|text| {
            let stage = text
                .split("stage=")
                .nth(1)
                .unwrap()
                .split(' ')
                .next()
                .unwrap();
            let at_ms = text
                .split("at_ms=")
                .nth(1)
                .unwrap()
                .split(' ')
                .next()
                .unwrap();
            (stage.to_owned(), at_ms.parse().unwrap())
        })
        .collect();
    let stages: Vec<&str> = times.iter().map(|(stage, _)| stage.as_str()).collect();
    assert_eq!(stages, ["interpreting", "interpreting", "designing"]);
    assert!(
        times.windows(2).all(|pair| pair[0].1 <= pair[1].1),
        "{times:?}"
    );
}

#[tokio::test]
async fn a_line_that_names_nothing_is_asked_again_once_and_then_still_refused() {
    let reason = "Nothing in the line is a place.";
    let host = scripted(vec![
        no_destination(reason),
        no_destination(reason),
        walks_to("never asked for"),
    ])
    .await;

    let report = danny_says(&host, "Hm.").await;

    assert_eq!(host.calls(), 2, "one second reading and no more");
    assert!(
        report.contains(&format!(
            "not moved: the movement names no destination: {reason}"
        )),
        "{report}"
    );
    assert_eq!(
        report.matches("detail=retry").count(),
        1,
        "one retry record:\n{report}"
    );
    assert!(
        report.contains("failed key=needs_clarification"),
        "{report}"
    );
}

#[tokio::test]
async fn a_line_nobody_declared_is_read_once_and_a_refusal_stands() {
    let host = scripted(vec![
        no_destination("no place"),
        walks_to("never asked for"),
    ])
    .await;

    let report = free_line(&host, "we go").await;

    assert_eq!(host.calls(), 1);
    assert!(report.contains("not moved: the movement names no destination: no place"));
    assert!(!report.contains("retry"), "{report}");
    assert_eq!(host.facts_of(0)["declared"], "unknown");
    assert_eq!(host.facts_of(0)["speaker"], Value::Null);
}

#[tokio::test]
async fn only_a_missing_destination_is_read_again() {
    let other = json!({
        "reading": "needs_clarification", "kind": null, "who": [], "to": null,
        "unclear": "other", "reason": "The line breaks off."
    });
    let not_a_move = json!({
        "reading": "not_a_move", "kind": null, "who": [], "to": null,
        "unclear": null, "reason": null
    });
    for (answer, shown) in [
        (
            other,
            "not moved: the line cannot be read as one movement: The line breaks off.",
        ),
        (not_a_move, "not moved: the line is not a movement"),
    ] {
        let host = scripted(vec![answer, walks_to("never asked for")]).await;

        let report = danny_says(&host, "How is the weather?").await;

        assert_eq!(host.calls(), 1, "{shown}");
        assert!(report.contains(shown), "{report}");
    }
}

#[test]
fn an_interpret_request_without_a_declaration_is_unknown() {
    let request: InterpretRequest = serde_json::from_value(json!({
        "line": "we walk to the square",
        "party": [{"id": "danny", "display_name": "Danny"}]
    }))
    .unwrap();
    assert_eq!(request.declared.key(), "unknown");
    assert!(request.refused_before.is_none());

    let refused: Result<InterpretRequest, _> = serde_json::from_value(json!({
        "line": "x", "party": [], "declared": "maybe"
    }));
    assert!(
        refused.is_err(),
        "a declaration outside the closed list is refused"
    );
}

fn design_request() -> DesignRequest {
    DesignRequest {
        intent: MoveIntent {
            kind: MoveKind::Walk,
            who: vec!["danny".to_owned()],
            to: "the market".to_owned(),
            line: "I'll head for the market next.".to_owned(),
        },
        party: vec![PartyMember {
            id: "danny".to_owned(),
            display_name: "Danny".to_owned(),
        }],
        context: Some(SITUATION.to_owned()),
        model: None,
    }
}

/// A design of the market that the checks accept.
fn market_design() -> Value {
    json!({
        "place": "a market square", "mood": "busy", "elements": ["awnings"],
        "things": [{"id": "fountain", "name": "the fountain", "look": "worn grey stone"}],
        "shown_items": [], "residents": [],
        "placements": [{"character": "danny", "depth": "foreground", "side": "left",
                        "doing": "walks in"}]
    })
}

/// A design whose thing id is no place id: the likeliest cause of a refusal in a market.
fn design_with_bad_thing_id() -> Value {
    let mut design = market_design();
    design["things"][0]["id"] = json!("Market Cross!");
    design
}

fn lines_starting_with(host: &Scripted, start: &str) -> Vec<String> {
    host.running
        .state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with(start))
        .collect()
}

async fn design(host: &Scripted) -> DesignReply {
    post_json(host.port, path::MOVE_DESIGN, &design_request())
        .await
        .expect("the design route answers")
}

#[tokio::test]
async fn a_refused_design_is_asked_for_once_more_with_the_rule_it_broke() {
    let host = scripted(vec![design_with_bad_thing_id(), market_design()]).await;

    let DevReply::Done { value } = design(&host).await else {
        panic!("the second answer stands");
    };

    assert_eq!(value.place, "a market square");
    assert_eq!(host.calls(), 2);
    assert_eq!(host.facts_of(0)["refused_before"], Value::Null);
    let refused = &host.facts_of(1)["refused_before"];
    assert_eq!(refused["error"], "bad_thing");
    assert!(refused["detail"].as_str().unwrap().contains("a thing id"));
    let retry = lines_starting_with(&host, "MOVE retry");
    assert_eq!(retry.len(), 1, "{retry:?}");
    assert!(retry[0].starts_with("MOVE retry verb=design reason=bad_thing detail="));
    let designed = lines_starting_with(&host, "MOVE designed");
    assert!(designed[0].contains("retried=true"), "{designed:?}");
    assert!(lines_starting_with(&host, "MOVE refused").is_empty());
    let everything: String = lines_starting_with(&host, "").join("\n");
    assert!(
        !everything.contains("Market Cross"),
        "the model's own text is in no log line"
    );
}

#[tokio::test]
async fn a_design_refused_twice_is_refused_with_its_typed_cause_after_two_askings_only() {
    let host = scripted(vec![
        design_with_bad_thing_id(),
        design_with_bad_thing_id(),
        market_design(),
    ])
    .await;

    let DevReply::Rejected { error, message } = design(&host).await else {
        panic!("two refused answers lose the move");
    };

    assert_eq!(error.key(), "answer_refused");
    assert!(
        message.contains("a thing id is not a place id"),
        "{message}"
    );
    assert_eq!(host.calls(), 2, "one second asking and no more");
    let refused = lines_starting_with(&host, "MOVE refused");
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(
        refused[0]
            .starts_with("MOVE refused verb=design error=answer_refused cause=bad_thing detail="),
        "{}",
        refused[0]
    );
    assert!(!refused[0].contains("Market Cross"), "{}", refused[0]);
    assert_eq!(lines_starting_with(&host, "MOVE retry").len(), 1);
}

#[tokio::test]
async fn a_walk_whose_design_stays_refused_names_the_cause_in_its_trace_and_failure() {
    let host = scripted(vec![
        walks_to("the market"),
        design_with_bad_thing_id(),
        design_with_bad_thing_id(),
    ])
    .await;
    let said = Said {
        speaker: "danny",
        situation: SITUATION,
    };

    let error = walk(
        host.port,
        "I'll head for the market next.",
        Some(TransitionStage::Designing),
        &danny(),
        Some(said),
        None,
    )
    .await
    .expect_err("the design is refused twice");

    let shown = error.to_string();
    assert!(
        shown.contains("stage=designing") && shown.contains("failed key=answer_refused message="),
        "{shown}"
    );
    assert!(
        shown.ends_with(
            "the transition failed: answer_refused: the model's design was refused: a thing id is \
             not a place id (lowercase letters, digits, '_' or '-'), or repeats another thing's"
        ),
        "{shown}"
    );
    assert!(!shown.contains("Market Cross"), "{shown}");
}

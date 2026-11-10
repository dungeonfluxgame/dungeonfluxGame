//! Tests of the `move` walk over the real movement routes with scripted model answers: Jev's
//! reading of a line (declared a move or not, and the one second reading) and the design call's
//! refusals, their typed cause and the one second asking.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use df_ai::text::{TextOutcome, TextRequest};
use df_ai::text_fixture::ScriptedBackend;
use serde_json::{Value, json};

use super::cli::{MoveCommand, MoveOptions, Said, parse, walk};
use super::routes::{
    DesignReply, DesignRequest, InterpretRequest, path, router_with_design_deadline,
};
use super::types::{
    MoveIntent, MoveKind, PartyMember, SceneDesign, TownRole, TransitionStage, WalkAsk,
};
use super::walk::Impulse;
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

    fn schema_of(&self, call: usize) -> Value {
        self.asked.lock().unwrap()[call].output_schema.clone()
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
    scripted_asking_for(answers, Duration::from_secs(60)).await
}

/// [`scripted`] where the design call asks again after refused answers for `design_deadline`.
async fn scripted_asking_for(answers: Vec<Value>, design_deadline: Duration) -> Scripted {
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
    let app = router_with_design_deadline(Arc::clone(&running.state), text, design_deadline);
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
        &MoveOptions::default(),
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
        &MoveOptions::default(),
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
        walk: None,
    }
}

/// A design of the market that the checks accept.
fn market_design() -> Value {
    json!({
        "place": "a market square", "mood": "busy", "feature": "fountain_square",
        "elements": ["awnings"],
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
    assert_eq!(host.facts_of(0)["refused_before"], json!([]));
    let refused = &host.facts_of(1)["refused_before"][0];
    assert_eq!(refused["error"], "bad_thing");
    assert!(refused["detail"].as_str().unwrap().contains("a thing id"));
    let retry = lines_starting_with(&host, "MOVE retry");
    assert_eq!(retry.len(), 1, "{retry:?}");
    assert!(retry[0].starts_with("MOVE retry verb=design attempt=2 reason=bad_thing detail="));
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
async fn a_design_still_refused_when_the_deadline_has_passed_is_refused_with_its_typed_cause() {
    let host = scripted_asking_for(
        vec![design_with_bad_thing_id(), market_design()],
        Duration::ZERO,
    )
    .await;

    let DevReply::Rejected { error, message } = design(&host).await else {
        panic!("a refused answer past the deadline loses the move");
    };

    assert_eq!(error.key(), "answer_refused");
    assert!(
        message.contains("a thing id is not a place id"),
        "{message}"
    );
    assert_eq!(host.calls(), 1, "no asking after the deadline");
    let refused = lines_starting_with(&host, "MOVE refused");
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(
        refused[0]
            .starts_with("MOVE refused verb=design error=answer_refused cause=bad_thing detail="),
        "{}",
        refused[0]
    );
    assert!(!refused[0].contains("Market Cross"), "{}", refused[0]);
    assert!(lines_starting_with(&host, "MOVE retry").is_empty());
}

#[tokio::test]
async fn a_walk_whose_design_stays_refused_names_the_cause_in_its_trace_and_failure() {
    let host = scripted_asking_for(
        vec![walks_to("the market"), design_with_bad_thing_id()],
        Duration::ZERO,
    )
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
        &MoveOptions::default(),
    )
    .await
    .expect_err("the design is refused past the deadline");

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

const TOWN_LOOK: &str = "half-timbered houses with steep red tile roofs, narrow cobbled lanes, \
                         low evening light";

fn two_sights() -> Value {
    json!([
        {"id": "fountain_square", "sight": "a stone fountain in a small square", "pose": "walking"},
        {"id": "shopfronts", "sight": "a row of timber shopfronts", "pose": "pausing"}
    ])
}

/// A design of the market on foot: the sights on the way and, when it is given, the town's look.
fn walking_design(stages: Value, town_look: Option<&str>) -> Value {
    let mut design = market_design();
    design["stages"] = stages;
    if let Some(look) = town_look {
        design["town_look"] = json!(look);
    }
    design
}

/// The design of a move painted as `move_id` that asks for `stages` sights and plays `town`.
async fn designed_walk(
    host: &Scripted,
    move_id: &str,
    stages: usize,
    town: TownRole,
) -> DesignReply {
    let request = DesignRequest {
        walk: Some(WalkAsk {
            stages,
            town,
            move_id: move_id.to_owned(),
        }),
        ..design_request()
    };
    post_json(host.port, path::MOVE_DESIGN, &request)
        .await
        .expect("the design route answers")
}

fn designed(reply: DesignReply) -> SceneDesign {
    match reply {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, message } => panic!("refused: {} {message}", error.key()),
    }
}

#[tokio::test]
async fn a_move_that_seeds_a_town_keeps_its_look_and_later_moves_in_it_reuse_it() {
    let host = scripted(vec![
        walking_design(two_sights(), Some(TOWN_LOOK)),
        walking_design(two_sights(), None),
        walking_design(json!([]), None),
        walking_design(json!([]), Some("grey stone houses under slate roofs")),
    ])
    .await;
    let state = Arc::clone(&host.running.state);

    // The first place of a town move seeds the town from the design's look.
    let first = designed(designed_walk(&host, "roam-01", 2, TownRole::Seeds).await);
    assert_eq!(first.stages.len(), 2);
    assert_eq!(first.stages[0].id, "fountain_square");
    assert_eq!(first.town_look.as_deref(), Some(TOWN_LOOK));
    let town = crate::world::town_of(&state).expect("the town is kept");
    assert_eq!(town.id, "town-roam-01");
    assert_eq!(town.created_at, "roam-01");
    assert_eq!(town.look, TOWN_LOOK);
    assert_eq!(
        host.facts_of(0)["route"],
        json!({"stages_wanted": 2, "town": null})
    );
    let schema = host.schema_of(0);
    assert_eq!(schema["properties"]["stages"]["maxItems"], json!(2));
    assert!(
        schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("town_look"))
    );

    // The next move in the town is told its look, asks for none, and answers with it.
    let second = designed(designed_walk(&host, "roam-02", 2, TownRole::Seeds).await);
    assert_eq!(second.town_look.as_deref(), Some(TOWN_LOOK));
    assert_eq!(host.facts_of(1)["route"]["town"], json!(TOWN_LOOK));
    let schema = host.schema_of(1);
    assert!(
        !schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("town_look"))
    );
    assert_eq!(crate::world::town_of(&state).unwrap().id, "town-roam-01");

    // A move with no sights is still in the town.
    let third = designed(designed_walk(&host, "roam-03", 0, TownRole::Within).await);
    assert!(third.stages.is_empty());
    assert_eq!(third.town_look.as_deref(), Some(TOWN_LOOK));
    let schema = host.schema_of(2);
    assert!(schema["properties"].get("stages").is_none());

    // Leaving by the road ends the town, and the next town move seeds a new one.
    let left = designed(designed_walk(&host, "roam-04", 1, TownRole::Leaves).await);
    assert_eq!(left.town_look, None);
    assert_eq!(crate::world::town_of(&state), None);
    assert_eq!(host.facts_of(3)["route"]["town"], Value::Null);

    let designs = lines_starting_with(&host, "MOVE designed");
    let towns: Vec<&str> = designs
        .iter()
        .map(|line| {
            line.split("town=")
                .nth(1)
                .unwrap()
                .split(' ')
                .next()
                .unwrap()
        })
        .collect();
    assert_eq!(towns, ["seeded", "reused", "reused", "left"]);
    assert_eq!(
        lines_starting_with(&host, "WORLD town_"),
        [
            "WORLD town_seeded id=town-roam-01 created_at=roam-01",
            "WORLD town_left id=town-roam-01"
        ]
    );
}

#[tokio::test]
async fn more_sights_than_the_move_asked_for_are_refused_and_asked_again_until_they_fit() {
    let three = json!([
        {"id": "one", "sight": "a well", "pose": "walking"},
        {"id": "two", "sight": "an arch", "pose": "walking"},
        {"id": "three", "sight": "a bridge", "pose": "pausing"}
    ]);
    let host = scripted(vec![
        walking_design(three.clone(), Some(TOWN_LOOK)),
        walking_design(three, Some(TOWN_LOOK)),
        walking_design(two_sights(), Some(TOWN_LOOK)),
    ])
    .await;

    let DevReply::Done { value } = designed_walk(&host, "roam-01", 2, TownRole::Seeds).await else {
        panic!("the third answer fits");
    };

    assert_eq!(value.stages.len(), 2);
    assert_eq!(host.calls(), 3);
    let refused = host.facts_of(2)["refused_before"].clone();
    assert_eq!(refused.as_array().map(Vec::len), Some(2));
    assert_eq!(refused[1]["error"], "too_many_stages");
    assert!(refused[1]["detail"].as_str().unwrap().contains("at most 2"));
}

#[tokio::test]
async fn more_sights_than_the_move_asked_for_past_the_deadline_lose_the_move() {
    let three = json!([
        {"id": "one", "sight": "a well", "pose": "walking"},
        {"id": "two", "sight": "an arch", "pose": "walking"},
        {"id": "three", "sight": "a bridge", "pose": "pausing"}
    ]);
    let host =
        scripted_asking_for(vec![walking_design(three, Some(TOWN_LOOK))], Duration::ZERO).await;

    let DevReply::Rejected { error, message } =
        designed_walk(&host, "roam-01", 2, TownRole::Seeds).await
    else {
        panic!("a refused answer past the deadline loses the move");
    };

    assert_eq!(error.key(), "answer_refused");
    assert!(message.contains("at most 2"), "{message}");
    assert_eq!(host.calls(), 1);
    assert_eq!(crate::world::town_of(&host.running.state), None);
    assert!(lines_starting_with(&host, "WORLD town_").is_empty());
}

#[tokio::test]
async fn a_move_that_seeds_a_town_without_its_look_is_refused() {
    let host = scripted_asking_for(vec![walking_design(two_sights(), None)], Duration::ZERO).await;

    let DevReply::Rejected { message, .. } =
        designed_walk(&host, "roam-01", 2, TownRole::Seeds).await
    else {
        panic!("a town needs its look");
    };

    assert!(message.contains("town_look is missing"), "{message}");
    assert_eq!(crate::world::town_of(&host.running.state), None);
}

#[tokio::test]
async fn a_move_that_asked_for_no_walk_takes_no_sights() {
    let host = scripted(vec![
        walking_design(two_sights(), None),
        walking_design(json!([]), None),
    ])
    .await;

    let plain = designed(design(&host).await);

    assert!(plain.stages.is_empty(), "the second answer stands");
    assert_eq!(host.calls(), 2);
    assert_eq!(host.facts_of(0).get("route"), None);
    assert_eq!(
        host.facts_of(1)["refused_before"][0]["error"],
        "too_many_stages"
    );
}

fn words(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_owned).collect()
}

fn act_options(options: &str) -> Result<MoveOptions, crate::command::CommandError> {
    match parse(&words(&format!(
        "act danny {options} we go to the next place"
    )))? {
        MoveCommand::Act {
            options, situation, ..
        } => {
            assert_eq!(situation, "we go to the next place");
            Ok(options)
        }
        other => panic!("not an act: {other:?}"),
    }
}

#[test]
fn the_walk_options_come_before_the_text_of_a_move() {
    let options = act_options("plate=roam-01 walk=2 town=seeds hold=6.5 impulse=walk:300").unwrap();

    assert_eq!(options.plate.as_deref(), Some("roam-01"));
    assert_eq!(options.walk.stages, 2);
    assert_eq!(options.walk.town, Some(TownRole::Seeds));
    assert_eq!(options.walk.hold, Some(Duration::from_millis(6_500)));
    assert_eq!(
        options.walk.impulse,
        Some(Impulse {
            kind: "walk".to_owned(),
            permille: 300
        })
    );
    assert_eq!(act_options("").unwrap(), MoveOptions::default());
}

#[test]
fn a_walk_option_outside_its_words_is_refused() {
    for refused in [
        "walk=many",
        "walk=-1",
        "town=village",
        "hold=4.9",
        "hold=soon",
        "hold=NaN",
        "impulse=walk",
        "impulse=:300",
        "impulse=walk:1001",
        "plate=roam.01.png",
    ] {
        assert!(act_options(refused).is_err(), "{refused} is refused");
    }
}

/// Makes the party have been to a place with `feature`, through the world's arrive route.
async fn been_to(host: &Scripted, id: &str, feature: &str) {
    let request = crate::world::request::ArriveRequest {
        place: df_content::world_bible::Place {
            id: id.to_owned(),
            name: format!("the place {id}"),
            description: "Mood: quiet.".to_owned(),
            stalls: Vec::new(),
            things: Vec::new(),
        },
        feature: Some(feature.to_owned()),
    };
    let reply: crate::world::request::WorldReply<crate::world::request::PlaceSet> =
        post_json(host.running.port, "/dev/world/arrive", &request)
            .await
            .expect("the world answers");
    assert!(matches!(reply, DevReply::Done { .. }), "arrived");
}

#[tokio::test]
async fn a_design_that_repeats_a_places_feature_is_refused_and_asked_again() {
    let mut repeats = market_design();
    repeats["feature"] = json!("open_road");
    let mut distinct = market_design();
    distinct["feature"] = json!("ford");
    let host = scripted(vec![repeats, distinct]).await;
    been_to(&host, "roam-01", "open_road").await;

    let DevReply::Done { value } = design(&host).await else {
        panic!("the distinct second answer stands");
    };

    assert_eq!(value.feature, "ford");
    assert_eq!(host.calls(), 2);
    assert_eq!(
        host.facts_of(0)["previously_designed"],
        json!([{"name": "the place roam-01", "feature": "open_road"}])
    );
    assert_eq!(
        host.facts_of(1)["previously_designed"],
        host.facts_of(0)["previously_designed"]
    );
    assert_eq!(
        host.facts_of(1)["refused_before"][0]["error"],
        "repeated_feature"
    );
    let retry = lines_starting_with(&host, "MOVE retry");
    assert!(
        retry[0].starts_with("MOVE retry verb=design attempt=2 reason=repeated_feature detail="),
        "{retry:?}"
    );
    let designed = lines_starting_with(&host, "MOVE designed");
    assert!(
        designed[0].contains("feature=ford retried=true"),
        "{designed:?}"
    );
}

#[tokio::test]
async fn a_distinct_feature_passes_at_the_first_asking_and_a_design_is_not_kept_until_arrival() {
    let mut distinct = market_design();
    distinct["feature"] = json!("ford");
    let host = scripted(vec![distinct.clone(), distinct]).await;
    been_to(&host, "roam-01", "open_road").await;

    let DevReply::Done { value } = design(&host).await else {
        panic!("a distinct feature stands");
    };
    assert_eq!(value.feature, "ford");
    assert_eq!(host.calls(), 1);

    // Designing alone keeps nothing: the same move designed again may use the same feature.
    let DevReply::Done { value } = design(&host).await else {
        panic!("a design not yet arrived at repeats nothing");
    };
    assert_eq!(value.feature, "ford");
    assert_eq!(host.calls(), 2);
    assert_eq!(crate::world::visited_of(&host.running.state).len(), 1);
}

/// A design that breaks the rule `breaker` changes.
fn design_breaking(breaker: impl Fn(&mut Value)) -> Value {
    let mut design = market_design();
    breaker(&mut design);
    design
}

#[tokio::test]
async fn refusals_keep_coming_back_with_all_of_them_until_an_answer_stands() {
    let bad_resident = design_breaking(|design| {
        design["residents"] = json!([{
            "id": "fruit-vendor", "display_name": "Mara", "job": "vendor", "look": "stout",
            "depth": "middle", "side": "right", "doing": "arranges pears"
        }]);
    });
    let resident_owner = design_breaking(|design| {
        design["shown_items"] = json!([{"owner": "fruit_vendor", "item": "easel"}]);
    });
    let host = scripted(vec![
        bad_resident.clone(),
        resident_owner,
        bad_resident,
        market_design(),
    ])
    .await;

    let DevReply::Done { value } = design(&host).await else {
        panic!("the fourth answer stands");
    };

    assert_eq!(value.place, "a market square");
    assert_eq!(host.calls(), 4, "no count caps the askings");
    let errors = |call: usize| -> Vec<String> {
        host.facts_of(call)["refused_before"]
            .as_array()
            .unwrap()
            .iter()
            .map(|refused| refused["error"].as_str().unwrap().to_owned())
            .collect()
    };
    assert!(errors(0).is_empty());
    assert_eq!(errors(1), ["bad_resident"]);
    assert_eq!(errors(2), ["bad_resident", "invalid_shown_item"]);
    assert_eq!(
        errors(3),
        ["bad_resident", "invalid_shown_item", "bad_resident"]
    );
    let retry = lines_starting_with(&host, "MOVE retry");
    assert_eq!(retry.len(), 3, "{retry:?}");
    for (index, line) in retry.iter().enumerate() {
        assert!(
            line.starts_with(&format!("MOVE retry verb=design attempt={} ", index + 2)),
            "{line}"
        );
    }
    assert!(lines_starting_with(&host, "MOVE refused").is_empty());
    let designed = lines_starting_with(&host, "MOVE designed");
    assert!(designed[0].contains("retried=true"), "{designed:?}");
}

#[tokio::test]
async fn a_service_that_gives_no_answer_ends_the_askings_with_its_own_error() {
    let host = scripted(vec![design_with_bad_thing_id(), design_with_bad_thing_id()]).await;

    let DevReply::Rejected { error, .. } = design(&host).await else {
        panic!("a script that runs out ends the askings");
    };

    assert_eq!(error.key(), "text_service");
    assert_eq!(lines_starting_with(&host, "MOVE retry").len(), 2);
    assert!(host.calls() >= 3, "{}", host.calls());
}

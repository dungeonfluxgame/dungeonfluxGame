//! One whole check on a virtual clock: the order of the call, the roll, the card and the result,
//! and the times each step happens at.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::Value;

use super::fixtures::{
    Table, VirtualClock, Waited, athletics_answer, dice_showing, judge_request, line_answer,
    routes_over, routes_speaking, table, trivial_answer,
};
use super::flow::run_check;
use super::limits::{HEARD_OUT_BASE_MS, RESULT_LEAD_MS};
use super::request::{
    CheckError, CheckRouteError, CheckRun, CheckRunRequest, Initiator, LineVoice,
};
use crate::cast::routes::line_rest_ms;
use crate::persona::request::PersonaError;
use crate::speech::fixtures::FakePort;

const BEFORE: &str = "The rock is cold under your hands.";
const AFTER: &str = "The ledge holds, and so do you.";
const CALLED_NUMBERS: &str = "Athletics, Strength. Difficulty fifteen. Roll it.";
const RESULT_NUMBERS: &str = "Fourteen on the die, plus three for Strength, plus two for \
                              proficiency. Nineteen against fifteen. A success.";

/// The scripted answers of a run that calls a roll: the judge's, then one for each of the two
/// lines, which either may take, since the lines are worded together.
fn answers() -> Vec<Value> {
    std::iter::once(athletics_answer())
        .chain(std::iter::repeat_n(line_answer(BEFORE, AFTER), 2))
        .collect()
}

fn request(action: &str) -> CheckRunRequest {
    CheckRunRequest {
        judge: judge_request(Initiator::Player, Some(action)),
        speak: true,
        inspiration: None,
        told: None,
    }
}

fn rest(line: &str) -> u64 {
    line_rest_ms(HEARD_OUT_BASE_MS, line.chars().count())
}

fn kinds_of(table: &Table) -> Vec<String> {
    table
        .lines_starting_with("CHECK ")
        .iter()
        .filter_map(|line| line.split_whitespace().nth(1).map(str::to_owned))
        .collect()
}

async fn run(table: &Table, faces: &[u64], request: &CheckRunRequest) -> (CheckRun, VirtualClock) {
    let routes = routes_over(table, dice_showing(faces));
    let clock = VirtualClock::over(table.app());
    let run = run_check(&routes, &clock, request)
        .await
        .expect("the check runs");
    (run, clock)
}

fn card_is_up(table: &Table) -> bool {
    table
        .app()
        .playground()
        .snapshot()
        .scene
        .roll
        .set_value()
        .is_some()
}

#[tokio::test]
async fn a_check_is_called_rolled_shown_read_and_cleared_in_that_order_at_these_times() {
    let table = table(answers()).await;
    let (run, clock) = run(&table, &[14], &request("I set my hands to the rock.")).await;
    let rolled = run.rolled.as_ref().expect("a roll was called");

    let called_line = format!("{BEFORE} {CALLED_NUMBERS}");
    let result_line = format!("{RESULT_NUMBERS} {AFTER}");
    assert_eq!(rolled.called_line, called_line);
    assert_eq!(rolled.result_line, result_line);

    let shown_ms = rest(&called_line);
    let result_start_ms = shown_ms + RESULT_LEAD_MS;
    let result_end_ms = result_start_ms + rest(&result_line);
    let min_hold_ms = 600 + rest(&result_line) + 2_000;
    let cleared_ms = result_start_ms + min_hold_ms + 400;
    let times = &rolled.times;
    assert_eq!(
        times.drawn_ms, 0,
        "the dice are drawn before the call is spoken"
    );
    assert_eq!(times.called_ms, 0);
    assert_eq!(
        times.shown_ms, shown_ms,
        "the card waits for the call to be heard out"
    );
    assert_eq!(
        times.result_start_ms, result_start_ms,
        "the result waits for the sum"
    );
    assert_eq!(times.result_end_ms, result_end_ms);
    assert_eq!(times.min_hold_ms, min_hold_ms);
    assert_eq!(times.cleared_ms, cleared_ms);
    assert_eq!(times.hold_ms, min_hold_ms + 400);
    assert!(
        times.cleared_ms - times.result_end_ms >= 2_000,
        "the card stays two seconds past the end of the result line"
    );
    assert!(
        times.cleared_ms - (times.shown_ms + RESULT_LEAD_MS) >= 6_000,
        "and never under six seconds"
    );

    assert_eq!(
        kinds_of(&table),
        [
            "called",
            "read_len",
            "read_len",
            "rolled",
            "shown",
            "result_read",
            "cleared"
        ]
    );
    let lines = table.lines_starting_with("CHECK ");
    assert_eq!(
        lines[0],
        "CHECK called at_ms=0 kind=ability_check skill=athletics dc=15 voice=text_only"
    );
    let tenths = |line: &str| line.chars().count() * 10 / 14;
    assert_eq!(
        lines[1],
        format!(
            "CHECK read_len moment=check_called chars={} est_s={}.{}",
            called_line.chars().count(),
            tenths(&called_line) / 10,
            tenths(&called_line) % 10
        )
    );
    assert_eq!(
        lines[2],
        format!(
            "CHECK read_len moment=check_result chars={} est_s={}.{}",
            result_line.chars().count(),
            tenths(&result_line) / 10,
            tenths(&result_line) % 10
        )
    );
    assert_eq!(
        lines[3],
        format!(
            "CHECK rolled at_ms={shown_ms} drawn_ms=0 dice=1 total=19 outcome=success natural=none"
        )
    );
    assert!(
        lines[4].starts_with(&format!("CHECK shown at_ms={shown_ms} id=check-athletics-")),
        "{}",
        lines[4]
    );
    assert!(
        lines[4].ends_with(&format!("min_hold_ms={min_hold_ms}")),
        "{}",
        lines[4]
    );
    assert_eq!(
        lines[5],
        format!(
            "CHECK result_read start_ms={result_start_ms} end_ms={result_end_ms} voice=text_only"
        )
    );
    assert_eq!(
        lines[6],
        format!(
            "CHECK cleared at_ms={cleared_ms} hold_ms={}",
            min_hold_ms + 400
        )
    );

    assert_eq!(
        clock.waits(),
        [
            Waited {
                until_ms: shown_ms,
                card_up: false,
                narration: Some(called_line.clone())
            },
            Waited {
                until_ms: result_start_ms,
                card_up: true,
                narration: Some(called_line)
            },
            Waited {
                until_ms: result_end_ms,
                card_up: true,
                narration: Some(result_line)
            },
            Waited {
                until_ms: cleared_ms,
                card_up: true,
                narration: Some(rolled.result_line.clone())
            },
        ],
        "the call is on screen before the card, the card is up while the result is read and held"
    );
    assert!(!card_is_up(&table), "the card is cleared at the end");
}

#[tokio::test]
async fn the_card_shows_the_servers_roll_and_the_dice_were_drawn_once_from_the_port() {
    let table = table(answers()).await;
    let dice = dice_showing(&[14]);
    let routes = routes_over(&table, Arc::clone(&dice));
    let clock = VirtualClock::over(table.app());
    let run = run_check(&routes, &clock, &request("I climb."))
        .await
        .expect("the check runs");
    let view = run.rolled.expect("a roll was called").view;
    assert_eq!(view.title, "Athletics (Strength)");
    assert_eq!(
        (view.dice[0].sides, view.dice[0].face, view.total),
        (20, 14, 19)
    );
    assert_eq!(
        view.outcome,
        Some(df_playground::fixture::RollOutcome::Success)
    );
    let drawn = dice.records();
    assert_eq!(drawn.len(), 1, "one d20 was drawn");
    assert_eq!(
        (drawn[0].draw.sides().get(), drawn[0].draw.value()),
        (20, 14)
    );
    assert!(
        table.lines_starting_with("RULES").is_empty()
            || table
                .lines_starting_with("RULES")
                .iter()
                .all(|line| line.contains("rolled")),
        "any rules line is the roll's own"
    );
}

#[tokio::test]
async fn neither_line_asks_the_storyteller_with_a_number() {
    let table = table(answers()).await;
    run(&table, &[14], &request("I climb.")).await;
    assert_eq!(table.calls.load(Ordering::SeqCst), 3);
    let asked = table.asked.lock().unwrap().clone();
    let flourish_asks: Vec<_> = asked
        .iter()
        .filter(|ask| ask.output_name == "check_called" || ask.output_name == "check_result")
        .collect();
    assert_eq!(flourish_asks.len(), 2);
    for ask in flourish_asks {
        let mut facts = ask.facts.clone();
        // The room for the flourish is a count of characters, not a number of the roll.
        facts
            .as_object_mut()
            .expect("facts are an object")
            .remove("room_chars");
        assert!(
            !facts
                .to_string()
                .chars()
                .any(|letter| letter.is_ascii_digit()),
            "{}",
            ask.facts
        );
    }
}

#[tokio::test]
async fn a_trivial_action_calls_no_roll_says_nothing_and_shows_no_card() {
    let table = table(vec![trivial_answer()]).await;
    let (run, clock) = run(&table, &[], &request("I step through the open door.")).await;
    assert_eq!(run.rolled, None);
    assert_eq!(table.calls.load(Ordering::SeqCst), 1);
    assert_eq!(clock.waits(), []);
    assert_eq!(
        table.lines_starting_with("CHECK "),
        ["CHECK skipped character=gauntlet_warrior reason=trivial_no_check"]
    );
    assert!(!card_is_up(&table));
    assert!(table.lines_starting_with("PERSONA narrated").is_empty());
}

#[tokio::test]
async fn heroic_inspiration_is_spent_on_the_named_die_and_the_line_says_so() {
    let table = table(answers()).await;
    let mut asked = request("I climb.");
    asked.inspiration = Some(0);
    let (run, _) = run(&table, &[6, 14], &asked).await;
    let rolled = run.rolled.expect("a roll was called");
    assert_eq!(
        rolled.view.inspiration_reroll,
        Some(df_playground::fixture::InspirationReroll { index: 0, was: 6 })
    );
    assert!(
        rolled
            .result_line
            .contains("Heroic Inspiration rerolled a six into a fourteen."),
        "{}",
        rolled.result_line
    );
}

#[tokio::test]
async fn with_tts_off_each_line_is_shown_as_text_only() {
    let table = table(answers()).await;
    let (run, _) = run(&table, &[14], &request("I climb.")).await;
    let rolled = run.rolled.expect("a roll was called");
    assert_eq!(
        (rolled.called_voice, rolled.result_voice),
        (LineVoice::TextOnly, LineVoice::TextOnly)
    );
    let narrated = table.lines_starting_with("PERSONA narrated");
    assert_eq!(narrated.len(), 2);
    assert!(
        narrated.iter().all(|line| line.contains("voiced=false")),
        "{narrated:?}"
    );
}

#[tokio::test]
async fn with_tts_on_and_a_failing_voice_the_lines_are_shown_and_the_run_goes_on() {
    let table = table(answers()).await;
    table.running.switch_speech_on();
    let port = Arc::new(FakePort::unconfigured());
    let routes = routes_speaking(&table, dice_showing(&[14]), &port);
    let clock = VirtualClock::over(table.app());
    let run = run_check(&routes, &clock, &request("I climb."))
        .await
        .expect("a voice that fails never stops the run");
    let rolled = run.rolled.expect("a roll was called");
    assert_eq!(
        (rolled.called_voice, rolled.result_voice),
        (LineVoice::Failed, LineVoice::Failed)
    );
    assert!(!table.lines_starting_with("SPEECH line_unvoiced").is_empty());
    assert_eq!(
        rolled.times.cleared_ms,
        rolled.times.hold_ms + rolled.times.shown_ms + RESULT_LEAD_MS
    );
}

#[tokio::test]
async fn a_character_with_no_persona_is_refused_before_the_dice_show_anything() {
    let table = table(answers()).await;
    std::fs::remove_file(
        table
            .running
            .playground_path("assets/personas/gauntlet_warrior.persona.json"),
    )
    .expect("removes the persona");
    let routes = routes_over(&table, dice_showing(&[14]));
    let clock = VirtualClock::over(table.app());
    let error = run_check(&routes, &clock, &request("I climb."))
        .await
        .expect_err("no persona, no names to keep out of the line");
    assert_eq!(
        error,
        CheckRouteError::BadRequest {
            cause: CheckError::Persona(Box::new(PersonaError::UnknownPersona {
                id: "gauntlet_warrior".to_owned()
            }))
        }
    );
    assert!(table.lines_starting_with("CHECK ").is_empty());
    assert!(!card_is_up(&table));
}

#[tokio::test]
async fn dice_that_cannot_be_drawn_stop_the_run_before_anything_is_said() {
    let table = table(answers()).await;
    let routes = routes_over(&table, dice_showing(&[]));
    let clock = VirtualClock::over(table.app());
    let error = run_check(&routes, &clock, &request("I climb."))
        .await
        .expect_err("the port has no dice to give");
    assert!(
        matches!(
            error,
            CheckRouteError::BadRequest {
                cause: CheckError::Rolling(_)
            }
        ),
        "{error:?}"
    );
    assert!(table.lines_starting_with("CHECK ").is_empty());
    assert!(table.lines_starting_with("PERSONA narrated").is_empty());
}

#[tokio::test]
async fn a_run_prints_the_judgement_the_two_lines_the_roll_and_the_times() {
    let table = table(answers()).await;
    let (run, _) = run(&table, &[14], &request("I climb.")).await;
    let printed = super::cli::format_run("gauntlet_warrior", &run);
    let lines: Vec<&str> = printed.lines().collect();
    assert_eq!(lines.len(), 7, "{printed}");
    assert!(lines[0].starts_with("gauntlet_warrior check: required=true initiator=player"));
    assert_eq!(
        lines[1],
        "  call: kind=ability_check ability=strength skill=athletics dc=15 mode=normal \
         grounds=none uses=-"
    );
    assert_eq!(lines[2], "  designed: kind=ability_check match=true");
    assert_eq!(
        lines[3],
        format!("  called (text_only): {BEFORE} {CALLED_NUMBERS}")
    );
    assert_eq!(
        lines[4],
        format!("  result (text_only): {RESULT_NUMBERS} {AFTER}")
    );
    assert_eq!(
        lines[5],
        "  roll: Athletics (Strength) kind=skill_check total=19 target=DC 15 outcome=success \
         natural=none dice=d20=14"
    );
    assert!(
        lines[6].starts_with("  times: drawn_ms=0 called_ms=0 shown_ms="),
        "{}",
        lines[6]
    );
}

#[tokio::test]
async fn the_next_run_is_told_the_flourishes_the_last_one_said() {
    let mut script = answers();
    script.extend(answers());
    let table = table(script).await;
    let routes = routes_over(&table, dice_showing(&[14, 14]));
    for _ in 0..2 {
        let clock = VirtualClock::over(table.app());
        run_check(&routes, &clock, &request("I climb."))
            .await
            .expect("the check runs");
    }
    let called = table.facts_under("check_called");
    let result = table.facts_under("check_result");
    assert_eq!(called[0]["said_before"], serde_json::json!([]));
    assert_eq!(
        result[0]["said_before"],
        serde_json::json!([BEFORE]),
        "the result is told the call of its own roll"
    );
    assert_eq!(called[1]["said_before"], serde_json::json!([BEFORE, AFTER]));
    assert_eq!(
        result[1]["said_before"],
        serde_json::json!([BEFORE, AFTER, BEFORE])
    );
}

#[tokio::test]
async fn the_result_is_worded_after_the_call_has_been_worded() {
    let table = table(answers()).await;
    run(&table, &[14], &request("I climb.")).await;
    let order: Vec<String> = table
        .asked
        .lock()
        .unwrap()
        .iter()
        .map(|ask| ask.output_name.clone())
        .collect();
    assert_eq!(order, ["check", "check_called", "check_result"]);
}

#[tokio::test]
async fn what_the_runner_just_said_aloud_reaches_the_call_so_it_is_not_restated() {
    let table = table(answers()).await;
    let mut asked = request("I climb.");
    asked.told = Some("A rope bridge sways over a gorge.".to_owned());
    run(&table, &[14], &asked).await;
    let scene = &table.facts_under("check_called")[0]["scene"];
    assert_eq!(
        scene["already_told_aloud"],
        "A rope bridge sways over a gorge."
    );
    let plain = super::fixtures::table(answers()).await;
    run(&plain, &[14], &request("I climb.")).await;
    assert_eq!(
        plain.facts_under("check_called")[0]["scene"]["already_told_aloud"],
        serde_json::Value::Null
    );
}

#[tokio::test]
async fn the_host_remembers_the_last_sixteen_flourishes_and_forgets_the_oldest() {
    let table = table(Vec::new()).await;
    let routes = routes_over(&table, dice_showing(&[]));
    for number in 0..18 {
        routes.remember(format!("Flourish {number}."));
    }
    routes.remember(String::new());
    let kept = routes.said_before();
    assert_eq!(kept.len(), 16);
    assert_eq!(kept[0], "Flourish 2.");
    assert_eq!(kept[15], "Flourish 17.");
}

#[tokio::test]
async fn the_call_is_worded_from_facts_that_do_not_depend_on_the_dice() {
    let mut told = Vec::new();
    for faces in [[2], [19]] {
        let table = table(answers()).await;
        run(&table, &faces, &request("I climb.")).await;
        told.push(table.facts_under("check_called").remove(0));
    }
    assert_eq!(told[0], told[1], "the same call whatever the dice show");
    let facts = &told[0];
    for hidden in ["outcome", "natural", "how_close"] {
        assert_eq!(facts[hidden], serde_json::Value::Null, "{hidden}");
    }
    let tones = facts["allowed_tones"].as_array().expect("a list");
    assert!(tones.iter().all(|tone| tone != "dread"), "{tones:?}");
}

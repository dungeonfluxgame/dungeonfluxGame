//! The dungeon master's lines around a roll: the composed slot is exact, and a flourish that does
//! not fit is refused and asked again.

use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::Duration;

use df_rules::roll::{
    D20Request, D20Test, Proficiency, RollMode, RollRequest, Skill, Target, TargetLabel,
};
use serde_json::{Value, json};

use super::fixtures::{WARRIOR_NAME, line_answer, table, toned};
use super::lines::{
    Closeness, LineFacts, LineJob, LineMoment, ResultFacts, SpokenLine, allowed_tones,
    closing_sentence, compose_line, verify_line, within_budget,
};
use super::numbers::{spoken_call, spoken_result};
use super::request::{CheckError, Initiator, ModeGround, Tone};
use crate::persona::judge_route::judge_route;

fn athletics() -> RollRequest {
    RollRequest::D20(D20Request {
        test: D20Test::SkillCheck(Skill::Athletics),
        mode: RollMode::Normal,
        ability_modifier: 3,
        proficiency: Proficiency::Proficient { bonus: 2 },
        target: Some(Target {
            label: TargetLabel::DifficultyClass,
            value: 15,
        }),
        heroic_inspiration: None,
    })
}

const CALLED: &str = "Athletics, Strength. Difficulty fifteen. Roll it.";
const READ: &str = "Fourteen on the die, plus three for Strength, plus two for proficiency. \
                    Nineteen against fifteen. A success.";

fn facts(moment: LineMoment) -> LineFacts {
    facts_by(moment, Initiator::Player)
}

fn facts_by(moment: LineMoment, initiator: Initiator) -> LineFacts {
    let request = athletics();
    let result = request.resolve(&[14]).expect("the die fits");
    LineFacts {
        moment,
        test: "Athletics, Strength".to_owned(),
        scene: json!({"place": "A cliff", "situation": "He sets his hands to the rock.",
                      "in_sight": [], "action": null}),
        initiator,
        mode: RollMode::Normal,
        grounds: ModeGround::None,
        difficulty: Some("medium"),
        result: (moment == LineMoment::Result).then(|| ResultFacts::of(&result)),
    }
}

fn names() -> Vec<String> {
    vec![WARRIOR_NAME.to_owned()]
}

#[test]
fn the_composed_numbers_are_the_rolls_whatever_the_flourish_says() {
    let request = athletics();
    let result = request.resolve(&[14]).expect("the die fits");
    assert_eq!(spoken_result(&request, &result), READ);
    assert_eq!(spoken_call(&request), CALLED);
}

#[test]
fn a_line_whose_numbers_are_not_the_rolls_is_refused() {
    let line = SpokenLine::new(
        "The rock is cold.".to_owned(),
        "Fifteen on the die, plus three for Strength, plus two for proficiency. Twenty against \
         fifteen. A success."
            .to_owned(),
        Some("The ledge holds.".to_owned()),
    );
    assert_eq!(verify_line(&line, READ), Err(CheckError::NumbersMismatch));
    let right = SpokenLine::new(
        "The rock is cold.".to_owned(),
        READ.to_owned(),
        Some("The ledge holds.".to_owned()),
    );
    assert_eq!(verify_line(&right, READ), Ok(()));
    assert_eq!(
        right.text(),
        format!("The rock is cold. {READ} The ledge holds.")
    );
}

#[test]
fn a_flourish_with_a_digit_in_it_is_refused() {
    let line = |before: &str, after: &str| {
        SpokenLine::new(before.to_owned(), READ.to_owned(), Some(after.to_owned()))
    };
    assert_eq!(
        verify_line(&line("You roll a 14.", "Good."), READ),
        Err(CheckError::FlavorHasDigits)
    );
    assert_eq!(verify_line(&line("Rock.", "Good."), READ), Ok(()));
}

#[test]
fn how_close_a_roll_was_is_narrow_clear_or_wide_either_side_of_the_target() {
    for (total, target, expected) in [
        (15, 15, Closeness::Narrow),
        (17, 15, Closeness::Narrow),
        (13, 15, Closeness::Narrow),
        (18, 15, Closeness::Clear),
        (9, 15, Closeness::Clear),
        (21, 15, Closeness::Clear),
        (22, 15, Closeness::Wide),
        (5, 15, Closeness::Wide),
    ] {
        assert_eq!(
            Closeness::of(total, target),
            expected,
            "{total} vs {target}"
        );
    }
}

/// The result facts of `request` resolved with `faces`.
fn went(request: &RollRequest, faces: &[u16]) -> ResultFacts {
    ResultFacts::of(&request.resolve(faces).expect("the faces fit the roll"))
}

fn attack() -> RollRequest {
    RollRequest::D20(D20Request {
        test: D20Test::Attack {
            ability: df_rules::roll::Ability::Strength,
            kind: df_rules::roll::AttackKind::Melee,
            critical_from: 19,
        },
        mode: RollMode::Normal,
        ability_modifier: 3,
        proficiency: Proficiency::Proficient { bonus: 2 },
        target: Some(Target {
            label: TargetLabel::ArmorClass,
            value: 16,
        }),
        heroic_inspiration: None,
    })
}

#[test]
fn a_call_has_only_tones_that_announce_no_outcome() {
    assert_eq!(
        allowed_tones(None),
        [
            Tone::Neutral,
            Tone::Anticipation,
            Tone::Awe,
            Tone::Wry,
            Tone::DryHumor
        ]
    );
    assert!(
        !allowed_tones(None).contains(&Tone::Dread),
        "dread telegraphs a failure before the dice are seen"
    );
}

#[test]
fn a_win_takes_the_tones_of_a_win_and_a_loss_those_of_a_loss() {
    let win = allowed_tones(Some(went(&athletics(), &[20])));
    let loss = allowed_tones(Some(went(&athletics(), &[2])));
    for tone in [Tone::Triumph, Tone::Relief, Tone::Awe, Tone::Neutral] {
        assert!(win.contains(&tone), "{tone:?}");
    }
    for tone in [Tone::Setback, Tone::Dread, Tone::Rue, Tone::Neutral] {
        assert!(loss.contains(&tone), "{tone:?}");
    }
    for tone in [Tone::Setback, Tone::Dread, Tone::Rue, Tone::Anticipation] {
        assert!(!win.contains(&tone), "a win is not {tone:?}");
    }
    for tone in [Tone::Triumph, Tone::Relief, Tone::Awe, Tone::Anticipation] {
        assert!(!loss.contains(&tone), "a loss is not {tone:?}");
    }
}

#[test]
fn a_near_thing_is_relief_or_wry_when_won_and_rue_or_wry_when_lost() {
    // 10 + 5 = 15 meets the DC of 15; 9 + 5 = 14 misses it by one.
    assert_eq!(
        allowed_tones(Some(went(&athletics(), &[10]))),
        [Tone::Relief, Tone::Wry]
    );
    assert_eq!(
        allowed_tones(Some(went(&athletics(), &[9]))),
        [Tone::Rue, Tone::Wry]
    );
}

#[test]
fn a_natural_twenty_is_awe_or_triumph_and_a_natural_one_dread_rue_or_a_turn_of_humor() {
    assert_eq!(
        allowed_tones(Some(went(&attack(), &[20]))),
        [Tone::Awe, Tone::Triumph]
    );
    assert_eq!(
        allowed_tones(Some(went(&attack(), &[1]))),
        [Tone::Dread, Tone::Rue, Tone::Wry, Tone::DryHumor]
    );
}

#[test]
fn a_roll_with_no_verdict_takes_the_plain_tones() {
    let initiative = RollRequest::D20(D20Request {
        test: D20Test::Initiative,
        mode: RollMode::Normal,
        ability_modifier: 1,
        proficiency: Proficiency::None,
        target: None,
        heroic_inspiration: None,
    });
    assert_eq!(
        allowed_tones(Some(went(&initiative, &[10]))),
        [Tone::Neutral, Tone::Wry, Tone::DryHumor, Tone::Awe]
    );
}

#[test]
fn no_tone_fits_both_a_win_and_a_loss_apart_from_the_plain_and_the_wry() {
    let win = allowed_tones(Some(went(&athletics(), &[20])));
    let loss = allowed_tones(Some(went(&athletics(), &[2])));
    let both: Vec<Tone> = win
        .iter()
        .copied()
        .filter(|tone| loss.contains(tone))
        .collect();
    assert_eq!(both, [Tone::Wry, Tone::Neutral, Tone::DryHumor]);
}

/// The line a job words for `moment` over `table`, within `budget`, after `said_before`.
async fn worded_as(
    table: &super::fixtures::Table,
    initiator: Initiator,
    moment: LineMoment,
    numbers: &str,
    said_before: &[String],
) -> Result<SpokenLine, super::request::CheckRouteError> {
    let route = judge_route::<CheckError>(&table.text, None).expect("the scripted route is usable");
    let names = names();
    let facts = facts_by(moment, initiator);
    let job = LineJob {
        app: table.app(),
        route: &route,
        names: &names,
        facts: &facts,
        said_before,
        budget: Duration::from_secs(3_600),
    };
    compose_line(&job, numbers.to_owned()).await
}

async fn worded_after(
    table: &super::fixtures::Table,
    moment: LineMoment,
    numbers: &str,
    said_before: &[String],
) -> Result<SpokenLine, super::request::CheckRouteError> {
    worded_as(table, Initiator::Player, moment, numbers, said_before).await
}

async fn worded(
    table: &super::fixtures::Table,
    moment: LineMoment,
    numbers: &str,
) -> Result<SpokenLine, super::request::CheckRouteError> {
    worded_after(table, moment, numbers, &[]).await
}

#[tokio::test]
async fn a_result_line_is_the_composed_numbers_then_one_flourish_after_the_verdict() {
    let table = table(vec![line_answer(
        "The rock is cold under your hands.",
        "The ledge holds, and so do you.",
    )])
    .await;
    let line = worded(&table, LineMoment::Result, READ)
        .await
        .expect("stands");
    assert_eq!(
        line.text(),
        format!("{READ} The ledge holds, and so do you.")
    );
    assert_eq!(
        table.calls.load(Ordering::SeqCst),
        1,
        "one call words a line"
    );
    let flavor_facts = table.facts_under("check_result").remove(0);
    assert_eq!(flavor_facts["moment"], "check_result");
    assert_eq!(flavor_facts["outcome"], "success");
    assert_eq!(flavor_facts["natural"], "none");
    assert_eq!(flavor_facts["how_close"], "clear");
    assert_eq!(flavor_facts["test"], "Athletics, Strength");
    assert_eq!(flavor_facts["difficulty"], "medium");
    assert_eq!(
        flavor_facts["allowed_tones"],
        json!(["triumph", "relief", "wry", "awe", "neutral", "dry_humor"])
    );
    let room = 12 * 14 - READ.chars().count();
    assert_eq!(
        flavor_facts["room_chars"], room,
        "a result is worded for 12 s"
    );
    let told = flavor_facts.to_string().replace(&room.to_string(), "");
    assert!(
        !told.chars().any(|letter| letter.is_ascii_digit()),
        "the flourish is told no numbers: {told}"
    );
    let check_lines = table.lines_starting_with("PERSONA check_line ");
    assert_eq!(check_lines.len(), 1);
    assert!(
        check_lines[0]
            .starts_with("PERSONA check_line moment=check_result tone=wry retried=0 duration_ms="),
        "{}",
        check_lines[0]
    );
}

#[tokio::test]
async fn the_schema_offers_only_the_tones_that_fit() {
    let table = table(vec![line_answer("The wall looms.", "")]).await;
    worded(&table, LineMoment::Called, CALLED)
        .await
        .expect("stands");
    let asked = table.asked.lock().unwrap()[0].output_schema.clone();
    assert_eq!(
        asked["properties"]["tone"]["enum"],
        json!(["neutral", "anticipation", "awe", "wry", "dry_humor"])
    );
    assert_eq!(asked["required"], json!(["tone", "before"]));
}

#[tokio::test]
async fn a_call_line_is_one_flourish_then_the_test_the_difficulty_and_the_order_to_roll() {
    let table = table(vec![toned("The wall looms over you.", "", "anticipation")]).await;
    let line = worded(&table, LineMoment::Called, CALLED)
        .await
        .expect("stands");
    assert_eq!(line.text(), format!("The wall looms over you. {CALLED}"));
    assert_eq!(
        table.calls.load(Ordering::SeqCst),
        1,
        "no other judge is asked"
    );
    let flavor_facts = table.facts_under("check_called").remove(0);
    assert_eq!(flavor_facts["moment"], "check_called");
    assert_eq!(flavor_facts["outcome"], Value::Null);
    for withheld in ["difficulty", "mode", "grounds"] {
        assert_eq!(
            flavor_facts[withheld],
            Value::Null,
            "{withheld}: a call is told nothing that leans on how the roll may go"
        );
    }
}

#[tokio::test]
async fn a_tone_that_does_not_fit_how_it_went_is_refused_and_asked_again() {
    let table = table(vec![
        toned("The rock is cold.", "A sorry sight.", "setback"),
        toned("The rock is cold.", "Fine work.", "triumph"),
    ])
    .await;
    let line = worded(&table, LineMoment::Result, READ)
        .await
        .expect("stands");
    assert!(line.text().ends_with("Fine work."));
    assert!(
        line.text().starts_with(READ),
        "a result is the verdict, then the flourish"
    );
    let told = table.facts_of(1)["previous_refusal"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        told,
        "the tone setback does not fit how it stands: choose one of allowed_tones"
    );
    assert_eq!(
        table.lines_starting_with("PERSONA check_line_retried"),
        ["PERSONA check_line_retried moment=check_result why=tone_not_allowed"]
    );
}

#[tokio::test]
async fn a_call_may_not_take_a_tone_of_a_win_or_a_loss() {
    let table = table(vec![
        toned("You will win this.", "", "triumph"),
        toned("The wall looms.", "", "anticipation"),
    ])
    .await;
    let line = worded(&table, LineMoment::Called, CALLED)
        .await
        .expect("stands");
    assert!(line.text().starts_with("The wall looms."));
    assert_eq!(
        table.lines_starting_with("PERSONA check_line_retried"),
        ["PERSONA check_line_retried moment=check_called why=tone_not_allowed"]
    );
}

#[tokio::test]
async fn an_answer_with_no_tone_or_an_unlisted_one_is_malformed_and_asked_again() {
    for bad in [
        json!({"before": "The rock is cold.", "after": "Fine."}),
        toned("The rock is cold.", "Fine.", "jolly"),
    ] {
        let table = table(vec![
            bad,
            line_answer("The rock is cold.", "The ledge holds."),
        ])
        .await;
        worded(&table, LineMoment::Result, READ)
            .await
            .expect("stands");
        assert_eq!(
            table.lines_starting_with("PERSONA check_line_retried"),
            ["PERSONA check_line_retried moment=check_result why=malformed_answer"]
        );
    }
}

#[tokio::test]
async fn a_flourish_with_a_digit_or_nothing_in_it_is_asked_again_once_with_why() {
    for (bad, why) in [
        ("You roll a 14.", "flavor_has_digits"),
        ("   ", "empty_flavor"),
    ] {
        let table = table(vec![
            line_answer("", bad),
            line_answer("", "The ledge holds."),
        ])
        .await;
        let line = worded(&table, LineMoment::Result, READ)
            .await
            .expect("stands");
        assert_eq!(line.text(), format!("{READ} The ledge holds."));
        assert_eq!(
            table.lines_starting_with("PERSONA check_line_retried"),
            [format!(
                "PERSONA check_line_retried moment=check_result why={why}"
            )]
        );
        assert!(table.facts_of(1)["previous_refusal"].is_string());
    }
}

#[tokio::test]
async fn a_semicolon_is_repaired_in_code_into_sentences_and_never_costs_an_ask() {
    let table = table(vec![line_answer(
        "",
        "Cold stone bites your palms; the rope sings; hold on",
    )])
    .await;
    let line = worded(&table, LineMoment::Result, READ)
        .await
        .expect("stands");
    assert_eq!(
        line.text(),
        format!("{READ} Cold stone bites your palms. The rope sings. Hold on.")
    );
    assert!(!line.text().contains(';'));
    assert_eq!(table.calls.load(Ordering::SeqCst), 1);
    assert!(
        table
            .lines_starting_with("PERSONA check_line_retried")
            .is_empty()
    );
    let called =
        super::fixtures::table(vec![line_answer("The rope sings; the wall looms.", "")]).await;
    let line = worded(&called, LineMoment::Called, CALLED)
        .await
        .expect("stands");
    assert_eq!(
        line.text(),
        format!("The rope sings. The wall looms. {CALLED}")
    );
    assert_eq!(called.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_line_is_asked_twice_at_most_and_then_keeps_what_is_clean_of_the_last_answer() {
    let table = table(vec![
        line_answer("", "You roll a 14."),
        line_answer("", "The ledge holds. It took 3 tries."),
        line_answer("", "Never asked for."),
    ])
    .await;
    let line = worded(&table, LineMoment::Result, READ)
        .await
        .expect("a refused line is not a failure");
    assert_eq!(line.text(), format!("{READ} The ledge holds."));
    assert_eq!(table.calls.load(Ordering::SeqCst), 2, "no third ask");
    assert_eq!(
        table.lines_starting_with("CHECK line_unjudged"),
        ["CHECK line_unjudged moment=check_result budget_ms=3600000 why=refused kept=true"]
    );
}

#[tokio::test]
async fn a_line_refused_throughout_with_nothing_clean_is_its_numbers_alone() {
    let table = table(vec![
        line_answer("", "You roll a 14."),
        line_answer("", "It took 3 tries."),
    ])
    .await;
    let line = worded(&table, LineMoment::Result, READ)
        .await
        .expect("a refused line is not a failure");
    assert_eq!(line.text(), READ);
    assert_eq!(
        table.lines_starting_with("CHECK line_unjudged"),
        ["CHECK line_unjudged moment=check_result budget_ms=3600000 why=refused kept=false"]
    );
}

#[tokio::test]
async fn a_flourish_that_says_the_characters_name_is_asked_again_whatever_its_case() {
    for bad in ["Corwen grips the rock.", "Hold on, CORWEN's boots slip."] {
        let table = table(vec![
            line_answer("", bad),
            line_answer("", "You grip the rock."),
        ])
        .await;
        let line = worded(&table, LineMoment::Result, READ)
            .await
            .expect("stands");
        assert!(line.text().ends_with("You grip the rock."));
        let told = table.facts_of(1)["previous_refusal"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(told, "the line called someone by name: name nobody");
        assert_eq!(
            table.lines_starting_with("PERSONA check_line_retried"),
            ["PERSONA check_line_retried moment=check_result why=names_called"]
        );
    }
}

#[tokio::test]
async fn the_last_flourishes_said_are_facts_so_the_next_does_not_repeat_them() {
    let table = table(vec![line_answer("The wall looms.", "")]).await;
    let said = vec![
        "A cold wind pulls at you.".to_owned(),
        "Stone groans.".to_owned(),
    ];
    worded_after(&table, LineMoment::Called, CALLED, &said)
        .await
        .expect("stands");
    assert_eq!(table.facts_of(0)["said_before"], json!(said));
}

#[tokio::test(start_paused = true)]
async fn a_line_not_worded_in_its_budget_is_said_as_its_numbers_alone_or_as_its_clean_candidate() {
    let table = table(Vec::new()).await;
    let route = judge_route::<CheckError>(&table.text, None).expect("the scripted route is usable");
    let names = names();
    let candidate_with = |after: &str| {
        Mutex::new(Some(SpokenLine::new(
            String::new(),
            READ.to_owned(),
            Some(after.to_owned()),
        )))
    };
    let cases = [
        (
            LineMoment::Called,
            CALLED,
            Mutex::new(None),
            CALLED.to_owned(),
        ),
        (LineMoment::Result, READ, Mutex::new(None), READ.to_owned()),
        (
            LineMoment::Result,
            READ,
            candidate_with("The ledge holds. It took 3 tries."),
            format!("{READ} The ledge holds."),
        ),
        (
            LineMoment::Result,
            READ,
            candidate_with("It took 3 tries. Corwen laughs."),
            READ.to_owned(),
        ),
    ];
    for (moment, numbers, candidate, spoken) in cases {
        let facts = facts(moment);
        let job = LineJob {
            app: table.app(),
            route: &route,
            names: &names,
            facts: &facts,
            said_before: &[],
            budget: Duration::from_millis(14_000),
        };
        let stalled = std::future::pending();
        let line = within_budget(&job, numbers.to_owned(), stalled, &candidate)
            .await
            .expect("a spent budget is not a failure");
        assert_eq!(line.text(), spoken);
    }
    assert_eq!(table.calls.load(Ordering::SeqCst), 0, "nothing was asked");
    assert_eq!(
        table.lines_starting_with("CHECK line_unjudged"),
        [
            "CHECK line_unjudged moment=check_called budget_ms=14000 why=budget kept=false",
            "CHECK line_unjudged moment=check_result budget_ms=14000 why=budget kept=false",
            "CHECK line_unjudged moment=check_result budget_ms=14000 why=budget kept=true",
            "CHECK line_unjudged moment=check_result budget_ms=14000 why=budget kept=false"
        ]
    );
}

#[tokio::test]
async fn the_line_log_never_holds_the_words_spoken() {
    let table = table(vec![line_answer(
        "The rock is cold under your hands.",
        "The ledge holds.",
    )])
    .await;
    worded(&table, LineMoment::Result, READ)
        .await
        .expect("stands");
    for line in table.lines_starting_with("PERSONA check_line") {
        assert!(!line.contains("rock"), "{line}");
        assert!(!line.contains("ledge"), "{line}");
    }
}

#[tokio::test]
async fn a_call_at_a_player_station_may_be_silent_because_the_numbers_are_the_call() {
    let table = table(vec![toned("", "", "neutral")]).await;
    let line = worded(&table, LineMoment::Called, CALLED)
        .await
        .expect("stands");
    assert_eq!(line.text(), CALLED);
    assert_eq!(table.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_call_at_a_narrator_station_names_the_trigger_and_is_never_silent() {
    let table = table(vec![
        toned("", "", "neutral"),
        toned("The first plank gives under your boot.", "", "anticipation"),
    ])
    .await;
    let line = worded_as(&table, Initiator::Narrator, LineMoment::Called, CALLED, &[])
        .await
        .expect("stands");
    assert_eq!(
        line.text(),
        format!("The first plank gives under your boot. {CALLED}")
    );
    assert_eq!(
        table.lines_starting_with("PERSONA check_line_retried"),
        ["PERSONA check_line_retried moment=check_called why=empty_flavor"]
    );
    assert_eq!(table.facts_of(0)["initiator"], "narrator");
}

#[tokio::test]
async fn the_instruction_asks_for_one_short_sentence_and_the_trigger_only() {
    let table = table(vec![line_answer("The wall looms.", "")]).await;
    worded(&table, LineMoment::Called, CALLED)
        .await
        .expect("stands");
    let told = table.asked.lock().unwrap()[0].instructions.clone();
    for needed in [
        "never about a verdict",
        "before anyone has seen the dice",
        "never hint at how it will go",
        "closers_used",
        "Never use he, she, they",
        "already_told_aloud",
        "one concrete sensory detail",
        "ONE short sentence",
        "never describe it again",
        "14 characters a second",
    ] {
        assert!(told.contains(needed), "{needed}");
    }
}

#[tokio::test]
async fn a_result_is_asked_for_a_tone_and_one_flourish_after_the_verdict() {
    let table = table(vec![line_answer("", "The ledge holds.")]).await;
    worded(&table, LineMoment::Result, READ)
        .await
        .expect("stands");
    let asked = table.asked.lock().unwrap()[0].output_schema.clone();
    assert_eq!(asked["required"], json!(["tone", "after"]));
    assert!(asked["properties"].get("before").is_none());
}

#[tokio::test]
async fn a_flourish_that_speaks_of_someone_in_the_third_person_is_asked_again() {
    for bad in [
        "That first plank shifts beneath his step.",
        "He's holding on, and so is the rope.",
        "The ledge answers their weight.",
    ] {
        let table = table(vec![
            line_answer("", bad),
            line_answer("", "Then the other planks answer here, under you."),
        ])
        .await;
        let line = worded(&table, LineMoment::Result, READ)
            .await
            .expect("stands");
        assert!(
            line.text()
                .ends_with("Then the other planks answer here, under you.")
        );
        assert_eq!(
            table.lines_starting_with("PERSONA check_line_retried"),
            ["PERSONA check_line_retried moment=check_result why=not_spoken_to_the_party"]
        );
        let told = table.facts_of(1)["previous_refusal"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(told.contains("speak to the party as you"), "{told}");
    }
}

#[test]
fn a_flourish_closes_on_its_last_sentence() {
    assert_eq!(
        closing_sentence("Cold stone bites your palms. The valley keeps its counsel."),
        "The valley keeps its counsel"
    );
    assert_eq!(
        closing_sentence("  One sentence only!  "),
        "One sentence only"
    );
    assert_eq!(closing_sentence(""), "");
}

#[tokio::test]
async fn the_last_six_flourishes_are_told_whole_and_the_closers_of_all_twelve_with_none_twice() {
    let table = table(vec![line_answer("The wall looms.", "")]).await;
    let said: Vec<String> = (0..8)
        .map(|number| {
            if number % 2 == 0 {
                format!("Opening {number}. The fire keeps its counsel.")
            } else {
                format!("Opening {number}. The planks have the last word.")
            }
        })
        .collect();
    worded_after(&table, LineMoment::Called, CALLED, &said)
        .await
        .expect("stands");
    let facts = table.facts_of(0);
    assert_eq!(facts["said_before"], json!(said[2..]));
    assert_eq!(
        facts["closers_used"],
        json!([
            "The fire keeps its counsel",
            "The planks have the last word"
        ])
    );
}

#[tokio::test]
async fn a_call_flourish_in_the_tone_of_dread_is_refused_because_it_telegraphs_a_failure() {
    let table = table(vec![
        toned("The wall looms.", "", "dread"),
        toned("The wall looms.", "", "anticipation"),
    ])
    .await;
    worded(&table, LineMoment::Called, CALLED)
        .await
        .expect("stands");
    assert_eq!(
        table.lines_starting_with("PERSONA check_line_retried"),
        ["PERSONA check_line_retried moment=check_called why=tone_not_allowed"]
    );
}

//! The `persona check` verbs: their grammar, and a judge through the command line, the route and a
//! scripted model with the real sheet and stations.

use std::path::PathBuf;

use df_rules::roll::{Ability, Skill};
use serde_json::json;

use super::cli::{CheckCommand, parse_check};
use super::fixtures::{athletics_answer, with};
use super::request::{CheckError, CheckKind, CheckRouteError, CheckTest, Initiator};
use crate::command::{Command, CommandError, parse};
use crate::dev::{DevError, ReplyError};
use crate::persona::cli::PersonaCommand;
use crate::persona::fixtures::scripted_host;

const SHEET: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../development/market-threads/gauntlet_warrior.sheet.json"
);
const STATIONS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../development/market-threads/gauntlet.stations.json"
);

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

fn args(command: CheckCommand) -> super::cli::CheckArgs {
    match command {
        CheckCommand::Judge(args) | CheckCommand::Run { args, .. } => args,
    }
}

#[test]
fn the_check_verbs_are_part_of_the_persona_grammar() {
    let line = words("persona check judge gauntlet_warrior station=athletics");
    assert!(matches!(
        parse(&line),
        Ok(Command::Persona(PersonaCommand::Check(
            CheckCommand::Judge(_)
        )))
    ));
}

#[test]
fn an_action_makes_a_player_moment_and_none_a_narrator_moment() {
    let player = args(
        parse_check(&[
            "judge".to_owned(),
            "gauntlet_warrior".to_owned(),
            "action=I climb the cliff.".to_owned(),
        ])
        .unwrap(),
    );
    assert_eq!(player.moment, Initiator::Player);
    assert_eq!(player.action.as_deref(), Some("I climb the cliff."));
    let narrator = args(parse_check(&words("judge gauntlet_warrior station=athletics")).unwrap());
    assert_eq!(narrator.moment, Initiator::Narrator);
    assert_eq!(narrator.station.as_deref(), Some("athletics"));
    let said = args(
        parse_check(&words(
            "judge gauntlet_warrior moment=narrator station=stealth",
        ))
        .unwrap(),
    );
    assert_eq!(said.moment, Initiator::Narrator);
}

#[test]
fn the_sheet_and_the_stations_have_defaults_in_the_data_folder_and_can_be_named() {
    let defaults = args(parse_check(&words("judge gauntlet_warrior")).unwrap());
    assert_eq!(
        defaults.sheet,
        PathBuf::from("development/market-threads/gauntlet_warrior.sheet.json")
    );
    assert_eq!(
        defaults.stations,
        PathBuf::from("development/market-threads/gauntlet.stations.json")
    );
    let named = args(
        parse_check(&words(
            "judge gauntlet_warrior sheet=a.json stations=b.json",
        ))
        .unwrap(),
    );
    assert_eq!(
        (named.sheet, named.stations),
        (PathBuf::from("a.json"), PathBuf::from("b.json"))
    );
}

#[test]
fn what_was_rolled_here_and_what_is_in_sight_can_be_given_more_than_once() {
    let given = args(
        parse_check(&words(
            "judge gauntlet_warrior place=cliff sight=rope sight=ledge model=fast \
             checked=initiative:dexterity checked=ability_check:strength:athletics \
             checked=death_save",
        ))
        .unwrap(),
    );
    assert_eq!(given.place.as_deref(), Some("cliff"));
    assert_eq!(given.sight, ["rope", "ledge"]);
    assert_eq!(given.model.as_deref(), Some("fast"));
    assert_eq!(
        given.checked,
        [
            CheckTest {
                kind: CheckKind::Initiative,
                ability: Some(Ability::Dexterity),
                skill: None
            },
            CheckTest {
                kind: CheckKind::AbilityCheck,
                ability: Some(Ability::Strength),
                skill: Some(Skill::Athletics)
            },
            CheckTest {
                kind: CheckKind::DeathSave,
                ability: None,
                skill: None
            },
        ]
    );
}

#[test]
fn a_run_takes_the_voice_and_the_inspiration() {
    let CheckCommand::Run {
        speak, inspiration, ..
    } = parse_check(&words("run gauntlet_warrior speak=false inspiration=1")).unwrap()
    else {
        panic!("a run");
    };
    assert_eq!((speak, inspiration), (false, Some(1)));
    let CheckCommand::Run {
        speak, inspiration, ..
    } = parse_check(&words("run gauntlet_warrior")).unwrap()
    else {
        panic!("a run");
    };
    assert_eq!((speak, inspiration), (true, None));
    let CheckCommand::Run { told, .. } =
        parse_check(&words("run gauntlet_warrior told=A_bridge_sways.")).unwrap()
    else {
        panic!("a run");
    };
    assert_eq!(told.as_deref(), Some("A_bridge_sways."));
    assert!(matches!(
        parse_check(&words("judge gauntlet_warrior told=x")),
        Err(CommandError::Usage(_))
    ));
}

#[test]
fn a_mistake_in_the_words_is_refused_with_the_usage_or_the_choices() {
    for bad in [
        "",
        "judge",
        "roll gauntlet_warrior",
        "judge gauntlet_warrior stray words",
        "judge gauntlet_warrior speak=false",
        "judge gauntlet_warrior inspiration=1",
    ] {
        assert!(
            matches!(parse_check(&words(bad)), Err(CommandError::Usage(_))),
            "{bad:?}"
        );
    }
    assert!(matches!(
        parse_check(&words("judge gauntlet_warrior station=a station=b")),
        Err(CommandError::DuplicateKey(_))
    ));
    for bad in [
        "judge gauntlet_warrior moment=boss",
        "judge gauntlet_warrior checked=dance",
        "judge gauntlet_warrior checked=ability_check:might",
        "judge gauntlet_warrior checked=ability_check:strength:athletics:more",
        "run gauntlet_warrior speak=loud",
    ] {
        assert!(
            matches!(
                parse_check(&words(bad)),
                Err(CommandError::BadChoice { .. })
            ),
            "{bad:?}"
        );
    }
    assert!(matches!(
        parse_check(&words("run gauntlet_warrior inspiration=first")),
        Err(CommandError::BadNumber { .. })
    ));
}

#[tokio::test]
async fn judging_through_the_command_line_prints_the_typed_answer_and_the_designed_match() {
    let host = scripted_host(vec![athletics_answer()]).await;
    let line = words(&format!(
        "persona check judge gauntlet_warrior sheet={SHEET} stations={STATIONS} \
         station=athletics action=I_set_my_hands_to_the_rock"
    ));
    let printed = crate::dev::run_dev(host.port, &line).await.expect("judges");
    let printed: Vec<&str> = printed.lines().collect();
    assert!(
        printed[0].starts_with(
            "gauntlet_warrior check: required=true initiator=player \
             reason=uncertain_with_consequence retried=0 duration_ms="
        ),
        "{printed:?}"
    );
    assert_eq!(
        printed[1],
        "  call: kind=ability_check ability=strength skill=athletics dc=15 mode=normal \
         grounds=none uses=-"
    );
    assert_eq!(printed[2], "  designed: kind=ability_check match=true");
    let sent = host.facts_of(0);
    assert_eq!(sent["action"], "I_set_my_hands_to_the_rock");
    assert!(sent["situation"].as_str().unwrap().contains("cliff"));
    assert_eq!(sent["character"]["ability_modifiers"]["strength"], 3);
    assert_eq!(host.calls(), 1);
}

#[tokio::test]
async fn the_narrators_moment_through_the_command_line_names_a_narrator_reason() {
    let answer = with(
        athletics_answer(),
        json!({"initiator": "narrator", "reason": "narrator_obstacle"}),
    );
    let host = scripted_host(vec![answer]).await;
    let line = words(&format!(
        "persona check judge gauntlet_warrior sheet={SHEET} stations={STATIONS} station=athletics"
    ));
    let printed = crate::dev::run_dev(host.port, &line).await.expect("judges");
    assert!(
        printed.contains("initiator=narrator reason=narrator_obstacle"),
        "{printed}"
    );
    assert_eq!(host.facts_of(0)["moment"], "narrator");
    assert_eq!(host.facts_of(0)["action"], json!(null));
}

#[tokio::test]
async fn a_refusal_of_the_route_comes_back_typed_and_is_logged_once() {
    let host = scripted_host(vec![athletics_answer()]).await;
    let line = words(&format!(
        "persona check judge gauntlet_warrior sheet={SHEET} moment=player"
    ));
    let error = crate::dev::run_dev(host.port, &line)
        .await
        .expect_err("a player moment needs its action");
    let DevError::Rejected { error, .. } = error else {
        panic!("a typed refusal: {error:?}");
    };
    assert_eq!(
        error,
        ReplyError::Check(CheckRouteError::BadRequest {
            cause: CheckError::NeedsAction
        })
    );
    assert_eq!(
        host.lines_starting_with("PERSONA refused verb=check"),
        ["PERSONA refused verb=check error=bad_request"]
    );
    assert_eq!(host.calls(), 0);
}

#[tokio::test]
async fn a_sheet_file_that_is_not_there_or_does_not_read_is_an_unreadable_file() {
    let host = scripted_host(Vec::new()).await;
    let missing = words("persona check judge gauntlet_warrior sheet=no_such_sheet.json action=x");
    assert!(matches!(
        crate::dev::run_dev(host.port, &missing).await,
        Err(DevError::UnreadableFile { .. })
    ));
    let not_a_sheet = words(&format!(
        "persona check judge gauntlet_warrior sheet={STATIONS} action=x"
    ));
    assert!(matches!(
        crate::dev::run_dev(host.port, &not_a_sheet).await,
        Err(DevError::UnreadableFile { .. })
    ));
    let no_station = words(&format!(
        "persona check judge gauntlet_warrior sheet={SHEET} stations={STATIONS} station=nothing"
    ));
    assert!(matches!(
        crate::dev::run_dev(host.port, &no_station).await,
        Err(DevError::UnreadableFile { .. })
    ));
}

#[tokio::test]
async fn running_a_trivial_action_through_the_command_line_says_no_roll_was_called() {
    let host = scripted_host(vec![super::fixtures::trivial_answer()]).await;
    let line = words(&format!(
        "persona check run gauntlet_warrior sheet={SHEET} stations={STATIONS} \
         station=athletics action=I_step_through_the_open_door"
    ));
    let printed = crate::dev::run_dev(host.port, &line).await.expect("runs");
    assert!(
        printed.starts_with(
            "gauntlet_warrior check: required=false initiator=player reason=trivial_no_check"
        ),
        "{printed}"
    );
    assert!(printed.ends_with("  no roll was called"), "{printed}");
    assert_eq!(host.calls(), 1);
}

#[test]
fn an_intended_test_is_read_like_a_checked_one_and_a_mistake_in_it_is_refused() {
    let intent = args(
        parse_check(&words(
            "judge gauntlet_warrior station=acrobatics intent=ability_check:dexterity:acrobatics",
        ))
        .unwrap(),
    )
    .intent;
    assert_eq!(
        intent,
        Some(CheckTest {
            kind: CheckKind::AbilityCheck,
            ability: Some(Ability::Dexterity),
            skill: Some(Skill::Acrobatics)
        })
    );
    assert!(matches!(
        parse_check(&words("run gauntlet_warrior intent=dance")),
        Err(CommandError::BadChoice { .. })
    ));
}

#[tokio::test]
async fn an_intent_reaches_the_judge_at_a_narrator_moment_and_is_refused_at_a_player_one() {
    let answer = with(
        athletics_answer(),
        json!({"initiator": "narrator", "reason": "narrator_obstacle"}),
    );
    let host = scripted_host(vec![answer]).await;
    let line = words(&format!(
        "persona check judge gauntlet_warrior sheet={SHEET} stations={STATIONS} \
         station=athletics intent=ability_check:strength:athletics"
    ));
    let printed = crate::dev::run_dev(host.port, &line).await.expect("judges");
    assert!(printed.contains("initiator=narrator"), "{printed}");
    assert_eq!(
        host.facts_of(0)["narrator_intent"],
        json!({"kind": "ability_check", "ability": "strength", "skill": "athletics"})
    );

    let with_action = words(&format!(
        "persona check judge gauntlet_warrior sheet={SHEET} action=I_climb \
         intent=ability_check:strength:athletics"
    ));
    let error = crate::dev::run_dev(host.port, &with_action)
        .await
        .expect_err("a player's own words decide");
    let DevError::Rejected { error, .. } = error else {
        panic!("a typed refusal: {error:?}");
    };
    assert_eq!(
        error,
        ReplyError::Check(CheckRouteError::BadRequest {
            cause: CheckError::IntentNeedsNarrator
        })
    );
    assert_eq!(host.calls(), 1, "the refusal asked no question");
}

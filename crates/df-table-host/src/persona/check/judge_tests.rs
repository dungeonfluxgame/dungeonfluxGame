//! The check judge: its facts and schema, each refusal of an answer, and the asking until one
//! stands.

use df_rules::roll::{Ability, RollMode, Skill};
use serde_json::{Value, json};

use super::fixtures::{athletics_answer, judge_request, table, trivial_answer, with};
use super::judge::{check_request, judge_check, judge_facts, judge_schema, parse_verdict};
use super::request::{
    AnswerField, CheckCall, CheckError, CheckKind, CheckReason, CheckRouteError, CheckTest,
    CheckVerdict, DcBand, Initiator, ModeGround, Tone, mode_from_key, mode_key,
};

fn player() -> super::request::CheckJudgeRequest {
    judge_request(
        Initiator::Player,
        Some("I set my hands to the rock and climb."),
    )
}

fn narrator() -> super::request::CheckJudgeRequest {
    judge_request(Initiator::Narrator, None)
}

fn refused(answer: &Value) -> CheckError {
    parse_verdict(answer, &player()).expect_err("the answer is refused")
}

fn called(answer: &Value) -> CheckCall {
    parse_verdict(answer, &player())
        .expect("the answer stands")
        .call
        .expect("a roll is called")
}

#[test]
fn an_athletics_check_stands_with_its_ability_skill_band_and_reason() {
    let verdict = parse_verdict(&athletics_answer(), &player()).expect("stands");
    assert_eq!(
        verdict,
        CheckVerdict {
            initiator: Initiator::Player,
            reason: CheckReason::UncertainWithConsequence,
            call: Some(CheckCall {
                kind: CheckKind::AbilityCheck,
                ability: Some(Ability::Strength),
                skill: Some(Skill::Athletics),
                dc_band: Some(DcBand::Medium),
                mode: RollMode::Normal,
                grounds: ModeGround::None,
                uses: None,
            }),
        }
    );
}

#[test]
fn a_narrator_moment_stands_with_a_narrator_reason() {
    let answer = with(
        athletics_answer(),
        json!({"initiator": "narrator", "reason": "narrator_obstacle"}),
    );
    let verdict = parse_verdict(&answer, &narrator()).expect("stands");
    assert_eq!(verdict.initiator, Initiator::Narrator);
    assert_eq!(verdict.reason, CheckReason::NarratorObstacle);
    assert!(verdict.call.is_some());
}

#[test]
fn an_action_that_is_trivial_impossible_or_without_stakes_is_not_rolled() {
    for reason in ["trivial_no_check", "impossible_no_check", "no_stakes"] {
        let answer = with(trivial_answer(), json!({"reason": reason}));
        let verdict = parse_verdict(&answer, &player()).expect("stands");
        assert_eq!(verdict.call, None, "{reason}");
        assert!(!verdict.reason.calls_for_check());
    }
}

#[test]
fn a_no_check_answer_that_still_names_a_roll_is_refused() {
    let answer = with(trivial_answer(), json!({"kind": "ability_check"}));
    assert_eq!(
        refused(&answer),
        CheckError::UnexpectedField {
            field: AnswerField::Kind
        }
    );
    let answer = with(
        trivial_answer(),
        json!({"mode": "advantage", "grounds": "ally_help"}),
    );
    assert_eq!(
        refused(&answer),
        CheckError::UngroundedMode {
            mode: RollMode::Advantage,
            grounds: ModeGround::AllyHelp
        }
    );
}

#[test]
fn an_initiator_that_is_not_the_moments_is_refused() {
    let answer = with(
        athletics_answer(),
        json!({"initiator": "narrator", "reason": "narrator_hazard"}),
    );
    assert_eq!(
        refused(&answer),
        CheckError::InitiatorMismatch {
            moment: Initiator::Player,
            initiator: Initiator::Narrator
        }
    );
}

#[test]
fn a_reason_that_does_not_fit_the_decision_or_the_initiator_is_refused() {
    let answer = with(athletics_answer(), json!({"reason": "trivial_no_check"}));
    assert_eq!(
        refused(&answer),
        CheckError::ReasonMismatch {
            check_required: true,
            reason: CheckReason::TrivialNoCheck
        }
    );
    let answer = with(
        trivial_answer(),
        json!({"reason": "uncertain_with_consequence"}),
    );
    assert_eq!(
        refused(&answer),
        CheckError::ReasonMismatch {
            check_required: false,
            reason: CheckReason::UncertainWithConsequence
        }
    );
    let answer = with(
        athletics_answer(),
        json!({"initiator": "narrator", "reason": "uncertain_with_consequence"}),
    );
    assert_eq!(
        parse_verdict(&answer, &narrator()),
        Err(CheckError::ReasonInitiatorMismatch {
            initiator: Initiator::Narrator,
            reason: CheckReason::UncertainWithConsequence
        })
    );
}

#[test]
fn an_ability_that_is_not_the_skills_is_refused() {
    let answer = with(athletics_answer(), json!({"ability": "dexterity"}));
    assert_eq!(
        refused(&answer),
        CheckError::AbilityMismatch {
            ability: Ability::Dexterity,
            expected: Ability::Strength
        }
    );
}

#[test]
fn a_band_off_the_ladder_is_refused_and_each_rung_stands() {
    for off in [0, 4, 14, 16, 35] {
        let answer = with(athletics_answer(), json!({"dc_band": off}));
        assert_eq!(refused(&answer), CheckError::DcOffLadder { value: off });
    }
    for band in [5, 10, 15, 20, 25, 30] {
        let answer = with(athletics_answer(), json!({"dc_band": band}));
        assert_eq!(
            called(&answer).dc_band.map(DcBand::value),
            Some(u8::try_from(band).unwrap())
        );
    }
    let answer = with(athletics_answer(), json!({"dc_band": null}));
    assert_eq!(
        refused(&answer),
        CheckError::MissingField {
            field: AnswerField::DcBand
        }
    );
}

#[test]
fn an_answer_that_is_not_in_the_closed_shape_is_refused() {
    for answer in [
        with(athletics_answer(), json!({"check_required": "yes"})),
        with(athletics_answer(), json!({"kind": "dance"})),
        with(athletics_answer(), json!({"dc_band": "15"})),
        with(athletics_answer(), json!({"dc_band": 12.5})),
        with(athletics_answer(), json!({"skill": 7})),
        with(athletics_answer(), json!({"mode": "double"})),
        with(athletics_answer(), json!({"uses": 4})),
    ] {
        assert!(
            matches!(refused(&answer), CheckError::MalformedAnswer { .. }),
            "{answer}"
        );
    }
    let mut missing = athletics_answer();
    missing.as_object_mut().unwrap().remove("check_required");
    assert!(matches!(
        refused(&missing),
        CheckError::MalformedAnswer { .. }
    ));
}

#[test]
fn each_kind_asks_for_the_fields_it_needs_and_no_others() {
    let save = with(
        athletics_answer(),
        json!({"kind": "saving_throw", "skill": null, "ability": "constitution"}),
    );
    let call = called(&save);
    assert_eq!(
        (call.kind, call.ability, call.skill),
        (CheckKind::SavingThrow, Some(Ability::Constitution), None)
    );
    let save_with_skill = with(save.clone(), json!({"skill": "athletics"}));
    assert_eq!(
        refused(&save_with_skill),
        CheckError::UnexpectedField {
            field: AnswerField::Skill
        }
    );

    let initiative = with(
        athletics_answer(),
        json!({"kind": "initiative", "skill": null, "ability": "dexterity", "dc_band": null}),
    );
    assert_eq!(called(&initiative).ability, Some(Ability::Dexterity));
    let initiative_with_band = with(initiative, json!({"dc_band": 10}));
    assert_eq!(
        refused(&initiative_with_band),
        CheckError::UnexpectedField {
            field: AnswerField::DcBand
        }
    );

    let death = with(
        athletics_answer(),
        json!({"kind": "death_save", "skill": null, "ability": null, "dc_band": null}),
    );
    assert_eq!(called(&death).kind, CheckKind::DeathSave);
    let death_with_ability = with(death, json!({"ability": "constitution"}));
    assert_eq!(
        refused(&death_with_ability),
        CheckError::UnexpectedField {
            field: AnswerField::Ability
        }
    );

    let passive = with(
        athletics_answer(),
        json!({"kind": "passive", "skill": null, "ability": null, "dc_band": null}),
    );
    let call = called(&passive);
    assert_eq!(
        (call.ability, call.skill),
        (Some(Ability::Wisdom), Some(Skill::Perception))
    );

    let table_roll = with(
        athletics_answer(),
        json!({"kind": "table", "skill": null, "ability": null, "dc_band": null}),
    );
    assert_eq!(called(&table_roll).kind, CheckKind::Table);
}

#[test]
fn a_weapon_or_healing_must_be_one_the_sheet_has() {
    let attack = with(
        athletics_answer(),
        json!({"kind": "attack_roll", "skill": null, "ability": null, "uses": "longsword"}),
    );
    let call = called(&attack);
    assert_eq!(
        call.ability,
        Some(Ability::Strength),
        "the weapon's own ability"
    );
    assert_eq!(call.uses.as_deref(), Some("longsword"));
    let ranged = with(
        attack.clone(),
        json!({"uses": "longbow", "ability": "dexterity"}),
    );
    assert_eq!(called(&ranged).ability, Some(Ability::Dexterity));
    let wrong_ability = with(attack.clone(), json!({"ability": "dexterity"}));
    assert_eq!(
        refused(&wrong_ability),
        CheckError::AbilityMismatch {
            ability: Ability::Dexterity,
            expected: Ability::Strength
        }
    );
    let unknown = with(attack.clone(), json!({"uses": "greataxe"}));
    assert_eq!(
        refused(&unknown),
        CheckError::UnknownUses {
            id: "greataxe".to_owned()
        }
    );
    let no_weapon = with(attack, json!({"uses": null}));
    assert_eq!(
        refused(&no_weapon),
        CheckError::MissingField {
            field: AnswerField::Uses
        }
    );

    let damage = with(
        athletics_answer(),
        json!({"kind": "damage", "skill": null, "ability": null, "dc_band": null,
               "uses": "longsword"}),
    );
    assert_eq!(called(&damage).kind, CheckKind::Damage);
    let healing = with(
        athletics_answer(),
        json!({"kind": "healing", "skill": null, "ability": null, "dc_band": null,
               "uses": "second_wind"}),
    );
    assert_eq!(called(&healing).uses.as_deref(), Some("second_wind"));
    let no_such_healing = with(healing, json!({"uses": "longsword"}));
    assert_eq!(
        refused(&no_such_healing),
        CheckError::UnknownUses {
            id: "longsword".to_owned()
        }
    );
}

#[test]
fn a_contest_is_one_of_the_two_the_rules_have() {
    let escape = with(athletics_answer(), json!({"kind": "contested"}));
    assert_eq!(called(&escape).kind, CheckKind::Contested);
    let acrobatics = with(
        escape.clone(),
        json!({"skill": "acrobatics", "ability": "dexterity"}),
    );
    assert_eq!(called(&acrobatics).skill, Some(Skill::Acrobatics));
    let spot = with(
        escape.clone(),
        json!({"skill": "perception", "ability": "wisdom"}),
    );
    assert_eq!(called(&spot).skill, Some(Skill::Perception));
    let stealth = with(escape, json!({"skill": "stealth", "ability": "dexterity"}));
    assert_eq!(
        refused(&stealth),
        CheckError::UnsupportedSkill {
            skill: Skill::Stealth
        }
    );
}

#[test]
fn a_mode_needs_a_ground_that_gives_it_and_the_sheet_holds_what_the_ground_needs() {
    let advantaged = with(
        athletics_answer(),
        json!({"mode": "advantage", "grounds": "character_feature"}),
    );
    assert_eq!(called(&advantaged).mode, RollMode::Advantage);
    let hunch = with(athletics_answer(), json!({"mode": "advantage"}));
    assert_eq!(
        refused(&hunch),
        CheckError::UngroundedMode {
            mode: RollMode::Advantage,
            grounds: ModeGround::None
        }
    );
    let wrong_way = with(
        athletics_answer(),
        json!({"mode": "disadvantage", "grounds": "favorable_circumstance"}),
    );
    assert_eq!(
        refused(&wrong_way),
        CheckError::UngroundedMode {
            mode: RollMode::Disadvantage,
            grounds: ModeGround::FavorableCircumstance
        }
    );
    let armored = with(
        athletics_answer(),
        json!({"kind": "ability_check", "skill": "stealth", "ability": "dexterity",
               "mode": "disadvantage", "grounds": "armor_penalty"}),
    );
    assert_eq!(called(&armored).mode, RollMode::Disadvantage);

    let mut bare = player();
    bare.sheet.features.clear();
    bare.sheet.equipment.clear();
    assert_eq!(
        parse_verdict(&advantaged, &bare),
        Err(CheckError::GroundNotInSheet {
            grounds: ModeGround::CharacterFeature
        })
    );
    assert_eq!(
        parse_verdict(&armored, &bare),
        Err(CheckError::GroundNotInSheet {
            grounds: ModeGround::ArmorPenalty
        })
    );
}

#[test]
fn a_roll_with_no_d20_has_no_mode() {
    let damage = with(
        athletics_answer(),
        json!({"kind": "damage", "skill": null, "ability": null, "dc_band": null,
               "uses": "longsword", "mode": "advantage", "grounds": "ally_help"}),
    );
    assert_eq!(
        refused(&damage),
        CheckError::ModeOnDice {
            kind: CheckKind::Damage
        }
    );
}

#[test]
fn the_same_test_is_not_called_twice_at_a_place() {
    let mut again = player();
    again.checked = vec![CheckTest {
        kind: CheckKind::AbilityCheck,
        ability: Some(Ability::Strength),
        skill: Some(Skill::Athletics),
    }];
    assert_eq!(
        parse_verdict(&athletics_answer(), &again),
        Err(CheckError::DoubleCheck)
    );
    let other = with(
        athletics_answer(),
        json!({"skill": "intimidation", "ability": "charisma"}),
    );
    assert!(
        parse_verdict(&other, &again).is_ok(),
        "another test is fine"
    );

    let already = with(trivial_answer(), json!({"reason": "already_checked"}));
    assert_eq!(refused(&already), CheckError::NothingChecked);
    assert_eq!(
        parse_verdict(&already, &again).map(|verdict| verdict.reason),
        Ok(CheckReason::AlreadyChecked)
    );
}

#[test]
fn a_request_states_the_action_at_a_player_moment_and_none_at_a_narrator_moment() {
    assert_eq!(check_request(&player()), Ok(()));
    assert_eq!(check_request(&narrator()), Ok(()));
    let mut silent = player();
    silent.action = Some("   ".to_owned());
    assert_eq!(check_request(&silent), Err(CheckError::NeedsAction));
    let mut told = narrator();
    told.action = Some("I climb.".to_owned());
    assert_eq!(check_request(&told), Err(CheckError::UnwantedAction));
    let mut other = player();
    other.character = "innkeeper".to_owned();
    assert_eq!(
        check_request(&other),
        Err(CheckError::SheetForOther {
            sheet: "gauntlet_warrior".to_owned(),
            character: "innkeeper".to_owned()
        })
    );
}

#[test]
fn the_closed_keys_match_their_serde_names_and_read_back() {
    for kind in CheckKind::ALL {
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(kind.key()));
        assert_eq!(CheckKind::from_key(kind.key()), Some(kind));
    }
    for initiator in Initiator::ALL {
        assert_eq!(
            serde_json::to_value(initiator).unwrap(),
            json!(initiator.key())
        );
        assert_eq!(Initiator::from_key(initiator.key()), Some(initiator));
    }
    for ground in ModeGround::ALL {
        assert_eq!(serde_json::to_value(ground).unwrap(), json!(ground.key()));
        assert_eq!(ModeGround::from_key(ground.key()), Some(ground));
    }
    for reason in CheckReason::ALL {
        assert_eq!(serde_json::to_value(reason).unwrap(), json!(reason.key()));
        assert_eq!(CheckReason::from_key(reason.key()), Some(reason));
    }
    for tone in Tone::ALL {
        assert_eq!(serde_json::to_value(tone).unwrap(), json!(tone.key()));
        assert_eq!(Tone::from_key(tone.key()), Some(tone));
    }
    for mode in [
        RollMode::Normal,
        RollMode::Advantage,
        RollMode::Disadvantage,
    ] {
        assert_eq!(serde_json::to_value(mode).unwrap(), json!(mode_key(mode)));
        assert_eq!(mode_from_key(mode_key(mode)), Some(mode));
    }
    for band in DcBand::ALL {
        assert_eq!(serde_json::to_value(band).unwrap(), json!(band.value()));
        assert_eq!(DcBand::from_value(i64::from(band.value())), Some(band));
    }
    for field in [AnswerField::DcBand, AnswerField::CheckRequired] {
        assert_eq!(serde_json::to_value(field).unwrap(), json!(field.key()));
    }
}

#[test]
fn the_schema_lists_exactly_the_closed_sets() {
    let schema = judge_schema();
    let listed = |field: &str| -> Vec<Value> {
        schema["properties"][field]["enum"]
            .as_array()
            .unwrap_or_else(|| panic!("{field} lists its keys"))
            .clone()
    };
    let mut kinds: Vec<Value> = CheckKind::ALL
        .iter()
        .map(|kind| json!(kind.key()))
        .collect();
    kinds.push(Value::Null);
    assert_eq!(listed("kind"), kinds);
    assert_eq!(
        listed("dc_band"),
        [
            json!(5),
            json!(10),
            json!(15),
            json!(20),
            json!(25),
            json!(30),
            Value::Null
        ]
    );
    assert_eq!(listed("reason").len(), CheckReason::ALL.len());
    assert_eq!(listed("grounds").len(), ModeGround::ALL.len());
    assert_eq!(listed("skill").len(), Skill::ALL.len() + 1);
    assert_eq!(listed("ability").len(), Ability::ALL.len() + 1);
    assert_eq!(schema["additionalProperties"], json!(false));
    assert_eq!(schema["required"].as_array().map(Vec::len), Some(10));
}

#[test]
fn the_facts_give_the_action_the_place_the_sheet_and_what_was_rolled_but_not_the_designed_test() {
    let mut request = player();
    request.checked = vec![CheckTest {
        kind: CheckKind::Initiative,
        ability: Some(Ability::Dexterity),
        skill: None,
    }];
    let facts = judge_facts(&request);
    assert_eq!(facts["moment"], "player");
    assert_eq!(facts["action"], "I set my hands to the rock and climb.");
    assert!(facts["place"].as_str().unwrap().contains("limestone"));
    assert!(facts["situation"].as_str().unwrap().contains("sheer cliff"));
    assert_eq!(facts["character"]["proficiency_bonus"], 2);
    assert_eq!(facts["character"]["ability_modifiers"]["strength"], 3);
    assert_eq!(facts["character"]["attacks"][0]["id"], "longsword");
    assert_eq!(facts["character"]["healing"][0], "second_wind");
    assert_eq!(facts["already_checked"][0]["kind"], "initiative");
    assert_eq!(facts["ladder"][2], json!({"dc": 15, "name": "medium"}));
    assert_eq!(facts["skills_table"].as_array().map(Vec::len), Some(18));
    let told = facts.to_string();
    assert!(
        !told.contains("ability_check"),
        "the station's designed kind is not told to the judge: {told}"
    );
    assert_eq!(judge_facts(&narrator())["action"], Value::Null);
}

#[tokio::test]
async fn a_refused_answer_is_asked_again_with_every_refusal_so_far_until_one_stands() {
    let off_ladder = with(athletics_answer(), json!({"dc_band": 14}));
    let wrong_ability = with(athletics_answer(), json!({"ability": "dexterity"}));
    let table = table(vec![off_ladder, wrong_ability, athletics_answer()]).await;
    let judged = judge_check(table.app(), &table.text, &player())
        .await
        .expect("the third answer stands");
    assert_eq!(judged.retried, 2);
    assert_eq!(judged.verdict.reason, CheckReason::UncertainWithConsequence);
    assert_eq!(table.calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert!(table.facts_of(0).get("previous_refusal").is_none());
    let second = table.facts_of(1)["previous_refusal"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        second.contains("14 is not on the difficulty ladder"),
        "{second}"
    );
    let third = table.facts_of(2)["previous_refusal"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        third.contains("14 is not on the difficulty ladder"),
        "{third}"
    );
    assert!(
        third.contains("the ability is dexterity but this test uses strength"),
        "{third}"
    );
    assert_eq!(
        table.lines_starting_with("PERSONA check_judge_retried"),
        [
            "PERSONA check_judge_retried why=dc_off_ladder",
            "PERSONA check_judge_retried why=ability_mismatch"
        ]
    );
    let judged_lines = table.lines_starting_with("PERSONA check_judged");
    assert_eq!(judged_lines.len(), 1);
    assert!(
        judged_lines[0].starts_with(
            "PERSONA check_judged character=gauntlet_warrior moment=player required=true \
             kind=ability_check reason=uncertain_with_consequence mode=normal retried=2 \
             duration_ms="
        ),
        "{}",
        judged_lines[0]
    );
}

#[tokio::test]
async fn the_judge_says_whether_the_station_got_the_test_it_was_designed_for() {
    let table = table(vec![athletics_answer()]).await;
    let judged = judge_check(table.app(), &table.text, &player())
        .await
        .expect("stands");
    assert_eq!(judged.designed.map(|designed| designed.matches), Some(true));

    let other = with(
        athletics_answer(),
        json!({"skill": "intimidation", "ability": "charisma"}),
    );
    let table = super::fixtures::table(vec![other]).await;
    let judged = judge_check(table.app(), &table.text, &player())
        .await
        .expect("stands");
    assert_eq!(
        judged.designed.map(|designed| designed.matches),
        Some(false)
    );
}

#[tokio::test]
async fn a_trivial_action_is_judged_with_one_question_and_no_roll() {
    let table = table(vec![trivial_answer()]).await;
    let judged = judge_check(table.app(), &table.text, &player())
        .await
        .expect("stands");
    assert_eq!(judged.verdict.call, None);
    assert_eq!(judged.verdict.reason, CheckReason::TrivialNoCheck);
    assert_eq!(table.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn asking_ends_with_the_text_services_refusal_when_it_has_no_more_answers() {
    let table = table(vec![with(athletics_answer(), json!({"dc_band": 14}))]).await;
    let error = judge_check(table.app(), &table.text, &player())
        .await
        .expect_err("the script runs dry");
    assert!(
        matches!(error, CheckRouteError::TextService { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_request_the_judge_cannot_use_is_refused_before_any_question() {
    let table = table(vec![athletics_answer()]).await;
    let mut request = player();
    request.action = None;
    let error = judge_check(table.app(), &table.text, &request)
        .await
        .expect_err("a player moment needs its action");
    assert_eq!(
        error,
        CheckRouteError::BadRequest {
            cause: CheckError::NeedsAction
        }
    );
    assert_eq!(table.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

fn intending(
    kind: CheckKind,
    ability: Option<Ability>,
    skill: Option<Skill>,
) -> super::request::CheckJudgeRequest {
    let mut request = narrator();
    request.intent = Some(CheckTest {
        kind,
        ability,
        skill,
    });
    request
}

fn narrators(answer: Value) -> Value {
    with(
        answer,
        json!({"initiator": "narrator", "reason": "narrator_obstacle"}),
    )
}

#[test]
fn an_intended_test_is_legal_only_at_a_narrator_moment() {
    let mut request = player();
    request.intent = Some(CheckTest {
        kind: CheckKind::AbilityCheck,
        ability: None,
        skill: Some(Skill::Acrobatics),
    });
    assert_eq!(
        check_request(&request),
        Err(CheckError::IntentNeedsNarrator)
    );
    let acrobatics = intending(CheckKind::AbilityCheck, None, Some(Skill::Acrobatics));
    assert_eq!(check_request(&acrobatics), Ok(()));
}

#[test]
fn the_judge_is_told_the_narrators_intended_test_and_only_at_a_narrator_moment_that_has_one() {
    let facts = judge_facts(&intending(
        CheckKind::AbilityCheck,
        Some(Ability::Dexterity),
        Some(Skill::Acrobatics),
    ));
    assert_eq!(
        facts["narrator_intent"],
        json!({"kind": "ability_check", "ability": "dexterity", "skill": "acrobatics"})
    );
    assert_eq!(judge_facts(&narrator())["narrator_intent"], Value::Null);
}

#[test]
fn the_intended_test_is_called_with_the_judges_own_band_mode_and_grounds() {
    let request = intending(CheckKind::AbilityCheck, None, Some(Skill::Acrobatics));
    let answer = narrators(with(
        athletics_answer(),
        json!({"skill": "acrobatics", "ability": "dexterity", "dc_band": 20,
               "mode": "disadvantage", "grounds": "armor_penalty"}),
    ));
    let call = parse_verdict(&answer, &request)
        .expect("stands")
        .call
        .expect("called");
    assert_eq!(call.skill, Some(Skill::Acrobatics));
    assert_eq!(call.dc_band, Some(DcBand::Hard));
    assert_eq!(call.mode, RollMode::Disadvantage);
}

#[test]
fn another_test_than_the_intended_one_is_refused() {
    let request = intending(CheckKind::AbilityCheck, None, Some(Skill::Acrobatics));
    let save = narrators(with(
        athletics_answer(),
        json!({"kind": "saving_throw", "skill": null, "ability": "dexterity"}),
    ));
    assert_eq!(
        parse_verdict(&save, &request),
        Err(CheckError::IntentIgnored)
    );
    let athletics = narrators(athletics_answer());
    assert_eq!(
        parse_verdict(&athletics, &request),
        Err(CheckError::IntentIgnored)
    );
    let with_ability = intending(
        CheckKind::AbilityCheck,
        Some(Ability::Strength),
        Some(Skill::Athletics),
    );
    assert!(parse_verdict(&athletics, &with_ability).is_ok());
    let wrong_ability = intending(CheckKind::SavingThrow, Some(Ability::Wisdom), None);
    let dexterity_save = narrators(with(
        athletics_answer(),
        json!({"kind": "saving_throw", "skill": null, "ability": "dexterity"}),
    ));
    assert_eq!(
        parse_verdict(&dexterity_save, &wrong_ability),
        Err(CheckError::IntentIgnored)
    );
}

#[test]
fn an_intent_that_makes_no_sense_is_refused_with_its_own_reason_and_no_other() {
    let request = intending(CheckKind::DeathSave, None, None);
    let unsuitable = with(
        trivial_answer(),
        json!({"initiator": "narrator", "reason": "intent_unsuitable"}),
    );
    let verdict = parse_verdict(&unsuitable, &request).expect("a typed refusal stands");
    assert_eq!(verdict.reason, CheckReason::IntentUnsuitable);
    assert_eq!(verdict.call, None);
    let trivial = with(trivial_answer(), json!({"initiator": "narrator"}));
    assert_eq!(
        parse_verdict(&trivial, &request),
        Err(CheckError::IntentIgnored)
    );
    assert_eq!(
        parse_verdict(&unsuitable, &narrator()),
        Err(CheckError::NoIntentToRefuse),
        "with no intent there is none to refuse"
    );
}

#[tokio::test]
async fn a_judge_that_ignores_the_intent_is_asked_again_and_the_intent_reaches_it() {
    let ignoring = narrators(athletics_answer());
    let following = narrators(with(
        athletics_answer(),
        json!({"skill": "acrobatics", "ability": "dexterity"}),
    ));
    let table = table(vec![ignoring, following]).await;
    let request = intending(CheckKind::AbilityCheck, None, Some(Skill::Acrobatics));
    let judged = judge_check(table.app(), &table.text, &request)
        .await
        .expect("the second answer stands");
    assert_eq!(judged.retried, 1);
    assert_eq!(table.facts_of(0)["narrator_intent"]["skill"], "acrobatics");
    assert!(
        table.facts_of(1)["previous_refusal"]
            .as_str()
            .unwrap()
            .contains("the narrator intends that test")
    );
    assert_eq!(
        table.lines_starting_with("PERSONA check_judge_retried"),
        ["PERSONA check_judge_retried why=intent_ignored"]
    );
}

//! The dev verb `rules check`: its grammar, the kit that runs a check through the production
//! adapter with a recorded draw, the text of the answer, and the route on a real listener.

use std::num::NonZeroU16;
use std::sync::Arc;

use df_table::ports::{ActionId, Dice, DrawRequest};
use df_table::testkit::correlation;

use axum::http::StatusCode;

use crate::client;
use crate::command::{CommandError, split_words};
use crate::dev::{DevError, ReplyError, run_dev};
use crate::dev_http::{BodyCause, DevReply, RouteError};
use crate::engine::report::UnreadableBodyCause;
use crate::engine::routes::path;
use crate::test_support::{STARTER_FILES, start};

use super::action_book::DifficultyClass;
use super::backend::{RulesBackend, dev_ports};
use super::dev_check::{
    RULES_CHECK_USAGE, RulesCheckError, RulesCheckKit, RulesCheckReply, RulesCheckReport,
    RulesCheckRequest, ask, audit_line, format_rules_check, parse, refusal_audit_line,
};
use super::dev_route;
use super::dice::RecordedDice;
use super::hero_profile::{Ability, Skill};

const SEED: u64 = 4_242;

fn words(line: &str) -> Vec<String> {
    split_words(line).expect("the line splits")
}

fn parsed(line: &str) -> Result<RulesCheckRequest, CommandError> {
    parse(&words(line)[1..])
}

fn kit() -> Arc<RulesCheckKit> {
    let check = ActionId::new("pick_lock").expect("a name");
    dev_ports(RulesBackend::DfRules, &check, SEED)
        .expect("builds")
        .check_kit
        .expect("the df-rules backend has a kit")
}

/// The first d20 the seeded dice of `SEED` roll, which is the die the kit's first check uses.
fn first_seeded_d20() -> u16 {
    let dice = RecordedDice::seeded(SEED);
    dice.draw(
        correlation(1),
        DrawRequest {
            sides: NonZeroU16::new(20).expect("sides"),
        },
    )
    .expect("a draw")
    .value()
}

fn request(skill: Skill, actor: u64, dc: Option<u8>) -> RulesCheckRequest {
    let dc = dc.map(|value| DifficultyClass::new(value).expect("a difficulty class"));
    RulesCheckRequest { skill, actor, dc }
}

fn fact(report: &RulesCheckReport, label: &str) -> Option<u64> {
    let RulesCheckReport::Resolved { facts, .. } = report else {
        panic!("not resolved: {report:?}");
    };
    facts
        .iter()
        .find(|fact| fact.label == label)
        .map(|fact| fact.amount)
}

#[test]
fn the_verb_reads_a_skill_a_seat_and_an_optional_dc() {
    assert_eq!(
        parsed("rules check perception 1"),
        Ok(request(Skill::Perception, 1, None))
    );
    assert_eq!(
        parsed("rules check sleight_of_hand 3 dc=20"),
        Ok(request(Skill::SleightOfHand, 3, Some(20)))
    );
}

#[test]
fn the_verb_names_its_usage_for_a_wrong_shape_and_the_choices_for_a_wrong_skill() {
    let usage = Err(CommandError::Usage(RULES_CHECK_USAGE));
    for line in [
        "rules check",
        "rules check perception",
        "rules roll perception 1",
        "rules check perception 1 15",
        "rules check perception 1 dc=15 dc=16",
    ] {
        assert_eq!(parsed(line), usage, "{line}");
    }
    let Err(CommandError::BadChoice { key, text, allowed }) = parsed("rules check lockpicking 1")
    else {
        panic!("a wrong skill is a bad choice");
    };
    assert_eq!((key.as_str(), text.as_str()), ("the skill", "lockpicking"));
    assert!(allowed.contains("sleight_of_hand") && allowed.contains("animal_handling"));
    assert!(matches!(
        parsed("rules check perception one"),
        Err(CommandError::BadCount {
            what: "the seat",
            ..
        })
    ));
    for dc in ["dc=high", "dc=0"] {
        assert!(
            matches!(
                parsed(&format!("rules check perception 1 {dc}")),
                Err(CommandError::BadCount {
                    what: "the difficulty class",
                    ..
                })
            ),
            "{dc}"
        );
    }
}

#[test]
fn a_check_draws_one_recorded_d20_and_reports_the_total_against_the_dc() {
    let die = u64::from(first_seeded_d20());

    let report = kit()
        .run(&request(Skill::SleightOfHand, 1, Some(15)))
        .expect("answers");

    let RulesCheckReport::Resolved {
        ability,
        difficulty_class,
        succeeded,
        draws,
        ..
    } = &report
    else {
        panic!("not resolved: {report:?}");
    };
    assert_eq!(*ability, Ability::Dexterity);
    assert_eq!(*difficulty_class, 15);
    assert_eq!(fact(&report, "natural_die"), Some(die));
    assert_eq!(fact(&report, "total"), Some(die + 4));
    assert_eq!(*succeeded, die + 4 >= 15);
    assert_eq!(draws.len(), 1);
    assert_eq!(
        (draws[0].sequence, draws[0].sides, u64::from(draws[0].value)),
        (1, 20, die)
    );
}

#[test]
fn the_dc_is_medium_when_not_given_and_each_check_adds_a_recorded_draw() {
    let kit = kit();

    let first = kit
        .run(&request(Skill::Perception, 1, None))
        .expect("answers");
    let second = kit
        .run(&request(Skill::Perception, 2, Some(10)))
        .expect("answers");

    assert_eq!(fact(&first, "difficulty_class"), Some(15));
    assert_eq!(fact(&second, "difficulty_class"), Some(10));
    let (
        RulesCheckReport::Resolved { draws: one, .. },
        RulesCheckReport::Resolved { draws: two, .. },
    ) = (&first, &second)
    else {
        panic!("not resolved");
    };
    assert_eq!((one[0].sequence, two[0].sequence), (1, 2));
}

#[test]
fn a_skill_the_hero_lacks_adds_no_proficiency_bonus() {
    let report = kit()
        .run(&request(Skill::Athletics, 1, Some(15)))
        .expect("answers");

    assert_eq!(fact(&report, "proficiency_bonus"), Some(0));
    assert_eq!(fact(&report, "ability_bonus"), Some(0));
    assert_eq!(fact(&report, "ability_penalty"), None);
}

#[test]
fn a_seat_with_no_hero_is_a_decline_and_draws_nothing() {
    let dice_check = ActionId::new("pick_lock").expect("a name");
    let ports = dev_ports(RulesBackend::DfRules, &dice_check, SEED).expect("builds");
    let kit = ports.check_kit.expect("a kit");

    let report = kit
        .run(&request(Skill::Perception, 9, None))
        .expect("answers");

    let RulesCheckReport::Declined {
        actor,
        code,
        missing_piece,
        ..
    } = &report
    else {
        panic!("not declined: {report:?}");
    };
    assert_eq!(*actor, 9);
    assert_eq!(code.key(), "missing_requirement");
    assert_eq!(missing_piece.as_deref(), Some("a hero for this seat"));
    let text = format_rules_check(&report);
    assert!(
        text.starts_with("1: seat 9 declined (missing_requirement): "),
        "{text}"
    );
    assert!(text.contains("Missing: a hero for this seat."), "{text}");
}

#[test]
fn a_dc_of_zero_never_reads_as_a_request() {
    let read = serde_json::from_str::<RulesCheckRequest>(r#"{"skill":"stealth","actor":1,"dc":0}"#);

    assert!(read.is_err(), "{read:?}");
}

#[test]
fn the_answer_is_numbered_and_names_the_check_the_facts_and_the_recorded_draw() {
    let die = first_seeded_d20();
    let report = kit()
        .run(&request(Skill::SleightOfHand, 1, Some(15)))
        .expect("answers");

    let text = format_rules_check(&report);

    let verdict = if u64::from(die) + 4 >= 15 {
        "success"
    } else {
        "failure"
    };
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(
        lines[0],
        format!("1: sleight_of_hand (dexterity) check for seat 1 against DC 15: {verdict}")
    );
    assert!(
        lines[1].starts_with(&format!("2: natural_die={die} ability_bonus=2 ")),
        "{text}"
    );
    assert_eq!(lines[2], format!("3: recorded draw 1: d20 = {die}"));
}

#[test]
fn the_audit_line_holds_closed_keys_and_numbers_only() {
    let asked = request(Skill::SleightOfHand, 1, Some(15));
    let report = kit().run(&asked).expect("answers");

    let line = audit_line(&asked, &report);

    let result = if matches!(
        report,
        RulesCheckReport::Resolved {
            succeeded: true,
            ..
        }
    ) {
        "success"
    } else {
        "failure"
    };
    assert_eq!(
        line,
        format!("ENGINE rules_check skill=sleight_of_hand seat=1 result={result}")
    );
}

#[test]
fn a_request_and_its_report_survive_json_and_an_unknown_field_is_refused() {
    let asked = request(Skill::AnimalHandling, 2, Some(12));
    let written = serde_json::to_string(&asked).expect("writes");
    assert_eq!(written, r#"{"skill":"animal_handling","actor":2,"dc":12}"#);
    assert_eq!(
        serde_json::from_str::<RulesCheckRequest>(&written).expect("reads"),
        asked
    );
    assert!(
        serde_json::from_str::<RulesCheckRequest>(
            r#"{"skill":"stealth","actor":1,"dc":null,"x":1}"#
        )
        .is_err()
    );

    let report = kit().run(&asked).expect("answers");
    let text = serde_json::to_string(&report).expect("writes");
    assert_eq!(
        serde_json::from_str::<RulesCheckReport>(&text).expect("reads"),
        report
    );
}

#[tokio::test]
async fn the_route_runs_a_check_through_the_kit_answers_json_and_audits_one_line() {
    let host = start(&STARTER_FILES).await;
    let listener = crate::server::bind(0).await.expect("binds");
    let port = listener.local_addr().expect("an address").port();
    let router = dev_route::router(Arc::clone(&host.state), Some(kit()));
    let serving = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let reply = client::post_json(
        port,
        path::RULES_CHECK,
        r#"{"skill":"sleight_of_hand","actor":1,"dc":15}"#,
    )
    .await
    .expect("answered");

    serving.abort();
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let parsed: RulesCheckReply = serde_json::from_str(&reply.body).expect("a reply");
    let DevReply::Done { value } = parsed else {
        panic!("rejected: {}", reply.body);
    };
    assert!(matches!(value, RulesCheckReport::Resolved { .. }));
    let logged = host.state.log().after_sequence(0, 100_000);
    let audit: Vec<_> = logged
        .iter()
        .filter(|line| {
            line.text
                .starts_with("ENGINE rules_check skill=sleight_of_hand seat=1 ")
        })
        .collect();
    assert_eq!(audit.len(), 1);
}

#[tokio::test]
async fn the_route_with_no_kit_says_the_check_is_not_available_and_a_bad_body_is_refused() {
    let host = start(&STARTER_FILES).await;
    let listener = crate::server::bind(0).await.expect("binds");
    let port = listener.local_addr().expect("an address").port();
    let router = dev_route::router(Arc::clone(&host.state), None);
    let serving = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let unavailable = client::post_json(
        port,
        path::RULES_CHECK,
        r#"{"skill":"stealth","actor":1,"dc":null}"#,
    )
    .await
    .expect("answered");
    let malformed = client::post_json(
        port,
        path::RULES_CHECK,
        r#"{"skill":"lockpicking","actor":1,"dc":null}"#,
    )
    .await
    .expect("answered");

    serving.abort();
    let refusal = |body: &str| match serde_json::from_str::<RulesCheckReply>(body) {
        Ok(DevReply::Rejected { error, .. }) => error,
        other => panic!("not a refusal: {other:?}"),
    };
    assert_eq!(unavailable.status, StatusCode::CONFLICT);
    assert_eq!(refusal(&unavailable.body), RulesCheckError::ScriptedBackend);
    assert_eq!(malformed.status, StatusCode::BAD_REQUEST);
    let RulesCheckError::InvalidBody(body_error) = refusal(&malformed.body) else {
        panic!("not an invalid body: {}", malformed.body);
    };
    assert_eq!(body_error.cause, BodyCause::Data);
    let logged = host.state.log().after_sequence(0, 100_000);
    let refused: Vec<_> = logged
        .iter()
        .filter(|line| line.text.starts_with("ENGINE refused verb=rules_check "))
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(refused.len(), 2, "{refused:?}");
    assert!(
        refused[0]
            .starts_with("ENGINE refused verb=rules_check error=scripted_backend duration_ms="),
        "{refused:?}"
    );
    assert!(
        refused[1].starts_with("ENGINE refused verb=rules_check error=invalid_body duration_ms="),
        "{refused:?}"
    );
}

#[tokio::test]
async fn a_body_over_the_limit_is_refused_413_typed_and_logged() {
    let host = start(&STARTER_FILES).await;
    let listener = crate::server::bind(0).await.expect("binds");
    let port = listener.local_addr().expect("an address").port();
    let router = dev_route::router(Arc::clone(&host.state), Some(kit()))
        .layer(axum::extract::DefaultBodyLimit::max(16));
    let serving = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let reply = client::post_json(
        port,
        path::RULES_CHECK,
        r#"{"skill":"sleight_of_hand","actor":1,"dc":15}"#,
    )
    .await
    .expect("answered");

    serving.abort();
    assert_eq!(
        reply.status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "{}",
        reply.body
    );
    let parsed: RulesCheckReply = serde_json::from_str(&reply.body).expect("a reply");
    let DevReply::Rejected { error, .. } = parsed else {
        panic!("not a refusal: {}", reply.body);
    };
    assert_eq!(
        error,
        RulesCheckError::UnreadableBody {
            cause: UnreadableBodyCause::TooLarge
        }
    );
    let logged = host.state.log().after_sequence(0, 100_000);
    let refused: Vec<_> = logged
        .iter()
        .filter(|line| line.text.starts_with("ENGINE refused verb=rules_check "))
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(
        refused[0]
            .starts_with("ENGINE refused verb=rules_check error=unreadable_body duration_ms="),
        "{refused:?}"
    );
}

#[tokio::test]
async fn the_dev_command_line_reaches_the_verb_and_a_scripted_table_refuses_it_by_type() {
    let host = start(&STARTER_FILES).await;
    let asked = parsed("rules check perception 1").expect("parses");

    let reply = ask(host.port, &asked).await.expect("answered");

    let DevReply::Rejected { error, message } = reply else {
        panic!("not a rejection: {reply:?}");
    };
    assert_eq!(error, RulesCheckError::ScriptedBackend);
    assert_eq!(message, error.to_string());
    // The verb as typed at the dev command line reads the same typed refusal and prints its
    // sentence.
    let refused = run_dev(host.port, &words("rules check perception 1"))
        .await
        .expect_err("the scripted backend has no check");
    assert_eq!(refused.to_string(), message);
    assert_eq!(
        refused,
        DevError::Rejected {
            error: ReplyError::RulesCheck(error),
            message,
        }
    );
    let usage = run_dev(host.port, &words("rules check"))
        .await
        .expect_err("a usage error");
    assert!(
        matches!(usage, DevError::Command(CommandError::Usage(_))),
        "{usage:?}"
    );
}

#[test]
fn every_refusal_answers_its_status_and_its_key_is_its_serde_tag() {
    let body_error =
        crate::dev_http::parse_body::<RulesCheckRequest>(b"{").expect_err("a truncated body");
    let errors = [
        (
            RulesCheckError::InvalidBody(body_error),
            StatusCode::BAD_REQUEST,
        ),
        (
            RulesCheckError::UnreadableBody {
                cause: UnreadableBodyCause::TooLarge,
            },
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
        (
            RulesCheckError::UnreadableBody {
                cause: UnreadableBodyCause::Interrupted,
            },
            StatusCode::BAD_REQUEST,
        ),
        (RulesCheckError::ScriptedBackend, StatusCode::CONFLICT),
        (
            RulesCheckError::PortFailed {
                port: df_table::ports::PortName::Dice,
                cause: df_table::event::FailureCause::Unavailable,
            },
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        (
            RulesCheckError::InvalidRoll {
                cause: df_rules::roll::RollError::UnusedDraws,
            },
            StatusCode::BAD_REQUEST,
        ),
        (
            RulesCheckError::UnexpectedResolution,
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        (
            RulesCheckError::NoVerdict,
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    ];
    for (error, status) in errors {
        assert_eq!(error.status(), status, "{error:?}");
        let written = serde_json::to_value(&error).expect("writes");
        assert_eq!(written["error"], error.key(), "{written}");
        assert_eq!(
            serde_json::from_value::<RulesCheckError>(written).expect("reads"),
            error
        );
        assert_eq!(
            refusal_audit_line(&error, 3),
            format!(
                "ENGINE refused verb=rules_check error={} duration_ms=3",
                error.key()
            )
        );
    }
}

// ---- `rules roll`: any typed roll through the production adapter with recorded draws ----

use df_rules::roll::{
    AttackKind, D20Request, D20Test, DiceExpression, DiceRequest, DieRole, DieSize,
    HeroicInspiration, ModifierLabel, PassivePerceptionRequest, Proficiency, RollError, RollMode,
    RollOutcome, RollRequest, Target, TargetLabel,
};

use super::dev_check::{
    RULES_ROLL_USAGE, RulesRollReply, RulesRollReport, format_rules_roll, parse_roll,
    roll_audit_line, roll_refusal_audit_line,
};

fn rolled(line: &str) -> Result<RollRequest, CommandError> {
    parse_roll(&words(line)[1..])
}

fn d20_request(
    test: D20Test,
    mode: RollMode,
    modifier: i8,
    proficiency: Proficiency,
    target: Option<Target>,
) -> D20Request {
    D20Request {
        test,
        mode,
        ability_modifier: modifier,
        proficiency,
        target,
        heroic_inspiration: None,
    }
}

fn d20_roll(
    test: D20Test,
    mode: RollMode,
    modifier: i8,
    proficiency: Proficiency,
    target: Option<Target>,
) -> RollRequest {
    RollRequest::D20(d20_request(test, mode, modifier, proficiency, target))
}

fn target(label: TargetLabel, value: i16) -> Option<Target> {
    Some(Target { label, value })
}

fn table_roll() -> DiceRequest {
    DiceRequest {
        role: DieRole::Table,
        expression: DiceExpression {
            count: 1,
            die: DieSize::D100,
            modifier: 0,
        },
        modifier_label: ModifierLabel::Flat,
        critical: false,
        heroic_inspiration: None,
    }
}

#[test]
fn the_roll_verb_reads_every_kind_with_its_options() {
    let proficient = Proficiency::Proficient { bonus: 2 };
    assert_eq!(
        rolled("rules roll skill athletics mod=3 prof=2 dc=15 mode=advantage"),
        Ok(d20_roll(
            D20Test::SkillCheck(Skill::Athletics),
            RollMode::Advantage,
            3,
            proficient,
            target(TargetLabel::DifficultyClass, 15)
        ))
    );
    assert_eq!(
        rolled("rules roll ability strength dc=10"),
        Ok(d20_roll(
            D20Test::AbilityCheck(Ability::Strength),
            RollMode::Normal,
            0,
            Proficiency::None,
            target(TargetLabel::DifficultyClass, 10)
        ))
    );
    assert_eq!(
        rolled("rules roll save constitution mod=2 prof=2 dc=13"),
        Ok(d20_roll(
            D20Test::SavingThrow(Ability::Constitution),
            RollMode::Normal,
            2,
            proficient,
            target(TargetLabel::DifficultyClass, 13)
        ))
    );
    assert_eq!(
        rolled("rules roll attack melee mod=3 prof=2 ac=16 crit=19"),
        Ok(d20_roll(
            D20Test::Attack {
                ability: Ability::Strength,
                kind: AttackKind::Melee,
                critical_from: 19
            },
            RollMode::Normal,
            3,
            proficient,
            target(TargetLabel::ArmorClass, 16)
        ))
    );
    assert_eq!(
        rolled("rules roll attack ranged mod=2 ac=12 mode=disadvantage"),
        Ok(d20_roll(
            D20Test::Attack {
                ability: Ability::Dexterity,
                kind: AttackKind::Ranged,
                critical_from: 20
            },
            RollMode::Disadvantage,
            2,
            Proficiency::None,
            target(TargetLabel::ArmorClass, 12)
        ))
    );
    assert_eq!(
        rolled("rules roll skill perception dc=15 stealth_total=17").map(|_| ()),
        Err(CommandError::Usage(RULES_ROLL_USAGE))
    );
    assert_eq!(
        rolled("rules roll initiative mod=2 mode=advantage"),
        Ok(d20_roll(
            D20Test::Initiative,
            RollMode::Advantage,
            2,
            Proficiency::None,
            None
        ))
    );
    assert_eq!(
        rolled("rules roll death_save reroll=0"),
        Ok(RollRequest::D20(D20Request {
            heroic_inspiration: Some(HeroicInspiration { replaces_die: 0 }),
            ..d20_request(
                D20Test::DeathSave,
                RollMode::Normal,
                0,
                Proficiency::None,
                None
            )
        }))
    );
    assert_eq!(
        rolled("rules roll passive_perception mod=2 expertise=2 mode=advantage"),
        Ok(RollRequest::PassivePerception(PassivePerceptionRequest {
            ability_modifier: 2,
            proficiency: Proficiency::Expertise { bonus: 2 },
            mode: RollMode::Advantage,
        }))
    );
    assert_eq!(
        rolled("rules roll dice damage 2d6+3 label=strength critical=true"),
        Ok(RollRequest::Dice(DiceRequest {
            role: DieRole::Damage,
            expression: DiceExpression {
                count: 2,
                die: DieSize::D6,
                modifier: 3
            },
            modifier_label: ModifierLabel::Strength,
            critical: true,
            heroic_inspiration: None,
        }))
    );
}

#[test]
fn the_roll_verb_names_what_is_wrong_and_never_ignores_a_word() {
    let usage = Err(CommandError::Usage(RULES_ROLL_USAGE));
    for line in ["rules roll", "rules roll skill", "rules roll dice damage"] {
        assert_eq!(rolled(line), usage, "{line}");
    }
    assert_eq!(rolled("rules roll attack melee ac=10 dc=10"), usage);
    assert_eq!(rolled("rules roll initiative prof=2 expertise=2"), usage);
    assert_eq!(
        rolled("rules roll initiative mod=1 mod=2"),
        Err(CommandError::DuplicateKey("mod".to_owned()))
    );
    assert!(matches!(
        rolled("rules roll initiative 15"),
        Err(CommandError::BadPair(_))
    ));
    for line in [
        "rules roll lockpick",
        "rules roll skill lockpicking",
        "rules roll ability might",
        "rules roll attack thrown ac=10",
        "rules roll initiative mode=lucky",
        "rules roll dice loot 1d6",
        "rules roll dice damage 1d7",
        "rules roll dice damage 1d6 critical=maybe",
        "rules roll initiative crit=19",
        "rules roll dice damage 1d6 mod=2",
    ] {
        assert!(
            matches!(rolled(line), Err(CommandError::BadChoice { .. })),
            "{line}: {:?}",
            rolled(line)
        );
    }
    assert!(matches!(
        rolled("rules roll initiative mod=high"),
        Err(CommandError::BadCount { what: "mod", .. })
    ));
}

fn roll_report(request: &RollRequest) -> RulesRollReport {
    kit().roll(request).expect("answers")
}

#[test]
fn a_roll_draws_each_die_of_its_plan_from_the_recording_dice_and_reports_them_in_order() {
    let request = d20_roll(
        D20Test::SkillCheck(Skill::Athletics),
        RollMode::Advantage,
        3,
        Proficiency::Proficient { bonus: 2 },
        target(TargetLabel::DifficultyClass, 15),
    );
    let seeded = RecordedDice::seeded(SEED);
    let expected: Vec<u16> = (0..2)
        .map(|_| {
            seeded
                .draw(
                    correlation(1),
                    DrawRequest {
                        sides: NonZeroU16::new(20).expect("sides"),
                    },
                )
                .expect("a draw")
                .value()
        })
        .collect();

    let report = roll_report(&request);

    let drawn: Vec<(u64, u16, u16)> = report
        .draws
        .iter()
        .map(|draw| (draw.sequence, draw.sides, draw.value))
        .collect();
    assert_eq!(drawn, [(1, 20, expected[0]), (2, 20, expected[1])]);
    let faces: Vec<u16> = report.result.dice.iter().map(|die| die.face).collect();
    assert_eq!(faces, expected);
    let kept = expected[0].max(expected[1]);
    assert_eq!(report.result.total, i16::try_from(kept).expect("small") + 5);
    assert_eq!(report.audit.len(), 1);
    assert!(
        report.audit[0].starts_with("RULES rolled action=dev_roll operation="),
        "{}",
        report.audit[0]
    );
    assert!(
        report.audit[0].contains(" subject=skill_check:athletics mode=advantage dice=d20:"),
        "{}",
        report.audit[0]
    );
}

#[test]
fn every_kind_of_roll_resolves_through_the_adapter_with_the_dice_its_plan_names() {
    let dc = target(TargetLabel::DifficultyClass, 12);
    let requests = [
        d20_roll(
            D20Test::AbilityCheck(Ability::Wisdom),
            RollMode::Normal,
            1,
            Proficiency::None,
            dc,
        ),
        d20_roll(
            D20Test::SavingThrow(Ability::Dexterity),
            RollMode::Disadvantage,
            2,
            Proficiency::None,
            dc,
        ),
        d20_roll(
            D20Test::Attack {
                ability: Ability::Strength,
                kind: AttackKind::Melee,
                critical_from: 19,
            },
            RollMode::Advantage,
            3,
            Proficiency::Proficient { bonus: 2 },
            target(TargetLabel::ArmorClass, 15),
        ),
        d20_roll(
            D20Test::Initiative,
            RollMode::Advantage,
            2,
            Proficiency::None,
            None,
        ),
        d20_roll(
            D20Test::DeathSave,
            RollMode::Normal,
            0,
            Proficiency::None,
            None,
        ),
        RollRequest::PassivePerception(PassivePerceptionRequest {
            ability_modifier: 1,
            proficiency: Proficiency::Proficient { bonus: 2 },
            mode: RollMode::Normal,
        }),
        RollRequest::Dice(DiceRequest::damage(
            DiceExpression {
                count: 3,
                die: DieSize::D8,
                modifier: 2,
            },
            Ability::Strength,
            true,
        )),
        RollRequest::Dice(table_roll()),
    ];

    for request in requests {
        let report = roll_report(&request);

        let planned = request.draw_plan().expect("a plan");
        assert_eq!(report.draws.len(), planned.len(), "{request:?}");
        let faces: Vec<u16> = report.draws.iter().map(|draw| draw.value).collect();
        assert_eq!(
            request
                .resolve(&faces)
                .expect("resolves from the recorded faces"),
            report.result,
            "{request:?}"
        );
    }
}

#[test]
fn heroic_inspiration_draws_one_more_die_and_the_recorded_faces_reproduce_the_result() {
    let request = RollRequest::D20(D20Request {
        heroic_inspiration: Some(HeroicInspiration { replaces_die: 0 }),
        ..d20_request(
            D20Test::SkillCheck(Skill::Stealth),
            RollMode::Normal,
            2,
            Proficiency::None,
            target(TargetLabel::DifficultyClass, 15),
        )
    });

    let report = roll_report(&request);

    assert_eq!(report.draws.len(), 2);
    assert_eq!(
        report.result.inspiration_reroll.map(|die| die.face),
        Some(report.draws[0].value)
    );
    assert_eq!(report.result.dice[0].face, report.draws[1].value);
    let faces: Vec<u16> = report.draws.iter().map(|draw| draw.value).collect();
    assert_eq!(request.resolve(&faces), Ok(report.result));
}

#[test]
fn a_roll_df_rules_refuses_is_a_typed_refusal_and_draws_nothing() {
    let refused = d20_roll(
        D20Test::AbilityCheck(Ability::Strength),
        RollMode::Normal,
        0,
        Proficiency::None,
        None,
    );

    let error = kit().roll(&refused).expect_err("a check needs a target");

    assert_eq!(
        error,
        RulesCheckError::InvalidRoll {
            cause: RollError::MissingTarget
        }
    );
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error.key(), "invalid_roll");
    assert_eq!(
        roll_refusal_audit_line(&error, 3),
        "ENGINE refused verb=rules_roll error=invalid_roll duration_ms=3"
    );
    let written = serde_json::to_value(&error).expect("writes");
    assert_eq!(
        serde_json::from_value::<RulesCheckError>(written).expect("reads"),
        error
    );
}

#[test]
fn the_text_of_a_roll_names_the_result_every_die_the_modifiers_the_source_and_the_audit() {
    let request = d20_roll(
        D20Test::Attack {
            ability: Ability::Strength,
            kind: AttackKind::Melee,
            critical_from: 19,
        },
        RollMode::Advantage,
        3,
        Proficiency::Proficient { bonus: 2 },
        target(TargetLabel::ArmorClass, 15),
    );
    let report = roll_report(&request);

    let text = format_rules_roll(&report);

    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[0].starts_with("1: attack:melee advantage: "),
        "{text}"
    );
    assert!(lines[0].contains("against armor_class 15"), "{text}");
    assert!(lines[1].starts_with("2: dice d20:"), "{text}");
    assert_eq!(lines[2], "3: modifiers strength +3, proficiency +2");
    assert!(
        text.contains("source SRD-5.2.1:page-7:D20-Tests:Attack-Rolls"),
        "{text}"
    );
    assert!(text.contains("recorded draw 1: d20 = "), "{text}");
    assert!(text.contains("RULES rolled action=dev_roll"), "{text}");
    assert!(matches!(
        report.result.outcome,
        RollOutcome::Hit | RollOutcome::Miss | RollOutcome::CriticalHit
    ));
    assert_eq!(
        roll_audit_line(&request, &report),
        format!(
            "ENGINE rules_roll kind=d20 subject=attack:melee outcome={}",
            report.result.outcome.key()
        )
    );
}

#[tokio::test]
async fn the_roll_route_answers_json_audits_one_line_and_refuses_a_bad_roll_by_type() {
    let host = start(&STARTER_FILES).await;
    let listener = crate::server::bind(0).await.expect("binds");
    let port = listener.local_addr().expect("an address").port();
    let router = dev_route::router(Arc::clone(&host.state), Some(kit()));
    let serving = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let damage = serde_json::to_string(&RollRequest::Dice(DiceRequest::damage(
        DiceExpression {
            count: 2,
            die: DieSize::D6,
            modifier: 3,
        },
        Ability::Strength,
        false,
    )))
    .expect("writes");
    let test_role = serde_json::to_string(&RollRequest::Dice(DiceRequest {
        role: DieRole::Test,
        ..table_roll()
    }))
    .expect("writes");

    let answered = client::post_json(port, path::RULES_ROLL, &damage)
        .await
        .expect("answered");
    let refused = client::post_json(port, path::RULES_ROLL, &test_role)
        .await
        .expect("answered");

    serving.abort();
    assert_eq!(answered.status, StatusCode::OK, "{}", answered.body);
    let parsed: RulesRollReply = serde_json::from_str(&answered.body).expect("a reply");
    let DevReply::Done { value } = parsed else {
        panic!("rejected: {}", answered.body);
    };
    assert_eq!(value.result.dice.len(), 2);
    assert_eq!(value.audit.len(), 2, "the rules line and the engine line");
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
    let DevReply::Rejected { error, .. } =
        serde_json::from_str::<RulesRollReply>(&refused.body).expect("a reply")
    else {
        panic!("not a refusal: {}", refused.body);
    };
    assert_eq!(
        error,
        RulesCheckError::InvalidRoll {
            cause: RollError::InvalidRole
        }
    );
    let logged = host.state.log().after_sequence(0, 100_000);
    let lines: Vec<&str> = logged.iter().map(|line| line.text.as_str()).collect();
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("ENGINE rules_roll kind=dice subject=damage "))
            .count(),
        1
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line
                .starts_with("ENGINE refused verb=rules_roll error=invalid_roll duration_ms="))
            .count(),
        1
    );
}

#[tokio::test]
async fn the_dev_command_line_reaches_the_roll_verb_and_a_scripted_table_refuses_it_by_type() {
    let host = start(&STARTER_FILES).await;

    let refused = run_dev(host.port, &words("rules roll initiative mod=2"))
        .await
        .expect_err("the scripted backend has no roll kit");

    assert_eq!(
        refused,
        DevError::Rejected {
            error: ReplyError::RulesCheck(RulesCheckError::ScriptedBackend),
            message: RulesCheckError::ScriptedBackend.to_string(),
        }
    );
    let usage = run_dev(host.port, &words("rules roll"))
        .await
        .expect_err("a usage error");
    assert!(
        matches!(usage, DevError::Command(CommandError::Usage(_))),
        "{usage:?}"
    );
}

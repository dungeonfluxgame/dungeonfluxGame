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
    assert_eq!(
        refused,
        [
            "ENGINE refused verb=rules_check error=scripted_backend",
            "ENGINE refused verb=rules_check error=invalid_body",
        ]
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
        (RulesCheckError::ScriptedBackend, StatusCode::CONFLICT),
        (
            RulesCheckError::PortFailed {
                port: df_table::ports::PortName::Dice,
                cause: df_table::event::FailureCause::Unavailable,
            },
            StatusCode::SERVICE_UNAVAILABLE,
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
            refusal_audit_line(&error),
            format!("ENGINE refused verb=rules_check error={}", error.key())
        );
    }
}

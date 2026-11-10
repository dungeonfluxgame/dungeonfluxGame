//! The roll path of the rules adapter: the dice a plan asks for, what it does with draws that do
//! not fit and the lines it writes.

use std::num::{NonZeroU16, NonZeroU128};

use df_rules::roll::{
    D20Request, D20Test, DiceExpression, DiceRequest, DieRole, DieSize, ModifierLabel, Proficiency,
    RollError, RollMode, RollRequest, Target, TargetLabel,
};
use df_table::identity::OperationId;
use df_table::ports::{ActionId, Draw, DrawRequest};

use super::audit::{RulesRefusal, refused_line, rolled_line};
use super::roll::{RollStep, prepare_roll, resolve_roll, step};

fn die(sides: u16, value: u16) -> Draw {
    Draw::new(NonZeroU16::new(sides).expect("sides"), value).expect("on the die")
}

fn action() -> ActionId {
    ActionId::new("grapple_escape").expect("a valid action name")
}

fn operation() -> OperationId {
    OperationId::from_nonzero_u128(NonZeroU128::new(9).expect("not zero"))
}

fn advantage_check() -> RollRequest {
    RollRequest::D20(D20Request {
        test: D20Test::SkillCheck(df_rules::roll::Skill::Athletics),
        mode: RollMode::Advantage,
        ability_modifier: 3,
        proficiency: Proficiency::Proficient { bonus: 2 },
        target: Some(Target {
            label: TargetLabel::EscapeDc,
            value: 13,
        }),
        heroic_inspiration: None,
    })
}

fn percentile() -> RollRequest {
    RollRequest::Dice(DiceRequest {
        role: DieRole::Table,
        expression: DiceExpression {
            count: 1,
            die: DieSize::D100,
            modifier: 0,
        },
        modifier_label: ModifierLabel::Flat,
        critical: false,
        heroic_inspiration: None,
    })
}

#[test]
fn the_dice_a_roll_needs_are_the_port_requests_of_its_plan() {
    let requests = prepare_roll(&action(), operation(), &advantage_check()).expect("a plan");
    let percentile_requests = prepare_roll(&action(), operation(), &percentile()).expect("a plan");

    let sides =
        |requests: &[DrawRequest]| requests.iter().map(|r| r.sides.get()).collect::<Vec<_>>();
    assert_eq!(sides(&requests), [20, 20]);
    assert_eq!(sides(&percentile_requests), [10, 10]);
}

#[test]
fn a_roll_with_dice_still_missing_asks_for_exactly_those_and_writes_no_result() {
    let step =
        resolve_roll(&action(), operation(), &advantage_check(), &[die(20, 7)]).expect("an answer");

    let RollStep::NeedsDraws(missing) = step else {
        panic!("not a request for dice");
    };
    assert_eq!(
        missing.iter().map(|r| r.sides.get()).collect::<Vec<_>>(),
        [20]
    );
}

#[test]
fn draws_that_are_not_the_dice_of_the_plan_are_unfit() {
    let request = percentile();

    assert_eq!(
        step(&request, &[die(100, 50)]),
        Err(RulesRefusal::DrawsUnfit),
        "a d100 is not a pair of d10"
    );
    assert_eq!(
        step(&request, &[die(10, 1), die(10, 2), die(10, 3)]),
        Err(RulesRefusal::DrawsUnfit),
        "more dice than the plan"
    );
    assert_eq!(
        step(&advantage_check(), &[die(20, 7), die(12, 7)]),
        Err(RulesRefusal::DrawsUnfit),
        "the second die is a d12"
    );
}

#[test]
fn a_complete_roll_resolves_and_its_line_carries_every_die_the_modifiers_and_the_outcome() {
    let draws = [die(20, 7), die(20, 11)];

    let RollStep::Rolled(result) =
        resolve_roll(&action(), operation(), &advantage_check(), &draws).expect("resolves")
    else {
        panic!("not resolved");
    };

    assert_eq!(result.total, 16);
    assert_eq!(
        rolled_line(&action(), operation(), &result),
        format!(
            "RULES rolled action=grapple_escape operation={} subject=skill_check:athletics \
             mode=advantage dice=d20:7:set_aside,d20:11:kept modifiers=strength:3,proficiency:2 \
             total=16 target=escape_dc:13 outcome=success natural=none replaced=-",
            operation()
        )
    );
}

#[test]
fn a_percentile_roll_line_marks_the_tens_and_the_ones() {
    let RollStep::Rolled(result) = resolve_roll(
        &action(),
        operation(),
        &percentile(),
        &[die(10, 10), die(10, 10)],
    )
    .expect("resolves") else {
        panic!("not resolved");
    };

    let line = rolled_line(&action(), operation(), &result);

    assert!(
        line.contains("dice=d10:10:kept:tens,d10:10:kept:ones modifiers=- total=100 target=-"),
        "{line}"
    );
}

#[test]
fn a_request_df_rules_refuses_has_a_refusal_line_with_the_roll_error_key() {
    let refused = RollRequest::D20(D20Request {
        target: None,
        ..match advantage_check() {
            RollRequest::D20(request) => request,
            _ => unreachable!("a d20 roll"),
        }
    });

    let error = prepare_roll(&action(), operation(), &refused).expect_err("no target");

    assert_eq!(error.cause, df_table::event::FailureCause::InvalidAnswer);
    assert_eq!(
        refused_line(
            &action(),
            operation(),
            RulesRefusal::RollRefused(RollError::MissingTarget)
        ),
        format!(
            "RULES refused action=grapple_escape operation={} cause=roll_refused \
             error=missing_target",
            operation()
        )
    );
}

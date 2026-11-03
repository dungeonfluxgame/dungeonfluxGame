//! The `RulesOwner` contract, run against `df-table`'s scripted fake and against the production
//! adapter with the same function (O.5: the same contract tests run against the adapter and the
//! fake). A behaviour only one of them has is tested in `rules_tests`; only what both promise is
//! here.

use std::num::NonZeroU16;

use df_table::identity::SeatId;
use df_table::outcome::DeclineCode;
use df_table::ports::{
    ActionId, Dice, Draw, DrawRequest, PlanSoFar, Preparation, Resolution, Resumption, RulesOwner,
};
use df_table::testkit::{QueuedDice, ScriptedRulesOwner, correlation};

use super::action_book::{ActionBook, DifficultyClass, MissingPiece, RulesEntry};
use super::dice::RecordedDice;
use super::hero_profile::{HeroProfile, SeatRoster, Skill};
use super::rules::DfRulesOwner;

const SEAT: SeatId = SeatId::new(1);

fn action(name: &str) -> ActionId {
    ActionId::new(name).expect("a valid action name")
}

fn d20_request() -> DrawRequest {
    DrawRequest {
        sides: NonZeroU16::new(20).expect("sides"),
    }
}

fn d20(value: u16) -> Draw {
    Draw::new(d20_request().sides, value).expect("on the die")
}

/// What the contract asks of an owner: a seat, one action that is supported and rolls one d20, and
/// one action that is not supported yet.
struct Fixture {
    supported: ActionId,
    unsupported: ActionId,
}

fn fixture() -> Fixture {
    Fixture {
        supported: action("pick_lock"),
        unsupported: action("cast_light"),
    }
}

fn scripted_fake(fixture: &Fixture) -> ScriptedRulesOwner {
    ScriptedRulesOwner::default()
        .with_choices(SEAT, vec![fixture.supported.clone()])
        .with_action(
            fixture.supported.clone(),
            Preparation::Ready {
                draws: vec![d20_request()],
            },
            Resolution::applied(),
        )
        .with_unsupported_action(fixture.unsupported.clone())
}

fn production_adapter(fixture: &Fixture) -> DfRulesOwner {
    let hero = HeroProfile::new([10, 14, 10, 10, 10, 10], 2, [Skill::SleightOfHand])
        .expect("a valid hero");
    let roster = SeatRoster::default()
        .with_hero(SEAT, hero)
        .expect("one hero");
    let book = ActionBook::default()
        .with_entry(
            fixture.supported.clone(),
            RulesEntry::skill_check(Skill::SleightOfHand, DifficultyClass::MEDIUM),
        )
        .and_then(|book| {
            book.with_entry(
                fixture.unsupported.clone(),
                RulesEntry::Unsupported(MissingPiece::Spellcasting),
            )
        })
        .expect("registers");
    DfRulesOwner::new(roster, book)
}

fn declined_not_supported_yet(preparation: Preparation) -> bool {
    matches!(preparation, Preparation::Declined(decline) if decline.code() == DeclineCode::NotSupportedYet)
}

/// Every promise of the port that the fake and the production adapter both keep.
fn the_rules_owner_contract(owner: &dyn RulesOwner, fixture: &Fixture) {
    let supported = &fixture.supported;
    let unsupported = &fixture.unsupported;
    let roll = [d20(14)];

    let choices = owner.legal_choices(correlation(1), SEAT).expect("answers");
    assert!(choices.contains(supported), "the supported action is legal");
    assert!(
        !choices.contains(unsupported),
        "the unsupported action is not"
    );

    let prepared = owner
        .prepare(correlation(1), SEAT, supported)
        .expect("answers");
    assert_eq!(
        prepared,
        Preparation::Ready {
            draws: vec![d20_request()]
        },
        "a check asks for one d20"
    );
    let declined = owner
        .prepare(correlation(1), SEAT, unsupported)
        .expect("answers");
    assert!(
        declined_not_supported_yet(declined),
        "an unsupported action declines not_supported_yet when prepared"
    );

    let resolved = owner
        .resolve(correlation(1), SEAT, supported, &roll)
        .expect("answers");
    assert!(matches!(resolved, Resolution::Applied(_)), "{resolved:?}");
    let again = owner
        .resolve(correlation(1), SEAT, supported, &roll)
        .expect("answers");
    assert_eq!(resolved, again, "the same draws give the same resolution");
    let refused = owner
        .resolve(correlation(1), SEAT, unsupported, &roll)
        .expect("answers");
    assert!(
        matches!(&refused, Resolution::Declined(decline) if decline.code() == DeclineCode::NotSupportedYet),
        "an unsupported action declines not_supported_yet when resolved: {refused:?}"
    );

    assert_eq!(
        owner
            .confirmation_needed(correlation(1), SEAT, supported)
            .expect("answers"),
        None,
        "a check harms no one"
    );
    assert_eq!(
        owner
            .ruling_needed(correlation(1), SEAT, supported)
            .expect("answers"),
        None,
        "the source leaves a check no open ruling"
    );
    assert!(
        owner
            .abandon(
                correlation(1),
                SEAT,
                supported,
                df_table::envelope::PendingId::new(1)
            )
            .is_ok()
    );

    let in_plan = owner
        .prepare_in_plan(
            correlation(1),
            SEAT,
            supported,
            &PlanSoFar {
                steps: &[],
                effects: &[],
            },
        )
        .expect("answers");
    assert_eq!(
        in_plan, prepared,
        "a first step is checked like a lone action"
    );
    let resumed = owner
        .resume(
            correlation(1),
            SEAT,
            supported,
            &Resumption {
                draws: &roll,
                answers: &[],
                ruling: None,
            },
        )
        .expect("answers");
    assert_eq!(resumed, resolved, "resuming with the dice resolves");
}

#[test]
fn the_scripted_fake_keeps_the_rules_owner_contract() {
    let fixture = fixture();

    the_rules_owner_contract(&scripted_fake(&fixture), &fixture);
}

#[test]
fn the_production_adapter_keeps_the_rules_owner_contract() {
    let fixture = fixture();

    the_rules_owner_contract(&production_adapter(&fixture), &fixture);
}

/// Every promise of the dice port that the fake and the production dice both keep: a draw is a die
/// of the sides asked for, with a face on it.
fn the_dice_contract(dice: &dyn Dice) {
    for sides in [20, 20, 20, 1] {
        let request = DrawRequest {
            sides: NonZeroU16::new(sides).expect("sides"),
        };

        let drawn = dice.draw(correlation(1), request).expect("a draw");

        assert_eq!(drawn.sides(), request.sides);
        assert!((1..=sides).contains(&drawn.value()), "{drawn:?}");
    }
}

#[test]
fn the_queued_fake_dice_keep_the_dice_contract() {
    the_dice_contract(&QueuedDice::new([20, 1, 13, 1]));
}

#[test]
fn the_recorded_production_dice_keep_the_dice_contract() {
    the_dice_contract(&RecordedDice::seeded(5));
    the_dice_contract(&RecordedDice::from_system_entropy());
}

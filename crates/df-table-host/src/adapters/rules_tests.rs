//! The production rules owner against `df-rules` and the SRD: each supported call, each decline,
//! the explicit-draw rules and the d20 boundary draws.

use std::num::NonZeroU16;

use df_rules::influence::{Attitude, InfluenceSkill, Willingness, WordQuality, WordTilt};
use df_table::event::FailureCause;
use df_table::identity::SeatId;
use df_table::outcome::DeclineCode;
use df_table::ports::{
    ActionId, ConfirmationReason, Draw, DrawRequest, PlanSoFar, PortName, Preparation, Resolution,
    Resumption, RulesOwner,
};
use df_table::testkit::correlation;

use super::action_book::{ActionBook, DifficultyClass, InfluenceSetup, MissingPiece, RulesEntry};
use super::hero_profile::{HeroProfile, SeatRoster, Skill};
use super::rules::DfRulesOwner;

const SEAT: SeatId = SeatId::new(1);

fn action(name: &str) -> ActionId {
    ActionId::new(name).expect("a valid action name")
}

fn draw(sides: u16, value: u16) -> Draw {
    Draw::new(NonZeroU16::new(sides).expect("sides"), value).expect("on the die")
}

fn d20(value: u16) -> Draw {
    draw(20, value)
}

fn dc(value: u8) -> DifficultyClass {
    DifficultyClass::new(value).expect("a DC")
}

/// A hero who is proficient in Sleight of Hand with Dexterity 14 (+2) and a +2 bonus: +4.
fn thief() -> HeroProfile {
    HeroProfile::new([10, 14, 10, 10, 10, 10], 2, [Skill::SleightOfHand]).expect("valid")
}

fn owner_of(hero: HeroProfile, book: ActionBook) -> DfRulesOwner {
    let roster = SeatRoster::default()
        .with_hero(SEAT, hero)
        .expect("one hero");
    DfRulesOwner::new(roster, book)
}

fn check_book(skill: Skill, difficulty_class: u8) -> ActionBook {
    ActionBook::default()
        .with_entry(
            action("check"),
            RulesEntry::skill_check(skill, dc(difficulty_class)),
        )
        .expect("registers")
}

fn fact(resolution: &Resolution, label: &str) -> Option<u64> {
    let Resolution::Applied(resolved) = resolution else {
        panic!("not applied: {resolution:?}");
    };
    resolved
        .facts
        .iter()
        .find(|fact| fact.label.as_str() == label)
        .map(|fact| fact.amount)
}

fn resolve(owner: &DfRulesOwner, name: &str, draws: &[Draw]) -> Resolution {
    owner
        .resolve(correlation(1), SEAT, &action(name), draws)
        .expect("answers")
}

fn prepared_draws(owner: &DfRulesOwner, name: &str) -> usize {
    match owner
        .prepare(correlation(1), SEAT, &action(name))
        .expect("answers")
    {
        Preparation::Ready { draws } => {
            assert!(draws.iter().all(|request| request.sides.get() == 20));
            draws.len()
        }
        Preparation::Declined(decline) => panic!("declined: {decline:?}"),
    }
}

#[test]
fn a_skill_check_needs_one_d20_and_succeeds_when_the_total_equals_the_dc() {
    let owner = owner_of(thief(), check_book(Skill::SleightOfHand, 15));

    assert_eq!(prepared_draws(&owner, "check"), 1);
    let meets = resolve(&owner, "check", &[d20(11)]);
    let misses = resolve(&owner, "check", &[d20(10)]);

    assert_eq!(fact(&meets, "total"), Some(15));
    assert_eq!(fact(&meets, "succeeded"), Some(1));
    assert_eq!(fact(&misses, "total"), Some(14));
    assert_eq!(fact(&misses, "succeeded"), Some(0));
}

#[test]
fn a_check_reports_the_die_the_bonuses_and_the_dc_as_typed_facts_and_changes_no_table_state() {
    let owner = owner_of(thief(), check_book(Skill::SleightOfHand, 15));

    let resolution = resolve(&owner, "check", &[d20(14)]);

    let Resolution::Applied(resolved) = &resolution else {
        panic!("not applied");
    };
    assert!(resolved.effects.is_empty());
    let facts: Vec<_> = resolved
        .facts
        .iter()
        .map(|fact| (fact.label.as_str(), fact.amount))
        .collect();
    assert_eq!(
        facts,
        [
            ("natural_die", 14),
            ("ability_bonus", 2),
            ("proficiency_bonus", 2),
            ("total", 18),
            ("difficulty_class", 15),
            ("succeeded", 1),
        ]
    );
}

#[test]
fn a_hero_without_the_skill_adds_no_proficiency_bonus() {
    let owner = owner_of(thief(), check_book(Skill::Stealth, 15));

    let resolution = resolve(&owner, "check", &[d20(10)]);

    assert_eq!(fact(&resolution, "proficiency_bonus"), Some(0));
    assert_eq!(fact(&resolution, "total"), Some(12));
}

#[test]
fn a_natural_1_does_not_fail_an_ability_check_that_the_modifiers_carry_to_the_dc() {
    // SRD 5.2.1 page 6: only an attack roll is decided by a natural 1 or 20. Strength 30 (+10)
    // and a +6 proficiency bonus make 1 + 16 = 17, which meets DC 15.
    let hero = HeroProfile::new([30, 10, 10, 10, 10, 10], 6, [Skill::Athletics]).expect("valid");
    let owner = owner_of(hero, check_book(Skill::Athletics, 15));

    let resolution = resolve(&owner, "check", &[d20(1)]);

    assert_eq!(fact(&resolution, "natural_die"), Some(1));
    assert_eq!(fact(&resolution, "total"), Some(17));
    assert_eq!(fact(&resolution, "succeeded"), Some(1));
}

#[test]
fn a_natural_20_does_not_pass_an_ability_check_that_the_total_leaves_short_of_the_dc() {
    // Charisma 1 (-5), no proficiency: 20 - 5 = 15 against DC 30.
    let hero = HeroProfile::new([10, 10, 10, 10, 10, 1], 2, []).expect("valid");
    let owner = owner_of(hero, check_book(Skill::Persuasion, 30));

    let resolution = resolve(&owner, "check", &[d20(20)]);

    assert_eq!(fact(&resolution, "natural_die"), Some(20));
    assert_eq!(fact(&resolution, "ability_penalty"), Some(5));
    assert_eq!(fact(&resolution, "ability_bonus"), None);
    assert_eq!(fact(&resolution, "total"), Some(15));
    assert_eq!(fact(&resolution, "succeeded"), Some(0));
}

#[test]
fn a_total_below_zero_is_reported_as_a_negative_total() {
    let hero = HeroProfile::new([10, 10, 10, 10, 10, 1], 2, []).expect("valid");
    let owner = owner_of(hero, check_book(Skill::Persuasion, 10));

    let resolution = resolve(&owner, "check", &[d20(1)]);

    assert_eq!(fact(&resolution, "negative_total"), Some(4));
    assert_eq!(fact(&resolution, "total"), None);
}

#[test]
fn the_same_draws_give_the_same_resolution_and_other_draws_can_differ() {
    let owner = owner_of(thief(), check_book(Skill::SleightOfHand, 15));

    let first = resolve(&owner, "check", &[d20(9)]);
    let again = resolve(&owner, "check", &[d20(9)]);
    let other = resolve(&owner, "check", &[d20(12)]);

    assert_eq!(first, again);
    assert_ne!(first, other);
}

#[test]
fn too_few_draws_ask_for_the_missing_d20_and_other_draws_are_refused() {
    let owner = owner_of(thief(), check_book(Skill::SleightOfHand, 15));

    let none = resolve(&owner, "check", &[]);
    let two = owner
        .resolve(correlation(1), SEAT, &action("check"), &[d20(3), d20(4)])
        .expect_err("two dice do not fit a check");
    let wrong_die = owner
        .resolve(correlation(1), SEAT, &action("check"), &[draw(6, 3)])
        .expect_err("a d6 does not fit a check");

    assert_eq!(
        none,
        Resolution::NeedsDraws(vec![DrawRequest {
            sides: NonZeroU16::new(20).expect("sides")
        }])
    );
    for error in [two, wrong_die] {
        assert_eq!(
            (error.port, error.cause),
            (PortName::RulesOwner, FailureCause::InvalidAnswer)
        );
    }
}

#[test]
fn a_seat_with_no_hero_has_no_choices_and_is_declined_missing_requirement() {
    let owner = owner_of(thief(), check_book(Skill::SleightOfHand, 15));
    let stranger = SeatId::new(9);

    let choices = owner
        .legal_choices(correlation(1), stranger)
        .expect("answers");
    let prepared = owner
        .prepare(correlation(1), stranger, &action("check"))
        .expect("answers");
    let resolved = owner
        .resolve(correlation(1), stranger, &action("check"), &[d20(10)])
        .expect("answers");

    assert!(choices.is_empty());
    let Preparation::Declined(prepared) = prepared else {
        panic!("not declined");
    };
    let Resolution::Declined(resolved) = resolved else {
        panic!("not declined");
    };
    for decline in [prepared, resolved] {
        assert_eq!(decline.code(), DeclineCode::MissingRequirement);
        assert_eq!(decline.missing_piece(), Some("a hero for this seat"));
    }
}

#[test]
fn legal_choices_are_the_runnable_actions_in_name_order_without_the_unsupported_ones() {
    let book = ActionBook::default()
        .with_entry(
            action("pick_lock"),
            RulesEntry::skill_check(Skill::SleightOfHand, dc(15)),
        )
        .and_then(|book| {
            book.with_entry(
                action("cast_light"),
                RulesEntry::Unsupported(MissingPiece::Spellcasting),
            )
        })
        .and_then(|book| {
            book.with_entry(
                action("climb"),
                RulesEntry::skill_check(Skill::Athletics, dc(10)),
            )
        })
        .expect("registers");
    let owner = owner_of(thief(), book);

    let choices = owner.legal_choices(correlation(1), SEAT).expect("answers");

    assert_eq!(choices, [action("climb"), action("pick_lock")]);
}

#[test]
fn every_missing_piece_is_declined_not_supported_yet_and_names_what_is_missing() {
    for piece in MissingPiece::ALL {
        let book = ActionBook::default()
            .with_entry(action("later"), RulesEntry::Unsupported(piece))
            .expect("registers");
        let owner = owner_of(thief(), book);

        let prepared = owner
            .prepare(correlation(1), SEAT, &action("later"))
            .expect("answers");
        let resolved = owner
            .resolve(correlation(1), SEAT, &action("later"), &[])
            .expect("answers");

        let (Preparation::Declined(prepared), Resolution::Declined(resolved)) =
            (prepared, resolved)
        else {
            panic!("{} did not decline", piece.key());
        };
        for decline in [prepared, resolved] {
            assert_eq!(
                decline.code(),
                DeclineCode::NotSupportedYet,
                "{}",
                piece.key()
            );
            assert_eq!(decline.missing_piece(), Some(piece.describe()));
        }
        assert!(!piece.describe().trim().is_empty());
    }
}

#[test]
fn an_action_that_is_not_in_the_book_is_declined_for_want_of_a_rules_entry() {
    let owner = owner_of(thief(), check_book(Skill::SleightOfHand, 15));

    let prepared = owner
        .prepare(correlation(1), SEAT, &action("fly_away"))
        .expect("answers");

    let Preparation::Declined(decline) = prepared else {
        panic!("not declined");
    };
    assert_eq!(decline.code(), DeclineCode::NotSupportedYet);
    assert_eq!(
        decline.missing_piece(),
        Some(MissingPiece::UnregisteredAction.describe())
    );
}

fn influence_owner(
    attitude: Attitude,
    willingness: Willingness,
    tilt: WordTilt,
    hero: HeroProfile,
) -> DfRulesOwner {
    let setup = InfluenceSetup::new(
        InfluenceSkill::Persuasion,
        attitude,
        10,
        willingness,
        WordQuality {
            tilt,
            reason: "you named what he is proud of",
        },
    )
    .expect("a setup");
    let book = ActionBook::default()
        .with_entry(action("persuade"), RulesEntry::influence(setup))
        .expect("registers");
    owner_of(hero, book)
}

/// Charisma 14 (+2), proficient in Persuasion (+2): +4 against the default Influence DC of 15.
fn speaker() -> HeroProfile {
    HeroProfile::new([10, 10, 10, 10, 10, 14], 2, [Skill::Persuasion]).expect("valid")
}

#[test]
fn an_indifferent_urging_of_a_hesitant_creature_rolls_one_d20_against_dc_15() {
    let owner = influence_owner(
        Attitude::Indifferent,
        Willingness::Hesitant,
        WordTilt::Neutral,
        speaker(),
    );

    assert_eq!(prepared_draws(&owner, "persuade"), 1);
    let success = resolve(&owner, "persuade", &[d20(11)]);
    let failure = resolve(&owner, "persuade", &[d20(10)]);

    assert_eq!(fact(&success, "total"), Some(15));
    assert_eq!(fact(&success, "succeeded"), Some(1));
    assert_eq!(fact(&success, "retry_wait_hours"), None);
    assert_eq!(fact(&failure, "succeeded"), Some(0));
    assert_eq!(fact(&failure, "retry_wait_hours"), Some(24));
}

#[test]
fn a_friendly_creature_gives_advantage_so_two_dice_are_drawn_and_the_higher_counts() {
    let owner = influence_owner(
        Attitude::Friendly,
        Willingness::Hesitant,
        WordTilt::Neutral,
        speaker(),
    );

    assert_eq!(prepared_draws(&owner, "persuade"), 2);
    let resolution = resolve(&owner, "persuade", &[d20(3), d20(14)]);

    assert_eq!(fact(&resolution, "natural_die"), Some(14));
    assert_eq!(fact(&resolution, "advantage"), Some(1));
    assert_eq!(fact(&resolution, "dice_rolled"), Some(2));
    assert_eq!(fact(&resolution, "succeeded"), Some(1));
}

#[test]
fn a_hostile_creature_gives_disadvantage_so_the_lower_die_counts() {
    let owner = influence_owner(
        Attitude::Hostile,
        Willingness::Hesitant,
        WordTilt::Neutral,
        speaker(),
    );

    assert_eq!(prepared_draws(&owner, "persuade"), 2);
    let resolution = resolve(&owner, "persuade", &[d20(18), d20(5)]);

    assert_eq!(fact(&resolution, "natural_die"), Some(5));
    assert_eq!(fact(&resolution, "disadvantage"), Some(1));
    assert_eq!(fact(&resolution, "succeeded"), Some(0));
}

#[test]
fn advantage_and_disadvantage_together_cancel_to_one_die() {
    let owner = influence_owner(
        Attitude::Friendly,
        Willingness::Hesitant,
        WordTilt::Unfavourable,
        speaker(),
    );

    assert_eq!(prepared_draws(&owner, "persuade"), 1);
    let resolution = resolve(&owner, "persuade", &[d20(8)]);

    assert_eq!(fact(&resolution, "advantage"), Some(0));
    assert_eq!(fact(&resolution, "disadvantage"), Some(0));
}

#[test]
fn a_willing_or_unwilling_creature_needs_no_roll_and_the_answer_says_whether_it_complied() {
    for (willingness, complied) in [(Willingness::Willing, 1), (Willingness::Unwilling, 0)] {
        let owner = influence_owner(
            Attitude::Indifferent,
            willingness,
            WordTilt::Neutral,
            speaker(),
        );

        assert_eq!(prepared_draws(&owner, "persuade"), 0);
        let resolution = resolve(&owner, "persuade", &[]);

        assert_eq!(fact(&resolution, "no_roll"), Some(1));
        assert_eq!(fact(&resolution, "complied"), Some(complied));
        assert_eq!(fact(&resolution, "total"), None);
    }
}

#[test]
fn an_influence_setup_needs_a_reason_for_the_tilt_of_the_words() {
    let error = InfluenceSetup::new(
        InfluenceSkill::Persuasion,
        Attitude::Indifferent,
        10,
        Willingness::Hesitant,
        WordQuality {
            tilt: WordTilt::Favourable,
            reason: "  ",
        },
    )
    .expect_err("a blank reason");

    assert_eq!(error, super::error::AdapterError::InfluenceReasonIsBlank);
}

#[test]
fn the_calls_with_no_special_rules_use_the_defaults_a_check_has_no_use_for() {
    let owner = owner_of(thief(), check_book(Skill::SleightOfHand, 15));
    let name = action("check");

    let confirmation = owner
        .confirmation_needed(correlation(1), SEAT, &name)
        .expect("answers");
    let ruling = owner
        .ruling_needed(correlation(1), SEAT, &name)
        .expect("answers");
    let abandoned = owner.abandon(
        correlation(1),
        SEAT,
        &name,
        df_table::envelope::PendingId::new(1),
    );
    let planned = owner
        .prepare_in_plan(
            correlation(1),
            SEAT,
            &name,
            &PlanSoFar {
                steps: &[action("check")],
                effects: &[],
            },
        )
        .expect("answers");
    let draws = [d20(11)];
    let resumed = owner
        .resume(
            correlation(1),
            SEAT,
            &name,
            &Resumption {
                draws: &draws,
                answers: &[],
                ruling: None,
            },
        )
        .expect("answers");

    assert_eq!(confirmation, None::<ConfirmationReason>);
    assert_eq!(ruling, None);
    assert!(abandoned.is_ok());
    assert_eq!(
        planned,
        owner.prepare(correlation(1), SEAT, &name).expect("answers")
    );
    assert_eq!(resumed, resolve(&owner, "check", &draws));
}

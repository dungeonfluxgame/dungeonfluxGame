//! The roll card built from a resolved roll: what the page is given for each kind of roll.

use df_playground::fixture::{
    DieRole as ViewRole, InspirationReroll, NaturalMark, RollKind, RollMode as ViewMode,
    RollOutcome as ViewOutcome, RollView,
};
use df_rules::roll::{
    Ability, AttackKind, D20Request, D20Test, DiceExpression, DiceRequest, DieRole, DieSize,
    HeroicInspiration, ModifierLabel, PassivePerceptionRequest, Proficiency, RollMode, RollRequest,
    Skill, Target, TargetLabel,
};

use super::view::roll_view;

const PROFICIENT: Proficiency = Proficiency::Proficient { bonus: 2 };

fn d20(
    test: D20Test,
    mode: RollMode,
    ability_modifier: i8,
    proficiency: Proficiency,
    target: Option<Target>,
) -> RollRequest {
    RollRequest::D20(D20Request {
        test,
        mode,
        ability_modifier,
        proficiency,
        target,
        heroic_inspiration: None,
    })
}

fn dc(value: i16) -> Option<Target> {
    Some(Target {
        label: TargetLabel::DifficultyClass,
        value,
    })
}

/// The card for `request` resolved with `faces`, which the page's own check accepts.
fn card(request: &RollRequest, faces: &[u16], contested: bool) -> RollView {
    let result = request.resolve(faces).expect("the faces fit the roll");
    let view = roll_view("check-test-1".to_owned(), request, &result, contested)
        .expect("the roll converts");
    view.check().expect("the page accepts the card");
    view
}

fn athletics(mode: RollMode) -> RollRequest {
    d20(
        D20Test::SkillCheck(Skill::Athletics),
        mode,
        3,
        PROFICIENT,
        dc(15),
    )
}

#[test]
fn a_skill_check_card_has_its_title_dice_sources_target_and_verdict() {
    let view = card(&athletics(RollMode::Normal), &[14], false);
    assert_eq!(view.title, "Athletics (Strength)");
    assert_eq!(view.kind, RollKind::SkillCheck);
    assert_eq!(view.mode, ViewMode::Normal);
    assert_eq!(view.dice.len(), 1);
    assert_eq!((view.dice[0].sides, view.dice[0].face), (20, 14));
    assert_eq!(view.dice[0].role, ViewRole::Test);
    assert!(view.dice[0].kept);
    let sources: Vec<(&str, i32)> = view
        .modifiers
        .iter()
        .map(|modifier| (modifier.label.as_str(), modifier.value))
        .collect();
    assert_eq!(sources, [("Strength", 3), ("Proficiency", 2)]);
    assert_eq!(view.total, 19);
    let target = view.target.as_ref().expect("a check has a target");
    assert_eq!((target.label.as_str(), target.value), ("DC", 15));
    assert_eq!(view.outcome, Some(ViewOutcome::Success));
    assert_eq!(view.natural, NaturalMark::Unmarked);
    assert_eq!(view.inspiration_reroll, None);
}

#[test]
fn advantage_shows_both_dice_with_the_unused_one_not_kept() {
    let view = card(&athletics(RollMode::Advantage), &[5, 17], false);
    assert_eq!(view.mode, ViewMode::Advantage);
    let shown: Vec<(u8, bool)> = view.dice.iter().map(|die| (die.face, die.kept)).collect();
    assert_eq!(shown, [(5, false), (17, true)]);
    assert_eq!(view.total, 22);
    let view = card(&athletics(RollMode::Disadvantage), &[5, 17], false);
    let shown: Vec<(u8, bool)> = view.dice.iter().map(|die| (die.face, die.kept)).collect();
    assert_eq!(shown, [(5, true), (17, false)]);
    assert_eq!(view.total, 10);
}

#[test]
fn a_percentile_roll_is_one_die_of_one_hundred_sides() {
    let table = RollRequest::Dice(DiceRequest {
        role: DieRole::Table,
        expression: DiceExpression {
            count: 1,
            die: DieSize::D100,
            modifier: 0,
        },
        modifier_label: ModifierLabel::Flat,
        critical: false,
        heroic_inspiration: None,
    });
    let view = card(&table, &[6, 2], false);
    assert_eq!(view.kind, RollKind::TableRoll);
    assert_eq!(view.dice.len(), 1);
    assert_eq!((view.dice[0].sides, view.dice[0].face), (100, 62));
    assert_eq!(view.dice[0].role, ViewRole::Table);
    assert_eq!((view.total, view.outcome, view.target), (62, None, None));
    assert_eq!(card(&table, &[10, 10], false).dice[0].face, 100);
}

#[test]
fn the_death_save_verdicts_the_card_has_no_word_for_read_as_success_or_failure_with_the_natural() {
    let save = d20(
        D20Test::DeathSave,
        RollMode::Normal,
        0,
        Proficiency::None,
        None,
    );
    let revived = card(&save, &[20], false);
    assert_eq!(revived.kind, RollKind::DeathSavingThrow);
    assert_eq!(revived.outcome, Some(ViewOutcome::Success));
    assert_eq!(revived.natural, NaturalMark::Twenty);
    let target = revived.target.expect("a death save is set against ten");
    assert_eq!((target.label.as_str(), target.value), ("DC", 10));
    let fallen = card(&save, &[1], false);
    assert_eq!(fallen.outcome, Some(ViewOutcome::Failure));
    assert_eq!(fallen.natural, NaturalMark::One);
}

#[test]
fn a_contest_reads_won_or_lost_against_the_escape_difficulty() {
    let escape = |value| {
        d20(
            D20Test::SkillCheck(Skill::Athletics),
            RollMode::Normal,
            3,
            PROFICIENT,
            Some(Target {
                label: TargetLabel::EscapeDc,
                value,
            }),
        )
    };
    let won = card(&escape(15), &[14], true);
    assert_eq!(won.kind, RollKind::ContestedCheck);
    assert_eq!(won.title, "Athletics (Strength) contest");
    assert_eq!(won.outcome, Some(ViewOutcome::Won));
    assert_eq!(won.target.expect("a target").label, "Escape DC");
    let lost = card(&escape(15), &[2], true);
    assert_eq!(lost.outcome, Some(ViewOutcome::Lost));
}

#[test]
fn an_attack_card_is_set_against_armor_class_and_marks_the_champions_critical() {
    let attack = d20(
        D20Test::Attack {
            ability: Ability::Strength,
            kind: AttackKind::Melee,
            critical_from: 19,
        },
        RollMode::Normal,
        3,
        PROFICIENT,
        Some(Target {
            label: TargetLabel::ArmorClass,
            value: 16,
        }),
    );
    let critical = card(&attack, &[19], false);
    assert_eq!(critical.kind, RollKind::MeleeAttack);
    assert_eq!(critical.title, "Melee attack");
    assert_eq!(critical.outcome, Some(ViewOutcome::CriticalHit));
    assert_eq!(critical.natural, NaturalMark::Unmarked);
    assert_eq!(critical.target.expect("a target").label, "AC");
    let natural_one = card(&attack, &[1], false);
    assert_eq!(natural_one.outcome, Some(ViewOutcome::Miss));
    assert_eq!(natural_one.natural, NaturalMark::One);
}

#[test]
fn a_reroll_says_which_die_was_replaced_and_the_face_it_had() {
    let request = RollRequest::D20(D20Request {
        test: D20Test::SkillCheck(Skill::Athletics),
        mode: RollMode::Normal,
        ability_modifier: 3,
        proficiency: PROFICIENT,
        target: dc(15),
        heroic_inspiration: Some(HeroicInspiration { replaces_die: 0 }),
    });
    let view = card(&request, &[6, 14], false);
    assert_eq!(view.dice[0].face, 14);
    assert_eq!(
        view.inspiration_reroll,
        Some(InspirationReroll { index: 0, was: 6 })
    );
}

#[test]
fn rolls_with_no_target_have_no_verdict() {
    let initiative = card(
        &d20(D20Test::Initiative, RollMode::Normal, 1, PROFICIENT, None),
        &[10],
        false,
    );
    assert_eq!(
        (initiative.kind, initiative.outcome, initiative.total),
        (RollKind::Initiative, None, 13)
    );
    let damage = card(
        &RollRequest::Dice(DiceRequest::damage(
            DiceExpression {
                count: 1,
                die: DieSize::D8,
                modifier: 3,
            },
            Ability::Strength,
            false,
        )),
        &[6],
        false,
    );
    assert_eq!(
        (
            damage.kind,
            damage.outcome,
            damage.total,
            damage.dice[0].role
        ),
        (RollKind::Damage, None, 9, ViewRole::Damage)
    );
}

#[test]
fn a_passive_score_has_no_dice_and_its_base_among_the_sources() {
    let passive = RollRequest::PassivePerception(PassivePerceptionRequest {
        ability_modifier: 1,
        proficiency: PROFICIENT,
        mode: RollMode::Normal,
    });
    let view = card(&passive, &[], false);
    assert_eq!(view.kind, RollKind::PassivePerception);
    assert_eq!(view.title, "Passive Perception");
    assert!(view.dice.is_empty());
    let sources: Vec<(&str, i32)> = view
        .modifiers
        .iter()
        .map(|modifier| (modifier.label.as_str(), modifier.value))
        .collect();
    assert_eq!(sources, [("Base", 10), ("Wisdom", 1), ("Proficiency", 2)]);
    assert_eq!((view.total, view.outcome), (13, None));
}

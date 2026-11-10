use super::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::fmt::Debug;

fn check(test: D20Test, target: Option<Target>) -> D20Request {
    D20Request {
        test,
        mode: RollMode::Normal,
        ability_modifier: 3,
        proficiency: Proficiency::Proficient { bonus: 2 },
        target,
        heroic_inspiration: None,
    }
}

fn dc(value: i16) -> Option<Target> {
    Some(Target {
        label: TargetLabel::DifficultyClass,
        value,
    })
}

fn armor_class(value: i16) -> Option<Target> {
    Some(Target {
        label: TargetLabel::ArmorClass,
        value,
    })
}

fn attack(critical_from: u8) -> D20Test {
    D20Test::Attack {
        ability: Ability::Strength,
        kind: AttackKind::Melee,
        critical_from,
    }
}

fn damage(count: u8, die: DieSize, critical: bool) -> DiceRequest {
    DiceRequest::damage(
        DiceExpression {
            count,
            die,
            modifier: 0,
        },
        Ability::Strength,
        critical,
    )
}

fn assert_key_contract<T>(all: &[T], key: fn(T) -> &'static str, from_key: fn(&str) -> Option<T>)
where
    T: Copy + Debug + PartialEq + Serialize,
{
    let mut seen: Vec<&str> = Vec::new();
    for &variant in all {
        let text = key(variant);
        assert_eq!(
            serde_json::to_value(variant).expect("a closed key serializes"),
            Value::String(text.to_owned()),
            "{variant:?}"
        );
        assert_eq!(from_key(text), Some(variant), "{text}");
        assert!(!seen.contains(&text), "{text} is used twice");
        seen.push(text);
    }
    assert_eq!(from_key("not-a-key"), None);
}

#[test]
fn every_closed_key_of_a_roll_is_its_serde_name() {
    assert_key_contract(Ability::ALL, Ability::key, Ability::from_key);
    assert_key_contract(Skill::ALL, Skill::key, Skill::from_key);
    assert_key_contract(DieSize::ALL, DieSize::key, DieSize::from_key);
    assert_key_contract(DieRole::ALL, DieRole::key, DieRole::from_key);
    assert_key_contract(DiePlace::ALL, DiePlace::key, DiePlace::from_key);
    assert_key_contract(
        ModifierLabel::ALL,
        ModifierLabel::key,
        ModifierLabel::from_key,
    );
    assert_key_contract(TargetLabel::ALL, TargetLabel::key, TargetLabel::from_key);
    assert_key_contract(Natural::ALL, Natural::key, Natural::from_key);
    assert_key_contract(RollOutcome::ALL, RollOutcome::key, RollOutcome::from_key);
    assert_key_contract(AttackKind::ALL, AttackKind::key, AttackKind::from_key);
}

#[test]
fn each_skill_uses_the_ability_the_skills_table_gives_it() {
    let table = [
        (Skill::Acrobatics, Ability::Dexterity),
        (Skill::AnimalHandling, Ability::Wisdom),
        (Skill::Arcana, Ability::Intelligence),
        (Skill::Athletics, Ability::Strength),
        (Skill::Deception, Ability::Charisma),
        (Skill::History, Ability::Intelligence),
        (Skill::Insight, Ability::Wisdom),
        (Skill::Intimidation, Ability::Charisma),
        (Skill::Investigation, Ability::Intelligence),
        (Skill::Medicine, Ability::Wisdom),
        (Skill::Nature, Ability::Intelligence),
        (Skill::Perception, Ability::Wisdom),
        (Skill::Performance, Ability::Charisma),
        (Skill::Persuasion, Ability::Charisma),
        (Skill::Religion, Ability::Intelligence),
        (Skill::SleightOfHand, Ability::Dexterity),
        (Skill::Stealth, Ability::Dexterity),
        (Skill::Survival, Ability::Wisdom),
    ];

    assert_eq!(table.len(), Skill::ALL.len());
    for (skill, ability) in table {
        assert_eq!(skill.ability(), ability, "{}", skill.key());
        let request = RollRequest::D20(check(D20Test::SkillCheck(skill), dc(10)));
        let result = request.resolve(&[10]).expect("a skill check resolves");
        assert_eq!(
            result.modifiers.first().map(|modifier| modifier.label),
            Some(ModifierLabel::ability(ability)),
            "{}",
            skill.key()
        );
    }
}

#[test]
fn every_ability_has_its_own_place_in_a_score_array() {
    let places: Vec<usize> = Ability::ALL.iter().map(|ability| ability.index()).collect();

    assert_eq!(places, [0, 1, 2, 3, 4, 5]);
}

#[test]
fn a_dice_expression_reads_count_die_and_signed_modifier() {
    let read = |text: &str| text.parse::<DiceExpression>();
    let expression = |count, die, modifier| DiceExpression {
        count,
        die,
        modifier,
    };

    assert_eq!(read("2d6+3"), Ok(expression(2, DieSize::D6, 3)));
    assert_eq!(read("3d8-1"), Ok(expression(3, DieSize::D8, -1)));
    assert_eq!(read("d20"), Ok(expression(1, DieSize::D20, 0)));
    assert_eq!(read("1d100"), Ok(expression(1, DieSize::D100, 0)));
    for size in DieSize::ALL {
        let text = format!("1d{}", size.sides());
        assert_eq!(read(&text), Ok(expression(1, *size, 0)), "{text}");
    }
}

#[test]
fn a_dice_expression_prints_the_text_it_was_read_from() {
    for text in ["2d6+3", "3d8-1", "1d20", "4d4"] {
        let expression: DiceExpression = text.parse().expect("an expression");

        assert_eq!(expression.to_string(), text);
    }
}

#[test]
fn a_text_that_is_not_a_dice_expression_is_refused_with_its_reason() {
    let read = |text: &str| text.parse::<DiceExpression>();

    for text in [
        "", "d", "2d", "2x6", "2d6+", "2d6++3", "2d6+-3", "300d6", "-2d6", "2d6 +3",
    ] {
        assert_eq!(
            read(text),
            Err(DiceExpressionError::InvalidSyntax),
            "{text:?}"
        );
    }
    for text in ["1d3", "1d7", "1d0", "2d1000"] {
        assert_eq!(
            read(text),
            Err(DiceExpressionError::UnsupportedDie),
            "{text:?}"
        );
    }
}

#[test]
fn a_percentile_die_is_drawn_as_two_d10_tens_then_ones() {
    let request = RollRequest::Dice(DiceRequest {
        role: DieRole::Table,
        expression: DiceExpression {
            count: 2,
            die: DieSize::D100,
            modifier: 0,
        },
        modifier_label: ModifierLabel::Flat,
        critical: false,
        heroic_inspiration: None,
    });

    let plan = request.draw_plan().expect("a plan");
    let result = request
        .resolve(&[10, 3, 4, 10])
        .expect("two percentile rolls");

    assert_eq!(plan, [DieSize::D10; 4]);
    let places: Vec<DiePlace> = result.dice.iter().map(|die| die.place).collect();
    assert_eq!(
        places,
        [
            DiePlace::Tens,
            DiePlace::Ones,
            DiePlace::Tens,
            DiePlace::Ones
        ]
    );
    assert_eq!(result.total, 3 + 40);
}

#[test]
fn the_draw_plan_asks_for_one_d20_or_two_and_one_more_for_inspiration() {
    let plan = |mode, reroll: Option<u8>| {
        RollRequest::D20(D20Request {
            mode,
            heroic_inspiration: reroll.map(|replaces_die| HeroicInspiration { replaces_die }),
            ..check(D20Test::Initiative, None)
        })
        .draw_plan()
    };

    assert_eq!(plan(RollMode::Normal, None), Ok(vec![DieSize::D20]));
    assert_eq!(plan(RollMode::Advantage, None), Ok(vec![DieSize::D20; 2]));
    assert_eq!(
        plan(RollMode::Disadvantage, Some(1)),
        Ok(vec![DieSize::D20; 3])
    );
    assert_eq!(plan(RollMode::Normal, Some(0)), Ok(vec![DieSize::D20; 2]));
}

#[test]
fn the_draw_plan_of_a_critical_hit_doubles_the_damage_dice_only() {
    let plan = |critical| RollRequest::Dice(damage(2, DieSize::D6, critical)).draw_plan();

    assert_eq!(plan(false), Ok(vec![DieSize::D6; 2]));
    assert_eq!(plan(true), Ok(vec![DieSize::D6; 4]));
}

#[test]
fn a_passive_score_draws_no_dice() {
    let request = RollRequest::PassivePerception(PassivePerceptionRequest {
        ability_modifier: 1,
        proficiency: Proficiency::None,
        mode: RollMode::Normal,
    });

    assert_eq!(request.draw_plan(), Ok(Vec::new()));
}

#[test]
fn the_same_faces_always_give_the_same_result() {
    let request = RollRequest::D20(D20Request {
        mode: RollMode::Advantage,
        ..check(D20Test::SkillCheck(Skill::Athletics), dc(15))
    });

    let first = request.resolve(&[7, 13]).expect("resolves");
    let again = request.resolve(&[7, 13]).expect("resolves");

    assert_eq!(first, again);
}

#[test]
fn both_dice_of_an_advantage_roll_stay_in_the_result_with_one_kept() {
    let request = RollRequest::D20(D20Request {
        mode: RollMode::Advantage,
        ..check(D20Test::SkillCheck(Skill::Athletics), dc(15))
    });

    let result = request.resolve(&[7, 13]).expect("resolves");

    let dice: Vec<(u16, bool)> = result.dice.iter().map(|die| (die.face, die.kept)).collect();
    assert_eq!(dice, [(7, false), (13, true)]);
    assert!(
        result
            .dice
            .iter()
            .all(|die| die.sides == 20 && die.role == DieRole::Test)
    );
}

#[test]
fn missing_and_extra_faces_are_typed_errors_that_name_the_die() {
    let advantage = RollRequest::D20(D20Request {
        mode: RollMode::Advantage,
        ..check(D20Test::Initiative, None)
    });
    let percentile = RollRequest::Dice(DiceRequest {
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

    assert_eq!(
        advantage.resolve(&[5]),
        Err(RollError::MissingDraw { sides: 20 })
    );
    assert_eq!(
        advantage.resolve(&[]),
        Err(RollError::MissingDraw { sides: 20 })
    );
    assert_eq!(advantage.resolve(&[5, 6, 7]), Err(RollError::UnusedDraws));
    assert_eq!(
        percentile.resolve(&[4]),
        Err(RollError::MissingDraw { sides: 10 })
    );
}

#[test]
fn a_face_off_its_die_is_refused() {
    let initiative = RollRequest::D20(check(D20Test::Initiative, None));
    let tens = RollRequest::Dice(damage(1, DieSize::D100, false));

    assert_eq!(
        initiative.resolve(&[21]),
        Err(RollError::FaceOutOfRange {
            sides: 20,
            face: 21
        })
    );
    assert_eq!(
        initiative.resolve(&[0]),
        Err(RollError::FaceOutOfRange { sides: 20, face: 0 })
    );
    assert_eq!(
        tens.resolve(&[11, 1]),
        Err(RollError::FaceOutOfRange {
            sides: 10,
            face: 11
        })
    );
}

#[test]
fn a_target_must_fit_the_kind_of_test() {
    let ability_check = D20Test::AbilityCheck(Ability::Strength);
    let plan = |test, target| RollRequest::D20(check(test, target)).draw_plan();

    assert_eq!(plan(ability_check, None), Err(RollError::MissingTarget));
    assert_eq!(plan(attack(20), None), Err(RollError::MissingTarget));
    assert_eq!(
        plan(ability_check, armor_class(15)),
        Err(RollError::MismatchedTarget)
    );
    assert_eq!(plan(attack(20), dc(15)), Err(RollError::MismatchedTarget));
    assert_eq!(
        plan(D20Test::Initiative, dc(15)),
        Err(RollError::UnexpectedTarget)
    );
    assert_eq!(
        RollRequest::D20(D20Request {
            ability_modifier: 0,
            proficiency: Proficiency::None,
            ..check(D20Test::DeathSave, dc(10))
        })
        .draw_plan(),
        Err(RollError::UnexpectedTarget)
    );
    assert_eq!(plan(ability_check, dc(0)), Err(RollError::InvalidTarget));
    assert_eq!(
        plan(attack(20), armor_class(0)),
        Err(RollError::InvalidTarget)
    );
}

#[test]
fn a_hidden_creatures_stealth_total_may_be_any_number() {
    let request = RollRequest::D20(check(
        D20Test::SkillCheck(Skill::Perception),
        Some(Target {
            label: TargetLabel::StealthTotal,
            value: -2,
        }),
    ));

    assert_eq!(
        request.resolve(&[1]).map(|result| result.outcome),
        Ok(RollOutcome::Success)
    );
}

#[test]
fn numbers_outside_the_source_ranges_are_refused() {
    let plan = |request: D20Request| RollRequest::D20(request).draw_plan();
    let base = check(D20Test::AbilityCheck(Ability::Strength), dc(10));

    for modifier in [-6, 11] {
        assert_eq!(
            plan(D20Request {
                ability_modifier: modifier,
                ..base
            }),
            Err(RollError::InvalidAbilityModifier),
            "{modifier}"
        );
    }
    for bonus in [0, 1, 10] {
        for proficiency in [
            Proficiency::Proficient { bonus },
            Proficiency::Expertise { bonus },
        ] {
            assert_eq!(
                plan(D20Request {
                    proficiency,
                    ..base
                }),
                Err(RollError::InvalidProficiencyBonus),
                "{proficiency:?}"
            );
        }
    }
    for critical_from in [0, 1, 21] {
        assert_eq!(
            plan(check(attack(critical_from), armor_class(10))),
            Err(RollError::InvalidCriticalRange),
            "{critical_from}"
        );
    }
}

#[test]
fn a_death_saving_throw_takes_no_modifier_or_proficiency() {
    let with = |ability_modifier, proficiency| {
        RollRequest::D20(D20Request {
            ability_modifier,
            proficiency,
            ..check(D20Test::DeathSave, None)
        })
        .draw_plan()
    };

    assert_eq!(
        with(1, Proficiency::None),
        Err(RollError::UnexpectedModifiers)
    );
    assert_eq!(
        with(0, Proficiency::Proficient { bonus: 2 }),
        Err(RollError::UnexpectedModifiers)
    );
    assert_eq!(with(0, Proficiency::None), Ok(vec![DieSize::D20]));
}

#[test]
fn heroic_inspiration_must_name_a_die_that_is_in_the_roll() {
    let initiative = |replaces_die| {
        RollRequest::D20(D20Request {
            heroic_inspiration: Some(HeroicInspiration { replaces_die }),
            ..check(D20Test::Initiative, None)
        })
        .draw_plan()
    };
    let damage_roll = |replaces_die| {
        RollRequest::Dice(DiceRequest {
            heroic_inspiration: Some(HeroicInspiration { replaces_die }),
            ..damage(2, DieSize::D6, false)
        })
        .draw_plan()
    };

    assert_eq!(initiative(1), Err(RollError::InvalidReroll));
    assert_eq!(damage_roll(2), Err(RollError::InvalidReroll));
    assert_eq!(damage_roll(1), Ok(vec![DieSize::D6; 3]));
}

#[test]
fn a_dice_request_refuses_the_test_role_a_critical_on_healing_and_unreasonable_sizes() {
    let plan = |request: DiceRequest| RollRequest::Dice(request).draw_plan();
    let base = damage(1, DieSize::D8, false);

    assert_eq!(
        plan(DiceRequest {
            role: DieRole::Test,
            ..base
        }),
        Err(RollError::InvalidRole)
    );
    assert_eq!(
        plan(DiceRequest {
            role: DieRole::Healing,
            critical: true,
            ..base
        }),
        Err(RollError::CriticalNeedsDamage)
    );
    assert_eq!(
        plan(damage(0, DieSize::D8, false)),
        Err(RollError::InvalidDiceCount)
    );
    assert_eq!(
        plan(damage(101, DieSize::D8, false)),
        Err(RollError::InvalidDiceCount)
    );
    assert_eq!(
        plan(damage(100, DieSize::D100, true)).map(|plan| plan.len()),
        Ok(400)
    );
    let heavy = DiceRequest {
        expression: DiceExpression {
            count: 1,
            die: DieSize::D8,
            modifier: 1001,
        },
        ..base
    };
    assert_eq!(plan(heavy), Err(RollError::InvalidDiceModifier));
}

#[test]
fn the_unarmed_strike_dc_is_eight_plus_strength_and_proficiency() {
    assert_eq!(unarmed_strike_dc(3, 2), 13);
    assert_eq!(unarmed_strike_dc(-1, 2), 9);
    assert_eq!(unarmed_strike_dc(5, 6), 19);
}

#[test]
fn a_request_and_its_result_travel_as_json_with_closed_keys() {
    let request = RollRequest::D20(D20Request {
        mode: RollMode::Advantage,
        ..check(D20Test::SkillCheck(Skill::Athletics), dc(15))
    });

    let wire = serde_json::to_value(request).expect("a request serializes");

    assert_eq!(
        wire,
        json!({
            "roll": "d20",
            "test": {"skill_check": "athletics"},
            "mode": "advantage",
            "ability_modifier": 3,
            "proficiency": {"proficient": {"bonus": 2}},
            "target": {"label": "difficulty_class", "value": 15},
            "heroic_inspiration": null,
        })
    );
    let back: RollRequest = serde_json::from_value(wire).expect("it reads back");
    assert_eq!(back, request);
    let result = request.resolve(&[7, 13]).expect("resolves");
    let result_wire = serde_json::to_value(&result).expect("a result serializes");
    let result_back: RollResult = serde_json::from_value(result_wire).expect("reads back");
    assert_eq!(result_back, result);
}

#[test]
fn a_request_with_an_unknown_field_is_refused_when_read() {
    let wire = json!({
        "roll": "d20",
        "test": "initiative",
        "mode": "normal",
        "ability_modifier": 2,
        "proficiency": "none",
        "target": null,
        "heroic_inspiration": null,
        "rolled": 20,
    });

    assert!(serde_json::from_value::<RollRequest>(wire).is_err());
}

fn assert_round_trip<T: Serialize + DeserializeOwned + PartialEq + Debug>(value: T) {
    let wire = serde_json::to_value(&value).expect("serializes");
    assert_eq!(
        serde_json::from_value::<T>(wire).expect("reads back"),
        value
    );
}

#[test]
fn every_kind_of_request_round_trips_through_json() {
    assert_round_trip(RollRequest::PassivePerception(PassivePerceptionRequest {
        ability_modifier: 1,
        proficiency: Proficiency::Expertise { bonus: 3 },
        mode: RollMode::Disadvantage,
    }));
    assert_round_trip(RollRequest::Dice(damage(2, DieSize::D100, true)));
    assert_round_trip(RollRequest::D20(check(attack(19), armor_class(14))));
}

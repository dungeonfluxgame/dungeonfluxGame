use super::*;
#[test]
fn srd_p47_p84_p87_maximum_hit_points_by_level_constitution_and_species_bonus() {
    // Level 1: d10 maximum 10 + CON 14 (+2) + Dwarven Toughness 1.
    assert_eq!(maximum_hit_points(1, 10, 14, 1), 13);
    // Level 5 adds the fixed 6 per further level and the per-level terms five times.
    assert_eq!(maximum_hit_points(5, 10, 14, 1), 10 + 24 + 3 * 5);
    // Human Tough adds 2 per level; a CON 20 hero is bounded by CON 20, not by the Dwarf row.
    assert_eq!(maximum_hit_points(1, 10, 20, 2), 17);
    assert_eq!(maximum_hit_points(1, 10, 20, 0), 15);
    // The progression and the creation rules agree with the one function.
    let progression = FighterProgression::new(5).unwrap();
    assert_eq!(
        u32::from(progression.hit_points(14, HeroSpecies::Dwarf)),
        maximum_hit_points(5, FIGHTER_FIRST_LEVEL_HIT_POINTS, 14, 1)
    );
}
#[test]
fn srd_p17_p47_p187_short_rest_hit_point_die_adds_modifier_minimum_and_cap() {
    assert_eq!(heal_hit_point_die(5, 2, 3, 13), Ok((10, 7)));
    assert_eq!(heal_hit_point_die(1, -5, 7, 13), Ok((8, 1)));
    assert_eq!(heal_hit_point_die(10, 5, 12, 13), Ok((13, 1)));
}
#[test]
fn srd_p47_p187_short_rest_hit_point_die_rejects_zero_hp_full_hp_and_invalid_faces() {
    assert_eq!(
        heal_hit_point_die(5, 2, 0, 13),
        Err(JourneyRuleError::InvalidTarget)
    );
    assert_eq!(
        heal_hit_point_die(5, 2, 13, 13),
        Err(JourneyRuleError::InvalidTarget)
    );
    assert_eq!(
        heal_hit_point_die(0, 2, 3, 13),
        Err(JourneyRuleError::InvalidDie)
    );
    assert_eq!(
        heal_hit_point_die(11, 2, 3, 13),
        Err(JourneyRuleError::InvalidDie)
    );
}
struct ExpectedSrdCreatureCore {
    id: &'static str,
    weapon_label: &'static str,
    page: u16,
    size: crate::tactical::Size,
    speed_ft: u16,
    weapons: &'static [crate::tactical::Weapon],
    shield: bool,
    hit_points: u8,
    armor_class: u8,
    abilities: [u8; 6],
    attack_bonus: i32,
    damage_modifier: i32,
    damage_dice: (u8, u8),
}

fn assert_srd_creature_core(expected: ExpectedSrdCreatureCore) {
    let creature = crate::SRD_521_CREATURES
        .iter()
        .find(|creature| creature.id == expected.id)
        .expect("compiled SRD creature");
    let tactical =
        crate::tactical::CombatantSpec::creature(3, expected.id, crate::tactical::Cell(2, 1), 0)
            .expect("compiled tactical creature");
    assert_eq!(creature.weapon_label, expected.weapon_label);
    assert_eq!(creature.page, expected.page);
    assert_eq!(tactical.size, expected.size);
    assert_eq!(tactical.speed_ft, expected.speed_ft);
    assert_eq!(tactical.weapons.as_slice(), expected.weapons);
    assert_eq!(tactical.shield, expected.shield);
    assert_eq!(creature.rules.hit_points, expected.hit_points);
    assert_eq!(creature.rules.armor_class, expected.armor_class);
    assert_eq!(creature.rules.abilities, expected.abilities);
    assert_eq!(creature.rules.attack_bonus, expected.attack_bonus);
    assert_eq!(creature.rules.damage_modifier, expected.damage_modifier);
    assert_eq!(
        (
            creature.rules.weapon.damage_dice,
            creature.rules.weapon.damage_sides,
        ),
        expected.damage_dice
    );
}

// SRD 5.2.1, p. 261. Damage modifier is the Scimitar's +1.
#[test]
fn bandit_core_stat_block_values_srd_521_p261() {
    assert_srd_creature_core(ExpectedSrdCreatureCore {
        id: "bandit",
        weapon_label: "scimitar",
        page: 261,
        size: crate::tactical::Size::Medium,
        speed_ft: 30,
        weapons: &[
            crate::tactical::Weapon::Scimitar,
            crate::tactical::Weapon::LightCrossbow,
        ],
        shield: false,
        hit_points: 11,
        armor_class: 12,
        abilities: [11, 12, 12, 10, 10, 10],
        attack_bonus: 3,
        damage_modifier: 1,
        damage_dice: (1, 6),
    });
}

// SRD 5.2.1, p. 278. The code folds the Sickle's +1 Slashing and +1 Necrotic into +2.
#[test]
fn cultist_core_stat_block_values_srd_521_p278() {
    assert_srd_creature_core(ExpectedSrdCreatureCore {
        id: "cultist",
        weapon_label: "ritual sickle",
        page: 278,
        size: crate::tactical::Size::Medium,
        speed_ft: 30,
        weapons: &[crate::tactical::Weapon::Sickle],
        shield: false,
        hit_points: 9,
        armor_class: 12,
        abilities: [11, 12, 10, 10, 11, 10],
        attack_bonus: 3,
        damage_modifier: 2,
        damage_dice: (1, 4),
    });
}

// SRD 5.2.1, p. 296. The compiled melee action is the Spear.
#[test]
fn guard_core_stat_block_values_srd_521_p296() {
    assert_srd_creature_core(ExpectedSrdCreatureCore {
        id: "guard",
        weapon_label: "spear",
        page: 296,
        size: crate::tactical::Size::Medium,
        speed_ft: 30,
        weapons: &[crate::tactical::Weapon::Spear],
        shield: true,
        hit_points: 11,
        armor_class: 16,
        abilities: [13, 12, 12, 10, 11, 10],
        attack_bonus: 3,
        damage_modifier: 1,
        damage_dice: (1, 6),
    });
}

// SRD 5.2.1, p. 290. The compiled melee action is the Scimitar.
#[test]
fn goblin_warrior_core_stat_block_values_srd_521_p290() {
    assert_srd_creature_core(ExpectedSrdCreatureCore {
        id: "goblin-warrior",
        weapon_label: "scimitar",
        page: 290,
        size: crate::tactical::Size::Small,
        speed_ft: 30,
        weapons: &[
            crate::tactical::Weapon::Scimitar,
            crate::tactical::Weapon::Shortbow,
        ],
        shield: true,
        hit_points: 10,
        armor_class: 15,
        abilities: [8, 15, 10, 10, 8, 8],
        attack_bonus: 4,
        damage_modifier: 2,
        damage_dice: (1, 6),
    });
}

// SRD 5.2.1, p. 302. The compiled melee action is the Dagger.
#[test]
fn kobold_warrior_core_stat_block_values_srd_521_p302() {
    assert_srd_creature_core(ExpectedSrdCreatureCore {
        id: "kobold-warrior",
        weapon_label: "dagger",
        page: 302,
        size: crate::tactical::Size::Small,
        speed_ft: 30,
        weapons: &[crate::tactical::Weapon::Dagger],
        shield: false,
        hit_points: 7,
        armor_class: 14,
        abilities: [7, 15, 9, 8, 7, 8],
        attack_bonus: 4,
        damage_modifier: 2,
        damage_dice: (1, 4),
    });
}

// SRD 5.2.1, p. 325. The compiled melee action is the Shortsword.
#[test]
fn skeleton_core_stat_block_values_srd_521_p325() {
    assert_srd_creature_core(ExpectedSrdCreatureCore {
        id: "skeleton",
        weapon_label: "shortsword",
        page: 325,
        size: crate::tactical::Size::Medium,
        speed_ft: 30,
        weapons: &[
            crate::tactical::Weapon::Shortsword,
            crate::tactical::Weapon::Shortbow,
        ],
        shield: false,
        hit_points: 13,
        armor_class: 14,
        abilities: [10, 16, 15, 6, 8, 5],
        attack_bonus: 5,
        damage_modifier: 3,
        damage_dice: (1, 6),
    });
}

// SRD 5.2.1, p. 343. The compiled action is Slam.
#[test]
fn zombie_core_stat_block_values_srd_521_p343() {
    assert_srd_creature_core(ExpectedSrdCreatureCore {
        id: "zombie",
        weapon_label: "slam",
        page: 343,
        size: crate::tactical::Size::Medium,
        speed_ft: 20,
        weapons: &[crate::tactical::Weapon::Slam],
        shield: false,
        hit_points: 15,
        armor_class: 8,
        abilities: [13, 6, 16, 3, 6, 5],
        attack_bonus: 3,
        damage_modifier: 1,
        damage_dice: (1, 8),
    });
}

// SRD 5.2.1, p. 364. The compiled action is Bite.
#[test]
fn wolf_core_stat_block_values_srd_521_p364() {
    assert_srd_creature_core(ExpectedSrdCreatureCore {
        id: "wolf",
        weapon_label: "bite",
        page: 364,
        size: crate::tactical::Size::Medium,
        speed_ft: 40,
        weapons: &[crate::tactical::Weapon::Bite],
        shield: false,
        hit_points: 11,
        armor_class: 12,
        abilities: [14, 15, 12, 3, 12, 6],
        attack_bonus: 4,
        damage_modifier: 2,
        damage_dice: (1, 6),
    });
}

// SRD 5.2.1, p. 353. The compiled action is Bite.
#[test]
fn giant_rat_core_stat_block_values_srd_521_p353() {
    assert_srd_creature_core(ExpectedSrdCreatureCore {
        id: "giant-rat",
        weapon_label: "bite",
        page: 353,
        size: crate::tactical::Size::Small,
        speed_ft: 30,
        weapons: &[crate::tactical::Weapon::Bite],
        shield: false,
        hit_points: 7,
        armor_class: 13,
        abilities: [7, 16, 11, 2, 10, 4],
        attack_bonus: 5,
        damage_modifier: 3,
        damage_dice: (1, 4),
    });
}

// SRD 5.2.1, p. 355. The compiled action is Bite.
#[test]
fn giant_weasel_core_stat_block_values_srd_521_p355() {
    assert_srd_creature_core(ExpectedSrdCreatureCore {
        id: "giant-weasel",
        weapon_label: "bite",
        page: 355,
        size: crate::tactical::Size::Medium,
        speed_ft: 40,
        weapons: &[crate::tactical::Weapon::Bite],
        shield: false,
        hit_points: 9,
        armor_class: 13,
        abilities: [11, 17, 10, 4, 12, 5],
        attack_bonus: 5,
        damage_modifier: 3,
        damage_dice: (1, 4),
    });
}

// SRD 5.2.1, p. 357. The compiled action is Bite.
#[test]
fn mastiff_core_stat_block_values_srd_521_p357() {
    assert_srd_creature_core(ExpectedSrdCreatureCore {
        id: "mastiff",
        weapon_label: "bite",
        page: 357,
        size: crate::tactical::Size::Medium,
        speed_ft: 40,
        weapons: &[crate::tactical::Weapon::Bite],
        shield: false,
        hit_points: 5,
        armor_class: 12,
        abilities: [13, 14, 12, 3, 12, 7],
        attack_bonus: 3,
        damage_modifier: 1,
        damage_dice: (1, 6),
    });
}

// SRD 5.2.1, p. 322. The compiled legacy attack is Shortsword.
#[test]
fn scout_core_stat_block_values_srd_521_p322() {
    assert_srd_creature_core(ExpectedSrdCreatureCore {
        id: "scout",
        weapon_label: "shortsword",
        page: 322,
        size: crate::tactical::Size::Medium,
        speed_ft: 30,
        weapons: &[
            crate::tactical::Weapon::Shortsword,
            crate::tactical::Weapon::Longbow,
        ],
        shield: false,
        hit_points: 16,
        armor_class: 13,
        abilities: [11, 14, 12, 11, 13, 11],
        attack_bonus: 4,
        damage_modifier: 2,
        damage_dice: (1, 6),
    });
}

fn choices() -> Vec<(&'static str, &'static str)> {
    vec![
        ("species", "dwarf"),
        ("class", "fighter-1"),
        ("background", "soldier"),
        ("array", "stalwart"),
        ("alignment", "neutral-good"),
        ("language-1", "dwarvish"),
        ("language-2", "elvish"),
        ("skill-1", "perception"),
        ("skill-2", "survival"),
        ("gaming-set", "dice"),
        ("equipment", "fighter-a-soldier-a"),
        ("style-masteries", "defense-greatsword-flail-javelin"),
        ("allied-ties", "join-order"),
    ]
}

#[test]
fn initiative_uses_supplied_faces_and_modifier_and_retains_authored_tie_order() {
    assert_eq!(
        initiative_order([(10, 13), (9, 14), (10, 12)]),
        Ok([0, 1, 2])
    );
    assert_eq!(
        initiative_order([(8, 13), (14, 14), (4, 12)]),
        Ok([1, 0, 2])
    );
    assert_eq!(
        initiative_order([(1, 1), (20, 20), (10, 10)]),
        Ok([1, 2, 0])
    );
}
#[test]
fn initiative_rejects_invalid_local_faces_and_ability_scores() {
    for position in 0..3 {
        for invalid in [0, 21] {
            let mut supplied = [(10, 12); 3];
            supplied[position].0 = invalid;
            assert_eq!(
                initiative_order(supplied),
                Err(JourneyRuleError::InvalidDie)
            );
            supplied[position] = (10, invalid as u8);
            assert_eq!(
                initiative_order(supplied),
                Err(JourneyRuleError::InvalidChoice)
            );
        }
    }
}
#[test]
fn legal_build_and_illegal_duplicate_skill() {
    let mut values = choices();
    let built = validate_character(&CreationRules::srd_521(), "Brynn", &values)
        .expect("source-legal selection");
    assert_eq!(built.abilities, [17, 13, 15, 10, 12, 8]);
    assert_eq!(
        (built.hit_points, built.armor_class, built.proficiency_bonus),
        (13, 17, 2)
    );
    values[8].1 = "perception";
    assert_eq!(
        validate_character(&CreationRules::srd_521(), "Brynn", &values),
        Err(JourneyRuleError::InvalidChoice)
    );
    values[8].1 = "athletics"; // Soldier already provides it.
    assert_eq!(
        validate_character(&CreationRules::srd_521(), "Brynn", &values),
        Err(JourneyRuleError::InvalidChoice)
    );
}
#[test]
fn critical_doubles_weapon_dice_and_knockout_precedes_zero() {
    let result = resolve_melee(NormalMeleeAttack {
        critical_threshold: CombatRules::srd_521().critical_threshold,
        damage_sides: SRD_521_BASELINE.hero_weapon.damage_sides,
        die: 20,
        modifier: 5,
        armor_class: 12,
        damage_dice: &[6, 5, 4, 3, 1, 1, 1, 1],
        weapon_dice: 2,
        damage_modifier: 3,
        savage_attacker_take_higher: true,
        graze: true,
        target_hit_points: 11,
    })
    .expect("critical explicit dice");
    assert!(result.hit && result.critical && result.knocked_out);
    assert_eq!((result.damage, result.target_hit_points), (21, 1));
    assert_eq!(result.damage_dice.len(), 8);
}
#[test]
fn natural_one_graze_and_bad_dice_are_explicit() {
    let attack = NormalMeleeAttack {
        critical_threshold: CombatRules::srd_521().critical_threshold,
        damage_sides: SRD_521_BASELINE.hero_weapon.damage_sides,
        die: 1,
        modifier: 50,
        armor_class: 12,
        damage_dice: &[],
        weapon_dice: 2,
        damage_modifier: 3,
        savage_attacker_take_higher: true,
        graze: true,
        target_hit_points: 11,
    };
    let result = resolve_melee(attack).expect("miss with Graze");
    assert!(!result.hit && result.grazed);
    assert_eq!((result.damage, result.target_hit_points), (3, 8));
    assert_eq!(
        resolve_melee(NormalMeleeAttack {
            damage_dice: &[6],
            ..attack
        }),
        Err(JourneyRuleError::InvalidDamageDice)
    );
}
#[test]
fn a_modifier_that_overflows_the_attack_total_is_refused_instead_of_panicking() {
    let attack = NormalMeleeAttack {
        critical_threshold: CombatRules::srd_521().critical_threshold,
        damage_sides: SRD_521_BASELINE.hero_weapon.damage_sides,
        die: 20,
        modifier: i32::MAX,
        armor_class: 12,
        damage_dice: &[6, 5, 4, 3],
        weapon_dice: 2,
        damage_modifier: 3,
        savage_attacker_take_higher: false,
        graze: false,
        target_hit_points: 11,
    };
    assert_eq!(resolve_melee(attack), Err(JourneyRuleError::InvalidRules));
    let overflowing_damage = NormalMeleeAttack {
        modifier: 5,
        damage_modifier: i32::MAX,
        ..attack
    };
    let result = resolve_melee(overflowing_damage).expect("a huge damage modifier still resolves");
    assert_eq!(result.damage, i32::MAX as u32 + 18);
    let negative_damage = NormalMeleeAttack {
        damage_modifier: i32::MIN,
        ..overflowing_damage
    };
    let result = resolve_melee(negative_damage).expect("a huge penalty still resolves");
    assert_eq!(result.damage, 0, "a negative damage total clamps to zero");
}
#[test]
fn an_armor_class_beyond_i32_is_never_hit_by_a_normal_roll() {
    let result = resolve_melee(NormalMeleeAttack {
        critical_threshold: CombatRules::srd_521().critical_threshold,
        damage_sides: SRD_521_BASELINE.hero_weapon.damage_sides,
        die: 19,
        modifier: 5,
        armor_class: u32::MAX,
        damage_dice: &[],
        weapon_dice: 2,
        damage_modifier: 3,
        savage_attacker_take_higher: false,
        graze: false,
        target_hit_points: 11,
    })
    .expect("a miss resolves");
    assert!(!result.hit);
}
#[test]
fn action_and_bonus_economy_and_healing_do_not_reset_action() {
    let start = ActionEconomy {
        action_used: false,
        bonus_action_used: false,
    };
    let attack = start.attack().expect("one action");
    assert_eq!(attack.attack(), Err(JourneyRuleError::ActionSpent));
    let healed = attack.second_wind(2).expect("one bonus action");
    assert!(healed.action_used && healed.bonus_action_used);
    assert_eq!(
        healed.second_wind(1),
        Err(JourneyRuleError::BonusActionSpent)
    );
    assert_eq!(
        heal_second_wind(&SecondWindRules::srd_521(), 10, 5, 13, 2),
        Ok((13, 1))
    );
    assert_eq!(
        heal_second_wind(&SecondWindRules::srd_521(), 1, 13, 13, 0),
        Err(JourneyRuleError::ResourceMissing)
    );
}
#[test]
fn no_hit_dice_rest_requires_uninterrupted_hour_and_caps_one_recovered_use() {
    let rest = NoHitDiceShortRest {
        started: 6,
        finished: 3606,
        ticks_per_second: 1,
        interrupted: false,
        hit_points: 1,
        fighter_second_wind_uses: Some(0),
        knockout_unconscious: true,
    };
    assert_eq!(
        complete_no_hit_dice_short_rest(
            &RestRules::srd_521(),
            NoHitDiceShortRest {
                finished: 3605,
                ..rest
            },
            SecondWindRules::srd_521().uses_max,
            SecondWindRules::srd_521().recharge_per_short_rest
        ),
        Err(JourneyRuleError::RestIncomplete)
    );
    assert_eq!(
        complete_no_hit_dice_short_rest(
            &RestRules::srd_521(),
            NoHitDiceShortRest {
                interrupted: true,
                ..rest
            },
            SecondWindRules::srd_521().uses_max,
            SecondWindRules::srd_521().recharge_per_short_rest
        ),
        Err(JourneyRuleError::RestInterrupted)
    );
    assert_eq!(
        complete_no_hit_dice_short_rest(
            &RestRules::srd_521(),
            NoHitDiceShortRest {
                hit_points: 0,
                ..rest
            },
            SecondWindRules::srd_521().uses_max,
            SecondWindRules::srd_521().recharge_per_short_rest
        ),
        Err(JourneyRuleError::InvalidRest)
    );
    let result = complete_no_hit_dice_short_rest(
        &RestRules::srd_521(),
        rest,
        SecondWindRules::srd_521().uses_max,
        SecondWindRules::srd_521().recharge_per_short_rest,
    )
    .unwrap();
    assert_eq!(
        result,
        ShortRestBenefits {
            hit_points: 1,
            fighter_second_wind_uses: Some(1),
            ends_knockout_unconscious: true
        }
    );
    for uses in [1, 2] {
        assert_eq!(
            complete_no_hit_dice_short_rest(
                &RestRules::srd_521(),
                NoHitDiceShortRest {
                    fighter_second_wind_uses: Some(uses),
                    ..rest
                },
                SecondWindRules::srd_521().uses_max,
                SecondWindRules::srd_521().recharge_per_short_rest
            )
            .unwrap()
            .fighter_second_wind_uses,
            Some(2)
        );
    }
    assert_eq!(
        short_rest_due(6, 2, RestRules::srd_521().short_rest_seconds),
        Ok(7206)
    );
    assert_eq!(
        short_rest_due(u64::MAX, 1, RestRules::srd_521().short_rest_seconds),
        Err(JourneyRuleError::InvalidRest)
    );
    assert_eq!(
        short_rest_due(0, 0, RestRules::srd_521().short_rest_seconds),
        Err(JourneyRuleError::InvalidRest)
    );
}
#[test]
fn baseline_arrays_preserve_both_character_sheets_and_owned_choices() {
    let mut selected = choices();
    for (array, abilities) in [
        ("stalwart", [17, 13, 15, 10, 12, 8]),
        ("vanguard", [17, 14, 14, 8, 10, 12]),
    ] {
        selected[3].1 = array;
        assert_eq!(
            validate_character(&CreationRules::srd_521(), "Brynn", &selected),
            Ok(LegalCharacter {
                name: "Brynn".to_owned(),
                abilities,
                choices: selected
                    .iter()
                    .map(|(group, option)| (group.to_string(), option.to_string()))
                    .collect(),
                hit_points: 13,
                armor_class: 17,
                proficiency_bonus: 2,
                second_wind_uses: 2,
            })
        );
    }
}
#[test]
fn admitted_species_apply_their_own_level_one_toughness_and_unsupported_species_refuse() {
    let mut selected = choices();
    for (species, expected_hp) in [("dwarf", 13), ("halfling", 12), ("orc", 12)] {
        selected[0].1 = species;
        let character = validate_character(&CreationRules::srd_521(), "Brynn", &selected)
            .expect("admitted species uses its own traits");
        assert_eq!(character.hit_points, expected_hp, "{species}");
    }
    selected[0].1 = "dragonborn";
    assert_eq!(
        validate_character(&CreationRules::srd_521(), "Brynn", &selected),
        Err(JourneyRuleError::MissingChoice)
    );
}
#[test]
fn background_origin_feats_match_the_srd_choices() {
    assert_eq!(
        background_origin_feat("acolyte"),
        Some(OriginFeat::MagicInitiate)
    );
    assert_eq!(background_origin_feat("criminal"), Some(OriginFeat::Alert));
    assert_eq!(
        background_origin_feat("sage"),
        Some(OriginFeat::MagicInitiate)
    );
    assert_eq!(
        background_origin_feat("soldier"),
        Some(OriginFeat::SavageAttacker)
    );
    assert_eq!(background_origin_feat("unknown"), None);
}
#[test]
fn srd_521_page_256_challenge_rating_xp_table_is_exact_and_cr_zero_stays_ambiguous() {
    for (cr, xp) in [
        (ChallengeRating::Eighth, 25),
        (ChallengeRating::Quarter, 50),
        (ChallengeRating::Half, 100),
        (ChallengeRating::Whole(1), 200),
        (ChallengeRating::Whole(3), 700),
        (ChallengeRating::Whole(10), 5_900),
        (ChallengeRating::Whole(30), 155_000),
    ] {
        assert_eq!(experience_points_for_challenge_rating(cr), Some(xp));
    }
    assert_eq!(
        experience_points_for_challenge_rating(ChallengeRating::Zero),
        None
    );
    assert_eq!(
        experience_points_for_challenge_rating(ChallengeRating::Whole(31)),
        None
    );
}
#[test]
fn creation_numbers_and_array_subset_change_the_character() {
    let arrays = [AbilityArray {
        id: "stalwart",
        scores: [16, 13, 16, 10, 12, 8],
    }];
    let rules = CreationRules {
        ability_arrays: &arrays,
        background_bonus: BackgroundBonus {
            strength: 1,
            constitution: 2,
        },
        base_hit_points: 20,
        armor_class: 19,
        proficiency_bonus: 4,
        second_wind_uses_max: 3,
    };
    let mut selected = choices();
    let built = validate_character(&rules, "Brynn", &selected).unwrap();
    assert_eq!(built.abilities, [17, 13, 18, 10, 12, 8]);
    assert_eq!(
        (
            built.hit_points,
            built.armor_class,
            built.proficiency_bonus,
            built.second_wind_uses
        ),
        (25, 19, 4, 3)
    );
    selected[3].1 = "vanguard";
    assert_eq!(
        validate_character(&rules, "Brynn", &selected),
        Err(JourneyRuleError::InvalidChoice)
    );
}
#[test]
fn creation_rejects_invalid_parameter_bounds_and_array_ids() {
    let baseline = CreationRules::srd_521();
    for rules in [
        CreationRules {
            base_hit_points: 0,
            ..baseline
        },
        CreationRules {
            base_hit_points: 61,
            ..baseline
        },
        CreationRules {
            armor_class: 9,
            ..baseline
        },
        CreationRules {
            armor_class: 31,
            ..baseline
        },
        CreationRules {
            proficiency_bonus: 1,
            ..baseline
        },
        CreationRules {
            proficiency_bonus: 7,
            ..baseline
        },
        CreationRules {
            second_wind_uses_max: 6,
            ..baseline
        },
        CreationRules {
            background_bonus: BackgroundBonus {
                strength: 5,
                constitution: 1,
            },
            ..baseline
        },
        CreationRules {
            background_bonus: BackgroundBonus {
                strength: 2,
                constitution: 5,
            },
            ..baseline
        },
        CreationRules {
            ability_arrays: &[],
            ..baseline
        },
    ] {
        assert_eq!(
            validate_character(&rules, "Brynn", &choices()),
            Err(JourneyRuleError::InvalidRules)
        );
    }
    for arrays in [
        vec![AbilityArray {
            id: "unknown",
            scores: [10; 6],
        }],
        vec![
            AbilityArray {
                id: "stalwart",
                scores: [10; 6]
            };
            2
        ],
        vec![AbilityArray {
            id: "stalwart",
            scores: [2, 10, 10, 10, 10, 10],
        }],
        vec![AbilityArray {
            id: "stalwart",
            scores: [19, 10, 10, 10, 10, 10],
        }],
        vec![AbilityArray {
            id: "stalwart",
            scores: [255; 6],
        }],
        vec![AbilityArray {
            id: "stalwart",
            scores: [18; 6],
        }],
    ] {
        let rules = CreationRules {
            ability_arrays: &arrays,
            background_bonus: BackgroundBonus {
                strength: 4,
                constitution: 4,
            },
            ..baseline
        };
        assert_eq!(rules.validate(), Err(JourneyRuleError::InvalidRules));
    }
    let low_constitution = [AbilityArray {
        id: "stalwart",
        scores: [10, 10, 3, 10, 10, 10],
    }];
    assert_eq!(
        CreationRules {
            ability_arrays: &low_constitution,
            base_hit_points: 1,
            ..baseline
        }
        .validate(),
        Err(JourneyRuleError::InvalidRules)
    );
}
#[test]
fn baseline_character_rejections_keep_their_typed_errors() {
    let rules = CreationRules::srd_521();
    for name in [
        "",
        "123",
        "R2-D2",
        " Vix",
        "Vix ",
        "Vix  Ash",
        "Vix--Ash",
        "-Vix",
        "Vix!",
        "ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFG",
    ] {
        assert_eq!(
            validate_character(&rules, name, &choices()),
            Err(JourneyRuleError::InvalidName),
            "{name:?}"
        );
    }
}
#[test]
fn hero_names_may_be_simple_or_several_words_in_any_alphabet() {
    let rules = CreationRules::srd_521();
    for name in [
        "Vix",
        "Vix Ashlantern",
        "Thorin Oakenshield II",
        "D'Artagnan",
        "Ka\u{2019}rel",
        "Jean-Luc",
        "St. Brynn",
        "Brynn Jr.",
        "Åsa",
        "Ælfwyn of the Red_Hill",
        "ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEF",
    ] {
        assert!(
            validate_character(&rules, name, &choices()).is_ok(),
            "{name:?}"
        );
    }
    assert_eq!(
        validate_character(&rules, "Brynn", &[]),
        Err(JourneyRuleError::MissingChoice)
    );
    let mut selected = choices();
    selected[1] = selected[0];
    assert_eq!(
        validate_character(&rules, "Brynn", &selected),
        Err(JourneyRuleError::DuplicateChoice)
    );
    selected = choices();
    selected[0].1 = "human";
    assert_eq!(
        validate_character(&rules, "Brynn", &selected),
        Err(JourneyRuleError::MissingChoice)
    );
    selected = choices();
    selected[6].1 = selected[5].1;
    assert_eq!(
        validate_character(&rules, "Brynn", &selected),
        Err(JourneyRuleError::InvalidChoice)
    );
}
fn ordinary_attack() -> NormalMeleeAttack<'static> {
    NormalMeleeAttack {
        critical_threshold: CombatRules::srd_521().critical_threshold,
        damage_sides: SRD_521_BASELINE.hero_weapon.damage_sides,
        die: 15,
        modifier: 5,
        armor_class: 12,
        damage_dice: &[6, 5],
        weapon_dice: 2,
        damage_modifier: 3,
        savage_attacker_take_higher: false,
        graze: false,
        target_hit_points: 20,
    }
}
#[test]
fn baseline_melee_preserves_the_entire_outcome_and_invalid_inputs() {
    assert_eq!(
        resolve_melee(ordinary_attack()),
        Ok(AttackOutcome {
            die: 15,
            modifier: 5,
            total: 20,
            armor_class: 12,
            hit: true,
            critical: false,
            damage_dice: vec![6, 5],
            damage: 14,
            target_hit_points: 6,
            knocked_out: false,
            grazed: false,
        })
    );
    for die in [0, 21] {
        assert_eq!(
            resolve_melee(NormalMeleeAttack {
                die,
                ..ordinary_attack()
            }),
            Err(JourneyRuleError::InvalidDie)
        );
    }
    for weapon_dice in [0, 3] {
        assert_eq!(
            resolve_melee(NormalMeleeAttack {
                weapon_dice,
                ..ordinary_attack()
            }),
            Err(JourneyRuleError::InvalidTarget)
        );
    }
    for damage_dice in [&[0, 5][..], &[7, 5], &[6]] {
        assert_eq!(
            resolve_melee(NormalMeleeAttack {
                damage_dice,
                ..ordinary_attack()
            }),
            Err(JourneyRuleError::InvalidDamageDice)
        );
    }
    assert_eq!(
        resolve_melee(NormalMeleeAttack {
            target_hit_points: 0,
            ..ordinary_attack()
        }),
        Err(JourneyRuleError::InvalidTarget)
    );
}
#[test]
fn configured_weapon_faces_counts_and_critical_threshold_change_damage() {
    for damage_sides in [4, 6, 8, 10, 12] {
        let dice = [u32::from(damage_sides)];
        let result = resolve_melee(NormalMeleeAttack {
            damage_sides,
            damage_dice: &dice,
            weapon_dice: 1,
            ..ordinary_attack()
        })
        .unwrap();
        assert_eq!(result.damage, u32::from(damage_sides) + 3);
    }
    assert_eq!(
        resolve_melee(NormalMeleeAttack {
            damage_sides: 8,
            damage_dice: &[8, 7],
            ..ordinary_attack()
        })
        .unwrap()
        .damage,
        18
    );
    assert_eq!(
        resolve_melee(NormalMeleeAttack {
            damage_dice: &[8, 7],
            ..ordinary_attack()
        }),
        Err(JourneyRuleError::InvalidDamageDice)
    );
    let result = resolve_melee(NormalMeleeAttack {
        die: 19,
        critical_threshold: 19,
        damage_dice: &[6, 5, 4, 3],
        armor_class: 100,
        ..ordinary_attack()
    })
    .unwrap();
    assert!(result.hit && result.critical);
    assert_eq!(result.damage, 21);
    for critical_threshold in [14, 21] {
        assert_eq!(
            resolve_melee(NormalMeleeAttack {
                critical_threshold,
                ..ordinary_attack()
            }),
            Err(JourneyRuleError::InvalidRules)
        );
    }
    for damage_sides in [0, 5, 20] {
        assert_eq!(
            resolve_melee(NormalMeleeAttack {
                damage_sides,
                ..ordinary_attack()
            }),
            Err(JourneyRuleError::InvalidRules)
        );
    }
}
#[test]
fn second_wind_die_bonus_and_boundaries_are_explicit() {
    let baseline = SecondWindRules::srd_521();
    let changed = SecondWindRules {
        die_sides: 6,
        flat_bonus: 4,
        ..baseline
    };
    assert_eq!(heal_second_wind(&baseline, 6, 1, 20, 2), Ok((8, 1)));
    assert_eq!(heal_second_wind(&changed, 6, 1, 20, 2), Ok((11, 1)));
    assert_eq!(
        heal_second_wind(&changed, 7, 1, 20, 2),
        Err(JourneyRuleError::InvalidDie)
    );
    for rules in [
        SecondWindRules {
            uses_max: 6,
            ..baseline
        },
        SecondWindRules {
            die_sides: 20,
            ..baseline
        },
        SecondWindRules {
            flat_bonus: 11,
            ..baseline
        },
        SecondWindRules {
            recharge_per_short_rest: 3,
            ..baseline
        },
    ] {
        assert_eq!(
            heal_second_wind(&rules, 1, 1, 20, 2),
            Err(JourneyRuleError::InvalidRules)
        );
    }
    for (current, maximum) in [(0, 13), (14, 13), (1, 0)] {
        assert_eq!(
            heal_second_wind(&baseline, 1, current, maximum, 2),
            Err(JourneyRuleError::InvalidTarget)
        );
    }
    assert_eq!(
        heal_second_wind(&baseline, 0, 1, 13, 2),
        Err(JourneyRuleError::InvalidDie)
    );
    assert_eq!(
        heal_second_wind(&baseline, 11, 1, 13, 2),
        Err(JourneyRuleError::InvalidDie)
    );
    assert_eq!(
        heal_second_wind(&baseline, 10, u32::MAX, u32::MAX, 2),
        Ok((u32::MAX, 1))
    );
}
#[test]
fn configured_rest_duration_and_recharge_change_only_selected_benefits() {
    let rules = RestRules {
        short_rest_seconds: 60,
    };
    let rest = NoHitDiceShortRest {
        started: 6,
        finished: 66,
        ticks_per_second: 1,
        interrupted: false,
        hit_points: 1,
        fighter_second_wind_uses: Some(1),
        knockout_unconscious: true,
    };
    assert_eq!(short_rest_due(6, 2, rules.short_rest_seconds), Ok(126));
    assert_eq!(
        complete_no_hit_dice_short_rest(&RestRules::srd_521(), rest, 2, 1),
        Err(JourneyRuleError::RestIncomplete)
    );
    assert_eq!(
        complete_no_hit_dice_short_rest(&rules, rest, 4, 2),
        Ok(ShortRestBenefits {
            hit_points: 1,
            fighter_second_wind_uses: Some(3),
            ends_knockout_unconscious: true,
        })
    );
    assert_eq!(
        complete_no_hit_dice_short_rest(&rules, rest, 2, 0)
            .unwrap()
            .fighter_second_wind_uses,
        Some(1)
    );
    assert_eq!(
        complete_no_hit_dice_short_rest(
            &rules,
            NoHitDiceShortRest {
                fighter_second_wind_uses: None,
                ..rest
            },
            2,
            1
        )
        .unwrap()
        .fighter_second_wind_uses,
        None
    );
    for seconds in [59, 86401] {
        assert_eq!(
            short_rest_due(0, 1, seconds),
            Err(JourneyRuleError::InvalidRules)
        );
    }
    for (uses_max, recharge) in [(6, 1), (2, 3)] {
        assert_eq!(
            complete_no_hit_dice_short_rest(&rules, rest, uses_max, recharge),
            Err(JourneyRuleError::InvalidRules)
        );
    }
    assert_eq!(
        complete_no_hit_dice_short_rest(
            &rules,
            NoHitDiceShortRest {
                fighter_second_wind_uses: Some(3),
                ..rest
            },
            2,
            1
        ),
        Err(JourneyRuleError::InvalidRest)
    );
}
#[test]
fn combat_configuration_rejects_each_out_of_bounds_field() {
    let baseline = CombatRules::srd_521();
    for rules in [
        CombatRules {
            critical_threshold: 14,
            ..baseline
        },
        CombatRules {
            critical_threshold: 21,
            ..baseline
        },
        CombatRules {
            round_ticks: 0,
            ..baseline
        },
        CombatRules {
            round_ticks: 601,
            ..baseline
        },
        CombatRules {
            combat_log_entries: 0,
            ..baseline
        },
        CombatRules {
            combat_log_entries: 33,
            ..baseline
        },
    ] {
        assert_eq!(rules.validate(), Err(JourneyRuleError::InvalidRules));
    }
    assert_eq!(baseline.validate(), Ok(()));
    assert_eq!(SecondWindRules::srd_521().validate(), Ok(()));
    assert_eq!(RestRules::srd_521().validate(), Ok(()));
}

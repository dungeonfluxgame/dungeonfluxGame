//! Conformance of the hero build to SRD 5.2.1 creation (pp. 19-23, 28-92): exact sheets of a few
//! classes and species, point-buy and standard-array edges, the cross-pick rules, the design-only
//! marks, the offers and "Surprise me".
use std::collections::BTreeMap;

use super::{
    CoreSpecies, DesignOnlyReason, FeatureStatus, HeroBackground, HeroClass, HeroSheet,
    OfferStatus, build_groups, derive_hero, hero_offers, option_id, surprise_build,
};
use crate::local_journey::JourneyRuleError;

fn picks(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(slot, value)| ((*slot).to_owned(), (*value).to_owned()))
        .collect()
}

fn with(mut base: BTreeMap<String, String>, pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    for (slot, value) in pairs {
        base.insert((*slot).to_owned(), (*value).to_owned());
    }
    base
}

fn without(mut base: BTreeMap<String, String>, slots: &[&str]) -> BTreeMap<String, String> {
    for slot in slots {
        base.remove(*slot);
    }
    base
}

/// A Human Fighter Soldier on the Fighter's standard array row, Chain Mail and Defense.
fn human_fighter() -> BTreeMap<String, String> {
    picks(&[
        ("species", "human"),
        ("species-size", "medium"),
        ("human-skill", "perception"),
        ("versatile", "alert"),
        ("class", "fighter"),
        ("background", "soldier"),
        ("ability-method", "standard-array"),
        ("strength", "15"),
        ("dexterity", "14"),
        ("constitution", "13"),
        ("intelligence", "8"),
        ("wisdom", "10"),
        ("charisma", "12"),
        ("ability-increase", "two-and-one"),
        ("increase-two", "strength"),
        ("increase-one", "constitution"),
        ("class-skill-1", "survival"),
        ("class-skill-2", "acrobatics"),
        ("language-1", "dwarvish"),
        ("language-2", "giant"),
        ("alignment", "lawful-good"),
        ("class-equipment", "a"),
        ("background-equipment", "a"),
        ("fighting-style", "defense"),
        ("gaming-set", "dice"),
    ])
}

/// The Wizard's spell picks of `dwarf_wizard`.
const WIZARD_SPELL_PICKS: [(&str, &str); 13] = [
    ("cantrip-1", "light"),
    ("cantrip-2", "mage-hand"),
    ("cantrip-3", "ray-of-frost"),
    ("spellbook-1", "detect-magic"),
    ("spellbook-2", "comprehend-languages"),
    ("spellbook-3", "identify"),
    ("spellbook-4", "magic-missile"),
    ("spellbook-5", "shield"),
    ("spellbook-6", "sleep"),
    ("prepared-1", "magic-missile"),
    ("prepared-2", "shield"),
    ("prepared-3", "sleep"),
    ("prepared-4", "detect-magic"),
];

/// A Dwarf Wizard Sage by point buy (exactly 27 points), +1 to each of the Sage's abilities.
fn dwarf_wizard() -> BTreeMap<String, String> {
    with(dwarf_wizard_without_spells(), &WIZARD_SPELL_PICKS)
}

/// `dwarf_wizard` before its spells are picked, for a class put in its place.
fn dwarf_wizard_without_spells() -> BTreeMap<String, String> {
    picks(&[
        ("species", "dwarf"),
        ("class", "wizard"),
        ("background", "sage"),
        ("ability-method", "point-buy"),
        ("strength", "8"),
        ("dexterity", "14"),
        ("constitution", "15"),
        ("intelligence", "15"),
        ("wisdom", "10"),
        ("charisma", "8"),
        ("ability-increase", "one-each"),
        ("class-skill-1", "investigation"),
        ("class-skill-2", "medicine"),
        ("language-1", "dwarvish"),
        ("language-2", "elvish"),
        ("alignment", "neutral-good"),
        ("class-equipment", "a"),
        ("background-equipment", "a"),
    ])
}

fn sheet(picks: &BTreeMap<String, String>) -> HeroSheet {
    derive_hero("hero-1", "Brin Ashford", picks).expect("the build is legal")
}

fn problems(picks: &BTreeMap<String, String>) -> Vec<(JourneyRuleError, Option<String>)> {
    derive_hero("hero-1", "Brin Ashford", picks)
        .expect_err("the build is refused")
        .into_iter()
        .map(|problem| (problem.error, problem.group))
        .collect()
}

fn attack<'s>(sheet: &'s HeroSheet, id: &str) -> &'s super::SheetAttack {
    sheet
        .attacks
        .iter()
        .find(|attack| attack.id == id)
        .expect("the attack is on the sheet")
}

#[test]
fn a_human_fighter_soldier_on_the_standard_array_has_the_srd_level_one_numbers() {
    let sheet = sheet(&human_fighter());
    let scores = sheet.ability_scores;
    assert_eq!(
        [
            scores.strength,
            scores.dexterity,
            scores.constitution,
            scores.intelligence,
            scores.wisdom,
            scores.charisma
        ],
        [17, 14, 14, 8, 10, 12]
    );
    assert_eq!(sheet.ability_modifiers.strength, 3);
    assert_eq!(sheet.ability_modifiers.intelligence, -1);
    assert_eq!(sheet.proficiency_bonus, 2);
    assert_eq!(sheet.saving_throw_proficiencies, ["str", "con"]);
    assert_eq!(sheet.saving_throws.strength, 5);
    assert_eq!(sheet.saving_throws.dexterity, 2);
    // 10 (the d10 maximum) + 2 (Constitution 14).
    assert_eq!(sheet.hit_points.maximum, 12);
    // Chain Mail 16, Defense +1; Strength 17 meets Chain Mail's 13, so Speed stays 30.
    assert_eq!(sheet.armor_class.value, 17);
    assert_eq!(sheet.speed_feet, Some(30));
    assert_eq!(sheet.stealth_disadvantage.as_deref(), Some("Chain Mail"));
    // Dexterity +2 and Alert's Proficiency Bonus.
    assert_eq!(sheet.initiative_modifier.value, 4);
    assert_eq!(sheet.skills["athletics"], 5);
    assert_eq!(sheet.skills["intimidation"], 3);
    assert_eq!(sheet.skills["perception"], 2);
    assert_eq!(sheet.skills["acrobatics"], 4);
    assert_eq!(sheet.skills["arcana"], -1);
    assert_eq!(sheet.passive_perception, 12);
    assert_eq!(sheet.skill_proficiencies["perception"], "Human Skillful");
    assert_eq!(sheet.skill_proficiencies["athletics"], "Soldier background");
    let greatsword = attack(&sheet, "greatsword");
    assert_eq!(
        (greatsword.attack_bonus, greatsword.damage.as_str()),
        (5, "2d6+3")
    );
    assert_eq!(attack(&sheet, "javelin").kind, "melee_or_thrown");
    let shortbow = attack(&sheet, "shortbow");
    assert_eq!(
        (
            shortbow.ability,
            shortbow.attack_bonus,
            shortbow.damage.as_str()
        ),
        ("dex", 4, "1d6+2")
    );
    assert_eq!(sheet.healing[0].dice, "1d10+1");
    assert_eq!(sheet.healing[1].dice, "1d10+2");
    // 4 GP from the Fighter's kit A and 14 GP from the Soldier's.
    assert_eq!(sheet.coins_gp, 18);
    assert_eq!(sheet.languages, ["Common", "Dwarvish", "Giant"]);
    assert!(sheet.class_features.contains_key("second_wind"));
    assert!(
        sheet
            .not_in_play
            .iter()
            .any(|mark| mark.subject == "Weapon Mastery")
    );
}

#[test]
fn a_dwarf_wizard_sage_by_point_buy_takes_dwarven_toughness_and_casts_with_intelligence() {
    let sheet = sheet(&dwarf_wizard());
    assert_eq!(sheet.ability_scores.constitution, 16);
    assert_eq!(sheet.ability_scores.intelligence, 16);
    assert_eq!(sheet.ability_scores.wisdom, 11);
    // 6 (the d6 maximum) + 3 (Constitution 16) + 1 (Dwarven Toughness).
    assert_eq!(sheet.hit_points.maximum, 10);
    // No armor: 10 + Dexterity 2.
    assert_eq!(sheet.armor_class.value, 12);
    assert_eq!(sheet.darkvision_feet, Some(120));
    assert_eq!(sheet.saving_throws.intelligence, 5);
    assert_eq!(sheet.saving_throws.strength, -1);
    let dagger = attack(&sheet, "dagger");
    assert_eq!((dagger.ability, dagger.damage.as_str()), ("dex", "1d4+2"));
    assert_eq!(attack(&sheet, "quarterstaff").damage, "1d6-1");
    assert_eq!(sheet.healing.len(), 1);
    for feature in ["spellcasting", "ritual_adept", "arcane_recovery"] {
        assert_eq!(
            sheet.feature_status.get(feature),
            Some(&FeatureStatus::InPlay),
            "{feature} is in play"
        );
    }
    for feature in ["Spellcasting", "Ritual Adept", "Arcane Recovery"] {
        assert!(
            !sheet.not_in_play.iter().any(|mark| mark.subject == feature),
            "{feature} is not marked"
        );
    }
    // The Sage's Magic Initiate (Wizard) is still not in play.
    assert!(
        sheet
            .not_in_play
            .iter()
            .any(|mark| mark.subject == "Magic Initiate"
                && mark.reason == DesignOnlyReason::NotInPlayYet)
    );
    assert!(
        sheet
            .ability_score_note
            .starts_with("Point buy (27 points)")
    );
}

#[test]
fn a_barbarian_in_no_armor_has_unarmored_defense_and_keeps_its_other_features_marked() {
    let picks = with(
        without(
            dwarf_wizard_without_spells(),
            &["class-skill-1", "class-skill-2"],
        ),
        &[
            ("class", "barbarian"),
            ("class-skill-1", "athletics"),
            ("class-skill-2", "survival"),
        ],
    );
    let sheet = sheet(&picks);
    // Unarmored Defense: 10 + Dexterity 2 + Constitution 3 (SRD p. 28).
    assert_eq!(sheet.armor_class.value, 15);
    assert_eq!(sheet.hit_points.maximum, 16);
    assert!(sheet.class_features.contains_key("unarmored_defense"));
    assert!(
        !sheet
            .not_in_play
            .iter()
            .any(|mark| mark.subject == "Unarmored Defense")
    );
    assert!(sheet.not_in_play.iter().any(|mark| mark.subject == "Rage"));
    let greataxe = attack(&sheet, "greataxe");
    assert_eq!(greataxe.damage, "1d12-1");
}

#[test]
fn a_wood_elf_ranger_in_studded_leather_moves_thirty_five_feet() {
    let picks = picks(&[
        ("species", "elf"),
        ("elven-lineage", "wood-elf"),
        ("keen-senses", "perception"),
        ("species-spellcasting", "wisdom"),
        ("class", "ranger"),
        ("background", "criminal"),
        ("ability-method", "standard-array"),
        ("strength", "12"),
        ("dexterity", "15"),
        ("constitution", "13"),
        ("intelligence", "8"),
        ("wisdom", "14"),
        ("charisma", "10"),
        ("ability-increase", "two-and-one"),
        ("increase-two", "dexterity"),
        ("increase-one", "constitution"),
        ("class-skill-1", "survival"),
        ("class-skill-2", "nature"),
        ("class-skill-3", "animal-handling"),
        ("language-1", "elvish"),
        ("language-2", "orc"),
        ("alignment", "chaotic-good"),
        ("class-equipment", "a"),
        ("background-equipment", "b"),
    ]);
    let sheet = sheet(&picks);
    assert_eq!(sheet.speed_feet, Some(35));
    // Studded Leather 12 + Dexterity 3 (17).
    assert_eq!(sheet.armor_class.value, 15);
    // Alert from the Criminal background.
    assert_eq!(sheet.initiative_modifier.value, 5);
    assert_eq!(attack(&sheet, "scimitar").ability, "dex");
    // Ranger kit A (7 GP) and the Criminal's 50 GP.
    assert_eq!(sheet.coins_gp, 57);
    assert_eq!(sheet.skill_proficiencies["perception"], "Elf Keen Senses");
}

#[test]
fn point_buy_accepts_exactly_twenty_seven_points_and_refuses_twenty_eight() {
    let over = with(
        dwarf_wizard(),
        &[
            ("strength", "15"),
            ("dexterity", "15"),
            ("constitution", "15"),
            ("intelligence", "8"),
            ("wisdom", "8"),
            ("charisma", "9"),
        ],
    );
    assert_eq!(
        problems(&over),
        [(
            JourneyRuleError::InvalidChoice,
            Some("ability-method".to_owned())
        )]
    );
    let exact = with(
        dwarf_wizard(),
        &[
            ("strength", "15"),
            ("dexterity", "15"),
            ("constitution", "15"),
            ("intelligence", "8"),
            ("wisdom", "8"),
            ("charisma", "8"),
        ],
    );
    assert!(derive_hero("hero-1", "Brin", &exact).is_ok());
}

#[test]
fn point_buy_offers_no_score_below_eight_or_above_fifteen() {
    for score in ["7", "16"] {
        let picks = with(dwarf_wizard(), &[("strength", score)]);
        assert_eq!(
            problems(&picks),
            [(JourneyRuleError::InvalidChoice, Some("strength".to_owned()))]
        );
    }
}

#[test]
fn the_standard_array_gives_each_score_once() {
    let twice = with(human_fighter(), &[("dexterity", "15")]);
    assert_eq!(
        problems(&twice),
        [(
            JourneyRuleError::InvalidChoice,
            Some("ability-method".to_owned())
        )]
    );
    let off_array = with(human_fighter(), &[("dexterity", "9")]);
    assert!(problems(&off_array).contains(&(
        JourneyRuleError::InvalidChoice,
        Some("dexterity".to_owned())
    )));
}

#[test]
fn the_background_increase_goes_to_two_different_abilities_of_the_background() {
    let same = with(human_fighter(), &[("increase-one", "strength")]);
    assert_eq!(
        problems(&same),
        [(
            JourneyRuleError::InvalidChoice,
            Some("increase-one".to_owned())
        )]
    );
    let foreign = with(human_fighter(), &[("increase-one", "wisdom")]);
    assert_eq!(
        problems(&foreign),
        [(
            JourneyRuleError::InvalidChoice,
            Some("increase-one".to_owned())
        )]
    );
    let one_each = with(
        without(human_fighter(), &["increase-two", "increase-one"]),
        &[("ability-increase", "one-each")],
    );
    let sheet = sheet(&one_each);
    assert_eq!(
        (
            sheet.ability_scores.strength,
            sheet.ability_scores.dexterity,
            sheet.ability_scores.constitution
        ),
        (16, 15, 14)
    );
}

#[test]
fn a_skill_already_granted_is_refused_on_the_later_pick() {
    let twice = with(human_fighter(), &[("class-skill-2", "athletics")]);
    assert_eq!(
        problems(&twice),
        [(
            JourneyRuleError::InvalidChoice,
            Some("class-skill-2".to_owned())
        )]
    );
    let species_twice = with(human_fighter(), &[("human-skill", "survival")]);
    assert_eq!(
        problems(&species_twice),
        [(
            JourneyRuleError::InvalidChoice,
            Some("human-skill".to_owned())
        )]
    );
}

#[test]
fn a_feat_the_background_already_gives_is_refused_unless_repeatable() {
    let alert_twice = with(
        human_fighter(),
        &[
            ("background", "criminal"),
            ("increase-two", "dexterity"),
            ("increase-one", "constitution"),
        ],
    );
    let alert_twice = without(alert_twice, &["gaming-set"]);
    let alert_twice = with(
        alert_twice,
        &[("human-skill", "perception"), ("versatile", "alert")],
    );
    assert_eq!(
        problems(&alert_twice),
        [(
            JourneyRuleError::InvalidChoice,
            Some("versatile".to_owned())
        )]
    );
    let skilled = with(
        human_fighter(),
        &[
            ("versatile", "skilled"),
            ("skilled-1", "arcana"),
            ("skilled-2", "history"),
            ("skilled-3", "stealth"),
        ],
    );
    let sheet = sheet(&skilled);
    assert_eq!(sheet.skill_proficiencies["stealth"], "Skilled feat");
    assert_eq!(sheet.initiative_modifier.value, 2);
}

#[test]
fn every_missing_pick_and_every_pick_not_asked_for_is_a_problem() {
    let empty = derive_hero("hero-1", "Brin", &BTreeMap::new()).expect_err("nothing is picked");
    let missing: Vec<_> = empty
        .iter()
        .filter(|problem| problem.error == JourneyRuleError::MissingChoice)
        .filter_map(|problem| problem.group.clone())
        .collect();
    for slot in [
        "species",
        "class",
        "background",
        "ability-method",
        "alignment",
    ] {
        assert!(missing.contains(&slot.to_owned()), "{slot} is missing");
    }
    let stray = with(dwarf_wizard(), &[("fighting-style", "defense")]);
    assert_eq!(
        problems(&stray),
        [(
            JourneyRuleError::InvalidChoice,
            Some("fighting-style".to_owned())
        )]
    );
    assert_eq!(
        derive_hero("hero-1", "", &dwarf_wizard())
            .expect_err("no name")
            .first()
            .map(|problem| problem.error),
        Some(JourneyRuleError::InvalidName)
    );
}

#[test]
fn a_species_and_background_without_an_admitted_source_derive_with_their_marks() {
    let picks = picks(&[
        ("species", "aasimar"),
        ("class", "cleric"),
        ("background", "farmer"),
        ("ability-method", "standard-array"),
        ("strength", "14"),
        ("dexterity", "8"),
        ("constitution", "13"),
        ("intelligence", "10"),
        ("wisdom", "15"),
        ("charisma", "12"),
        ("class-skill-1", "insight"),
        ("class-skill-2", "medicine"),
        ("language-1", "draconic"),
        ("language-2", "halfling"),
        ("alignment", "neutral"),
        ("class-equipment", "a"),
    ]);
    let sheet = sheet(&picks);
    assert_eq!(sheet.speed_feet, None);
    assert_eq!(sheet.size, None);
    // No background increase: the scores are the array's.
    assert_eq!(sheet.ability_scores.wisdom, 15);
    // Chain Shirt 13 + Dexterity -1, Shield +2.
    assert_eq!(sheet.armor_class.value, 14);
    let not_admitted: Vec<_> = sheet
        .not_in_play
        .iter()
        .filter(|mark| mark.reason == DesignOnlyReason::SourceNotAdmitted)
        .map(|mark| mark.subject.as_str())
        .collect();
    assert_eq!(not_admitted, ["Aasimar", "Farmer"]);
    let increase = with(picks, &[("ability-increase", "one-each")]);
    assert_eq!(
        problems(&increase),
        [(
            JourneyRuleError::InvalidChoice,
            Some("ability-increase".to_owned())
        )]
    );
}

#[test]
fn the_offers_list_every_core_option_with_the_fighter_rogue_and_wizard_in_play() {
    let offers = hero_offers();
    assert_eq!(offers.species.len(), CoreSpecies::ALL.len());
    assert_eq!(offers.classes.len(), 12);
    assert_eq!(offers.backgrounds.len(), 16);
    let playable = |list: &[super::HeroOffer]| {
        list.iter()
            .filter(|offer| offer.status == OfferStatus::Playable)
            .map(|offer| offer.id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(playable(&offers.classes), ["fighter", "rogue", "wizard"]);
    assert_eq!(
        playable(&offers.backgrounds),
        ["acolyte", "criminal", "sage", "soldier"]
    );
    assert_eq!(playable(&offers.species).len(), 9);
    let aasimar = offers
        .species
        .iter()
        .find(|offer| offer.id == "aasimar")
        .expect("the Aasimar is offered");
    assert!(matches!(
        aasimar.status,
        OfferStatus::DesignOnly {
            reason: DesignOnlyReason::SourceNotAdmitted,
            ..
        }
    ));
}

#[test]
fn surprise_me_proposes_a_legal_build_for_every_species_and_class() {
    for species in CoreSpecies::ALL {
        for class in HeroClass::ALL {
            for seed in [0_usize, 1, 7, 13] {
                let mut counter = seed;
                let mut pick = |count: usize| {
                    counter = counter.wrapping_mul(31).wrapping_add(17);
                    counter % count
                };
                let species_id = option_id(species.key());
                let class_id = option_id(class.key());
                let start = picks(&[("species", &species_id), ("class", &class_id)]);
                let built = surprise_build(&start, &mut pick);
                assert_eq!(built.get("species"), start.get("species"));
                assert_eq!(built.get("class"), start.get("class"));
                if let Err(problems) = derive_hero("hero-1", "Brin", &built) {
                    panic!("{species:?} {class:?} seed {seed}: {problems:?}");
                }
            }
        }
    }
}

#[test]
fn surprise_me_keeps_a_design_only_background_the_player_chose() {
    let start = picks(&[
        ("species", "orc"),
        ("class", "rogue"),
        ("background", "sailor"),
    ]);
    let built = surprise_build(&start, &mut |_| 0);
    assert_eq!(built.get("background").map(String::as_str), Some("sailor"));
    let sheet = sheet(&built);
    assert!(
        sheet
            .not_in_play
            .iter()
            .any(|mark| mark.subject == HeroBackground::Sailor.title())
    );
}

#[test]
fn the_groups_a_build_asks_for_follow_the_class_species_and_background() {
    let groups = build_groups(&human_fighter());
    let slots: Vec<&str> = groups
        .iter()
        .flat_map(|group| group.slots.iter().map(String::as_str))
        .collect();
    for slot in [
        "fighting-style",
        "gaming-set",
        "human-skill",
        "increase-two",
    ] {
        assert!(slots.contains(&slot), "{slot} is asked");
    }
    let wizard_slots: Vec<String> = build_groups(&dwarf_wizard())
        .into_iter()
        .flat_map(|group| group.slots)
        .collect();
    assert!(!wizard_slots.contains(&"fighting-style".to_owned()));
    assert!(!wizard_slots.contains(&"increase-two".to_owned()));
}

#[test]
fn the_sheet_is_written_in_the_gauntlet_sheet_file_shape() {
    let written = serde_json::to_value(sheet(&human_fighter())).expect("the sheet writes");
    assert_eq!(written["ability_modifiers"]["str"], 3);
    assert_eq!(written["initiative_modifier"]["value"], 4);
    assert_eq!(
        written["skill_proficiencies"]["sleight_of_hand"],
        serde_json::Value::Null
    );
    assert_eq!(written["attacks"][0]["damage"], "2d6+3");
    assert_eq!(written["healing"][0]["bonus_label"], "class_level");
    assert_eq!(written["critical_range"][0], 20);
    assert!(written.get("abilities").is_none());
}

#[test]
fn fill_keeps_every_pick_still_offered_and_fills_the_rest() {
    let start = picks(&[
        ("species", "dwarf"),
        ("class", "wizard"),
        ("background", "sage"),
        ("class-skill-1", "medicine"),
        ("language-1", "giant"),
    ]);
    let built = super::fill_build(&start, &Default::default(), &mut |count| count / 2);
    for (slot, value) in &start {
        assert_eq!(built.get(slot), Some(value), "{slot} is kept");
    }
    assert!(derive_hero("hero-1", "Brin", &built).is_ok());
}

#[test]
fn fill_lets_a_refused_pick_give_way_and_drops_what_the_build_no_longer_asks() {
    // Athletics is the Soldier's already; the gaming set and the Fighter's style are not asked of
    // a Sage Wizard; Strength is not one of the Sage's abilities to raise.
    let start = with(
        human_fighter(),
        &[
            ("class", "wizard"),
            ("background", "sage"),
            ("class-skill-1", "arcana"),
        ],
    );
    let built = super::fill_build(&start, &Default::default(), &mut |_| 0);
    let sheet = sheet(&built);
    assert_eq!(sheet.class, "Wizard");
    assert!(!built.contains_key("gaming-set"));
    assert!(!built.contains_key("fighting-style"));
    assert_ne!(
        built.get("increase-two").map(String::as_str),
        Some("strength")
    );
    assert_eq!(
        built.get("human-skill").map(String::as_str),
        Some("perception")
    );
}

#[test]
fn the_equipment_choices_read_as_what_each_kit_holds() {
    let groups = build_groups(&human_fighter());
    let kits = groups
        .iter()
        .find(|group| group.id == "class-equipment")
        .expect("the class's kits are asked");
    assert_eq!(
        kits.options[0].label,
        "A: Chain Mail, Greatsword, Flail, Javelin (8), Dungeoneer's Pack, 4 GP"
    );
    assert_eq!(kits.options[2].label, "C: 155 GP");
    assert!(super::because("class-skills").is_some());
}

#[test]
fn a_monk_takes_wisdom_into_armor_class_without_armor_or_shield() {
    let picks = with(
        without(
            dwarf_wizard_without_spells(),
            &["class-skill-1", "class-skill-2"],
        ),
        &[
            ("class", "monk"),
            ("class-skill-1", "acrobatics"),
            ("class-skill-2", "stealth"),
            ("monk-tool", "flute"),
        ],
    );
    let sheet = sheet(&picks);
    // 10 + Dexterity 2 + Wisdom 0 (11).
    assert_eq!(sheet.armor_class.value, 12);
    assert!(sheet.armor_class.note.starts_with("Unarmored Defense"));
}

#[test]
fn vitals_take_damage_heal_spend_a_hit_die_and_rest_by_the_rules() {
    use super::{HeroVitals, VitalsChange, VitalsError};
    let fighter = sheet(&human_fighter());
    let fresh = HeroVitals::fresh(&fighter);
    assert_eq!(
        (fresh.current_hp, fresh.max_hp, fresh.hit_dice_left),
        (12, 12, 1)
    );
    assert_eq!(fresh.features[0].left, 2);
    let hurt = fresh
        .apply(&VitalsChange::GainTemporary { amount: 3 })
        .and_then(|vitals| vitals.apply(&VitalsChange::Damage { amount: 20 }))
        .expect("damage applies");
    assert_eq!((hurt.temp_hp, hurt.current_hp), (0, 0));
    let healed = hurt
        .apply(&VitalsChange::SpendHitDie { face: 1 })
        .expect("a Hit Point Die is left");
    // 1 + Constitution 2.
    assert_eq!((healed.current_hp, healed.hit_dice_left), (3, 0));
    assert_eq!(
        healed.apply(&VitalsChange::SpendHitDie { face: 4 }),
        Err(VitalsError::NoHitDiceLeft)
    );
    assert_eq!(
        fresh.apply(&VitalsChange::SpendHitDie { face: 11 }),
        Err(VitalsError::InvalidFace {
            face: 11,
            sides: 10
        })
    );
    let winded = fresh
        .apply(&VitalsChange::UseFeature {
            feature: "second_wind".to_owned(),
        })
        .and_then(|vitals| {
            vitals.apply(&VitalsChange::UseFeature {
                feature: "second_wind".to_owned(),
            })
        })
        .expect("two uses at level 1");
    assert!(matches!(
        winded.apply(&VitalsChange::UseFeature {
            feature: "second_wind".to_owned()
        }),
        Err(VitalsError::NoUsesLeft { .. })
    ));
    let rested = winded.apply(&VitalsChange::ShortRest).expect("rests");
    assert_eq!(rested.features[0].left, 1);
    let renewed = healed.apply(&VitalsChange::LongRest).expect("rests");
    assert_eq!((renewed.current_hp, renewed.hit_dice_left), (12, 1));
    assert_eq!(
        fresh.apply(&VitalsChange::Heal { amount: 0 }),
        Err(VitalsError::ZeroAmount)
    );
}

#[test]
fn a_fighter_starts_with_the_soldier_and_fighter_kits_priced_and_its_gold_apart() {
    let picks = [
        ("species", "human"),
        ("species-size", "medium"),
        ("human-skill", "perception"),
        ("versatile", "alert"),
        ("class", "fighter"),
        ("background", "soldier"),
        ("ability-method", "standard-array"),
        ("strength", "15"),
        ("dexterity", "14"),
        ("constitution", "13"),
        ("intelligence", "8"),
        ("wisdom", "10"),
        ("charisma", "12"),
        ("ability-increase", "two-and-one"),
        ("increase-two", "strength"),
        ("increase-one", "constitution"),
        ("class-skill-1", "survival"),
        ("class-skill-2", "acrobatics"),
        ("language-1", "dwarvish"),
        ("language-2", "giant"),
        ("alignment", "lawful-good"),
        ("class-equipment", "a"),
        ("background-equipment", "a"),
        ("fighting-style", "defense"),
        ("gaming-set", "dice"),
    ]
    .into_iter()
    .map(|(slot, value)| (slot.to_owned(), value.to_owned()))
    .collect();
    let kit = super::starting_kit(&picks).expect("a legal build");
    assert_eq!(kit.gold_gp, 18);
    let item = |id: &str| kit.items.iter().find(|item| item.id == id).cloned();
    let chain = item("chain-mail").expect("chain mail");
    assert_eq!((chain.count, chain.price_cp), (1, Some(7_500)));
    assert_eq!(item("javelin").map(|item| item.count), Some(8));
    assert_eq!(item("arrows").and_then(|item| item.price_cp), Some(5));
    assert_eq!(
        item("dungeoneers-pack").and_then(|item| item.price_cp),
        Some(1_200)
    );
    assert_eq!(
        kit.gear_words().as_deref(),
        Some("wearing Chain Mail, wielding a Greatsword")
    );
}

#[test]
fn the_sheet_names_the_style_each_attack_rolls_with() {
    use crate::roll::AttackStyle;
    let archer = sheet(&with(human_fighter(), &[("fighting-style", "archery")]));
    let shortbow = attack(&archer, "shortbow");
    assert_eq!(shortbow.style, Some(AttackStyle::Archery));
    // 1d6+2 Dexterity, +2 Proficiency and +2 Archery.
    assert_eq!(shortbow.attack_bonus, 6);
    assert_eq!(attack(&archer, "greatsword").style, None);
    let great = sheet(&with(
        human_fighter(),
        &[("fighting-style", "great-weapon-fighting")],
    ));
    assert_eq!(
        attack(&great, "greatsword").style,
        Some(AttackStyle::GreatWeaponFighting)
    );
    assert_eq!(attack(&great, "flail").style, None);
    assert_eq!(attack(&great, "shortbow").style, None);
    let written = serde_json::to_value(&great).expect("writes");
    assert_eq!(written["attacks"][0]["style"], "great_weapon_fighting");
}

/// A Tiefling Rogue Criminal on the Rogue's standard array row, Expertise in Stealth and
/// Perception, mastering the Dagger and the Shortbow.
fn tiefling_rogue() -> BTreeMap<String, String> {
    picks(&[
        ("species", "tiefling"),
        ("fiendish-legacy", "infernal"),
        ("species-size", "medium"),
        ("species-spellcasting", "charisma"),
        ("class", "rogue"),
        ("background", "criminal"),
        ("ability-method", "standard-array"),
        ("strength", "12"),
        ("dexterity", "15"),
        ("constitution", "13"),
        ("intelligence", "14"),
        ("wisdom", "10"),
        ("charisma", "8"),
        ("ability-increase", "two-and-one"),
        ("increase-two", "dexterity"),
        ("increase-one", "constitution"),
        ("class-skill-1", "acrobatics"),
        ("class-skill-2", "deception"),
        ("class-skill-3", "perception"),
        ("class-skill-4", "investigation"),
        ("expertise-1", "stealth"),
        ("expertise-2", "perception"),
        ("mastery-1", "dagger"),
        ("mastery-2", "shortbow"),
        ("rogue-language", "infernal"),
        ("language-1", "elvish"),
        ("language-2", "dwarvish"),
        ("alignment", "chaotic-good"),
        ("class-equipment", "a"),
        ("background-equipment", "a"),
    ])
}

#[test]
fn a_rogue_doubles_the_proficiency_bonus_of_its_two_expertise_skills_only() {
    let sheet = sheet(&tiefling_rogue());
    // Dexterity 15 + 2 (Criminal) = 17, +3; Wisdom 10, +0; Proficiency Bonus 2 (SRD p. 23).
    assert_eq!(sheet.expertise, ["stealth", "perception"]);
    // Expertise doubles the bonus (SRD p. 182): 3 + 4 and 0 + 4.
    assert_eq!(sheet.skills["stealth"], 7);
    assert_eq!(sheet.skills["perception"], 4);
    // Proficient only: 3 + 2.
    assert_eq!(sheet.skills["sleight_of_hand"], 5);
    assert_eq!(sheet.skills["acrobatics"], 5);
    // Not proficient: Strength 12, +1.
    assert_eq!(sheet.skills["athletics"], 1);
    assert_eq!(sheet.passive_perception, 14);
    // Leather Armor 11 + Dexterity 3; 8 (d8) + Constitution 2.
    assert_eq!(sheet.armor_class.value, 14);
    assert_eq!(sheet.hit_points.maximum, 10);
    assert_eq!(sheet.saving_throw_proficiencies, ["dex", "int"]);
    assert_eq!(
        sheet
            .tool_proficiencies
            .iter()
            .filter(|tool| *tool == "Thieves' Tools")
            .count(),
        1,
        "the Criminal's and the Rogue's Thieves' Tools are one proficiency"
    );
    assert!(sheet.languages.contains(&"Thieves' Cant".to_owned()));
    assert!(sheet.languages.contains(&"Infernal".to_owned()));
    assert!(sheet.spellcasting.is_none());
}

#[test]
fn a_rogue_shows_sneak_attack_and_weapon_mastery_as_applying_in_combat() {
    let sheet = sheet(&tiefling_rogue());
    for (feature, status) in [
        ("expertise", FeatureStatus::InPlay),
        ("sneak_attack", FeatureStatus::AppliesInCombat),
        ("thieves_cant", FeatureStatus::InPlay),
        ("weapon_mastery", FeatureStatus::AppliesInCombat),
    ] {
        assert_eq!(
            sheet.feature_status.get(feature),
            Some(&status),
            "{feature}"
        );
        assert!(sheet.class_features.contains_key(feature), "{feature}");
    }
    assert!(sheet.class_features["sneak_attack"].contains("1d6"));
    let masteries: Vec<_> = sheet
        .weapon_masteries
        .iter()
        .map(|mastery| (mastery.id, mastery.mastery))
        .collect();
    assert_eq!(masteries, [("dagger", "Nick"), ("shortbow", "Vex")]);
    for feature in [
        "Expertise",
        "Sneak Attack",
        "Thieves' Cant",
        "Weapon Mastery",
    ] {
        assert!(
            !sheet.not_in_play.iter().any(|mark| mark.subject == feature),
            "{feature} is in play"
        );
    }
}

#[test]
fn a_rogue_picks_expertise_among_its_skills_and_masters_only_weapons_it_is_proficient_with() {
    let groups = build_groups(&tiefling_rogue());
    let options = |id: &str| {
        groups
            .iter()
            .find(|group| group.id == id)
            .expect("the group is asked")
            .options
            .iter()
            .map(|option| option.id.clone())
            .collect::<Vec<_>>()
    };
    let expertise = options("expertise");
    for skill in ["sleight-of-hand", "stealth", "acrobatics", "perception"] {
        assert!(expertise.contains(&skill.to_owned()), "{skill}");
    }
    assert!(!expertise.contains(&"athletics".to_owned()));
    let mastery = options("weapon-mastery");
    for weapon in ["dagger", "rapier", "shortsword", "hand-crossbow", "club"] {
        assert!(mastery.contains(&weapon.to_owned()), "{weapon}");
    }
    for weapon in ["longsword", "greatsword", "longbow"] {
        assert!(!mastery.contains(&weapon.to_owned()), "{weapon}");
    }
    for (slot, value) in [
        ("expertise-2", "athletics"),
        ("expertise-2", "stealth"),
        ("rogue-language", "elvish"),
    ] {
        assert_eq!(
            problems(&with(tiefling_rogue(), &[(slot, value)])),
            [(JourneyRuleError::InvalidChoice, Some(slot.to_owned()))],
            "{slot} {value}"
        );
    }
}

#[test]
fn a_wizard_has_its_spell_numbers_cantrips_spellbook_and_two_slots() {
    let sheet = sheet(&dwarf_wizard());
    let casting = sheet.spellcasting.expect("a Wizard casts");
    // Intelligence 16, +3; Proficiency Bonus 2; DC 8 + 3 + 2 (SRD p. 106).
    assert_eq!(casting.ability, "int");
    assert_eq!((casting.save_dc, casting.attack_bonus), (13, 5));
    assert_eq!(casting.prepared_max, 4);
    assert_eq!(casting.slots, [super::SlotCount { level: 1, count: 2 }]);
    assert_eq!(casting.spells.len(), 9);
    let cantrips: Vec<_> = casting
        .spells
        .iter()
        .filter(|spell| spell.level == 0)
        .map(|spell| (spell.id, spell.prepared))
        .collect();
    assert_eq!(
        cantrips,
        [("light", true), ("mage-hand", true), ("ray-of-frost", true)]
    );
    let prepared: Vec<_> = casting
        .spells
        .iter()
        .filter(|spell| spell.level == 1 && spell.prepared)
        .map(|spell| spell.id)
        .collect();
    assert_eq!(
        prepared,
        ["detect-magic", "magic-missile", "shield", "sleep"]
    );
    let identify = casting.known("identify").expect("in the spellbook");
    assert!(identify.ritual && !identify.prepared);
}

#[test]
fn a_wizard_prepares_only_spells_of_its_spellbook_and_each_once() {
    let groups = build_groups(&dwarf_wizard());
    let prepared = groups
        .iter()
        .find(|group| group.id == "prepared-spells")
        .expect("the Wizard prepares spells");
    assert_eq!(prepared.options.len(), 6);
    for (slot, value) in [
        ("prepared-4", "burning-hands"),
        ("prepared-4", "shield"),
        ("cantrip-3", "magic-missile"),
    ] {
        assert_eq!(
            problems(&with(dwarf_wizard(), &[(slot, value)])),
            [(JourneyRuleError::InvalidChoice, Some(slot.to_owned()))],
            "{slot} {value}"
        );
    }
}

#[test]
fn a_wizard_spends_its_slots_and_gets_them_back_on_a_long_rest() {
    use super::{HeroVitals, VitalsChange, VitalsError};
    let fresh = HeroVitals::fresh(&sheet(&dwarf_wizard()));
    assert_eq!(fresh.slots_left(1), 2);
    let spend = VitalsChange::SpendSpellSlot { level: 1 };
    let spent = fresh
        .apply(&spend)
        .and_then(|vitals| vitals.apply(&spend))
        .expect("two level 1 slots");
    assert_eq!(spent.slots_left(1), 0);
    assert_eq!(
        spent.apply(&spend),
        Err(VitalsError::NoSpellSlot { level: 1 })
    );
    assert_eq!(
        fresh.apply(&VitalsChange::SpendSpellSlot { level: 2 }),
        Err(VitalsError::NoSpellSlot { level: 2 })
    );
    // A Short Rest gives no slot back; a Long Rest gives back every one (SRD p. 104).
    let short = spent.apply(&VitalsChange::ShortRest).expect("rests");
    assert_eq!(short.slots_left(1), 0);
    let long = spent.apply(&VitalsChange::LongRest).expect("rests");
    assert_eq!(long.slots_left(1), 2);
    let fighter = HeroVitals::fresh(&sheet(&human_fighter()));
    assert_eq!(
        fighter.apply(&spend),
        Err(VitalsError::NoSpellSlot { level: 1 })
    );
}

#[test]
fn arcane_recovery_brings_back_one_slot_level_once_per_long_rest() {
    use super::{HeroVitals, VitalsChange, VitalsError};
    let fresh = HeroVitals::fresh(&sheet(&dwarf_wizard()));
    // Half of Wizard level 1, rounded up (SRD p. 78).
    assert_eq!(fresh.recovery_levels, 1);
    let recover = |levels: &[u8]| VitalsChange::ArcaneRecovery {
        levels: levels.to_vec(),
    };
    assert_eq!(
        fresh.apply(&recover(&[1])),
        Err(VitalsError::SlotNotExpended { level: 1 })
    );
    let spend = VitalsChange::SpendSpellSlot { level: 1 };
    let spent = fresh
        .apply(&spend)
        .and_then(|vitals| vitals.apply(&spend))
        .expect("two level 1 slots");
    assert_eq!(
        spent.apply(&recover(&[1, 1])),
        Err(VitalsError::RecoveryOverBudget {
            levels: 2,
            budget: 1
        })
    );
    assert_eq!(
        spent.apply(&recover(&[6])),
        Err(VitalsError::RecoveryLevelTooHigh { level: 6 })
    );
    assert_eq!(
        spent.apply(&recover(&[])),
        Err(VitalsError::NothingToRecover)
    );
    let recovered = spent.apply(&recover(&[1])).expect("one level recovered");
    assert_eq!(recovered.slots_left(1), 1);
    let spent_again = recovered.apply(&spend).expect("a slot is back");
    assert!(matches!(
        spent_again.apply(&recover(&[1])),
        Err(VitalsError::NoUsesLeft { .. })
    ));
    let rested = spent_again.apply(&VitalsChange::LongRest).expect("rests");
    let again = rested
        .apply(&spend)
        .and_then(|vitals| vitals.apply(&recover(&[1])))
        .expect("a Long Rest gives Arcane Recovery back");
    assert_eq!(again.slots_left(1), 2);
    let rogue = HeroVitals::fresh(&sheet(&tiefling_rogue()));
    assert_eq!(rogue.recovery_levels, 0);
    assert!(rogue.spell_slots.is_empty());
}

#[test]
fn vitals_of_another_build_do_not_fit() {
    use super::HeroVitals;
    let wizard = HeroVitals::fresh(&sheet(&dwarf_wizard()));
    let mut kept = wizard.clone();
    kept.spell_slots.clear();
    assert!(wizard.fits(&wizard));
    assert!(!kept.fits(&wizard));
    let rogue = HeroVitals::fresh(&sheet(&tiefling_rogue()));
    assert!(!rogue.fits(&wizard));
}

#[test]
fn a_wizard_casts_a_prepared_spell_with_a_slot_and_a_ritual_from_the_book_without_one() {
    use super::{CastError, plan_cast};
    let wizard = sheet(&dwarf_wizard());
    let missile = plan_cast(&wizard, "magic-missile", 1, false).expect("prepared");
    assert_eq!((missile.slot, missile.ritual), (Some(1), false));
    // Identify is in the spellbook, not prepared: Ritual Adept casts it as a Ritual (SRD p. 78).
    let identify = plan_cast(&wizard, "identify", 1, true).expect("a ritual of the book");
    assert_eq!(
        (identify.slot, identify.ritual, identify.extra_minutes),
        (None, true, 10)
    );
    let light = plan_cast(&wizard, "light", 0, false).expect("a cantrip");
    assert_eq!(light.slot, None);
    let named = |spell: &str| spell.to_owned();
    for (spell, level, ritual, error) in [
        (
            "identify",
            1,
            false,
            CastError::NotPrepared {
                spell: named("identify"),
            },
        ),
        (
            "magic-missile",
            1,
            true,
            CastError::NotARitual {
                spell: named("magic-missile"),
            },
        ),
        (
            "alarm",
            1,
            true,
            CastError::NotKnown {
                spell: named("alarm"),
            },
        ),
        (
            "fireball",
            3,
            false,
            CastError::UnknownSpell {
                spell: named("fireball"),
            },
        ),
        (
            "light",
            1,
            false,
            CastError::InvalidLevel {
                spell: named("light"),
                level: 1,
            },
        ),
        (
            "detect-magic",
            2,
            true,
            CastError::InvalidLevel {
                spell: named("detect-magic"),
                level: 2,
            },
        ),
        ("magic-missile", 2, false, CastError::NoSlot { level: 2 }),
    ] {
        assert_eq!(
            plan_cast(&wizard, spell, level, ritual),
            Err(error),
            "{spell} {level} {ritual}"
        );
    }
    let rogue = sheet(&tiefling_rogue());
    assert_eq!(
        plan_cast(&rogue, "light", 0, false),
        Err(CastError::NotACaster)
    );
}

#[test]
fn surprise_me_fills_the_rogue_and_wizard_choices() {
    for (class, slots) in [
        ("rogue", ["expertise-1", "mastery-2", "rogue-language"]),
        ("wizard", ["cantrip-3", "spellbook-6", "prepared-4"]),
    ] {
        for seed in [0_usize, 3, 11] {
            let mut counter = seed;
            let mut pick = |count: usize| {
                counter = counter.wrapping_mul(31).wrapping_add(17);
                counter % count
            };
            let built = surprise_build(&picks(&[("class", class)]), &mut pick);
            for slot in slots {
                assert!(built.contains_key(slot), "{class} {slot}");
            }
            derive_hero("hero-1", "Brin", &built).expect("a legal build");
        }
    }
}

#[test]
fn the_rogue_and_wizard_kits_are_catalog_items_and_the_spellbook_weighs_three_pounds() {
    use crate::srd_equipment_prices::{Weight, equipment};
    // The Sage's 50 GP in place of its kit, so only the Wizard's class kit is checked.
    let wizard_kit_only = with(dwarf_wizard(), &[("background-equipment", "b")]);
    for build in [tiefling_rogue(), wizard_kit_only] {
        let kit = super::starting_kit(&build).expect("a legal build");
        for item in &kit.items {
            if item.id == "spellbook" {
                // The Spellbook weighs 3 pounds; the SRD prints no cost for it (p. 78).
                let book = equipment(&item.id).expect("in the catalog");
                assert_eq!(book.unit_weight(), Weight::pounds(3));
                continue;
            }
            assert!(
                equipment(&item.id).is_some(),
                "{} is in the catalog",
                item.id
            );
            assert!(item.price_cp.is_some(), "{} is priced", item.id);
        }
    }
    let wizard = super::starting_kit(&dwarf_wizard()).expect("a legal build");
    assert!(wizard.items.iter().any(|item| item.id == "spellbook"));
    assert!(wizard.items.iter().any(|item| item.id == "scholars-pack"));
    let rogue = super::starting_kit(&tiefling_rogue()).expect("a legal build");
    for id in [
        "burglars-pack",
        "thieves-tools",
        "leather-armor",
        "shortbow",
    ] {
        assert!(rogue.items.iter().any(|item| item.id == id), "{id}");
    }
}

#[test]
fn a_thousand_seeded_rolls_never_pick_an_option_not_in_play() {
    let offers = hero_offers();
    let design_only = |list: &[super::HeroOffer], id: Option<&String>| {
        list.iter()
            .any(|offer| Some(&offer.id) == id && offer.status != OfferStatus::Playable)
    };
    let classes = ["fighter", "rogue", "wizard"];
    for seed in 0..1_000_usize {
        let mut counter = seed;
        let mut pick = |count: usize| {
            counter = counter
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (counter >> 33) % count
        };
        let start = picks(&[("class", classes[seed % classes.len()])]);
        let rolled = if seed % 2 == 0 {
            surprise_build(&BTreeMap::new(), &mut pick)
        } else {
            super::fill_build(&start, &Default::default(), &mut pick)
        };
        assert!(
            !design_only(&offers.species, rolled.get("species")),
            "seed {seed}: {rolled:?}"
        );
        assert!(
            !design_only(&offers.classes, rolled.get("class")),
            "seed {seed}: {rolled:?}"
        );
        assert!(
            !design_only(&offers.backgrounds, rolled.get("background")),
            "seed {seed}: {rolled:?}"
        );
        derive_hero("hero-1", "Brin", &rolled).expect("a legal build");
        let book: Vec<_> = super::slots::SPELLBOOK
            .iter()
            .filter_map(|slot| rolled.get(*slot))
            .collect();
        for slot in super::slots::PREPARED {
            if let Some(spell) = rolled.get(slot) {
                assert!(book.contains(&spell), "seed {seed}: {spell} is in the book");
            }
        }
    }
}

#[test]
fn fill_leaves_a_slot_the_player_cleared_empty_and_fills_the_rest() {
    let cleared: std::collections::BTreeSet<String> =
        ["cantrip-2".to_owned(), "spellbook-4".to_owned()].into();
    let start = without(dwarf_wizard(), &["cantrip-2", "spellbook-4", "prepared-1"]);
    let filled = super::fill_build(&start, &cleared, &mut |count| count / 2);
    assert!(!filled.contains_key("cantrip-2"));
    assert!(!filled.contains_key("spellbook-4"));
    // A slot the player never cleared is filled, and every other pick is kept.
    assert!(filled.contains_key("prepared-1"));
    assert_eq!(filled.get("cantrip-1").map(String::as_str), Some("light"));
    assert_eq!(filled.get("species").map(String::as_str), Some("dwarf"));
    let problems = problems(&filled);
    assert_eq!(
        problems,
        [
            (
                JourneyRuleError::MissingChoice,
                Some("cantrip-2".to_owned())
            ),
            (
                JourneyRuleError::MissingChoice,
                Some("spellbook-4".to_owned())
            ),
        ]
    );
}

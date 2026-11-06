use super::*;

// SRD 5.2.1, pp. 290, 302, 325, 343, 353, 355, 357, 364: these eight blocks have Darkvision 60 ft.
#[test]
fn darkvision_is_disclosed_for_every_srd_creature_that_has_it() {
    for (id, page) in [
        ("goblin-warrior", 290),
        ("kobold-warrior", 302),
        ("skeleton", 325),
        ("zombie", 343),
        ("giant-rat", 353),
        ("giant-weasel", 355),
        ("mastiff", 357),
        ("wolf", 364),
    ] {
        let creature = SRD_521_CREATURES
            .iter()
            .find(|creature| creature.id == id)
            .expect("SRD creature from source list");
        assert_eq!(creature.page, page);
        let darkvision = SRD_521_CREATURE_FEATURES
            .iter()
            .find(|row| row.creature == id && row.name == "Darkvision 60 ft.")
            .unwrap_or_else(|| panic!("{id} at SRD p. {page} needs Darkvision disclosed"));
        assert_eq!(darkvision.kind, Sense);
        assert_eq!(darkvision.status, Implemented);
        assert!(
            darkvision
                .note
                .contains("can see in darkness within 60 ft.")
        );
        assert!(darkvision.note.contains("tactical fight"));
    }
    for (id, absent_2014_trait) in [
        ("wolf", "Keen Hearing and Smell"),
        ("giant-rat", "Keen Smell"),
        ("giant-weasel", "Keen Hearing and Smell"),
        ("mastiff", "Keen Hearing and Smell"),
    ] {
        assert!(
            !SRD_521_CREATURE_FEATURES
                .iter()
                .any(|row| row.creature == id && row.name == absent_2014_trait),
            "{} incorrectly uses the non-SRD 5.2.1 trait {absent_2014_trait}",
            id
        );
    }
}

#[test]
fn creature_languages_distinguish_speech_from_understanding() {
    let creature = |id| {
        SRD_521_CREATURES
            .iter()
            .find(|creature| creature.id == id)
            .unwrap()
    };
    assert_eq!(creature("giant-weasel").languages, "None");
    assert!(!creature("giant-weasel").can_speak());
    assert_eq!(creature("bandit").languages, "Common, Thieves' Cant");
    assert!(creature("bandit").can_speak());
    assert_eq!(
        creature("scout").languages,
        "Common plus one other language"
    );
    assert!(creature("scout").can_speak());
    assert_eq!(
        creature("skeleton").languages,
        "Understands Common plus one other language but can't speak"
    );
    assert!(!creature("skeleton").can_speak());
    assert_eq!(
        creature("zombie").languages,
        "Understands Common plus one other language but can't speak"
    );
    assert!(!creature("zombie").can_speak());
}

#[test]
fn compiled_creatures_preserve_the_bandit_baseline_and_exact_subset() {
    assert_eq!(
        SRD_521_CREATURES[0].rules,
        crate::local_journey::SRD_521_BASELINE.bandit
    );
    assert_eq!(SRD_521_CREATURES.iter().filter(|c| c.exact).count(), 7);
    assert!(
        !SRD_521_CREATURES
            .iter()
            .find(|c| c.id == "zombie")
            .unwrap()
            .exact
    );
}
#[test]
fn every_compiled_creature_has_feature_rows_and_missing_ones_are_named() {
    for creature in &SRD_521_CREATURES {
        assert!(
            SRD_521_CREATURE_FEATURES
                .iter()
                .any(|row| row.creature == creature.id && row.kind == Action),
            "{} lists no action",
            creature.id
        );
    }
    for row in SRD_521_CREATURE_FEATURES {
        assert!(
            SRD_521_CREATURES
                .iter()
                .chain([&SCENE_COMMONER])
                .any(|c| c.id == row.creature),
            "{}",
            row.creature
        );
        assert!(!row.note.is_empty());
    }
    assert_eq!(missing_features("giant-rat").count(), 1);
    assert_eq!(missing_features("kobold-warrior").count(), 0);
    assert_eq!(missing_features("goblin-warrior").count(), 1);
}

// SRD 5.2.1 p. 290: only Goblin Boss lists Redirect Attack, not Goblin Warrior.
#[test]
fn goblin_warrior_does_not_gain_boss_redirect_attack() {
    assert!(
        !SRD_521_CREATURE_FEATURES
            .iter()
            .any(|row| { row.creature == "goblin-warrior" && row.name == "Redirect Attack" })
    );
}

// SRD 5.2.1, p. 278: "Hit: 3 (1d4 + 1) Slashing damage plus 1 Necrotic damage."
#[test]
fn cultist_sickle_discloses_mixed_damage_as_missing() {
    let sickle = SRD_521_CREATURE_FEATURES
        .iter()
        .find(|row| row.creature == "cultist" && row.name == "Ritual Sickle")
        .expect("Cultist Sickle action");
    assert_eq!(sickle.kind, Action);
    assert_eq!(sickle.status, Missing);
    assert!(sickle.note.contains("1d4 + 1 Slashing"));
    assert!(sickle.note.contains("1 Necrotic"));
    assert!(!sickle.note.contains("as Slashing"));
}

#[test]
fn wolf_and_mastiff_bite_ledger_matches_the_prone_rider() {
    // SRD 5.2.1 pp. 357, 364: each Bite knocks a Medium or smaller target Prone on a hit.
    for creature in ["wolf", "mastiff"] {
        let bite = SRD_521_CREATURE_FEATURES
            .iter()
            .find(|row| row.creature == creature && row.name == "Bite")
            .expect("each creature has a Bite feature");
        assert_eq!(bite.status, Implemented);
        assert!(bite.note.contains("on a hit"));
        assert!(bite.note.contains("Medium or smaller"));
        assert!(bite.note.contains("Prone"));
    }
    let pack_tactics = SRD_521_CREATURE_FEATURES
        .iter()
        .find(|row| row.creature == "wolf" && row.name == "Pack Tactics")
        .expect("Wolf has Pack Tactics");
    assert!(pack_tactics.note.contains("non-Incapacitated ally"));
    assert!(pack_tactics.note.contains("within 5 ft of the target"));
}

/// SRD 5.2.1 p. 18 permits damage at 0 Hit Points; this table pins the game's enemy policy.
#[test]
fn unconscious_target_policy_is_explicit_for_each_creature() {
    for creature in &SRD_521_CREATURES {
        let expected = matches!(
            creature.id,
            "zombie" | "wolf" | "giant-rat" | "giant-weasel" | "mastiff"
        );
        assert_eq!(
            creature.attacks_unconscious, expected,
            "{} policy at SRD page {}",
            creature.id, creature.page
        );
    }
}

fn compiled(id: &str) -> &'static SrdCreature {
    SRD_521_CREATURES
        .iter()
        .chain([&SCENE_COMMONER])
        .find(|creature| creature.id == id)
        .expect("shop cast creature from SRD 5.2.1")
}

// SRD 5.2.1 p. 275 (Commoner: AC 10, HP 4 (1d8), all abilities 10, Club +2, 2 (1d4));
// p. 296 (Guard: AC 16, HP 11, Str 13 Dex 12 Con 12 Int 10 Wis 11 Cha 10, Spear +3, 4 (1d6+1));
// p. 322 (Scout: AC 13, HP 16, Str 11 Dex 14 Con 12 Int 11 Wis 13 Cha 11, Shortsword +4, 5
// (1d6+2)).
#[test]
fn the_shop_cast_stat_blocks_match_their_srd_pages() {
    let commoner = compiled("commoner");
    assert_eq!(commoner.page, 275);
    assert_eq!(commoner.rules.hit_points, 4);
    assert_eq!(commoner.rules.armor_class, 10);
    assert_eq!(commoner.rules.abilities, [10; 6]);
    assert_eq!(commoner.rules.attack_bonus, 2);
    assert_eq!(commoner.rules.damage_modifier, 0);
    assert_eq!(commoner.rules.weapon.damage_dice, 1);
    assert_eq!(commoner.rules.weapon.damage_sides, 4);

    let guard = compiled("guard");
    assert_eq!(guard.page, 296);
    assert_eq!(guard.rules.hit_points, 11);
    assert_eq!(guard.rules.armor_class, 16);
    assert_eq!(guard.rules.abilities, [13, 12, 12, 10, 11, 10]);
    assert_eq!(guard.rules.attack_bonus, 3);
    assert_eq!(guard.rules.damage_modifier, 1);
    assert_eq!(guard.rules.weapon.damage_sides, 6);

    let scout = compiled("scout");
    assert_eq!(scout.page, 322);
    assert_eq!(scout.rules.hit_points, 16);
    assert_eq!(scout.rules.armor_class, 13);
    assert_eq!(scout.rules.abilities, [11, 14, 12, 11, 13, 11]);
    assert_eq!(scout.rules.attack_bonus, 4);
    assert_eq!(scout.rules.damage_modifier, 2);
    assert_eq!(scout.rules.weapon.damage_sides, 6);
}

// The three shop cast stat blocks each list their actions, and the Commoner's skill
// Training is disclosed as missing rather than silently omitted.
#[test]
fn the_shop_cast_srd_feature_disclosure_covers_each_creature_used() {
    for id in ["commoner", "guard", "scout"] {
        let rows = SRD_521_CREATURE_FEATURES
            .iter()
            .filter(|row| row.creature == id)
            .collect::<Vec<_>>();
        assert!(
            rows.iter().any(|row| row.kind == Action),
            "{id} lists no action"
        );
        assert!(rows.iter().all(|row| !row.note.is_empty()));
    }
    let training = SRD_521_CREATURE_FEATURES
        .iter()
        .find(|row| row.creature == "commoner" && row.name == "Training")
        .expect("commoner Training disclosed");
    assert_eq!(training.status, Missing);
    assert_eq!(
        missing_features("commoner")
            .map(|row| row.name)
            .collect::<Vec<_>>(),
        vec!["Training"]
    );
    assert_eq!(missing_features("guard").count(), 0);
    assert_eq!(missing_features("scout").count(), 0);
}

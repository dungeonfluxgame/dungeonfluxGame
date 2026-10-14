//! Compiled single-melee creature authority, pinned to SRD 5.2.1 (CC-BY-4.0).
//! `exact` applies only to the legacy night/interior, single-opponent, single-melee-weapon
//! encounter. Tactical fights resolve every stat-block action and trait the rules engine
//! supports; `SRD_521_CREATURE_FEATURES` lists each trait and action of the 2024 stat block with
//! what is implemented, what no fight can reach yet, and what is missing and must be disclosed.
use crate::local_journey::{BanditRules, WeaponRules};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SrdCreature {
    pub id: &'static str,
    pub name: &'static str,
    pub page: u16,
    pub languages: &'static str,
    pub exact: bool,
    /// Whether the enemy policy may target a hero at 0 Hit Points when no standing hero is actionable.
    pub attacks_unconscious: bool,
    pub weapon_label: &'static str,
    pub rules: BanditRules,
}
impl SrdCreature {
    /// Whether the creature can produce speech, rather than only understand languages.
    pub fn can_speak(&self) -> bool {
        self.languages != "None" && !self.languages.contains("can't speak")
    }
}
pub const SRD_521_CREATURES: [SrdCreature; 12] = [
    // SRD 5.2.1, p. 261. Single melee attack taken as written. Light Crossbow is an unused alternative action.
    SrdCreature {
        id: "bandit",
        name: "Bandit",
        page: 261,
        languages: "Common",
        exact: true,
        attacks_unconscious: false,
        weapon_label: "scimitar",
        rules: BanditRules {
            hit_points: 11,
            armor_class: 12,
            abilities: [11, 12, 12, 10, 10, 10],
            attack_bonus: 3,
            damage_modifier: 1,
            weapon: WeaponRules {
                damage_dice: 1,
                damage_sides: 6,
            },
        },
    },
    // SRD 5.2.1, p. 278. The extra 1 necrotic damage is folded into the flat damage modifier (1 + 1 = 2) because the combat model carries one damage type.
    SrdCreature {
        id: "cultist",
        name: "Cultist",
        page: 278,
        languages: "Common",
        exact: false,
        attacks_unconscious: false,
        weapon_label: "ritual sickle",
        rules: BanditRules {
            hit_points: 9,
            armor_class: 12,
            abilities: [11, 12, 10, 10, 11, 10],
            attack_bonus: 3,
            damage_modifier: 2,
            weapon: WeaponRules {
                damage_dice: 1,
                damage_sides: 4,
            },
        },
    },
    // SRD 5.2.1, p. 296. Spear used as a melee attack, as the stat block allows. Thrown use is not offered.
    SrdCreature {
        id: "guard",
        name: "Guard",
        page: 296,
        languages: "Common",
        exact: true,
        attacks_unconscious: false,
        weapon_label: "spear",
        rules: BanditRules {
            hit_points: 11,
            armor_class: 16,
            abilities: [13, 12, 12, 10, 11, 10],
            attack_bonus: 3,
            damage_modifier: 1,
            weapon: WeaponRules {
                damage_dice: 1,
                damage_sides: 6,
            },
        },
    },
    // SRD 5.2.1 p. 290: "AC 15" and "Gear Chain Shirt, Scimitar, Shield, Shortbow";
    // "Shortbow. Ranged Attack Roll: +4, range 80/320 ft." is an action alongside Scimitar.
    // Nimble Escape and the Advantage rider have no effect in encounters without movement or Advantage.
    SrdCreature {
        id: "goblin-warrior",
        name: "Goblin Warrior",
        page: 290,
        languages: "Common, Goblin",
        exact: true,
        attacks_unconscious: false,
        weapon_label: "scimitar",
        rules: BanditRules {
            hit_points: 10,
            armor_class: 15,
            abilities: [8, 15, 10, 10, 8, 8],
            attack_bonus: 4,
            damage_modifier: 2,
            weapon: WeaponRules {
                damage_dice: 1,
                damage_sides: 6,
            },
        },
    },
    // SRD 5.2.1, p. 302. Legacy single-opponent encounter: dagger as a melee attack, no ally for Pack Tactics, night or interior so no Sunlight Sensitivity. Tactical fights implement Pack Tactics, Sunlight Sensitivity (a typed scene fact) and the thrown Dagger (20/60 ft).
    SrdCreature {
        id: "kobold-warrior",
        name: "Kobold Warrior",
        page: 302,
        languages: "Common, Draconic",
        exact: true,
        attacks_unconscious: false,
        weapon_label: "dagger",
        rules: BanditRules {
            hit_points: 7,
            armor_class: 14,
            abilities: [7, 15, 9, 8, 7, 8],
            attack_bonus: 4,
            damage_modifier: 2,
            weapon: WeaponRules {
                damage_dice: 1,
                damage_sides: 4,
            },
        },
    },
    // SRD 5.2.1, p. 325. Shortsword used as written. Bludgeoning vulnerability is not exercised (the encounter offers a slashing Greatsword only); poison immunity is unused.
    SrdCreature {
        id: "skeleton",
        name: "Skeleton",
        page: 325,
        languages: "Understands the languages it knew in life but can't speak",
        exact: true,
        attacks_unconscious: false,
        weapon_label: "shortsword",
        rules: BanditRules {
            hit_points: 13,
            armor_class: 14,
            abilities: [10, 16, 15, 6, 8, 5],
            attack_bonus: 5,
            damage_modifier: 3,
            weapon: WeaponRules {
                damage_dice: 1,
                damage_sides: 6,
            },
        },
    },
    // SRD 5.2.1, p. 343. Undead Fortitude is not modeled. The SRD Knocking Out rule (p.17) replaces 0 HP with 1 HP, which arguably never triggers the trait, but the ruling is not settled here; the owner may reclassify this entry as exact.
    SrdCreature {
        id: "zombie",
        name: "Zombie",
        page: 343,
        languages: "Understands the languages it knew in life but can't speak",
        exact: false,
        attacks_unconscious: true,
        weapon_label: "slam",
        rules: BanditRules {
            hit_points: 15,
            armor_class: 8,
            abilities: [13, 6, 16, 3, 6, 5],
            attack_bonus: 3,
            damage_modifier: 1,
            weapon: WeaponRules {
                damage_dice: 1,
                damage_sides: 8,
            },
        },
    },
    // SRD 5.2.1, p. 364. Tactical combat applies the Bite's Prone rider; this legacy single-opponent encounter has no Pack Tactics ally.
    SrdCreature {
        id: "wolf",
        name: "Wolf",
        page: 364,
        languages: "None",
        exact: false,
        attacks_unconscious: true,
        weapon_label: "bite",
        rules: BanditRules {
            hit_points: 11,
            armor_class: 12,
            abilities: [14, 15, 12, 3, 12, 6],
            attack_bonus: 4,
            damage_modifier: 2,
            weapon: WeaponRules {
                damage_dice: 1,
                damage_sides: 6,
            },
        },
    },
    // SRD 5.2.1, p. 353. Single bite taken as written. Pack Tactics needs an ally and none is present.
    SrdCreature {
        id: "giant-rat",
        name: "Giant Rat",
        page: 353,
        languages: "None",
        exact: true,
        attacks_unconscious: true,
        weapon_label: "bite",
        rules: BanditRules {
            hit_points: 7,
            armor_class: 13,
            abilities: [7, 16, 11, 2, 10, 4],
            attack_bonus: 5,
            damage_modifier: 3,
            weapon: WeaponRules {
                damage_dice: 1,
                damage_sides: 4,
            },
        },
    },
    // SRD 5.2.1, p. 355. Single bite taken as written.
    SrdCreature {
        id: "giant-weasel",
        name: "Giant Weasel",
        page: 355,
        languages: "None",
        exact: true,
        attacks_unconscious: true,
        weapon_label: "bite",
        rules: BanditRules {
            hit_points: 9,
            armor_class: 13,
            abilities: [11, 17, 10, 4, 12, 5],
            attack_bonus: 5,
            damage_modifier: 3,
            weapon: WeaponRules {
                damage_dice: 1,
                damage_sides: 4,
            },
        },
    },
    // SRD 5.2.1, p. 357. Tactical combat applies the Bite's Prone rider; the legacy single-opponent encounter does not.
    SrdCreature {
        id: "mastiff",
        name: "Mastiff",
        page: 357,
        languages: "None",
        exact: false,
        attacks_unconscious: true,
        weapon_label: "bite",
        rules: BanditRules {
            hit_points: 5,
            armor_class: 12,
            abilities: [13, 14, 12, 3, 12, 7],
            attack_bonus: 3,
            damage_modifier: 1,
            weapon: WeaponRules {
                damage_dice: 1,
                damage_sides: 6,
            },
        },
    },
    // SRD 5.2.1, p. 322. Multiattack (two attacks per action) is dropped: the combat model resolves one attack per turn. This makes the Scout weaker than written.
    SrdCreature {
        id: "scout",
        name: "Scout",
        page: 322,
        languages: "Common, Goblin",
        exact: false,
        attacks_unconscious: false,
        weapon_label: "shortsword",
        rules: BanditRules {
            hit_points: 16,
            armor_class: 13,
            abilities: [11, 14, 12, 11, 13, 11],
            attack_bonus: 4,
            damage_modifier: 2,
            weapon: WeaponRules {
                damage_dice: 1,
                damage_sides: 6,
            },
        },
    },
];

/// Where an SRD 5.2.1 stat-block feature stands in the tactical rules.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeatureStatus {
    /// Resolved by the rules engine and reachable in a fight now.
    Implemented,
    /// Supported by the rules engine, but nothing in a fight can reach it yet (for example an
    /// immunity to a damage type no fight deals) or the enemy policy never chooses it.
    NotExercised,
    /// Not modelled. A fight with this creature must say so in its disclosed rules note.
    Missing,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeatureKind {
    Trait,
    Action,
    BonusAction,
    Movement,
}
/// One trait, action, bonus action or movement mode of a 2024 stat block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreatureFeature {
    pub creature: &'static str,
    pub name: &'static str,
    pub kind: FeatureKind,
    pub status: FeatureStatus,
    pub note: &'static str,
}
const fn feature(
    creature: &'static str,
    name: &'static str,
    kind: FeatureKind,
    status: FeatureStatus,
    note: &'static str,
) -> CreatureFeature {
    CreatureFeature {
        creature,
        name,
        kind,
        status,
        note,
    }
}
use FeatureKind::{Action, BonusAction, Movement, Trait};
use FeatureStatus::{Implemented, Missing, NotExercised};
/// Every trait and action of the twelve compiled stat blocks (SRD 5.2.1 page in the creature
/// row above), checked by a test so a creature cannot gain a row-less feature silently.
pub const SRD_521_CREATURE_FEATURES: &[CreatureFeature] = &[
    feature(
        "bandit",
        "Scimitar",
        Action,
        Implemented,
        "melee, reach 5 ft",
    ),
    feature(
        "bandit",
        "Light Crossbow",
        Action,
        Implemented,
        "ranged 80/320 ft, ammunition counted",
    ),
    feature(
        "cultist",
        "Ritual Sickle",
        Action,
        Implemented,
        "melee; the 1 Necrotic damage is separate from the Slashing damage",
    ),
    feature(
        "guard",
        "Spear (melee)",
        Action,
        Implemented,
        "reach 5 ft, always 1d6+1",
    ),
    feature(
        "guard",
        "Spear (thrown)",
        Action,
        Implemented,
        "range 20/60 ft; the enemy policy throws when no melee square is reachable",
    ),
    feature(
        "goblin-warrior",
        "Scimitar",
        Action,
        Implemented,
        "melee, plus 1d4 when the attack roll had Advantage",
    ),
    feature(
        "goblin-warrior",
        "Shortbow",
        Action,
        Implemented,
        "ranged 80/320 ft, plus 1d4 with Advantage; used at stat-block AC 15 without a doff action",
    ),
    feature(
        "goblin-warrior",
        "Nimble Escape (Disengage)",
        BonusAction,
        NotExercised,
        "the rules engine resolves it; the enemy policy never chooses it",
    ),
    feature(
        "goblin-warrior",
        "Nimble Escape (Hide)",
        BonusAction,
        Missing,
        "no Hide or stealth rules in the tactical subset",
    ),
    feature(
        "kobold-warrior",
        "Pack Tactics",
        Trait,
        Implemented,
        "Advantage when an ally that is not Incapacitated is within 5 ft of the target",
    ),
    feature(
        "kobold-warrior",
        "Sunlight Sensitivity",
        Trait,
        Implemented,
        "Disadvantage on attack rolls and, through Initiative, on ability checks, when the scene is in sunlight",
    ),
    feature(
        "kobold-warrior",
        "Dagger (melee)",
        Action,
        Implemented,
        "reach 5 ft",
    ),
    feature(
        "kobold-warrior",
        "Dagger (thrown)",
        Action,
        Implemented,
        "range 20/60 ft, three daggers carried",
    ),
    feature(
        "skeleton",
        "Vulnerability: Bludgeoning",
        Trait,
        Implemented,
        "damage doubled after bonuses (SRD p17)",
    ),
    feature(
        "skeleton",
        "Immunity: Poison, Poisoned, Exhaustion",
        Trait,
        NotExercised,
        "no fight deals poison or applies these conditions",
    ),
    feature(
        "skeleton",
        "Shortsword",
        Action,
        Implemented,
        "melee, reach 5 ft",
    ),
    feature(
        "skeleton",
        "Shortbow",
        Action,
        Implemented,
        "ranged 80/320 ft",
    ),
    feature(
        "zombie",
        "Undead Fortitude",
        Trait,
        Implemented,
        "Constitution save DC 5 + damage, not against a Critical Hit",
    ),
    feature(
        "zombie",
        "Undead Fortitude (Radiant exception)",
        Trait,
        NotExercised,
        "no Radiant damage exists in a fight",
    ),
    feature(
        "zombie",
        "Immunity: Poison, Poisoned, Exhaustion",
        Trait,
        NotExercised,
        "no fight deals poison or applies these conditions",
    ),
    feature("zombie", "Slam", Action, Implemented, "melee, reach 5 ft"),
    feature(
        "wolf",
        "Pack Tactics",
        Trait,
        Implemented,
        "Advantage when a non-Incapacitated ally is within 5 ft of the target",
    ),
    feature(
        "wolf",
        "Bite",
        Action,
        Implemented,
        "melee, reach 5 ft; on a hit, a Medium or smaller target is knocked Prone",
    ),
    feature(
        "giant-rat",
        "Pack Tactics",
        Trait,
        Implemented,
        "as the Kobold's",
    ),
    feature(
        "giant-rat",
        "Bite",
        Action,
        Implemented,
        "melee, reach 5 ft",
    ),
    feature(
        "giant-rat",
        "Climb Speed 30 ft",
        Movement,
        Missing,
        "the grid has no vertical movement",
    ),
    feature(
        "giant-weasel",
        "Bite",
        Action,
        Implemented,
        "melee, reach 5 ft",
    ),
    feature(
        "giant-weasel",
        "Climb Speed 30 ft",
        Movement,
        Missing,
        "the grid has no vertical movement",
    ),
    feature(
        "mastiff",
        "Bite",
        Action,
        Implemented,
        "melee, reach 5 ft; on a hit, a Medium or smaller target is knocked Prone",
    ),
    feature(
        "scout",
        "Multiattack",
        Action,
        Implemented,
        "two attacks in any mix of Shortsword and Longbow",
    ),
    feature(
        "scout",
        "Shortsword",
        Action,
        Implemented,
        "melee, reach 5 ft",
    ),
    feature("scout", "Longbow", Action, Implemented, "ranged 150/600 ft"),
];
/// The gaps a fight against this creature must disclose.
pub fn missing_features(creature: &str) -> impl Iterator<Item = &'static CreatureFeature> + '_ {
    SRD_521_CREATURE_FEATURES
        .iter()
        .filter(move |row| row.creature == creature && row.status == Missing)
}
#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(creature("bandit").can_speak());
        assert!(!creature("skeleton").can_speak());
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
                SRD_521_CREATURES.iter().any(|c| c.id == row.creature),
                "{}",
                row.creature
            );
            assert!(!row.note.is_empty());
        }
        assert_eq!(missing_features("giant-rat").count(), 1);
        assert_eq!(missing_features("kobold-warrior").count(), 0);
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
}

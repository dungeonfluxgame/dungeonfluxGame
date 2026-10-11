//! Compiled single-melee creature authority, pinned to SRD 5.2.1 (CC-BY-4.0).
//! Exact applies only to the approved night/interior, single-opponent, slashing encounter.
use crate::local_journey::{BanditRules, WeaponRules};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SrdCreature {
    pub id: &'static str,
    pub name: &'static str,
    pub page: u16,
    pub exact: bool,
    pub weapon_label: &'static str,
    pub rules: BanditRules,
}
pub const SRD_521_CREATURES: [SrdCreature; 12] = [
    // SRD 5.2.1, p. 261. Single melee attack taken as written. Light Crossbow is an unused alternative action.
    SrdCreature {
        id: "bandit",
        name: "Bandit",
        page: 261,
        exact: true,
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
        exact: false,
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
        exact: true,
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
    // SRD 5.2.1, p. 290. Single melee attack taken as written. Nimble Escape (a movement option) and the Advantage rider have no effect because the encounter has no movement and no Advantage.
    SrdCreature {
        id: "goblin-warrior",
        name: "Goblin Warrior",
        page: 290,
        exact: true,
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
    // SRD 5.2.1, p. 302. Dagger used as a melee attack. Pack Tactics needs an ally and none is present; Sunlight Sensitivity cannot apply because every generated scene is night or interior.
    SrdCreature {
        id: "kobold-warrior",
        name: "Kobold Warrior",
        page: 302,
        exact: true,
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
        exact: true,
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
        exact: false,
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
    // SRD 5.2.1, p. 364. The Prone rider on a bite against a Medium or smaller target is not modeled. Pack Tactics needs an ally and none is present.
    SrdCreature {
        id: "wolf",
        name: "Wolf",
        page: 364,
        exact: false,
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
        exact: true,
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
        exact: true,
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
    // SRD 5.2.1, p. 357. The Prone rider on a bite against a Medium or smaller target is not modeled.
    SrdCreature {
        id: "mastiff",
        name: "Mastiff",
        page: 357,
        exact: false,
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
        exact: false,
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
#[cfg(test)]
mod tests {
    use super::*;
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
}

//! Compiled single-melee creature authority, pinned to SRD 5.2.1 (CC-BY-4.0).
//! `exact` applies only to the legacy night/interior, single-opponent, single-melee-weapon
//! encounter. Tactical fights resolve every stat-block action and trait the rules engine
//! supports; `SRD_521_CREATURE_FEATURES` lists each trait and action of the 2024 stat block with
//! what is implemented, what no fight can reach yet, and what is missing and must be disclosed.
use crate::local_journey::{BanditRules, ChallengeRating, WeaponRules};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SrdCreature {
    pub id: &'static str,
    pub name: &'static str,
    pub page: u16,
    pub challenge_rating: ChallengeRating,
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

    pub fn experience_points(&self) -> Option<u32> {
        crate::local_journey::experience_points_for_challenge_rating(self.challenge_rating)
    }
}
/// Authored scene-NPC profile used only by an explicit scene combat binding.
/// Source: official SRD v5.2.1 (CC-BY-4.0), Wizards of the Coast,
/// https://www.dndbeyond.com/srd, p. 275 (Commoner, CR 0). Commoner is kept separate from the
/// generated antagonist table because it is not a generated adventure antagonist.
pub const SCENE_COMMONER: SrdCreature = SrdCreature {
    id: "commoner",
    name: "Commoner",
    page: 275,
    challenge_rating: ChallengeRating::Zero,
    languages: "Common",
    exact: true,
    attacks_unconscious: false,
    weapon_label: "club",
    rules: BanditRules {
        hit_points: 4,
        armor_class: 10,
        abilities: [10, 10, 10, 10, 10, 10],
        attack_bonus: 2,
        damage_modifier: 0,
        weapon: WeaponRules {
            damage_dice: 1,
            damage_sides: 4,
        },
    },
};

pub const SRD_521_CREATURES: [SrdCreature; 17] = [
    // SRD 5.2.1, p. 261. Single melee attack taken as written. Light Crossbow is an unused alternative action.
    SrdCreature {
        id: "bandit",
        name: "Bandit",
        page: 261,
        challenge_rating: ChallengeRating::Eighth,
        languages: "Common, Thieves' Cant",
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
    // SRD 5.2.1, p. 278. The tactical outcome carries the 1d4+1 Slashing and
    // 1 Necrotic portions separately; no admitted target resists or is immune
    // to either type, so that interaction remains a disclosed catalog gap.
    SrdCreature {
        id: "cultist",
        name: "Cultist",
        page: 278,
        challenge_rating: ChallengeRating::Eighth,
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
    // Shop caveat: Jessa is off duty in plain clothes, but the block's Chain Shirt and Shield (AC 16)
    // are kept as written rather than inventing an unarmored variant.
    SrdCreature {
        id: "guard",
        name: "Guard",
        page: 296,
        challenge_rating: ChallengeRating::Eighth,
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
    // SRD 5.2.1 p. 290: Warrior has Nimble Escape; Redirect Attack belongs to
    // the separate Goblin Boss stat block, which is outside this compiled catalog.
    SrdCreature {
        id: "goblin-warrior",
        name: "Goblin Warrior",
        page: 290,
        challenge_rating: ChallengeRating::Quarter,
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
        challenge_rating: ChallengeRating::Eighth,
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
        challenge_rating: ChallengeRating::Quarter,
        languages: "Understands Common plus one other language but can't speak",
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
    // SRD 5.2.1, pp. 17, 343. Tactical combat applies Undead Fortitude; the legacy single-opponent encounter does not.
    SrdCreature {
        id: "zombie",
        name: "Zombie",
        page: 343,
        challenge_rating: ChallengeRating::Quarter,
        languages: "Understands Common plus one other language but can't speak",
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
        challenge_rating: ChallengeRating::Quarter,
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
    // SRD 5.2.1, p. 353. Bite is taken as written; Climb Speed 30 ft. and
    // Darkvision 60 ft. are catalogued separately from the flat-grid movement.
    SrdCreature {
        id: "giant-rat",
        name: "Giant Rat",
        page: 353,
        challenge_rating: ChallengeRating::Eighth,
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
    // SRD 5.2.1, p. 355. Bite is taken as written; Climb Speed 30 ft. and
    // Darkvision 60 ft. are catalogued separately from the flat-grid movement.
    SrdCreature {
        id: "giant-weasel",
        name: "Giant Weasel",
        page: 355,
        challenge_rating: ChallengeRating::Eighth,
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
        challenge_rating: ChallengeRating::Eighth,
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
    // SRD 5.2.1, p. 322. Tactical combat implements Multiattack; the legacy single-opponent encounter uses one attack.
    SrdCreature {
        id: "scout",
        name: "Scout",
        page: 322,
        challenge_rating: ChallengeRating::Half,
        languages: "Common plus one other language",
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
    // SRD 5.2.1 p. 312; tactical feature exclusions are enumerated below.
    SrdCreature {
        id: "ogre",
        name: "Ogre",
        page: 312,
        challenge_rating: ChallengeRating::Whole(2),
        languages: "Common, Giant",
        exact: false,
        attacks_unconscious: false,
        weapon_label: "greatclub",
        rules: BanditRules {
            hit_points: 68,
            armor_class: 11,
            abilities: [19, 8, 16, 5, 7, 7],
            attack_bonus: 6,
            damage_modifier: 4,
            weapon: WeaponRules {
                damage_dice: 2,
                damage_sides: 8,
            },
        },
    },
    // SRD 5.2.1 p. 313; tactical feature exclusions are enumerated below.
    SrdCreature {
        id: "owlbear",
        name: "Owlbear",
        page: 313,
        challenge_rating: ChallengeRating::Whole(3),
        languages: "None",
        exact: false,
        attacks_unconscious: false,
        weapon_label: "rend",
        rules: BanditRules {
            hit_points: 59,
            armor_class: 13,
            abilities: [20, 12, 17, 3, 12, 7],
            attack_bonus: 7,
            damage_modifier: 5,
            weapon: WeaponRules {
                damage_dice: 2,
                damage_sides: 8,
            },
        },
    },
    // SRD 5.2.1 p. 341; tactical feature exclusions are enumerated below.
    SrdCreature {
        id: "wight",
        name: "Wight",
        page: 341,
        challenge_rating: ChallengeRating::Whole(3),
        languages: "Common plus one other language",
        exact: false,
        attacks_unconscious: false,
        weapon_label: "necrotic sword",
        rules: BanditRules {
            hit_points: 82,
            armor_class: 14,
            abilities: [15, 14, 16, 10, 13, 15],
            attack_bonus: 4,
            damage_modifier: 2,
            weapon: WeaponRules {
                damage_dice: 1,
                damage_sides: 8,
            },
        },
    },
    // SRD 5.2.1 p. 298; tactical feature exclusions are enumerated below.
    SrdCreature {
        id: "hill-giant",
        name: "Hill Giant",
        page: 298,
        challenge_rating: ChallengeRating::Whole(5),
        languages: "Giant",
        exact: false,
        attacks_unconscious: false,
        weapon_label: "tree club",
        rules: BanditRules {
            hit_points: 105,
            armor_class: 13,
            abilities: [21, 8, 19, 5, 9, 6],
            attack_bonus: 8,
            damage_modifier: 5,
            weapon: WeaponRules {
                damage_dice: 3,
                damage_sides: 8,
            },
        },
    },
    // SRD 5.2.1 p. 333; tactical feature exclusions are enumerated below.
    SrdCreature {
        id: "troll",
        name: "Troll",
        page: 333,
        challenge_rating: ChallengeRating::Whole(5),
        languages: "Giant",
        exact: false,
        attacks_unconscious: false,
        weapon_label: "rend",
        rules: BanditRules {
            hit_points: 94,
            armor_class: 15,
            abilities: [18, 13, 20, 7, 9, 7],
            attack_bonus: 7,
            damage_modifier: 4,
            weapon: WeaponRules {
                damage_dice: 2,
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
    Reaction,
    Movement,
    Sense,
}
/// One trait, action, bonus action, reaction, movement mode or sense of a 2024 stat block.
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
use FeatureKind::{Action, BonusAction, Movement, Sense, Trait};
use FeatureStatus::{Implemented, Missing, NotExercised};
const DARKVISION_NOTE: &str = "It can see in darkness within 60 ft. in this tactical fight.";
/// Every trait, action, bonus action, reaction, movement mode and sense of the seventeen compiled
/// stat blocks (SRD 5.2.1 page in the creature row above), checked so none is omitted silently.
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
        Missing,
        "The outcome retains 1d4 + 1 Slashing and 1 Necrotic damage separately; no admitted target resists or is immune to either type.",
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
        Implemented,
        "the enemy policy spends a Bonus Action before leaving a threatened space",
    ),
    feature(
        "goblin-warrior",
        "Nimble Escape (Hide)",
        BonusAction,
        Missing,
        "no Hide or stealth rules in the tactical subset",
    ),
    // SRD 5.2.1, p. 290: "Darkvision 60 ft."
    feature(
        "goblin-warrior",
        "Darkvision 60 ft.",
        Sense,
        Implemented,
        DARKVISION_NOTE,
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
    // SRD 5.2.1, p. 302: "Darkvision 60 ft."
    feature(
        "kobold-warrior",
        "Darkvision 60 ft.",
        Sense,
        Implemented,
        DARKVISION_NOTE,
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
    // SRD 5.2.1, p. 325: "Darkvision 60 ft."
    feature(
        "skeleton",
        "Darkvision 60 ft.",
        Sense,
        Implemented,
        DARKVISION_NOTE,
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
    // SRD 5.2.1, p. 343: "Darkvision 60 ft."
    feature(
        "zombie",
        "Darkvision 60 ft.",
        Sense,
        Implemented,
        DARKVISION_NOTE,
    ),
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
    // SRD 5.2.1, p. 364: "Darkvision 60 ft."; Keen Hearing and Smell is not in this 2024 block.
    feature(
        "wolf",
        "Darkvision 60 ft.",
        Sense,
        Implemented,
        DARKVISION_NOTE,
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
    // SRD 5.2.1, p. 353: "Darkvision 60 ft."; Keen Smell is not in this 2024 block.
    feature(
        "giant-rat",
        "Darkvision 60 ft.",
        Sense,
        Implemented,
        DARKVISION_NOTE,
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
    // SRD 5.2.1, p. 355: "Darkvision 60 ft."; Keen Hearing and Smell is not in this 2024 block.
    feature(
        "giant-weasel",
        "Darkvision 60 ft.",
        Sense,
        Implemented,
        DARKVISION_NOTE,
    ),
    feature(
        "mastiff",
        "Bite",
        Action,
        Implemented,
        "melee, reach 5 ft; on a hit, a Medium or smaller target is knocked Prone",
    ),
    // SRD 5.2.1, p. 357: "Darkvision 60 ft."; Keen Hearing and Smell is not in this 2024 block.
    feature(
        "mastiff",
        "Darkvision 60 ft.",
        Sense,
        Implemented,
        DARKVISION_NOTE,
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
    // SRD 5.2.1, p. 275. Training grants Advantage on one GM-chosen skill check outside combat;
    // no tactical fight resolves skill checks, so this trait is disclosed rather than modelled.
    feature(
        "commoner",
        "Training",
        Trait,
        Missing,
        "Advantage on one GM-chosen skill check is not modelled; no tactical fight rolls skill checks.",
    ),
    feature(
        "commoner",
        "Club",
        Action,
        Implemented,
        "melee, reach 5 ft, 1d4 bludgeoning",
    ),
    feature(
        "ogre",
        "Darkvision 60 ft.",
        Sense,
        Implemented,
        "It can see in darkness within 60 ft. in this tactical fight.",
    ),
    feature(
        "owlbear",
        "Darkvision 60 ft.",
        Sense,
        Implemented,
        "It can see in darkness within 60 ft. in this tactical fight.",
    ),
    feature(
        "wight",
        "Darkvision 60 ft.",
        Sense,
        Implemented,
        "It can see in darkness within 60 ft. in this tactical fight.",
    ),
    feature(
        "troll",
        "Darkvision 60 ft.",
        Sense,
        Implemented,
        "It can see in darkness within 60 ft. in this tactical fight.",
    ),
    feature(
        "ogre",
        "Greatclub",
        Action,
        Implemented,
        "+6, reach 5 ft, 2d8 + 4 Bludgeoning; Large 10-foot space.",
    ),
    feature(
        "ogre",
        "Javelin",
        Action,
        Implemented,
        "+6, reach 5 ft or range 30/120 ft, 2d6 + 4 Piercing; three counted javelins.",
    ),
    feature(
        "owlbear",
        "Multiattack",
        Action,
        Implemented,
        "Two Rend attacks.",
    ),
    feature(
        "owlbear",
        "Rend",
        Action,
        Implemented,
        "+7, reach 5 ft, 2d8 + 5 Slashing; Large 10-foot space.",
    ),
    feature(
        "owlbear",
        "Climb Speed 40 ft.",
        Movement,
        Missing,
        "Excluded: the tactical map has no vertical path admission; flat walking remains 40 ft.",
    ),
    feature(
        "wight",
        "Multiattack",
        Action,
        Implemented,
        "Two Necrotic Sword or Necrotic Bow attacks in any combination; optional Life Drain replacement excluded separately.",
    ),
    feature(
        "wight",
        "Necrotic Sword",
        Action,
        Implemented,
        "+4, reach 5 ft, 1d8 + 2 Slashing plus 1d8 Necrotic; critical doubles both dice.",
    ),
    feature(
        "wight",
        "Necrotic Bow",
        Action,
        Implemented,
        "+4, range 150/600 ft, 1d8 + 2 Piercing plus 1d8 Necrotic; stat-block attack has no ammunition Gear entry.",
    ),
    feature(
        "wight",
        "Life Drain",
        Action,
        Missing,
        "Excluded: no hit-point-maximum reduction, 24-hour Zombie creation or control ownership contract; not offered.",
    ),
    feature(
        "wight",
        "Sunlight Sensitivity",
        Trait,
        Implemented,
        "Disadvantage on ability checks and attack rolls in the typed Sunlight scene.",
    ),
    feature(
        "wight",
        "Resistance: Necrotic",
        Trait,
        Implemented,
        "Necrotic damage halved after immunity and vulnerability.",
    ),
    feature(
        "wight",
        "Immunity: Poison, Exhaustion, Poisoned",
        Trait,
        NotExercised,
        "Poison damage immunity implemented; no admitted action applies Exhaustion or Poisoned.",
    ),
    feature(
        "hill-giant",
        "Multiattack",
        Action,
        Implemented,
        "Two Tree Club attacks; optional Trash Lob excluded separately.",
    ),
    feature(
        "hill-giant",
        "Tree Club",
        Action,
        Implemented,
        "+8, reach 10 ft, 3d8 + 5 Bludgeoning; hits knock Large or smaller targets Prone; Huge 15-foot space.",
    ),
    feature(
        "hill-giant",
        "Trash Lob",
        Action,
        Missing,
        "Excluded: Poisoned until end of the target next turn has no admitted condition handler; the entire action is not offered.",
    ),
    feature(
        "troll",
        "Multiattack",
        Action,
        Implemented,
        "Three Rend attacks.",
    ),
    feature(
        "troll",
        "Rend",
        Action,
        Implemented,
        "+7, reach 10 ft, 2d6 + 4 Slashing; Large 10-foot space.",
    ),
    feature(
        "troll",
        "Regeneration",
        Trait,
        Implemented,
        "Regains 15 HP at turn start, including from zero; ordinary weapon damage cannot kill it. Acid/Fire inhibition is unreachable: no admitted action deals those types.",
    ),
    feature(
        "troll",
        "Loathsome Limbs (4/Day)",
        Trait,
        Missing,
        "Excluded: dynamic Troll Limb admission, independent turns and Exhaustion per severed limb are not supported; slashing cannot silently create approximate limbs.",
    ),
    feature(
        "troll",
        "Charge",
        BonusAction,
        Missing,
        "Excluded: no straight-toward-visible-enemy bonus movement action contract; not replaced with Dash or free movement.",
    ),
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
    // p. 322 (Scout: AC 13, HP 16, Str 11 Dex 14 Con 12 Int 11 Wis 13 Cha 11, Shortsword +4, 5 (1d6+2)).
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
}

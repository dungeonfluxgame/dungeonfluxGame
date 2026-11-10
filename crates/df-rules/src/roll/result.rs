//! What a resolved roll carries: every die, the modifiers by label, the total, the target and the
//! outcome (SRD 5.2.1 pages 6 to 8 and 17 to 18).
//!
//! It owns the result shape one typed request resolves to. A display or a narrator reads it; it
//! decides nothing and holds no wire format of a client.
use super::abilities::{Ability, Skill};
use super::dice::{DieRole, RolledDie};
use crate::influence::RollMode;
use crate::listed::listed_enum;
use serde::{Deserialize, Serialize};

listed_enum! {
    keyed
    /// Where a modifier of a total comes from.
    #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ModifierLabel {
        /// The 10 a passive score starts from (page 22).
        Base,
        Strength,
        Dexterity,
        Constitution,
        Intelligence,
        Wisdom,
        Charisma,
        Proficiency,
        /// Doubled Proficiency Bonus (page 182).
        Expertise,
        /// The +5 of a passive score with Advantage (page 186).
        Advantage,
        /// The -5 of a passive score with Disadvantage (page 186).
        Disadvantage,
        /// The class level a feature adds, such as the Fighter level of Second Wind.
        ClassLevel,
        /// A flat number the request carries, such as a damage modifier.
        Flat,
    }
}

impl ModifierLabel {
    pub fn key(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Strength => "strength",
            Self::Dexterity => "dexterity",
            Self::Constitution => "constitution",
            Self::Intelligence => "intelligence",
            Self::Wisdom => "wisdom",
            Self::Charisma => "charisma",
            Self::Proficiency => "proficiency",
            Self::Expertise => "expertise",
            Self::Advantage => "advantage",
            Self::Disadvantage => "disadvantage",
            Self::ClassLevel => "class_level",
            Self::Flat => "flat",
        }
    }

    pub const fn ability(ability: Ability) -> Self {
        match ability {
            Ability::Strength => Self::Strength,
            Ability::Dexterity => Self::Dexterity,
            Ability::Constitution => Self::Constitution,
            Ability::Intelligence => Self::Intelligence,
            Ability::Wisdom => Self::Wisdom,
            Ability::Charisma => Self::Charisma,
        }
    }
}

/// One labelled number added to the dice.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Modifier {
    pub label: ModifierLabel,
    pub value: i16,
}

listed_enum! {
    keyed
    /// What a target number is.
    #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum TargetLabel {
        /// A Difficulty Class (page 6), also a save DC.
        DifficultyClass,
        /// An Armor Class (page 7).
        ArmorClass,
        /// The DC to end a grapple, 8 plus Strength modifier and Proficiency Bonus (page 190).
        EscapeDc,
        /// The total of a hider's Dexterity (Stealth) check, the DC to find them (page 183).
        StealthTotal,
    }
}

impl TargetLabel {
    pub fn key(self) -> &'static str {
        match self {
            Self::DifficultyClass => "difficulty_class",
            Self::ArmorClass => "armor_class",
            Self::EscapeDc => "escape_dc",
            Self::StealthTotal => "stealth_total",
        }
    }
}

/// The number a D20 Test total must equal or exceed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Target {
    pub label: TargetLabel,
    pub value: i16,
}

listed_enum! {
    keyed
    /// A rolled face only where the source gives a natural 1 or natural 20 an effect for that
    /// test: attack rolls (page 7) and Death Saving Throws (page 18).
    #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Natural {
        One,
        Twenty,
        None,
    }
}

impl Natural {
    pub fn key(self) -> &'static str {
        match self {
            Self::One => "one",
            Self::Twenty => "twenty",
            Self::None => "none",
        }
    }
}

listed_enum! {
    keyed
    /// How a roll came out.
    #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum RollOutcome {
        Success,
        Failure,
        Hit,
        Miss,
        CriticalHit,
        /// A Death Saving Throw of 10 or more (page 17).
        DeathSaveSuccess,
        /// A Death Saving Throw below 10, one failure (page 17).
        DeathSaveFailure,
        /// A natural 1 on a Death Saving Throw, two failures (page 18).
        DeathSaveTwoFailures,
        /// A natural 20 on a Death Saving Throw, 1 Hit Point regained (page 18).
        DeathSaveRevived,
        /// A roll with no target: initiative, damage, healing, a hit die, a bonus die, a table.
        Rolled,
        /// A passive score, which is not rolled (page 22).
        Score,
    }
}

impl RollOutcome {
    pub fn key(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Hit => "hit",
            Self::Miss => "miss",
            Self::CriticalHit => "critical_hit",
            Self::DeathSaveSuccess => "death_save_success",
            Self::DeathSaveFailure => "death_save_failure",
            Self::DeathSaveTwoFailures => "death_save_two_failures",
            Self::DeathSaveRevived => "death_save_revived",
            Self::Rolled => "rolled",
            Self::Score => "score",
        }
    }
}

/// What a roll tested, enough to title it and to find its source clause.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RollSubject {
    AbilityCheck(Ability),
    SkillCheck(Skill),
    SavingThrow(Ability),
    Attack(AttackKind),
    Initiative,
    DeathSave,
    PassivePerception,
    Dice(DieRole),
}

listed_enum! {
    keyed
    /// Melee or ranged (Attack Roll Abilities table, page 7).
    #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum AttackKind {
        Melee,
        Ranged,
    }
}

impl AttackKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Melee => "melee",
            Self::Ranged => "ranged",
        }
    }
}

impl RollSubject {
    /// The pinned clause of this kind of roll.
    pub const fn source(self) -> &'static str {
        match self {
            Self::AbilityCheck(_) => "SRD-5.2.1:page-6:D20-Tests:Ability-Checks",
            Self::SkillCheck(_) => "SRD-5.2.1:page-9:Skills",
            Self::SavingThrow(_) => "SRD-5.2.1:page-7:D20-Tests:Saving-Throws",
            Self::Attack(_) => "SRD-5.2.1:page-7:D20-Tests:Attack-Rolls",
            Self::Initiative => "SRD-5.2.1:page-13:Combat:Initiative",
            Self::DeathSave => "SRD-5.2.1:page-17:Damage-and-Healing:Death-Saving-Throws",
            Self::PassivePerception => "SRD-5.2.1:page-186:Passive-Perception",
            Self::Dice(DieRole::Damage) => "SRD-5.2.1:page-16:Damage-and-Healing:Critical-Hits",
            Self::Dice(DieRole::Healing) => "SRD-5.2.1:page-17:Damage-and-Healing:Healing",
            Self::Dice(DieRole::HitDie) => "SRD-5.2.1:page-183:Hit-Point-Dice",
            Self::Dice(DieRole::Table) => "SRD-5.2.1:page-26:Trinkets:1d100",
            Self::Dice(DieRole::Bonus | DieRole::Test) => {
                "SRD-5.2.1:page-106:Spells:Combining-Spell-Effects:Bonus-Dice"
            }
        }
    }
}

/// A resolved roll: every die, the modifiers, the total and what it meant.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RollResult {
    pub subject: RollSubject,
    pub mode: RollMode,
    pub dice: Vec<RolledDie>,
    pub modifiers: Vec<Modifier>,
    pub total: i16,
    pub target: Option<Target>,
    pub outcome: RollOutcome,
    pub natural: Natural,
    /// The die Heroic Inspiration replaced (page 8); `dice` holds the replacement.
    pub inspiration_reroll: Option<RolledDie>,
}

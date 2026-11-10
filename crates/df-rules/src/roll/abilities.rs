//! The six abilities and eighteen skills a D20 Test names (SRD 5.2.1 pages 5, 6 and 9).
//!
//! It owns the closed lists and the ability each skill uses (Skills table, page 9). It holds no
//! scores and decides no proficiency; the caller supplies the numbers.
use crate::listed::listed_enum;
use serde::{Deserialize, Serialize};

listed_enum! {
    keyed
    /// An ability, in the order of a score array (Ability Descriptions table, page 5).
    #[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Ability {
        Strength,
        Dexterity,
        Constitution,
        Intelligence,
        Wisdom,
        Charisma,
    }
}

impl Ability {
    pub fn key(self) -> &'static str {
        match self {
            Self::Strength => "strength",
            Self::Dexterity => "dexterity",
            Self::Constitution => "constitution",
            Self::Intelligence => "intelligence",
            Self::Wisdom => "wisdom",
            Self::Charisma => "charisma",
        }
    }

    /// The place of this ability in a score array; always below six.
    pub const fn index(self) -> usize {
        match self {
            Self::Strength => 0,
            Self::Dexterity => 1,
            Self::Constitution => 2,
            Self::Intelligence => 3,
            Self::Wisdom => 4,
            Self::Charisma => 5,
        }
    }
}

listed_enum! {
    keyed
    /// A skill of the Skills table (page 9). The key is snake_case (`animal_handling`).
    #[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Skill {
        Acrobatics,
        AnimalHandling,
        Arcana,
        Athletics,
        Deception,
        History,
        Insight,
        Intimidation,
        Investigation,
        Medicine,
        Nature,
        Perception,
        Performance,
        Persuasion,
        Religion,
        SleightOfHand,
        Stealth,
        Survival,
    }
}

impl Skill {
    pub fn key(self) -> &'static str {
        match self {
            Self::Acrobatics => "acrobatics",
            Self::AnimalHandling => "animal_handling",
            Self::Arcana => "arcana",
            Self::Athletics => "athletics",
            Self::Deception => "deception",
            Self::History => "history",
            Self::Insight => "insight",
            Self::Intimidation => "intimidation",
            Self::Investigation => "investigation",
            Self::Medicine => "medicine",
            Self::Nature => "nature",
            Self::Perception => "perception",
            Self::Performance => "performance",
            Self::Persuasion => "persuasion",
            Self::Religion => "religion",
            Self::SleightOfHand => "sleight_of_hand",
            Self::Stealth => "stealth",
            Self::Survival => "survival",
        }
    }

    /// The ability a check of this skill adds (Skills table, page 9).
    pub fn ability(self) -> Ability {
        match self {
            Self::Athletics => Ability::Strength,
            Self::Acrobatics | Self::SleightOfHand | Self::Stealth => Ability::Dexterity,
            Self::Arcana | Self::History | Self::Investigation | Self::Nature | Self::Religion => {
                Ability::Intelligence
            }
            Self::AnimalHandling
            | Self::Insight
            | Self::Medicine
            | Self::Perception
            | Self::Survival => Ability::Wisdom,
            Self::Deception | Self::Intimidation | Self::Performance | Self::Persuasion => {
                Ability::Charisma
            }
        }
    }
}

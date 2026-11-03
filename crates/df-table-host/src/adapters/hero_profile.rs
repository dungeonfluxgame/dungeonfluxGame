//! What the rules adapter knows about a hero: ability scores, proficiency bonus and skills (SRD
//! 5.2.1 pages 5, 6, 8 and 9).
//!
//! It owns the closed lists of abilities and skills with the ability each skill uses, the validated
//! numbers a check needs, and the roster that says which hero a seat plays. It never rolls, never
//! decides a difficulty and never invents a hero: a seat with no profile is simply absent, and the
//! adapter declines for it. It fails with `AdapterError` when a score or bonus is outside the range
//! `df-rules` accepts, so a profile that exists can always be turned into a check.

use std::collections::{BTreeMap, BTreeSet};

use df_rules::local_journey::ability_modifier;
use df_table::identity::SeatId;
use serde::{Deserialize, Serialize};

use super::error::AdapterError;

/// The lowest ability score of a creature (SRD 5.2.1 page 5, "Ability Scores").
pub const MINIMUM_ABILITY_SCORE: u8 = 1;
/// The highest ability score of any creature (SRD 5.2.1 page 5).
pub const MAXIMUM_ABILITY_SCORE: u8 = 30;
/// The lowest proficiency bonus `df-rules` accepts in a check (SRD 5.2.1 page 8, levels up to 4).
pub const MINIMUM_PROFICIENCY_BONUS: u8 = 2;
/// The highest proficiency bonus `df-rules` accepts in a check (SRD 5.2.1 page 8, levels 17 to 20).
pub const MAXIMUM_PROFICIENCY_BONUS: u8 = 6;

/// The six abilities, in the order of a score array (`LegalCharacter::abilities` in `df-rules`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ability {
    Strength,
    Dexterity,
    Constitution,
    Intelligence,
    Wisdom,
    Charisma,
}

impl Ability {
    pub const ALL: [Ability; 6] = [
        Self::Strength,
        Self::Dexterity,
        Self::Constitution,
        Self::Intelligence,
        Self::Wisdom,
        Self::Charisma,
    ];

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

    /// Always below six, the length of every array indexed by an ability.
    fn index(self) -> usize {
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

/// The eighteen skills of the SRD 5.2.1 Skills table (page 9), with the ability each most often
/// uses. The key is the kebab-case name `df-rules` uses for a skill (`animal-handling`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
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

impl Skill {
    pub const ALL: [Skill; 18] = [
        Self::Acrobatics,
        Self::AnimalHandling,
        Self::Arcana,
        Self::Athletics,
        Self::Deception,
        Self::History,
        Self::Insight,
        Self::Intimidation,
        Self::Investigation,
        Self::Medicine,
        Self::Nature,
        Self::Perception,
        Self::Performance,
        Self::Persuasion,
        Self::Religion,
        Self::SleightOfHand,
        Self::Stealth,
        Self::Survival,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Acrobatics => "acrobatics",
            Self::AnimalHandling => "animal-handling",
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
            Self::SleightOfHand => "sleight-of-hand",
            Self::Stealth => "stealth",
            Self::Survival => "survival",
        }
    }

    /// The skill a key names, at the parse boundary of a command.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|skill| skill.key() == key)
    }

    /// The ability a check of this skill adds (SRD 5.2.1 page 9, Skills table).
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

/// The ability modifiers of six validated scores, in the order of `Ability::ALL`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AbilityModifiers([i8; 6]);

impl AbilityModifiers {
    fn from_scores(scores: [u8; 6]) -> Result<Self, AdapterError> {
        let mut modifiers = [0_i8; 6];
        for ability in Ability::ALL {
            let score = scores[ability.index()];
            let out_of_range = AdapterError::AbilityScoreOutOfRange { ability, score };
            if !(MINIMUM_ABILITY_SCORE..=MAXIMUM_ABILITY_SCORE).contains(&score) {
                return Err(out_of_range);
            }
            modifiers[ability.index()] =
                i8::try_from(ability_modifier(score)).map_err(|_| out_of_range)?;
        }
        Ok(Self(modifiers))
    }

    fn of(self, ability: Ability) -> i8 {
        self.0[ability.index()]
    }
}

/// What a skill check adds to the die: the ability modifier and the proficiency bonus, which is
/// zero when the hero lacks the skill (`df-rules` reads zero as "no proficiency").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillBonus {
    pub ability_modifier: i8,
    pub proficiency_bonus: u8,
}

/// One hero as a check sees them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeroProfile {
    modifiers: AbilityModifiers,
    proficiency_bonus: u8,
    proficient_skills: BTreeSet<Skill>,
}

impl HeroProfile {
    /// `scores` are strength, dexterity, constitution, intelligence, wisdom and charisma.
    pub fn new(
        scores: [u8; 6],
        proficiency_bonus: u8,
        proficient_skills: impl IntoIterator<Item = Skill>,
    ) -> Result<Self, AdapterError> {
        if !(MINIMUM_PROFICIENCY_BONUS..=MAXIMUM_PROFICIENCY_BONUS).contains(&proficiency_bonus) {
            return Err(AdapterError::ProficiencyBonusOutOfRange {
                bonus: proficiency_bonus,
            });
        }
        Ok(Self {
            modifiers: AbilityModifiers::from_scores(scores)?,
            proficiency_bonus,
            proficient_skills: proficient_skills.into_iter().collect(),
        })
    }

    pub fn skill_bonus(&self, skill: Skill) -> SkillBonus {
        SkillBonus {
            ability_modifier: self.modifiers.of(skill.ability()),
            proficiency_bonus: if self.proficient_skills.contains(&skill) {
                self.proficiency_bonus
            } else {
                0
            },
        }
    }
}

/// Which hero each seat plays. A seat holds at most one hero.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeatRoster {
    heroes: BTreeMap<SeatId, HeroProfile>,
}

impl SeatRoster {
    pub fn with_hero(mut self, seat: SeatId, hero: HeroProfile) -> Result<Self, AdapterError> {
        if self.heroes.insert(seat, hero).is_some() {
            return Err(AdapterError::SeatAlreadyHasHero { seat });
        }
        Ok(self)
    }

    pub fn hero(&self, seat: SeatId) -> Option<&HeroProfile> {
        self.heroes.get(&seat)
    }
}

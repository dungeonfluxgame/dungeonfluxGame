//! What the rules adapter knows about a hero: ability scores, proficiency bonus and skills (SRD
//! 5.2.1 pages 5, 6, 8 and 9).
//!
//! It owns the validated numbers a check needs and the roster that says which hero a seat plays;
//! the closed lists of abilities and skills are `df-rules`', re-exported here. It never rolls,
//! never decides a difficulty and never invents a hero: a seat with no profile is simply absent,
//! and the adapter declines for it. It fails with `AdapterError` when a score or bonus is outside
//! the range `df-rules` accepts, so a profile that exists can always be turned into a check.

use std::collections::{BTreeMap, BTreeSet};

use df_rules::local_journey::ability_modifier;
pub use df_rules::roll::{Ability, Skill};
use df_table::identity::SeatId;

use super::error::AdapterError;

/// The lowest ability score of a creature (SRD 5.2.1 page 5, "Ability Scores").
pub const MINIMUM_ABILITY_SCORE: u8 = 1;
/// The highest ability score of any creature (SRD 5.2.1 page 5).
pub const MAXIMUM_ABILITY_SCORE: u8 = 30;
/// The lowest proficiency bonus `df-rules` accepts in a check (SRD 5.2.1 page 8, levels up to 4).
pub const MINIMUM_PROFICIENCY_BONUS: u8 = 2;
/// The highest proficiency bonus `df-rules` accepts in a check (SRD 5.2.1 page 8, levels 17 to 20).
pub const MAXIMUM_PROFICIENCY_BONUS: u8 = 6;

/// The ability modifiers of six validated scores, in the order of `Ability::ALL`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AbilityModifiers([i8; 6]);

impl AbilityModifiers {
    fn from_scores(scores: [u8; 6]) -> Result<Self, AdapterError> {
        let mut modifiers = [0_i8; 6];
        for &ability in Ability::ALL {
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

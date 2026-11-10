//! Every roll the table makes, as one typed request resolved to one typed result (SRD 5.2.1 pages
//! 6 to 9, 13, 16 to 18, 22, 26 to 27, 49, 183 and 186).
//!
//! It owns the closed kinds of roll (ability and skill checks, saving throws, attack rolls,
//! initiative, Death Saving Throws, passive Perception, and `NdS+M` dice for damage, healing, hit
//! dice, bonus dice and tables), the draw plan each needs and the arithmetic of its result. Every
//! die is in the result with its sides, face, role and whether it counted, beside the modifiers by
//! label, the total, the target and the outcome.
//!
//! It never rolls, reads a clock or does I/O. The caller asks `draw_plan` for the dice, draws each
//! from its dice port, and passes the faces to `resolve`; the same faces always give the same
//! result. It never decides whether a roll is called for, which Advantage or Disadvantage applies
//! or what the target is: those arrive in the request. It fails with a `RollError` that names the
//! invalid input or the draw that is missing, unfit or unused.
//!
//! Source gaps, each visible rather than approximated: the SRD has no opposed-check rule (see
//! `d20`), no dice-notation or percentile-dice section (see `dice`), and no generic bonus-die
//! rule. Heroic Inspiration is modelled as the SRD words it: the player replaces one die after
//! seeing the roll (pages 8 and 183).
mod abilities;
mod d20;
mod dice;
mod expression;
mod result;
#[cfg(test)]
mod roll_golden_tests;
#[cfg(test)]
mod roll_tests;

pub use crate::influence::RollMode;
pub use abilities::{Ability, Skill};
pub use d20::{
    D20Request, D20Test, HeroicInspiration, PassivePerceptionRequest, Proficiency,
    unarmed_strike_dc,
};
pub use dice::{DiceExpression, DiceExpressionError, DiePlace, DieRole, DieSize, RolledDie};
pub use expression::DiceRequest;
pub use result::{
    AttackKind, Modifier, ModifierLabel, Natural, RollOutcome, RollResult, RollSubject, Target,
    TargetLabel,
};

use serde::{Deserialize, Serialize};
use std::fmt;

/// Source of Advantage and Disadvantage on a d20 roll: two d20s, the higher or the lower counts.
pub const ADVANTAGE_SOURCE: &str = "SRD-5.2.1:page-8:D20-Tests:Advantage-Disadvantage";
/// Source of Heroic Inspiration: expend it to reroll any die and use the new roll.
pub const HEROIC_INSPIRATION_SOURCE: &str = "SRD-5.2.1:page-8:Heroic-Inspiration";
/// Source of the Champion's Improved Critical (Critical Hit on 19 or 20).
pub const IMPROVED_CRITICAL_SOURCE: &str = "SRD-5.2.1:page-49:Champion:Improved-Critical";
/// Source of a percentile table roll where "00" reads as 100.
pub const PERCENTILE_SOURCE: &str = "SRD-5.2.1:page-27:Trinkets:1d100";

/// One roll to resolve.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "roll", rename_all = "snake_case")]
pub enum RollRequest {
    D20(D20Request),
    PassivePerception(PassivePerceptionRequest),
    Dice(DiceRequest),
}

impl RollRequest {
    /// The dice to draw, in order, and the sizes they must be. A percentile die is two d10, tens
    /// then ones.
    pub fn draw_plan(&self) -> Result<Vec<DieSize>, RollError> {
        match self {
            Self::D20(request) => d20::d20_plan(request),
            Self::PassivePerception(request) => d20::resolve_passive(request).map(|_| Vec::new()),
            Self::Dice(request) => expression::dice_plan(request),
        }
    }

    /// Resolves the request from the faces drawn for its plan, one per die in plan order.
    pub fn resolve(&self, faces: &[u16]) -> Result<RollResult, RollError> {
        let mut reader = Faces::new(faces);
        let result = match self {
            Self::D20(request) => d20::resolve_d20(request, &mut reader)?,
            Self::PassivePerception(request) => d20::resolve_passive(request)?,
            Self::Dice(request) => expression::resolve_dice(request, &mut reader)?,
        };
        reader.finish()?;
        Ok(result)
    }
}

/// Supplied faces, read in order.
struct Faces<'a> {
    supplied: &'a [u16],
    used: usize,
}

impl<'a> Faces<'a> {
    fn new(supplied: &'a [u16]) -> Self {
        Self { supplied, used: 0 }
    }

    fn take(&mut self, size: DieSize) -> Result<u16, RollError> {
        let sides = size.sides();
        let face = *self
            .supplied
            .get(self.used)
            .ok_or(RollError::MissingDraw { sides })?;
        if !(1..=sides).contains(&face) {
            return Err(RollError::FaceOutOfRange { sides, face });
        }
        self.used += 1;
        Ok(face)
    }

    fn finish(self) -> Result<(), RollError> {
        if self.used == self.supplied.len() {
            Ok(())
        } else {
            Err(RollError::UnusedDraws)
        }
    }
}

/// Why a request could not be planned or resolved.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RollError {
    /// The ability modifier is outside -5 to 10 (page 5).
    InvalidAbilityModifier,
    /// The Proficiency Bonus is outside 2 to 9 (page 8).
    InvalidProficiencyBonus,
    /// A target number below 1 (a Stealth total may be any number).
    InvalidTarget,
    /// A check, save or attack without a target.
    MissingTarget,
    /// A target on initiative or a Death Saving Throw, whose target is fixed.
    UnexpectedTarget,
    /// An Armor Class on a check or save, or another target on an attack roll.
    MismatchedTarget,
    /// A modifier or Proficiency Bonus on a Death Saving Throw, which has none (page 17).
    UnexpectedModifiers,
    /// A critical range outside 2 to 20.
    InvalidCriticalRange,
    /// A dice role of `Test`, which only a D20 Test has.
    InvalidRole,
    /// A critical roll of dice that are not damage dice.
    CriticalNeedsDamage,
    /// A dice count of 0 or above the maximum.
    InvalidDiceCount,
    /// A flat modifier beyond the maximum.
    InvalidDiceModifier,
    /// Heroic Inspiration replaces a die that is not in the roll.
    InvalidReroll,
    /// The faces ran out: this die has no face.
    MissingDraw { sides: u16 },
    /// A face that is not on a die of that many sides.
    FaceOutOfRange { sides: u16, face: u16 },
    /// More faces than the plan has dice.
    UnusedDraws,
}

impl RollError {
    pub fn key(self) -> &'static str {
        match self {
            Self::InvalidAbilityModifier => "invalid_ability_modifier",
            Self::InvalidProficiencyBonus => "invalid_proficiency_bonus",
            Self::InvalidTarget => "invalid_target",
            Self::MissingTarget => "missing_target",
            Self::UnexpectedTarget => "unexpected_target",
            Self::MismatchedTarget => "mismatched_target",
            Self::UnexpectedModifiers => "unexpected_modifiers",
            Self::InvalidCriticalRange => "invalid_critical_range",
            Self::InvalidRole => "invalid_role",
            Self::CriticalNeedsDamage => "critical_needs_damage",
            Self::InvalidDiceCount => "invalid_dice_count",
            Self::InvalidDiceModifier => "invalid_dice_modifier",
            Self::InvalidReroll => "invalid_reroll",
            Self::MissingDraw { .. } => "missing_draw",
            Self::FaceOutOfRange { .. } => "face_out_of_range",
            Self::UnusedDraws => "unused_draws",
        }
    }
}

impl fmt::Display for RollError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAbilityModifier => {
                formatter.write_str("the ability modifier is outside -5 to 10")
            }
            Self::InvalidProficiencyBonus => {
                formatter.write_str("the Proficiency Bonus is outside 2 to 9")
            }
            Self::InvalidTarget => formatter.write_str("the target number is below 1"),
            Self::MissingTarget => formatter.write_str("this roll needs a target number"),
            Self::UnexpectedTarget => {
                formatter.write_str("this roll has a fixed target and takes none")
            }
            Self::MismatchedTarget => {
                formatter.write_str("an attack roll needs an Armor Class and a check or save a DC")
            }
            Self::UnexpectedModifiers => {
                formatter.write_str("a Death Saving Throw adds no modifier or proficiency")
            }
            Self::InvalidCriticalRange => {
                formatter.write_str("the critical range starts between 2 and 20")
            }
            Self::InvalidRole => formatter.write_str("a dice roll cannot have the test role"),
            Self::CriticalNeedsDamage => {
                formatter.write_str("only damage dice can be rolled for a Critical Hit")
            }
            Self::InvalidDiceCount => formatter.write_str("the dice count is outside 1 to 100"),
            Self::InvalidDiceModifier => {
                formatter.write_str("the flat modifier is outside -1000 to 1000")
            }
            Self::InvalidReroll => {
                formatter.write_str("Heroic Inspiration names a die that is not in the roll")
            }
            Self::MissingDraw { sides } => write!(formatter, "a d{sides} face is missing"),
            Self::FaceOutOfRange { sides, face } => {
                write!(formatter, "{face} is not a face of a d{sides}")
            }
            Self::UnusedDraws => formatter.write_str("more faces were supplied than the roll uses"),
        }
    }
}

impl std::error::Error for RollError {}

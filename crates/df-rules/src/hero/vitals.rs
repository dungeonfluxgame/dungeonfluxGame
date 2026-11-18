//! A hero's live vitals: current and temporary Hit Points, Hit Point Dice left, the uses of the
//! limited features in play and the spell slots left (SRD 5.2.1 pp. 17-18, 23, 47, 78, 104,
//! 186-187).
//!
//! Owns `HeroVitals` (made whole from a derived sheet) and `HeroVitals::apply`, which checks one
//! `VitalsChange` and answers the vitals after it: damage takes Temporary Hit Points first and
//! leaves Hit Points at 0 at the least; healing stops at the maximum; Temporary Hit Points do not
//! add up, the higher stands; a Hit Point Die spent heals its face plus the Constitution modifier
//! (at least 1); a Short Rest gives back one Second Wind use; a spell slot spent is gone until a
//! Long Rest (p. 104) or Arcane Recovery (p. 78); a Long Rest gives back every Hit Point, every
//! Hit Point Die, every use and every slot, and ends Temporary Hit Points. Never does: roll a
//! die (a face arrives in the change), track death saves or conditions, or heal by a feature's
//! own dice (a feature use and its healing are two changes). Fails with a typed `VitalsError`.
use std::fmt;

use serde::{Deserialize, Serialize};

use super::sheet::HeroSheet;
use crate::roll::DieSize;

/// The feature id of the Fighter's Second Wind.
pub const SECOND_WIND: &str = "second_wind";
/// Second Wind uses at Fighter level 1 (Fighter Features table, SRD p. 47).
const SECOND_WIND_USES_LEVEL_ONE: u8 = 2;
/// The feature id of the Wizard's Arcane Recovery: once, then not again until a Long Rest (p. 78).
pub const ARCANE_RECOVERY: &str = "arcane_recovery";
/// Arcane Recovery recovers no slot of this level or higher (SRD p. 78).
const ARCANE_RECOVERY_LEVEL_LIMIT: u8 = 6;

/// The spell slots of one level: how many are left and how many at most.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpellSlots {
    pub level: u8,
    pub left: u8,
    pub maximum: u8,
}

/// One limited feature: its id, its uses left and its uses at most.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureUses {
    pub feature: String,
    pub left: u8,
    pub maximum: u8,
}

/// A hero's live vitals.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HeroVitals {
    pub max_hp: u32,
    pub current_hp: u32,
    pub temp_hp: u32,
    pub hit_die: DieSize,
    pub hit_dice_max: u8,
    pub hit_dice_left: u8,
    /// The Constitution modifier a Hit Point Die adds.
    pub constitution: i8,
    pub features: Vec<FeatureUses>,
    /// The spell slots by level; empty for a hero who casts no spells with slots.
    #[serde(default)]
    pub spell_slots: Vec<SpellSlots>,
    /// The combined slot levels Arcane Recovery may recover: half the Wizard level, rounded up
    /// (SRD p. 78); 0 for a hero without it.
    #[serde(default)]
    pub recovery_levels: u8,
}

/// One change to a hero's vitals.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "change", rename_all = "snake_case", deny_unknown_fields)]
pub enum VitalsChange {
    Damage {
        amount: u32,
    },
    Heal {
        amount: u32,
    },
    /// Temporary Hit Points granted; the higher of these and those held stands.
    GainTemporary {
        amount: u32,
    },
    /// One Hit Point Die spent (on a Short Rest), with the face the host rolled for it.
    SpendHitDie {
        face: u16,
    },
    /// One use of a limited feature (`SECOND_WIND`).
    UseFeature {
        feature: String,
    },
    /// One spell slot of `level` spent on a spell.
    SpendSpellSlot {
        level: u8,
    },
    /// Arcane Recovery, on finishing a Short Rest: one expended slot back for each level listed.
    ArcaneRecovery {
        levels: Vec<u8>,
    },
    ShortRest,
    LongRest,
}

impl VitalsChange {
    /// The change's key, as the log names it.
    pub fn key(&self) -> &'static str {
        match self {
            Self::Damage { .. } => "damage",
            Self::Heal { .. } => "heal",
            Self::GainTemporary { .. } => "gain_temporary",
            Self::SpendHitDie { .. } => "spend_hit_die",
            Self::UseFeature { .. } => "use_feature",
            Self::SpendSpellSlot { .. } => "spend_spell_slot",
            Self::ArcaneRecovery { .. } => "arcane_recovery",
            Self::ShortRest => "short_rest",
            Self::LongRest => "long_rest",
        }
    }
}

/// Why a change was refused.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum VitalsError {
    /// A damage, healing or grant of 0.
    ZeroAmount,
    NoHitDiceLeft,
    /// The face is not one of the hero's Hit Point Die.
    InvalidFace {
        face: u16,
        sides: u16,
    },
    /// The hero has no such limited feature.
    UnknownFeature {
        feature: String,
    },
    NoUsesLeft {
        feature: String,
    },
    /// The hero has no spell slot of the level left (or none of that level at all).
    NoSpellSlot {
        level: u8,
    },
    /// Arcane Recovery was asked for no slot.
    NothingToRecover,
    /// Arcane Recovery recovers no slot of level 6 or higher.
    RecoveryLevelTooHigh {
        level: u8,
    },
    /// The slots asked for add up to more levels than Arcane Recovery allows.
    RecoveryOverBudget {
        levels: u16,
        budget: u8,
    },
    /// No slot of the level is expended, so none can be recovered.
    SlotNotExpended {
        level: u8,
    },
}

impl VitalsError {
    pub fn key(&self) -> &'static str {
        match self {
            Self::ZeroAmount => "zero_amount",
            Self::NoHitDiceLeft => "no_hit_dice_left",
            Self::InvalidFace { .. } => "invalid_face",
            Self::UnknownFeature { .. } => "unknown_feature",
            Self::NoUsesLeft { .. } => "no_uses_left",
            Self::NoSpellSlot { .. } => "no_spell_slot",
            Self::NothingToRecover => "nothing_to_recover",
            Self::RecoveryLevelTooHigh { .. } => "recovery_level_too_high",
            Self::RecoveryOverBudget { .. } => "recovery_over_budget",
            Self::SlotNotExpended { .. } => "slot_not_expended",
        }
    }
}

impl fmt::Display for VitalsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroAmount => formatter.write_str("the amount is 0"),
            Self::NoHitDiceLeft => formatter.write_str("no Hit Point Die is left to spend"),
            Self::InvalidFace { face, sides } => {
                write!(formatter, "{face} is not a face of a d{sides}")
            }
            Self::UnknownFeature { feature } => {
                write!(formatter, "the hero has no limited feature {feature}")
            }
            Self::NoUsesLeft { feature } => write!(formatter, "{feature} has no use left"),
            Self::NoSpellSlot { level } => write!(formatter, "no level {level} spell slot is left"),
            Self::NothingToRecover => formatter.write_str("no spell slot was named to recover"),
            Self::RecoveryLevelTooHigh { level } => write!(
                formatter,
                "Arcane Recovery recovers no slot of level {level} (level 6 or higher)"
            ),
            Self::RecoveryOverBudget { levels, budget } => write!(
                formatter,
                "the slots add up to {levels} levels; Arcane Recovery recovers {budget}"
            ),
            Self::SlotNotExpended { level } => {
                write!(formatter, "no level {level} spell slot is expended")
            }
        }
    }
}

impl std::error::Error for VitalsError {}

fn nonzero(amount: u32) -> Result<u32, VitalsError> {
    if amount == 0 {
        return Err(VitalsError::ZeroAmount);
    }
    Ok(amount)
}

impl HeroVitals {
    /// The vitals of a hero fresh from its sheet: every Hit Point, every Hit Point Die (one per
    /// level, p. 23) and every use of its limited features in play.
    pub fn fresh(sheet: &HeroSheet) -> Self {
        let mut features = Vec::new();
        if sheet.class_features.contains_key(SECOND_WIND) {
            features.push(FeatureUses {
                feature: SECOND_WIND.to_owned(),
                left: SECOND_WIND_USES_LEVEL_ONE,
                maximum: SECOND_WIND_USES_LEVEL_ONE,
            });
        }
        let has_recovery = sheet.class_features.contains_key(ARCANE_RECOVERY);
        if has_recovery {
            features.push(FeatureUses {
                feature: ARCANE_RECOVERY.to_owned(),
                left: 1,
                maximum: 1,
            });
        }
        let spell_slots = sheet
            .spellcasting
            .iter()
            .flat_map(|casting| casting.slots.iter())
            .map(|slots| SpellSlots {
                level: slots.level,
                left: slots.count,
                maximum: slots.count,
            })
            .collect();
        let hit_die = sheet.hit_die_size;
        Self {
            max_hp: sheet.hit_points.maximum,
            current_hp: sheet.hit_points.maximum,
            temp_hp: 0,
            hit_die,
            hit_dice_max: sheet.level,
            hit_dice_left: sheet.level,
            constitution: sheet.ability_modifiers.constitution,
            features,
            spell_slots,
            recovery_levels: if has_recovery {
                sheet.level.div_ceil(2)
            } else {
                0
            },
        }
    }

    /// Whether these vitals belong to a build of the same shape as `whole` (the vitals fresh from
    /// a sheet): the same maxima of Hit Points, Hit Point Dice, feature uses and spell slots.
    pub fn fits(&self, whole: &HeroVitals) -> bool {
        let uses = |vitals: &HeroVitals| {
            vitals
                .features
                .iter()
                .map(|uses| (uses.feature.clone(), uses.maximum))
                .collect::<Vec<_>>()
        };
        let slots = |vitals: &HeroVitals| {
            vitals
                .spell_slots
                .iter()
                .map(|slots| (slots.level, slots.maximum))
                .collect::<Vec<_>>()
        };
        self.max_hp == whole.max_hp
            && self.hit_die == whole.hit_die
            && self.hit_dice_max == whole.hit_dice_max
            && self.recovery_levels == whole.recovery_levels
            && uses(self) == uses(whole)
            && slots(self) == slots(whole)
    }

    /// The spell slots of `level` left; 0 for a level the hero has no slot of.
    pub fn slots_left(&self, level: u8) -> u8 {
        self.spell_slots
            .iter()
            .find(|slots| slots.level == level)
            .map_or(0, |slots| slots.left)
    }

    fn use_feature(&mut self, feature: &str) -> Result<(), VitalsError> {
        let uses = self
            .features
            .iter_mut()
            .find(|uses| uses.feature == feature)
            .ok_or_else(|| VitalsError::UnknownFeature {
                feature: feature.to_owned(),
            })?;
        if uses.left == 0 {
            return Err(VitalsError::NoUsesLeft {
                feature: feature.to_owned(),
            });
        }
        uses.left -= 1;
        Ok(())
    }

    /// Arcane Recovery (SRD p. 78): one use, slots below level 6, combined levels within the
    /// budget, each slot recovered one that was expended.
    fn recover_slots(&mut self, levels: &[u8]) -> Result<(), VitalsError> {
        if levels.is_empty() {
            return Err(VitalsError::NothingToRecover);
        }
        if let Some(level) = levels
            .iter()
            .find(|level| **level >= ARCANE_RECOVERY_LEVEL_LIMIT)
        {
            return Err(VitalsError::RecoveryLevelTooHigh { level: *level });
        }
        let total: u16 = levels.iter().map(|level| u16::from(*level)).sum();
        if total > u16::from(self.recovery_levels) {
            return Err(VitalsError::RecoveryOverBudget {
                levels: total,
                budget: self.recovery_levels,
            });
        }
        self.use_feature(ARCANE_RECOVERY)?;
        for level in levels {
            let slots = self
                .spell_slots
                .iter_mut()
                .find(|slots| slots.level == *level && slots.left < slots.maximum)
                .ok_or(VitalsError::SlotNotExpended { level: *level })?;
            slots.left += 1;
        }
        Ok(())
    }

    /// The vitals after `change`, or why it is refused (and nothing changed).
    pub fn apply(&self, change: &VitalsChange) -> Result<Self, VitalsError> {
        let mut after = self.clone();
        match change {
            VitalsChange::Damage { amount } => {
                let amount = nonzero(*amount)?;
                let absorbed = amount.min(after.temp_hp);
                after.temp_hp -= absorbed;
                after.current_hp = after.current_hp.saturating_sub(amount - absorbed);
            }
            VitalsChange::Heal { amount } => {
                let amount = nonzero(*amount)?;
                after.current_hp = after.current_hp.saturating_add(amount).min(after.max_hp);
            }
            VitalsChange::GainTemporary { amount } => {
                after.temp_hp = after.temp_hp.max(nonzero(*amount)?);
            }
            VitalsChange::SpendHitDie { face } => {
                let sides = after.hit_die.sides();
                if !(1..=sides).contains(face) {
                    return Err(VitalsError::InvalidFace { face: *face, sides });
                }
                if after.hit_dice_left == 0 {
                    return Err(VitalsError::NoHitDiceLeft);
                }
                after.hit_dice_left -= 1;
                let healed = (i32::from(*face) + i32::from(after.constitution)).max(1);
                let healed = u32::try_from(healed).unwrap_or(1);
                after.current_hp = after.current_hp.saturating_add(healed).min(after.max_hp);
            }
            VitalsChange::UseFeature { feature } => after.use_feature(feature)?,
            VitalsChange::SpendSpellSlot { level } => {
                let slots = after
                    .spell_slots
                    .iter_mut()
                    .find(|slots| slots.level == *level && slots.left > 0)
                    .ok_or(VitalsError::NoSpellSlot { level: *level })?;
                slots.left -= 1;
            }
            VitalsChange::ArcaneRecovery { levels } => after.recover_slots(levels)?,
            VitalsChange::ShortRest => {
                // Second Wind: one expended use back on a Short Rest (SRD p. 47).
                for uses in &mut after.features {
                    if uses.feature == SECOND_WIND {
                        uses.left = (uses.left + 1).min(uses.maximum);
                    }
                }
            }
            VitalsChange::LongRest => {
                after.current_hp = after.max_hp;
                after.temp_hp = 0;
                after.hit_dice_left = after.hit_dice_max;
                for uses in &mut after.features {
                    uses.left = uses.maximum;
                }
                // Finishing a Long Rest restores every expended spell slot (SRD p. 104).
                for slots in &mut after.spell_slots {
                    slots.left = slots.maximum;
                }
            }
        }
        Ok(after)
    }
}

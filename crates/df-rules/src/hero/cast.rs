//! Whether a hero may cast a spell, and how: the checks before a slot is spent (Spells, SRD 5.2.1
//! pp. 103-106; Wizard, pp. 77-78).
//!
//! Owns `plan_cast`: the hero must cast spells in play and know the spell; a cantrip spends no
//! slot and is cast at level 0 (p. 104); a Ritual is a Ritual-tagged spell of the spellbook, cast
//! at its own level, 10 minutes longer and without a slot (Ritual Adept, p. 78; pp. 104, 187);
//! any other cast needs the spell prepared and a slot of the spell's level or higher that the
//! hero has (p. 104). Never does: spend the slot (the vitals do, and refuse a slot that is not
//! left) or say what the spell shows (the dungeon master narrates; Jev judges it).
use std::fmt;

use serde::{Deserialize, Serialize};

use super::sheet::{HeroSheet, Spellcasting};
use crate::spells::{RITUAL_EXTRA_MINUTES, SpellFacts, spell};

/// A cast the rules allow: the spell, the slot level it spends (none for a cantrip or a Ritual)
/// and the minutes a Ritual adds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CastPlan {
    pub spell: &'static SpellFacts,
    /// The level the spell is cast at: 0 for a cantrip.
    pub level: u8,
    pub slot: Option<u8>,
    pub ritual: bool,
    pub extra_minutes: u16,
}

/// Why a cast is refused.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum CastError {
    /// The hero casts no spells in play.
    NotACaster,
    /// The catalog has no spell of the id.
    UnknownSpell { spell: String },
    /// The hero does not know the spell (not a cantrip of theirs, not in their spellbook).
    NotKnown { spell: String },
    /// A level 1+ spell cast with a slot must be prepared.
    NotPrepared { spell: String },
    /// Only a spell with the Ritual tag is cast as a Ritual.
    NotARitual { spell: String },
    /// The level does not suit the spell: a cantrip is cast at 0, a Ritual at the spell's own
    /// level, and a slot is never below the spell's level.
    InvalidLevel { spell: String, level: u8 },
    /// The hero has no spell slot of the level left (or none of that level at all).
    NoSlot { level: u8 },
}

impl CastError {
    pub fn key(&self) -> &'static str {
        match self {
            Self::NotACaster => "not_a_caster",
            Self::UnknownSpell { .. } => "unknown_spell",
            Self::NotKnown { .. } => "not_known",
            Self::NotPrepared { .. } => "not_prepared",
            Self::NotARitual { .. } => "not_a_ritual",
            Self::InvalidLevel { .. } => "invalid_level",
            Self::NoSlot { .. } => "no_slot",
        }
    }
}

impl fmt::Display for CastError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotACaster => formatter.write_str("the hero casts no spells"),
            Self::UnknownSpell { spell } => write!(formatter, "there is no spell {spell}"),
            Self::NotKnown { spell } => write!(formatter, "the hero does not know {spell}"),
            Self::NotPrepared { spell } => write!(formatter, "{spell} is not prepared"),
            Self::NotARitual { spell } => write!(formatter, "{spell} is not a ritual"),
            Self::InvalidLevel { spell, level } => {
                write!(formatter, "{spell} can't be cast at level {level}")
            }
            Self::NoSlot { level } => write!(formatter, "no level {level} spell slot is left"),
        }
    }
}

impl std::error::Error for CastError {}

/// The cast of `id` at `level` (as a Ritual when `as_ritual`) the rules allow `sheet`'s hero, or
/// why not.
pub fn plan_cast(
    sheet: &HeroSheet,
    id: &str,
    level: u8,
    as_ritual: bool,
) -> Result<CastPlan, CastError> {
    let casting = sheet.spellcasting.as_ref().ok_or(CastError::NotACaster)?;
    let facts = spell(id).ok_or_else(|| CastError::UnknownSpell {
        spell: id.to_owned(),
    })?;
    let known = casting.known(id).ok_or_else(|| CastError::NotKnown {
        spell: id.to_owned(),
    })?;
    let invalid_level = || CastError::InvalidLevel {
        spell: id.to_owned(),
        level,
    };
    if as_ritual {
        if !facts.ritual {
            return Err(CastError::NotARitual {
                spell: id.to_owned(),
            });
        }
        // A Ritual spends no slot, so it is cast only at the spell's own level (p. 187).
        if level != facts.level {
            return Err(invalid_level());
        }
        return Ok(CastPlan {
            spell: facts,
            level,
            slot: None,
            ritual: true,
            extra_minutes: RITUAL_EXTRA_MINUTES,
        });
    }
    if facts.is_cantrip() {
        if level != 0 {
            return Err(invalid_level());
        }
        return Ok(CastPlan {
            spell: facts,
            level,
            slot: None,
            ritual: false,
            extra_minutes: 0,
        });
    }
    if !known.prepared {
        return Err(CastError::NotPrepared {
            spell: id.to_owned(),
        });
    }
    if level < facts.level {
        return Err(invalid_level());
    }
    if !has_slot_level(casting, level) {
        return Err(CastError::NoSlot { level });
    }
    Ok(CastPlan {
        spell: facts,
        level,
        slot: Some(level),
        ritual: false,
        extra_minutes: 0,
    })
}

fn has_slot_level(casting: &Spellcasting, level: u8) -> bool {
    casting
        .slots
        .iter()
        .any(|slots| slots.level == level && slots.count > 0)
}

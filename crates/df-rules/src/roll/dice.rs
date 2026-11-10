//! Die sizes, rolled dice and the `NdS+M` expression (SRD 5.2.1 pages 16 and 26 to 27).
//!
//! It owns the closed die sizes, what a rolled die records, the percentile pair and the expression
//! text. It rolls nothing and reads no draw; the resolvers read supplied faces.
//!
//! Source gap: the SRD has no dice-notation or percentile-dice section. `NdS` is read as the SRD
//! writes it in rules text (page 16, "roll 2d4"), and a percentile roll as two d10 (tens and ones)
//! follows the brief, with "00" meaning 100 as the last row of the page 27 Trinkets table shows.
use crate::listed::listed_enum;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// The most dice one expression may roll. A sanity bound for a typed request, not a rule.
pub const MAXIMUM_DICE_PER_EXPRESSION: u8 = 100;
/// The largest flat modifier a request may carry, either way. A sanity bound, not a rule.
pub const MAXIMUM_FLAT_MODIFIER: i16 = 1000;

listed_enum! {
    keyed
    /// A die size. `D100` is percentile: two d10 (tens, then ones).
    #[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum DieSize {
        D4,
        D6,
        D8,
        D10,
        D12,
        D20,
        D100,
    }
}

impl DieSize {
    pub fn key(self) -> &'static str {
        match self {
            Self::D4 => "d4",
            Self::D6 => "d6",
            Self::D8 => "d8",
            Self::D10 => "d10",
            Self::D12 => "d12",
            Self::D20 => "d20",
            Self::D100 => "d100",
        }
    }

    pub const fn sides(self) -> u16 {
        match self {
            Self::D4 => 4,
            Self::D6 => 6,
            Self::D8 => 8,
            Self::D10 => 10,
            Self::D12 => 12,
            Self::D20 => 20,
            Self::D100 => 100,
        }
    }

    /// The physical dice one die of this size is drawn as: a percentile die is two d10.
    pub(crate) const fn physical(self) -> &'static [DieSize] {
        match self {
            Self::D4 => &[Self::D4],
            Self::D6 => &[Self::D6],
            Self::D8 => &[Self::D8],
            Self::D10 => &[Self::D10],
            Self::D12 => &[Self::D12],
            Self::D20 => &[Self::D20],
            Self::D100 => &[Self::D10, Self::D10],
        }
    }
}

listed_enum! {
    keyed
    /// What a die is rolled for.
    #[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum DieRole {
        Test,
        Bonus,
        Damage,
        Healing,
        HitDie,
        Table,
    }
}

impl DieRole {
    pub fn key(self) -> &'static str {
        match self {
            Self::Test => "test",
            Self::Bonus => "bonus",
            Self::Damage => "damage",
            Self::Healing => "healing",
            Self::HitDie => "hit_die",
            Self::Table => "table",
        }
    }
}

listed_enum! {
    keyed
    /// Which digit of a percentile pair a d10 is. A d10 face of 10 is the digit 0.
    #[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum DiePlace {
        Whole,
        Tens,
        Ones,
    }
}

impl DiePlace {
    pub fn key(self) -> &'static str {
        match self {
            Self::Whole => "whole",
            Self::Tens => "tens",
            Self::Ones => "ones",
        }
    }
}

/// One die as rolled: its sides, the face the server drew and what it was for. `kept` is false for
/// the die an Advantage or Disadvantage roll set aside.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RolledDie {
    pub sides: u16,
    pub face: u16,
    pub role: DieRole,
    pub kept: bool,
    pub place: DiePlace,
}

impl RolledDie {
    pub(crate) fn new(size: DieSize, face: u16, role: DieRole, place: DiePlace) -> Self {
        Self {
            sides: size.sides(),
            face,
            role,
            kept: true,
            place,
        }
    }

    /// What this die adds on its own: the face, or the digit of a percentile pair (a d10 face of
    /// 10 is 0).
    pub const fn counted(self) -> u16 {
        match self.place {
            DiePlace::Whole => self.face,
            DiePlace::Tens => (self.face % 10) * 10,
            DiePlace::Ones => self.face % 10,
        }
    }
}

/// The value of a percentile pair: 1 to 100, with "00" as 100 (page 27, last row of the Trinkets
/// table).
pub(crate) const fn percentile_value(tens: RolledDie, ones: RolledDie) -> u16 {
    match tens.counted() + ones.counted() {
        0 => 100,
        value => value,
    }
}

/// `NdS+M`: `count` dice of `die` and a flat modifier.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiceExpression {
    pub count: u8,
    pub die: DieSize,
    pub modifier: i16,
}

/// Why a text is not a dice expression.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiceExpressionError {
    /// The text is not `[N]dS`, optionally followed by `+M` or `-M`.
    InvalidSyntax,
    /// The die has a number of sides that is not one of the supported sizes.
    UnsupportedDie,
}

impl fmt::Display for DiceExpressionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSyntax => "a dice expression reads like 2d6+3: dice, d, sides, modifier",
            Self::UnsupportedDie => "the die must be one of d4, d6, d8, d10, d12, d20 or d100",
        })
    }
}

impl std::error::Error for DiceExpressionError {}

impl FromStr for DiceExpression {
    type Err = DiceExpressionError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (count, rest) = text
            .split_once('d')
            .ok_or(DiceExpressionError::InvalidSyntax)?;
        let count = if count.is_empty() {
            1
        } else {
            count
                .parse::<u8>()
                .map_err(|_| DiceExpressionError::InvalidSyntax)?
        };
        let (sides, modifier) = match rest.find(['+', '-']) {
            Some(split) => {
                let (sides, signed) = rest.split_at(split);
                let magnitude = signed
                    .get(1..)
                    .filter(|digits| digits.bytes().all(|byte| byte.is_ascii_digit()))
                    .and_then(|digits| digits.parse::<i16>().ok())
                    .ok_or(DiceExpressionError::InvalidSyntax)?;
                let modifier = if signed.starts_with('-') {
                    -magnitude
                } else {
                    magnitude
                };
                (sides, modifier)
            }
            None => (rest, 0),
        };
        let sides = sides
            .parse::<u16>()
            .map_err(|_| DiceExpressionError::InvalidSyntax)?;
        let die = DieSize::ALL
            .iter()
            .copied()
            .find(|die| die.sides() == sides)
            .ok_or(DiceExpressionError::UnsupportedDie)?;
        Ok(Self {
            count,
            die,
            modifier,
        })
    }
}

impl fmt::Display for DiceExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}{}", self.count, self.die.key())?;
        match self.modifier {
            0 => Ok(()),
            modifier => write!(formatter, "{modifier:+}"),
        }
    }
}

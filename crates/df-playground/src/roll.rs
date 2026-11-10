//! The roll card on the shared screen: one roll the server resolved, staged as a call, a tumble of
//! the dice, the sum and the verdict, then taken away.
//!
//! The server owns every face, modifier, total and verdict (`RollView`); this module holds what the
//! page adds to it: the checks a live change passes, when each stage of the card begins
//! (`RollSchedule`), the cosmetic tumble of each die (`TumblePlan`, random-looking and landing on
//! the server's faces) and whether the card covers the screen's other panels (`covers_panels`).
//! `kit::roll_card` words and lays out the card and `view::roll` draws it. It never works out a
//! result, never rerolls and never hides a face.

use serde::{Deserialize, Serialize};

use crate::checks::labelled;
use crate::fixture::RollView;
use crate::limits::{
    MAXIMUM_ROLL_DICE, MAXIMUM_ROLL_MODIFIERS, MAXIMUM_ROLL_TUMBLE_MS, MINIMUM_ROLL_HOLD_MS,
    MINIMUM_ROLL_TUMBLE_MS, ROLL_CALL_MS, ROLL_EXIT_MS, ROLL_SETTLE_MS, ROLL_VERDICT_DELAY_MS,
};
use crate::media::is_plain_file_name;
use crate::scene::SceneError;

/// The dice a roll may hold; 100 is a percentile die, drawn as two d10.
pub const DIE_SIDES: [u8; 7] = [4, 6, 8, 10, 12, 20, 100];

/// A roll on the screen: the server's `RollView`, how long it must stay up, and the host's stamps
/// that let a page opened in the middle of it carry on from where it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShownRoll {
    pub view: RollView,
    /// The least time the card stays up after the verdict appears, in milliseconds: the narrator's
    /// line that reads the result and two seconds more. The card never leaves sooner than
    /// `MINIMUM_ROLL_HOLD_MS` after the verdict either.
    #[serde(default)]
    pub min_hold_ms: u64,
    /// Counts the rolls of a session, so a page can tell a new roll from the one it has begun. The
    /// host sets it; leave it out of a scene file.
    #[serde(default)]
    pub serial: u64,
    /// The host's clock, in milliseconds since it started, when this roll began. The host sets it;
    /// leave it out of a scene file.
    #[serde(default)]
    pub began_ms: u64,
    /// How long ago the roll began, as the host's clock read when it built the answer. The host
    /// sets it; leave it out of a scene file.
    #[serde(default)]
    pub elapsed_ms: u64,
}

impl RollView {
    /// Checks the roll the way a live change is checked: a plain id, dice the page can draw with
    /// faces they have, a reroll that points at a die, and a total that is the kept dice and the
    /// modifiers added up, so the card never writes a sum that is not one.
    pub fn check(&self) -> Result<(), SceneError> {
        if !is_plain_file_name(&self.id) {
            return Err(SceneError::InvalidRollId(self.id.clone()));
        }
        labelled("a roll's title", &self.title)?;
        for (what, count, limit) in [
            ("a roll's dice", self.dice.len(), MAXIMUM_ROLL_DICE),
            (
                "a roll's modifiers",
                self.modifiers.len(),
                MAXIMUM_ROLL_MODIFIERS,
            ),
        ] {
            if count > limit {
                return Err(SceneError::TooMany { what, limit });
            }
        }
        for die in &self.dice {
            check_die(die.sides, die.face)?;
        }
        for modifier in &self.modifiers {
            labelled("a roll modifier's label", &modifier.label)?;
        }
        if let Some(target) = &self.target {
            labelled("a roll target's label", &target.label)?;
        }
        if let Some(reroll) = &self.inspiration_reroll {
            let die = self
                .dice
                .get(reroll.index)
                .ok_or(SceneError::InvalidReroll)?;
            check_die(die.sides, reroll.was).map_err(|_| SceneError::InvalidReroll)?;
        }
        let sum = self.sum();
        if sum != i64::from(self.total) {
            return Err(SceneError::RollTotalMismatch {
                sum,
                total: self.total,
            });
        }
        Ok(())
    }

    /// The kept dice and the modifiers added up.
    fn sum(&self) -> i64 {
        let dice: i64 = self
            .dice
            .iter()
            .filter(|die| die.kept)
            .map(|die| i64::from(die.face))
            .sum();
        let modifiers: i64 = self
            .modifiers
            .iter()
            .map(|modifier| i64::from(modifier.value))
            .sum();
        dice + modifiers
    }
}

fn check_die(sides: u8, face: u8) -> Result<(), SceneError> {
    if DIE_SIDES.contains(&sides) && (1..=sides).contains(&face) {
        Ok(())
    } else {
        Err(SceneError::InvalidDie { sides, face })
    }
}

/// Where a roll is in its staging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollPhase {
    /// The title and the target, while the narrator calls the roll.
    Call,
    /// The dice roll in and settle.
    Tumble,
    /// The dice lie still and their faces appear.
    Settle,
    /// The sum, the total and the verdict, held to be read.
    Breakdown,
    /// The card fades away.
    Leaving,
    /// The card is gone.
    Gone,
}

impl RollPhase {
    /// Whether the card hides the screen's other panels (the inventory, the clue board, the
    /// story's mark): from the call until it begins to leave, so they fade back as it fades.
    pub fn covers_panels(self) -> bool {
        matches!(
            self,
            Self::Call | Self::Tumble | Self::Settle | Self::Breakdown
        )
    }
}

/// When each stage of a roll begins, in milliseconds after the card appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RollSchedule {
    tumble_ms: u64,
    hold_ms: u64,
}

impl RollSchedule {
    /// A schedule whose tumble takes `tumble_ms` (held to the allowed range) and whose card stays
    /// up at least `min_hold_ms` after the verdict appears (and never under
    /// `MINIMUM_ROLL_HOLD_MS`): the whole result, sum and verdict, is read for that long.
    pub fn new(tumble_ms: u64, min_hold_ms: u64) -> Self {
        Self {
            tumble_ms: tumble_ms.clamp(MINIMUM_ROLL_TUMBLE_MS, MAXIMUM_ROLL_TUMBLE_MS),
            hold_ms: min_hold_ms.max(MINIMUM_ROLL_HOLD_MS),
        }
    }

    pub fn tumble_start_ms(self) -> u64 {
        ROLL_CALL_MS
    }

    pub fn tumble_ms(self) -> u64 {
        self.tumble_ms
    }

    /// When the dice lie still and their faces begin to appear.
    pub fn tumble_end_ms(self) -> u64 {
        ROLL_CALL_MS + self.tumble_ms
    }

    /// When the sum begins to appear: after the dice have been still for `ROLL_SETTLE_MS`.
    pub fn breakdown_ms(self) -> u64 {
        self.tumble_end_ms() + ROLL_SETTLE_MS
    }

    /// When the verdict begins to appear.
    pub fn verdict_ms(self) -> u64 {
        self.breakdown_ms() + ROLL_VERDICT_DELAY_MS
    }

    /// How long the verdict stays up before the card leaves.
    pub fn hold_ms(self) -> u64 {
        self.hold_ms
    }

    /// When the card begins to leave.
    pub fn off_ms(self) -> u64 {
        self.verdict_ms() + self.hold_ms
    }

    /// How long the sum is up, from its appearing to the card's leaving.
    pub fn sum_ms(self) -> u64 {
        self.off_ms() - self.breakdown_ms()
    }

    /// When the card is gone.
    pub fn gone_ms(self) -> u64 {
        self.off_ms() + ROLL_EXIT_MS
    }

    /// The stage `age_ms` after the card appeared is in.
    pub fn phase_at(self, age_ms: u64) -> RollPhase {
        if age_ms < self.tumble_start_ms() {
            RollPhase::Call
        } else if age_ms < self.tumble_end_ms() {
            RollPhase::Tumble
        } else if age_ms < self.breakdown_ms() {
            RollPhase::Settle
        } else if age_ms < self.off_ms() {
            RollPhase::Breakdown
        } else if age_ms < self.gone_ms() {
            RollPhase::Leaving
        } else {
            RollPhase::Gone
        }
    }
}

impl ShownRoll {
    /// Checks the roll the way a live change is checked.
    pub fn check(&self) -> Result<(), SceneError> {
        self.view.check()
    }

    /// Whether the roll has run its course by the host's reckoning, for a page that opens late.
    pub fn is_over(&self, schedule: RollSchedule) -> bool {
        self.elapsed_ms >= schedule.gone_ms()
    }
}

/// Whether the screen's other panels step aside: a roll is on screen and the page has not seen it
/// begin to leave (`left_serial` is the serial of the roll that has).
pub fn covers_panels(roll: Option<&ShownRoll>, left_serial: Option<u64>) -> bool {
    roll.is_some_and(|roll| left_serial != Some(roll.serial))
}

/// A small deterministic generator (SplitMix64), so a tumble is the same for the same seed and the
/// tests need no clock.
#[derive(Debug, Clone)]
struct Draw(u64);

impl Draw {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^ (mixed >> 31)
    }

    /// A whole number from `low` to `high`, both included.
    fn between(&mut self, low: i32, high: i32) -> i32 {
        let span = u64::try_from(high - low + 1).unwrap_or(1);
        low + i32::try_from(self.next() % span).unwrap_or(0)
    }

    /// `between`, with a random sign.
    fn signed(&mut self, low: i32, high: i32) -> i32 {
        let value = self.between(low, high);
        if self.next().is_multiple_of(2) {
            value
        } else {
            -value
        }
    }
}

/// How one die tumbles: a cosmetic toss across the tray that ends where the die lies, never a
/// face. The start is in cqw from where the die lies, the rest offsets in percent of the die's own
/// size, angles in degrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DieTumble {
    pub from_x_cqw: i32,
    pub from_y_cqw: i32,
    pub spin_degrees: i32,
    pub rest_degrees: i32,
    pub rest_x_percent: i32,
    pub rest_y_percent: i32,
    /// How high the die hops after its first landing, in percent of its size.
    pub hop_percent: i32,
    /// How long after the tumble begins this die is thrown, in milliseconds.
    pub delay_ms: u64,
}

/// The tumble of every die of a roll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TumblePlan {
    pub tumble_ms: u64,
    pub dice: Vec<DieTumble>,
}

/// The longest wait before one die is thrown, in milliseconds.
const LONGEST_THROW_DELAY_MS: i32 = 180;

impl TumblePlan {
    /// The tumble `seed` gives `dice` dice. The same seed gives the same plan; every die lands by
    /// the end of the tumble. The seed is the page's own randomness, never the roll's faces.
    pub fn new(seed: u64, dice: usize) -> Self {
        let mut draw = Draw(seed);
        let tumble_ms = u64::try_from(draw.between(
            i32::try_from(MINIMUM_ROLL_TUMBLE_MS).unwrap_or(1_200),
            i32::try_from(MAXIMUM_ROLL_TUMBLE_MS).unwrap_or(1_800),
        ))
        .unwrap_or(MINIMUM_ROLL_TUMBLE_MS);
        let dice = (0..dice)
            .map(|_| {
                let (from_x_cqw, from_y_cqw) = match draw.between(0, 2) {
                    0 => (-draw.between(17, 25), -draw.between(1, 6)),
                    1 => (draw.between(17, 25), -draw.between(1, 6)),
                    _ => (draw.signed(0, 8), -draw.between(9, 11)),
                };
                DieTumble {
                    from_x_cqw,
                    from_y_cqw,
                    spin_degrees: draw.signed(540, 1_260),
                    rest_degrees: draw.signed(4, 26),
                    rest_x_percent: draw.signed(4, 18),
                    rest_y_percent: draw.signed(2, 6),
                    hop_percent: draw.between(14, 30),
                    delay_ms: u64::try_from(draw.between(0, LONGEST_THROW_DELAY_MS)).unwrap_or(0),
                }
            })
            .collect();
        Self { tumble_ms, dice }
    }
}

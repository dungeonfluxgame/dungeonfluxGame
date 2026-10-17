//! What the shared display shows for a server d20 roll: the dice (two for Advantage or
//! Disadvantage, the counted one marked), then a result card, and how long each stays.
//!
//! Everything here is a decision on the server's numbers: the faces, the modifier, the total and
//! the number rolled against come from the wire `TableRoll` and are never recomputed or
//! invented. The result card's words are composed from those numbers, so the card always matches
//! the die on screen. The browser layer only plays this plan.
use df_protocol::common as rpc;

use crate::dice_roll::{DicePlay, RollMode, play_of};
use crate::stage_machines::reading_time_ms;

/// The least time a result card stays up after the die lands, whatever its length: long enough
/// to read across a room (the line rule's floor for a short line is shorter than this).
pub(crate) const MIN_RESULT_HOLD_MS: u32 = 3_000;
/// The rest time budgeted before the manifest has loaded or when no clip plays.
pub(crate) const DEFAULT_REST_MS: u64 = 1_400;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Against {
    DifficultyClass,
    ArmorClass,
}

impl Against {
    fn word(self) -> &'static str {
        match self {
            Self::DifficultyClass => "DC",
            Self::ArmorClass => "AC",
        }
    }
}

/// One face on the table and whether it is the one the server counted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CardDie {
    pub(crate) face: u8,
    pub(crate) counted: bool,
}

/// The display plan for one roll.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RollCard {
    pub(crate) label: String,
    /// The counted die first; a second, uncounted die only when the server rolled two.
    pub(crate) dice: Vec<CardDie>,
    pub(crate) mode: RollMode,
    pub(crate) modifier: i32,
    pub(crate) total: i32,
    pub(crate) against: Option<(Against, u32)>,
    pub(crate) succeeded: bool,
    pub(crate) kind: rpc::RollKind,
}

/// When things happen after the roll starts, in milliseconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CardTiming {
    /// The die comes to rest, the server's number lands and the result card appears.
    pub(crate) rest_ms: u64,
    /// How long the result card is held after the die lands.
    pub(crate) hold_ms: u32,
    /// The roll is over: the card has been held and starts to fade.
    pub(crate) end_ms: u64,
}

impl RollCard {
    /// The plan for the server's roll, or `None` when it carries no usable die. A second face
    /// is shown only when the server sent one for Advantage or Disadvantage.
    pub(crate) fn from_wire(roll: &rpc::TableRoll) -> Option<Self> {
        let play = play_of(roll.kept, &roll.faces, roll.roll_mode)?;
        let mut dice = vec![CardDie {
            face: play.kept,
            counted: true,
        }];
        if let Some(other) = play.other {
            dice.push(CardDie {
                face: other,
                counted: false,
            });
        }
        let against = match (
            rpc::RollAgainst::try_from(roll.against).ok()?,
            roll.target_number,
        ) {
            (rpc::RollAgainst::Dc, number) => Some((Against::DifficultyClass, number)),
            (rpc::RollAgainst::Ac, number) => Some((Against::ArmorClass, number)),
            (rpc::RollAgainst::Unspecified, _) => None,
        };
        Some(Self {
            label: roll.label.trim().to_owned(),
            dice,
            mode: play.mode,
            modifier: roll.modifier,
            total: roll.total,
            against,
            succeeded: roll.succeeded,
            kind: rpc::RollKind::try_from(roll.kind).unwrap_or(rpc::RollKind::Unspecified),
        })
    }

    /// The counted face, which the die settles on.
    pub(crate) fn counted_face(&self) -> u8 {
        self.dice.first().map_or(0, |die| die.face)
    }

    /// The dice for the clip layer: the kept face, the other draw, the mode, the natural.
    pub(crate) fn play(&self) -> DicePlay {
        let kept = self.counted_face();
        DicePlay {
            kept,
            other: self.dice.get(1).map(|die| die.face),
            mode: self.mode,
            natural: match kept {
                20 => Some(crate::dice_roll::Natural::Twenty),
                1 => Some(crate::dice_roll::Natural::One),
                _ => None,
            },
        }
    }

    /// "13 + 1 = 14 vs DC 10": the counted face, the modifier, the total and what it was
    /// rolled against, from the server's numbers.
    pub(crate) fn sum_text(&self) -> String {
        let face = self.counted_face();
        let mut text = if self.modifier == 0 {
            format!("{face} = {}", self.total)
        } else {
            format!(
                "{face} {} {} = {}",
                if self.modifier < 0 { "\u{2212}" } else { "+" },
                self.modifier.unsigned_abs(),
                self.total
            )
        };
        if let Some((against, number)) = self.against {
            text.push_str(&format!(" vs {} {number}", against.word()));
        }
        text
    }

    /// The one-word verdict, or `None` when nothing was rolled against (initiative).
    pub(crate) fn verdict(&self) -> Option<&'static str> {
        self.against?;
        Some(match (self.kind, self.succeeded) {
            (rpc::RollKind::Attack, true) => "Hit",
            (rpc::RollKind::Attack, false) => "Miss",
            (_, true) => "Success",
            (_, false) => "Failure",
        })
    }

    /// Everything the card says, for the reading-time rule.
    fn card_text(&self) -> String {
        format!(
            "{} {} {}",
            self.label,
            self.sum_text(),
            self.verdict().unwrap_or_default()
        )
    }

    /// The die tumbles to rest at `rest_ms` (the clip's own time; the budget when no clip plays),
    /// then the result card is held for its reading time (the line rule) and never less than
    /// [`MIN_RESULT_HOLD_MS`]. A display that is catching up holds it for that floor alone: the
    /// die and the result still show, only the reading allowance is cut.
    pub(crate) fn timing(&self, rest_ms: u64, catching_up: bool) -> CardTiming {
        let rest_ms = if rest_ms == 0 {
            DEFAULT_REST_MS
        } else {
            rest_ms
        };
        let hold_ms = if catching_up {
            MIN_RESULT_HOLD_MS
        } else {
            reading_time_ms(&self.card_text()).max(MIN_RESULT_HOLD_MS)
        };
        CardTiming {
            rest_ms,
            hold_ms,
            end_ms: rest_ms + u64::from(hold_ms),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roll(kept: u32, faces: &[u32], mode: i32) -> rpc::TableRoll {
        rpc::TableRoll {
            faces: faces.to_vec(),
            kept,
            roll_mode: mode,
            modifier: -1,
            total: i32::try_from(kept).unwrap() - 1,
            against: rpc::RollAgainst::Dc as i32,
            target_number: 15,
            succeeded: false,
            label: "Brakka \u{b7} Deception".to_owned(),
            kind: rpc::RollKind::Check as i32,
        }
    }

    #[test]
    fn a_plain_roll_is_one_counted_die() {
        let card = RollCard::from_wire(&roll(8, &[8], 0)).expect("a usable roll");
        assert_eq!(
            card.dice,
            [CardDie {
                face: 8,
                counted: true
            }]
        );
        assert_eq!(card.mode, RollMode::Plain);
        assert_eq!(card.counted_face(), 8);
    }

    #[test]
    fn advantage_and_disadvantage_show_two_dice_with_the_counted_one_marked() {
        let advantage = RollCard::from_wire(&roll(8, &[8, 1], 1)).expect("a usable roll");
        assert_eq!(
            advantage.dice,
            [
                CardDie {
                    face: 8,
                    counted: true
                },
                CardDie {
                    face: 1,
                    counted: false
                }
            ]
        );
        assert_eq!(advantage.mode, RollMode::Advantage);
        assert_eq!(advantage.play().other, Some(1));

        let disadvantage = RollCard::from_wire(&roll(3, &[19, 3], -1)).expect("a usable roll");
        assert_eq!(disadvantage.mode, RollMode::Disadvantage);
        assert_eq!(disadvantage.counted_face(), 3);
        assert_eq!(disadvantage.dice.len(), 2);
        assert_eq!(
            disadvantage.dice.iter().filter(|die| die.counted).count(),
            1
        );
    }

    #[test]
    fn the_client_never_invents_a_second_die_or_a_face() {
        // Advantage without a second draw is one die; a face off the d20 is no roll at all.
        let one = RollCard::from_wire(&roll(8, &[8], 1)).expect("a usable roll");
        assert_eq!(one.dice.len(), 1);
        assert_eq!(RollCard::from_wire(&roll(0, &[], 0)), None);
        assert_eq!(RollCard::from_wire(&roll(21, &[21], 0)), None);
        let outsider = RollCard::from_wire(&roll(8, &[8, 99], 1)).expect("a usable roll");
        assert_eq!(outsider.dice.len(), 1);
    }

    #[test]
    fn the_card_words_come_from_the_servers_numbers() {
        let card = RollCard::from_wire(&roll(8, &[8, 1], 1)).expect("a usable roll");
        assert_eq!(card.sum_text(), "8 \u{2212} 1 = 7 vs DC 15");
        assert_eq!(card.verdict(), Some("Failure"));

        let mut stabilise = roll(13, &[13], 0);
        stabilise.modifier = 1;
        stabilise.total = 14;
        stabilise.target_number = 10;
        stabilise.succeeded = true;
        stabilise.kind = rpc::RollKind::Stabilize as i32;
        let card = RollCard::from_wire(&stabilise).expect("a usable roll");
        assert_eq!(card.sum_text(), "13 + 1 = 14 vs DC 10");
        assert_eq!(card.verdict(), Some("Success"));

        let mut attack = roll(17, &[17], 0);
        attack.kind = rpc::RollKind::Attack as i32;
        attack.against = rpc::RollAgainst::Ac as i32;
        attack.succeeded = true;
        let card = RollCard::from_wire(&attack).expect("a usable roll");
        assert!(card.sum_text().ends_with("vs AC 15"));
        assert_eq!(card.verdict(), Some("Hit"));

        let mut initiative = roll(12, &[12], 0);
        initiative.against = rpc::RollAgainst::Unspecified as i32;
        assert_eq!(
            RollCard::from_wire(&initiative).expect("usable").verdict(),
            None
        );
    }

    #[test]
    fn the_result_is_held_at_least_three_seconds_and_at_least_its_reading_time() {
        let card = RollCard::from_wire(&roll(8, &[8, 1], 1)).expect("a usable roll");
        let timing = card.timing(1_300, false);
        assert_eq!(timing.rest_ms, 1_300);
        assert!(timing.hold_ms >= MIN_RESULT_HOLD_MS);
        assert!(timing.hold_ms >= reading_time_ms(&card.card_text()));
        assert_eq!(timing.end_ms, 1_300 + u64::from(timing.hold_ms));

        // A long label is held for its reading time even though it exceeds the floor.
        let mut long = roll(8, &[8], 0);
        long.label = "A very long label ".repeat(12);
        let card = RollCard::from_wire(&long).expect("a usable roll");
        assert!(card.timing(1_300, false).hold_ms >= reading_time_ms(&card.card_text()));
    }

    #[test]
    fn catching_up_shortens_the_hold_but_never_the_die_or_the_three_second_floor() {
        let card = RollCard::from_wire(&roll(8, &[8], 0)).expect("a usable roll");
        let normal = card.timing(1_300, false);
        let short = card.timing(1_300, true);
        assert!(short.hold_ms <= normal.hold_ms);
        assert_eq!(
            short.hold_ms, MIN_RESULT_HOLD_MS,
            "the card still shows for 3 s"
        );
        assert_eq!(short.rest_ms, 1_300, "the die still tumbles to its rest");
    }

    #[test]
    fn a_roll_with_no_clip_uses_the_default_rest_budget() {
        let card = RollCard::from_wire(&roll(8, &[8], 0)).expect("a usable roll");
        assert_eq!(card.timing(0, false).rest_ms, DEFAULT_REST_MS);
    }
}

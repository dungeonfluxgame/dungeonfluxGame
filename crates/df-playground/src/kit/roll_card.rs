//! The roll card: what a resolved roll is called, which dice stand on the tray and how the sum is
//! written. The words and the layout of a card are decided here as plain functions native tests
//! cover; `kit::draw::roll_card` makes the elements and `view::roll` stages them.
//!
//! Every number on the card is the server's (`RollView`): a face, a modifier, the total, the
//! target. The card writes the dice and modifiers as one sum, once, and never works out a result.

use serde::{Deserialize, Serialize};

use crate::fixture::{DieRole, NaturalMark, RollDie, RollKind, RollMode, RollOutcome, RollView};
use crate::kit::roll_tray::TrayLayout;
use crate::limits::{ROLL_BREAKDOWN_MS, ROLL_EXIT_MS, ROLL_FACES_MS};
use crate::roll::{DieTumble, RollSchedule};

/// The picture a die is drawn from: `assets/playground/dice/<key>.png`, with no numeral on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DieArt {
    D4,
    D6,
    D8,
    D10,
    D12,
    D20,
}

/// Where the numeral goes on a die's picture, in thousandths of the picture's width, read from the
/// pale-gold panel that faces the viewer (the largest panel of the picture). How large the numeral
/// is does not depend on the die (`kit::roll_tray`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaceGeometry {
    pub x_permille: u16,
    pub y_permille: u16,
    /// How much the picture is drawn larger than the others, in percent: a d4 and a d8 fill less
    /// of their picture.
    pub scale_percent: u16,
}

impl DieArt {
    pub const ALL: [Self; 6] = [
        Self::D4,
        Self::D6,
        Self::D8,
        Self::D10,
        Self::D12,
        Self::D20,
    ];

    /// The picture's name without its type: `d20`.
    pub fn key(self) -> &'static str {
        match self {
            Self::D4 => "d4",
            Self::D6 => "d6",
            Self::D8 => "d8",
            Self::D10 => "d10",
            Self::D12 => "d12",
            Self::D20 => "d20",
        }
    }

    pub fn geometry(self) -> FaceGeometry {
        let (x_permille, y_permille, scale_percent) = match self {
            Self::D4 => (530, 630, 118),
            Self::D6 => (350, 540, 100),
            Self::D8 => (518, 424, 116),
            Self::D10 => (508, 393, 104),
            Self::D12 => (494, 423, 100),
            Self::D20 => (520, 503, 100),
        };
        FaceGeometry {
            x_permille,
            y_permille,
            scale_percent,
        }
    }

    /// The picture a die of `sides` is drawn from, and whether it is half of a percentile pair.
    pub fn of_sides(sides: u8) -> Option<(Self, bool)> {
        match sides {
            4 => Some((Self::D4, false)),
            6 => Some((Self::D6, false)),
            8 => Some((Self::D8, false)),
            10 => Some((Self::D10, false)),
            12 => Some((Self::D12, false)),
            20 => Some((Self::D20, false)),
            100 => Some((Self::D10, true)),
            _ => None,
        }
    }
}

/// How a die stands in the sum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DieState {
    /// It counts: lit.
    Counted,
    /// It was rolled and left out (the other d20 of an advantage, a dropped die): dimmed, struck.
    Unused,
    /// A Heroic Inspiration reroll took its place: dimmed and struck, beside the die that did.
    Replaced,
}

impl DieState {
    /// The word that ends the CSS class.
    pub fn key(self) -> &'static str {
        match self {
            Self::Counted => "counted",
            Self::Unused => "unused",
            Self::Replaced => "replaced",
        }
    }
}

/// One die on the tray.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DieSlot {
    pub art: DieArt,
    /// What is written on the face: the server's number, or one half of a percentile roll.
    pub numeral: String,
    pub state: DieState,
    /// A small word under the die: which half of a percentile roll it is.
    pub caption: Option<&'static str>,
}

/// One term of the written sum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    /// Whether it is taken away: written with a minus where it is not the first term.
    pub negative: bool,
    /// The magnitude of the number.
    pub number: String,
    /// What the number is (a die's role where the sum mixes them, a modifier's name).
    pub label: Option<String>,
}

/// The look of the verdict's word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Positive,
    Negative,
    Neutral,
}

impl Tone {
    /// The word that ends the CSS class.
    pub fn key(self) -> &'static str {
        match self {
            Self::Positive => "positive",
            Self::Negative => "negative",
            Self::Neutral => "neutral",
        }
    }
}

/// The card's kicker: what kind of roll it is.
pub fn kind_word(kind: RollKind) -> &'static str {
    match kind {
        RollKind::AbilityCheck => "Ability check",
        RollKind::SkillCheck => "Skill check",
        RollKind::PassivePerception => "Passive score",
        RollKind::SavingThrow => "Saving throw",
        RollKind::MeleeAttack => "Melee attack",
        RollKind::RangedAttack => "Ranged attack",
        RollKind::ContestedCheck => "Contested check",
        RollKind::Initiative => "Initiative",
        RollKind::DeathSavingThrow => "Death saving throw",
        RollKind::Damage => "Damage",
        RollKind::Healing => "Healing",
        RollKind::HitDie => "Hit die",
        RollKind::BonusDie => "Bonus die",
        RollKind::TableRoll => "Table roll",
    }
}

/// The word that tells a die from the others in a sum that mixes roles.
pub fn role_word(role: DieRole) -> &'static str {
    match role {
        DieRole::Test => "Test die",
        DieRole::Bonus => "Bonus die",
        DieRole::Damage => "Damage",
        DieRole::Healing => "Healing",
        DieRole::HitDie => "Hit die",
        DieRole::Table => "Table die",
    }
}

/// The verdict's word and look.
pub fn outcome_word(outcome: RollOutcome) -> (&'static str, Tone) {
    match outcome {
        RollOutcome::Success => ("Success", Tone::Positive),
        RollOutcome::Failure => ("Failure", Tone::Negative),
        RollOutcome::Hit => ("Hit", Tone::Positive),
        RollOutcome::Miss => ("Miss", Tone::Negative),
        RollOutcome::CriticalHit => ("Critical hit", Tone::Positive),
        RollOutcome::Won => ("Won", Tone::Positive),
        RollOutcome::Lost => ("Lost", Tone::Negative),
        RollOutcome::Tied => ("Tied", Tone::Neutral),
    }
}

/// The sides of the percentile die, which is drawn as two d10.
const PERCENTILE_SIDES: u8 = 100;

/// What the total's place shows while the dice roll, when the roll has no target.
pub const PENDING_MARK: &str = "\u{2014}";

/// A sum of more terms than this is written smaller, so a crowd of dice still fits its line.
pub const TERMS_BEFORE_SMALLER_SUM: usize = 8;

/// A verdict of more characters than this is stamped smaller, so it stays on one line.
pub const VERDICT_CHARACTERS_BEFORE_SMALLER: usize = 8;

/// A total of more characters than this is written smaller, so it keeps its box beside the target.
const TOTAL_CHARACTERS_BEFORE_SMALLER: u32 = 2;

/// A target of more characters than this is set beside a smaller total, so its words stay whole.
pub const TARGET_CHARACTERS_BEFORE_SMALLER_TOTAL: u32 = 8;

/// A title longer than this many characters is written smaller, so its two lines stay clear of
/// the narration under the card.
pub const TITLE_CHARACTERS_BEFORE_SMALLER: usize = 44;

/// A sum whose terms, with their names and signs, run to more characters than this is written
/// smaller too, so it stays within two lines.
pub const SUM_CHARACTERS_BEFORE_SMALLER: usize = 50;

/// The sizes the card's height is worked out from (`assets/playground/kit.css`), in tenths of a
/// cqw: the stage is 100cqw wide and 56.25cqw tall, and the card hangs 2.6cqw from its top.
const STAGE_HEIGHT_TENTHS: u32 = 562;
const CARD_TOP_TENTHS: u32 = 26;
/// The space kept clear at the foot of the screen under the stamp.
const SCREEN_FOOT_MARGIN_TENTHS: u32 = 16;
/// The card is never drawn smaller than this, in percent.
const LEAST_FIT_PERCENT: u32 = 60;
/// The width the card's content has, between its padding.
const CARD_CONTENT_WIDTH_TENTHS: u32 = 286;
/// The pane's padding above and below, and the room the foot keeps for the stamp.
const CARD_PADDING_TENTHS: u32 = 13 + 14 + 27;
const SECTION_GAP_TENTHS: u32 = 7;
const MARKS_ROW_TENTHS: u32 = 24;
const MARKS_ROW_GAP_TENTHS: u32 = 3;
/// The gap between a kicker and its chips, and the widths of their characters: a kicker's
/// capitals are spaced, a chip is smaller and padded.
const MARKS_GAP_TENTHS: u32 = 7;
const KICKER_CHARACTER_TENTHS: u32 = 17;
const CHIP_CHARACTER_TENTHS: u32 = 14;
const CHIP_PADDING_TENTHS: u32 = 25;
/// A chip's height: its lines of text, and its padding and border.
const CHIP_LINE_TENTHS: u32 = 20;
const CHIP_FIXED_TENTHS: u32 = 11;
const TITLE_FONT_TENTHS: u32 = 29;
const TITLE_FONT_LONG_TENTHS: u32 = 22;
/// How many characters of the title fit a line at each size.
const TITLE_LINE_CHARACTERS: u32 = 22;
const TITLE_LINE_CHARACTERS_LONG: u32 = 29;
/// A tally's tray.
const TALLY_TRAY_TENTHS: u32 = 135;
const TOTAL_BOX_TENTHS: u32 = 112;
const TOTAL_FONT_TENTHS: u32 = 100;
const TOTAL_FONT_SMALL_TENTHS: u32 = 80;
/// The box of the total pads its sides and keeps a gap between the total and the target's pill.
const TOTAL_BOX_SIDE_TENTHS: u32 = 16;
const TOTAL_BOX_GAP_TENTHS: u32 = 12;
const TOTAL_BOX_PADDING_TENTHS: u32 = 12;
/// The target's pill: its side padding, a line of its words, and what is over them ("vs" and the
/// padding).
const PILL_SIDE_TENTHS: u32 = 15;
const PILL_SIDE_LONG_TENTHS: u32 = 11;
const PILL_CHARACTER_TENTHS: u32 = 13;
const PILL_SPACE_TENTHS: u32 = 6;
const PILL_LINE_TENTHS: u32 = 33;
const PILL_FIXED_TENTHS: u32 = 38;
const SUM_FONT_TENTHS: u32 = 30;
const SUM_FONT_LONG_TENTHS: u32 = 23;
const TERM_LABEL_FONT_TENTHS: u32 = 20;
/// The gap between the parts of a term, and between one term and the next.
const SUM_PART_GAP_TENTHS: u32 = 6;
const SUM_TERM_GAP_TENTHS: u32 = 8;
/// How far the verdict's seal hangs below the card.
const STAMP_HANG_TENTHS: u32 = 40;

/// The characters of `text`.
fn character_count(text: &str) -> u32 {
    u32::try_from(text.chars().count()).unwrap_or(u32::MAX)
}

/// The sign before how many dice showed a face, on a tally's chip.
pub const COUNT_SIGN: char = '\u{d7}';

/// The minus sign of a negative number on the card.
pub const MINUS: char = '\u{2212}';

/// A signed whole number as the card writes it.
pub fn signed(value: i32) -> String {
    if value < 0 {
        format!("{MINUS}{}", value.unsigned_abs())
    } else {
        value.to_string()
    }
}

/// The face of a percentile roll as its two dice show it: tens ("00" to "90") and ones ("0" to
/// "9"); a face of 100 is "00" and "0".
pub fn percentile_numerals(face: u8) -> (String, String) {
    let value = if face == 100 { 0 } else { face };
    (format!("{:02}", value / 10 * 10), (value % 10).to_string())
}

/// The roll card's input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RollCard {
    pub view: RollView,
}

impl From<&RollView> for RollCard {
    fn from(view: &RollView) -> Self {
        Self { view: view.clone() }
    }
}

impl RollCard {
    pub fn title(&self) -> &str {
        self.view.title.trim()
    }

    /// Whether the title is written smaller: it is long.
    pub fn title_is_long(&self) -> bool {
        self.title().chars().count() > TITLE_CHARACTERS_BEFORE_SMALLER
    }

    pub fn kicker(&self) -> Option<&'static str> {
        let word = kind_word(self.view.kind);
        (!self.title().eq_ignore_ascii_case(word)).then_some(word)
    }

    /// "Advantage" or "Disadvantage" over a roll made so.
    pub fn mode_word(&self) -> Option<&'static str> {
        match self.view.mode {
            RollMode::Normal => None,
            RollMode::Advantage => Some("Advantage"),
            RollMode::Disadvantage => Some("Disadvantage"),
        }
    }

    /// The target as it is read: "DC 15".
    pub fn target_text(&self) -> Option<String> {
        self.view
            .target
            .as_ref()
            .map(|target| format!("{} {}", target.label.trim(), target.value))
    }

    pub fn total_text(&self) -> String {
        signed(self.view.total)
    }

    /// The verdict's word and look, when the roll has a verdict.
    pub fn verdict(&self) -> Option<(&'static str, Tone)> {
        self.view.outcome.map(outcome_word)
    }

    /// The mark of a natural 1 or 20, only when the server says so.
    pub fn natural_word(&self) -> Option<&'static str> {
        match self.view.natural {
            NaturalMark::One => Some("Natural 1"),
            NaturalMark::Twenty => Some("Natural 20"),
            NaturalMark::Unmarked => None,
        }
    }

    /// The mark of a Heroic Inspiration reroll, when a die was rerolled.
    pub fn inspiration_word(&self) -> Option<String> {
        self.view
            .inspiration_reroll
            .map(|reroll| format!("Heroic Inspiration, was {}", reroll.was))
    }

    /// The dice on the tray, left to right: each die of the roll in the server's order, a
    /// percentile die as its tens and ones, and the die a Heroic Inspiration reroll replaced
    /// standing before the die that took its place.
    pub fn slots(&self) -> Vec<DieSlot> {
        let mut slots = Vec::new();
        for (index, die) in self.view.dice.iter().enumerate() {
            let Some((art, percentile)) = DieArt::of_sides(die.sides) else {
                continue;
            };
            if let Some(reroll) = self.view.inspiration_reroll.filter(|r| r.index == index) {
                push_die(&mut slots, art, percentile, reroll.was, DieState::Replaced);
            }
            let state = if die.kept {
                DieState::Counted
            } else {
                DieState::Unused
            };
            push_die(&mut slots, art, percentile, die.face, state);
        }
        slots
    }

    /// How the dice lie on the tray.
    pub fn layout(&self) -> TrayLayout {
        TrayLayout::of(&self.slots())
    }

    /// The sum, once: the kept dice and then the modifiers.
    pub fn terms(&self) -> Vec<Term> {
        let primary = self.view.dice.first().map(|die| die.role);
        let dice = self
            .view
            .dice
            .iter()
            .filter(|die| die.kept)
            .flat_map(|die| die_terms(die, primary));
        let modifiers = self.view.modifiers.iter().map(|modifier| Term {
            negative: modifier.value < 0,
            number: modifier.value.unsigned_abs().to_string(),
            label: Some(modifier.label.trim().to_owned()),
        });
        dice.chain(modifiers).collect()
    }

    /// Whether the sum is written smaller: it has many terms or long names.
    pub fn sum_is_long(&self) -> bool {
        let terms = self.terms();
        let characters: usize = terms
            .iter()
            .map(|term| {
                term.number.chars().count()
                    + term
                        .label
                        .as_deref()
                        .map_or(0, |label| label.chars().count())
                    + 4
            })
            .sum();
        terms.len() > TERMS_BEFORE_SMALLER_SUM || characters > SUM_CHARACTERS_BEFORE_SMALLER
    }

    /// How many lines the sum takes in the width of the card: terms wrap only between one term
    /// and the next, and the "= total" ending stays whole. An estimate from the stylesheet's
    /// sizes, worked in tenths of a cqw.
    fn sum_lines(&self) -> u32 {
        let font = if self.sum_is_long() {
            SUM_FONT_LONG_TENTHS
        } else {
            SUM_FONT_TENTHS
        };
        let label_font = TERM_LABEL_FONT_TENTHS;
        let ending = |chars: u32| font * 7 / 10 + SUM_PART_GAP_TENTHS + chars * font * 55 / 100;
        let mut widths: Vec<u32> = self
            .terms()
            .iter()
            .enumerate()
            .map(|(index, term)| {
                let sign = if index > 0 || term.negative {
                    font * 7 / 10 + SUM_PART_GAP_TENTHS
                } else {
                    0
                };
                let label = term.label.as_deref().map_or(0, |label| {
                    SUM_PART_GAP_TENTHS + character_count(label) * label_font * 70 / 100
                });
                sign + character_count(&term.number) * font * 55 / 100 + label
            })
            .collect();
        widths.push(ending(character_count(&self.total_text())));
        let mut lines = 1;
        let mut used = 0;
        for width in widths {
            if used > 0 && used + SUM_TERM_GAP_TENTHS + width > CARD_CONTENT_WIDTH_TENTHS {
                lines += 1;
                used = width;
            } else {
                used += if used > 0 { SUM_TERM_GAP_TENTHS } else { 0 } + width;
            }
        }
        lines
    }

    /// How tall the kicker and the chips beside it are: they wrap when they do not fit one row, a
    /// row is as tall as its tallest mark, and a chip too long for a row breaks onto lines.
    fn marks_height_tenths(&self) -> u32 {
        let chips = [
            self.mode_word().map(str::to_owned),
            self.inspiration_word(),
            self.natural_word().map(str::to_owned),
        ];
        let marks = self
            .kicker()
            .map(|kicker| {
                (
                    character_count(kicker) * KICKER_CHARACTER_TENTHS,
                    MARKS_ROW_TENTHS,
                )
            })
            .into_iter()
            .chain(chips.iter().flatten().map(|chip| {
                let width = character_count(chip) * CHIP_CHARACTER_TENTHS + CHIP_PADDING_TENTHS;
                let lines = width.div_ceil(CARD_CONTENT_WIDTH_TENTHS);
                (
                    width.min(CARD_CONTENT_WIDTH_TENTHS),
                    lines * CHIP_LINE_TENTHS + CHIP_FIXED_TENTHS,
                )
            }));
        let (mut height, mut row, mut used) = (0, MARKS_ROW_TENTHS, 0);
        for (width, mark) in marks {
            if used > 0 && used + MARKS_GAP_TENTHS + width > CARD_CONTENT_WIDTH_TENTHS {
                height += row + MARKS_ROW_GAP_TENTHS;
                row = MARKS_ROW_TENTHS.max(mark);
                used = width;
            } else {
                used += if used > 0 { MARKS_GAP_TENTHS } else { 0 } + width;
                row = row.max(mark);
            }
        }
        height + row
    }

    /// Whether the total has so many characters that it is written smaller, to keep its box
    /// beside the target.
    pub fn total_is_wide(&self) -> bool {
        character_count(&self.total_text()) > TOTAL_CHARACTERS_BEFORE_SMALLER
    }

    /// Whether the total is set smaller beside a long target, so the target's words keep whole.
    pub fn target_is_long(&self) -> bool {
        self.target_text()
            .is_some_and(|target| character_count(&target) > TARGET_CHARACTERS_BEFORE_SMALLER_TOTAL)
    }

    /// How tall the box of the total and the target is: the total's, or the target's pill when
    /// its words wrap onto more lines.
    fn total_box_tenths(&self) -> u32 {
        let Some(target) = self.target_text() else {
            return TOTAL_BOX_TENTHS;
        };
        let number_font = if self.target_is_long() || self.total_is_wide() {
            TOTAL_FONT_SMALL_TENTHS
        } else {
            TOTAL_FONT_TENTHS
        };
        let pill_padding = if self.target_is_long() {
            PILL_SIDE_LONG_TENTHS
        } else {
            PILL_SIDE_TENTHS
        };
        let inner = CARD_CONTENT_WIDTH_TENTHS
            .saturating_sub(2 * TOTAL_BOX_SIDE_TENTHS + TOTAL_BOX_GAP_TENTHS)
            .saturating_sub(character_count(&self.total_text()) * number_font * 55 / 100)
            .saturating_sub(2 * pill_padding)
            .max(1);
        let (mut lines, mut used) = (1, 0);
        for word in target.split_whitespace() {
            let width = character_count(word) * PILL_CHARACTER_TENTHS;
            if used > 0 && used + PILL_SPACE_TENTHS + width > inner {
                lines += 1;
                used = 0;
            }
            lines += width.saturating_sub(1) / inner;
            used += if used > 0 { PILL_SPACE_TENTHS } else { 0 } + width % inner.max(1);
        }
        (PILL_FIXED_TENTHS + lines * PILL_LINE_TENTHS + TOTAL_BOX_PADDING_TENTHS)
            .max(TOTAL_BOX_TENTHS)
    }

    /// How tall the card is, in tenths of a cqw, without the stamp that hangs below it: worked
    /// out from the stylesheet's sizes, so a card that would run off the screen is known before it
    /// is drawn.
    pub fn height_tenths(&self) -> u32 {
        let (title_font, line_characters) = if self.title_is_long() {
            (TITLE_FONT_LONG_TENTHS, TITLE_LINE_CHARACTERS_LONG)
        } else {
            (TITLE_FONT_TENTHS, TITLE_LINE_CHARACTERS)
        };
        let title_lines = character_count(self.title())
            .div_ceil(line_characters)
            .clamp(1, 2);
        let head = self.marks_height_tenths() + title_lines * title_font * 11 / 10;
        let slots = self.slots();
        let tray = match self.layout() {
            TrayLayout::Dice(grid) => Some(grid.tray_height_tenths()),
            TrayLayout::Tally(_) => Some(TALLY_TRAY_TENTHS),
        }
        .filter(|_| !slots.is_empty());
        let sum = if self.writes_a_sum() {
            let font = if self.sum_is_long() {
                SUM_FONT_LONG_TENTHS
            } else {
                SUM_FONT_TENTHS
            };
            let lines = self.sum_lines();
            lines * font * 12 / 10 + (lines - 1) * 2
        } else {
            0
        };
        let sections = 2 + u32::from(tray.is_some());
        CARD_PADDING_TENTHS
            + head
            + tray.unwrap_or(0)
            + self.total_box_tenths()
            + sum
            + SECTION_GAP_TENTHS * sections
    }

    /// How much of its size the card keeps, in percent: all of it, unless the card with its
    /// stamp would run past the foot of the screen, and then it is drawn smaller to fit.
    pub fn fit_percent(&self) -> u32 {
        let tall = self.height_tenths() + STAMP_HANG_TENTHS;
        let room = STAGE_HEIGHT_TENTHS - CARD_TOP_TENTHS - SCREEN_FOOT_MARGIN_TENTHS;
        if tall <= room {
            100
        } else {
            (room * 100 / tall).max(LEAST_FIT_PERCENT)
        }
    }

    /// Whether the sum is worth writing: a lone number is the total already.
    pub fn writes_a_sum(&self) -> bool {
        self.terms().len() > 1
    }

    /// The card in one sentence, for the page's accessibility tree and the log.
    pub fn summary(&self) -> String {
        let mut sentence = format!("{}: ", self.title());
        let terms = self.terms();
        if terms.len() > 1 {
            for (index, term) in terms.iter().enumerate() {
                if index > 0 {
                    sentence.push_str(if term.negative { " minus " } else { " plus " });
                } else if term.negative {
                    sentence.push_str("minus ");
                }
                sentence.push_str(&term.number);
                if let Some(label) = &term.label {
                    sentence.push(' ');
                    sentence.push_str(label);
                }
            }
            sentence.push_str(" makes ");
        }
        sentence.push_str(&self.total_text());
        if let Some(target) = self.target_text() {
            sentence.push_str(" against ");
            sentence.push_str(&target);
        }
        if let Some((word, _)) = self.verdict() {
            sentence.push_str(". ");
            sentence.push_str(word);
        }
        sentence
    }
}

fn push_die(slots: &mut Vec<DieSlot>, art: DieArt, percentile: bool, face: u8, state: DieState) {
    let halves = if percentile {
        let (tens, ones) = percentile_numerals(face);
        vec![(tens, Some("tens")), (ones, Some("ones"))]
    } else {
        vec![(face.to_string(), None)]
    };
    slots.extend(halves.into_iter().map(|(numeral, caption)| DieSlot {
        art,
        numeral,
        state,
        caption,
    }));
}

/// The terms a die adds to the sum: its face, or a percentile die's tens and ones, so "57" reads
/// "50 + 7" and a face of 100 reads "00 + 0".
fn die_terms(die: &RollDie, primary: Option<DieRole>) -> Vec<Term> {
    let label = (Some(die.role) != primary).then(|| role_word(die.role).to_owned());
    let numbers = if die.sides == PERCENTILE_SIDES {
        let (tens, ones) = percentile_numerals(die.face);
        vec![tens, ones]
    } else {
        vec![die.face.to_string()]
    };
    numbers
        .into_iter()
        .map(|number| Term {
            negative: false,
            number,
            label: label.clone(),
        })
        .collect()
}

/// The custom properties every animation of the card reads, for the element that holds the card:
/// `age_ms` is how far along the card already is, and each stage's animation starts at its own
/// time minus the age, so a card drawn late carries on.
pub fn staging_vars(schedule: RollSchedule, age_ms: u64) -> String {
    format!(
        concat!(
            "--roll-age:{}ms;--roll-call-at:0ms;--roll-tumble-at:{}ms;--roll-tumble:{}ms;",
            "--roll-faces-at:{}ms;--roll-faces:{}ms;--roll-sum-at:{}ms;--roll-sum:{}ms;",
            "--roll-verdict-at:{}ms;--roll-off-at:{}ms;--roll-exit:{}ms;",
        ),
        age_ms,
        schedule.tumble_start_ms(),
        schedule.tumble_ms(),
        schedule.tumble_end_ms(),
        ROLL_FACES_MS,
        schedule.breakdown_ms(),
        ROLL_BREAKDOWN_MS,
        schedule.verdict_ms(),
        schedule.off_ms(),
        ROLL_EXIT_MS,
    )
}

/// One die's own custom properties: where its numeral goes and how large the picture is drawn
/// (`FaceGeometry`), and, when the card is staged, the path of its tumble: where it comes in from,
/// how it spins and where it lies, in percent of its own size and degrees, all from the plan.
pub fn die_vars(art: DieArt, tumble: Option<&DieTumble>) -> String {
    let geometry = art.geometry();
    let mut vars = format!(
        "--art-url:url({});--art-scale:{};--face-x:{}%;--face-y:{}%;",
        crate::path::dice_file(&format!("{}.png", art.key())),
        f64::from(geometry.scale_percent) / 100.0,
        f64::from(geometry.x_permille) / 10.0,
        f64::from(geometry.y_permille) / 10.0,
    );
    let Some(tumble) = tumble else {
        return vars;
    };
    vars.push_str(&format!(
        concat!(
            "--from-x:{}cqw;--from-y:{}cqw;--r0:{}deg;--r3:{}deg;",
            "--rest-x:{}%;--rest-y:{}%;--hop:-{}%;--throw:{}ms;",
        ),
        tumble.from_x_cqw,
        tumble.from_y_cqw,
        tumble.rest_degrees + tumble.spin_degrees,
        tumble.rest_degrees,
        tumble.rest_x_percent,
        tumble.rest_y_percent,
        tumble.hop_percent,
        tumble.delay_ms,
    ));
    vars
}

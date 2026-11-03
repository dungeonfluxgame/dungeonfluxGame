//! The view decisions of the narration box and the party strip, as plain functions of their data.
//!
//! What the box is called, which cards the strip draws, what colour a health bar is: all of it is
//! decided here and tested natively, and the page only turns the answers into elements. None of it
//! is a game rule; every input is a value the server projected.

use crate::fixture::PartyMember;
use crate::layout::hit_point_permille;
use crate::limits::{HEALTH_AMBER_PERCENT, HEALTH_RED_PERCENT};
use crate::overlay::{NarrationBox, PartyLook, Progress, SeatStrip};

/// What the narrator is called when a narration box does not say.
pub const DEFAULT_NARRATOR_LABEL: &str = "Dungeon Master";
/// The word on an open seat's card.
pub const OPEN_SEAT_WORD: &str = "Open Seat";

/// The name above a narration box's line.
pub fn narrator_label(narration: &NarrationBox) -> &str {
    narration
        .speaker
        .as_deref()
        .map(str::trim)
        .filter(|speaker| !speaker.is_empty())
        .unwrap_or(DEFAULT_NARRATOR_LABEL)
}

/// "Scene 2 of 5: Introduction", or "Scene 2 of 5" when the scene has no name.
pub fn progress_text(progress: &Progress) -> String {
    match progress
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
    {
        Some(title) => format!("Scene {} of {}: {title}", progress.index, progress.total),
        None => format!("Scene {} of {}", progress.index, progress.total),
    }
}

/// One card of the party strip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StripCard<'a> {
    /// A hero: the party member and how the scene says they look.
    Hero(&'a PartyMember, Option<&'a PartyLook>),
    /// A seat nobody sits in.
    Open,
}

/// The cards of the party strip, in order: every party member (always shown, K.13) with their look
/// when the scene gave one, then the open seats.
pub fn strip_cards<'a>(
    party: &'a [PartyMember],
    seats: Option<&'a SeatStrip>,
) -> Vec<StripCard<'a>> {
    let mut cards: Vec<StripCard<'a>> = party
        .iter()
        .map(|member| {
            let look =
                seats.and_then(|seats| seats.looks.iter().find(|look| look.name == member.name));
            StripCard::Hero(member, look)
        })
        .collect();
    let open = seats.map_or(0, |seats| usize::from(seats.open));
    cards.extend(std::iter::repeat_n(StripCard::Open, open));
    cards
}

/// "Fighter Lv. 3", "Fighter" or "Lv. 3"; `None` when there is neither.
pub fn class_line(class: Option<&str>, level: Option<u8>) -> Option<String> {
    let class = class.map(str::trim).filter(|class| !class.is_empty());
    match (class, level) {
        (Some(class), Some(level)) => Some(format!("{class} Lv. {level}")),
        (Some(class), None) => Some(class.to_owned()),
        (None, Some(level)) => Some(format!("Lv. {level}")),
        (None, None) => None,
    }
}

/// How healthy a hero looks, which sets the colour of the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthTier {
    High,
    Amber,
    Red,
}

impl HealthTier {
    pub fn key(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Amber => "amber",
            Self::Red => "red",
        }
    }
}

/// The tier of a hero's bar: red below a quarter, amber below a half, otherwise high.
pub fn health_tier(hit_points: u32, maximum: u32) -> HealthTier {
    let percent = hit_point_permille(hit_points, maximum) / 10;
    if percent < HEALTH_RED_PERCENT {
        HealthTier::Red
    } else if percent < HEALTH_AMBER_PERCENT {
        HealthTier::Amber
    } else {
        HealthTier::High
    }
}

/// A health bar's fill in percent, to one decimal.
pub fn bar_percent(value: u32, maximum: u32) -> f32 {
    f32::from(hit_point_permille(value, maximum)) / 10.0
}

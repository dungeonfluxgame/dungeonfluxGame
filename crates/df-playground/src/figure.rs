//! The inline style of a figure on the shared screen: its place, size, layer, colour and turn.
//!
//! It owns what a figure's style says and which way round a turn goes, from numbers and a checked
//! colour only: no id, label or other text of a fixture reaches a style. It never touches the page,
//! so native tests cover it; `view_tv` applies what it returns.

use crate::fixture::{ElementShape, StageElement};
use crate::layout::{stage_percent, stage_size_percent};
use crate::limits::MAXIMUM_PIVOT_PERCENT;
use crate::style::css_color;

/// A full turn is the same picture as none.
const FULL_TURN_DEGREES: i64 = 360;
const HALF_TURN_DEGREES: i64 = 180;

/// The turn to draw for a figure that is new to the page, with no earlier turn to glide from.
pub fn first_turn(item: &StageElement) -> i64 {
    i64::from(item.rotation_degrees) % FULL_TURN_DEGREES
}

/// The turn to draw when a figure already drawn at `drawn_turn` is now at `rotation_degrees`, so
/// that the browser's glide takes the short way round: from 350 to 10 is 20 degrees on, not 340
/// back. The result differs from `rotation_degrees` by whole turns and keeps growing or shrinking
/// as a figure goes round, because a drawn turn is never wrapped back (that would spin it).
pub fn next_turn(drawn_turn: i64, rotation_degrees: u16) -> i64 {
    let target = i64::from(rotation_degrees) % FULL_TURN_DEGREES;
    let clockwise = (target - drawn_turn).rem_euclid(FULL_TURN_DEGREES);
    let step = if clockwise > HALF_TURN_DEGREES {
        clockwise - FULL_TURN_DEGREES
    } else {
        clockwise
    };
    drawn_turn + step
}

/// The inline style of a figure turned `turn_degrees`: its place, size, layer and colour, and a
/// transform that keeps the figure centred on its place and turns it about its pivot.
pub fn figure_style(item: &StageElement, turn_degrees: i64) -> String {
    let (left, top) = stage_percent(item.x, item.y);
    let height = match item.shape {
        ElementShape::Circle => item.width_units,
        ElementShape::Rect => item.height_units,
    };
    let (width, height) = stage_size_percent(item.width_units, height);
    // Lower things on the stage draw over higher ones, as they would stand in front.
    let layer = (top * 10.0) as i32;
    let color = item
        .color
        .as_deref()
        .and_then(css_color)
        .map_or_else(String::new, |color| format!("background:{color};"));
    let pivot_x = item.pivot.x_percent.min(MAXIMUM_PIVOT_PERCENT);
    let pivot_y = item.pivot.y_percent.min(MAXIMUM_PIVOT_PERCENT);
    format!(
        "left:{left:.2}%;top:{top:.2}%;width:{width:.2}%;height:{height:.2}%;z-index:{layer};{color}\
         transform:translate(-50%,-50%) rotate({turn_degrees}deg);transform-origin:{pivot_x}% {pivot_y}%;"
    )
}

/// The turn of a figure's label for a figure turned `figure_turn` degrees: the same turn the other
/// way, so the two cancel and the text stays upright and readable whatever the figure does (O.13
/// rule 6). They cancel while they move together, so the label and the figure carry the same
/// glide in the theme (a test pins that).
pub fn label_turn(figure_turn: i64) -> i64 {
    -figure_turn
}

/// The inline style of a figure's label, turned back by the figure's turn.
pub fn label_style(turn_degrees: i64) -> String {
    format!(
        "transform:translateX(-50%) rotate({}deg);",
        label_turn(turn_degrees)
    )
}

/// What a figure's inline style was made from, so a frame in which nothing it depends on changed
/// writes nothing: every write of a style makes the browser work out styles again, and a moving
/// scene has a frame every 30 ms.
#[derive(Debug, Clone, PartialEq)]
pub struct FigureLook {
    x_bits: u32,
    y_bits: u32,
    width_bits: u32,
    height_bits: u32,
    shape: ElementShape,
    color: Option<String>,
    pivot: (u8, u8),
    turn: i64,
}

impl FigureLook {
    pub fn of(item: &StageElement, turn: i64) -> Self {
        Self {
            x_bits: item.x.to_bits(),
            y_bits: item.y.to_bits(),
            width_bits: item.width_units.to_bits(),
            height_bits: item.height_units.to_bits(),
            shape: item.shape,
            color: item.color.clone(),
            pivot: (item.pivot.x_percent, item.pivot.y_percent),
            turn,
        }
    }

    /// Whether `figure_style(item, turn)` would be the style this look was made for.
    pub fn matches(&self, item: &StageElement, turn: i64) -> bool {
        self.turn == turn
            && self.x_bits == item.x.to_bits()
            && self.y_bits == item.y.to_bits()
            && self.width_bits == item.width_units.to_bits()
            && self.height_bits == item.height_units.to_bits()
            && self.shape == item.shape
            && self.color == item.color
            && self.pivot == (item.pivot.x_percent, item.pivot.y_percent)
    }
}

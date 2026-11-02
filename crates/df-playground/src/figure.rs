//! The inline style of a figure on the shared screen: its place, size, layer, colour and turn.
//!
//! It owns what a figure's style says and which way round a turn goes, from numbers and a checked
//! colour only: no id, label or other text of a fixture reaches a style. It never touches the page,
//! so native tests cover it; `view_tv` applies what it returns.

use crate::fixture::{ElementShape, StageElement};
use crate::layout::{stage_percent, stage_size_percent};
use crate::style::css_color;

/// A full turn is the same picture as none.
const FULL_TURN_DEGREES: i64 = 360;
const HALF_TURN_DEGREES: i64 = 180;
const MAXIMUM_PIVOT_PERCENT: u8 = 100;

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

/// The inline style of a figure's label: turned back by the figure's turn, so the text stays
/// upright and readable whatever the figure does (O.13 rule 6).
pub fn label_style(turn_degrees: i64) -> String {
    format!("transform:translateX(-50%) rotate({}deg);", -turn_degrees)
}

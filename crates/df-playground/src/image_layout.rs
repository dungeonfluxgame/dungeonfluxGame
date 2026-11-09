//! The playground's side of the scene layout: what it draws from a `df_scene_layout` record.
//!
//! The record, the character anchor API, the cast and the chat cues live in `df-scene-layout`, the
//! one contract between the host that makes a layout, this page that draws it and a chat system
//! that reads and cues it. They are re-exported here under the names the page uses. What stays is
//! the page's own placement: where an anchored bubble's box and tail go, where a marker sits, and
//! the camera targets as stage positions. Without a layout, or when a name is not in it, an item
//! keeps its own written place or is not drawn; nothing is placed on the wrong character.

use serde::{Deserialize, Serialize};

pub use df_scene_layout::{
    CastMember, CharacterCue, CharacterPoint, CueKind, FULL_PERMILLE, HEAD_GAP_PERMILLE,
    PictureBinding, REGION_TARGET_PREFIX, RegionRef, SCHEMA_VERSION, SceneLayout as ImageLayout,
    SceneRegion as LayoutRegion, Shot, cast, percent_text, point_of, region_target, region_targets,
    shot_focus, shot_zoom_percent,
};

use crate::lobby::model::{Bubble, BubbleTone, TAIL_INSET_PERCENT};

/// A bubble's anchor: the name of a character in the backdrop's layout and the point of that
/// character its tail points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BubbleAnchor {
    pub region: String,
    /// Moves the box sideways from straight over the point, in percent of the screen's width, so
    /// the trail leans toward it; negative is left.
    #[serde(default)]
    pub shift_percent: i8,
    /// The character point the tail points at; the head by default.
    #[serde(default)]
    pub point: CharacterPoint,
}

/// The region targets of `layout` as stage elements the camera can follow. They are positions
/// only: they are handed to the camera, never drawn as figures.
pub fn region_target_elements(layout: &ImageLayout) -> Vec<crate::fixture::StageElement> {
    region_targets(layout)
        .into_iter()
        .map(|(id, (x, y))| crate::fixture::StageElement {
            label: id.clone(),
            id,
            kind: "region".to_owned(),
            x: f32::from(x) / 10.0,
            y: f32::from(y) * 56.25 / 1000.0,
            color: None,
            shape: crate::fixture::ElementShape::Rect,
            width_units: 0.0,
            height_units: 0.0,
            fields: serde_json::Map::new(),
            rotation_degrees: 0,
            pivot: crate::fixture::StagePivot::default(),
        })
        .collect()
}

/// A small item tied to a character, such as a name tag or a status mark, drawn at one of its
/// points and moved by an offset in percent of the screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterMarker {
    pub region: String,
    #[serde(default)]
    pub point: CharacterPoint,
    pub text: String,
    #[serde(default)]
    pub glyph: Option<crate::overlay::Glyph>,
    #[serde(default)]
    pub dx_percent: i8,
    #[serde(default)]
    pub dy_percent: i8,
}

/// Where `marker` is drawn, in thousandths of the screen, or `None` when the layout has no
/// character of its name (a marker without a character is not drawn).
pub fn marker_place(marker: &CharacterMarker, layout: Option<&ImageLayout>) -> Option<(u16, u16)> {
    let region = layout?.region(&marker.region)?;
    let (x, y) = point_of(region, marker.point);
    let moved = |value: u16, by: i8| {
        u16::try_from((i32::from(value) + i32::from(by) * 10).clamp(0, i32::from(FULL_PERMILLE)))
            .unwrap_or(0)
    };
    Some((moved(x, marker.dx_percent), moved(y, marker.dy_percent)))
}

/// Where an anchored bubble is drawn: its box's left edge, the height of its bottom edge (the tip
/// of its tail) from the bottom of the screen, its width, and where its tail's root and tip sit
/// along the box, and the point its tip touches. The page moves the box from this first place when
/// it would cover the screen's chrome or another bubble (`bubble_place`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchoredPlace {
    pub left_permille: u16,
    pub bottom_permille: u16,
    /// Where the tail's tip touches, in thousandths of the screen from its left and top.
    pub tip_x_permille: u16,
    pub tip_y_permille: u16,
    pub width_permille: u16,
    pub root_percent: u8,
    pub tip_percent: u8,
}

/// The widest an anchored bubble is first drawn, in percent of the screen: wide enough that a long
/// line wraps to four or five lines. The page then measures the bubble and keeps only its own width
/// (`view::bubble_layout`), so a short line hugs its words.
const ANCHORED_WIDTH_PERCENT: u16 = 31;

/// Where `bubble` goes when it is anchored to a character of `layout`; `None` when it has no
/// anchor, there is no layout, or the layout has no character of that name (the bubble keeps its
/// own place).
pub fn anchored_place(bubble: &Bubble, layout: Option<&ImageLayout>) -> Option<AnchoredPlace> {
    let anchor = bubble.anchor.as_ref()?;
    let region = layout?.region(&anchor.region)?;
    let width = u16::from(bubble.width_percent).clamp(ANCHORED_WIDTH_PERCENT, 100) * 10;
    let (point_x, point_y) = point_of(region, anchor.point);
    let head_x = point_x.min(FULL_PERMILLE);
    // Over the head the tip stops short of the hair; at any other point it touches it.
    let gap = if anchor.point == CharacterPoint::Head {
        HEAD_GAP_PERMILLE
    } else {
        0
    };
    let tip_y = point_y.min(FULL_PERMILLE).saturating_sub(gap);
    let centre = i32::from(head_x) + i32::from(anchor.shift_percent) * 10;
    let left = (centre - i32::from(width) / 2).clamp(0, i32::from(FULL_PERMILLE - width));
    let along = (i32::from(head_x) - left) * 100 / i32::from(width);
    let inset = i32::from(TAIL_INSET_PERCENT);
    Some(AnchoredPlace {
        left_permille: u16::try_from(left).unwrap_or(0),
        bottom_permille: FULL_PERMILLE - tip_y,
        tip_x_permille: head_x,
        tip_y_permille: tip_y,
        width_permille: width,
        root_percent: u8::try_from(along.clamp(inset, 100 - inset)).unwrap_or(TAIL_INSET_PERCENT),
        tip_percent: u8::try_from(along.clamp(0, 100)).unwrap_or(TAIL_INSET_PERCENT),
    })
}

/// The characters whose figures are outlined while they speak, each with the mask the outline is
/// traced from: the characters of `bubbles` that show words (a thinking indicator does not name a
/// speaker), once each in the order they speak. A character without a mask, or whose name is not
/// in the layout, has no outline.
pub fn speaker_masks<'a>(
    bubbles: &'a [Bubble],
    layout: Option<&'a ImageLayout>,
) -> Vec<(&'a str, &'a str)> {
    let mut found: Vec<(&str, &str)> = Vec::new();
    for bubble in bubbles
        .iter()
        .filter(|bubble| bubble.tone != BubbleTone::Thought)
    {
        let Some(anchor) = &bubble.anchor else {
            continue;
        };
        let Some(mask) = layout
            .and_then(|layout| layout.region(&anchor.region))
            .and_then(LayoutRegion::mask)
        else {
            continue;
        };
        if !found.iter().any(|(region, _)| *region == anchor.region) {
            found.push((anchor.region.as_str(), mask));
        }
    }
    found
}

//! Keeping the anchored bubbles clear of the screen's chrome and of each other.
//!
//! The shared screen draws a bubble over its character's head in the world layer, where the camera
//! moves it. This module measures what the browser laid out (the bubbles, and the chrome that must
//! stay clear: logo, title bar, pills, status chip, caption bars, party strip, markers), asks
//! `bubble_place` where each bubble goes and writes the answer as the bubble's own style and tail
//! side. It decides nothing itself.
//!
//! Everything is measured in the stage's layout pixels. A bubble keeps one size on the screen
//! (`--cam-scale`), so its measured box is already a screen box; a character's head is mapped
//! through the camera's resting framing, so the answer is for where the camera will rest, and the
//! bubble then travels with its head during a glide. The page asks for a placement only when the
//! bubbles, the layout of the stage or the camera's framing changed, never every frame; a bubble
//! that fits where it is stays, and a placement that did not change writes nothing.

use std::collections::HashMap;

use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Element, HtmlElement};

use crate::bubble_place::{
    BubbleRequest, Choice, Clearance, Rect, WorldBox, anchor_on_stage, class_for_side,
    place_bubbles, placed_style,
};
use crate::camera::Framing;
use crate::dom::{js_message, set_style};
use crate::lobby::model::TAIL_INSET_PERCENT;
use crate::page_log::{note_error, now_ms, with_log};
use crate::timeline::ErrorSource;

/// The bubbles of the world layer that are anchored to a character.
const BUBBLES: &str = ".tv-anchors .rp-bubble[data-character]";

/// The chrome that stays clear of the bubbles: the logo, the place name, the roleplay title bar,
/// the top bar's pills, the status chip, the caption bars, the party strip and the markers.
const KEEP_OUT: &str = ".tv-logo, .tb-logo, .tv-place, .rp-title-wrap, .tb-pills, .tv-status, \
                       .tv-caption, .rp-caption, .tv-party, .rp-marker";

/// The least distance to the stage's edge, as a fraction of its width.
const MARGIN_OF_WIDTH: f32 = 0.004;
/// The least distance to the chrome and to another bubble, as a fraction of the stage's width.
const GAP_OF_WIDTH: f32 = 0.006;
/// How far under a head a bubble with its tail up stops, as a fraction of the stage's width.
const DROP_OF_WIDTH: f32 = 0.006;
/// A tail that moved less than this along its box (a fraction of its width) did not move.
const SAME_TIP: f32 = 0.02;
/// The camera's zoom must change by this much before the bubbles are placed again.
const FRAMING_SCALE_STEP: f32 = 0.01;
/// The camera's offset, in percent of the stage, must change by this much.
const FRAMING_OFFSET_STEP: f32 = 0.5;

/// What the page keeps of the bubbles' placement between draws.
#[derive(Default)]
pub struct BubbleLayout {
    /// The side and tail of each character's bubble as last decided.
    chosen: HashMap<String, Choice>,
    /// What each character's bubble showed when last placed: its kind and its words. A bubble the
    /// page draws again with the same content is not new, so it does not pop in a second time.
    shown: HashMap<String, String>,
    /// The framing the bubbles were last placed for.
    framing: Option<Framing>,
}

/// A bubble as the page measured it.
struct Measured {
    element: Element,
    character: String,
    world: WorldBox,
    request: BubbleRequest,
}

impl BubbleLayout {
    /// Whether `framing` is far enough from the one the bubbles were placed for to place them
    /// again. A camera that follows a moving figure moves a little at a time: the bubbles wait
    /// until it has moved a step.
    pub fn framing_moved(&self, framing: &Framing) -> bool {
        self.framing.is_none_or(|placed| {
            (placed.scale - framing.scale).abs() >= FRAMING_SCALE_STEP
                || (placed.x_percent - framing.x_percent).abs() >= FRAMING_OFFSET_STEP
                || (placed.y_percent - framing.y_percent).abs() >= FRAMING_OFFSET_STEP
        })
    }

    /// Places the anchored bubbles of `stage` for the camera resting at `framing`. A stage the
    /// browser has not laid out (not on the page yet) is left for a later call.
    pub fn place(&mut self, stage: &Element, framing: &Framing) -> Result<(), JsValue> {
        let width = stage.client_width() as f32;
        let height = stage.client_height() as f32;
        let drawn_width = stage
            .dyn_ref::<HtmlElement>()
            .map_or(0, HtmlElement::offset_width);
        if width <= 0.0 || height <= 0.0 || drawn_width <= 0 {
            return Ok(());
        }
        self.framing = Some(*framing);
        let screen = stage.get_bounding_client_rect();
        let zoom = screen.width() as f32 / drawn_width as f32;
        let border = (stage.client_left() as f32, stage.client_top() as f32);
        let to_stage = |rect: &web_sys::DomRect| {
            Rect::new(
                (rect.left() - screen.left()) as f32 / zoom - border.0,
                (rect.top() - screen.top()) as f32 / zoom - border.1,
                rect.width() as f32 / zoom,
                rect.height() as f32 / zoom,
            )
        };
        let bubbles = self.measure_bubbles(stage, framing, (width, height))?;
        self.settle_repeats(&bubbles)?;
        if bubbles.is_empty() {
            self.chosen.clear();
            return Ok(());
        }
        let mut keep_out = Vec::new();
        let chrome = stage.query_selector_all(KEEP_OUT)?;
        for index in 0..chrome.length() {
            let Some(item) = chrome
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let rect = to_stage(&item.get_bounding_client_rect());
            if rect.width > 0.5 && rect.height > 0.5 {
                keep_out.push(rect);
            }
        }
        let clearance = Clearance {
            margin: width * MARGIN_OF_WIDTH,
            gap: width * GAP_OF_WIDTH,
            drop: width * DROP_OF_WIDTH,
        };
        let requests: Vec<BubbleRequest> = bubbles
            .iter()
            .map(|bubble| bubble.request.clone())
            .collect();
        let stage_rect = Rect::new(0.0, 0.0, width, height);
        let faces = all_faces(stage, framing, (width, height));
        let placements = place_bubbles(&stage_rect, &requests, &keep_out, &faces, clearance);
        let mut used = vec![false; bubbles.len()];
        let mut present = HashMap::new();
        for placement in placements {
            let Some(index) = bubbles
                .iter()
                .enumerate()
                .position(|(slot, bubble)| !used[slot] && bubble.character == placement.id)
            else {
                continue;
            };
            used[index] = true;
            let bubble = &bubbles[index];
            self.apply(bubble, &placement, clearance.drop)?;
            present.insert(placement.id.clone(), placement.choice());
        }
        self.chosen = present;
        Ok(())
    }

    /// Marks the bubbles that show what their character's bubble already showed as settled, so
    /// the pop-in (`rp-bubble-in`) plays once per new bubble and not each time the layer is drawn
    /// again for another character's cue; then remembers what is shown now.
    fn settle_repeats(&mut self, bubbles: &[Measured]) -> Result<(), JsValue> {
        let mut shown = HashMap::new();
        for bubble in bubbles {
            let content = format!(
                "{}|{}",
                bubble.element.class_name().contains("rp-tone-thought"),
                bubble.element.text_content().unwrap_or_default()
            );
            let repeated = self.shown.get(&bubble.character) == Some(&content);
            if repeated && !bubble.element.class_name().contains("rp-settled") {
                let classes = bubble.element.class_name();
                bubble
                    .element
                    .set_class_name(&format!("{classes} rp-settled"));
            }
            shown.insert(bubble.character.clone(), content);
        }
        self.shown = shown;
        Ok(())
    }

    /// Reads every anchored bubble of `stage`: where its head is on the stage for `framing`, how
    /// large it is and where it was last placed.
    fn measure_bubbles(
        &self,
        stage: &Element,
        framing: &Framing,
        (width, height): (f32, f32),
    ) -> Result<Vec<Measured>, JsValue> {
        let found = stage.query_selector_all(BUBBLES)?;
        let mut bubbles = Vec::new();
        for index in 0..found.length() {
            let Some(element) = found
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let (Some(character), Some(tip_x), Some(tip_y), Some(width_permille), Some(lean)) = (
                element.get_attribute("data-character"),
                number(&element, "data-tip-x"),
                number(&element, "data-tip-y"),
                number(&element, "data-width"),
                number(&element, "data-lean"),
            ) else {
                continue;
            };
            let Some(drawn) = element.dyn_ref::<HtmlElement>() else {
                continue;
            };
            let tip = (permille(tip_x), permille(tip_y));
            let (anchor_x, anchor_y) = anchor_on_stage(tip, framing, width, height);
            let lean = lean / 100.0;
            let face_permille = element
                .get_attribute("data-face")
                .and_then(|text| face_of(&text))
                .unwrap_or((tip.0, tip.1, tip.0, tip.1));
            let face = face_on_stage(face_permille, framing, (width, height));
            let beside = !element.class_name().contains("rp-tone-thought");
            let request = BubbleRequest {
                id: character.clone(),
                anchor_x,
                anchor_y,
                width: drawn.offset_width() as f32,
                height: drawn.offset_height() as f32,
                preferred_tip: lean,
                previous: self.chosen.get(&character).copied(),
                face,
                beside,
            };
            bubbles.push(Measured {
                world: WorldBox {
                    tip_x_permille: tip.0,
                    tip_y_permille: tip.1,
                    width_permille: permille(width_permille),
                    drop_px: 0.0,
                    root_inset_percent: f32::from(TAIL_INSET_PERCENT),
                    face_permille,
                    height_px: drawn.offset_height() as f32,
                },
                element,
                character,
                request,
            });
        }
        Ok(bubbles)
    }

    /// Writes a bubble's place when it differs from what the element shows, and notes on the page's
    /// timeline a bubble that is new or changed its side or tail.
    fn apply(
        &self,
        bubble: &Measured,
        placement: &crate::bubble_place::Placement,
        drop_px: f32,
    ) -> Result<(), JsValue> {
        let world = WorldBox {
            drop_px,
            ..bubble.world
        };
        let marker = format!(
            "{}:{:.3}:{:.1}",
            placement.side.key(),
            placement.tip_fraction,
            drop_px
        );
        if bubble.element.get_attribute("data-placed").as_deref() != Some(marker.as_str()) {
            set_style(
                &bubble.element,
                &placed_style(&world, placement.side, placement.tip_fraction),
            )?;
            bubble.element.set_attribute(
                "class",
                &class_for_side(&bubble.element.class_name(), placement.side),
            )?;
            bubble.element.set_attribute("data-placed", &marker)?;
        }
        let before = self.chosen.get(&bubble.character);
        let changed = before.is_none_or(|choice| {
            choice.side != placement.side
                || (choice.tip_fraction - placement.tip_fraction).abs() > SAME_TIP
        });
        if changed {
            let moved = before.is_some();
            let (character, side) = (placement.id.as_str(), placement.side);
            let now = now_ms();
            with_log(|log| log.note_bubble(character, side, moved, now));
        }
        Ok(())
    }
}

/// The faces of every named character of `stage`'s bubble layer, on the stage.
fn all_faces(stage: &Element, framing: &Framing, size: (f32, f32)) -> Vec<Rect> {
    let Ok(Some(layer)) = stage.query_selector(".tv-anchors") else {
        return Vec::new();
    };
    layer
        .get_attribute("data-faces")
        .map(|text| {
            text.split(';')
                .filter_map(face_of)
                .map(|face| face_on_stage(face, framing, size))
                .collect()
        })
        .unwrap_or_default()
}

/// A face as `left,top,right,bottom` thousandths of the picture.
fn face_of(text: &str) -> Option<(u16, u16, u16, u16)> {
    let mut parts = text.split(',').map(|part| part.trim().parse::<u16>());
    let face = (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    );
    parts.next().is_none().then_some(face)
}

/// A face (thousandths of the picture) as a box on the stage under the camera's resting framing.
fn face_on_stage(
    face: (u16, u16, u16, u16),
    framing: &Framing,
    (width, height): (f32, f32),
) -> Rect {
    let (left, top) = anchor_on_stage((face.0, face.1), framing, width, height);
    let (right, bottom) = anchor_on_stage((face.2, face.3), framing, width, height);
    Rect::new(left, top, (right - left).max(0.0), (bottom - top).max(0.0))
}

/// A whole-number attribute of `element`, as a number.
fn number(element: &Element, name: &str) -> Option<f32> {
    element.get_attribute(name)?.parse().ok()
}

/// Thousandths of the screen from an attribute's number.
fn permille(value: f32) -> u16 {
    value.round().clamp(0.0, 1000.0) as u16
}

/// Notes a failure to place the bubbles: a draw that cannot finish is shown in the console and
/// the page's log, not hidden.
pub fn log_failure(error: &JsValue) {
    let message = format!("could not place the bubbles: {}", js_message(error));
    web_sys::console::error_1(&JsValue::from_str(&message));
    note_error(ErrorSource::Stage, &message);
}

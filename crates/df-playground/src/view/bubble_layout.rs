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

use std::collections::{HashMap, HashSet};

use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Element, HtmlElement};

use crate::bubble_place::{
    BubbleRequest, BubbleSide, Choice, Clearance, Rect, WorldBox, anchor_on_stage, class_for_side,
    place_bubbles, placed_style,
};
use crate::camera::Framing;
use crate::clues::board_covers;
use crate::dom::{js_message, set_style};
use crate::kit::speech::TailSide;
use crate::lobby::model::TAIL_INSET_PERCENT;
use crate::page_log::{note_error, now_ms, with_log};
use crate::timeline::ErrorSource;

/// The bubbles of the world layer that are anchored to a character.
const BUBBLES: &str = ".tv-anchors .rp-bubble[data-character]";

/// The outlines of the speaking characters.
const OUTLINES: &str = ".tv-anchors .rp-outline[data-character]";

/// The narration and the caption bar: what the clue board must not cover.
const LOWER_THIRD: &str = ".tv-caption, .kit-caption";

/// The chrome that stays clear of the bubbles: the logo, the place name, the scene's title banner,
/// the top bar's pills, the status chip, the caption bars, the party strip, the markers and
/// the clue board.
const KEEP_OUT: &str = ".kit-brand, .tv-place, .rp-title-wrap, .tb-pills, .tv-status, \
                       .tv-caption, .kit-caption, .tv-party, .rp-marker, .tv-clues .kit-clues";

/// The room a bubble narrower than its frame keeps beyond its words, in layout pixels.
const BUBBLE_ROOM_PX: i32 = 2;
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
    shown: HashMap<String, (bool, String)>,
    /// The characters whose outline was drawn when last placed, so an outline drawn again for the
    /// same speaker does not fade in a second time.
    outlined: HashSet<String>,
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
        let faces = all_faces(stage, framing, (width, height));
        settle_clue_board(stage, &to_stage, &faces)?;
        let bubbles = self.measure_bubbles(stage, framing, (width, height))?;
        self.settle_repeats(&bubbles);
        self.settle_outlines(stage)?;
        if bubbles.is_empty() {
            self.chosen.clear();
            return settle_clue_board(stage, &to_stage, &faces);
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
        settle_clue_board(stage, &to_stage, &faces)
    }

    /// Marks the bubbles that show what their character's bubble already showed as settled, so
    /// the pop-in (`rp-bubble-in`) plays once per new bubble and not each time the layer is drawn
    /// again for another character's cue; then remembers what is shown now.
    // TODO(DEV-12): a browser-target test that a bubble drawn again with the same content is
    // marked settled and a changed one is not.
    fn settle_repeats(&mut self, bubbles: &[Measured]) {
        let mut shown = HashMap::new();
        for bubble in bubbles {
            let classes = bubble.element.class_name();
            let has = |wanted: &str| classes.split_whitespace().any(|class| class == wanted);
            let content = (
                has("rp-tone-thought"),
                bubble.element.text_content().unwrap_or_default(),
            );
            if self.shown.get(&bubble.character) == Some(&content) && !has("rp-settled") {
                bubble
                    .element
                    .set_class_name(&format!("{classes} rp-settled"));
            }
            shown.insert(bubble.character.clone(), content);
        }
        self.shown = shown;
    }

    /// Marks the outlines of the characters that were already outlined as settled, then
    /// remembers who is outlined now.
    fn settle_outlines(&mut self, stage: &Element) -> Result<(), JsValue> {
        let found = stage.query_selector_all(OUTLINES)?;
        let mut outlined = HashSet::new();
        for index in 0..found.length() {
            let Some(element) = found
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let Some(character) = element.get_attribute("data-character") else {
                continue;
            };
            if self.outlined.contains(&character) {
                let classes = element.class_name();
                element.set_class_name(&format!("{classes} rp-settled"));
            }
            outlined.insert(character);
        }
        self.outlined = outlined;
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
            let thought = element.class_name().contains("rp-tone-thought");
            // The kit bubble inside the frame is as wide as its words (at most the frame): that
            // width is measured, so the box is the bubble's and its tail stays on the head. A
            // bubble narrower than its frame keeps a little room, so it does not wrap again once
            // the frame is cut to it. The thinking cue is as wide as its dots.
            let cue_width = element
                .query_selector(".kit-bubble")
                .ok()
                .flatten()
                .and_then(|cue| cue.dyn_into::<HtmlElement>().ok())
                .map(|cue| {
                    let measured = cue.offset_width();
                    let room = if thought || measured >= drawn.offset_width() {
                        0
                    } else {
                        BUBBLE_ROOM_PX
                    };
                    (measured + room) as f32
                });
            let tip = (permille(tip_x), permille(tip_y));
            let (anchor_x, anchor_y) = anchor_on_stage(tip, framing, width, height);
            let lean = lean / 100.0;
            let face_permille = element
                .get_attribute("data-face")
                .and_then(|text| face_of(&text))
                .unwrap_or((tip.0, tip.1, tip.0, tip.1));
            let face = face_on_stage(face_permille, framing, (width, height));
            let beside = !thought;
            let request = BubbleRequest {
                id: character.clone(),
                anchor_x,
                anchor_y,
                width: cue_width.unwrap_or(drawn.offset_width() as f32),
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
                    width_permille: cue_width.map_or(permille(width_permille), |cue| {
                        permille(cue * 1000.0 / width)
                    }),
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
            follow_side(&bubble.element, placement.side)?;
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

/// Sets the kit bubble inside `frame` to the tail shape of the side the layout chose, so the tail
/// leaves the edge the frame's own classes name. A thinking cue has no tail edge and stays as is.
fn follow_side(frame: &Element, side: BubbleSide) -> Result<(), JsValue> {
    let Some(shape) = frame.query_selector(".kit-bubble:not(.kit-thought)")? else {
        return Ok(());
    };
    let tail = match side {
        BubbleSide::AboveHead => TailSide::Below,
        BubbleSide::BelowHead => TailSide::Above,
        BubbleSide::LeftOfFace => TailSide::Right,
        BubbleSide::RightOfFace => TailSide::Left,
    };
    let classes = shape.class_name();
    let kept: Vec<&str> = classes
        .split_whitespace()
        .filter(|class| !class.starts_with("kit-shape-tail-"))
        .collect();
    shape.set_attribute(
        "class",
        &format!("{} kit-shape-tail-{}", kept.join(" "), tail.key()),
    )
}

/// Keeps the clue board off the people and the lower third. The board, open or folded, fades out
/// (`clue-hidden`) while it covers a face, the narration or the caption bar (a folded strip also a
/// bubble), and back in once it is clear. The board never moves.
fn settle_clue_board(
    stage: &Element,
    to_stage: &dyn Fn(&web_sys::DomRect) -> Rect,
    faces: &[Rect],
) -> Result<(), JsValue> {
    let Some(layer) = stage.query_selector(".tv-clues")? else {
        return Ok(());
    };
    let Some(board) = layer.query_selector(".kit-clues")? else {
        return Ok(());
    };
    let classes = layer.class_name();
    let has = |wanted: &str| classes.split_whitespace().any(|class| class == wanted);
    let folded = has("clue-shut") || stage.query_selector(".kit-tc")?.is_some();
    let mut boxes = rects_of(stage, LOWER_THIRD, to_stage)?;
    if folded {
        boxes.extend(rects_of(stage, BUBBLES, to_stage)?);
    }
    let covers = board_covers(&to_stage(&board.get_bounding_client_rect()), faces, &boxes);
    set_class(&layer, "clue-hidden", covers)
}

/// The boxes of the elements of `stage` that match `selector`, on the stage.
fn rects_of(
    stage: &Element,
    selector: &str,
    to_stage: &dyn Fn(&web_sys::DomRect) -> Rect,
) -> Result<Vec<Rect>, JsValue> {
    let found = stage.query_selector_all(selector)?;
    let mut rects = Vec::new();
    for index in 0..found.length() {
        if let Some(item) = found
            .item(index)
            .and_then(|node| node.dyn_into::<Element>().ok())
        {
            let rect = to_stage(&item.get_bounding_client_rect());
            if rect.width > 0.5 && rect.height > 0.5 {
                rects.push(rect);
            }
        }
    }
    Ok(rects)
}

/// Adds `name` to `element`'s classes, or takes it away, writing only a change.
fn set_class(element: &Element, name: &str, on: bool) -> Result<(), JsValue> {
    let current = element.class_name();
    if current.split_whitespace().any(|class| class == name) == on {
        return Ok(());
    }
    let kept: Vec<&str> = current
        .split_whitespace()
        .filter(|class| *class != name)
        .collect();
    let next = if on {
        [kept, vec![name]].concat()
    } else {
        kept
    };
    element.set_attribute("class", &next.join(" "))
}

/// The faces of every named character of `stage`'s bubble layer, on the stage.
fn all_faces(stage: &Element, framing: &Framing, size: (f32, f32)) -> Vec<Rect> {
    let Ok(Some(layer)) = stage.query_selector(".tv-world-top") else {
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

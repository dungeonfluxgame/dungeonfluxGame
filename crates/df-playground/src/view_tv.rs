//! The shared screen: a stage with figures at their canvas positions, a caption, a status banner
//! and the party strip.
//!
//! It turns a `TvScene` and the chosen controls into page elements, and nothing else: all the
//! wording and timing come from `copy`, all the geometry from `layout`, every colour passes
//! `style::css_color` before it reaches a style and every style of a figure or the camera is made
//! from numbers by `figure` and `motion_style`. The screen is built once as a shell of named
//! pieces (`Part`); a piece is drawn into its slot and drawn again alone when only it changed. The
//! figures and the camera are not pieces: they move in place, frame by frame, and this module keeps
//! what it needs to touch only what changed.

use std::collections::HashMap;

use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element, HtmlElement};

use crate::continuity::CameraPlan;
use crate::controls::Controls;
use crate::copy::{caption_dwell_ms, status_blocks_input, status_copy, word_count};
use crate::dom::{
    append, element, js_message, now_ms, picture, set_style, text_element, with_continuity,
};
use crate::draw_plan::Part;
use crate::figure::{FigureLook, figure_style, first_turn, label_style, next_turn};
use crate::fixture::{Camera, ElementShape, PartyMember, StageElement, Status, TvScene};
use crate::layout::hit_point_permille;
use crate::media::{MediaKind, media_kind};
use crate::motion_style::{CameraAction, CameraDrawn, camera_action, camera_style};
use crate::style::css_color;

/// The parts of the shared screen, in the order they are drawn.
pub const PARTS: [Part; 7] = [
    Part::TvBackdrop,
    Part::TvGrid,
    Part::TvPlace,
    Part::TvCard,
    Part::TvStatus,
    Part::TvCaption,
    Part::TvParty,
];

fn figure_class(shape: ElementShape) -> &'static str {
    match shape {
        ElementShape::Circle => "figure figure-circle",
        ElementShape::Rect => "figure figure-rect",
    }
}

fn media_address(name: &str) -> String {
    format!("/playground/media/{name}")
}

/// A figure on the page and what was last written to it, so a frame in which nothing it depends on
/// changed writes nothing.
struct FigureNode {
    element: Element,
    label: Element,
    look: FigureLook,
    turn: i64,
    shape: ElementShape,
    kind: String,
    label_text: String,
    /// The portrait picture, when the figure has one, and the media name it shows.
    portrait: Option<(Element, String)>,
    /// The frame this figure was last found in, so figures that are gone can be told.
    frame: u64,
}

/// The figures on one stage, found by the id of the element each one draws.
#[derive(Default)]
struct Figures {
    nodes: HashMap<String, FigureNode>,
    frame: u64,
}

impl Figures {
    /// Makes the figures in `layer` match `items`: new ones are added, gone ones are removed, and
    /// the rest are updated in place, only where something changed. No id is copied for a figure
    /// that is already there.
    fn sync(
        &mut self,
        document: &Document,
        layer: &Element,
        items: &[StageElement],
    ) -> Result<(), JsValue> {
        self.frame += 1;
        for item in items {
            match self.nodes.get_mut(item.id.as_str()) {
                Some(node) => update_figure(document, node, item, self.frame)?,
                None => {
                    let node = new_figure(document, item, self.frame)?;
                    append(layer, &node.element)?;
                    self.nodes.insert(item.id.clone(), node);
                }
            }
        }
        let frame = self.frame;
        self.nodes.retain(|_, node| {
            let kept = node.frame == frame;
            if !kept {
                node.element.remove();
            }
            kept
        });
        Ok(())
    }
}

fn new_figure(document: &Document, item: &StageElement, frame: u64) -> Result<FigureNode, JsValue> {
    let element = element(document, "div", figure_class(item.shape))?;
    element.set_attribute("data-id", &item.id)?;
    let turn = first_turn(item);
    set_style(&element, &figure_style(item, turn))?;
    element.set_attribute("title", &format!("{} ({})", item.id, item.kind))?;
    let label = text_element(document, "span", "figure-label", &item.label)?;
    set_style(&label, &label_style(turn))?;
    append(&element, &label)?;
    let mut node = FigureNode {
        element,
        label,
        look: FigureLook::of(item, turn),
        turn,
        shape: item.shape,
        kind: item.kind.clone(),
        label_text: item.label.clone(),
        portrait: None,
        frame,
    };
    sync_portrait(document, &mut node, item)?;
    Ok(node)
}

/// Draws the figure's portrait, when it has one, as a picture filling the figure. A portrait that
/// is not a picture of a known type is left out, so a figure never shows a broken image. The
/// picture is kept as a known child, so no query finds it again.
fn sync_portrait(
    document: &Document,
    node: &mut FigureNode,
    item: &StageElement,
) -> Result<(), JsValue> {
    let wanted = item
        .portrait()
        .filter(|name| media_kind(name) == Some(MediaKind::Image));
    match (wanted, &mut node.portrait) {
        (Some(name), Some((image, shown))) => {
            if shown != name {
                image.set_attribute("src", &media_address(name))?;
                image.set_attribute("data-media", name)?;
                name.clone_into(shown);
            }
        }
        (Some(name), None) => {
            let image = element(document, "img", "figure-portrait")?;
            image.set_attribute("src", &media_address(name))?;
            image.set_attribute("data-media", name)?;
            image.set_attribute("alt", &item.label)?;
            append(&node.element, &image)?;
            node.portrait = Some((image, name.to_owned()));
        }
        (None, Some(_)) => {
            if let Some((image, _)) = node.portrait.take() {
                image.remove();
            }
        }
        (None, None) => {}
    }
    Ok(())
}

/// Brings an existing figure up to date without replacing it, so the browser can glide it from
/// where it was to where it is now. Only what differs is written.
fn update_figure(
    document: &Document,
    node: &mut FigureNode,
    item: &StageElement,
    frame: u64,
) -> Result<(), JsValue> {
    node.frame = frame;
    let turn = next_turn(node.turn, item.rotation_degrees);
    if !node.look.matches(item, turn) {
        set_style(&node.element, &figure_style(item, turn))?;
        if node.turn != turn {
            set_style(&node.label, &label_style(turn))?;
        }
        node.look = FigureLook::of(item, turn);
        node.turn = turn;
    }
    if node.shape != item.shape {
        node.element
            .set_attribute("class", figure_class(item.shape))?;
        node.shape = item.shape;
    }
    if node.kind != item.kind {
        node.element
            .set_attribute("title", &format!("{} ({})", item.id, item.kind))?;
        node.kind.clone_from(&item.kind);
    }
    if node.label_text != item.label {
        node.label.set_text_content(Some(&item.label));
        node.label_text.clone_from(&item.label);
    }
    sync_portrait(document, node, item)
}

/// What the page keeps of the shared screen between draws.
pub struct TvHandles {
    /// The layer the camera moves.
    world: Element,
    /// The layer the figures are in.
    layer: Element,
    figures: Figures,
    /// What was last written to the world's style.
    camera_drawn: Option<CameraDrawn>,
}

impl TvHandles {
    /// Makes the figures match `elements` and frames the world for `camera`. `elements` is `None`
    /// while the page does not know the stage yet; the figures are left as they are and the whole
    /// stage shows.
    pub fn sync(
        &mut self,
        document: &Document,
        elements: Option<&[StageElement]>,
        camera: Option<&Camera>,
    ) -> Result<(), JsValue> {
        if let Some(elements) = elements {
            self.figures.sync(document, &self.layer, elements)?;
        }
        let plan = with_continuity(|continuity| continuity.camera_plan(now_ms(), camera, elements));
        self.frame_camera(&plan)
    }

    /// Writes the camera's style when it differs from what is there. A move that is under way is
    /// only aimed at its new end, so a target that is travelling is followed without restarting
    /// the move.
    fn frame_camera(&mut self, plan: &CameraPlan) -> Result<(), JsValue> {
        match camera_action(self.camera_drawn.as_ref(), plan) {
            CameraAction::Unchanged => {}
            CameraAction::Retarget => {
                let transform = plan.to.transform();
                let html = self
                    .world
                    .dyn_ref::<HtmlElement>()
                    .ok_or_else(|| JsValue::from_str("the world layer is not an html element"))?;
                let declarations = html.style();
                declarations.set_property("--cam-to", &transform)?;
                declarations.set_property("transform", &transform)?;
            }
            CameraAction::Restyle => set_style(&self.world, &camera_style(plan))?,
        }
        self.camera_drawn = Some(CameraDrawn::of(plan));
        Ok(())
    }
}

/// An empty slot for a piece: it takes no box of its own, so its content lies in the screen as if
/// the slot were not there.
pub fn slot(document: &Document, part: Part) -> Result<Element, JsValue> {
    let slot = element(document, "div", "part")?;
    slot.set_attribute("data-part", part.key())?;
    Ok(slot)
}

/// The shared screen with its slots empty.
pub struct TvShell {
    pub section: Element,
    pub handles: TvHandles,
    pub slots: Vec<(Part, Element)>,
}

/// Builds the shared screen's shell: the stage, the world layer with its slots and figure layer,
/// and a slot for each of the other pieces.
pub fn shell(document: &Document) -> Result<TvShell, JsValue> {
    let section = element(document, "section", "device device-tv")?;
    section.set_attribute("aria-label", "The shared screen")?;
    let stage = element(document, "div", "tv-stage")?;
    // The camera moves this layer; the place, card, status, caption and party stay on the screen.
    let world = element(document, "div", "tv-world")?;
    let layer = element(document, "div", "tv-figures")?;
    let mut slots = Vec::new();
    for part in PARTS {
        let holder = slot(document, part)?;
        let parent = match part {
            Part::TvBackdrop | Part::TvGrid => &world,
            _ => &stage,
        };
        // The figures come after the backdrop and the grid, and before everything on the screen.
        if part == Part::TvPlace {
            append(&world, &layer)?;
            append(&stage, &world)?;
        }
        append(parent, &holder)?;
        slots.push((part, holder));
    }
    append(&section, &stage)?;
    Ok(TvShell {
        section,
        handles: TvHandles {
            world,
            layer,
            figures: Figures::default(),
            camera_drawn: None,
        },
        slots,
    })
}

fn grid(document: &Document) -> Result<Element, JsValue> {
    element(document, "div", "stage-grid")
}

fn status_banner(document: &Document, status: &Status) -> Result<Option<Element>, JsValue> {
    let Some(copy) = status_copy(status) else {
        return Ok(None);
    };
    let class = if status_blocks_input(status) {
        "tv-status tv-status-blocking"
    } else {
        "tv-status"
    };
    text_element(document, "div", class, &copy).map(Some)
}

fn caption(
    document: &Document,
    scene: &TvScene,
    controls: &Controls,
) -> Result<Option<Element>, JsValue> {
    let Some(caption) = &scene.caption else {
        return Ok(None);
    };
    let node = element(document, "div", "tv-caption")?;
    if let Some(speaker) = &caption.speaker {
        append(
            &node,
            &text_element(document, "div", "tv-speaker", speaker)?,
        )?;
    }
    append(
        &node,
        &text_element(document, "div", "tv-line", &caption.text)?,
    )?;
    let dwell_ms = caption_dwell_ms(word_count(&caption.text), controls.pace);
    append(
        &node,
        &text_element(
            document,
            "div",
            "tv-dwell",
            &format!(
                "stays up for at least {:.1} s",
                f64::from(dwell_ms) / 1000.0
            ),
        )?,
    )?;
    Ok(Some(node))
}

fn party_card(document: &Document, member: &PartyMember) -> Result<Element, JsValue> {
    let card = element(document, "div", "party-card")?;
    let name = text_element(document, "div", "party-name", &member.name)?;
    if let Some(color) = member.color.as_deref().and_then(css_color) {
        set_style(&name, &format!("border-color:{color};"))?;
    }
    append(&card, &name)?;
    append(
        &card,
        &text_element(
            document,
            "div",
            "party-hp",
            &format!("{} / {}", member.hit_points, member.maximum_hit_points),
        )?,
    )?;
    let bar = element(document, "div", "party-bar")?;
    let fill = element(document, "div", "party-bar-fill")?;
    let permille = hit_point_permille(member.hit_points, member.maximum_hit_points);
    set_style(&fill, &format!("width:{:.1}%;", f32::from(permille) / 10.0))?;
    append(&bar, &fill)?;
    append(&card, &bar)?;
    if !member.conditions.is_empty() {
        let chips = element(document, "div", "party-conditions")?;
        for condition in &member.conditions {
            append(&chips, &text_element(document, "span", "chip", condition)?)?;
        }
        append(&card, &chips)?;
    }
    Ok(card)
}

fn party(document: &Document, scene: &TvScene) -> Result<Option<Element>, JsValue> {
    if scene.party.is_empty() {
        return Ok(None);
    }
    let strip = element(document, "div", "tv-party")?;
    for member in &scene.party {
        append(&strip, &party_card(document, member)?)?;
    }
    Ok(Some(strip))
}

/// What goes in the slot of `part`, or `None` when the piece is empty. A part that is not of the
/// shared screen is an error: the caller asked the wrong view.
pub fn build_part(
    document: &Document,
    part: Part,
    scene: &TvScene,
    controls: &Controls,
) -> Result<Option<Element>, JsValue> {
    match part {
        Part::TvBackdrop => scene
            .backdrop
            .as_ref()
            .map(|image| picture(document, "tv-backdrop", image))
            .transpose(),
        Part::TvGrid => controls.show_grid.then(|| grid(document)).transpose(),
        Part::TvPlace => text_element(document, "div", "tv-place", &scene.place).map(Some),
        Part::TvCard => scene
            .card
            .as_ref()
            .map(|image| picture(document, "tv-card", image))
            .transpose(),
        Part::TvStatus => status_banner(document, &controls.status_for(&scene.status)),
        Part::TvCaption => caption(document, scene, controls),
        Part::TvParty => party(document, scene),
        other => Err(JsValue::from_str(&format!(
            "{} is not a piece of the shared screen",
            other.key()
        ))),
    }
}

/// Logs a failure to move the figures; a draw that cannot finish is shown in the console, not hidden.
pub fn log_sync_failure(error: &JsValue) {
    web_sys::console::error_1(&JsValue::from_str(&format!(
        "could not move the figures: {}",
        js_message(error)
    )));
}

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
use web_sys::{Document, Element, HtmlElement, HtmlImageElement};

use crate::camera::{Framing, clamped_zoom, zoom_ceiling_under};
use crate::continuity::CameraPlan;
use crate::controls::{Controls, ZoomLimit};
use crate::dom::{
    append, element, js_message, now_ms, picture, set_style, text_element, with_continuity,
};
use crate::draw_plan::Part;
use crate::figure::{FigureLook, figure_style, first_turn, label_style, next_turn};
use crate::fixture::{Camera, ElementShape, StageElement, Status, TvScene};
use crate::image_layout::{ImageLayout, region_target_elements};
use crate::kit::KitPanels;
use crate::kit::narration::Narration;
use crate::kit::status::StatusChip;
use crate::limits::MAXIMUM_ZOOM_PERCENT;
use crate::media::{MediaKind, media_kind};
use crate::motion_style::{CameraAction, CameraDrawn, camera_action, camera_style};
use crate::overlay::NarrationBox;
use crate::path::media_file;
use crate::plate_fit::PlateFit;
use crate::view::bubble_layout::BubbleLayout;
use crate::view::kit::dwell_note;

/// The parts of the shared screen, in the order they are drawn.
pub const PARTS: [Part; 15] = [
    Part::TvBackdrop,
    Part::TvGrid,
    Part::TvSam,
    Part::TvAnchors,
    Part::TvPlace,
    Part::TvCard,
    Part::TvPanels,
    Part::TvStatus,
    Part::TvPills,
    Part::TvCaption,
    Part::TvParty,
    Part::TvClues,
    Part::TvStory,
    Part::TvInventory,
    Part::TvRoll,
];

fn figure_class(shape: ElementShape) -> &'static str {
    match shape {
        ElementShape::Circle => "figure figure-circle",
        ElementShape::Rect => "figure figure-rect",
    }
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
                image.set_attribute("src", &media_file(name))?;
                image.set_attribute("data-media", name)?;
                name.clone_into(shown);
            }
        }
        (Some(name), None) => {
            let image = element(document, "img", "figure-portrait")?;
            image.set_attribute("src", &media_file(name))?;
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
    // TODO(DEV-12): a browser-target test that `top` is the stage's last child, takes the camera's
    // style with `world`, and that the leaving copy freezes both.
    /// The layer of the anchored bubbles. It follows the camera exactly as `world` does, but it is
    /// the last thing on the stage, so a bubble lies over the caption, the party strip and every
    /// other overlay and never under them (they cannot be ordered between the pieces of `world`,
    /// which is one layer for the picture and the bubbles).
    top: Element,
    /// The layer the figures are in.
    layer: Element,
    figures: Figures,
    /// What was last written to the world's style.
    camera_drawn: Option<CameraDrawn>,
    /// The slot of the backdrop, whose picture is the plate the camera zooms into.
    backdrop: Element,
    /// The zoom the plate can bear, worked out for one move and one plate width.
    ceiling: Option<ZoomCeiling>,
    /// The move whose cut zoom was last logged, so it is said once.
    clamp_logged: u64,
    /// The stage, which the anchored bubbles are measured in.
    stage: Element,
    /// Where the anchored bubbles were placed, so they move only when they must.
    bubbles: BubbleLayout,
}

/// A zoom ceiling and what it was worked out from.
#[derive(Clone, Copy)]
struct ZoomCeiling {
    serial: u64,
    plate_px: Option<(u32, u32)>,
    limit: ZoomLimit,
    percent: u16,
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
        layout: Option<&ImageLayout>,
        zoom_limit: ZoomLimit,
    ) -> Result<(), JsValue> {
        if let Some(elements) = elements {
            self.figures.sync(document, &self.layer, elements)?;
        }
        let ceiling = match camera {
            Some(camera) => self.zoom_ceiling(camera, zoom_limit),
            None => MAXIMUM_ZOOM_PERCENT,
        };
        // The camera can also aim at a character of the backdrop's layout (`region:<label>`): its
        // targets join the stage's elements for the camera only and are never drawn.
        let with_regions;
        let followed = match layout {
            Some(layout) => {
                let mut all = elements.map(<[StageElement]>::to_vec).unwrap_or_default();
                all.extend(region_target_elements(layout));
                with_regions = all;
                Some(with_regions.as_slice())
            }
            None => elements,
        };
        // The faces of the plate's cast, whoever is speaking, for the bubbles and the clue board.
        self.top.set_attribute(
            "data-faces",
            &layout
                .map(crate::view::roleplay::named_faces)
                .unwrap_or_default(),
        )?;
        let now = now_ms()?;
        let plan =
            with_continuity(|continuity| continuity.camera_plan(now, camera, followed, ceiling));
        self.frame_camera(&plan)
    }

    /// The pixel size of the backdrop's picture, once it has loaded. During a crossfade the slot
    /// holds several pictures: the newest one that has loaded counts, so the camera keeps its
    /// ceiling while the new plate loads.
    fn plate_px(&self) -> Option<(u32, u32)> {
        let layers = self.backdrop.children();
        (0..layers.length())
            .rev()
            .filter_map(|index| layers.item(index))
            .filter_map(|frame| frame.first_element_child())
            .filter_map(|image| image.dyn_into::<HtmlImageElement>().ok())
            .map(|image| (image.natural_width(), image.natural_height()))
            .find(|(width, _)| *width > 0)
    }

    /// The zoom the picture can bear for this move. A request over it is drawn at the ceiling and
    /// noted once on the camera's timeline record, not cut silently. The stage is measured only
    /// when the move or the plate changes, because reading its width makes the browser lay the
    /// page out.
    fn zoom_ceiling(&mut self, camera: &Camera, limit: ZoomLimit) -> u16 {
        let plate = self.plate_px();
        let known = self.ceiling.filter(|known| {
            known.serial == camera.serial && known.plate_px == plate && known.limit == limit
        });
        let percent = match known {
            Some(known) => known.percent,
            None => {
                let ratio = web_sys::window().map_or(1.0, |window| window.device_pixel_ratio());
                // The picture's pixels are spread over the width it is drawn at, which is the
                // stage's only when the picture fills the frame's width.
                let drawn = plate.map_or(1.0, |(width, height)| {
                    f64::from(PlateFit::of_size(width, height).width())
                });
                let stage_px =
                    (f64::from(self.world.client_width()) * ratio * drawn).round() as u32;
                let percent = zoom_ceiling_under(limit, plate.map(|(width, _)| width), stage_px);
                self.ceiling = Some(ZoomCeiling {
                    serial: camera.serial,
                    plate_px: plate,
                    limit,
                    percent,
                });
                percent
            }
        };
        if let (_, Some(allowed)) = clamped_zoom(camera.zoom_percent, percent)
            && self.clamp_logged != camera.serial
        {
            self.clamp_logged = camera.serial;
            let now = crate::page_log::now_ms();
            crate::page_log::with_log(|log| {
                log.note_camera_clamp(camera.serial, camera.zoom_percent, allowed, now)
            });
        }
        percent
    }

    /// Writes the camera's style when it differs from what is there. A move that is under way is
    /// only aimed at its new end, so a target that is travelling is followed without restarting
    /// the move.
    fn frame_camera(&mut self, plan: &CameraPlan) -> Result<(), JsValue> {
        match camera_action(self.camera_drawn.as_ref(), plan) {
            CameraAction::Unchanged => {}
            CameraAction::Retarget => {
                let transform = plan.to.transform();
                for layer in [&self.world, &self.top] {
                    let html = layer
                        .dyn_ref::<HtmlElement>()
                        .ok_or_else(|| JsValue::from_str("a world layer is not an html element"))?;
                    let declarations = html.style();
                    declarations.set_property("--cam-to", &transform)?;
                    declarations.set_property("transform", &transform)?;
                }
            }
            CameraAction::Restyle => {
                // A new move written over a move's finished animation of the same name does not
                // restart it: the picture would cut to the new end. Taking the animation off and
                // reading the layout makes the browser start the new one from its first frame.
                if self.camera_drawn.as_ref().is_some_and(|drawn| drawn.moving) {
                    for layer in [&self.world, &self.top] {
                        let html = layer.dyn_ref::<HtmlElement>().ok_or_else(|| {
                            JsValue::from_str("a world layer is not an html element")
                        })?;
                        html.style().set_property("animation", "none")?;
                        let _ = html.offset_width();
                    }
                }
                let style = camera_style(plan);
                set_style(&self.world, &style)?;
                set_style(&self.top, &style)?;
            }
        }
        self.camera_drawn = Some(CameraDrawn::of(plan));
        if self.bubbles.framing_moved(&plan.to) {
            self.place_bubbles()?;
        }
        Ok(())
    }

    /// Places the anchored bubbles clear of the screen's chrome and of each other, for the camera
    /// as it rests (or will rest, when it is moving). The page calls it when the bubbles, the
    /// stage's layout or the camera's framing changed; a placement that did not change writes
    /// nothing.
    pub fn place_bubbles(&mut self) -> Result<(), JsValue> {
        let framing = self
            .camera_drawn
            .as_ref()
            .map_or(Framing::WHOLE_STAGE, |drawn| drawn.to);
        self.bubbles.place(&self.stage, &framing)
    }
}

/// An empty slot for a piece: it takes no box of its own, so its content lies in the screen as if
/// the slot were not there.
pub fn slot(document: &Document, part: Part) -> Result<Element, JsValue> {
    let slot = element(document, "div", "part")?;
    slot.set_attribute("data-part", part.dom_marker())?;
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
    // The same camera, lying over the caption and the rest of the screen, for the bubbles.
    let top = element(document, "div", "tv-world-top")?;
    // The top left of the screen is one column, so the place, the scene's title and the clue board
    // lie under one another instead of over each other.
    let heading = element(document, "div", "tv-heading")?;
    let layer = element(document, "div", "tv-figures")?;
    let mut slots = Vec::new();
    let mut backdrop = None;
    for part in PARTS {
        let holder = slot(document, part)?;
        if part == Part::TvBackdrop {
            backdrop = Some(holder.clone());
        }
        let parent = match part {
            Part::TvBackdrop | Part::TvGrid | Part::TvSam => &world,
            Part::TvAnchors => &top,
            Part::TvPlace | Part::TvClues => &heading,
            _ => &stage,
        };
        // The figures come after the backdrop and the grid, and before everything on the screen.
        if part == Part::TvPlace {
            append(&world, &layer)?;
            append(&stage, &world)?;
            append(&stage, &heading)?;
        }
        append(parent, &holder)?;
        slots.push((part, holder));
    }
    append(&stage, &top)?;
    append(&section, &stage)?;
    Ok(TvShell {
        section,
        handles: TvHandles {
            world,
            top,
            layer,
            figures: Figures::default(),
            camera_drawn: None,
            backdrop: backdrop
                .ok_or_else(|| JsValue::from_str("the shared screen has no backdrop slot"))?,
            ceiling: None,
            clamp_logged: 0,
            stage,
            bubbles: BubbleLayout::default(),
        },
        slots,
    })
}

fn grid(document: &Document) -> Result<Element, JsValue> {
    element(document, "div", "stage-grid")
}

fn status_banner(document: &Document, status: Status) -> Result<Option<Element>, JsValue> {
    let chip = StatusChip { status };
    if chip.text().is_none() {
        return Ok(None);
    }
    crate::kit::draw::panels(
        document,
        &KitPanels {
            status: Some(chip),
            ..KitPanels::default()
        },
        None,
    )
    .map(Some)
}

fn caption(
    document: &Document,
    scene: &TvScene,
    controls: &Controls,
) -> Result<Option<Element>, JsValue> {
    if let Some(narration) = &scene.narration {
        return narration_box(document, narration).map(Some);
    }
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
    append(&node, &dwell_note(document, &caption.text, controls.pace)?)?;
    Ok(Some(node))
}

/// The narrator's box: the kit's (`kit::narration`) in the caption layer, so it keeps the caption's
/// place in the lower third and lies over the outline layer, the camera never moving it. The kit
/// has no dwell note, so there is none.
fn narration_box(document: &Document, narration: &NarrationBox) -> Result<Element, JsValue> {
    let layer = element(
        document,
        "div",
        &format!(
            "tv-caption tv-narration tv-narration-{}",
            narration.placement.key()
        ),
    )?;
    append(
        &layer,
        &crate::kit::draw::panels(
            document,
            &KitPanels {
                narration: Some(Narration::from(narration)),
                ..KitPanels::default()
            },
            None,
        )?,
    )?;
    Ok(layer)
}

fn party(document: &Document, scene: &TvScene) -> Result<Option<Element>, JsValue> {
    crate::view::seats::strip(document, scene)
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
            .map(|image| {
                let layer = picture(document, "tv-backdrop", image)?;
                // A plate with a scan is fitted at once from the scan's picture size; any other
                // when its image has loaded (`plate_fade::on_image`).
                if let Some(layout) = scene.backdrop_layout() {
                    crate::view::plate_fade::fit_backdrop(
                        &layer,
                        PlateFit::of_picture(layout.picture()),
                    )?;
                }
                Ok(layer)
            })
            .transpose(),
        Part::TvGrid => controls.show_grid.then(|| grid(document)).transpose(),
        Part::TvAnchors => crate::view::roleplay::anchored(document, scene),
        Part::TvSam => match scene.backdrop_layout() {
            Some(layout) if controls.show_sam => {
                crate::view::sam::layer(document, layout).map(Some)
            }
            _ => Ok(None),
        },
        Part::TvPlace => crate::view::brand::place(document, scene).map(Some),
        Part::TvCard => scene
            .card
            .as_ref()
            .map(|image| picture(document, "tv-card", image))
            .transpose(),
        Part::TvPanels => crate::view::overlays::panels(document, scene),
        Part::TvStatus => status_banner(document, controls.status_for(&scene.status)),
        Part::TvPills => crate::view::pills::build(document, scene),
        Part::TvCaption => caption(document, scene, controls),
        Part::TvParty => party(document, scene),
        Part::TvClues => crate::view::clues::board(document, scene),
        Part::TvStory => crate::view::story::mark(document, scene),
        Part::TvInventory => crate::view::inventory::panel(document, scene, controls),
        Part::TvRoll => crate::view::roll::card(document, scene),
        other => Err(JsValue::from_str(&format!(
            "{} is not a piece of the shared screen",
            other.dom_marker()
        ))),
    }
}

/// Logs a failure to move the figures; a draw that cannot finish is shown in the console, not
/// hidden.
pub fn log_sync_failure(error: &JsValue) {
    let message = format!("could not move the figures: {}", js_message(error));
    web_sys::console::error_1(&JsValue::from_str(&message));
    crate::page_log::note_error(crate::timeline::ErrorSource::Stage, &message);
}

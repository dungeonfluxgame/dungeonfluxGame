//! The shared screen: a stage with figures at their canvas positions, a caption, a status banner
//! and the party strip.
//!
//! It turns a `TvScene` and the chosen controls into page elements, and nothing else: all the
//! wording and timing come from `copy`, all the geometry from `layout`, and every colour passes
//! `style::css_color` before it reaches a style.

use std::collections::{BTreeMap, BTreeSet};

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::controls::Controls;
use crate::copy::{caption_dwell_ms, status_blocks_input, status_copy, word_count};
use crate::dom::{append, element, set_style, text_element};
use crate::fixture::{ElementShape, PartyMember, StageElement, Status, TvScene};
use crate::layout::{hit_point_permille, stage_percent, stage_size_percent};
use crate::style::css_color;

/// The class of the layer that holds the figures. The page finds it again to move figures in place.
pub const FIGURE_LAYER_CLASS: &str = "tv-figures";

fn figure_class(item: &StageElement) -> &'static str {
    match item.shape {
        ElementShape::Circle => "figure figure-circle",
        ElementShape::Rect => "figure figure-rect",
    }
}

/// The inline style of a figure: its place, size, layer and colour, from numbers and a checked
/// colour only.
fn figure_style(item: &StageElement) -> String {
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
    format!(
        "left:{left:.2}%;top:{top:.2}%;width:{width:.2}%;height:{height:.2}%;z-index:{layer};{color}"
    )
}

fn figure(document: &Document, item: &StageElement) -> Result<Element, JsValue> {
    let node = element(document, "div", figure_class(item))?;
    node.set_attribute("data-id", &item.id)?;
    set_style(&node, &figure_style(item))?;
    node.set_attribute("title", &format!("{} ({})", item.id, item.kind))?;
    append(
        &node,
        &text_element(document, "span", "figure-label", &item.label)?,
    )?;
    Ok(node)
}

/// Brings an existing figure up to date without replacing it, so the browser can glide it from
/// where it was to where it is now.
fn update_figure(node: &Element, item: &StageElement) -> Result<(), JsValue> {
    set_style(node, &figure_style(item))?;
    node.set_attribute("class", figure_class(item))?;
    node.set_attribute("title", &format!("{} ({})", item.id, item.kind))?;
    if let Some(label) = node.first_element_child()
        && label.text_content().as_deref() != Some(item.label.as_str())
    {
        label.set_text_content(Some(&item.label));
    }
    Ok(())
}

/// Makes the figures in `layer` match `items`: new ones are added, gone ones are removed, and the
/// rest are updated in place. The page calls it for every frame of a moving scene, so it touches
/// only what changed and never rebuilds the layer.
pub fn sync_figures(
    document: &Document,
    layer: &Element,
    items: &[StageElement],
) -> Result<(), JsValue> {
    let mut existing: BTreeMap<String, Element> = BTreeMap::new();
    let children = layer.children();
    for index in 0..children.length() {
        if let Some(child) = children.item(index)
            && let Some(id) = child.get_attribute("data-id")
        {
            existing.insert(id, child);
        }
    }
    let wanted: BTreeSet<&str> = items.iter().map(|item| item.id.as_str()).collect();
    for (id, node) in &existing {
        if !wanted.contains(id.as_str()) {
            node.remove();
        }
    }
    for item in items {
        match existing.get(&item.id) {
            Some(node) => update_figure(node, item)?,
            None => append(layer, &figure(document, item)?)?,
        }
    }
    Ok(())
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

/// Draws the shared screen.
pub fn render(
    document: &Document,
    scene: &TvScene,
    elements: &[StageElement],
    status: &Status,
    controls: &Controls,
) -> Result<Element, JsValue> {
    let frame = element(document, "section", "device device-tv")?;
    frame.set_attribute("aria-label", "The shared screen")?;
    let stage = element(document, "div", "tv-stage")?;
    append(
        &stage,
        &text_element(document, "div", "tv-place", &scene.place)?,
    )?;
    if controls.show_grid {
        append(&stage, &grid(document)?)?;
    }
    let layer = element(document, "div", FIGURE_LAYER_CLASS)?;
    sync_figures(document, &layer, elements)?;
    append(&stage, &layer)?;
    if let Some(banner) = status_banner(document, status)? {
        append(&stage, &banner)?;
    }
    if let Some(caption) = caption(document, scene, controls)? {
        append(&stage, &caption)?;
    }
    if !scene.party.is_empty() {
        let strip = element(document, "div", "tv-party")?;
        for member in &scene.party {
            append(&strip, &party_card(document, member)?)?;
        }
        append(&stage, &strip)?;
    }
    append(&frame, &stage)?;
    Ok(frame)
}

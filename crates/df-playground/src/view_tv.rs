//! The shared screen: a stage with figures at their canvas positions, a caption, a status banner
//! and the party strip.
//!
//! It turns a `TvScene` and the chosen controls into page elements, and nothing else: all the
//! wording and timing come from `copy`, all the geometry from `layout`, and every colour passes
//! `style::css_color` before it reaches a style.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::controls::Controls;
use crate::copy::{caption_dwell_ms, status_blocks_input, status_copy, word_count};
use crate::dom::{append, element, set_style, text_element};
use crate::fixture::{ElementShape, PartyMember, StageElement, Status, TvScene};
use crate::layout::{hit_point_permille, stage_percent, stage_size_percent};
use crate::style::css_color;

fn figure(document: &Document, item: &StageElement) -> Result<Element, JsValue> {
    let (left, top) = stage_percent(item.x, item.y);
    let height = match item.shape {
        ElementShape::Circle => item.width_units,
        ElementShape::Rect => item.height_units,
    };
    let (width, height) = stage_size_percent(item.width_units, height);
    let shape = match item.shape {
        ElementShape::Circle => "figure figure-circle",
        ElementShape::Rect => "figure figure-rect",
    };
    let node = element(document, "div", shape)?;
    // Lower things on the stage draw over higher ones, as they would stand in front.
    let layer = (top * 10.0) as i32;
    let color = item
        .color
        .as_deref()
        .and_then(css_color)
        .map_or_else(String::new, |color| format!("background:{color};"));
    set_style(
        &node,
        &format!(
            "left:{left:.2}%;top:{top:.2}%;width:{width:.2}%;height:{height:.2}%;z-index:{layer};{color}"
        ),
    )?;
    node.set_attribute("title", &format!("{} ({})", item.id, item.kind))?;
    append(
        &node,
        &text_element(document, "span", "figure-label", &item.label)?,
    )?;
    Ok(node)
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
    for item in elements {
        append(&stage, &figure(document, item)?)?;
    }
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

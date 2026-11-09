//! Draws a title chip (`kit::title_chip`).

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::kit::Fill;
use crate::kit::draw::{Shape, compass_rose, glass, icon, track};
use crate::kit::title_chip::TitleChip;

/// The chip, or `None` when it has no title.
pub fn draw(document: &Document, chip: &TitleChip) -> Result<Option<Element>, JsValue> {
    let Some(title) = chip.title() else {
        return Ok(None);
    };
    let (root, pane) = glass(
        document,
        Shape::Capsule,
        Fill::Blurred,
        "kit-chip kit-tone-night",
    )?;
    let mark = match chip.icon {
        Some(which) => icon(document, which, "kit-chip-icon")?,
        // The outlined compass in its ring (render 02), so the chip's mark is not the brand's star.
        None => compass_rose(document, "kit-chip-icon kit-chip-compass")?,
    };
    append(&pane, &mark)?;
    let body = element(document, "div", "kit-chip-body")?;
    let line = element(document, "div", "kit-chip-line")?;
    append(
        &line,
        &text_element(document, "span", "kit-chip-title", title)?,
    )?;
    for part in chip.parts() {
        append(
            &line,
            &text_element(document, "span", "kit-sep", "\u{2022}")?,
        )?;
        append(
            &line,
            &text_element(document, "span", "kit-chip-part", part)?,
        )?;
    }
    append(&body, &line)?;
    if let Some(subtitle) = chip.subtitle() {
        append(
            &body,
            &text_element(document, "div", "kit-chip-sub", subtitle)?,
        )?;
    }
    let steps = chip.track();
    if !steps.is_empty() {
        append(
            &body,
            &track(document, "kit-chip-track", "kit-track-dot", &steps, false)?,
        )?;
    }
    append(&pane, &body)?;
    Ok(Some(root))
}

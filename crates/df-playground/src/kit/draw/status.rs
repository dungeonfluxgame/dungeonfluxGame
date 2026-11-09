//! Draws the status banner (`kit::status`).

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, text_element};
use crate::kit::Fill;
use crate::kit::draw::{Shape, glass};
use crate::kit::status::StatusChip;

/// The banner, or `None` when all is well.
pub fn draw(document: &Document, status: &StatusChip) -> Result<Option<Element>, JsValue> {
    let Some(text) = status.text() else {
        return Ok(None);
    };
    let class = if status.blocks() {
        "kit-status kit-tone-ember"
    } else {
        "kit-status kit-tone-night"
    };
    let (root, pane) = glass(document, Shape::Capsule, Fill::Blurred, class)?;
    root.set_attribute("role", "status")?;
    append(
        &pane,
        &text_element(document, "span", "kit-status-text", &text)?,
    )?;
    Ok(Some(root))
}

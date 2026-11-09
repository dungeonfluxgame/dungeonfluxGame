//! Draws the italic caption bar (`kit::caption_bar`).

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, text_element};
use crate::kit::Fill;
use crate::kit::caption_bar::CaptionBar;
use crate::kit::draw::{Shape, glass};

/// The bar, or `None` when it has no line.
pub fn draw(document: &Document, bar: &CaptionBar) -> Result<Option<Element>, JsValue> {
    let Some(text) = bar.text() else {
        return Ok(None);
    };
    let (root, pane) = glass(
        document,
        Shape::Band,
        Fill::Blurred,
        "kit-caption kit-tone-ink",
    )?;
    append(
        &pane,
        &text_element(document, "p", "kit-caption-line", text)?,
    )?;
    Ok(Some(root))
}

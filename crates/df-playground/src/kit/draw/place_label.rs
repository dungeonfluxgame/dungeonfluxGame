//! Draws a map place label (`kit::place_label`).

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, set_style, text_element};
use crate::kit::draw::icon;
use crate::kit::place_label::PlaceLabel;

/// The label, or `None` when it has no words.
pub fn draw(document: &Document, label: &PlaceLabel) -> Result<Option<Element>, JsValue> {
    let Some(text) = label.text() else {
        return Ok(None);
    };
    let root = element(
        document,
        "div",
        &format!("kit-place kit-place-{}", label.look.key()),
    )?;
    let (x_permille, y_permille) = label.held_permille();
    set_style(
        &root,
        &format!(
            "left:{:.1}%;top:{:.1}%",
            f32::from(x_permille) / 10.0,
            f32::from(y_permille) / 10.0
        ),
    )?;
    if let Some(which) = label.glyph {
        append(&root, &icon(document, which, "kit-place-icon")?)?;
    }
    append(
        &root,
        &text_element(document, "span", "kit-place-text", text)?,
    )?;
    Ok(Some(root))
}

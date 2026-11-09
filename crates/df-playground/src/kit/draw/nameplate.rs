//! Draws the speaker nameplate (`kit::nameplate`).

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::kit::nameplate::Nameplate;

/// The nameplate, or `None` when it has no line.
pub fn draw(document: &Document, plate: &Nameplate) -> Result<Option<Element>, JsValue> {
    let Some(text) = plate.text() else {
        return Ok(None);
    };
    let root = element(document, "div", "kit-nameplate")?;
    if let Some(speaker) = plate.speaker() {
        let head = element(document, "div", "kit-np-head")?;
        append(&head, &element(document, "span", "kit-np-rule")?)?;
        append(
            &head,
            &text_element(document, "span", "kit-np-name", speaker)?,
        )?;
        append(
            &head,
            &element(document, "span", "kit-np-rule kit-np-rule-after")?,
        )?;
        append(&root, &head)?;
    }
    append(&root, &text_element(document, "p", "kit-np-line", text)?)?;
    Ok(Some(root))
}

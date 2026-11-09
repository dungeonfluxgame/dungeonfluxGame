//! Draws the section label (`kit::section_label`).

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::kit::draw::icon;
use crate::kit::section_label::SectionLabel;

/// The label, or `None` when it has no title.
pub fn draw(document: &Document, label: &SectionLabel) -> Result<Option<Element>, JsValue> {
    let Some(title) = label.title() else {
        return Ok(None);
    };
    let root = element(document, "div", "kit-section")?;
    append(&root, &element(document, "span", "kit-section-bar")?)?;
    if let Some(which) = label.icon {
        append(&root, &icon(document, which, "kit-section-icon")?)?;
    }
    let text = element(document, "div", "kit-section-text")?;
    append(
        &text,
        &text_element(document, "div", "kit-section-title", title)?,
    )?;
    if let Some(subtitle) = label.subtitle() {
        append(
            &text,
            &text_element(document, "div", "kit-section-sub", subtitle)?,
        )?;
    }
    append(&root, &text)?;
    Ok(Some(root))
}

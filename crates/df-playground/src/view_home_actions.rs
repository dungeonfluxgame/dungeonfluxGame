//! The landing page's action bar: a row of buttons, one of them the gold primary, each with its
//! drawing, its label and an optional subtitle. A function of the data (`home_nav`).
//!
//! The shared screen takes no input: a button is a picture of what the phones do. A button that
//! cannot be used now is drawn dimmed and says so to a screen reader.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::home_nav::{ActionBar, ActionButton, ActionVariant};
use crate::view_home_kit::icon;

fn button(document: &Document, number: usize, button: &ActionButton) -> Result<Element, JsValue> {
    let variant = match button.variant {
        ActionVariant::Primary => "hm-action-primary",
        ActionVariant::Secondary => "hm-action-secondary",
    };
    let state = if button.enabled { "" } else { " hm-action-off" };
    let root = element(document, "div", &format!("hm-action {variant}{state}"))?;
    root.set_attribute("role", "img")?;
    root.set_attribute("data-focus-id", &format!("actions:{number}"))?;
    let described = match &button.subtitle {
        Some(subtitle) => format!("{}, {subtitle}", button.label),
        None => button.label.clone(),
    };
    let described = if button.enabled {
        described
    } else {
        format!("{described}, not available")
    };
    root.set_attribute("aria-label", &described)?;
    let material = match button.variant {
        ActionVariant::Primary => "ui-gold",
        ActionVariant::Secondary => "ui-lit-quiet",
    };
    let face = element(document, "div", &format!("hm-action-face {material}"))?;
    if let Some(glyph) = button.glyph {
        append(&face, &icon(document, glyph, "hm-action-icon")?)?;
    }
    let words = element(document, "span", "hm-action-words")?;
    append(
        &words,
        &text_element(document, "span", "hm-action-label", &button.label)?,
    )?;
    if let Some(subtitle) = &button.subtitle {
        append(
            &words,
            &text_element(document, "span", "hm-action-subtitle", subtitle)?,
        )?;
    }
    append(&face, &words)?;
    append(&root, &face)?;
    Ok(root)
}

/// The action bar.
pub fn actions(document: &Document, bar: &ActionBar) -> Result<Element, JsValue> {
    let root = element(document, "div", "hm-actions")?;
    for (index, item) in bar.buttons.iter().enumerate() {
        append(&root, &button(document, index + 1, item)?)?;
    }
    Ok(root)
}

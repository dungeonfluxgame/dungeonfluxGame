//! Small helpers for building page elements. They hold no state.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

pub fn document() -> Result<Document, JsValue> {
    web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| JsValue::from_str("the page has no document"))
}

/// A new element with a class list (space separated, may be empty).
pub fn element(document: &Document, tag: &str, class: &str) -> Result<Element, JsValue> {
    let element = document.create_element(tag)?;
    if !class.is_empty() {
        element.set_attribute("class", class)?;
    }
    Ok(element)
}

/// A new element holding only text. Text is set as text, never as markup.
pub fn text_element(
    document: &Document,
    tag: &str,
    class: &str,
    text: &str,
) -> Result<Element, JsValue> {
    let element = element(document, tag, class)?;
    element.set_text_content(Some(text));
    Ok(element)
}

pub fn append(parent: &Element, child: &Element) -> Result<(), JsValue> {
    parent.append_child(child).map(|_| ())
}

/// Sets the inline style from numbers and validated colours only (callers never pass text from a
/// fixture here unchecked).
pub fn set_style(element: &Element, css: &str) -> Result<(), JsValue> {
    element.set_attribute("style", css)
}

/// Marks a control with the action a click or a change on it asks for.
pub fn set_action(element: &Element, action: &str) -> Result<(), JsValue> {
    element.set_attribute("data-action", action)
}

pub fn clear(element: &Element) {
    element.set_text_content(None);
}

//! Small helpers for building page elements. They hold no state.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::fixture::ImageRef;
use crate::media::{MediaKind, media_kind};

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

/// A picture from the media library, with its caption under it, in an element of class `class`.
/// A name that is not a picture of a known type is shown as a note instead of a broken image, so
/// the mistake is visible. The image says which media file it is, so the page can report which
/// pictures loaded.
pub fn picture(document: &Document, class: &str, image: &ImageRef) -> Result<Element, JsValue> {
    let frame = element(document, "div", class)?;
    if media_kind(&image.media) != Some(MediaKind::Image) {
        append(
            &frame,
            &text_element(
                document,
                "div",
                "media-missing",
                &format!("'{}' is not a picture the playground can show", image.media),
            )?,
        )?;
        return Ok(frame);
    }
    let picture = element(document, "img", &format!("{class}-image"))?;
    picture.set_attribute("src", &format!("/playground/media/{}", image.media))?;
    picture.set_attribute("data-media", &image.media)?;
    picture.set_attribute("alt", image.caption.as_deref().unwrap_or(&image.media))?;
    append(&frame, &picture)?;
    if let Some(caption) = &image.caption {
        append(
            &frame,
            &text_element(document, "div", &format!("{class}-caption"), caption)?,
        )?;
    }
    Ok(frame)
}

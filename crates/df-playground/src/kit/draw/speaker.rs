//! Draws the off-picture speaker card (`kit::speaker`).

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, set_style, text_element};
use crate::kit::Fill;
use crate::kit::draw::{Shape, glass, thinking_dots};
use crate::kit::speaker::SpeakerPanel;
use crate::path::media_file;
use crate::view::kit::portrait;

pub fn draw(document: &Document, card: &SpeakerPanel) -> Result<Element, JsValue> {
    let (root, pane) = glass(
        document,
        Shape::Octagon,
        Fill::Blurred,
        &format!(
            "kit-speaker kit-tone-{}{}",
            card.rim_tone().key(),
            if card.twin { " kit-speaker-twin" } else { "" }
        ),
    )?;
    append(&pane, &face(document, card)?)?;
    let body = element(document, "div", "kit-speaker-body")?;
    append(
        &body,
        &text_element(document, "div", "kit-speaker-name", card.name())?,
    )?;
    match card.line() {
        Some(line) => append(
            &body,
            &text_element(document, "p", "kit-speaker-line", line)?,
        )?,
        None => {
            body.set_attribute("aria-label", &format!("{} is thinking", card.name()))?;
            append(&body, &thinking_dots(document)?)?;
        }
    }
    append(&pane, &body)?;
    Ok(root)
}

/// The round face: the cut of a room picture round the head when the card has one (sized and
/// moved as `PortraitCrop::frame` measures), otherwise the whole picture or the initial.
fn face(document: &Document, card: &SpeakerPanel) -> Result<Element, JsValue> {
    let (Some(media), Some(crop)) = (card.portrait.as_deref(), card.crop) else {
        return portrait(document, "kit-face", card.portrait.as_deref(), card.name());
    };
    let frame = element(document, "div", "kit-face kit-face-cut")?;
    let image = element(document, "img", "kit-face-cut-image")?;
    image.set_attribute("src", &media_file(media))?;
    image.set_attribute("alt", card.name())?;
    let (width, height, left, top) = crop.frame();
    set_style(
        &image,
        &format!("width:{width:.2}%;height:{height:.2}%;left:{left:.2}%;top:{top:.2}%;"),
    )?;
    append(&frame, &image)?;
    Ok(frame)
}

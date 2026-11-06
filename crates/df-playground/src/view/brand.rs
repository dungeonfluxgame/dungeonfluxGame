//! The top left of the shared screen: the place name, and under the DungeonFlux logo when the scene
//! asks for it.
//!
//! The logo is part of the screen's chrome, a compass star and the wordmark. It is drawn from
//! fixed shapes of this crate and never moves with the camera.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::fixture::TvScene;
use crate::overlay::Glyph;
use crate::view::kit::glyph;

/// The wordmark's text.
const WORDMARK: &str = crate::home::chrome::DEFAULT_WORDMARK;

/// The place name, with the logo above it when the scene has `brand` set.
pub fn place(document: &Document, scene: &TvScene) -> Result<Element, JsValue> {
    let place = text_element(document, "div", "tv-place", &scene.place)?;
    if !scene.brand {
        return Ok(place);
    }
    let block = element(document, "div", "tv-brandplate")?;
    let logo = element(document, "div", "tv-logo")?;
    append(&logo, &glyph(document, Glyph::Star, "tv-logo-mark")?)?;
    append(
        &logo,
        &text_element(document, "span", "tv-logo-word", WORDMARK)?,
    )?;
    append(&block, &logo)?;
    append(&block, &place)?;
    Ok(block)
}

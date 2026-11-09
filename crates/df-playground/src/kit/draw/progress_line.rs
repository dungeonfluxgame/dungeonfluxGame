//! Draws the scene progress line (`kit::progress_line`).

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::home::glyph::{HomeGlyph, HomeIcon};
use crate::kit::draw::{icon, track};
use crate::kit::progress_line::ProgressLine;

/// The line, or `None` for a run of one scene.
pub fn draw(document: &Document, line: &ProgressLine) -> Result<Option<Element>, JsValue> {
    if !line.is_drawn() {
        return Ok(None);
    }
    let root = element(document, "div", "kit-progress")?;
    let label = element(document, "div", "kit-progress-label")?;
    if line.show_play {
        let ring = element(document, "span", "kit-play")?;
        append(
            &ring,
            &icon(document, HomeIcon::Page(HomeGlyph::Play), "kit-play-icon")?,
        )?;
        append(&label, &ring)?;
    }
    append(
        &label,
        &text_element(document, "span", "kit-progress-count", &line.count_text())?,
    )?;
    if let Some(title) = line.title() {
        append(
            &label,
            &text_element(document, "span", "kit-sep", "\u{2022}")?,
        )?;
        append(
            &label,
            &text_element(document, "span", "kit-progress-title", title)?,
        )?;
    }
    append(&root, &label)?;
    let segments = track(
        document,
        "kit-progress-track",
        "kit-segment",
        &line.segments(),
        true,
    )?;
    segments.set_attribute("aria-hidden", "true")?;
    append(&root, &segments)?;
    if let Some(word) = line.continue_label() {
        let capsule = element(document, "span", "kit-progress-continue")?;
        append(&capsule, &element(document, "span", "kit-continue-inlay")?)?;
        append(
            &capsule,
            &text_element(document, "span", "kit-progress-continue-word", word)?,
        )?;
        append(
            &capsule,
            &icon(document, HomeIcon::Page(HomeGlyph::Play), "kit-play-icon")?,
        )?;
        append(&root, &capsule)?;
    }
    Ok(Some(root))
}

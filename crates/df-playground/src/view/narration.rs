//! The narrator's box: a glass panel in the caption layer with the speaker's name between rules,
//! the line, a continue mark and the dwell note.
//!
//! It takes the caption's place and keeps the caption's contract: it never moves with the camera,
//! nothing is drawn over it, and the dwell is the same `copy` rule. The continue mark is a hint
//! only; the shared screen takes no input.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::controls::Controls;
use crate::dom::{append, element, text_element};
use crate::overlay::NarrationBox;
use crate::overlay::rules::{narrator_label, progress_text};
use crate::view::kit::{dwell_note, glass, glyph};

/// The narration box for `narration`, in the caption layer.
pub fn narration(
    document: &Document,
    narration: &NarrationBox,
    controls: &Controls,
) -> Result<Element, JsValue> {
    let layer = element(
        document,
        "div",
        &format!(
            "tv-caption tv-narration tv-narration-{}",
            narration.placement.key()
        ),
    )?;
    let (wrapper, panel) = glass(document, "nar-wrap", "nar")?;
    if let Some(badge) = narration.badge {
        let medal = element(document, "div", "nar-badge")?;
        append(&medal, &glyph(document, badge, "nar-badge-glyph")?)?;
        append(&panel, &medal)?;
    }
    let main = element(document, "div", "nar-main")?;
    let head = element(document, "div", "nar-head")?;
    if narration.voice {
        append(
            &head,
            &glyph(document, crate::overlay::Glyph::Voice, "nar-voice")?,
        )?;
    }
    append(
        &head,
        &text_element(document, "span", "nar-speaker", narrator_label(narration))?,
    )?;
    append(&head, &element(document, "span", "nar-rule")?)?;
    if let Some(progress) = &narration.progress {
        append(
            &head,
            &text_element(document, "span", "nar-progress", &progress_text(progress))?,
        )?;
    }
    append(&main, &head)?;
    append(
        &main,
        &text_element(document, "div", "nar-text", &narration.text)?,
    )?;
    append(
        &main,
        &dwell_note(document, &narration.text, controls.pace)?,
    )?;
    append(&panel, &main)?;
    if let Some(label) = narration
        .continue_label
        .as_deref()
        .filter(|label| !label.trim().is_empty())
    {
        let hint = element(document, "div", "nar-continue")?;
        append(&hint, &element(document, "span", "nar-continue-ring")?)?;
        append(
            &hint,
            &text_element(document, "span", "nar-continue-label", label)?,
        )?;
        append(&panel, &hint)?;
    }
    append(&layer, &wrapper)?;
    Ok(layer)
}

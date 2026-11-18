//! The Act panel as elements: the narration in a glass panel, the note only this player sees, and
//! the suggested action cards with the groups and the actions that are off.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::choices::group_glyph;
use crate::copy::{PRIVATE_NOTE_TITLE, UNAVAILABLE_TITLE};
use crate::dom::{append, element, text_element};
use crate::phone_kit::act::{ActChoices, ActionCard};
use crate::phone_kit::draw::glyph;
use crate::phone_kit::glyph::PhoneGlyph;
use crate::view::kit::glyph as overlay_glyph;

/// The narration lines in one glass panel, or `None` when there are none.
pub fn narration(document: &Document, lines: &[String]) -> Result<Option<Element>, JsValue> {
    if lines.is_empty() {
        return Ok(None);
    }
    let panel = element(document, "section", "ph-narration")?;
    for line in lines {
        append(&panel, &text_element(document, "p", "ph-line", line)?)?;
    }
    Ok(Some(panel))
}

/// A note only this player can see.
pub fn private_note(document: &Document, text: &str) -> Result<Element, JsValue> {
    let note = element(document, "aside", "ph-note")?;
    append(
        &note,
        &text_element(document, "div", "ph-note-title", PRIVATE_NOTE_TITLE)?,
    )?;
    append(&note, &text_element(document, "p", "ph-note-text", text)?)?;
    Ok(note)
}

/// The private note with what the character saw as a short list under it, in their own words and
/// with no label.
pub fn private_note_seen(
    document: &Document,
    text: &str,
    seen: &[String],
) -> Result<Element, JsValue> {
    let note = private_note(document, text)?;
    if !seen.is_empty() {
        let list = element(document, "ul", "ph-note-seen")?;
        for what in seen {
            append(&list, &text_element(document, "li", "", what)?)?;
        }
        append(&note, &list)?;
    }
    Ok(note)
}

/// One action card: its mark, its label and the line that explains it.
pub fn card(document: &Document, card: &ActionCard) -> Result<Element, JsValue> {
    let button = element(
        document,
        "button",
        if card.lit {
            "ph-card ph-card-lit"
        } else {
            "ph-card"
        },
    )?;
    button.set_attribute("type", "button")?;
    let mark = element(document, "span", "ph-card-mark")?;
    if let Some(drawing) = card.glyph {
        append(&mark, &overlay_glyph(document, drawing, "ph-card-glyph")?)?;
    }
    append(&button, &mark)?;
    let text = element(document, "span", "ph-card-text")?;
    append(
        &text,
        &text_element(document, "span", "ph-card-label", &card.label)?,
    )?;
    if let Some(explain) = &card.explain {
        append(
            &text,
            &text_element(document, "span", "ph-card-explain", explain)?,
        )?;
    }
    append(&button, &text)?;
    Ok(button)
}

fn chip(document: &Document, choice: &ActionCard) -> Result<Element, JsValue> {
    let chip = text_element(document, "button", "ph-chip", &choice.label)?;
    chip.set_attribute("type", "button")?;
    Ok(chip)
}

/// The things the player may do: up to four cards, then the fixed groups as chips, then what is
/// off and why.
pub fn choices(document: &Document, choices: &ActChoices) -> Result<Element, JsValue> {
    let list = element(document, "div", "ph-choices")?;
    if !choices.suggested.is_empty() {
        let cards = element(document, "div", "ph-cards")?;
        for suggested in &choices.suggested {
            append(&cards, &card(document, suggested)?)?;
        }
        if choices.left_out > 0 {
            append(
                &cards,
                &text_element(
                    document,
                    "div",
                    "ph-more-note",
                    &format!("{} more ideas are waiting", choices.left_out),
                )?,
            )?;
        }
        append(&list, &cards)?;
    }
    for group in &choices.groups {
        let section = element(document, "section", "ph-group")?;
        let name = element(document, "h3", "ph-group-name")?;
        append(
            &name,
            &overlay_glyph(document, group_glyph(&group.name), "ph-group-glyph")?,
        )?;
        append(&name, &text_element(document, "span", "", &group.name)?)?;
        append(&section, &name)?;
        let row = element(document, "div", "ph-chips")?;
        for choice in &group.choices {
            append(&row, &chip(document, choice)?)?;
        }
        append(&section, &row)?;
        append(&list, &section)?;
    }
    if !choices.unavailable.is_empty() {
        let section = element(document, "section", "ph-group ph-group-off")?;
        let name = element(document, "h3", "ph-group-name")?;
        append(
            &name,
            &glyph(document, PhoneGlyph::Warning, "ph-group-glyph")?,
        )?;
        append(
            &name,
            &text_element(document, "span", "", UNAVAILABLE_TITLE)?,
        )?;
        append(&section, &name)?;
        for off in &choices.unavailable {
            let row = element(document, "div", "ph-off")?;
            append(
                &row,
                &text_element(document, "span", "ph-off-label", &off.label)?,
            )?;
            append(
                &row,
                &text_element(document, "span", "ph-off-reason", &off.reason)?,
            )?;
            append(&section, &row)?;
        }
        append(&list, &section)?;
    }
    Ok(list)
}

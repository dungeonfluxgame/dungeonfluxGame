//! Draws the restyled clue board (`kit::clue_board`). Only the board's pane blurs; the notes are
//! plain tiles inside it.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::clues::{NoteLook, PlacedNote};
use crate::dom::{append, element, text_element};
use crate::kit::Fill;
use crate::kit::clue_board::{KitClues, note_marker};
use crate::kit::draw::{Shape, glass};
use crate::overlay::Glyph;
use crate::view::kit::{glyph, photo};

/// The board, or `None` when it has no notes.
pub fn draw(document: &Document, clues: &KitClues) -> Result<Option<Element>, JsValue> {
    let notes = clues.notes();
    if notes.is_empty() {
        return Ok(None);
    }
    let (root, pane) = glass(
        document,
        Shape::Octagon,
        Fill::Blurred,
        if clues.open {
            "kit-clues"
        } else {
            "kit-clues kit-clues-shut"
        },
    )?;
    let head = element(document, "div", "kit-clues-head")?;
    append(&head, &glyph(document, Glyph::Book, "kit-clues-icon")?)?;
    append(
        &head,
        &text_element(document, "span", "kit-clues-title", "Clues")?,
    )?;
    append(
        &head,
        &text_element(
            document,
            "span",
            "kit-clues-count",
            &clues.board.notes.len().to_string(),
        )?,
    )?;
    append(&pane, &head)?;
    let list = element(document, "div", "kit-notes")?;
    list.set_attribute("role", "list")?;
    for placed in &notes {
        append(&list, &note_element(document, placed)?)?;
    }
    append(&pane, &list)?;
    Ok(Some(root))
}

fn note_element(document: &Document, placed: &PlacedNote<'_>) -> Result<Element, JsValue> {
    let note = placed.note;
    let item = element(
        document,
        "div",
        &format!(
            "kit-note kit-note-{}{}",
            note_marker(placed.look),
            if note.worked_out {
                " kit-note-solved"
            } else {
                ""
            }
        ),
    )?;
    item.set_attribute("role", "listitem")?;
    if let Some(picture) = photo(
        document,
        "kit-note-picture",
        note.picture.as_deref(),
        &note.name,
    )? {
        append(&item, &picture)?;
    }
    let text = element(document, "div", "kit-note-text")?;
    let name = element(document, "div", "kit-note-name")?;
    append(&name, &text_element(document, "span", "", &note.name)?)?;
    if note.worked_out {
        append(&name, &glyph(document, Glyph::Check, "kit-note-check")?)?;
    }
    append(&text, &name)?;
    if placed.look == NoteLook::Newest {
        append(
            &text,
            &text_element(document, "p", "kit-note-detail", &note.detail)?,
        )?;
    }
    append(&item, &text)?;
    Ok(item)
}

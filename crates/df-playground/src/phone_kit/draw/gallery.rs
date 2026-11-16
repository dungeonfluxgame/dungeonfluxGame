//! The phone gallery's page: one board's sample screens side by side, each in a phone frame, with
//! its name under it. A development tool only (`phone_kit::gallery`).

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::phone_kit::draw::{ensure_style, screen};
use crate::phone_kit::gallery::{GALLERY_QUERY_KEY, PhoneGallery, board_number};

/// The board the page's address names (`?phone_kit=N`), or `None` when it names none.
pub fn page(document: &Document) -> Result<Option<Element>, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("the page has no window"))?;
    let Some(number) = board_number(&window.location().search()?) else {
        return Ok(None);
    };
    let gallery = PhoneGallery::read().map_err(|error| JsValue::from_str(&error.to_string()))?;
    let board = gallery.board(number).ok_or_else(|| {
        JsValue::from_str(&format!(
            "the phone gallery has boards 1 to {}, not {number}",
            gallery.boards.len()
        ))
    })?;
    ensure_style(document)?;
    let page = element(document, "div", "ph-gallery")?;

    let nav = element(document, "nav", "ph-gallery-nav")?;
    for (index, other) in gallery.boards.iter().enumerate() {
        let link = text_element(
            document,
            "a",
            if index + 1 == number {
                "ph-gallery-link ph-gallery-here"
            } else {
                "ph-gallery-link"
            },
            &format!("{}. {}", index + 1, other.name),
        )?;
        link.set_attribute("href", &format!("?{GALLERY_QUERY_KEY}={}", index + 1))?;
        append(&nav, &link)?;
    }
    append(&page, &nav)?;
    let mut tag = format!(
        "Phone kit board {number} of {}: {}",
        gallery.boards.len(),
        board.name
    );
    if let Some(after) = &board.after {
        tag.push_str(&format!(" (after {after})"));
    }
    append(
        &page,
        &text_element(document, "h1", "ph-gallery-tag", &tag)?,
    )?;

    let figures = element(document, "div", "ph-figures")?;
    for sample in &board.screens {
        let drawn = screen(document, &sample.panels)?;
        append(&figures, &figure(document, &sample.title, Some(&drawn))?)?;
    }
    append(&page, &figures)?;
    Ok(Some(page))
}

/// One sample of a board: its name over a phone frame holding `screen` (an empty frame for none).
pub fn figure(
    document: &Document,
    title: &str,
    screen: Option<&Element>,
) -> Result<Element, JsValue> {
    let figure = element(document, "figure", "ph-figure")?;
    append(
        &figure,
        &text_element(document, "figcaption", "ph-figure-title", title)?,
    )?;
    let frame = element(document, "div", "ph-frame")?;
    if let Some(screen) = screen {
        append(&frame, screen)?;
    }
    append(&figure, &frame)?;
    Ok(figure)
}

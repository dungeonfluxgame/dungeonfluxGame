//! The player lobby's layer on the shared screen: one box over the whole stage in which each
//! component sits in its own slot, as on the landing page.
//!
//! This module places each component's element in its slot and gives the one element the remote is
//! on the focus class; the title block is drawn here and the rest are `view_lobby_seat` (a seat's
//! card) and `view_lobby_panels` (the join code, the party summary and the begin button). A part
//! the scene does not name leaves its slot out, so a lobby can be assembled one component at a
//! time. The shared screen takes no input: every element is a picture of what the phones and the
//! remote do, never a control. Text from a scene is set as text.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::lobby_model::{
    LOBBY_EYEBROW, LobbyScreen, seat_focus_id, seat_size_key, title_size_key,
};

/// A slot named `name` holding `content`, put in `layer`.
fn put(document: &Document, layer: &Element, name: &str, content: &Element) -> Result<(), JsValue> {
    let slot = element(document, "div", &format!("pl-slot pl-slot-{name}"))?;
    append(&slot, content)?;
    append(layer, &slot)
}

/// Gives the one element whose `data-focus-id` is `id` the focus class. An id the page has no
/// element for leaves nothing focused; the id was checked to hold no character that needs quoting.
fn mark_focus(layer: &Element, id: Option<&str>) -> Result<(), JsValue> {
    let Some(id) = id.filter(|id| crate::home_model::is_focus_id(id)) else {
        return Ok(());
    };
    if let Some(target) = layer.query_selector(&format!("[data-focus-id=\"{id}\"]"))? {
        let class = target.get_attribute("class").unwrap_or_default();
        target.set_attribute("class", &format!("{class} hm-focused"))?;
    }
    Ok(())
}

/// The rule with a diamond at its middle, under the subtitle.
fn flourish(document: &Document) -> Result<Element, JsValue> {
    let row = element(document, "div", "pl-flourish")?;
    for part in ["pl-rule", "pl-diamond", "pl-rule"] {
        append(&row, &element(document, "span", part)?)?;
    }
    Ok(row)
}

/// The session's name: a line over it between two diamonds, the name in gold, its words and a rule.
fn title_block(document: &Document, lobby: &LobbyScreen) -> Result<Element, JsValue> {
    let block = element(
        document,
        "header",
        &format!("pl-title pl-title-{}", title_size_key(&lobby.title)),
    )?;
    block.set_attribute("data-component", "lobby-title")?;
    let eyebrow = element(document, "div", "pl-eyebrow")?;
    for part in ["pl-rule", "pl-diamond"] {
        append(&eyebrow, &element(document, "span", part)?)?;
    }
    let words = lobby.eyebrow.as_deref().unwrap_or(LOBBY_EYEBROW);
    append(
        &eyebrow,
        &text_element(document, "span", "pl-eyebrow-text", words)?,
    )?;
    for part in ["pl-diamond", "pl-rule"] {
        append(&eyebrow, &element(document, "span", part)?)?;
    }
    append(&block, &eyebrow)?;
    append(
        &block,
        &text_element(document, "h2", "pl-name", lobby.title.trim())?,
    )?;
    if let Some(subtitle) = lobby
        .subtitle
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        append(
            &block,
            &text_element(document, "p", "pl-subtitle", subtitle.trim())?,
        )?;
    }
    append(&block, &flourish(document)?)?;
    Ok(block)
}

/// The player lobby of `lobby`: each component the scene has, in its slot.
pub fn build(document: &Document, lobby: &LobbyScreen) -> Result<Element, JsValue> {
    let layer = element(document, "div", "pl")?;
    if !lobby.title.trim().is_empty() {
        put(document, &layer, "title", &title_block(document, lobby)?)?;
    }
    let shown = lobby.shown_seats();
    if !shown.is_empty() {
        let seats = element(
            document,
            "ul",
            &format!("pl-seats pl-seats-{}", seat_size_key(shown.len())),
        )?;
        seats.set_attribute("aria-label", "Seats at the table")?;
        for (index, seat) in shown.iter().enumerate() {
            let card = crate::view_lobby_seat::seat(document, seat, index)?;
            card.set_attribute("data-focus-id", &seat_focus_id(index + 1))?;
            append(&seats, &card)?;
        }
        put(document, &layer, "seats", &seats)?;
    }
    if let Some(join) = &lobby.join {
        put(
            document,
            &layer,
            "join",
            &crate::view_lobby_panels::join(document, join)?,
        )?;
    }
    if let Some(begin) = &lobby.begin {
        put(
            document,
            &layer,
            "begin",
            &crate::view_lobby_panels::begin(document, begin)?,
        )?;
    }
    if let Some(party) = &lobby.party {
        put(
            document,
            &layer,
            "party",
            &crate::view_lobby_panels::party(document, party)?,
        )?;
    }
    mark_focus(&layer, lobby.focus_id().as_deref())?;
    Ok(layer)
}

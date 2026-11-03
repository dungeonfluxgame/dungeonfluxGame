//! One seat card of the player lobby (renders 11 and 24, drawn for a television): a portrait or the
//! empty-seat silhouette on lit glass, the name, the class line and the seat's seal.
//!
//! A seat is in one of four states (`lobby_model::SeatState`). A locked-in hero shows the name, the
//! class and level and a gold seal; a player making a character shows the stage and a bar in
//! violet; an open seat shows a plus in a ring and the word Open; a closed seat shows a lock. The
//! host's crown sits at the top of the card. The card is a function of the seat's data: the page
//! decides nothing about the rules. The focus id is put on the element by the page.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, set_style, text_element};
use crate::lobby_model::{LobbySeat, SeatState, seat_view};
use crate::overlay::Glyph;
use crate::view_icons::{Icon, icon};
use crate::view_kit::glyph;
use crate::view_lobby::photo;

/// The card of the seat at `index` (counted from 0), as a list item.
pub fn seat(document: &Document, seat: &LobbySeat, index: usize) -> Result<Element, JsValue> {
    let view = seat_view(seat, index);
    let item = element(document, "li", &format!("pl-seat pl-seat-{}", view.key))?;
    let card = element(document, "div", "pl-card")?;
    match seat.state {
        SeatState::LockedIn | SeatState::Creating => {
            let art = element(document, "div", "pl-art")?;
            match photo(document, "pl-photo", seat.portrait.as_deref(), &view.name)? {
                Some(image) => append(&art, &image)?,
                None => append(&art, &icon(document, Icon::Silhouette, "pl-silhouette")?)?,
            }
            append(&card, &art)?;
            append(&card, &element(document, "div", "pl-shade")?)?;
            if seat.controller {
                append(&card, &icon(document, Icon::Controller, "pl-controller")?)?;
            }
            if seat.host {
                append(&card, &glyph(document, Glyph::Crown, "pl-crown")?)?;
            }
            let words = element(document, "div", "pl-words")?;
            append(
                &words,
                &text_element(document, "div", "pl-name", &view.name)?,
            )?;
            if let Some(sub_line) = &view.sub_line {
                append(&words, &sub_line_of(document, seat, sub_line)?)?;
            }
            if let Some(percent) = view.progress_percent {
                append(&words, &progress_bar(document, percent)?)?;
            }
            if let Some(badge) = &view.badge {
                append(&words, &seal(document, seat, badge)?)?;
            }
            append(&card, &words)?;
        }
        SeatState::Open | SeatState::Closed => {
            let ring = element(document, "div", "pl-ring")?;
            let mark = if seat.state == SeatState::Open {
                Glyph::Plus
            } else {
                Glyph::Lock
            };
            append(&ring, &glyph(document, mark, "pl-ring-mark")?)?;
            append(&card, &ring)?;
            let words = element(document, "div", "pl-words pl-words-open")?;
            append(
                &words,
                &text_element(document, "div", "pl-name", &view.name)?,
            )?;
            if let Some(sub_line) = &view.sub_line {
                append(
                    &words,
                    &text_element(document, "div", "pl-open-word", sub_line)?,
                )?;
            }
            append(&card, &words)?;
        }
    }
    append(&item, &card)?;
    Ok(item)
}

/// The line under a hero's name: the class with its small picture, or a maker's stage.
fn sub_line_of(document: &Document, seat: &LobbySeat, words: &str) -> Result<Element, JsValue> {
    let line = element(document, "div", "pl-line")?;
    if seat.state == SeatState::LockedIn
        && let Some(mark) = seat.glyph
    {
        append(&line, &glyph(document, mark, "pl-line-glyph")?)?;
    }
    append(&line, &text_element(document, "span", "", words)?)?;
    Ok(line)
}

/// The bar of a player making a character; the number stays in the data.
fn progress_bar(document: &Document, percent: u8) -> Result<Element, JsValue> {
    let bar = element(document, "div", "pl-bar")?;
    bar.set_attribute("role", "progressbar")?;
    bar.set_attribute("aria-valuemin", "0")?;
    bar.set_attribute("aria-valuemax", "100")?;
    bar.set_attribute("aria-valuenow", &percent.to_string())?;
    bar.set_attribute("aria-label", "Character creation progress")?;
    let fill = element(document, "div", "pl-bar-fill")?;
    set_style(&fill, &format!("width:{percent}%;"))?;
    append(&bar, &fill)?;
    Ok(bar)
}

/// The seal at the foot of a card: gold with a check for a hero who is ready, violet for a word
/// the scene gives to a player still making a character.
fn seal(document: &Document, seat: &LobbySeat, words: &str) -> Result<Element, JsValue> {
    let class = if seat.state == SeatState::LockedIn {
        "pl-seal pl-seal-ready"
    } else {
        "pl-seal pl-seal-creating"
    };
    let seal = element(document, "div", class)?;
    if seat.state == SeatState::LockedIn {
        append(&seal, &glyph(document, Glyph::Check, "pl-seal-mark")?)?;
    }
    append(&seal, &text_element(document, "span", "", words)?)?;
    Ok(seal)
}

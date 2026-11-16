//! The join screens as elements (`seat::screen::JoinScreen`), in the phone kit's regions and look:
//! the header, the status, a scrolling body (the room, the heading, the cards, the name field, the
//! notice) and a dock of buttons low under the thumb. It decides nothing; every tap is the
//! screen's `SeatTap`, carried as `data-action`.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, set_action, text_element};
use crate::phone_kit::draw::{act as kit_act, regions, shell as kit_shell};
use crate::phone_kit::shell::ButtonSpec;
use crate::seat::screen::{JoinButton, JoinCard, JoinScreen, RoomBanner};
use crate::view::kit::photo;

/// The id of the guest's name field, read when Join the table is tapped.
pub const NAME_FIELD_ID: &str = "ph-join-name";

fn room(document: &Document, banner: &RoomBanner) -> Result<Element, JsValue> {
    let card = element(document, "section", "ph-glass ph-join-room")?;
    if let Some(image) = photo(
        document,
        "ph-join-room-picture",
        banner.picture.as_deref(),
        &banner.title,
    )? {
        append(&card, &image)?;
    }
    append(
        &card,
        &text_element(document, "h1", "ph-join-room-title", &banner.title)?,
    )?;
    let joined = if banner.players.is_empty() {
        "Nobody has joined yet.".to_owned()
    } else {
        format!("At the table: {}", banner.players.join(", "))
    };
    append(
        &card,
        &text_element(document, "p", "ph-join-room-players", &joined)?,
    )?;
    Ok(card)
}

/// The kit's action card, carrying its tap, with its pill after the words.
fn card(document: &Document, join: &JoinCard) -> Result<Element, JsValue> {
    let item = kit_act::card(document, &join.card)?;
    set_action(&item, &join.tap.action())?;
    if let Some(mark) = &join.mark {
        append(&item, &kit_shell::pill(document, mark)?)?;
    }
    Ok(item)
}

fn name_field(document: &Document) -> Result<Element, JsValue> {
    let label = element(document, "label", "ph-join-field")?;
    append(
        &label,
        &text_element(document, "span", "ph-join-field-label", "Name")?,
    )?;
    let input = element(document, "input", "ph-join-name")?;
    input.set_attribute("id", NAME_FIELD_ID)?;
    input.set_attribute("type", "text")?;
    input.set_attribute("autocomplete", "nickname")?;
    input.set_attribute("enterkeyhint", "go")?;
    input.set_attribute("placeholder", "Mara Vell")?;
    append(&label, &input)?;
    Ok(label)
}

fn button(document: &Document, spec: &JoinButton) -> Result<Element, JsValue> {
    kit_shell::button(
        document,
        &ButtonSpec {
            label: spec.label.clone(),
            kind: spec.kind,
            enabled: true,
            reason: None,
            action: Some(spec.tap.action()),
        },
    )
}

fn body(document: &Document, screen: &JoinScreen) -> Result<Element, JsValue> {
    let body = element(document, "main", "ph-body ph-join-body")?;
    if let Some(banner) = &screen.room {
        append(&body, &room(document, banner)?)?;
    }
    if let Some(heading) = &screen.heading {
        append(
            &body,
            &text_element(document, "h2", "ph-join-heading", heading)?,
        )?;
    }
    if let Some(lead) = &screen.lead {
        append(&body, &text_element(document, "p", "ph-join-lead", lead)?)?;
    }
    if let Some(notice) = &screen.notice {
        append(&body, &kit_shell::status(document, notice)?)?;
    }
    if screen.name_field {
        append(&body, &name_field(document)?)?;
    }
    if !screen.cards.is_empty() {
        let cards = element(document, "div", "ph-cards ph-join-cards")?;
        for item in &screen.cards {
            append(&cards, &card(document, item)?)?;
        }
        append(&body, &cards)?;
    }
    if !screen.roster.is_empty() {
        let row = element(document, "div", "ph-pill-row")?;
        for player in &screen.roster {
            append(&row, &kit_shell::pill(document, player)?)?;
        }
        append(&body, &row)?;
    }
    if let Some(empty) = &screen.empty {
        append(&body, &kit_shell::empty(document, empty)?)?;
    }
    if let Some(loading) = &screen.loading {
        append(&body, &kit_shell::loading(document, loading)?)?;
    }
    Ok(body)
}

/// One join screen, in the kit's regions (`phone_kit::draw::regions`).
pub fn build(document: &Document, screen: &JoinScreen) -> Result<Element, JsValue> {
    let dock = element(document, "div", "ph-dock ph-join-dock")?;
    for spec in &screen.buttons {
        append(&dock, &button(document, spec)?)?;
    }
    regions(
        document,
        "ph-live ph-join",
        false,
        screen.header.as_ref(),
        screen.status.as_ref(),
        &body(document, screen)?,
        &dock,
    )
}

/// The join board of the phone gallery (`?phone_kit=join`): every join screen side by side, each
/// in a phone frame with its name, or `None` when the address names another board. A development
/// tool only.
#[cfg(debug_assertions)]
pub fn gallery_page(document: &Document) -> Result<Option<Element>, JsValue> {
    use crate::phone_kit::draw::ensure_style;
    use crate::phone_kit::draw::gallery::figure;
    use crate::seat::samples::{join_samples, names_join_board};
    use crate::seat::screen::screen_of;

    let window = web_sys::window().ok_or_else(|| JsValue::from_str("the page has no window"))?;
    if !names_join_board(&window.location().search()?) {
        return Ok(None);
    }
    ensure_style(document)?;
    let page = element(document, "div", "ph-gallery")?;
    append(
        &page,
        &text_element(
            document,
            "h1",
            "ph-gallery-tag",
            "Phone kit board: joining, from the room to a seat",
        )?,
    )?;
    let figures = element(document, "div", "ph-figures")?;
    for (title, machine) in join_samples() {
        let drawn = screen_of(&machine)
            .map(|screen| build(document, &screen))
            .transpose()?;
        append(&figures, &figure(document, title, drawn.as_ref())?)?;
    }
    append(&page, &figures)?;
    Ok(Some(page))
}

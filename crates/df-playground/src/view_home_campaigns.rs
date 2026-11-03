//! The landing page's Recent Campaigns strip (render 19, along the bottom): a row of campaign cards,
//! each an art thumbnail under a dark gradient with the title and the last-played line, the selected
//! one ringed in bright teal.
//!
//! The screen is a television read from a sofa, and the strip's box is short, so at the size that is
//! readable across a room (the `home-campaigns` block of `theme.css`) it draws only the first
//! `CARDS_SHOWN` cards and no header: the strip's title and the see-all words stay in the data (the
//! title is the strip's accessible name). Selection (which campaign is picked) and focus (where the
//! remote is) are separate: a card carries its focus id (`campaigns:2`), and the page draws whichever
//! its focus names with the focus class.
//!
//! The builder is a function of `CampaignsData`; it fills the slot box the page gives it
//! (`width: 100%; height: 100%`). Every text is set as text.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::home_campaigns::{CampaignCard, CampaignsData, card_focus_id};
use crate::view_home_kit::photo;

/// The campaigns component, or an error when the page cannot be built.
pub fn build(document: &Document, data: &CampaignsData) -> Result<Element, JsValue> {
    let panel = element(document, "div", "ui-lit-quiet hc-panel")?;
    panel.set_attribute("data-component", "home-campaigns")?;
    panel.set_attribute("role", "region")?;
    panel.set_attribute("aria-label", &data.title)?;
    let cards = element(document, "ul", "hc-cards")?;
    for (index, card) in data.shown_cards().iter().enumerate() {
        append(&cards, &campaign(document, card, index + 1)?)?;
    }
    append(&panel, &cards)?;
    Ok(panel)
}

fn campaign(document: &Document, card: &CampaignCard, number: usize) -> Result<Element, JsValue> {
    let item = element(
        document,
        "li",
        if card.selected {
            "hc-card hc-card-selected"
        } else {
            "hc-card"
        },
    )?;
    item.set_attribute("data-focus-id", &card_focus_id(number))?;
    if card.selected {
        item.set_attribute("aria-current", "true")?;
    }
    match photo(document, "hc-art", card.art.as_deref(), "")? {
        Some(image) => append(&item, &image)?,
        None => {
            let plate = element(document, "div", "hc-art hc-art-plate")?;
            plate.set_attribute("aria-hidden", "true")?;
            append(&item, &plate)?;
        }
    }
    append(&item, &element(document, "div", "hc-shade")?)?;
    let words = element(document, "div", "hc-words")?;
    if !card.title.is_empty() {
        append(
            &words,
            &text_element(document, "h3", "hc-name", &card.title)?,
        )?;
    }
    if !card.last_played.is_empty() {
        append(
            &words,
            &text_element(document, "p", "hc-last", &card.last_played)?,
        )?;
    }
    append(&item, &words)?;
    Ok(item)
}

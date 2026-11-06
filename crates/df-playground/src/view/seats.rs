//! The party strip drawn as the concept's seat cards: a glass bar holding one card per hero, each
//! with a full-bleed portrait, the hero's name with a class glyph, the class line, hit points with
//! a bar along the card's foot, and the open seats after the heroes.
//!
//! Every hero in the party is always drawn (K.13), with their hit points; the strip never moves
//! with the camera. The decisions (which cards, the class line, the bar's tier) are in
//! `overlay::rules`; this module turns them into elements.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, set_style, text_element};
use crate::fixture::{PartyMember, TvScene};
use crate::overlay::rules::{
    OPEN_SEAT_WORD, StripCard, bar_percent, class_line, health_tier, strip_cards,
};
use crate::overlay::{Glyph, PartyLook};
use crate::view::kit::{accent_style, glass, glyph, portrait};

fn hero_card(
    document: &Document,
    member: &PartyMember,
    look: Option<&PartyLook>,
) -> Result<Element, JsValue> {
    let (card, body) = glass(document, "party-card", "party-card-body")?;
    let accent = look
        .and_then(|look| look.accent.as_deref())
        .or(member.color.as_deref());
    let style = accent_style("--accent", accent);
    if !style.is_empty() {
        set_style(&card, &style)?;
    }
    append(
        &body,
        &portrait(
            document,
            "party-portrait",
            look.and_then(|look| look.portrait.as_deref()),
            &member.name,
        )?,
    )?;
    let info = element(document, "div", "party-info")?;
    let name_line = element(document, "div", "party-nameline")?;
    if let Some(mark) = look.and_then(|look| look.glyph) {
        append(&name_line, &glyph(document, mark, "party-glyph")?)?;
    }
    append(
        &name_line,
        &text_element(document, "span", "party-name", &member.name)?,
    )?;
    append(&info, &name_line)?;
    if let Some(line) = look.and_then(|look| class_line(look.class.as_deref(), look.level)) {
        append(&info, &text_element(document, "div", "party-class", &line)?)?;
    }
    append(&body, &info)?;
    append(
        &body,
        &text_element(
            document,
            "div",
            "party-hp",
            &format!("HP {} / {}", member.hit_points, member.maximum_hit_points),
        )?,
    )?;
    if !member.conditions.is_empty() {
        let chips = element(document, "div", "party-conditions")?;
        for condition in &member.conditions {
            append(&chips, &text_element(document, "span", "chip", condition)?)?;
        }
        append(&body, &chips)?;
    }
    let tier = health_tier(member.hit_points, member.maximum_hit_points);
    let bar = element(
        document,
        "div",
        &format!("party-bar party-bar-{}", tier.dom_marker()),
    )?;
    let fill = element(document, "div", "party-bar-fill")?;
    set_style(
        &fill,
        &format!(
            "width:{:.1}%;",
            bar_percent(member.hit_points, member.maximum_hit_points)
        ),
    )?;
    append(&bar, &fill)?;
    append(&body, &bar)?;
    Ok(card)
}

fn open_card(document: &Document) -> Result<Element, JsValue> {
    let (card, body) = glass(document, "party-card party-card-open", "party-card-body")?;
    let ring = element(document, "div", "party-open-ring")?;
    append(&ring, &glyph(document, Glyph::Plus, "party-open-plus")?)?;
    append(&body, &ring)?;
    append(
        &body,
        &text_element(document, "div", "party-open-word", OPEN_SEAT_WORD)?,
    )?;
    Ok(card)
}

/// The party strip, or `None` when there is no one to draw.
pub fn strip(document: &Document, scene: &TvScene) -> Result<Option<Element>, JsValue> {
    let cards = strip_cards(&scene.party, scene.seats.as_ref());
    if cards.is_empty() {
        return Ok(None);
    }
    let (strip, bar) = glass(document, "tv-party", "tv-party-bar")?;
    for card in cards {
        let node = match card {
            StripCard::Hero(member, look) => hero_card(document, member, look)?,
            StripCard::Open => open_card(document)?,
        };
        append(&bar, &node)?;
    }
    Ok(Some(strip))
}

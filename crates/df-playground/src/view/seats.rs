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
    OPEN_SEAT_WORD, StripCard, armor_class_text, bar_percent, card_accent, card_portrait,
    class_line, health_tier, hit_points_text, strip_cards,
};
use crate::overlay::{Glyph, PartyLook};
use crate::view::kit::{accent_style, glass, glyph, portrait};

fn hero_card(
    document: &Document,
    member: &PartyMember,
    look: Option<&PartyLook>,
) -> Result<Element, JsValue> {
    let (card, body) = glass(document, "party-card", "party-card-body")?;
    let style = accent_style("--accent", card_accent(member, look));
    if !style.is_empty() {
        set_style(&card, &style)?;
    }
    let picture = portrait(
        document,
        "party-portrait",
        card_portrait(member, look),
        &member.name,
    )?;
    // The hero's own picture is the square headshot of its pack, framed whole (the theme's
    // `portrait-image-head`); a look's portrait is half the body, which the theme zooms to the
    // face.
    if member.portrait.is_some()
        && let Some(image) = picture.query_selector(".portrait-image")?
    {
        image.set_attribute("class", "portrait-image portrait-image-head")?;
    }
    append(&body, &picture)?;
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
    // The hit points and Armor Class are a line of the card's words, under the class line, so a
    // narrow screen keeps them in sight with the name.
    let health = member.health();
    let words: Vec<String> = [
        health.map(|(hit_points, maximum)| hit_points_text(hit_points, maximum)),
        member.armor_class.map(armor_class_text),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !words.is_empty() {
        append(
            &info,
            &text_element(document, "div", "party-hp", &words.join(" · "))?,
        )?;
    }
    append(&body, &info)?;
    if !member.conditions.is_empty() {
        let chips = element(document, "div", "party-conditions")?;
        for condition in &member.conditions {
            append(&chips, &text_element(document, "span", "chip", condition)?)?;
        }
        append(&body, &chips)?;
    }
    // A hero without a sheet yet shows no hit points and no bar, never a sample.
    if let Some((hit_points, maximum)) = health {
        let tier = health_tier(hit_points, maximum);
        let bar = element(
            document,
            "div",
            &format!("party-bar party-bar-{}", tier.dom_marker()),
        )?;
        let fill = element(document, "div", "party-bar-fill")?;
        set_style(
            &fill,
            &format!("width:{:.1}%;", bar_percent(hit_points, maximum)),
        )?;
        append(&bar, &fill)?;
        append(&body, &bar)?;
    }
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

/// The style of the TV's party strip once it has faded out: drawn, so the screen's layout stays,
/// but not seen.
const STRIP_HIDDEN_STYLE: &str = "opacity:0;visibility:hidden;";
/// The style that fades the TV's party strip out from where it shows (1.5 s).
const STRIP_FADING_STYLE: &str =
    "opacity:0;visibility:hidden;transition:opacity 1500ms ease,visibility 0s linear 1500ms;";
/// The marker of a TV party strip drawn hidden.
const STRIP_HIDDEN_MARKER: &str = "data-hidden";

/// The TV's party strip: [`strip`], drawn hidden when the host has faded it out
/// (`TvScene::party_hidden`). The phone's mirror of it is not hidden.
pub fn tv_strip(document: &Document, scene: &TvScene) -> Result<Option<Element>, JsValue> {
    let built = strip(document, scene)?;
    if scene.party_hidden
        && let Some(strip) = &built
    {
        set_style(strip, STRIP_HIDDEN_STYLE)?;
        strip.set_attribute(STRIP_HIDDEN_MARKER, "true")?;
    }
    Ok(built)
}

/// Fades out, in place, the TV strip `slot` shows when the scene now hides it (`hidden`): the
/// strip that showed stays in the slot and fades (the redraw is skipped), so the fade is seen
/// rather than the strip going at once. A strip already hidden, or none, is redrawn as usual, so a
/// change of the party while it is out of sight never brings it back. Whether it faded.
pub fn fade_out(slot: &Element, hidden: bool) -> Result<bool, JsValue> {
    if !hidden {
        return Ok(false);
    }
    let Some(shown) = slot.query_selector(".tv-party")? else {
        return Ok(false);
    };
    if shown.has_attribute(STRIP_HIDDEN_MARKER) {
        return Ok(false);
    }
    set_style(&shown, STRIP_FADING_STYLE)?;
    shown.set_attribute(STRIP_HIDDEN_MARKER, "true")?;
    Ok(true)
}

/// The party strip, or `None` when there is no one to draw, or the lobby is up: the lobby shows
/// its own seats, not the last run's party.
pub fn strip(document: &Document, scene: &TvScene) -> Result<Option<Element>, JsValue> {
    // TODO(DEV-12): test: a scene with a lobby and a stale party draws no strip, and the clue
    // board (`view::clues::board`) draws no Clues chip.
    if scene.lobby.is_some() {
        return Ok(None);
    }
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

//! Draws the party strip (`kit::party`). A framed strip is one glass bar holding unblurred tiles,
//! so no blur is nested; a loose strip is a row of small fine-glass cards. A card is a grid: the
//! portrait, the glyph and name, the class and level as one run of words, and the
//! health bar across the card's foot. A loose compact card's portrait stands on the card itself,
//! outside its clipped pane, so the head rises over the top rim (render 02).
//!
//! A card's condition chips stand out of the card's flow, so every name of a row shares one
//! line: inside the top-right corner over a tall card's portrait, and as a tag on a compact
//! card's top rim. The chips past the layout's few are counted by a "+N" chip; every condition is
//! named to a screen reader.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, set_style, text_element};
use crate::kit::draw::{Shape, glass, put};
use crate::kit::party::{HeroCard, KitCard, PartyLayout, PartyStrip};
use crate::overlay::Glyph;
use crate::overlay::rules::OPEN_SEAT_WORD;
use crate::view::kit::{accent_style, glyph, portrait};

/// The strip, or `None` when it has no cards.
pub fn draw(document: &Document, strip: &PartyStrip) -> Result<Option<Element>, JsValue> {
    let cards = strip.cards();
    if cards.is_empty() {
        return Ok(None);
    }
    let mut class = format!("kit-party kit-party-{}", strip.layout().key());
    if strip.is_dense() {
        class.push_str(" kit-party-dense");
    }
    let row = element(document, "div", "kit-party-row")?;
    for card in &cards {
        let (holder, face) = if strip.framed() {
            let tile = element(document, "div", "kit-card")?;
            (tile.clone(), tile)
        } else {
            glass(document, Shape::Octagon, strip.fill(), "kit-card-glass")?
        };
        match card {
            KitCard::Hero(hero) => {
                hero_card(document, &holder, &face, hero, strip)?;
                put(
                    &holder,
                    conditions(document, hero, strip.conditions_at_most())?,
                )?;
            }
            KitCard::Open => open_card(document, &face)?,
        }
        append(&row, &holder)?;
    }
    if !strip.framed() {
        let root = element(document, "div", &format!("{class} kit-party-loose"))?;
        append(&root, &row)?;
        return Ok(Some(root));
    }
    let (root, pane) = glass(document, Shape::Octagon, strip.fill(), &class)?;
    append(&pane, &row)?;
    Ok(Some(root))
}

/// Fills `face` with a hero's card; `holder` is the card's outermost element, which takes the
/// hero's accent and the downed look.
fn hero_card(
    document: &Document,
    holder: &Element,
    face: &Element,
    hero: &HeroCard<'_>,
    strip: &PartyStrip,
) -> Result<(), JsValue> {
    if hero.downed {
        holder.set_attribute("class", &format!("{} kit-card-downed", holder.class_name()))?;
    }
    let style = accent_style("--kit-accent", hero.accent);
    if !style.is_empty() {
        set_style(holder, &style)?;
    }
    face.set_attribute("class", &format!("{} kit-card-face", face.class_name()))?;
    let rises_over_rim =
        !strip.framed() && !strip.is_dense() && strip.layout() == PartyLayout::Compact;
    let portrait_parent = if rises_over_rim { holder } else { face };
    append(
        portrait_parent,
        &portrait(document, "kit-card-portrait", hero.portrait, hero.name)?,
    )?;
    let body = element(document, "div", "kit-card-body")?;
    let name_row = element(document, "div", "kit-card-name")?;
    if let Some(which) = hero.glyph {
        append(&name_row, &glyph(document, which, "kit-card-glyph")?)?;
    }
    append(
        &name_row,
        &text_element(document, "span", "kit-card-name-text", hero.name)?,
    )?;
    append(&body, &name_row)?;
    if hero.class_name.is_some() || hero.level.is_some() {
        // The class and the level as one run of words that wraps as a phrase ("Fighter Lv. 3",
        // render 02); the hero's one glyph stands before the name.
        let class_row = element(document, "div", "kit-card-class")?;
        if let Some(class_name) = &hero.class_name {
            append(
                &class_row,
                &text_element(document, "span", "kit-card-class-name", class_name)?,
            )?;
            if hero.level.is_some() {
                class_row.append_with_str_1(" ")?;
            }
        }
        if let Some(level) = &hero.level {
            append(
                &class_row,
                &text_element(document, "span", "kit-card-level", level)?,
            )?;
        }
        append(&body, &class_row)?;
    }
    if strip.show_hp_text {
        append(
            &body,
            &text_element(document, "div", "kit-card-hp", &hero.hit_points_text)?,
        )?;
    }
    let bar = element(
        document,
        "div",
        &format!("kit-bar kit-bar-{}", hero.tier.dom_marker()),
    )?;
    bar.set_attribute("role", "img")?;
    bar.set_attribute("aria-label", &hero.hit_points_text)?;
    let fill = element(document, "span", "kit-bar-fill")?;
    set_style(&fill, &format!("width:{:.1}%", hero.bar_percent))?;
    append(&bar, &fill)?;
    append(face, &body)?;
    append(face, &bar)?;
    Ok(())
}

/// A hero's condition chips, the first `at_most` and a "+N" chip for the rest, or `None` when
/// they have none.
fn conditions(
    document: &Document,
    hero: &HeroCard<'_>,
    at_most: usize,
) -> Result<Option<Element>, JsValue> {
    let chips = hero.condition_chips(at_most);
    if chips.shown.is_empty() {
        return Ok(None);
    }
    let badges = element(document, "div", "kit-card-conditions")?;
    badges.set_attribute("role", "img")?;
    badges.set_attribute("aria-label", &hero.conditions().join(", "))?;
    for condition in chips.shown {
        append(
            &badges,
            &text_element(document, "span", "kit-condition", condition)?,
        )?;
    }
    if let Some(more) = &chips.more {
        append(
            &badges,
            &text_element(document, "span", "kit-condition kit-condition-more", more)?,
        )?;
    }
    Ok(Some(badges))
}

fn open_card(document: &Document, face: &Element) -> Result<(), JsValue> {
    face.set_attribute("class", &format!("{} kit-card-open", face.class_name()))?;
    let ring = element(document, "span", "kit-open-ring")?;
    append(&ring, &glyph(document, Glyph::Plus, "kit-open-icon")?)?;
    append(face, &ring)?;
    append(
        face,
        &text_element(document, "span", "kit-open-word", OPEN_SEAT_WORD)?,
    )?;
    Ok(())
}

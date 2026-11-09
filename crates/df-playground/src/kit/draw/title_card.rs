//! Draws an item or chapter title card (`kit::title_card`).

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::kit::draw::{compass_rose, diamond_rule, icon};
use crate::kit::title_card::{CardAlign, CardScale, TitleCard};

/// The card, or `None` when it has neither a title nor a line.
pub fn draw(document: &Document, card: &TitleCard) -> Result<Option<Element>, JsValue> {
    if !card.is_drawn() {
        return Ok(None);
    }
    let root = element(
        document,
        "div",
        &format!(
            "kit-tc kit-tc-{} kit-tc-{}",
            card.align.key(),
            card.scale.key()
        ),
    )?;
    // The blurred ground is its own element, so its fade never sits on an ancestor of the blur.
    append(&root, &element(document, "div", "kit-tc-ground")?)?;
    let body = element(document, "div", "kit-tc-body")?;
    if card.scale == CardScale::Item {
        append(&body, &ornament(document, card)?)?;
    }
    if let Some(kicker) = card.kicker() {
        let row = element(document, "div", "kit-tc-kicker")?;
        append(&row, &element(document, "span", "kit-tc-kicker-rule")?)?;
        append(&row, &text_element(document, "span", "", kicker)?)?;
        append(
            &row,
            &element(
                document,
                "span",
                "kit-tc-kicker-rule kit-tc-kicker-rule-after",
            )?,
        )?;
        append(&body, &row)?;
    }
    if let Some(title) = card.title() {
        append(&body, &title_row(document, card, title)?)?;
    }
    if let Some(rarity) = card.rarity {
        append(
            &body,
            &text_element(
                document,
                "div",
                &format!("kit-tc-rarity kit-rarity-{}", rarity.key()),
                rarity.word(),
            )?,
        )?;
    }
    if card.scale == CardScale::Chapter {
        append(&body, &diamond_rule(document, "kit-tc-rule")?)?;
    }
    if let Some(text) = card.text() {
        append(&body, &text_element(document, "p", "kit-tc-text", text)?)?;
    }
    // A side card closes with the star rule it opens with (render 07).
    if card.align == CardAlign::Right && card.scale == CardScale::Item {
        let foot = ornament(document, card)?;
        foot.set_attribute("class", "kit-tc-ornament kit-tc-ornament-foot")?;
        append(&body, &foot)?;
    }
    append(&root, &body)?;
    Ok(Some(root))
}

/// The title; a centred item's sits between two flank rules on its own baseline (render 01). A
/// side card has its star rules above and below instead, so its title keeps the card's width
/// (render 07).
fn title_row(document: &Document, card: &TitleCard, title: &str) -> Result<Element, JsValue> {
    let heading = text_element(document, "h2", "kit-tc-title", title)?;
    if card.scale == CardScale::Chapter || card.align == CardAlign::Right {
        return Ok(heading);
    }
    let row = element(document, "div", "kit-tc-title-row")?;
    append(&row, &element(document, "span", "kit-tc-flank")?)?;
    append(&row, &heading)?;
    append(
        &row,
        &element(document, "span", "kit-tc-flank kit-tc-flank-after")?,
    )?;
    Ok(row)
}

/// The ornament row: the card's drawing, or the compass rose, between two short hairline rules
/// that fade outward (renders 01 and 07).
fn ornament(document: &Document, card: &TitleCard) -> Result<Element, JsValue> {
    let mark = element(document, "div", "kit-tc-ornament")?;
    mark.set_attribute("aria-hidden", "true")?;
    append(&mark, &element(document, "span", "kit-tc-orn-rule")?)?;
    match card.ornament {
        Some(which) => append(&mark, &icon(document, which, "kit-tc-orn-icon")?)?,
        None => append(&mark, &compass_rose(document, "kit-tc-orn-rose")?)?,
    }
    append(
        &mark,
        &element(document, "span", "kit-tc-orn-rule kit-tc-orn-rule-after")?,
    )?;
    Ok(mark)
}

//! Draws the quest journal (`kit::journal`): parchment, not glass.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::kit::draw::{diamond_rule, icon};
use crate::kit::journal::{Journal, QuestDetail, QuestEntry, QuestState};
use crate::overlay::Glyph;
use crate::view::icons::Icon;
use crate::view::kit::{glyph, photo};

/// The book, or `None` when it has neither entries nor a page of detail.
pub fn draw(document: &Document, journal: &Journal) -> Result<Option<Element>, JsValue> {
    if !journal.is_drawn() {
        return Ok(None);
    }
    let entries = journal.entries();
    let detail = journal.detail();
    let root = element(document, "div", "kit-journal")?;
    let book = element(document, "div", "kit-book")?;
    let left = element(document, "div", "kit-page kit-page-left")?;
    corners(document, &left)?;
    let heading = element(document, "div", "kit-j-heading")?;
    append(
        &heading,
        &text_element(document, "h2", "kit-j-title", journal.title())?,
    )?;
    append(&heading, &glyph(document, Glyph::Star, "kit-j-star")?)?;
    append(&left, &heading)?;
    append(&left, &diamond_rule(document, "kit-j-rule")?)?;
    let list = element(document, "ul", "kit-quests")?;
    for entry in entries {
        append(&list, &entry_element(document, entry)?)?;
    }
    append(&left, &list)?;
    if let Some(more) = journal.more() {
        append(&left, &text_element(document, "p", "kit-j-more", &more)?)?;
    }
    append(&book, &left)?;
    append(&book, &element(document, "div", "kit-ribbon")?)?;
    let right = element(document, "div", "kit-page kit-page-right")?;
    corners(document, &right)?;
    if let Some(detail) = detail {
        page(document, &right, detail)?;
    }
    append(&book, &right)?;
    append(&root, &book)?;
    Ok(Some(root))
}

/// The ink ornaments in a page's four corners.
fn corners(document: &Document, page: &Element) -> Result<(), JsValue> {
    for corner in ["tl", "tr", "bl", "br"] {
        let mark = element(
            document,
            "span",
            &format!("kit-j-corner kit-j-corner-{corner}"),
        )?;
        mark.set_attribute("aria-hidden", "true")?;
        append(page, &mark)?;
    }
    Ok(())
}

fn entry_element(document: &Document, entry: &QuestEntry) -> Result<Element, JsValue> {
    let item = element(
        document,
        "li",
        &format!("kit-quest kit-quest-{}", entry.state.key()),
    )?;
    let mark = element(document, "span", "kit-quest-mark")?;
    match (entry.state, entry.marker) {
        (QuestState::Done, _) => append(&mark, &glyph(document, Glyph::Check, "kit-quest-icon")?)?,
        (_, Some(which)) => append(&mark, &icon(document, which, "kit-quest-icon")?)?,
        (_, None) => append(
            &mark,
            &crate::view::icons::icon(document, Icon::Compass, "kit-quest-icon kit-quest-star")?,
        )?,
    }
    append(&item, &mark)?;
    append(
        &item,
        &text_element(document, "span", "kit-quest-title", entry.title.trim())?,
    )?;
    // The current entry's sparkles on its bottom and right rims (render 36).
    if entry.state == QuestState::Current {
        let sparks = element(document, "span", "kit-quest-sparks")?;
        sparks.set_attribute("aria-hidden", "true")?;
        append(&item, &sparks)?;
    }
    Ok(item)
}

/// The right page: the title, the body, always whole, and the sketch under it, which takes only
/// the room the body leaves.
fn page(document: &Document, right: &Element, detail: &QuestDetail) -> Result<(), JsValue> {
    append(
        right,
        &text_element(document, "h3", "kit-j-detail-title", detail.title.trim())?,
    )?;
    append(right, &diamond_rule(document, "kit-j-rule")?)?;
    append(
        right,
        &text_element(document, "p", "kit-j-body", detail.body.trim())?,
    )?;
    if let Some(sketch) = photo(
        document,
        "kit-j-sketch",
        detail.sketch.as_deref(),
        detail.title.trim(),
    )? {
        append(right, &sketch)?;
    }
    Ok(())
}

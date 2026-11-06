//! The landing page's Tonight's Adventure card (render 19, the glass card at the right): an italic
//! heading, the adventure's scene art with its title over the foot of the picture, an italic blurb,
//! fact rows and the gold launch button.
//!
//! The screen is a television read from a sofa, so the card is sized and counted for that (the
//! `home-adventure` block of `theme.css`): the first `FACTS_SHOWN` fact rows are drawn, the blurb
//! is cut at three lines, and the launch button is the card's one control: it carries its focus id
//! (`adventure:launch`), and the page draws it focused (the `hm-focused` class) when its focus
//! names it.
//!
//! The builder is a function of `AdventureData`; it fills the slot box the page gives it
//! (`width: 100%; height: 100%`). Every text is set as text, a picture becomes an address only when
//! it names an image of the media library, and the drawings are fixed strings of this crate.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::home::adventure::{AdventureData, FactRow, LAUNCH_FOCUS_ID};
use crate::home::glyph::HomeGlyph;
use crate::view::home::kit::{icon, page_glyph};
use crate::view::kit::photo;

/// The adventure component, or an error when the page cannot be built.
pub fn build(document: &Document, data: &AdventureData) -> Result<Element, JsValue> {
    // The card is the lit glass itself: no wrapper with a filter, which would stop it blurring the
    // plate.
    let panel = element(document, "div", "ui-lit ha-panel")?;
    panel.set_attribute("data-component", "home-adventure")?;
    panel.set_attribute("role", "region")?;
    let name = if data.heading.is_empty() {
        data.title.as_str()
    } else {
        data.heading.as_str()
    };
    panel.set_attribute("aria-label", name)?;
    append(&panel, &picture(document, data)?)?;
    if !data.blurb.is_empty() {
        append(
            &panel,
            &text_element(document, "p", "ha-blurb", &data.blurb)?,
        )?;
    }
    if !data.facts.is_empty() {
        append(&panel, &facts(document, data.shown_facts())?)?;
    }
    if !data.launch_label.is_empty() {
        append(&panel, &launch(document, data)?)?;
    }
    Ok(panel)
}

/// The picture with the heading and the title over its foot (the card's box is too short for them
/// to take rows of their own at television size): a neutral plate when there is no picture.
fn picture(document: &Document, data: &AdventureData) -> Result<Element, JsValue> {
    let figure = element(document, "figure", "ha-picture")?;
    match photo(
        document,
        "ha-art",
        data.picture.as_deref(),
        &format!("Scene art of {}", data.title),
    )? {
        Some(image) => append(&figure, &image)?,
        None => append(&figure, &element(document, "div", "ha-art ha-art-plate")?)?,
    }
    append(&figure, &element(document, "div", "ha-fade")?)?;
    let caption = element(document, "figcaption", "ha-caption")?;
    if !data.heading.is_empty() {
        append(
            &caption,
            &text_element(document, "h2", "ha-heading", &data.heading)?,
        )?;
    }
    if !data.title.is_empty() {
        append(
            &caption,
            &text_element(document, "h3", "ha-title", &data.title)?,
        )?;
    }
    append(&figure, &caption)?;
    Ok(figure)
}

fn facts(document: &Document, facts: &[FactRow]) -> Result<Element, JsValue> {
    let list = element(document, "dl", "ha-facts")?;
    for fact in facts {
        let row = element(document, "div", "ha-fact")?;
        append(&row, &icon(document, fact.icon, "ha-fact-icon")?)?;
        append(
            &row,
            &text_element(document, "dt", "ha-fact-label", &fact.label)?,
        )?;
        append(
            &row,
            &text_element(document, "dd", "ha-fact-value", &fact.value)?,
        )?;
        append(&list, &row)?;
    }
    Ok(list)
}

/// The launch button: a play mark and the words, gold when it can be used and grey when it cannot.
fn launch(document: &Document, data: &AdventureData) -> Result<Element, JsValue> {
    let class = if data.enabled {
        "ha-launch-wrap"
    } else {
        "ha-launch-wrap ha-launch-off"
    };
    let button = element(document, "button", class)?;
    button.set_attribute("type", "button")?;
    button.set_attribute("data-focus-id", LAUNCH_FOCUS_ID)?;
    if !data.enabled {
        button.set_attribute("aria-disabled", "true")?;
    }
    let face = element(document, "span", "ui-gold ha-launch")?;
    append(
        &face,
        &page_glyph(document, HomeGlyph::Play, "ha-launch-icon")?,
    )?;
    append(
        &face,
        &text_element(document, "span", "ha-launch-label", &data.launch_label)?,
    )?;
    append(&button, &face)?;
    Ok(button)
}

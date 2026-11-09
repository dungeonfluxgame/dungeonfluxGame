//! Draws the brand mark (`kit::brand`): the wordmark beside the eight-point compass rose inline at
//! the top left, or stacked as a crest: a gold ring centred behind the bevelled wordmark, the
//! rose's long points piercing the ring above and below the word, hairlines running out from the
//! ring (render 02).

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::kit::brand::BrandMark;
use crate::kit::draw::compass_rose;
use crate::lobby::model::LogoStyle;

pub fn draw(document: &Document, mark: &BrandMark) -> Result<Element, JsValue> {
    let root = element(
        document,
        "div",
        &format!("kit-brand kit-brand-{}", mark.style().key()),
    )?;
    let lockup = element(document, "div", "kit-brand-lockup")?;
    lockup.set_attribute("role", "img")?;
    lockup.set_attribute("aria-label", mark.wordmark())?;
    if mark.style() == LogoStyle::Stacked {
        let ring = element(document, "span", "kit-brand-ring")?;
        ring.set_attribute("aria-hidden", "true")?;
        append(&lockup, &ring)?;
    }
    append(&lockup, &compass_rose(document, "kit-brand-rose")?)?;
    let word = element(document, "div", "kit-brand-text")?;
    append(
        &word,
        &text_element(document, "span", "kit-brand-word", mark.wordmark())?,
    )?;
    append(&word, &element(document, "span", "kit-brand-rule")?)?;
    append(&lockup, &word)?;
    append(&root, &lockup)?;
    if let Some(place) = mark.place() {
        append(
            &root,
            &text_element(document, "div", "kit-brand-place", place)?,
        )?;
    }
    Ok(root)
}

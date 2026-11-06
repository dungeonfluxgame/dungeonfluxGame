//! The loading screen, and the dispatch of the player lobby (`view::lobby::page`,
//! `view::lobby::seat` and `view::lobby::panels`): the loading screen's bar and ready chips are
//! drawn here.
//!
//! The screens are pure functions of the scene's data (`lobby::model`): this module turns the
//! answers of the model into elements and decides nothing. Text from a scene is set as text, a
//! picture name becomes an address only when it names an image of the media library, and a number
//! reaches a style only as a number.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

pub mod page;
pub mod panels;
pub mod seat;

use crate::dom::{append, element, set_style, text_element};
use crate::fixture::TvScene;
use crate::lobby::model::{LOADING_LABEL, LoadingScreen, READY_WORD, bar_percent};
use crate::overlay::Glyph;
use crate::view::icons::{Icon, icon};
use crate::view::kit::{glass, glyph, photo};

/// The lobby or the loading screen of `scene`, or `None` when it has neither.
pub fn build(document: &Document, scene: &TvScene) -> Result<Option<Element>, JsValue> {
    if let Some(lobby) = &scene.lobby {
        return crate::view::lobby::page::build(document, lobby).map(Some);
    }
    if let Some(loading) = &scene.loading {
        return loading_screen(document, loading).map(Some);
    }
    Ok(None)
}

/// An element of `tag` and `class` holding `text` as text.
fn text(document: &Document, tag: &str, class: &str, words: &str) -> Result<Element, JsValue> {
    text_element(document, tag, class, words)
}

/// A line with a diamond at each end and a rule beyond it, as over the lobby's title.
pub(crate) fn eyebrow(document: &Document, words: &str, class: &str) -> Result<Element, JsValue> {
    let row = element(document, "div", &format!("lb-eyebrow {class}"))?;
    for part in ["lb-rule", "lb-diamond"] {
        append(&row, &element(document, "span", part)?)?;
    }
    append(&row, &text(document, "span", "lb-eyebrow-text", words)?)?;
    for part in ["lb-diamond", "lb-rule"] {
        append(&row, &element(document, "span", part)?)?;
    }
    Ok(row)
}

/// The thin rule with a diamond in the middle, under the subtitle.
fn flourish(document: &Document) -> Result<Element, JsValue> {
    let row = element(document, "div", "lb-flourish")?;
    for part in ["lb-rule", "lb-diamond lb-diamond-open", "lb-rule"] {
        append(&row, &element(document, "span", part)?)?;
    }
    Ok(row)
}

fn loading_screen(document: &Document, loading: &LoadingScreen) -> Result<Element, JsValue> {
    let root = element(document, "div", "lb lb-loading")?;
    let head = element(document, "header", "lb-load-head")?;
    if let Some(kicker) = &loading.kicker {
        append(&head, &eyebrow(document, kicker, "lb-eyebrow-load")?)?;
    }
    append(
        &head,
        &text(document, "h2", "lb-title lb-title-load", &loading.title)?,
    )?;
    append(&head, &flourish(document)?)?;
    if let Some(subtitle) = &loading.subtitle {
        append(
            &head,
            &text(document, "p", "lb-subtitle lb-subtitle-load", subtitle)?,
        )?;
    }
    append(&root, &head)?;
    if let Some(narrator) = &loading.narrator {
        let (wrapper, panel) = glass(document, "lb-narrator-wrap", "lb-narrator")?;
        let face = element(document, "div", "lb-narrator-face")?;
        if let Some(image) = photo(
            document,
            "lb-photo",
            narrator.portrait.as_deref(),
            &narrator.name,
        )? {
            append(&face, &image)?;
        }
        append(&panel, &face)?;
        let words = element(document, "div", "lb-narrator-words")?;
        append(
            &words,
            &text(document, "div", "lb-narrator-name", &narrator.name)?,
        )?;
        if let Some(status) = &narrator.status {
            append(
                &words,
                &text(document, "div", "lb-narrator-status", status)?,
            )?;
        }
        append(&panel, &words)?;
        append(&panel, &element(document, "span", "lb-vrule")?)?;
        append(&panel, &icon(document, Icon::Waveform, "lb-wave")?)?;
        append(&root, &wrapper)?;
    }
    let label = loading.label.as_deref().unwrap_or(LOADING_LABEL);
    append(&root, &eyebrow(document, label, "lb-eyebrow-bar")?)?;
    append(&root, &loading_bar(document, bar_percent(loading.percent))?)?;
    let chips = element(document, "div", "lb-chips")?;
    chips.set_attribute("role", "list")?;
    set_style(
        &chips,
        &format!("--lb-chips:{};", loading.seats.len().max(1)),
    )?;
    for chip in &loading.seats {
        let (wrapper, panel) = glass(
            document,
            if chip.ready {
                "lb-chip lb-chip-ready"
            } else {
                "lb-chip"
            },
            "lb-chip-panel",
        )?;
        wrapper.set_attribute("role", "listitem")?;
        let face = element(document, "div", "lb-chip-face")?;
        match photo(document, "lb-photo", chip.portrait.as_deref(), &chip.name)? {
            Some(image) => append(&face, &image)?,
            None => append(&face, &icon(document, Icon::Silhouette, "lb-silhouette")?)?,
        }
        append(&panel, &face)?;
        let words = element(document, "div", "lb-chip-words")?;
        append(&words, &text(document, "div", "lb-chip-name", &chip.name)?)?;
        if chip.ready {
            let ready = element(document, "div", "lb-chip-ready-word")?;
            append(&ready, &glyph(document, Glyph::Check, "lb-badge-glyph")?)?;
            append(&ready, &text(document, "span", "", READY_WORD)?)?;
            append(&words, &ready)?;
        }
        append(&panel, &words)?;
        append(&chips, &wrapper)?;
    }
    append(&root, &chips)?;
    Ok(root)
}

fn loading_bar(document: &Document, percent: u8) -> Result<Element, JsValue> {
    let (wrapper, panel) = glass(document, "lb-loadbar-wrap", "lb-loadbar")?;
    panel.set_attribute("role", "progressbar")?;
    panel.set_attribute("aria-valuemin", "0")?;
    panel.set_attribute("aria-valuemax", "100")?;
    panel.set_attribute("aria-valuenow", &percent.to_string())?;
    panel.set_attribute("aria-label", "Loading progress")?;
    let track = element(document, "div", "lb-loadbar-track")?;
    let fill = element(document, "div", "lb-loadbar-fill")?;
    set_style(&fill, &format!("width:{percent}%;"))?;
    append(&track, &fill)?;
    append(&panel, &track)?;
    append(
        &panel,
        &text(
            document,
            "span",
            "lb-loadbar-percent",
            &format!("{percent}%"),
        )?,
    )?;
    Ok(wrapper)
}

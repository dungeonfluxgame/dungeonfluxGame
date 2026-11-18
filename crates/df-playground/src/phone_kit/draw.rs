//! The browser drawing of the phone kit: each child turns one component's input into elements and
//! decides nothing (the decisions are the component modules'). This module holds what they share:
//! the kit's stylesheet, the line glyphs, and the screen that puts the pieces in order.
//!
//! A screen is a column: the link line, the header, the status, a scrolling body, and a dock that
//! stays low under the thumb (the composer and the tab bar). The same pieces make the
//! gallery's sample screens and the live phone (`view::phone`).

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::copy::RECONNECTING_WORDS;
use crate::dom::{append, element, line_svg, set_style};
use crate::media::{MediaKind, media_kind};
use crate::path::media_file;
use crate::phone_kit::PhonePanels;
use crate::phone_kit::glyph::PhoneGlyph;
use crate::phone_kit::shell::{PhoneHeader, Pill, TabKey};

pub mod act;
#[cfg(debug_assertions)]
pub mod gallery;
pub mod nav;
pub mod page;
pub mod roll;
pub mod shell;
pub mod talk;

const PHONE_STYLE_ID: &str = "phone-style";
/// The phone kit's stylesheet. The host serves only `theme.css` of the playground's styles, so the
/// kit's rules travel in the package and are put in the page once, like the TV kit's.
const PHONE_CSS: &str = include_str!("../../../../assets/playground/phone.css");

/// Puts the phone kit's stylesheet in the page once.
pub fn ensure_style(document: &Document) -> Result<(), JsValue> {
    // The shared screen's parts the phone mirrors (the roll card, the party strip, the clue board)
    // are the kit's: its sheet goes in first, so the phone's wins ties.
    crate::kit::draw::ensure_style(document)?;
    crate::dom::ensure_style(document, PHONE_STYLE_ID, PHONE_CSS)
}

/// A phone glyph as an `svg` of class `class`, drawn in the current text colour.
pub fn glyph(document: &Document, glyph: PhoneGlyph, class: &str) -> Result<Element, JsValue> {
    line_svg(document, glyph.markup(), "1.7", class)
}

/// The regions of one phone screen, in order: the link line (when `link_down`), the header pinned
/// top, the status, the one scrolling `body` and the `dock` pinned low under the thumb. Every
/// phone screen is laid out here, so each has exactly one scrolling region. `class` names the
/// screen beside `ph-screen`.
pub fn regions(
    document: &Document,
    class: &str,
    link_down: bool,
    header: Option<&PhoneHeader>,
    status: Option<&Pill>,
    body: &Element,
    dock: &Element,
) -> Result<Element, JsValue> {
    let classes = if class.is_empty() {
        "ph-screen".to_owned()
    } else {
        format!("ph-screen {class}")
    };
    let screen = element(document, "div", &classes)?;
    if link_down {
        append(&screen, &shell::link_bar(document, RECONNECTING_WORDS)?)?;
    }
    if let Some(header) = header {
        append(&screen, &shell::header(document, header)?)?;
    }
    if let Some(status) = status {
        append(&screen, &shell::status(document, status)?)?;
    }
    append(&screen, body)?;
    append(&screen, dock)?;
    Ok(screen)
}

/// Puts `media` behind `screen` as its full-bleed scene. A name that is not an image of the media
/// library leaves the plain night gradient.
pub fn set_scene(screen: &Element, media: Option<&str>) -> Result<(), JsValue> {
    let Some(name) = media.filter(|name| media_kind(name) == Some(MediaKind::Image)) else {
        return Ok(());
    };
    set_style(screen, &format!("--ph-scene:url(\"{}\")", media_file(name)))
}

/// One sample screen of the gallery: every piece `panels` names, in the order of a phone. The
/// open panel of the tab bar decides whether the story (narration and note) or the actions (the
/// choices) are drawn; a screen without a bar draws both.
pub fn screen(document: &Document, panels: &PhonePanels) -> Result<Element, JsValue> {
    let body = body(document, panels)?;
    let dock = element(document, "div", "ph-dock")?;
    if let Some(sample) = &panels.composer {
        append(
            &dock,
            &crate::view::composer::build(document, &sample.view(), &sample.text)?,
        )?;
    }
    if let Some(tabs) = &panels.tabs {
        append(&dock, &shell::tab_bar(document, tabs)?)?;
    }
    let screen = regions(
        document,
        "",
        panels.link_down,
        panels.header.as_ref(),
        panels.status.as_ref(),
        &body,
        &dock,
    )?;
    set_scene(&screen, panels.scene.as_deref())?;
    Ok(screen)
}

/// The scrolling body of a screen: every piece `panels` names, in the order of a phone.
pub fn body(document: &Document, panels: &PhonePanels) -> Result<Element, JsValue> {
    let open = panels.tabs.as_ref().and_then(|tabs| tabs.active);
    let story = open.is_none_or(|tab| tab == TabKey::Story);
    let actions = open.is_none_or(|tab| tab == TabKey::Act);
    let body = element(document, "main", "ph-body")?;
    if let Some(toast) = &panels.toast {
        append(&body, &shell::toast(document, toast)?)?;
    }
    if !panels.pills.is_empty() {
        let row = element(document, "div", "ph-pill-row")?;
        for pill in &panels.pills {
            append(&row, &shell::pill(document, pill)?)?;
        }
        append(&body, &row)?;
    }
    for hp in &panels.hp {
        append(&body, &shell::hp_bar(document, hp)?)?;
    }
    for spec in &panels.buttons {
        append(&body, &shell::button(document, spec)?)?;
    }
    if let Some(act) = &panels.act {
        if story && let Some(lines) = act::narration(document, &act.narration)? {
            append(&body, &lines)?;
        }
        if story && let Some(note) = &act.private_note {
            append(&body, &act::private_note(document, note)?)?;
        }
        if actions && !act.choices.is_empty() {
            append(&body, &act::choices(document, &act.choices)?)?;
        }
    }
    if let Some(talk) = &panels.talk {
        if !talk.cast.is_empty() {
            append(&body, &talk::cast_strip(document, talk)?)?;
        }
        if !talk.thread.is_empty() {
            append(&body, &talk::thread(document, talk)?)?;
        }
    }
    if let Some(roll) = &panels.roll {
        append(&body, &roll::panel(document, roll)?)?;
    }
    for item in &panels.blocks {
        append(&body, &page::block(document, item)?)?;
    }
    if let Some(empty) = &panels.empty {
        append(&body, &shell::empty(document, empty)?)?;
    }
    if let Some(loading) = &panels.loading {
        append(&body, &shell::loading(document, loading)?)?;
    }
    Ok(body)
}

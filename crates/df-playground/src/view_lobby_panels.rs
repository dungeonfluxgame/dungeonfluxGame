//! The three panels of the player lobby below the seats: the join code with its QR picture, the
//! party summary and the begin button.
//!
//! The join code is the text a player types on a phone, drawn large and as plain text; the picture
//! beside it is a placeholder, not a scannable code (the real one comes from the server). The party
//! summary says how many are at the table and what they are doing. The begin button is the one
//! primary action of the screen; whether it may be pressed is the server's word, so a button that
//! may not be pressed is drawn quiet with the note the scene gives. The shared screen takes no
//! input: none of these is a control.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::lobby_model::{
    BEGIN_LABEL, BeginButton, JOIN_HINT, JoinCode, PartySummary, SummaryIcon, count_text,
    summary_word,
};
use crate::view_icons::{Icon, icon, qr_placeholder};

/// The join panel: the QR picture and, beside it, the words over the game code and the code.
pub fn join(document: &Document, join: &JoinCode) -> Result<Element, JsValue> {
    let panel = element(document, "section", "pl-join")?;
    panel.set_attribute("data-component", "lobby-join")?;
    panel.set_attribute("aria-label", "How to join from a phone")?;
    let code = join
        .game_code
        .as_deref()
        .map(str::trim)
        .filter(|code| !code.is_empty());
    let source = code.or(join.join_link.as_deref());
    if let Some(source) = source {
        let block = element(document, "div", "pl-qr")?;
        append(&block, &qr_placeholder(document, source, "pl-qr-cells")?)?;
        append(&panel, &block)?;
    }
    let words = element(document, "div", "pl-join-words")?;
    let hint = join.hint.as_deref().unwrap_or(JOIN_HINT);
    append(&words, &text_element(document, "p", "pl-join-hint", hint)?)?;
    if let Some(code) = code {
        append(&words, &text_element(document, "p", "pl-code", code)?)?;
    }
    append(&panel, &words)?;
    Ok(panel)
}

/// The party summary: a seal and the count over its word.
pub fn party(document: &Document, party: &PartySummary) -> Result<Element, JsValue> {
    let panel = element(document, "section", "pl-party")?;
    panel.set_attribute("data-component", "lobby-party")?;
    let which = match party.icon {
        SummaryIcon::Check => (Icon::CheckRing, "pl-party-icon pl-party-check"),
        SummaryIcon::People => (Icon::People, "pl-party-icon pl-party-people"),
        SummaryIcon::Spinner => (Icon::Spinner, "pl-party-icon pl-party-spinner"),
    };
    append(&panel, &icon(document, which.0, which.1)?)?;
    let words = element(document, "div", "pl-party-words")?;
    append(
        &words,
        &text_element(
            document,
            "p",
            "pl-party-count",
            &count_text(party.joined, party.capacity),
        )?,
    )?;
    append(
        &words,
        &text_element(document, "p", "pl-party-word", summary_word(party))?,
    )?;
    if let Some(detail) = party
        .detail
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        append(
            &words,
            &text_element(document, "p", "pl-party-detail", detail)?,
        )?;
    }
    append(&panel, &words)?;
    Ok(panel)
}

/// The begin button: a gold plate with a play mark and the label, and under it the note a button
/// that may not be pressed carries.
pub fn begin(document: &Document, begin: &BeginButton) -> Result<Element, JsValue> {
    let class = if begin.enabled {
        "pl-begin"
    } else {
        "pl-begin pl-begin-off"
    };
    let button = element(document, "div", class)?;
    button.set_attribute("data-component", "lobby-begin")?;
    button.set_attribute("data-focus-id", crate::lobby_model::BEGIN_FOCUS_ID)?;
    let face = element(document, "div", "pl-begin-face")?;
    face.set_attribute("role", "button")?;
    if !begin.enabled {
        face.set_attribute("aria-disabled", "true")?;
    }
    append(&face, &icon(document, Icon::Play, "pl-begin-play")?)?;
    let label = begin.label.as_deref().unwrap_or(BEGIN_LABEL);
    append(
        &face,
        &text_element(document, "span", "pl-begin-label", label)?,
    )?;
    append(&button, &face)?;
    if !begin.enabled
        && let Some(note) = begin.note.as_deref().filter(|text| !text.trim().is_empty())
    {
        append(
            &button,
            &text_element(document, "p", "pl-begin-note", note)?,
        )?;
    }
    Ok(button)
}

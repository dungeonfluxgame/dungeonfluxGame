//! One player's phone: the hero and goal, what is happening, a private note, the choice list and
//! the composer.
//!
//! It turns a `PhoneScene` and the chosen controls into page elements. The choice rules (how many,
//! which groups, what is hidden) come from `choices`; this module only draws what it returns, and
//! it shows every warning, so a fixture that breaks the contract is visible and not repaired.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::choices::{ChoiceListView, build};
use crate::controls::Controls;
use crate::copy::{status_blocks_input, status_copy};
use crate::dom::{append, element, text_element};
use crate::fixture::{Choice, PhoneScene, Status};

fn choice_button(document: &Document, choice: &Choice) -> Result<Element, JsValue> {
    let button = element(document, "button", "choice")?;
    button.set_attribute("type", "button")?;
    append(
        &button,
        &text_element(document, "span", "choice-label", &choice.label)?,
    )?;
    if let Some(explain) = &choice.explain {
        append(
            &button,
            &text_element(document, "span", "choice-explain", explain)?,
        )?;
    }
    Ok(button)
}

fn choice_list(document: &Document, view: &ChoiceListView<'_>) -> Result<Element, JsValue> {
    let list = element(document, "div", "choices")?;
    if !view.suggested.is_empty() {
        let suggested = element(document, "div", "choice-suggested")?;
        for choice in view.suggested {
            append(&suggested, &choice_button(document, choice)?)?;
        }
        if view.left_out > 0 {
            append(
                &suggested,
                &text_element(
                    document,
                    "div",
                    "choice-more",
                    &format!("{} more not shown", view.left_out),
                )?,
            )?;
        }
        append(&list, &suggested)?;
    }
    for group in &view.groups {
        let section = element(document, "div", "choice-group")?;
        append(
            &section,
            &text_element(document, "div", "choice-group-name", &group.name)?,
        )?;
        let row = element(document, "div", "choice-row")?;
        for choice in &group.choices {
            append(&row, &choice_button(document, choice)?)?;
        }
        append(&section, &row)?;
        append(&list, &section)?;
    }
    if !view.unavailable.is_empty() {
        let section = element(document, "div", "choice-unavailable")?;
        append(
            &section,
            &text_element(document, "div", "choice-group-name", "Not available now")?,
        )?;
        for choice in view.unavailable {
            let row = element(document, "div", "choice-off")?;
            append(
                &row,
                &text_element(document, "span", "choice-label", &choice.label)?,
            )?;
            append(
                &row,
                &text_element(document, "span", "choice-explain", &choice.reason)?,
            )?;
            append(&section, &row)?;
        }
        append(&list, &section)?;
    }
    Ok(list)
}

fn status_chip(document: &Document, status: &Status) -> Result<Option<Element>, JsValue> {
    let Some(copy) = status_copy(status) else {
        return Ok(None);
    };
    let class = if status_blocks_input(status) {
        "phone-status phone-status-blocking"
    } else {
        "phone-status"
    };
    text_element(document, "div", class, &copy).map(Some)
}

fn composer(document: &Document, blocked: bool) -> Result<Element, JsValue> {
    let bar = element(document, "div", "composer")?;
    let field = text_element(
        document,
        "div",
        if blocked {
            "composer-field composer-field-off"
        } else {
            "composer-field"
        },
        "Say or do something",
    )?;
    let send = text_element(document, "div", "composer-send", "Send")?;
    append(&bar, &field)?;
    append(&bar, &send)?;
    Ok(bar)
}

/// Draws one phone. Returns the element and the warnings the choice rule found.
pub fn render(
    document: &Document,
    scene: &PhoneScene,
    status: &Status,
    controls: &Controls,
) -> Result<(Element, Vec<String>), JsValue> {
    let view = build(scene, controls.show_unavailable);
    let frame = element(document, "section", "device device-phone")?;
    frame.set_attribute("aria-label", "A player's phone")?;
    let screen = element(document, "div", "phone-screen")?;

    let head = element(document, "header", "phone-head")?;
    append(
        &head,
        &text_element(document, "div", "phone-hero", &scene.hero)?,
    )?;
    if let Some(goal) = &scene.goal {
        append(&head, &text_element(document, "div", "phone-goal", goal)?)?;
    }
    append(&screen, &head)?;
    if let Some(chip) = status_chip(document, status)? {
        append(&screen, &chip)?;
    }

    let story = element(document, "div", "phone-story")?;
    for line in &scene.narration {
        append(&story, &text_element(document, "p", "phone-line", line)?)?;
    }
    append(&screen, &story)?;
    if let Some(note) = &scene.note {
        let card = element(document, "div", "note-card")?;
        append(
            &card,
            &text_element(document, "div", "note-title", "A note only you can see")?,
        )?;
        append(
            &card,
            &text_element(document, "div", "note-text", &note.text)?,
        )?;
        append(&screen, &card)?;
    }
    append(&screen, &choice_list(document, &view)?)?;
    append(&screen, &composer(document, status_blocks_input(status))?)?;
    append(&frame, &screen)?;
    let warnings = view.warnings.iter().map(ToString::to_string).collect();
    Ok((frame, warnings))
}

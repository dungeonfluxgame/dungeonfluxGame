//! One player's phone: the hero and goal, what is happening, a private note, the choice list and
//! the composer (`composer`).
//!
//! It turns a `PhoneScene` and the chosen controls into page elements. The choice rules (how many,
//! which groups, what is hidden) come from `choices`; this module only draws what it returns, and
//! it shows every warning, so a fixture that breaks the contract is visible and not repaired.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::choices::{ChoiceListView, build};
use crate::controls::Controls;
use crate::copy::{status_blocks_input, status_copy};
use crate::dom::{append, element, picture, text_element};
use crate::draw_plan::Part;
use crate::fixture::{Choice, PhoneScene, Status};
use crate::view::tv::slot;

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

/// The parts of a phone, in the order they are drawn.
pub const PARTS: [Part; 7] = [
    Part::PhoneHead,
    Part::PhoneStatus,
    Part::PhoneImage,
    Part::PhoneNarration,
    Part::PhoneNote,
    Part::PhoneChoices,
    Part::PhoneComposer,
];

/// A phone with its slots empty.
pub struct PhoneShell {
    pub section: Element,
    pub slots: Vec<(Part, Element)>,
}

/// Builds one phone's shell: the screen with a slot for each piece. The story is one scrolling
/// column that holds the picture and the narration, so a redraw of one keeps the other.
pub fn shell(document: &Document) -> Result<PhoneShell, JsValue> {
    let section = element(document, "section", "device device-phone")?;
    section.set_attribute("aria-label", "A player's phone")?;
    let screen = element(document, "div", "phone-screen")?;
    let story = element(document, "div", "phone-story")?;
    let mut slots = Vec::new();
    for part in PARTS {
        let holder = slot(document, part)?;
        match part {
            Part::PhoneImage | Part::PhoneNarration => append(&story, &holder)?,
            _ => {
                // The story sits between the status and the note.
                if part == Part::PhoneNote {
                    append(&screen, &story)?;
                }
                append(&screen, &holder)?;
            }
        }
        slots.push((part, holder));
    }
    append(&section, &screen)?;
    Ok(PhoneShell { section, slots })
}

fn head(document: &Document, scene: &PhoneScene) -> Result<Element, JsValue> {
    let head = element(document, "header", "phone-head")?;
    append(
        &head,
        &text_element(document, "div", "phone-hero", &scene.hero)?,
    )?;
    if let Some(goal) = &scene.goal {
        append(&head, &text_element(document, "div", "phone-goal", goal)?)?;
    }
    Ok(head)
}

fn narration(document: &Document, scene: &PhoneScene) -> Result<Option<Element>, JsValue> {
    if scene.narration.is_empty() {
        return Ok(None);
    }
    let lines = element(document, "div", "phone-lines")?;
    for line in &scene.narration {
        append(&lines, &text_element(document, "p", "phone-line", line)?)?;
    }
    Ok(Some(lines))
}

fn note(document: &Document, scene: &PhoneScene) -> Result<Option<Element>, JsValue> {
    let Some(note) = &scene.note else {
        return Ok(None);
    };
    let card = element(document, "div", "note-card")?;
    append(
        &card,
        &text_element(document, "div", "note-title", "A note only you can see")?,
    )?;
    append(
        &card,
        &text_element(document, "div", "note-text", &note.text)?,
    )?;
    Ok(Some(card))
}

/// What goes in the slot of `part`, or `None` when the piece is empty. A part that is not of a
/// phone is an error: the caller asked the wrong view.
pub fn build_part(
    document: &Document,
    part: Part,
    scene: &PhoneScene,
    controls: &Controls,
) -> Result<Option<Element>, JsValue> {
    let status = controls.status_for(&scene.status);
    match part {
        Part::PhoneHead => head(document, scene).map(Some),
        Part::PhoneStatus => status_chip(document, &status),
        Part::PhoneImage => scene
            .image
            .as_ref()
            .map(|image| picture(document, "phone-picture", image))
            .transpose(),
        Part::PhoneNarration => narration(document, scene),
        Part::PhoneNote => note(document, scene),
        Part::PhoneChoices => {
            let view = build(scene, controls.show_unavailable);
            choice_list(document, &view).map(Some)
        }
        Part::PhoneComposer => Err(JsValue::from_str(
            "the composer is drawn by view::composer, which needs the composer's state",
        )),
        other => Err(JsValue::from_str(&format!(
            "{} is not a piece of a phone",
            other.dom_marker()
        ))),
    }
}

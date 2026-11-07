//! The phone's composer as elements: who to talk to, the strip that shows a recording, a note, and
//! the row with the microphone, the box and Send.
//!
//! It draws a `ComposerView` and decides nothing. The composer is built with the phone, and after
//! that `sync` brings the same elements up to date in place, so the box keeps its focus, caret and
//! text while a recording runs or a question is on its way. Text is set as text, never as markup,
//! and every element a later update needs carries a `data-composer` marker.

use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element, HtmlInputElement, HtmlSelectElement};

use crate::composer::{ComposerView, MicLook};
use crate::dom::{append, button, document, element, option, select, text_element};

const MAXIMUM_LEVEL_PERMILLE: u16 = 1_000;

fn mark(element: &Element, name: &str) -> Result<(), JsValue> {
    element.set_attribute("data-composer", name)
}

fn find(root: &Element, name: &str) -> Result<Element, JsValue> {
    root.query_selector(&format!("[data-composer=\"{name}\"]"))?
        .ok_or_else(|| JsValue::from_str(&format!("the composer has no {name}")))
}

fn set_flag(element: &Element, attribute: &str, on: bool) -> Result<(), JsValue> {
    if on {
        element.set_attribute(attribute, "")
    } else {
        element.remove_attribute(attribute)
    }
}

/// A button with a marker, so `sync` finds it again.
fn marked_button(
    document: &Document,
    name: &str,
    class: &str,
    label: &str,
    action: &str,
) -> Result<Element, JsValue> {
    let control = button(document, class, label, action)?;
    mark(&control, name)?;
    Ok(control)
}

/// The composer, drawn for `view`, with `draft` in its box.
pub fn build(document: &Document, view: &ComposerView, draft: &str) -> Result<Element, JsValue> {
    let bar = element(document, "div", "composer")?;

    let who = element(document, "label", "composer-who")?;
    mark(&who, "who")?;
    append(
        &who,
        &text_element(document, "span", "composer-who-label", "Talk to")?,
    )?;
    let menu = select(document, "Talk to", "talk-to")?;
    mark(&menu, "menu")?;
    append(&who, &menu)?;
    append(&bar, &who)?;

    let live = element(document, "div", "composer-live")?;
    mark(&live, "live")?;
    append(&live, &element(document, "span", "composer-rec-dot")?)?;
    let live_text = element(document, "span", "composer-live-text")?;
    mark(&live_text, "live-text")?;
    append(&live, &live_text)?;
    let level = element(document, "span", "composer-level")?;
    mark(&level, "level")?;
    append(&level, &element(document, "i", "")?)?;
    append(&live, &level)?;
    append(
        &live,
        &marked_button(
            document,
            "cancel",
            "composer-cancel",
            "Cancel",
            "mic-cancel",
        )?,
    )?;
    append(&bar, &live)?;

    let note = element(document, "div", "composer-note")?;
    mark(&note, "note")?;
    note.set_attribute("role", "status")?;
    append(&bar, &note)?;

    let row = element(document, "div", "composer-row")?;
    append(
        &row,
        &marked_button(document, "mic", "composer-mic", "Talk", "mic")?,
    )?;
    let field = element(document, "input", "composer-field")?;
    mark(&field, "field")?;
    field.set_attribute("type", "text")?;
    field.set_attribute("autocomplete", "off")?;
    field.set_attribute("aria-label", "Say or do something")?;
    field
        .clone()
        .dyn_into::<HtmlInputElement>()?
        .set_value(draft);
    append(&row, &field)?;
    append(
        &row,
        &marked_button(document, "send", "composer-send", "Send", "send")?,
    )?;
    append(&bar, &row)?;

    apply(&bar, view)?;
    Ok(bar)
}

/// Brings the composer on the page up to date with `view`, without touching its box's text.
pub fn sync(view: &ComposerView) -> Result<(), JsValue> {
    let Some(bar) = document()?.query_selector(".composer")? else {
        return Ok(());
    };
    apply(&bar, view)
}

/// Puts `text` in the box (a transcript, or nothing after an answer) and, when asked, gives the
/// box the focus so the person can read and change it.
pub fn set_text(text: &str, focus: bool) -> Result<(), JsValue> {
    let Some(bar) = document()?.query_selector(".composer")? else {
        return Ok(());
    };
    let field = find(&bar, "field")?.dyn_into::<HtmlInputElement>()?;
    field.set_value(text);
    if focus {
        field.focus()?;
    }
    Ok(())
}

fn apply(bar: &Element, view: &ComposerView) -> Result<(), JsValue> {
    let document = document()?;
    bar.set_attribute("data-state", view.state.key())?;

    set_flag(&find(bar, "who")?, "hidden", view.targets.is_empty())?;
    let menu = find(bar, "menu")?;
    let listed = menu.children();
    let same = listed.length() as usize == view.targets.len()
        && view.targets.iter().enumerate().all(|(index, target)| {
            u32::try_from(index)
                .ok()
                .and_then(|index| listed.item(index))
                .and_then(|item| item.get_attribute("value"))
                .is_some_and(|value| value == target.id)
        });
    if !same {
        menu.set_text_content(None);
        for target in &view.targets {
            let chosen = view.selected.as_deref() == Some(target.id.as_str());
            append(&menu, &option(&document, &target.id, &target.name, chosen)?)?;
        }
    }
    if let Some(selected) = &view.selected {
        menu.dyn_into::<HtmlSelectElement>()?.set_value(selected);
    }

    let live = find(bar, "live")?;
    set_flag(&live, "hidden", view.live.is_none())?;
    find(bar, "live-text")?.set_text_content(view.live.as_deref());
    let level = find(bar, "level")?;
    level.set_attribute(
        "style",
        &format!(
            "--level: {}%",
            view.level_permille.min(MAXIMUM_LEVEL_PERMILLE) / 10
        ),
    )?;

    let (note, is_hint) = match (&view.note, &view.mic) {
        (Some(note), _) => (Some(note.as_str()), false),
        (None, MicLook::Unavailable { reason }) => (Some(reason.as_str()), true),
        (None, _) => (None, false),
    };
    let note_element = find(bar, "note")?;
    note_element.set_text_content(note);
    set_flag(&note_element, "hidden", note.is_none())?;
    set_flag(&note_element, "data-hint", is_hint)?;

    let mic = find(bar, "mic")?;
    mic.set_text_content(Some(view.mic.label()));
    mic.set_attribute("title", &view.mic.title())?;
    mic.set_attribute("aria-label", &view.mic.title())?;
    mic.set_attribute(
        "aria-pressed",
        if view.mic == MicLook::Recording {
            "true"
        } else {
            "false"
        },
    )?;
    set_flag(&mic, "disabled", !view.mic.enabled())?;

    let field = find(bar, "field")?.dyn_into::<HtmlInputElement>()?;
    field.set_placeholder(view.field_placeholder);
    field.set_disabled(!view.field_enabled);
    field.set_max_length(i32::try_from(view.field_limit).unwrap_or(i32::MAX));

    let send = find(bar, "send")?;
    send.set_text_content(Some(view.send_label));
    set_flag(&send, "disabled", !view.send_enabled)?;
    Ok(())
}

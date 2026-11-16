//! One player's phone: the hero and goal, what is happening, a private note, the choice list and
//! the composer (`composer`).
//!
//! It turns a `PhoneScene` and the chosen controls into page elements. The choice rules (how many,
//! which groups, what is hidden) come from `choices`; this module only draws what it returns, and
//! it shows every warning, so a fixture that breaks the contract is visible and not repaired.
//!
//! A real phone (`PageRole::Phone`) is drawn with the phone kit (`phone_kit::draw`): the same
//! slots, the kit's look, and a tab bar under the Act panel. The playground's mock phone keeps its
//! own plain look, so the studio page does not change.

use wasm_bindgen::prelude::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element};

use crate::choices::{ChoiceListView, build};
use crate::controls::Controls;
use crate::copy::{PRIVATE_NOTE_TITLE, UNAVAILABLE_TITLE, status_blocks_input, status_copy};
use crate::dom::{append, element, picture, text_element};
use crate::draw_plan::Part;
use crate::fixture::{Choice, PhoneScene, Status};
use crate::phone_kit::act::ActChoices;
use crate::phone_kit::draw::nav::{live_tabs, paint_live};
use crate::phone_kit::draw::{act as kit_act, set_scene, shell as kit_shell};
use crate::phone_kit::shell::{PhoneHeader, Pill, PillTone};
use crate::role::PageRole;
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
            &text_element(document, "div", "choice-group-name", UNAVAILABLE_TITLE)?,
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

/// A real phone's shell: the header and status, one scrolling story (picture, narration, note and
/// choices together), and a dock low on the screen for the composer and the tab bar.
fn player_shell(document: &Document) -> Result<PhoneShell, JsValue> {
    let section = element(document, "section", "device device-phone")?;
    section.set_attribute("aria-label", "Your hero")?;
    let screen = element(document, "div", "ph-screen ph-live")?;
    let story = element(document, "main", "ph-body ph-story")?;
    let dock = element(document, "div", "ph-dock")?;
    let mut slots = Vec::new();
    for part in PARTS {
        let holder = slot(document, part)?;
        match part {
            Part::PhoneImage | Part::PhoneNarration | Part::PhoneNote | Part::PhoneChoices => {
                append(&story, &holder)?;
            }
            Part::PhoneComposer => append(&dock, &holder)?,
            _ => append(&screen, &holder)?,
        }
        slots.push((part, holder));
    }
    append(&dock, &kit_shell::tab_bar(document, &live_tabs())?)?;
    // A page of the bar other than Story and Act is a sample page, drawn here while it is open.
    let sample = element(document, "main", "ph-body ph-sample")?;
    sample.set_attribute("hidden", "")?;
    append(&screen, &story)?;
    append(&screen, &sample)?;
    append(&screen, &dock)?;
    paint_live(document, &screen)?;
    append(&section, &screen)?;
    Ok(PhoneShell { section, slots })
}

/// Builds one phone's shell: the screen with a slot for each piece. The story is one scrolling
/// column that holds the picture and the narration, so a redraw of one keeps the other.
pub fn shell(document: &Document, role: PageRole) -> Result<PhoneShell, JsValue> {
    if role == PageRole::Phone {
        return player_shell(document);
    }
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

/// Puts the shared screen's scene (the backdrop picture) behind a real phone's screen, dimmed by
/// the kit's own fades. The studio's mock phone has no scene.
pub fn paint_scene(section: &Element, role: PageRole, media: Option<&str>) -> Result<(), JsValue> {
    if role != PageRole::Phone {
        return Ok(());
    }
    match section.query_selector(".ph-screen")? {
        Some(screen) => set_scene(&screen, media),
        None => Ok(()),
    }
}

/// The line a real phone (`role::PageRole::Phone`) shows above its screen while its link to the
/// host is down, in the words of the reconnecting status. It starts hidden; the page keeps it and
/// only shows or hides it (`show_link_line`), so a redraw never makes it flicker.
pub fn link_line(document: &Document) -> Result<Element, JsValue> {
    let words = status_copy(&Status::Reconnecting)
        .ok_or_else(|| JsValue::from_str("the reconnecting status has no words"))?;
    let line = kit_shell::link_bar(document, &words)?;
    line.set_attribute("hidden", "")?;
    Ok(line)
}

/// Shows the link line while the link is down, and hides it once the host answers again.
pub fn show_link_line(line: &Element, down: bool) -> Result<(), JsValue> {
    if down {
        line.remove_attribute("hidden")
    } else {
        line.set_attribute("hidden", "")
    }
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
        &text_element(document, "div", "note-title", PRIVATE_NOTE_TITLE)?,
    )?;
    append(
        &card,
        &text_element(document, "div", "note-text", &note.text)?,
    )?;
    Ok(Some(card))
}

/// Once the narration is on the page, scrolls the story to its newest line when that line is out
/// of view. The narration is attached after this returns, so the scroll waits for the next frame.
fn reveal_newest_line() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("the page has no window"))?;
    let reveal = Closure::once_into_js(|| {
        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return;
        };
        let line = document
            .query_selector(".ph-line:last-child")
            .ok()
            .flatten();
        let story = document.query_selector(".ph-story").ok().flatten();
        if let (Some(line), Some(story)) = (line, story) {
            let (line_box, story_box) = (
                line.get_bounding_client_rect(),
                story.get_bounding_client_rect(),
            );
            if line_box.bottom() > story_box.bottom() || line_box.top() < story_box.top() {
                line.scroll_into_view_with_bool(true);
            }
        }
    });
    window.request_animation_frame(reveal.unchecked_ref())?;
    Ok(())
}

/// What goes in the slot of `part` on a real phone, drawn with the phone kit.
fn player_part(
    document: &Document,
    part: Part,
    scene: &PhoneScene,
    controls: &Controls,
) -> Result<Option<Element>, JsValue> {
    match part {
        Part::PhoneHead => {
            let header = PhoneHeader {
                name: scene.hero.clone(),
                subtitle: scene.goal.clone(),
                portrait: None,
                hp: None,
            };
            kit_shell::header(document, &header).map(Some)
        }
        Part::PhoneStatus => {
            let status = controls.status_for(&scene.status);
            let Some(words) = status_copy(&status) else {
                return Ok(None);
            };
            let tone = if status_blocks_input(&status) {
                PillTone::Warn
            } else {
                PillTone::Accent
            };
            let pill = Pill {
                text: words,
                tone,
                glyph: None,
            };
            kit_shell::status(document, &pill).map(Some)
        }
        Part::PhoneImage => scene
            .image
            .as_ref()
            .map(|image| picture(document, "ph-picture", image))
            .transpose(),
        Part::PhoneNarration => {
            let lines = kit_act::narration(document, &scene.narration)?;
            if lines.is_some() {
                reveal_newest_line()?;
            }
            Ok(lines)
        }
        Part::PhoneNote => scene
            .note
            .as_ref()
            .map(|note| kit_act::private_note(document, &note.text))
            .transpose(),
        Part::PhoneChoices => {
            let view = build(scene, controls.show_unavailable);
            let choices = ActChoices::from(&view);
            if choices.is_empty() {
                return Ok(None);
            }
            kit_act::choices(document, &choices).map(Some)
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

/// What goes in the slot of `part`, or `None` when the piece is empty. A part that is not of a
/// phone is an error: the caller asked the wrong view.
pub fn build_part(
    document: &Document,
    part: Part,
    scene: &PhoneScene,
    controls: &Controls,
    role: PageRole,
) -> Result<Option<Element>, JsValue> {
    if role == PageRole::Phone {
        return player_part(document, part, scene, controls);
    }
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

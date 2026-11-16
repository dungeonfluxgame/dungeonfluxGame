//! The Talk panel as elements: the cast strip with portraits, the line that says whom the hero is
//! speaking to, and the conversation as speaker and line.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::phone_kit::draw::glyph;
use crate::phone_kit::glyph::PhoneGlyph;
use crate::phone_kit::talk::TalkPanel;
use crate::view::kit::portrait;

/// The people here, by the name the hero has learned, with the one addressed lit, and the line
/// that says whom the next words go to.
pub fn cast_strip(document: &Document, panel: &TalkPanel) -> Result<Element, JsValue> {
    let wrap = element(document, "section", "ph-cast-wrap")?;
    let strip = element(document, "div", "ph-cast")?;

    let room = element(
        document,
        "button",
        if panel.addressing.is_none() {
            "ph-cast-chip ph-cast-on"
        } else {
            "ph-cast-chip"
        },
    )?;
    room.set_attribute("type", "button")?;
    let room_face = element(document, "span", "ph-cast-portrait ph-cast-room")?;
    append(
        &room_face,
        &glyph(document, PhoneGlyph::Talk, "ph-cast-glyph")?,
    )?;
    append(&room, &room_face)?;
    append(
        &room,
        &text_element(document, "span", "ph-cast-name", "The room")?,
    )?;
    append(&strip, &room)?;

    for member in &panel.cast {
        let chip = element(
            document,
            "button",
            if panel.is_addressed(&member.name) {
                "ph-cast-chip ph-cast-on"
            } else {
                "ph-cast-chip"
            },
        )?;
        chip.set_attribute("type", "button")?;
        append(
            &chip,
            &portrait(
                document,
                "ph-cast-portrait",
                member.portrait.as_deref(),
                &member.name,
            )?,
        )?;
        append(
            &chip,
            &text_element(document, "span", "ph-cast-name", &member.name)?,
        )?;
        if let Some(known) = &member.known_as {
            append(
                &chip,
                &text_element(document, "span", "ph-cast-known", known)?,
            )?;
        }
        append(&strip, &chip)?;
    }
    append(&wrap, &strip)?;
    append(
        &wrap,
        &text_element(document, "p", "ph-address", &panel.addressing_text())?,
    )?;
    Ok(wrap)
}

/// The conversation: each turn has the speaker's name once, then the lines.
pub fn thread(document: &Document, panel: &TalkPanel) -> Result<Element, JsValue> {
    let thread = element(document, "section", "ph-thread")?;
    thread.set_attribute("aria-label", "The conversation")?;
    for turn in panel.turns() {
        let block = element(
            document,
            "div",
            &format!("ph-turn ph-voice-{}", turn.voice.key()),
        )?;
        append(
            &block,
            &text_element(document, "div", "ph-speaker", turn.speaker)?,
        )?;
        for line in turn.lines {
            append(&block, &text_element(document, "p", "ph-said", line)?)?;
        }
        append(&thread, &block)?;
    }
    Ok(thread)
}

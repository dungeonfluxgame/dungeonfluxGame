//! The roleplay card: a title bar, speech bubbles whose tails point at whoever is speaking, and a
//! caption.
//!
//! A pure function of the scene's data (`lobby_model`). A bubble's place and tail come from the
//! scene as percents of the screen and are written to the style as numbers only; where the tail
//! sits on its box is decided by `lobby_model::tail_of`. The caption sits above the party strip,
//! which this card never covers.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, set_style, text_element};
use crate::fixture::TvScene;
use crate::lobby_model::{Bubble, RoleplayScene, tail_of};
use crate::view_kit::{glass, glyph};
use crate::view_lobby::photo;

/// The roleplay card of `scene`, or `None` when it has none.
pub fn build(document: &Document, scene: &TvScene) -> Result<Option<Element>, JsValue> {
    let Some(card) = &scene.roleplay else {
        return Ok(None);
    };
    card_element(document, card).map(Some)
}

fn card_element(document: &Document, card: &RoleplayScene) -> Result<Element, JsValue> {
    let root = element(document, "div", "rp")?;
    if let Some(title) = &card.title {
        let (wrapper, panel) = glass(document, "rp-title-wrap", "rp-title")?;
        append(&panel, &glyph(document, title.glyph, "rp-title-glyph")?)?;
        append(
            &panel,
            &text_element(document, "span", "rp-title-text", &title.text)?,
        )?;
        append(&root, &wrapper)?;
    }
    for bubble in &card.bubbles {
        append(&root, &bubble_element(document, bubble)?)?;
    }
    if let Some(caption) = &card.caption {
        append(&root, &text_element(document, "p", "rp-caption", caption)?)?;
    }
    Ok(root)
}

fn bubble_element(document: &Document, bubble: &Bubble) -> Result<Element, JsValue> {
    let tail = tail_of(bubble);
    let side = if tail.below { "rp-below" } else { "rp-above" };
    let wrapper = element(
        document,
        "div",
        &format!("rp-bubble rp-tone-{} {side}", bubble.tone.key()),
    )?;
    set_style(
        &wrapper,
        &format!(
            "left:{}%;top:{}%;width:{}%;--tail-root:{}%;--tail-tip:{}%;",
            bubble.left_percent,
            bubble.top_percent,
            bubble.width_percent,
            tail.root_percent,
            tail.tip_percent
        ),
    )?;
    let body = element(document, "div", "rp-body")?;
    let head = element(document, "div", "rp-head")?;
    match photo(
        document,
        "rp-face",
        bubble.portrait.as_deref(),
        &bubble.speaker,
    )? {
        Some(face) => append(&head, &face)?,
        None => {
            if let Some(mark) = bubble.glyph {
                append(&head, &glyph(document, mark, "rp-speaker-glyph")?)?;
            }
        }
    }
    append(
        &head,
        &text_element(document, "span", "rp-speaker", &bubble.speaker)?,
    )?;
    append(&body, &head)?;
    append(
        &body,
        &text_element(document, "p", "rp-line", &bubble.line)?,
    )?;
    if let Some(emphasis) = &bubble.emphasis {
        append(
            &body,
            &text_element(document, "p", "rp-line rp-emphasis", emphasis)?,
        )?;
    }
    append(&wrapper, &body)?;
    Ok(wrapper)
}

//! Draws a speech bubble or the thinking cue (`kit::speech`). A bubble is a small pane of fine
//! glass; its halo and bloom are rings of the same outline, tail and all, so the glow follows the
//! tail without a filter on the bubble that would stop its blur.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, set_style, text_element};
use crate::kit::draw::{Shape, glass, icon, thinking_dots};
use crate::kit::speech::{NameStyle, SpeechBubble};
use crate::view::kit::{accent_style, photo};

pub fn draw(document: &Document, bubble: &SpeechBubble) -> Result<Element, JsValue> {
    let thinking = bubble.is_thinking();
    let shape = if thinking {
        Shape::Capsule
    } else {
        Shape::Tailed(bubble.tail)
    };
    let (root, pane) = glass(
        document,
        shape,
        bubble.fill(),
        &format!(
            "kit-bubble kit-tone-{} kit-name-{} kit-lean-{}{}",
            bubble.tone.key(),
            bubble.name_style.key(),
            if bubble.tail_leans_left() {
                "left"
            } else {
                "right"
            },
            if thinking { " kit-thought" } else { "" }
        ),
    )?;
    let mut style = accent_style("--kit-accent", bubble.accent.as_deref());
    if let Some(place) = bubble.drawn_place() {
        style.push_str(&format!(
            "left:{}%;top:{}%;",
            place.left_percent, place.tail_y_percent
        ));
        // The thinking cue is as wide as its dots; a bubble shrinks to its words, at most its
        // place's width, its tail held where the place put it and clear of the chamfers.
        if !thinking {
            style.push_str(&format!(
                "max-width:{}%;--kit-tail-x:clamp(2.2cqw,{:.2}cqw,100% - 2.2cqw);",
                place.width_percent,
                place.tail_offset_percent()
            ));
        }
    }
    if !style.is_empty() {
        set_style(&root, &style)?;
    }
    if thinking {
        root.set_attribute("aria-label", &format!("{} is thinking", bubble.speaker()))?;
        append(&pane, &thinking_dots(document)?)?;
        return Ok(root);
    }
    match bubble.name_style {
        NameStyle::Head => append(&pane, &head(document, bubble, "kit-bubble-head")?)?,
        NameStyle::Tab => append(&root, &head(document, bubble, "kit-bubble-tab")?)?,
    }
    if let Some(line) = bubble.line() {
        append(
            &pane,
            &text_element(document, "p", "kit-bubble-line", line)?,
        )?;
    }
    if let Some(emphasis) = bubble.emphasis() {
        append(
            &pane,
            &text_element(document, "p", "kit-bubble-em", emphasis)?,
        )?;
    }
    Ok(root)
}

/// The speaker's face or glyph and name, as the head row inside the bubble or the tab on its top
/// edge.
fn head(document: &Document, bubble: &SpeechBubble, class: &str) -> Result<Element, JsValue> {
    let head = element(document, "div", class)?;
    match photo(
        document,
        "kit-bubble-face",
        bubble.portrait.as_deref(),
        bubble.speaker(),
    )? {
        Some(face) => append(&head, &face)?,
        None => {
            if let Some(which) = bubble.glyph {
                append(&head, &icon(document, which, "kit-bubble-icon")?)?;
            }
        }
    }
    append(
        &head,
        &text_element(document, "span", "kit-bubble-name", bubble.speaker())?,
    )?;
    Ok(head)
}

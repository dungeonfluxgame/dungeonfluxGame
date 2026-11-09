//! The roleplay card: a title bar, speech bubbles whose tails point at whoever is speaking, and a
//! caption.
//!
//! A pure function of the scene's data (`lobby::model`). A bubble's place and tail come from the
//! scene as percents of the screen and are written to the style as numbers only; where the tail
//! sits on its box is decided by `lobby::model::tail_of`. The caption sits above the party strip,
//! which this card never covers.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, set_style, text_element};
use crate::fixture::TvScene;
use crate::image_layout::{ImageLayout, anchored_place, marker_place, percent_text, speaker_masks};
use crate::lobby::model::{Bubble, BubbleTone, RoleplayScene, tail_of};
use crate::overlay::Glyph;
use crate::view::kit::photo;
use crate::view::kit::{glass, glyph};

/// The roleplay card of `scene`, or `None` when it has none.
pub fn build(document: &Document, scene: &TvScene) -> Result<Option<Element>, JsValue> {
    let Some(card) = &scene.roleplay else {
        return Ok(None);
    };
    let layout = scene.backdrop_layout();
    card_element(document, card, layout).map(Some)
}

fn card_element(
    document: &Document,
    card: &RoleplayScene,
    layout: Option<&ImageLayout>,
) -> Result<Element, JsValue> {
    let root = element(document, "div", "rp")?;
    if let Some(title) = &card.title {
        let (wrapper, panel) = glass(document, "rp-title-wrap", "rp-title")?;
        let glyph_class = if title.glyph == Glyph::Spinner {
            "rp-title-glyph rp-title-spin"
        } else {
            "rp-title-glyph"
        };
        append(&panel, &glyph(document, title.glyph, glyph_class)?)?;
        append(
            &panel,
            &text_element(document, "span", "rp-title-text", &title.text)?,
        )?;
        append(&root, &wrapper)?;
    }
    // A bubble anchored to a character is drawn in the world layer (`anchored`), so it follows the
    // camera; here are only the bubbles that keep their own place on the screen.
    for bubble in &card.bubbles {
        if anchored_place(bubble, layout).is_none() {
            append(&root, &bubble_element(document, bubble, layout)?)?;
        }
    }
    if let Some(caption) = &card.caption {
        append(&root, &text_element(document, "p", "rp-caption", caption)?)?;
    }
    Ok(root)
}

/// The items of `scene`'s roleplay card that are tied to characters of the backdrop's layout (the
/// anchored bubbles and every marker), for the world layer, where they move with the camera and
/// keep one size on the screen (`--cam-scale`); `None` when there are none.
pub fn anchored(document: &Document, scene: &TvScene) -> Result<Option<Element>, JsValue> {
    let (Some(card), Some(layout)) = (&scene.roleplay, scene.backdrop_layout()) else {
        return Ok(None);
    };
    let root = element(document, "div", "tv-anchors")?;
    // Every named face, so the page keeps every bubble off every face (`view::bubble_layout`).
    let faces: Vec<String> = layout
        .regions()
        .iter()
        .filter(|region| region.label().is_some())
        .map(|region| face_text(layout, region))
        .collect();
    root.set_attribute("data-faces", &faces.join(";"))?;
    let mut any = false;
    // The speakers' outlines come first, so every bubble is over them.
    for (character, mask) in speaker_masks(&card.bubbles, Some(layout)) {
        append(&root, &outline_element(document, character, mask)?)?;
        any = true;
    }
    for bubble in &card.bubbles {
        if anchored_place(bubble, Some(layout)).is_some() {
            append(&root, &bubble_element(document, bubble, Some(layout))?)?;
            any = true;
        }
    }
    for marker in &card.markers {
        // A marker whose character is not in the layout is not drawn.
        let Some((x, y)) = marker_place(marker, Some(layout)) else {
            continue;
        };
        let chip = element(document, "div", "rp-marker")?;
        set_style(
            &chip,
            &format!("left:{}%;top:{}%;", percent_text(x), percent_text(y)),
        )?;
        if let Some(mark) = marker.glyph {
            append(&chip, &glyph(document, mark, "rp-marker-glyph")?)?;
        }
        append(
            &chip,
            &text_element(document, "span", "rp-marker-text", &marker.text)?,
        )?;
        append(&root, &chip)?;
        any = true;
    }
    Ok(any.then_some(root))
}

/// The outline of the speaking character `character`, traced from its `mask`: a rim and a glow
/// around the silhouette, outside it, so the figure itself is never covered. The silhouette is
/// cut out of the glow in the theme (`.rp-outline`).
fn outline_element(document: &Document, character: &str, mask: &str) -> Result<Element, JsValue> {
    let outline = element(document, "div", "rp-outline")?;
    outline.set_attribute("data-character", character)?;
    outline.set_attribute("aria-hidden", "true")?;
    let silhouette = element(document, "div", "rp-outline-shape")?;
    set_style(
        &silhouette,
        &format!("-webkit-mask-image:url(\"{mask}\");mask-image:url(\"{mask}\");"),
    )?;
    append(&outline, &silhouette)?;
    set_style(
        &outline,
        &format!(
            "-webkit-mask-image:url(\"{mask}\"),linear-gradient(#000,#000);\
             mask-image:url(\"{mask}\"),linear-gradient(#000,#000);"
        ),
    )?;
    Ok(outline)
}

fn bubble_element(
    document: &Document,
    bubble: &Bubble,
    layout: Option<&ImageLayout>,
) -> Result<Element, JsValue> {
    let placed = anchored_place(bubble, layout);
    let (side, style) = match placed {
        // Anchored: the box's bottom edge is the tail's tip, just above the scanned head point.
        // This is the first place; the page moves it clear of the chrome and the other bubbles
        // (`view::bubble_layout`).
        Some(place) => (
            "rp-below rp-anchored",
            format!(
                "left:{}%;bottom:{}%;width:{}%;--tail-root:{}%;--tail-tip:{}%;",
                percent_text(place.left_permille),
                percent_text(place.bottom_permille),
                percent_text(place.width_permille),
                place.root_percent,
                place.tip_percent
            ),
        ),
        None => {
            let tail = tail_of(bubble);
            (
                if tail.below { "rp-below" } else { "rp-above" },
                format!(
                    "left:{}%;top:{}%;width:{}%;--tail-root:{}%;--tail-tip:{}%;",
                    bubble.left_percent,
                    bubble.top_percent,
                    bubble.width_percent,
                    tail.root_percent,
                    tail.tip_percent
                ),
            )
        }
    };
    let wrapper = element(
        document,
        "div",
        &format!("rp-bubble rp-tone-{} {side}", bubble.tone.key()),
    )?;
    set_style(&wrapper, &style)?;
    if let (Some(place), Some(anchor)) = (placed, &bubble.anchor) {
        // What the page needs to place the bubble again without the layout: the character, the
        // point the tip touches and the tail's first place, in thousandths of the screen.
        wrapper.set_attribute("data-character", &anchor.region)?;
        wrapper.set_attribute("data-tip-x", &place.tip_x_permille.to_string())?;
        wrapper.set_attribute("data-tip-y", &place.tip_y_permille.to_string())?;
        wrapper.set_attribute("data-width", &place.width_permille.to_string())?;
        wrapper.set_attribute("data-lean", &place.tip_percent.to_string())?;
        if let Some(layout) = layout
            && let Some(region) = layout.region(&anchor.region)
        {
            wrapper.set_attribute("data-face", &face_text(layout, region))?;
        }
    }
    // A bubble anchored to a character carries the name its layout region shows (the SAM tag's),
    // so a rename shows on both at once; any other bubble keeps its written speaker.
    let speaker = bubble
        .anchor
        .as_ref()
        .and_then(|anchor| layout?.region(&anchor.region)?.display_name())
        .unwrap_or(&bubble.speaker);
    let body = element(document, "div", "rp-body")?;
    if bubble.tone == BubbleTone::Thought {
        // A thinking indicator shows no words: three pulsing dots, named for a screen reader.
        wrapper.set_attribute("role", "img")?;
        wrapper.set_attribute("aria-label", &format!("{speaker} is thinking"))?;
        let dots = element(document, "div", "rp-think")?;
        for _ in 0..3 {
            append(&dots, &element(document, "span", "rp-think-dot")?)?;
        }
        append(&body, &dots)?;
        append(&wrapper, &body)?;
        return Ok(wrapper);
    }
    let head = element(document, "div", "rp-head")?;
    match photo(document, "rp-face", bubble.portrait.as_deref(), speaker)? {
        Some(face) => append(&head, &face)?,
        None => {
            if let Some(mark) = bubble.glyph {
                append(&head, &glyph(document, mark, "rp-speaker-glyph")?)?;
            }
        }
    }
    append(
        &head,
        &text_element(document, "span", "rp-speaker", speaker)?,
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

/// A character's face box as `left,top,right,bottom` in thousandths of the picture.
fn face_text(layout: &ImageLayout, region: &df_scene_layout::SceneRegion) -> String {
    let picture = layout.picture();
    let face = df_scene_layout::face_box(region, picture.width_px, picture.height_px);
    format!(
        "{},{},{},{}",
        face.left_permille, face.top_permille, face.right_permille, face.bottom_permille
    )
}

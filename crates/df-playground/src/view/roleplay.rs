//! The roleplay card: speech bubbles whose tails point at whoever is speaking, and the
//! lower third: the off-picture speaker's card and the caption bar, or on a close-up the item's
//! title card.
//!
//! A pure function of the scene's data (`lobby::model`). A bubble's place and tail come from the
//! scene as percents of the screen and are written to the style as numbers only; where the tail
//! sits on its box is decided by `lobby::model::tail_of`. The lower third is the kit's
//! (`kit::speaker`, `kit::caption_bar`, `kit::title_card`); the caption sits above the party strip,
//! which this card never covers.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::closeup::{CloseUp, scene_closeup};
use crate::dom::{append, element, set_style, text_element};
use crate::fixture::TvScene;
use crate::home::glyph::HomeIcon;
use crate::image_layout::{ImageLayout, anchored_place, marker_place, percent_text, speaker_masks};
use crate::kit::KitPanels;
use crate::kit::caption_bar::CaptionBar;
use crate::kit::speaker::SpeakerPanel;
use crate::kit::speech::{NameStyle, SpeechBubble, TailLean, TailSide};
use crate::kit::title_card::TitleCard;
use crate::lobby::model::{Bubble, BubbleTone, RoleplayScene, tail_of};
use crate::plate_fit::PlateFit;
use crate::speaker_card::{SpeakerCard, scene_card as speaker_card, scene_twin};
use crate::view::kit::glyph;

/// The roleplay card of `scene`, or `None` when it has none.
pub fn build(document: &Document, scene: &TvScene) -> Result<Option<Element>, JsValue> {
    let Some(card) = &scene.roleplay else {
        return Ok(None);
    };
    let layout = scene.backdrop_layout();
    card_element(
        document,
        card,
        layout,
        speaker_card(scene).or_else(|| scene_twin(scene)).as_ref(),
        scene_closeup(scene).as_ref(),
    )
    .map(Some)
}

fn card_element(
    document: &Document,
    card: &RoleplayScene,
    layout: Option<&ImageLayout>,
    speaker: Option<&SpeakerCard>,
    closeup: Option<&CloseUp>,
) -> Result<Element, JsValue> {
    let root = element(document, "div", "rp")?;
    // A bubble anchored to a character is drawn in the world layer (`anchored`), so it follows the
    // camera; here are only the bubbles that keep their own place on the screen.
    // A bubble anchored to a character who is not in the picture is not drawn: that speaker has
    // the card below instead.
    for bubble in &card.bubbles {
        if bubble.anchor.is_none() {
            append(&root, &bubble_element(document, bubble, layout)?)?;
        }
    }
    if let Some(lower_third) = lower_third(document, card, speaker, closeup)? {
        append(&root, &lower_third)?;
    }
    Ok(root)
}

/// The kit layer of the card's lower third, or `None` when it has nothing in it. On a close-up the
/// caption is the item's title card, not the bar (`closeup`). The speaker card and the title card
/// carry their keys, which say whether a later draw shows the same card again
/// (`overlays::carry_over`).
fn lower_third(
    document: &Document,
    card: &RoleplayScene,
    speaker: Option<&SpeakerCard>,
    closeup: Option<&CloseUp>,
) -> Result<Option<Element>, JsValue> {
    let layer = crate::kit::draw::panels(
        document,
        &KitPanels {
            speaker: speaker.map(SpeakerPanel::from),
            caption: match closeup {
                Some(_) => None,
                None => CaptionBar::from_roleplay(card),
            },
            title_cards: closeup.map(TitleCard::from).into_iter().collect(),
            ..KitPanels::default()
        },
        None,
    )?;
    if layer.child_element_count() == 0 {
        return Ok(None);
    }
    if let (Some(root), Some(speaker)) = (layer.query_selector(".kit-speaker")?, speaker) {
        root.set_attribute("data-character", &speaker.character)?;
        root.set_attribute("data-card", &speaker.card_identity())?;
        root.set_attribute("role", "status")?;
    }
    if let (Some(root), Some(closeup)) = (layer.query_selector(".kit-tc")?, closeup) {
        root.set_attribute("data-closeup", &closeup.card_identity())?;
        root.set_attribute("role", "status")?;
    }
    Ok(Some(layer))
}

/// The items of `scene`'s roleplay card that are tied to characters of the backdrop's layout (the
/// anchored bubbles and every marker), for the world layer, where they move with the camera and
/// keep one size on the screen (`--cam-scale`); `None` when there are none.
pub fn anchored(document: &Document, scene: &TvScene) -> Result<Option<Element>, JsValue> {
    let (Some(card), Some(layout)) = (&scene.roleplay, scene.backdrop_layout()) else {
        return Ok(None);
    };
    let root = element(document, "div", "tv-anchors")?;
    let mut any = false;
    let fit = PlateFit::of_picture(layout.picture());
    // The speakers' outlines come first, so every bubble is over them.
    for (character, mask) in speaker_masks(&card.bubbles, Some(layout)) {
        append(&root, &outline_element(document, character, mask, fit)?)?;
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
/// cut out of the glow in the theme (`.rp-outline`). The mask is the size of the picture, so the
/// outline lies where the picture does (`fit`).
fn outline_element(
    document: &Document,
    character: &str,
    mask: &str,
    fit: PlateFit,
) -> Result<Element, JsValue> {
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
            "{}-webkit-mask-image:url(\"{mask}\"),linear-gradient(#000,#000);\
             mask-image:url(\"{mask}\"),linear-gradient(#000,#000);",
            fit.box_style()
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
    let (side, style, tail, lean) = match placed {
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
            TailSide::Below,
            lean_of(place.root_percent, place.tip_percent),
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
                if tail.below {
                    TailSide::Below
                } else {
                    TailSide::Above
                },
                lean_of(tail.root_percent, tail.tip_percent),
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
    if bubble.tone == BubbleTone::Thought {
        // A thinking indicator shows no words: the kit's cue (three pulsing dots), named for a
        // screen reader. The wrapper keeps the bubble's anchoring and placement.
        wrapper.set_attribute("role", "img")?;
        wrapper.set_attribute("aria-label", &format!("{speaker} is thinking"))?;
        let cue = SpeechBubble {
            speaker: speaker.to_owned(),
            line: None,
            emphasis: None,
            glyph: None,
            portrait: None,
            tone: BubbleTone::Thought,
            accent: None,
            name_style: NameStyle::Head,
            tail: TailSide::Below,
            lean: None,
            place: None,
        };
        append(
            &wrapper,
            &crate::kit::draw::panels(
                document,
                &KitPanels {
                    bubbles: vec![cue],
                    ..KitPanels::default()
                },
                None,
            )?,
        )?;
        return Ok(wrapper);
    }
    // The kit draws the bubble; the page's layout still places the frame and, as it changes the
    // side, keeps the kit's tail shape in step (`view::bubble_layout`).
    let said = SpeechBubble {
        speaker: speaker.to_owned(),
        line: Some(bubble.line.clone()),
        emphasis: bubble.emphasis.clone(),
        glyph: bubble.glyph.map(HomeIcon::Overlay),
        portrait: bubble.portrait.clone(),
        tone: bubble.tone,
        accent: None,
        name_style: NameStyle::Head,
        tail,
        lean: Some(lean),
        place: None,
    };
    append(
        &wrapper,
        &crate::kit::draw::panels(
            document,
            &KitPanels {
                bubbles: vec![said],
                ..KitPanels::default()
            },
            None,
        )?,
    )?;
    Ok(wrapper)
}

/// Which way a tail's tip leans: toward the start of its edge when it is before the root.
fn lean_of(root_percent: u8, tip_percent: u8) -> TailLean {
    if tip_percent < root_percent {
        TailLean::Left
    } else {
        TailLean::Right
    }
}

/// Every named character's face box of `layout` as `face_text` gives it, joined with `;`: what the
/// page keeps every bubble and the clue board off (`view::bubble_layout`).
pub fn named_faces(layout: &ImageLayout) -> String {
    layout
        .regions()
        .iter()
        .filter(|region| region.label().is_some())
        .map(|region| face_text(layout, region))
        .collect::<Vec<_>>()
        .join(";")
}

/// A character's face box as `left,top,right,bottom` in thousandths of the screen, where the
/// picture's fit puts it.
fn face_text(layout: &ImageLayout, region: &df_scene_layout::SceneRegion) -> String {
    let picture = layout.picture();
    let face = df_scene_layout::face_box(region, picture.width_px, picture.height_px);
    let (left, top, right, bottom) = PlateFit::of_picture(picture).stage_box_permille((
        face.left_permille,
        face.top_permille,
        face.right_permille,
        face.bottom_permille,
    ));
    format!("{left},{top},{right},{bottom}")
}

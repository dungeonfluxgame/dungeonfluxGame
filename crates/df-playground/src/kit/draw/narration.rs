//! Draws the narration box and its twist tone (`kit::narration`).

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::home::glyph::{HomeGlyph, HomeIcon};
use crate::kit::Fill;
use crate::kit::draw::{Shape, diamond_rule, glass, icon, voice_wave};
use crate::kit::narration::{ContinueHint, ContinueStyle, Narration, NarrationHeader};

pub fn draw(document: &Document, narration: &Narration) -> Result<Element, JsValue> {
    let (root, pane) = glass(
        document,
        Shape::Octagon,
        Fill::Blurred,
        &format!(
            "kit-narration kit-narration-{} kit-tone-{}",
            narration.placement.key(),
            narration.tone.key()
        ),
    )?;
    if let Some(badge) = narration.badge {
        let medal = element(document, "div", "kit-medal")?;
        append(&medal, &icon(document, badge, "kit-medal-icon")?)?;
        append(&pane, &medal)?;
    }
    let main = element(document, "div", "kit-nar-main")?;
    append(&main, &head(document, narration)?)?;
    append(&main, &line(document, narration)?)?;
    if let Some(emphasis) = narration.emphasis() {
        append(
            &main,
            &text_element(document, "p", "kit-nar-emphasis", emphasis)?,
        )?;
    }
    append(&pane, &main)?;
    if let Some(hint) = narration.continue_hint() {
        append(&pane, &continue_hint(document, hint)?)?;
    }
    Ok(root)
}

/// The header: the speaker and the voice wave, moving while the line is spoken and still when it
/// is not, then the rule running on, and the scene count at the right. The tab style puts the name
/// and then the wave in a hairline tab whose foot the rule continues (render 02); the plain style
/// puts the wave before the name (renders 13, 04 and 35), and the compact box has an italic header
/// and no rule (render 04).
fn head(document: &Document, narration: &Narration) -> Result<Element, JsValue> {
    let style = narration.header_style();
    let head = element(
        document,
        "div",
        &format!("kit-nar-head kit-nar-head-{}", style.key()),
    )?;
    let speaker = text_element(document, "span", "kit-nar-speaker", narration.speaker())?;
    let wave = voice_wave(document, narration.voice)?;
    match style {
        NarrationHeader::Tab => {
            let tab = element(document, "span", "kit-nar-tab")?;
            append(&tab, &speaker)?;
            append(&tab, &wave)?;
            append(&head, &tab)?;
        }
        NarrationHeader::Plain => {
            append(&head, &wave)?;
            append(&head, &speaker)?;
        }
    }
    if !narration.is_compact() {
        append(&head, &diamond_rule(document, "kit-nar-rule")?)?;
    }
    if let Some(progress) = narration.progress_text() {
        append(
            &head,
            &text_element(document, "span", "kit-nar-progress", &progress)?,
        )?;
    }
    Ok(head)
}

/// The line as its pieces, the marked ones in gold.
fn line(document: &Document, narration: &Narration) -> Result<Element, JsValue> {
    let line = element(document, "p", "kit-nar-line")?;
    for segment in narration.segments() {
        let class = if segment.highlight { "kit-hl" } else { "" };
        append(
            &line,
            &text_element(document, "span", class, &segment.text)?,
        )?;
    }
    Ok(line)
}

fn continue_hint(document: &Document, hint: ContinueHint<'_>) -> Result<Element, JsValue> {
    let mark = element(
        document,
        "div",
        &format!("kit-continue kit-continue-{}", hint.style.key()),
    )?;
    mark.set_attribute("aria-label", hint.label)?;
    let chevron = icon(
        document,
        HomeIcon::Page(HomeGlyph::ChevronDown),
        "kit-continue-icon",
    )?;
    match hint.style {
        ContinueStyle::Chevron => append(&mark, &chevron)?,
        ContinueStyle::Ring => {
            let ring = element(document, "span", "kit-ring-mark")?;
            append(&ring, &chevron)?;
            append(&mark, &ring)?;
            append(
                &mark,
                &text_element(document, "span", "kit-continue-label", hint.label)?,
            )?;
        }
    }
    Ok(mark)
}

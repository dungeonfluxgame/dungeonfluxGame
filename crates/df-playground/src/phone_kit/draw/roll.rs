//! The Roll panel as elements: the check card, the die with the result, and the big Roll button
//! low on the screen.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::fixture::RollMode;
use crate::kit::roll_card::{RollCard, Tone, outcome_word};
use crate::phone_kit::draw::glyph;
use crate::phone_kit::glyph::PhoneGlyph;
use crate::phone_kit::roll::{RollPanel, RollState, kept_text, sum_text};

fn die(document: &Document, state: &RollState) -> Result<Element, JsValue> {
    let wrap = element(document, "div", "ph-result")?;
    let (face, class) = match state {
        RollState::Ready => ("20".to_owned(), "ph-die"),
        RollState::Rolling => ("?".to_owned(), "ph-die ph-die-rolling"),
        RollState::Done(result) => (RollCard::from(result).total_text(), "ph-die ph-die-done"),
    };
    let shape = element(document, "div", class)?;
    shape.set_attribute("aria-hidden", "true")?;
    append(
        &shape,
        &text_element(document, "span", "ph-die-face", &face)?,
    )?;
    append(&wrap, &shape)?;
    if let RollState::Done(result) = state {
        let words = element(document, "div", "ph-result-words")?;
        let total = text_element(
            document,
            "p",
            "ph-total",
            &format!("You rolled {}", RollCard::from(result).total_text()),
        )?;
        total.set_attribute("role", "status")?;
        append(&words, &total)?;
        append(
            &words,
            &text_element(document, "p", "ph-sum", &sum_text(result))?,
        )?;
        if let Some(kept) = kept_text(result) {
            append(&words, &text_element(document, "p", "ph-kept", &kept)?)?;
        }
        if let Some((word, tone)) = result.outcome.map(outcome_word) {
            let class = match tone {
                Tone::Positive => "good",
                Tone::Negative => "danger",
                Tone::Neutral => "neutral",
            };
            append(
                &words,
                &text_element(
                    document,
                    "span",
                    &format!("ph-pill ph-outcome ph-pill-{class}"),
                    word,
                )?,
            )?;
        }
        append(&wrap, &words)?;
    }
    Ok(wrap)
}

/// The panel for the check that was called.
pub fn panel(document: &Document, panel: &RollPanel) -> Result<Element, JsValue> {
    let section = element(document, "section", "ph-roll")?;
    let card = element(document, "div", "ph-glass ph-roll-card")?;
    let title = element(document, "div", "ph-roll-title")?;
    append(&title, &glyph(document, PhoneGlyph::Roll, "ph-roll-glyph")?)?;
    append(
        &title,
        &text_element(document, "h2", "ph-check", &panel.check)?,
    )?;
    append(&card, &title)?;
    if let Some(basis) = &panel.basis {
        append(&card, &text_element(document, "p", "ph-basis", basis)?)?;
    }
    append(
        &card,
        &text_element(document, "p", "ph-stakes", &panel.stakes)?,
    )?;
    if let Some(words) = panel.mode_pill() {
        let tone = match panel.mode {
            RollMode::Disadvantage => "warn",
            RollMode::Advantage | RollMode::Normal => "good",
        };
        append(
            &card,
            &text_element(document, "span", &format!("ph-pill ph-pill-{tone}"), &words)?,
        )?;
    }
    if let Some(reason) = &panel.mode_reason {
        append(
            &card,
            &text_element(document, "p", "ph-edge-reason", reason)?,
        )?;
    }
    append(&section, &card)?;
    append(&section, &die(document, &panel.state)?)?;
    let roll = text_element(
        document,
        "button",
        "ph-btn ph-btn-primary ph-btn-big",
        &panel.button_label(),
    )?;
    roll.set_attribute("type", "button")?;
    if !panel.can_roll() {
        roll.set_attribute("disabled", "")?;
    }
    append(&section, &roll)?;
    Ok(section)
}

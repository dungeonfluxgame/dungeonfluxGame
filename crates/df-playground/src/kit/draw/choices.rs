//! Draws the story choice panel (`kit::choices`). Each tile is a small pane of fine glass, its
//! icon (or the empty slot of one) in a column of its own, the title over the description beside
//! it and the pips at the end, so every choice's words start on one line (render 04).

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, set_style, text_element};
use crate::kit::choices::{ChoicePanel, DrawnChoice};
use crate::kit::draw::{Shape, glass, icon, put};

/// The panel, or `None` when no choice has a label.
pub fn draw(document: &Document, panel: &ChoicePanel) -> Result<Option<Element>, JsValue> {
    let choices = panel.drawn();
    if choices.is_empty() {
        return Ok(None);
    }
    let root = element(document, "div", &panel.class())?;
    if let Some(prompt) = panel.prompt() {
        append(
            &root,
            &text_element(document, "p", "kit-choices-prompt", prompt)?,
        )?;
    }
    let list = element(document, "div", "kit-choice-list")?;
    list.set_attribute("role", "list")?;
    let icon_column = panel.reserves_icon_column();
    for choice in &choices {
        append(
            &list,
            &choice_element(document, panel, choice, icon_column)?,
        )?;
    }
    append(&root, &list)?;
    Ok(Some(root))
}

fn choice_element(
    document: &Document,
    panel: &ChoicePanel,
    drawn: &DrawnChoice<'_>,
    icon_column: bool,
) -> Result<Element, JsValue> {
    let (root, pane) = glass(
        document,
        Shape::Octagon,
        panel.fill(),
        &format!(
            "kit-choice kit-look-{}{}",
            drawn.choice.look.key(),
            if drawn.dimmed { " kit-dimmed" } else { "" }
        ),
    )?;
    root.set_attribute("role", "listitem")?;
    // The pip count lets a narrow card keep its last line of words clear of the pips.
    set_style(
        &root,
        &format!(
            "--kit-order:{};--kit-votes:{}",
            drawn.order,
            drawn.choice.votes.unwrap_or(0)
        ),
    )?;
    // The icon, or an empty slot of its width when another choice of the list has one.
    let mark = match drawn.choice.icon {
        Some(which) => Some(icon(document, which, "kit-choice-icon")?),
        None if icon_column => {
            let slot = element(document, "span", "kit-choice-icon kit-choice-icon-slot")?;
            slot.set_attribute("aria-hidden", "true")?;
            Some(slot)
        }
        None => None,
    };
    let label = text_element(document, "span", "kit-choice-label", drawn.label)?;
    let description = drawn
        .description
        .map(|description| text_element(document, "span", "kit-choice-desc", description))
        .transpose()?;
    let text = element(document, "div", "kit-choice-text")?;
    let pips = votes(document, drawn)?;
    put(&pane, mark)?;
    append(&text, &label)?;
    put(&text, description)?;
    append(&pane, &text)?;
    // A row's pips stand at its right end, a card's in its top-right corner.
    if pips.child_element_count() > 0 {
        append(&pane, &pips)?;
    }
    Ok(root)
}

/// One gold pip per seat leaning this way.
fn votes(document: &Document, drawn: &DrawnChoice<'_>) -> Result<Element, JsValue> {
    let pips = element(document, "span", "kit-votes")?;
    if let Some(votes) = drawn.choice.votes.filter(|votes| *votes > 0) {
        pips.set_attribute("aria-label", &format!("{votes} leaning this way"))?;
        for _ in 0..votes {
            append(&pips, &element(document, "span", "kit-vote")?)?;
        }
    }
    Ok(pips)
}

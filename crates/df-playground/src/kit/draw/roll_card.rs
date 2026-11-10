//! Draws the roll card (`kit::roll_card`): the title and the target, the dice on their tray, the
//! total and the written sum, each staged by CSS animations whose delays come from the schedule.
//!
//! The card is a solid glass panel, so the faces and the sum are always read on a plate. The dice
//! are the art's pictures with the numerals drawn over them in the page's own font; a die whose
//! picture fails still shows its face on a plain shape. A face stays hidden until the die has
//! settled, so the tumble never shows a number that is not the server's.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, set_style, text_element};
use crate::fixture::NaturalMark;
use crate::kit::Fill;
use crate::kit::draw::{Shape, glass};
use crate::kit::roll_card::{
    COUNT_SIGN, DieSlot, MINUS, PENDING_MARK, RollCard, Term, VERDICT_CHARACTERS_BEFORE_SMALLER,
    die_vars,
};
use crate::kit::roll_tray::{Tally, TrayLayout};
use crate::roll::{DieTumble, TumblePlan};

/// The card, drawn for the roll `card`, its dice tumbling as `plan` says.
pub fn draw(document: &Document, card: &RollCard, plan: &TumblePlan) -> Result<Element, JsValue> {
    let slots = card.slots();
    let mut class = String::from("kit-roll");
    if let Some((_, tone)) = card.verdict() {
        class.push_str(&format!(" kit-roll-tone-{}", tone.key()));
    }
    match card.view.natural {
        NaturalMark::Twenty => class.push_str(" kit-roll-crit"),
        NaturalMark::One => class.push_str(" kit-roll-fumble"),
        NaturalMark::Unmarked => {}
    }
    let (root, pane) = glass(document, Shape::Octagon, Fill::Solid, &class)?;
    root.set_attribute("aria-label", &card.summary())?;
    root.set_attribute("role", "img")?;
    root.set_attribute("data-dice", &slots.len().to_string())?;
    let fit = card.fit_percent();
    if fit < 100 {
        set_style(&root, &format!("--roll-fit:{};", f64::from(fit) / 100.0))?;
    }
    append(&pane, &element(document, "span", "kit-roll-wash")?)?;
    append(&pane, &head(document, card)?)?;
    if !slots.is_empty() {
        append(&pane, &tray(document, &slots, plan)?)?;
    }
    append(&pane, &total(document, card)?)?;
    append(&pane, &foot(document, card)?)?;
    if let Some(seal) = seal(document, card)? {
        append(&root, &seal)?;
    }
    for stage in ["call", "tumble", "sum", "exit"] {
        append(&root, &clock(document, stage)?)?;
    }
    Ok(root)
}

/// A stage's timer: an invisible element whose animation begins and ends with the stage, so the
/// page logs the stage from the browser's own animation events and not from a second clock.
fn clock(document: &Document, stage: &str) -> Result<Element, JsValue> {
    let clock = element(document, "span", &format!("roll-clock roll-clock-{stage}"))?;
    clock.set_attribute("aria-hidden", "true")?;
    Ok(clock)
}

fn head(document: &Document, card: &RollCard) -> Result<Element, JsValue> {
    let head = element(document, "div", "kit-roll-head")?;
    let heading = element(document, "div", "kit-roll-heading")?;
    let marks = element(document, "div", "kit-roll-marks")?;
    if let Some(kicker) = card.kicker() {
        append(
            &marks,
            &text_element(document, "span", "kit-roll-kicker", kicker)?,
        )?;
    }
    let chips = [
        card.mode_word().map(|word| {
            (
                format!("kit-roll-mode kit-roll-mode-{}", card.view.mode.key()),
                word.to_owned(),
            )
        }),
        card.inspiration_word()
            .map(|word| ("kit-roll-mode kit-roll-mode-inspiration".to_owned(), word)),
        card.natural_word().map(|word| {
            let tone = if card.view.natural == NaturalMark::Twenty {
                "high"
            } else {
                "low"
            };
            (
                format!("kit-roll-mode kit-roll-natural-{tone}"),
                word.to_owned(),
            )
        }),
        card.dice_word()
            .map(|word| ("kit-roll-mode kit-roll-mode-dice".to_owned(), word)),
    ];
    for (class, word) in chips.into_iter().flatten() {
        append(&marks, &text_element(document, "span", &class, &word)?)?;
    }
    append(&heading, &marks)?;
    let title_class = if card.title_is_long() {
        "kit-roll-title kit-roll-title-long"
    } else {
        "kit-roll-title"
    };
    append(
        &heading,
        &text_element(document, "h2", title_class, card.title())?,
    )?;
    append(&head, &heading)?;
    Ok(head)
}

fn tray(document: &Document, slots: &[DieSlot], plan: &TumblePlan) -> Result<Element, JsValue> {
    let tray = element(document, "div", "kit-roll-tray")?;
    let toss = |index: usize| plan.dice.get(index % plan.dice.len().max(1));
    match TrayLayout::of(slots) {
        TrayLayout::Dice(grid) => {
            set_style(&tray, &grid.tray_vars())?;
            let dice = element(document, "div", "kit-roll-dice")?;
            set_style(&dice, &grid.vars())?;
            for (index, slot) in slots.iter().enumerate() {
                append(&dice, &die(document, slot, toss(index), true)?)?;
            }
            append(&tray, &dice)?;
        }
        TrayLayout::Tally(tally) => {
            tray.set_attribute("class", "kit-roll-tray kit-roll-tray-tally")?;
            let cluster = element(document, "div", "kit-roll-cluster")?;
            set_style(&cluster, &tally.vars())?;
            for (index, art) in tally.cluster.iter().enumerate() {
                let slot = DieSlot {
                    art: *art,
                    numeral: String::new(),
                    state: crate::kit::roll_card::DieState::Counted,
                    caption: None,
                };
                append(&cluster, &die(document, &slot, toss(index), false)?)?;
            }
            append(&tray, &cluster)?;
            append(&tray, &chips(document, &tally)?)?;
        }
    }
    Ok(tray)
}

/// The faces counted: a chip for each face, the largest first, with how many dice showed it.
fn chips(document: &Document, tally: &Tally) -> Result<Element, JsValue> {
    let chips = element(document, "div", "kit-roll-tally")?;
    set_style(&chips, &tally.vars())?;
    for chip in &tally.chips {
        let class = format!("kit-roll-chip kit-roll-chip-{}", chip.state.key());
        let node = element(document, "span", &class)?;
        append(
            &node,
            &text_element(document, "span", "kit-roll-chip-face", &chip.face)?,
        )?;
        if chip.count > 1 {
            append(
                &node,
                &text_element(
                    document,
                    "span",
                    "kit-roll-chip-count",
                    &format!("{COUNT_SIGN}{}", chip.count),
                )?,
            )?;
        }
        append(&chips, &node)?;
    }
    Ok(chips)
}

/// One die: the art over a plain shape, a highlight, the face and what is under it. A cluster die
/// (`with_face` false) is only the art, tumbling beside a tally.
fn die(
    document: &Document,
    slot: &DieSlot,
    tumble: Option<&DieTumble>,
    with_face: bool,
) -> Result<Element, JsValue> {
    let decor = if with_face { "" } else { " kit-roll-die-decor" };
    let die = element(
        document,
        "div",
        &format!(
            "kit-roll-die kit-roll-die-{} kit-roll-art-{}{decor}",
            slot.state.key(),
            slot.art.key()
        ),
    )?;
    set_style(&die, &die_vars(slot.art, tumble))?;
    let hop = element(document, "span", "kit-roll-hop")?;
    append(&hop, &element(document, "span", "kit-roll-base")?)?;
    let art = element(document, "img", "kit-roll-art")?;
    art.set_attribute(
        "src",
        &crate::path::dice_file(&format!("{}.png", slot.art.key())),
    )?;
    art.set_attribute("data-die", slot.art.key())?;
    art.set_attribute("alt", "")?;
    art.set_attribute("draggable", "false")?;
    append(&hop, &art)?;
    append(&hop, &element(document, "span", "kit-roll-shine")?)?;
    if with_face {
        append(&hop, &element(document, "span", "kit-roll-strike")?)?;
        let wide = if slot.numeral.chars().count() > 1 {
            " kit-roll-face-wide"
        } else {
            ""
        };
        append(
            &hop,
            &text_element(
                document,
                "span",
                &format!("kit-roll-face{wide}"),
                &slot.numeral,
            )?,
        )?;
    }
    append(&die, &hop)?;
    if let Some(caption) = slot.caption {
        append(
            &die,
            &text_element(document, "span", "kit-roll-caption", caption)?,
        )?;
    }
    Ok(die)
}

fn total(document: &Document, card: &RollCard) -> Result<Element, JsValue> {
    let long = if card.target_is_long() {
        " kit-roll-total-long"
    } else {
        ""
    };
    let total = element(document, "div", &format!("kit-roll-total{long}"))?;
    append(&total, &element(document, "span", "kit-roll-aura")?)?;
    let wide = if card.total_is_wide() {
        " kit-roll-total-wide"
    } else {
        ""
    };
    append(
        &total,
        &text_element(
            document,
            "span",
            &format!("kit-roll-total-number{wide}"),
            &card.total_text(),
        )?,
    )?;
    if let Some(target) = card.target_text() {
        let against = element(document, "span", "kit-roll-against")?;
        append(
            &against,
            &text_element(document, "span", "kit-roll-against-vs", "vs")?,
        )?;
        append(&against, &text_element(document, "span", "", &target)?)?;
        append(&total, &against)?;
    }
    append(
        &total,
        &text_element(
            document,
            "span",
            "kit-roll-pending",
            &card
                .target_text()
                .unwrap_or_else(|| PENDING_MARK.to_owned()),
        )?,
    )?;
    Ok(total)
}

fn foot(document: &Document, card: &RollCard) -> Result<Element, JsValue> {
    let foot = element(document, "div", "kit-roll-foot")?;
    if card.writes_a_sum() {
        let long = if card.sum_is_long() {
            " kit-roll-sum-long"
        } else {
            ""
        };
        let sum = element(document, "p", &format!("kit-roll-sum{long}"))?;
        for (index, term) in card.terms().iter().enumerate() {
            append(&sum, &term_element(document, index, term)?)?;
        }
        let ending = element(document, "span", "kit-roll-term kit-roll-ending")?;
        append(
            &ending,
            &text_element(document, "span", "kit-roll-op kit-roll-equals", "=")?,
        )?;
        append(
            &ending,
            &text_element(document, "span", "kit-roll-sum-total", &card.total_text())?,
        )?;
        append(&sum, &ending)?;
        append(&foot, &sum)?;
    }
    Ok(foot)
}

/// The verdict as a seal over the panel's foot. It is a child of the glass and not of its pane:
/// the pane's mask would cut it at the panel's edge.
fn seal(document: &Document, card: &RollCard) -> Result<Option<Element>, JsValue> {
    let Some((word, tone)) = card.verdict() else {
        return Ok(None);
    };
    let long = if word.chars().count() > VERDICT_CHARACTERS_BEFORE_SMALLER {
        " kit-roll-verdict-long"
    } else {
        ""
    };
    let seal = element(
        document,
        "span",
        &format!("kit-roll-verdict kit-roll-verdict-{}{long}", tone.key()),
    )?;
    append(
        &seal,
        &text_element(document, "span", "kit-roll-verdict-word", word)?,
    )?;
    Ok(Some(seal))
}

/// One term of the written sum: its sign (a first term has none unless it is taken away), its
/// number and what it is.
fn term_element(document: &Document, index: usize, term: &Term) -> Result<Element, JsValue> {
    let span = element(document, "span", "kit-roll-term")?;
    if index > 0 || term.negative {
        let sign = if index > 0 && !term.negative {
            '+'
        } else {
            MINUS
        };
        append(
            &span,
            &text_element(document, "span", "kit-roll-op", &sign.to_string())?,
        )?;
    }
    append(
        &span,
        &text_element(document, "span", "kit-roll-number", &term.number)?,
    )?;
    if let Some(label) = &term.label {
        append(
            &span,
            &text_element(document, "span", "kit-roll-term-label", label)?,
        )?;
    }
    Ok(span)
}

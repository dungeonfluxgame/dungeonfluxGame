//! Draws a loot or shop list (`kit::ledger`): on glass, or floating on the plate.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, set_style, text_element};
use crate::kit::draw::{Shape, diamond_rule, glass, icon};
use crate::kit::ledger::{Ledger, LedgerKind, LedgerRow};
use crate::overlay::Glyph;
use crate::view::kit::{glyph, photo};

/// The list, or `None` when no row has a name.
pub fn draw(document: &Document, ledger: &Ledger) -> Result<Option<Element>, JsValue> {
    let rows = ledger.rows();
    if rows.is_empty() {
        return Ok(None);
    }
    let class = format!("kit-ledger kit-ledger-{}", ledger.kind.key());
    let (root, pane) = if ledger.framed {
        glass(document, Shape::Octagon, ledger.fill(), &class)?
    } else {
        let root = element(document, "div", &format!("{class} kit-ledger-floating"))?;
        let pane = element(document, "div", "kit-ledger-body")?;
        append(&root, &pane)?;
        (root, pane)
    };
    if ledger.title().is_some() || ledger.icon.is_some() {
        let head = element(document, "div", "kit-ledger-head")?;
        if let Some(which) = ledger.icon {
            append(&head, &icon(document, which, "kit-ledger-icon")?)?;
        }
        if let Some(title) = ledger.title() {
            append(
                &head,
                &text_element(document, "h2", "kit-ledger-title", title)?,
            )?;
        }
        append(&pane, &head)?;
        append(&pane, &diamond_rule(document, "kit-ledger-rule")?)?;
    }
    let list = element(document, "ul", "kit-rows")?;
    for row in &rows {
        append(&list, &row_element(document, ledger.kind, row)?)?;
    }
    append(&pane, &list)?;
    Ok(Some(root))
}

fn row_element(
    document: &Document,
    kind: LedgerKind,
    row: &LedgerRow<'_>,
) -> Result<Element, JsValue> {
    let mut class = String::from("kit-row");
    if row.item.taken {
        class.push_str(" kit-row-taken");
    }
    if row.item.highlight {
        class.push_str(" kit-row-lit");
    }
    let item = element(document, "li", &class)?;
    set_style(&item, &format!("--kit-order:{}", row.order))?;
    if kind == LedgerKind::Loot {
        let mark = element(document, "span", "kit-row-mark")?;
        match photo(
            document,
            "kit-row-picture",
            row.item.picture.as_deref(),
            row.name,
        )? {
            Some(picture) => append(&mark, &picture)?,
            None => {
                if let Some(which) = row.item.icon {
                    append(&mark, &icon(document, which, "kit-row-icon")?)?;
                }
            }
        }
        append(&item, &mark)?;
    }
    let name_class = match row.item.rarity {
        Some(rarity) => format!("kit-row-name kit-rarity-{}", rarity.key()),
        None => String::from("kit-row-name"),
    };
    append(
        &item,
        &text_element(document, "span", &name_class, row.name)?,
    )?;
    if kind == LedgerKind::Shop {
        append(&item, &element(document, "span", "kit-row-leader")?)?;
    }
    if let Some(figure) = row.figure {
        append(
            &item,
            &text_element(document, "span", "kit-row-figure", figure)?,
        )?;
    }
    if row.item.taken {
        append(&item, &glyph(document, Glyph::Check, "kit-row-check")?)?;
    }
    Ok(item)
}

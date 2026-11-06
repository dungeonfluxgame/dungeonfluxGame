//! The landing page's brand mark, title block, navigation rail and top bar, each a pure function
//! of its data (`home::chrome`, `home::nav`).
//!
//! The shared screen takes no input, so the rail and the bell are pictures of what the phones
//! show. Text from a scene is set as text, a drawing is a constant of this crate, and a number
//! reaches a style only as a number.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::fixture::TvScene;
use crate::home::chrome::{BrandMark, BrandSize, HomeTopBar, TitleBlock, bell_label, unread_text};
use crate::home::glyph::HomeGlyph;
use crate::home::nav::NavRail;
use crate::view::home::kit::{diamond_rule, icon, page_glyph};
use crate::view::icons::{Icon, icon as compass_icon};

/// The logo: the compass star over the wordmark, or the big wordmark alone.
pub fn brand(document: &Document, brand: &BrandMark) -> Result<Element, JsValue> {
    let class = match brand.size {
        BrandSize::Compact => "hm-brand hm-brand-compact",
        BrandSize::Display => "hm-brand hm-brand-display",
    };
    let root = element(document, "div", class)?;
    root.set_attribute("role", "img")?;
    root.set_attribute("aria-label", &brand.wordmark)?;
    if brand.star {
        append(
            &root,
            &compass_icon(document, Icon::Compass, "hm-brand-star")?,
        )?;
    }
    append(
        &root,
        &text_element(document, "span", "hm-wordmark", &brand.wordmark)?,
    )?;
    Ok(root)
}

/// The title, its tagline and the diamond rule.
pub fn title(document: &Document, block: &TitleBlock) -> Result<Element, JsValue> {
    let root = element(document, "div", "hm-title")?;
    append(
        &root,
        &text_element(document, "h1", "hm-title-name hm-wordmark", &block.title)?,
    )?;
    if let Some(tagline) = &block.tagline {
        append(&root, &text_element(document, "p", "hm-tagline", tagline)?)?;
    }
    if block.rule {
        append(&root, &diamond_rule(document, "hm-title-rule")?)?;
    }
    Ok(root)
}

/// The navigation rail: a glass panel with one row for each item.
pub fn nav(document: &Document, rail: &NavRail) -> Result<Element, JsValue> {
    let root = element(document, "nav", "hm-nav")?;
    root.set_attribute("aria-label", "Main")?;
    let panel = element(document, "ul", "hm-nav-panel ui-lit")?;
    for (index, item) in rail.items.iter().enumerate() {
        let class = if item.active {
            "hm-nav-item hm-nav-active"
        } else {
            "hm-nav-item"
        };
        let row = element(document, "li", class)?;
        row.set_attribute("data-focus-id", &format!("nav:{}", index + 1))?;
        if item.active {
            row.set_attribute("aria-current", "page")?;
        }
        append(&row, &icon(document, item.glyph, "hm-nav-icon")?)?;
        append(
            &row,
            &text_element(document, "span", "hm-nav-label", &item.label)?,
        )?;
        if let Some(badge) = &item.badge {
            append(
                &row,
                &text_element(document, "span", "hm-nav-badge", badge)?,
            )?;
        }
        append(&panel, &row)?;
    }
    append(&root, &panel)?;
    Ok(root)
}

/// The bell, as a round mark like the profile mark, with a count when there are unread notices.
fn bell(document: &Document, top: &HomeTopBar) -> Result<Element, JsValue> {
    let root = element(document, "div", "hm-bell")?;
    root.set_attribute("role", "img")?;
    root.set_attribute("aria-label", &bell_label(top.unread))?;
    append(
        &root,
        &page_glyph(document, HomeGlyph::Bell, "hm-bell-icon")?,
    )?;
    if let Some(count) = unread_text(top.unread) {
        append(
            &root,
            &text_element(document, "span", "hm-bell-count", &count)?,
        )?;
    }
    Ok(root)
}

/// The top bar's right side: the scene's pills (balance, rate, profile) with the bell between.
pub fn top(document: &Document, top: &HomeTopBar, scene: &TvScene) -> Result<Element, JsValue> {
    let bar = scene.pills.clone().unwrap_or_default();
    let bell = if top.bell {
        Some(bell(document, top)?)
    } else {
        None
    };
    crate::view::pills::pills(document, &bar, bell)
}

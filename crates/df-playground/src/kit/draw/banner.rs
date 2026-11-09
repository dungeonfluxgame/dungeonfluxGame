//! Draws a scene header or encounter banner (`kit::banner`).

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::kit::Fill;
use crate::kit::banner::{Banner, BannerPlace};
use crate::kit::draw::{Shape, glass, icon};

/// The banner, or `None` when it has no title.
pub fn draw(document: &Document, banner: &Banner) -> Result<Option<Element>, JsValue> {
    let Some(title) = banner.title() else {
        return Ok(None);
    };
    let placement = banner.placement();
    let shape = match placement {
        BannerPlace::Header => Shape::Tab,
        BannerPlace::TopCenter => Shape::Hexagon,
    };
    let (root, pane) = glass(
        document,
        shape,
        Fill::Blurred,
        &format!(
            "kit-banner kit-banner-{} kit-tone-{}",
            placement.key(),
            banner.tone.key()
        ),
    )?;
    if placement == BannerPlace::TopCenter {
        append(&pane, &element(document, "span", "kit-inlay")?)?;
    }
    if let Some(which) = banner.icon {
        let class = if banner.is_working() {
            "kit-banner-icon kit-turning"
        } else {
            "kit-banner-icon"
        };
        append(&pane, &icon(document, which, class)?)?;
    }
    let words = element(document, "span", "kit-banner-words")?;
    if let Some(number) = banner.number_text() {
        append(
            &words,
            &text_element(document, "span", "kit-banner-number", &number)?,
        )?;
    }
    append(
        &words,
        &text_element(document, "span", "kit-banner-title", title)?,
    )?;
    append(&pane, &words)?;
    Ok(Some(root))
}

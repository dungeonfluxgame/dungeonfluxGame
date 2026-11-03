//! Small page pieces the landing page's components share: a drawing of either icon set, and a
//! picture of the media library that is never a broken image.
//!
//! Everything is built from fixed strings of this crate and from text set as text, never as
//! markup. A picture name from a scene file becomes an address only when it names an image of
//! the media library.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element};
use crate::home_glyph::{HomeGlyph, HomeIcon};
use crate::media::{MediaKind, media_kind};
use crate::view_kit::glyph;

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

/// A drawing of this page's own set as an `svg` of class `class`, in the current text colour.
pub fn page_glyph(document: &Document, which: HomeGlyph, class: &str) -> Result<Element, JsValue> {
    let svg = document.create_element_ns(Some(SVG_NAMESPACE), "svg")?;
    for (name, value) in [
        ("viewBox", "0 0 24 24"),
        ("fill", "none"),
        ("stroke", "currentColor"),
        ("stroke-width", "1.7"),
        ("stroke-linecap", "round"),
        ("stroke-linejoin", "round"),
        ("aria-hidden", "true"),
        ("focusable", "false"),
        ("class", class),
    ] {
        svg.set_attribute(name, value)?;
    }
    svg.set_inner_html(which.markup());
    Ok(svg)
}

/// A drawing of either set as an `svg` of class `class`.
pub fn icon(document: &Document, which: HomeIcon, class: &str) -> Result<Element, JsValue> {
    match which {
        HomeIcon::Page(page) => page_glyph(document, page, class),
        HomeIcon::Overlay(overlay) => glyph(document, overlay, class),
    }
}

/// A picture of the media library in an `img` of class `class`, or `None` when `media` does not
/// name an image.
pub fn photo(
    document: &Document,
    class: &str,
    media: Option<&str>,
    alt: &str,
) -> Result<Option<Element>, JsValue> {
    let Some(name) = media.filter(|name| media_kind(name) == Some(MediaKind::Image)) else {
        return Ok(None);
    };
    let image = element(document, "img", class)?;
    image.set_attribute("src", &format!("/playground/media/{name}"))?;
    image.set_attribute("data-media", name)?;
    image.set_attribute("alt", alt)?;
    Ok(Some(image))
}

/// The diamond rule: a line, a diamond and a line.
pub fn diamond_rule(document: &Document, class: &str) -> Result<Element, JsValue> {
    let row = element(document, "div", class)?;
    for part in ["hm-rule", "hm-diamond", "hm-rule"] {
        append(&row, &element(document, "span", part)?)?;
    }
    Ok(row)
}

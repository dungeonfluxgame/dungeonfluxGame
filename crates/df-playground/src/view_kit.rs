//! Small page pieces the overlay components share: a line-drawn glyph, a portrait with a fallback
//! and a glass panel with its glow wrapper.
//!
//! Everything here is built from fixed strings of this crate and from text set as text, never as
//! markup: a glyph's drawing is a constant (`Glyph::markup`) and a picture name from a scene file
//! becomes an address only when it is a plain image name of the media library.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::media::{MediaKind, media_kind};
use crate::overlay::Glyph;
use crate::style::css_color;

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

/// A glyph as an `svg` of class `class`, drawn in the current text colour.
pub fn glyph(document: &Document, glyph: Glyph, class: &str) -> Result<Element, JsValue> {
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
    svg.set_inner_html(glyph.markup());
    Ok(svg)
}

/// A picture of the media library in an element of class `class`: the picture when `media` names
/// an image, otherwise the first letter of `name` on a plain medallion, so a card is never empty
/// and never shows a broken image.
pub fn portrait(
    document: &Document,
    class: &str,
    media: Option<&str>,
    name: &str,
) -> Result<Element, JsValue> {
    let frame = element(document, "div", class)?;
    match media.filter(|media| media_kind(media) == Some(MediaKind::Image)) {
        Some(media) => {
            let image = element(document, "img", "portrait-image")?;
            image.set_attribute("src", &format!("/playground/media/{media}"))?;
            image.set_attribute("data-media", media)?;
            image.set_attribute("alt", name)?;
            append(&frame, &image)?;
        }
        None => {
            let initial: String = name.chars().take(1).collect();
            append(
                &frame,
                &text_element(document, "span", "portrait-initial", &initial)?,
            )?;
        }
    }
    Ok(frame)
}

/// The declaration `property:color;` when `color` is a plain hex colour of a scene file, and
/// nothing otherwise, so a colour never carries anything but a colour into a style.
pub fn accent_style(property: &str, color: Option<&str>) -> String {
    color
        .and_then(css_color)
        .map_or_else(String::new, |color| format!("{property}:{color};"))
}

/// A chamfered glass panel inside a glow wrapper: the wrapper takes the layout and the glow, the
/// inner panel the rim and fill. Returns `(wrapper, panel)`; put the content in the panel.
pub fn glass(
    document: &Document,
    wrapper_class: &str,
    panel_class: &str,
) -> Result<(Element, Element), JsValue> {
    let wrapper = element(document, "div", &format!("ui-glow {wrapper_class}"))?;
    let panel = element(document, "div", &format!("ui-glass {panel_class}"))?;
    append(&wrapper, &panel)?;
    Ok((wrapper, panel))
}

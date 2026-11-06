//! The small pictures of the lobby, the pills and the loading screen: a fixed set of drawings
//! built as `svg` elements, and the QR-like placeholder block.
//!
//! Each drawing is a constant of this crate, never text from a scene file, and is set as the
//! inside of an `svg` made here. The icons take the current text colour, so a state recolours
//! them with CSS and no script. The QR block is a placeholder: its cells come from the code's
//! text (`lobby::model::qr_like_cells`) and it is not a code anyone can scan.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::lobby::model::{QR_CELLS, qr_like_cells};

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

/// The drawings of this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    People,
    Controller,
    Coins,
    Bolt,
    User,
    Play,
    Silhouette,
    Spinner,
    CheckRing,
    Waveform,
    Compass,
}

impl Icon {
    /// The inside of the icon's `svg` and the box it is drawn in.
    fn drawing(self) -> (&'static str, &'static str, bool) {
        match self {
            Self::People => (
                "0 0 32 24",
                r#"<circle cx="16" cy="7" r="4.2"/><path d="M7.5 21c0-5 3.6-8 8.5-8s8.5 3 8.5 8z"/><circle cx="6" cy="9" r="3"/><path d="M0.5 20c0-3.6 2.2-5.8 5.5-5.8 1 0 1.8.2 2.6.6-1.6 1.4-2.5 3.2-2.7 5.2z"/><circle cx="26" cy="9" r="3"/><path d="M31.5 20c0-3.6-2.2-5.8-5.5-5.8-1 0-1.8.2-2.6.6 1.6 1.4 2.5 3.2 2.7 5.2z"/>"#,
                true,
            ),
            Self::Controller => (
                "0 0 24 24",
                r#"<path d="M7 7h10a5 5 0 0 1 4.8 6.4l-.9 3.3a2.4 2.4 0 0 1-4.2.8L15 15H9l-1.7 2.5a2.4 2.4 0 0 1-4.2-.8l-.9-3.3A5 5 0 0 1 7 7z"/><path d="M8 10v3M6.5 11.5h3"/><circle cx="15.5" cy="10.5" r=".6"/><circle cx="17.5" cy="12.5" r=".6"/>"#,
                false,
            ),
            Self::Coins => (
                "0 0 24 24",
                r#"<ellipse cx="9" cy="6" rx="6" ry="2.6"/><path d="M3 6v4c0 1.4 2.7 2.6 6 2.6s6-1.2 6-2.6V6"/><path d="M3 10v4c0 1.4 2.7 2.6 6 2.6s6-1.2 6-2.6v-4"/><path d="M9 17v1.4c0 1.4 2.7 2.6 6 2.6s6-1.2 6-2.6V14c0-1.2-2-2.2-4.6-2.5"/>"#,
                false,
            ),
            Self::Bolt => (
                "0 0 24 24",
                r#"<path d="M13.5 1.5L4.5 13.5h6L9 22.5l10.5-13h-6.2z"/>"#,
                true,
            ),
            Self::User => (
                "0 0 24 24",
                r#"<circle cx="12" cy="8" r="4.6"/><path d="M3.5 21.5c0-5.2 3.6-8 8.5-8s8.5 2.8 8.5 8z"/>"#,
                true,
            ),
            Self::Play => ("0 0 24 24", r#"<path d="M5 2.5l16.5 9.5L5 21.5z"/>"#, true),
            Self::Silhouette => (
                "0 0 100 120",
                r#"<path d="M50 6c-12 0-20 9-20 21 0 8 3 14 8 18-3 3-6 5-10 6C14 55 4 62 2 78l-2 42h100l-2-42c-2-16-12-23-26-27-4-1-7-3-10-6 5-4 8-10 8-18C70 15 62 6 50 6z"/>"#,
                true,
            ),
            Self::Spinner => ("0 0 24 24", r#"<path d="M12 3a9 9 0 1 0 9 9"/>"#, false),
            Self::CheckRing => (
                "0 0 24 24",
                r#"<circle cx="12" cy="12" r="10"/><path d="M7 12.5l3.4 3.4L17 9"/>"#,
                false,
            ),
            Self::Waveform => (
                "0 0 120 40",
                r#"<path d="M2 20h8M14 14v12M18 8v24M22 16v8M26 4v32M30 11v18M34 15v10M38 1v38M42 9v22M46 13v14M50 5v30M54 12v16M58 16v8M62 8v24M66 2v36M70 11v18M74 15v10M78 6v28M82 12v16M86 17v6M90 9v22M94 14v12M98 18v4M102 20h6M112 20h6"/>"#,
                false,
            ),
            Self::Compass => (
                "0 0 48 48",
                r#"<path d="M24 1l3.4 17.6L47 24l-19.6 5.4L24 47l-3.4-17.6L1 24l19.6-5.4z"/><circle cx="24" cy="24" r="8.5" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M24 15l1.8 7.2 7.2 1.8-7.2 1.8L24 33l-1.8-7.2L15 24l7.2-1.8z" fill="rgb(10,16,32)"/>"#,
                true,
            ),
        }
    }
}

/// The icon as an `svg` of class `class`, drawn in the current text colour. A filled icon is
/// filled; the others are strokes.
pub fn icon(document: &Document, which: Icon, class: &str) -> Result<Element, JsValue> {
    let (view_box, markup, filled) = which.drawing();
    let svg = document.create_element_ns(Some(SVG_NAMESPACE), "svg")?;
    let (fill, stroke) = if filled {
        ("currentColor", "none")
    } else {
        ("none", "currentColor")
    };
    for (name, value) in [
        ("viewBox", view_box),
        ("fill", fill),
        ("stroke", stroke),
        ("stroke-width", "1.8"),
        ("stroke-linecap", "round"),
        ("stroke-linejoin", "round"),
        ("aria-hidden", "true"),
        ("focusable", "false"),
        ("class", class),
    ] {
        svg.set_attribute(name, value)?;
    }
    svg.set_inner_html(markup);
    Ok(svg)
}

/// The QR-like placeholder block for `code`: white cells on the dark block, one path. It is not
/// scannable: the real code comes from the server.
pub fn qr_placeholder(document: &Document, code: &str, class: &str) -> Result<Element, JsValue> {
    let svg = document.create_element_ns(Some(SVG_NAMESPACE), "svg")?;
    svg.set_attribute(
        "viewBox",
        &format!("-1 -1 {} {}", QR_CELLS + 2, QR_CELLS + 2),
    )?;
    svg.set_attribute("class", class)?;
    svg.set_attribute("role", "img")?;
    svg.set_attribute("aria-label", "A placeholder for the join code's QR picture")?;
    svg.set_attribute("shape-rendering", "crispEdges")?;
    let mut path = String::new();
    for (index, dark) in qr_like_cells(code).into_iter().enumerate() {
        if dark {
            path.push_str(&format!(
                "M{} {}h1v1h-1z",
                index % QR_CELLS,
                index / QR_CELLS
            ));
        }
    }
    let cells = document.create_element_ns(Some(SVG_NAMESPACE), "path")?;
    cells.set_attribute("d", &path)?;
    cells.set_attribute("fill", "currentColor")?;
    svg.append_child(&cells)?;
    Ok(svg)
}

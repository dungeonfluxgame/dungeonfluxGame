//! The look studio page as elements (`look::studio::LookScreen`), in the phone kit's regions: the
//! header, a heading, the one scrolling body of blocks and the dock of buttons, over the scene. It
//! decides nothing; every tap is the screen's `LookTap`, carried as `data-action`.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, set_action, text_element};
use crate::look::studio::{LookScreen, LookViewer};
use crate::phone_kit::draw::{glyph, page, regions, set_scene, shell};
use crate::phone_kit::glyph::PhoneGlyph;
use crate::phone_kit::shell::PhoneHeader;
use crate::view::kit::photo;

/// The class of the viewer's frame, which a double tap zooms (`ZOOMED_CLASS`) and a swipe steps.
pub const VIEWER_FRAME_CLASS: &str = "ph-viewer-frame";
/// The class the frame takes while zoomed in.
pub const ZOOMED_CLASS: &str = "ph-viewer-zoomed";

/// A round control of the viewer: a glyph or a sign, and its tap.
fn control(
    document: &Document,
    class: &str,
    label: &str,
    tap: Option<&str>,
) -> Result<Element, JsValue> {
    let button = element(document, "button", &format!("ph-viewer-control {class}"))?;
    button.set_attribute("type", "button")?;
    button.set_attribute("aria-label", label)?;
    match tap {
        Some(action) => set_action(&button, action)?,
        None => button.set_attribute("disabled", "")?,
    }
    Ok(button)
}

/// The full-screen viewer over the page: the picture fitted whole, its label and place among the
/// pictures, the steps, the close, and what can be done with the picture.
pub fn viewer(document: &Document, viewer: &LookViewer) -> Result<Element, JsValue> {
    let overlay = element(document, "div", "ph-viewer")?;
    overlay.set_attribute("role", "dialog")?;
    overlay.set_attribute("aria-label", &viewer.item.label)?;
    let close = viewer.close.clone();
    let scrim = element(document, "div", "ph-viewer-scrim")?;
    set_action(&scrim, &close)?;
    append(&overlay, &scrim)?;
    let frame = element(document, "figure", VIEWER_FRAME_CLASS)?;
    if let Some(image) = photo(
        document,
        "ph-viewer-image",
        Some(&viewer.item.picture),
        &viewer.item.label,
    )? {
        append(&frame, &image)?;
    }
    append(&overlay, &frame)?;
    let head = element(document, "div", "ph-viewer-head")?;
    append(
        &head,
        &text_element(
            document,
            "span",
            "ph-viewer-label",
            &format!(
                "{}  ({} of {})",
                viewer.item.label,
                viewer.index + 1,
                viewer.count
            ),
        )?,
    )?;
    let shut = control(document, "ph-viewer-close", "Close", Some(&close))?;
    append(
        &shut,
        &glyph(document, PhoneGlyph::Close, "ph-viewer-glyph")?,
    )?;
    append(&head, &shut)?;
    append(&overlay, &head)?;
    let previous = control(
        document,
        "ph-viewer-previous",
        "Previous picture",
        viewer.previous.as_deref(),
    )?;
    let next = control(
        document,
        "ph-viewer-next",
        "Next picture",
        viewer.next.as_deref(),
    )?;
    append(&overlay, &previous)?;
    append(&overlay, &next)?;
    if !viewer.item.buttons.is_empty() {
        let dock = element(document, "div", "ph-viewer-dock")?;
        for spec in &viewer.item.buttons {
            append(&dock, &shell::button(document, spec)?)?;
        }
        append(&overlay, &dock)?;
    }
    Ok(overlay)
}

/// The look studio page, with `scene` behind it.
pub fn build(
    document: &Document,
    screen: &LookScreen,
    header: &PhoneHeader,
    scene: Option<&str>,
) -> Result<Element, JsValue> {
    let body = element(document, "main", "ph-body ph-look-body")?;
    append(
        &body,
        &text_element(document, "h2", "ph-join-heading", &screen.heading)?,
    )?;
    for block in &screen.blocks {
        append(&body, &page::block(document, block)?)?;
    }
    let dock = element(document, "div", "ph-dock ph-join-dock")?;
    for spec in &screen.buttons {
        append(&dock, &shell::button(document, spec)?)?;
    }
    let drawn = regions(
        document,
        "ph-live ph-look",
        false,
        Some(header),
        None,
        &body,
        &dock,
    )?;
    if let Some(open) = &screen.viewer {
        append(&drawn, &viewer(document, open)?)?;
    }
    set_scene(&drawn, scene)?;
    Ok(drawn)
}

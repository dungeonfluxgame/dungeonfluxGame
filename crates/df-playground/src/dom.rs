//! Small helpers for building page elements, and the page's memory of what it has begun. The memory
//! is `continuity`; this module only reads the browser's clock for it.

use std::cell::RefCell;
use std::fmt;

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::continuity::Continuity;
use crate::fixture::{ImageRef, Slide};
use crate::media::{MediaKind, media_kind};
use crate::motion_style::slide_style;
use crate::path::media_file;

pub fn document() -> Result<Document, JsValue> {
    web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| JsValue::from_str("the page has no document"))
}

/// The reading of the browser's steady clock, in milliseconds. It never goes backwards, unlike the
/// wall clock, so an elapsed time made from it is never negative.
pub fn now_ms() -> Result<f64, JsValue> {
    web_sys::window()
        .and_then(|window| window.performance())
        .map(|performance| performance.now())
        .ok_or_else(|| JsValue::from_str("the page has no performance clock"))
}

/// An exception the browser threw, kept as it was thrown. It prints as its message.
#[derive(Debug, Clone, PartialEq)]
pub struct BrowserError(JsValue);

impl From<JsValue> for BrowserError {
    fn from(thrown: JsValue) -> Self {
        Self(thrown)
    }
}

impl fmt::Display for BrowserError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&js_message(&self.0))
    }
}

impl std::error::Error for BrowserError {}

/// What a thrown value says, for a message that names the cause.
pub fn js_message(error: &JsValue) -> String {
    if let Some(text) = error.as_string() {
        return text;
    }
    js_sys::Reflect::get(error, &JsValue::from_str("message"))
        .ok()
        .and_then(|message| message.as_string())
        .unwrap_or_else(|| format!("{error:?}"))
}

/// A new element with a class list (space separated, may be empty).
pub fn element(document: &Document, tag: &str, class: &str) -> Result<Element, JsValue> {
    let element = document.create_element(tag)?;
    if !class.is_empty() {
        element.set_attribute("class", class)?;
    }
    Ok(element)
}

/// A new element holding only text. Text is set as text, never as markup.
pub fn text_element(
    document: &Document,
    tag: &str,
    class: &str,
    text: &str,
) -> Result<Element, JsValue> {
    let element = element(document, tag, class)?;
    element.set_text_content(Some(text));
    Ok(element)
}

pub fn append(parent: &Element, child: &Element) -> Result<(), JsValue> {
    parent.append_child(child).map(|_| ())
}

/// Sets the inline style from numbers and validated colours only (callers never pass text from a
/// fixture here unchecked).
pub fn set_style(element: &Element, css: &str) -> Result<(), JsValue> {
    element.set_attribute("style", css)
}

/// Marks a control with the action a click or a change on it asks for.
pub fn set_action(element: &Element, action: &str) -> Result<(), JsValue> {
    element.set_attribute("data-action", action)
}

pub fn clear(element: &Element) {
    element.set_text_content(None);
}

thread_local! {
    static CONTINUITY: RefCell<Continuity> = RefCell::new(Continuity::default());
}

/// Reads or changes what the page remembers of the slides and camera moves it has begun.
pub fn with_continuity<R>(change: impl FnOnce(&mut Continuity) -> R) -> R {
    CONTINUITY.with(|continuity| change(&mut continuity.borrow_mut()))
}

/// A picture from the media library, with its caption under it, in an element of class `class`.
/// A name that is not a picture of a known type is shown as a note instead of a broken image, so
/// the mistake is visible. The image says which media file it is, so the page can report which
/// pictures loaded.
pub fn picture(document: &Document, class: &str, image: &ImageRef) -> Result<Element, JsValue> {
    let frame = element(document, "div", class)?;
    if let Some(slide) = &image.slide {
        // Numbers only, so a scene file cannot put text into a style.
        frame.set_attribute("style", &slide_for_page(slide)?)?;
    }
    if media_kind(&image.media) != Some(MediaKind::Image) {
        append(
            &frame,
            &text_element(
                document,
                "div",
                "media-missing",
                &format!("'{}' is not a picture the playground can show", image.media),
            )?,
        )?;
        return Ok(frame);
    }
    let picture = element(document, "img", &format!("{class}-image"))?;
    picture.set_attribute("src", &media_file(&image.media))?;
    picture.set_attribute("data-media", &image.media)?;
    picture.set_attribute("alt", image.caption.as_deref().unwrap_or(&image.media))?;
    if image.rotation_degrees != 0 {
        // A number, never text from a fixture, so it cannot carry style. A turn of 360 or more is
        // refused when a scene is read or changed, so it does not get here.
        picture.set_attribute(
            "style",
            &format!("transform: rotate({}deg);", image.rotation_degrees),
        )?;
    }
    append(&frame, &picture)?;
    if let Some(caption) = &image.caption {
        append(
            &frame,
            &text_element(document, "div", &format!("{class}-caption"), caption)?,
        )?;
    }
    Ok(frame)
}

fn slide_for_page(slide: &Slide) -> Result<String, JsValue> {
    let now = now_ms()?;
    let elapsed = with_continuity(|continuity| continuity.slide_elapsed(now, slide));
    Ok(slide_style(slide, elapsed))
}

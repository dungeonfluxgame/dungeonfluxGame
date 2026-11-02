//! Small helpers for building page elements. They hold no state.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use std::cell::RefCell;

use crate::camera::{Framing, progress};
use crate::fixture::{Camera, ImageRef, Slide};
use crate::media::{MediaKind, media_kind};

pub fn document() -> Result<Document, JsValue> {
    web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| JsValue::from_str("the page has no document"))
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

/// Slides this page has begun, each with the time it began, in milliseconds since the epoch. A
/// redraw in the middle of a slide finds its slide here and carries on from where it was instead of
/// starting again.
const MAXIMUM_REMEMBERED_SLIDES: usize = 16;

thread_local! {
    static SLIDES: RefCell<Vec<(Slide, f64)>> = const { RefCell::new(Vec::new()) };
}

/// How long ago this slide began, noting now as its start the first time it is asked about.
fn slide_elapsed_ms(slide: &Slide) -> f64 {
    let now = js_sys::Date::now();
    SLIDES.with(|slides| {
        let mut slides = slides.borrow_mut();
        if let Some((_, began)) = slides.iter().find(|(known, _)| known == slide) {
            return now - began;
        }
        if slides.len() >= MAXIMUM_REMEMBERED_SLIDES {
            slides.remove(0);
        }
        slides.push((*slide, now));
        0.0
    })
}

/// Forgets every slide, so the next scene's slides begin from their start.
pub fn forget_slides() {
    SLIDES.with(|slides| slides.borrow_mut().clear());
}

/// The inline style that makes a card slide: its place at the end, and an animation that starts
/// part-way through when the slide is already under way.
fn slide_style(slide: &Slide) -> String {
    let elapsed = slide_elapsed_ms(slide);
    if elapsed >= f64::from(slide.duration_ms) {
        return format!("left: {}%;", slide.to_percent);
    }
    format!(
        "left: {to}%; --slide-from: {from}%; --slide-to: {to}%; animation: card-slide {duration}ms linear forwards; animation-delay: -{elapsed:.0}ms;",
        to = slide.to_percent,
        from = slide.from_percent,
        duration = slide.duration_ms,
    )
}

/// The camera move this page is part-way through, so a redraw carries on from where the picture
/// was instead of jumping, and a new move starts from where the last one had got to.
struct CameraRun {
    serial: u64,
    began: f64,
    duration_ms: u32,
    from: Framing,
    to: Framing,
}

thread_local! {
    static CAMERA: RefCell<Option<CameraRun>> = const { RefCell::new(None) };
}

/// Forgets the camera, so the next scene starts on the whole stage.
pub fn forget_camera() {
    CAMERA.with(|run| *run.borrow_mut() = None);
}

/// The style that frames the world layer, and what a later frame of the same move needs to know.
pub struct CameraStyle {
    /// The whole inline style.
    pub full: String,
    /// The transform the move ends on.
    pub transform: String,
    /// Whether the move is still under way.
    pub moving: bool,
    /// Which move this is.
    pub serial: u64,
}

/// The style for a camera looking for `to`. A move that is already under way keeps its start and
/// its clock; a new one starts from where the last had got to. Numbers only.
pub fn camera_style(camera: Option<&Camera>, to: Framing) -> CameraStyle {
    let settled = |to: Framing, serial: u64| CameraStyle {
        full: format!(
            "transform: {}; transition: transform 90ms linear;",
            to.transform()
        ),
        transform: to.transform(),
        moving: false,
        serial,
    };
    let Some(camera) = camera else {
        forget_camera();
        return settled(Framing::WHOLE_STAGE, 0);
    };
    let now = js_sys::Date::now();
    let (from, began) = CAMERA.with(|run| {
        let mut run = run.borrow_mut();
        match run.as_mut() {
            Some(known) if known.serial == camera.serial => {
                known.to = to;
                (known.from, known.began)
            }
            previous => {
                let from = previous.map_or(Framing::WHOLE_STAGE, |known| {
                    let done = progress(now - known.began, known.duration_ms);
                    known.from.blend(&known.to, done)
                });
                *run = Some(CameraRun {
                    serial: camera.serial,
                    began: now,
                    duration_ms: camera.duration_ms,
                    from,
                    to,
                });
                (from, now)
            }
        }
    });
    let elapsed = now - began;
    if progress(elapsed, camera.duration_ms) >= 1.0 {
        return settled(to, camera.serial);
    }
    CameraStyle {
        full: format!(
            "transform: {to_transform}; --cam-from: {}; --cam-to: {to_transform}; animation: camera-move {}ms linear forwards; animation-delay: -{elapsed:.0}ms;",
            from.transform(),
            camera.duration_ms,
            to_transform = to.transform(),
        ),
        transform: to.transform(),
        moving: true,
        serial: camera.serial,
    }
}

/// A picture from the media library, with its caption under it, in an element of class `class`.
/// A name that is not a picture of a known type is shown as a note instead of a broken image, so
/// the mistake is visible. The image says which media file it is, so the page can report which
/// pictures loaded.
pub fn picture(document: &Document, class: &str, image: &ImageRef) -> Result<Element, JsValue> {
    let frame = element(document, "div", class)?;
    if let Some(slide) = &image.slide {
        // Numbers only, so a scene file cannot put text into a style.
        frame.set_attribute("style", &slide_style(slide))?;
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
    picture.set_attribute("src", &format!("/playground/media/{}", image.media))?;
    picture.set_attribute("data-media", &image.media)?;
    picture.set_attribute("alt", image.caption.as_deref().unwrap_or(&image.media))?;
    if !image.rotation_degrees.is_multiple_of(360) {
        // A number, never text from a fixture, so it cannot carry style.
        picture.set_attribute(
            "style",
            &format!("transform: rotate({}deg);", image.rotation_degrees % 360),
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

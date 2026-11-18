//! The live phones beside the shared screen on the playground: the real phone page (`path::PLAY`)
//! in a frame at a phone's size, so one person can join a seat and play the whole day from the dev
//! screen with the screens in view. View "Both" shows one; View "Table" shows two side by side,
//! the second loaded from the other loopback origin (`localhost` beside `127.0.0.1`), so each
//! keeps its own seat key in its own storage and the two play as two seats.
//!
//! Each frame is made once with the page and only shown or hidden after, never rebuilt: a scene
//! change or a device switch never reloads a phone, which keeps its own seat key and its own pages
//! open. A frame loads its page the first time it is shown. The playground's fixture-drawn phone
//! stays for Devices "phone", where a scenario's phone part is reviewed.
//!
//! In View "Both" the phone page lays out at a phone's size (390 by `PHONE_HEIGHT`); a window too
//! short for it shows it scaled down whole (`fit`) so all of it fits below where the stage starts
//! (the toolbar and the scenario strip above it), and it stays under the sticky toolbar while the
//! studio scrolls. In View "Table" nothing is scaled: each frame is `TABLE_WIDTH` wide at the
//! height the window leaves (at most `TABLE_HEIGHT`), with no transform on it or around it, so a
//! tap is a plain tap on the page inside; a window too narrow for the row scrolls sideways
//! (`TABLE_MARK` on the stage, read by the theme).

use wasm_bindgen::prelude::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element};

use crate::controls::Device;
use crate::dom::{append, element, set_style};
use crate::path::PLAY;

/// The size the phone page lays out at, in CSS pixels.
const PHONE_HEIGHT: f64 = 844.0;
/// The room kept above and below the frame, in CSS pixels.
const FRAME_GAP: f64 = 12.0;
/// The smallest the frame is drawn, so a very short window still shows a usable phone.
const SMALLEST_SCALE: f64 = 0.35;
/// The attribute on the stage and on each shown frame while View "Table" shows the phones at
/// their real size.
const TABLE_MARK: &str = "data-table";
/// The tallest a frame is in View "Table", and the shortest it is made to fit a short window, in
/// CSS pixels (its width is the theme's, 375).
const TABLE_HEIGHT: f64 = 812.0;
const SHORTEST_TABLE_HEIGHT: f64 = 480.0;

/// The height of a frame in View "Table" in a window `window_height` tall when it starts `top`
/// down the page: what is left below that, at most `TABLE_HEIGHT`.
fn table_height_for(window_height: f64, top: f64) -> f64 {
    (window_height - top - FRAME_GAP).clamp(SHORTEST_TABLE_HEIGHT, TABLE_HEIGHT)
}

/// The share of its size the frame is drawn at in a window `window_height` tall when it starts
/// `top` down the page: whole when it fits below that, scaled down to fit when it does not.
fn scale_for(window_height: f64, top: f64) -> f64 {
    ((window_height - top - FRAME_GAP) / PHONE_HEIGHT).clamp(SMALLEST_SCALE, 1.0)
}

/// Scales the frame to the window: all of it fits below where the stage starts on the page (read
/// from the stage, which never sticks, at any scroll), and it sticks under the toolbar.
fn fit(section: &Element) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("the page has no window"))?;
    let window_height = window.inner_height()?.as_f64().unwrap_or(PHONE_HEIGHT);
    let document = crate::dom::document()?;
    let bar_height = document
        .query_selector(".studio-bar")?
        .map_or(0.0, |bar| bar.get_bounding_client_rect().height());
    let stage_top = section
        .parent_element()
        .map_or(bar_height, |stage| {
            stage.get_bounding_client_rect().top() + window.scroll_y().unwrap_or(0.0)
        })
        .max(bar_height);
    let style = if section.has_attribute(TABLE_MARK) {
        format!(
            "--live-phone-height:{}px",
            table_height_for(window_height, stage_top).round()
        )
    } else {
        format!(
            "--live-phone-scale:{:.4};--live-phone-top:{}px",
            scale_for(window_height, stage_top),
            (bar_height + FRAME_GAP).round()
        )
    };
    if section.get_attribute("style").as_deref() == Some(style.as_str()) {
        return Ok(());
    }
    set_style(section, &style)
}

/// Fits the frame again each time `target` fires `event` (the scenario strip opening or closing),
/// for the life of the page.
fn refit_on(target: &Element, event: &str, section: &Element) -> Result<(), JsValue> {
    let moved = section.clone();
    let on_event = Closure::<dyn FnMut()>::new(move || {
        if let Err(error) = refit(&moved) {
            crate::page_log::note_error(
                crate::timeline::ErrorSource::Draw,
                &crate::dom::js_message(&error),
            );
        }
    });
    target.add_event_listener_with_callback(event, on_event.as_ref().unchecked_ref())?;
    on_event.forget();
    Ok(())
}

/// Fits the frame again where the stage starts may have moved: the toolbar or the scenario strip
/// above it grew or shrank. A frame that is hidden is left as it is.
fn refit(section: &Element) -> Result<(), JsValue> {
    if section.has_attribute("hidden") {
        return Ok(());
    }
    fit(section)
}

/// Fits the frame again whenever the window changes size, for the life of the page (the frame is
/// made once), and once the page is drawn. The page fits it again after each draw too
/// (`refit`), as the toolbar above it may have changed height.
fn follow_window(section: &Element) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("the page has no window"))?;
    let resized = section.clone();
    let on_resize = Closure::<dyn FnMut()>::new(move || {
        if let Err(error) = fit(&resized) {
            crate::page_log::note_error(
                crate::timeline::ErrorSource::Draw,
                &crate::dom::js_message(&error),
            );
        }
    });
    window.add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref())?;
    window.request_animation_frame(on_resize.as_ref().unchecked_ref())?;
    on_resize.forget();
    Ok(())
}

/// The attribute a frame keeps its page's address in until it is first shown.
const SOURCE_ATTRIBUTE: &str = "data-src";

/// Which of the live phones: the first loads from the page's own origin, the second from the other
/// loopback origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveFrame {
    First,
    Second,
}

impl LiveFrame {
    /// Whether this phone is shown for `device`: the first beside the shared screen in View
    /// "Both" and "Table", the second only in "Table".
    pub fn shows(self, device: Device) -> bool {
        match self {
            Self::First => matches!(device, Device::Both | Device::Table),
            Self::Second => device == Device::Table,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::First => "The first live phone",
            Self::Second => "The second live phone",
        }
    }
}

/// The loopback host beside `hostname`, so a frame from it keeps storage of its own: `localhost`
/// for `127.0.0.1` (or any other host), `127.0.0.1` for `localhost`.
pub fn other_loopback(hostname: &str) -> &'static str {
    if hostname == "localhost" {
        "127.0.0.1"
    } else {
        "localhost"
    }
}

/// The address `phone` loads: the phone page on this origin, or on the other loopback origin at
/// the same port.
fn source(phone: LiveFrame) -> Result<String, JsValue> {
    match phone {
        LiveFrame::First => Ok(PLAY.to_owned()),
        LiveFrame::Second => {
            let window =
                web_sys::window().ok_or_else(|| JsValue::from_str("the page has no window"))?;
            let location = window.location();
            let port = location.port()?;
            let host = other_loopback(&location.hostname()?);
            let port = if port.is_empty() {
                String::new()
            } else {
                format!(":{port}")
            };
            Ok(format!("{}//{host}{port}{PLAY}", location.protocol()?))
        }
    }
}

/// A live phone's frame, hidden and not yet loaded until it is shown.
fn frame(document: &Document, phone: LiveFrame) -> Result<Element, JsValue> {
    let section = element(document, "section", "live-phone")?;
    section.set_attribute("aria-label", phone.label())?;
    section.set_attribute("hidden", "")?;
    let frame = element(document, "iframe", "live-phone-frame")?;
    frame.set_attribute(SOURCE_ATTRIBUTE, &source(phone)?)?;
    frame.set_attribute("title", phone.label())?;
    frame.set_attribute("allow", "microphone; autoplay")?;
    append(&section, &frame)?;
    follow_window(&section)?;
    Ok(section)
}

/// Sets `TABLE_MARK` on `element` exactly when `device` is View "Table".
fn mark_table(element: &Element, device: Device) -> Result<(), JsValue> {
    if device == Device::Table {
        element.set_attribute(TABLE_MARK, "")
    } else {
        element.remove_attribute(TABLE_MARK)
    }
}

/// Shows `section` for `device` as `phone`, loading its page the first time, or hides it; a frame
/// once loaded stays loaded either way.
fn show(section: &Element, phone: LiveFrame, device: Device) -> Result<(), JsValue> {
    if !phone.shows(device) {
        return section.set_attribute("hidden", "");
    }
    mark_table(section, device)?;
    if let Some(frame) = section.query_selector(".live-phone-frame")?
        && let Some(address) = frame.get_attribute(SOURCE_ATTRIBUTE)
    {
        frame.remove_attribute(SOURCE_ATTRIBUTE)?;
        frame.set_attribute("src", &address)?;
    }
    section.remove_attribute("hidden")?;
    fit(section)
}

/// The live phones of the playground, made once with the page, and the stage they stand on.
pub struct LivePhones {
    stage: Element,
    sections: [(LiveFrame, Element); 2],
}

impl LivePhones {
    /// Both frames, appended to `stage` after the devices, each shown as `device` asks.
    pub fn build(document: &Document, stage: &Element, device: Device) -> Result<Self, JsValue> {
        let sections = [
            (LiveFrame::First, frame(document, LiveFrame::First)?),
            (LiveFrame::Second, frame(document, LiveFrame::Second)?),
        ];
        mark_table(stage, device)?;
        for (phone, section) in &sections {
            append(stage, section)?;
            show(section, *phone, device)?;
        }
        Ok(Self {
            stage: stage.clone(),
            sections,
        })
    }

    /// Shows the phones `device` asks for and hides the others.
    pub fn show(&self, device: Device) -> Result<(), JsValue> {
        mark_table(&self.stage, device)?;
        for (phone, section) in &self.sections {
            show(section, *phone, device)?;
        }
        Ok(())
    }

    /// Fits the shown phones again where the stage starts may have moved.
    pub fn refit(&self) -> Result<(), JsValue> {
        for (_, section) in &self.sections {
            refit(section)?;
        }
        Ok(())
    }

    /// Fits the phones again each time `target` fires `event`, for the life of the page.
    pub fn refit_on(&self, target: &Element, event: &str) -> Result<(), JsValue> {
        for (_, section) in &self.sections {
            refit_on(target, event, section)?;
        }
        Ok(())
    }
}

/// Whether the playground shows a live phone for `device`, in place of the fixture-drawn phone.
pub fn shows(device: Device) -> bool {
    LiveFrame::First.shows(device)
}

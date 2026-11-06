//! The page as drawn: the toolbar, the scenario strip and the devices, kept as pieces so a change
//! redraws only what it touches.
//!
//! `draw_plan` says what to redraw from the state last drawn and the state now. The leaving copy
//! of a scene change lives beside the devices, not in them, so a redraw never takes it away and a
//! transition in progress is never cut short. A copy that leaves is frozen where its original
//! stood: a clone of an element with a running animation would start that animation again.

use std::collections::HashMap;

use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element, HtmlElement};

use super::state::{App, ROOT_ID, log_error, stage_elements_of, with_app_mut};
use super::toolbar::toolbar;
use crate::audio;
use crate::choices::build;
use crate::controls::Transition;
use crate::decisions::is_animating;
use crate::dom::{append, clear, document, element, js_message, text_element};
use crate::draw_plan::{DrawPlan, Part, is_leaving, leaving_class};
use crate::fixture::Fixture;
use crate::media::AudioState;
use crate::motion_style::{frozen_card_style, frozen_world_style};
use crate::view;

/// The pieces of the page once it is drawn.
pub struct Page {
    toolbar: Element,
    wrap: Element,
    devices: Element,
    /// The scenario strip. It is kept and only its words are redrawn, so it stays open or closed.
    notes: Notes,
    tv: Option<view::tv::TvHandles>,
    slots: HashMap<Part, Element>,
}

/// The scenario strip: the `details`, its `summary` and its body, each kept across redraws.
struct Notes {
    details: Element,
    summary: Element,
    body: Element,
}

impl Notes {
    fn new(document: &Document) -> Result<Self, JsValue> {
        let details = element(document, "details", "studio-notes")?;
        let summary = element(document, "summary", "studio-notes-summary")?;
        let body = element(document, "div", "studio-notes-body")?;
        append(&details, &summary)?;
        append(&details, &body)?;
        Ok(Self {
            details,
            summary,
            body,
        })
    }
}

/// The devices as built: the element, and what the page keeps of them.
struct Devices {
    element: Element,
    tv: Option<view::tv::TvHandles>,
    slots: HashMap<Part, Element>,
}

fn replace(old: &Element, new: &Element) -> Result<(), JsValue> {
    old.replace_with_with_node_1(new)
}

fn fill(slot: &Element, content: Option<Element>) -> Result<(), JsValue> {
    clear(slot);
    match content {
        Some(content) => append(slot, &content),
        None => Ok(()),
    }
}

fn shown_fixture(app: &App) -> Option<&Fixture> {
    match app.shown.as_deref() {
        Some(Ok(fixture)) => Some(fixture),
        _ => None,
    }
}

fn build_part(
    document: &Document,
    part: Part,
    fixture: &Fixture,
    app: &App,
) -> Result<Option<Element>, JsValue> {
    if view::tv::PARTS.contains(&part) {
        view::tv::build_part(document, part, &fixture.tv, &app.controls)
    } else {
        view::phone::build_part(document, part, &fixture.phone, &app.controls)
    }
}

/// Builds both devices from the fixture shown, every piece drawn.
fn build_devices(document: &Document, app: &App) -> Result<Devices, JsValue> {
    let stage = element(document, "main", "devices")?;
    let mut slots = HashMap::new();
    let mut tv = None;
    let Some(fixture) = shown_fixture(app) else {
        return Ok(Devices {
            element: stage,
            tv,
            slots,
        });
    };
    if app.controls.device.shows_tv() {
        let shell = view::tv::shell(document)?;
        for (part, holder) in &shell.slots {
            fill(holder, build_part(document, *part, fixture, app)?)?;
        }
        let mut handles = shell.handles;
        handles.sync(
            document,
            app.stage_elements(),
            fixture.tv.camera.as_ref(),
            fixture.tv.backdrop_layout(),
        )?;
        slots.extend(shell.slots);
        append(&stage, &shell.section)?;
        tv = Some(handles);
    }
    if app.controls.device.shows_phone() {
        let shell = view::phone::shell(document)?;
        for (part, holder) in &shell.slots {
            fill(holder, build_part(document, *part, fixture, app)?)?;
        }
        slots.extend(shell.slots);
        append(&stage, &shell.section)?;
    }
    Ok(Devices {
        element: stage,
        tv,
        slots,
    })
}

fn pill(document: &Document, kind: &str, text: &str) -> Result<Element, JsValue> {
    let pill = text_element(document, "span", &format!("studio-pill {kind}"), text)?;
    pill.set_attribute("title", text)?;
    Ok(pill)
}

/// The words of the scenario strip, written into the strip `details`, `summary` and body that the
/// page keeps, so whether it is open and a focus on its summary survive every redraw. Closed, the
/// strip still shows the scenario's title, its file, and a pill for live changes, for contract
/// warnings and for a problem; open, it shows the description, the warnings and the problem in
/// full.
fn fill_notes(document: &Document, notes: &Notes, app: &App) -> Result<(), JsValue> {
    let Notes { summary, body, .. } = notes;
    clear(summary);
    clear(body);
    let mut warnings = Vec::new();
    let title = match app.shown.as_deref() {
        Some(Ok(fixture)) => {
            if app.controls.device.shows_phone() {
                warnings = build(&fixture.phone, app.controls.show_unavailable)
                    .warnings
                    .iter()
                    .map(ToString::to_string)
                    .collect();
            }
            fixture.name.as_str()
        }
        Some(Err(_)) => "This scenario cannot be read",
        None => "No scenario is loaded",
    };
    append(
        summary,
        &text_element(document, "h2", "studio-notes-title", title)?,
    )?;
    if let Some(file) = &app.loaded_name {
        append(
            summary,
            &text_element(document, "span", "studio-file", file)?,
        )?;
    }
    if !app.scene.is_empty() {
        append(summary, &pill(document, "is-live", "Live changes")?)?;
    }
    if !warnings.is_empty() {
        let count = match warnings.len() {
            1 => "1 contract warning".to_owned(),
            many => format!("{many} contract warnings"),
        };
        append(summary, &pill(document, "is-warn", &count)?)?;
    }
    if let Some(Err(_)) = app.shown.as_deref() {
        append(summary, &pill(document, "is-error", "Cannot be read")?)?;
    }
    if let Some(problem) = &app.connection_problem {
        append(summary, &pill(document, "is-error", &problem.to_string())?)?;
        append(
            body,
            &text_element(document, "pre", "note-error", &problem.to_string())?,
        )?;
    }
    match app.shown.as_deref() {
        Some(Ok(fixture)) => {
            if let Some(first_line) = fixture.description.lines().next() {
                append(
                    summary,
                    &text_element(document, "span", "studio-summary-text", first_line)?,
                )?;
            }
            if !app.scene.is_empty() {
                append(
                    body,
                    &text_element(
                        document,
                        "p",
                        "studio-live",
                        "Live changes are drawn over this scenario; scene reset removes them.",
                    )?,
                )?;
            }
            if !fixture.description.is_empty() {
                append(
                    body,
                    &text_element(document, "p", "", &fixture.description)?,
                )?;
            }
        }
        Some(Err(message)) => append(body, &text_element(document, "pre", "note-error", message)?)?,
        None => {}
    }
    if !warnings.is_empty() {
        append(
            body,
            &text_element(
                document,
                "h3",
                "",
                "Where this scenario breaks the contract",
            )?,
        )?;
        let list = element(document, "ul", "studio-warnings")?;
        for warning in &warnings {
            append(&list, &text_element(document, "li", "", warning)?)?;
        }
        append(body, &list)?;
    }
    Ok(())
}

/// Freezes the animation of the element `selector` finds in `copy`, at the place the same element
/// of `original` has now. Only an element that is animating (its style says so) is touched.
fn freeze(
    original: &Element,
    copy: &Element,
    selector: &str,
    property: &str,
    style_for: fn(&str) -> String,
) -> Result<(), JsValue> {
    let (Some(from), Some(to)) = (
        original.query_selector(selector)?,
        copy.query_selector(selector)?,
    ) else {
        return Ok(());
    };
    if !from
        .get_attribute("style")
        .is_some_and(|style| is_animating(&style))
    {
        return Ok(());
    }
    let computed = web_sys::window()
        .ok_or_else(|| JsValue::from_str("the page has no window"))?
        .get_computed_style(&from)?
        .ok_or_else(|| JsValue::from_str("the browser gave no computed style"))?
        .get_property_value(property)?;
    to.set_attribute("style", &style_for(&computed))
}

/// A copy of the devices on screen, dressed to leave: it lies over the new scene and a CSS
/// animation takes it away, so the new scene is revealed underneath. Its pictures stop counting as
/// shown, and the animations of the original
/// (a slide, a camera move) are frozen where they stand instead of starting again.
fn leaving_copy(old: &Element, transition: Transition) -> Result<Element, JsValue> {
    let copy = old
        .clone_node_with_deep(true)?
        .dyn_into::<Element>()
        .map_err(|_| JsValue::from_str("the copy of the devices is not an element"))?;
    copy.set_attribute("class", leaving_class(transition.style))?;
    copy.set_attribute(
        "style",
        &format!("--transition-ms: {}ms;", transition.duration_ms),
    )?;
    copy.set_attribute("aria-hidden", "true")?;
    let pictures = copy.query_selector_all("img[data-media]")?;
    for index in 0..pictures.length() {
        if let Some(picture) = pictures
            .item(index)
            .and_then(|node| node.dyn_into::<Element>().ok())
        {
            picture.remove_attribute("data-media")?;
        }
    }
    freeze(old, &copy, ".tv-world", "transform", frozen_world_style)?;
    freeze(old, &copy, ".tv-card", "left", frozen_card_style)?;
    Ok(copy)
}

/// Takes the leaving copy away when its animation has ended.
pub fn on_animation_end(target: &Element) {
    if target
        .get_attribute("class")
        .is_some_and(|class| is_leaving(&class))
    {
        target.remove();
    }
}

/// Replaces the toolbar, and gives the focus back to the same control when it had it.
fn rebuild_toolbar(document: &Document, page: &mut Page, app: &App) -> Result<(), JsValue> {
    let focused = document
        .active_element()
        .filter(|active| page.toolbar.contains(Some(active)))
        .and_then(|active| active.get_attribute("data-action"));
    let bar = toolbar(document, app)?;
    replace(&page.toolbar, &bar)?;
    page.toolbar = bar;
    if let Some(action) = focused
        && let Some(control) = page
            .toolbar
            .query_selector(&format!("[data-action=\"{action}\"]"))?
        && let Ok(control) = control.dyn_into::<HtmlElement>()
    {
        control.focus()?;
    }
    Ok(())
}

/// Whether the person asked for reduced motion. A scene then changes with a cut: a copy that is
/// never shown would never finish an animation, and so would never be taken away.
fn reduced_motion() -> Result<bool, JsValue> {
    let query = web_sys::window()
        .ok_or_else(|| JsValue::from_str("the page has no window"))?
        .match_media("(prefers-reduced-motion: reduce)")?;
    Ok(query.is_some_and(|query| query.matches()))
}

fn rebuild_devices(
    document: &Document,
    page: &mut Page,
    app: &App,
    leaving: Option<Transition>,
) -> Result<(), JsValue> {
    let leaving = if reduced_motion()? { None } else { leaving };
    let copy = leaving
        .map(|transition| leaving_copy(&page.devices, transition))
        .transpose()?;
    let built = build_devices(document, app)?;
    replace(&page.devices, &built.element)?;
    page.devices = built.element;
    page.tv = built.tv;
    page.slots = built.slots;
    if let Some(copy) = copy {
        append(&page.wrap, &copy)?;
    }
    Ok(())
}

fn redraw_parts(
    document: &Document,
    page: &Page,
    app: &App,
    parts: impl Iterator<Item = Part>,
) -> Result<(), JsValue> {
    let Some(fixture) = shown_fixture(app) else {
        return Ok(());
    };
    for part in parts {
        if let Some(slot) = page.slots.get(&part) {
            fill(slot, build_part(document, part, fixture, app)?)?;
        }
    }
    Ok(())
}

/// Builds the page from nothing.
fn build_page(document: &Document, root: &Element, app: &App) -> Result<Page, JsValue> {
    let shell = element(document, "div", "studio")?;
    let bar = toolbar(document, app)?;
    append(&shell, &bar)?;
    let body = element(document, "div", "studio-body")?;
    let notes = Notes::new(document)?;
    fill_notes(document, &notes, app)?;
    append(&body, &notes.details)?;
    let wrap = element(document, "div", "stage-wrap")?;
    let built = build_devices(document, app)?;
    append(&wrap, &built.element)?;
    append(&body, &wrap)?;
    append(&shell, &body)?;
    clear(root);
    append(root, &shell)?;
    Ok(Page {
        toolbar: bar,
        wrap,
        devices: built.element,
        notes,
        tv: built.tv,
        slots: built.slots,
    })
}

fn draw(document: &Document, root: &Element, app: &mut App) -> Result<(), JsValue> {
    let unlocked = audio::state() == AudioState::Unlocked;
    let now = app.snapshot(unlocked);
    let Some(mut page) = app.page.take() else {
        app.page = Some(build_page(document, root, app)?);
        app.drawn = Some(now);
        return Ok(());
    };
    let plan = DrawPlan::between(app.drawn.as_ref(), &now);
    let result = apply_plan(document, &mut page, app, &plan);
    app.page = Some(page);
    app.drawn = Some(now);
    result
}

fn apply_plan(
    document: &Document,
    page: &mut Page,
    app: &mut App,
    plan: &DrawPlan,
) -> Result<(), JsValue> {
    if plan.toolbar {
        rebuild_toolbar(document, page, app)?;
    }
    if plan.devices {
        let leaving = app.pending_transition.take();
        rebuild_devices(document, page, app, leaving)?;
    } else {
        redraw_parts(document, page, app, plan.parts.iter().copied())?;
        if plan.stage
            && let Some(tv) = &mut page.tv
        {
            let camera = shown_fixture(app).and_then(|fixture| fixture.tv.camera.as_ref());
            let layout = shown_fixture(app).and_then(|fixture| fixture.tv.backdrop_layout());
            tv.sync(document, app.stage_elements(), camera, layout)?;
        }
    }
    if plan.notes {
        fill_notes(document, &page.notes, app)?;
    }
    Ok(())
}

/// Draws what changed since the last draw.
pub fn render() {
    let drawn = document().and_then(|document| {
        let root = document
            .get_element_by_id(ROOT_ID)
            .ok_or_else(|| JsValue::from_str("the playground root is missing"))?;
        with_app_mut(|app| draw(&document, &root, app))
    });
    if let Err(error) = drawn {
        log_error(&format!(
            "could not draw the playground: {}",
            js_message(&error)
        ));
    }
}

/// Moves the figures to the newest canvas and frames the camera again, leaving everything else as
/// it is.
pub fn update_stage() {
    let moved = document().and_then(|document| {
        with_app_mut(|app| {
            let App {
                shown, live, page, ..
            } = app;
            let (camera, layout) = match shown.as_deref() {
                Some(Ok(fixture)) => (fixture.tv.camera.as_ref(), fixture.tv.backdrop_layout()),
                _ => (None, None),
            };
            let elements = stage_elements_of(shown.as_deref(), live.as_ref());
            match page.as_mut().and_then(|page| page.tv.as_mut()) {
                Some(tv) => tv.sync(&document, elements, camera, layout),
                None => Ok(()),
            }
        })
    });
    if let Err(error) = moved {
        view::tv::log_sync_failure(&error);
    }
}

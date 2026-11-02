//! The playground page: loads a fixture, draws it, and follows the host it came from.
//!
//! It owns the page's copy of the state (the loaded fixture, the controls, the live canvas) and the
//! loop that polls for changes. The host owns the controls, so the dev CLI and this page change
//! the same thing: a click is sent to the host and the answer is drawn, and a change made at the
//! CLI arrives on the next poll. A style or fixture file that changes on disk is fetched and drawn again
//! without a rebuild, and the canvas the `df-table` host serves is drawn live. It never decides
//! wording, timing or geometry: the views ask `copy`, `choices` and `layout`.

use std::cell::RefCell;

use gloo_timers::future::TimeoutFuture;
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Document, Element, Event, HtmlSelectElement};

use crate::controls::{Controls, ControlsChange, ControlsReply, Device, FIXTURE_STATUS_KEY};
use crate::copy::{ReadingPace, STATUS_KEYS};
use crate::dom::{append, clear, document, element, set_action, text_element};
use crate::fixture::{ElementSource, Fixture, StageElement};
use crate::net::{fetch_text, post_json};
use crate::scene::SceneOverlay;
use crate::{view_phone, view_tv};

/// Milliseconds between looks at the server for changed files and a changed canvas.
const POLL_MS: u32 = 600;
const REVISION_ADDRESS: &str = "/playground/revision";
const THEME_ADDRESS: &str = "/playground/theme.css";
const FIXTURE_LIST_ADDRESS: &str = "/playground/fixtures.json";
const FIXTURE_DIRECTORY_ADDRESS: &str = "/playground/fixtures/";
const CANVAS_ADDRESS: &str = "/canvas.json";
const CONTROLS_ADDRESS: &str = "/playground/controls.json";
const CHANGE_ADDRESS: &str = "/dev/playground/controls";
const ROOT_ID: &str = "playground";
const THEME_ID: &str = "playground-theme";

/// The canvas the host serves, as far as the stage needs it.
#[derive(Debug, Deserialize)]
struct LiveCanvas {
    revision: u64,
    elements: Vec<StageElement>,
}

#[derive(Default)]
struct App {
    fixture_names: Vec<String>,
    /// The name of the fixture in `fixture`, so a change of choice is noticed.
    loaded_name: Option<String>,
    /// The loaded fixture, or the words that say why it could not be read.
    fixture: Option<Result<Fixture, String>>,
    live: Option<LiveCanvas>,
    asset_revision: String,
    controls: Controls,
    /// Live changes to the scene, drawn on top of the loaded fixture.
    scene: SceneOverlay,
    /// The host's count of changes to the controls when `controls` was read.
    controls_revision: Option<u64>,
    /// Set while the server cannot be reached.
    connection_problem: Option<String>,
}

thread_local! {
    static APP: RefCell<App> = RefCell::new(App::default());
}

fn with_app<R>(read: impl FnOnce(&App) -> R) -> R {
    APP.with(|app| read(&app.borrow()))
}

fn with_app_mut<R>(change: impl FnOnce(&mut App) -> R) -> R {
    APP.with(|app| change(&mut app.borrow_mut()))
}

fn log_error(message: &str) {
    web_sys::console::error_1(&JsValue::from_str(message));
}

fn set_theme(css: &str) -> Result<(), JsValue> {
    let document = document()?;
    let style = document
        .get_element_by_id(THEME_ID)
        .ok_or_else(|| JsValue::from_str("the theme element is missing"))?;
    style.set_text_content(Some(css));
    Ok(())
}

/// Reads the theme and the fixture list, and remembers the revision they came from.
async fn refresh_assets() {
    match fetch_text(THEME_ADDRESS).await {
        Ok(css) => {
            if let Err(error) = set_theme(&css) {
                log_error(&format!("{error:?}"));
            }
        }
        Err(message) => with_app_mut(|app| app.connection_problem = Some(message)),
    }
    match fetch_text(FIXTURE_LIST_ADDRESS).await {
        Ok(text) => match serde_json::from_str::<Vec<String>>(&text) {
            Ok(names) => with_app_mut(|app| {
                app.connection_problem = None;
                app.fixture_names = names;
            }),
            Err(error) => with_app_mut(|app| {
                app.connection_problem =
                    Some(format!("{FIXTURE_LIST_ADDRESS} is not a list: {error}"));
            }),
        },
        Err(message) => with_app_mut(|app| app.connection_problem = Some(message)),
    }
    if let Ok(revision) = fetch_text(REVISION_ADDRESS).await {
        with_app_mut(|app| app.asset_revision = revision);
    }
}

/// What a look at the host's controls found.
#[derive(Debug, Clone, Copy, Default)]
struct ControlsUpdate {
    changed: bool,
    fixture_changed: bool,
}

/// Takes the host's controls as the page's own when they are newer than the ones it has.
fn apply_controls(reply: ControlsReply) -> ControlsUpdate {
    with_app_mut(|app| {
        if app.controls_revision == Some(reply.revision) {
            return ControlsUpdate::default();
        }
        let fixture_changed =
            app.controls.fixture != reply.controls.fixture || app.controls_revision.is_none();
        app.controls = reply.controls;
        app.scene = reply.scene;
        app.controls_revision = Some(reply.revision);
        ControlsUpdate {
            changed: true,
            fixture_changed,
        }
    })
}

/// Reads the host's controls. An unreadable answer is shown as a connection problem.
async fn refresh_controls() -> ControlsUpdate {
    let reply = match fetch_text(CONTROLS_ADDRESS).await {
        Ok(text) => serde_json::from_str::<ControlsReply>(&text)
            .map_err(|error| format!("{CONTROLS_ADDRESS} is not the controls: {error}")),
        Err(message) => Err(message),
    };
    match reply {
        Ok(reply) => apply_controls(reply),
        Err(message) => {
            with_app_mut(|app| app.connection_problem = Some(message));
            ControlsUpdate::default()
        }
    }
}

/// Sends one change to the host and draws what it answers.
async fn send_change(change: ControlsChange) {
    let body = match serde_json::to_string(&change) {
        Ok(body) => body,
        Err(error) => {
            log_error(&format!("could not write the change: {error}"));
            return;
        }
    };
    let answer = post_json(CHANGE_ADDRESS, &body).await.and_then(|text| {
        serde_json::from_str::<ControlsReply>(&text)
            .map_err(|error| format!("{CHANGE_ADDRESS} did not answer with the controls: {error}"))
    });
    match answer {
        Ok(reply) => {
            let update = apply_controls(reply);
            if update.fixture_changed {
                load_selected_fixture().await;
            }
            refresh_live_canvas().await;
        }
        Err(message) => with_app_mut(|app| app.connection_problem = Some(message)),
    }
    render();
}

async fn load_selected_fixture() {
    let Some(name) = with_app(|app| app.controls.fixture.clone()) else {
        with_app_mut(|app| {
            app.fixture = None;
            app.loaded_name = None;
        });
        return;
    };
    let address = format!("{FIXTURE_DIRECTORY_ADDRESS}{name}");
    let loaded = match fetch_text(&address).await {
        Ok(text) => Fixture::parse(&text).map_err(|error| {
            format!(
                "{name}, line {}, column {}: {}",
                error.line, error.column, error.message
            )
        }),
        Err(message) => Err(message),
    };
    with_app_mut(|app| {
        app.fixture = Some(loaded);
        app.loaded_name = Some(name);
    });
}

/// Reads the live canvas when the fixture asks for it, and reports whether it changed.
async fn refresh_live_canvas() -> bool {
    let wants_live = with_app(|app| {
        matches!(
            &app.fixture,
            Some(Ok(fixture)) if matches!(fixture.tv.elements, ElementSource::Live(_))
        )
    });
    if !wants_live {
        return with_app_mut(|app| app.live.take().is_some());
    }
    let Ok(text) = fetch_text(CANVAS_ADDRESS).await else {
        return false;
    };
    let Ok(canvas) = serde_json::from_str::<LiveCanvas>(&text) else {
        return false;
    };
    with_app_mut(|app| {
        let changed = app
            .live
            .as_ref()
            .is_none_or(|known| known.revision != canvas.revision);
        if changed {
            app.live = Some(canvas);
        }
        changed
    })
}

/// The package half of a revision (`<assets>.<package>`).
fn package_part(revision: &str) -> Option<&str> {
    revision.split_once('.').map(|(_, package)| package)
}

/// One look at the server: a rebuilt package reloads the page, changed files reload everything,
/// a changed canvas redraws the stage.
async fn poll() {
    let Ok(revision) = fetch_text(REVISION_ADDRESS).await else {
        let already_known = with_app(|app| app.connection_problem.is_some());
        with_app_mut(|app| {
            app.connection_problem = Some("The playground server is not answering.".to_owned());
        });
        if !already_known {
            render();
        }
        return;
    };
    let package_changed = with_app(|app| {
        matches!(
            (package_part(&app.asset_revision), package_part(&revision)),
            (Some(known), Some(now)) if known != now
        )
    });
    if package_changed {
        if let Some(Err(error)) = web_sys::window().map(|window| window.location().reload()) {
            log_error(&format!("could not reload the page: {error:?}"));
        }
        return;
    }
    let files_changed = with_app(|app| app.asset_revision != revision);
    let was_down = with_app(|app| app.connection_problem.is_some());
    if files_changed || was_down {
        refresh_assets().await;
    }
    let update = refresh_controls().await;
    if files_changed || was_down || update.fixture_changed {
        load_selected_fixture().await;
    }
    let canvas_changed = refresh_live_canvas().await;
    if files_changed || was_down || update.changed || canvas_changed {
        render();
    }
}

async fn run() {
    refresh_assets().await;
    refresh_controls().await;
    load_selected_fixture().await;
    refresh_live_canvas().await;
    render();
    loop {
        TimeoutFuture::new(POLL_MS).await;
        poll().await;
    }
}

fn option(
    document: &Document,
    value: &str,
    label: &str,
    selected: bool,
) -> Result<Element, JsValue> {
    let option = text_element(document, "option", "", label)?;
    option.set_attribute("value", value)?;
    if selected {
        option.set_attribute("selected", "selected")?;
    }
    Ok(option)
}

fn select(
    document: &Document,
    label: &str,
    action: &str,
    options: &[(String, String)],
    chosen: &str,
) -> Result<Element, JsValue> {
    let wrapper = element(document, "label", "control")?;
    append(
        &wrapper,
        &text_element(document, "span", "control-label", label)?,
    )?;
    let select = element(document, "select", "")?;
    set_action(&select, action)?;
    for (value, text) in options {
        append(&select, &option(document, value, text, value == chosen)?)?;
    }
    append(&wrapper, &select)?;
    Ok(wrapper)
}

fn button(document: &Document, action: &str, label: &str, on: bool) -> Result<Element, JsValue> {
    let button = text_element(
        document,
        "button",
        if on { "toggle on" } else { "toggle" },
        label,
    )?;
    button.set_attribute("type", "button")?;
    button.set_attribute("aria-pressed", if on { "true" } else { "false" })?;
    set_action(&button, action)?;
    Ok(button)
}

/// The loaded fixture with the live scene changes drawn on top of it.
fn shown_fixture(app: &App) -> Option<Result<Fixture, String>> {
    app.fixture.as_ref().map(|loaded| {
        loaded.clone().map(|mut fixture| {
            app.scene.apply_to(&mut fixture);
            fixture
        })
    })
}

fn toolbar(document: &Document, app: &App) -> Result<Element, JsValue> {
    let bar = element(document, "header", "toolbar")?;
    append(
        &bar,
        &text_element(document, "h1", "", "Gameplay display playground")?,
    )?;
    let names: Vec<(String, String)> = app
        .fixture_names
        .iter()
        .map(|name| (name.clone(), name.clone()))
        .collect();
    append(
        &bar,
        &select(
            document,
            "Scenario",
            "fixture",
            &names,
            app.controls.fixture.as_deref().unwrap_or(""),
        )?,
    )?;
    let devices = element(document, "div", "control")?;
    append(
        &devices,
        &text_element(document, "span", "control-label", "Show")?,
    )?;
    let segment = element(document, "div", "segment")?;
    for (device, label) in [
        (Device::Both, "Both"),
        (Device::Tv, "Screen"),
        (Device::Phone, "Phone"),
    ] {
        append(
            &segment,
            &button(
                document,
                &format!("device:{}", device.key()),
                label,
                app.controls.device == device,
            )?,
        )?;
    }
    append(&devices, &segment)?;
    append(&bar, &devices)?;
    let mut statuses = vec![(
        FIXTURE_STATUS_KEY.to_owned(),
        "As in the scenario".to_owned(),
    )];
    statuses.extend(
        STATUS_KEYS
            .iter()
            .map(|key| ((*key).to_owned(), key.replace('_', " "))),
    );
    append(
        &bar,
        &select(
            document,
            "Try a status",
            "status",
            &statuses,
            &app.controls.status_key,
        )?,
    )?;
    let paces = [("slow", "Slow"), ("normal", "Normal"), ("fast", "Fast")]
        .map(|(key, label)| (key.to_owned(), label.to_owned()));
    let current_pace = match app.controls.pace {
        ReadingPace::Slow => "slow",
        ReadingPace::Normal => "normal",
        ReadingPace::Fast => "fast",
    };
    append(
        &bar,
        &select(document, "Reading pace", "pace", &paces, current_pace)?,
    )?;
    append(
        &bar,
        &button(
            document,
            "toggle:unavailable",
            "Show unavailable",
            app.controls.show_unavailable,
        )?,
    )?;
    append(
        &bar,
        &button(document, "toggle:grid", "Show grid", app.controls.show_grid)?,
    )?;
    Ok(bar)
}

fn notes(
    document: &Document,
    app: &App,
    shown: &Option<Result<Fixture, String>>,
    warnings: &[String],
) -> Result<Element, JsValue> {
    let panel = element(document, "aside", "notes")?;
    if let Some(problem) = &app.connection_problem {
        append(
            &panel,
            &text_element(document, "div", "note note-error", problem)?,
        )?;
    }
    match shown {
        Some(Ok(fixture)) => {
            append(&panel, &text_element(document, "h2", "", &fixture.name)?)?;
            if !app.scene.is_empty() {
                append(
                    &panel,
                    &text_element(
                        document,
                        "p",
                        "note note-live",
                        "Live changes are drawn on top of this scenario (scene reset removes them).",
                    )?,
                )?;
            }
            if !fixture.description.is_empty() {
                append(
                    &panel,
                    &text_element(document, "p", "", &fixture.description)?,
                )?;
            }
        }
        Some(Err(message)) => {
            append(
                &panel,
                &text_element(document, "h2", "", "This scenario cannot be read")?,
            )?;
            append(
                &panel,
                &text_element(document, "pre", "note note-error", message)?,
            )?;
        }
        None => append(
            &panel,
            &text_element(document, "p", "", "No scenario is loaded.")?,
        )?,
    }
    if !warnings.is_empty() {
        append(
            &panel,
            &text_element(
                document,
                "h2",
                "",
                "Where this scenario breaks the contract",
            )?,
        )?;
        let list = element(document, "ul", "warnings")?;
        for warning in warnings {
            append(&list, &text_element(document, "li", "", warning)?)?;
        }
        append(&panel, &list)?;
    }
    Ok(panel)
}

fn devices(
    document: &Document,
    app: &App,
    shown: &Option<Result<Fixture, String>>,
) -> Result<(Element, Vec<String>), JsValue> {
    let stage = element(document, "main", "devices")?;
    let mut warnings = Vec::new();
    let Some(Ok(fixture)) = shown else {
        return Ok((stage, warnings));
    };
    if app.controls.device.shows_tv() {
        let elements: &[StageElement] = match &fixture.tv.elements {
            ElementSource::Inline(inline) => inline,
            ElementSource::Live(_) => app
                .live
                .as_ref()
                .map_or(&[], |live| live.elements.as_slice()),
        };
        let status = app.controls.status_for(&fixture.tv.status);
        append(
            &stage,
            &view_tv::render(document, &fixture.tv, elements, &status, &app.controls)?,
        )?;
    }
    if app.controls.device.shows_phone() {
        let status = app.controls.status_for(&fixture.phone.status);
        let (phone, found) = view_phone::render(document, &fixture.phone, &status, &app.controls)?;
        warnings = found;
        append(&stage, &phone)?;
    }
    Ok((stage, warnings))
}

fn build_page(document: &Document, app: &App) -> Result<Element, JsValue> {
    let shell = element(document, "div", "shell")?;
    append(&shell, &toolbar(document, app)?)?;
    let shown = shown_fixture(app);
    let (stage, warnings) = devices(document, app, &shown)?;
    let body = element(document, "div", "body")?;
    append(&body, &stage)?;
    append(&body, &notes(document, app, &shown, &warnings)?)?;
    append(&shell, &body)?;
    Ok(shell)
}

fn render() {
    let drawn = document().and_then(|document| {
        let root = document
            .get_element_by_id(ROOT_ID)
            .ok_or_else(|| JsValue::from_str("the playground root is missing"))?;
        let page = with_app(|app| build_page(&document, app))?;
        clear(&root);
        append(&root, &page)
    });
    if let Err(error) = drawn {
        log_error(&format!("could not draw the playground: {error:?}"));
    }
}

fn action_of(event: &Event) -> Option<(Element, String)> {
    let target = event.target()?.dyn_into::<Element>().ok()?;
    let carrier = target.closest("[data-action]").ok()??;
    let action = carrier.get_attribute("data-action")?;
    Some((carrier, action))
}

fn on_click(event: &Event) {
    let Some((_, action)) = action_of(event) else {
        return;
    };
    let change = with_app(|app| match action.split_once(':') {
        Some(("device", key)) => Device::from_key(key).map(ControlsChange::Device),
        Some(("toggle", "unavailable")) => Some(ControlsChange::ShowUnavailable(
            !app.controls.show_unavailable,
        )),
        Some(("toggle", "grid")) => Some(ControlsChange::ShowGrid(!app.controls.show_grid)),
        _ => None,
    });
    if let Some(change) = change {
        spawn_local(send_change(change));
    }
}

fn on_change(event: &Event) {
    let Some((carrier, action)) = action_of(event) else {
        return;
    };
    let Ok(select) = carrier.dyn_into::<HtmlSelectElement>() else {
        return;
    };
    let value = select.value();
    let change = match action.as_str() {
        "fixture" => Some(ControlsChange::Fixture(value)),
        "status" => Some(ControlsChange::Status(value)),
        "pace" => ReadingPace::from_key(&value).map(ControlsChange::Pace),
        _ => None,
    };
    if let Some(change) = change {
        spawn_local(send_change(change));
    }
}

/// Builds the page's skeleton, wires its two listeners and starts the loop.
pub fn start() -> Result<(), JsValue> {
    std::panic::set_hook(Box::new(|info| {
        log_error(&format!("playground panic: {info}"))
    }));
    let document = document()?;
    let body = document
        .body()
        .ok_or_else(|| JsValue::from_str("the page has no body"))?;
    let style = element(&document, "style", "")?;
    style.set_attribute("id", THEME_ID)?;
    append(&body, &style)?;
    let root = element(&document, "div", "")?;
    root.set_attribute("id", ROOT_ID)?;
    append(&body, &root)?;

    // Two listeners on the root serve every control, and they live as long as the page, so they
    // are leaked on purpose: nothing ever removes them.
    let click = Closure::<dyn FnMut(Event)>::new(|event: Event| on_click(&event));
    root.add_event_listener_with_callback("click", click.as_ref().unchecked_ref())?;
    click.forget();
    let change = Closure::<dyn FnMut(Event)>::new(|event: Event| on_change(&event));
    root.add_event_listener_with_callback("change", change.as_ref().unchecked_ref())?;
    change.forget();

    spawn_local(run());
    Ok(())
}

//! The playground page: loads a fixture, draws it, and follows the host it came from.
//!
//! It owns the page's copy of the state (`state`), the loop that follows the host (`frames`), the
//! files it reads (`assets`), the drawing of the page as pieces (`page`, `toolbar`), the sound
//! (`sound`), what it tells the host (`report`) and what it does when someone acts (`events`). The
//! host owns the controls, so the dev CLI and this page change the same thing: a click is sent to
//! the host and the answer is drawn, and a change made at the CLI arrives in the next frame. A
//! style or fixture file that changes on disk is fetched and drawn again without a rebuild, and
//! the canvas the `df-table` host serves is drawn live. It never decides wording, timing or
//! geometry: the views ask `copy`, `choices` and `layout`.

mod assets;
mod events;
mod frames;
mod page;
mod report;
mod seat;
mod sound;
mod state;
mod toolbar;
mod voice;

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::Event;

use crate::dom::{append, document, element, js_message};
use crate::role::PageRole;
use crate::timeline::ErrorSource;
use events::{
    on_animation_ended, on_animation_started, on_change, on_click, on_input, on_key,
    on_media_event, on_transition_ended, on_visibility_change,
};
use page::place_bubbles;
use state::{ROOT_ID, THEME_ID, log_error, with_app_mut};

thread_local! {
    /// Set by the first panic, so a panic while reporting one does not report again for ever.
    static PANICKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Where a phone keeps its name in the browser's storage, so a reload is the same phone.
const PHONE_NAME_KEY: &str = "df-phone-client";

/// 32 random bits for a page's name.
fn random_bits() -> u32 {
    // `Math.random()` is in [0, 1), so the product is below 2^32 and the cast keeps it whole.
    (js_sys::Math::random() * 4_294_967_296.0) as u32
}

/// The value a phone keeps across reloads under `storage_key`: the one in the browser's storage
/// when `is_valid` accepts it, or a new one from `make` put there. Storage that cannot be read or
/// written (a private window, storage turned off) gives a new value for this load only, and the
/// failure is noted for the report as about the phone's `what`.
fn kept_in_storage(
    window: &web_sys::Window,
    storage_key: &str,
    what: &str,
    is_valid: impl Fn(&str) -> bool,
    make: impl FnOnce() -> Result<String, JsValue>,
) -> Result<String, JsValue> {
    let storage = match window.local_storage() {
        Ok(storage) => storage,
        Err(error) => {
            log_error(
                ErrorSource::Asset,
                &format!(
                    "could not open the browser's storage: {}",
                    js_message(&error)
                ),
            );
            None
        }
    };
    // A read that fails is not noted here: a new value is made, and the write after it says why.
    if let Some(storage) = &storage
        && let Ok(Some(kept)) = storage.get_item(storage_key)
        && is_valid(&kept)
    {
        return Ok(kept);
    }
    let made = make()?;
    if let Some(storage) = &storage
        && let Err(error) = storage.set_item(storage_key, &made)
    {
        log_error(
            ErrorSource::Asset,
            &format!("could not keep the phone's {what}: {}", js_message(&error)),
        );
    }
    Ok(made)
}

/// The name this phone keeps across reloads (`kept_in_storage`).
fn kept_phone_name(window: &web_sys::Window) -> Result<String, JsValue> {
    kept_in_storage(
        window,
        PHONE_NAME_KEY,
        "name",
        |kept| PageRole::Phone.is_client_name(kept),
        || Ok(PageRole::Phone.client_name(random_bits())),
    )
}

/// Draws the phone kit board the address names into `root`. Returns whether it drew one.
#[cfg(debug_assertions)]
fn draw_phone_gallery(
    document: &web_sys::Document,
    root: &web_sys::Element,
) -> Result<bool, JsValue> {
    // The demo is one clickable phone screen: its taps are page navigation, through the page's one
    // click routing.
    if let Some(demo) = crate::phone_kit::draw::nav::demo_page(document)? {
        append(root, &demo)?;
        let click = Closure::<dyn FnMut(Event)>::new(|event: Event| {
            if let Some((_, action)) = events::action_of(&event) {
                crate::phone_kit::draw::nav::on_action(&action);
            }
        });
        root.add_event_listener_with_callback("click", click.as_ref().unchecked_ref())?;
        click.forget();
        return Ok(true);
    }
    let board = match crate::phone_kit::draw::gallery::page(document)? {
        Some(page) => Some(page),
        None => crate::view::seat::gallery_page(document)?,
    };
    let Some(page) = board else {
        return Ok(false);
    };
    append(root, &page)?;
    Ok(true)
}

/// A release build has no gallery.
#[cfg(not(debug_assertions))]
fn draw_phone_gallery(
    _document: &web_sys::Document,
    _root: &web_sys::Element,
) -> Result<bool, JsValue> {
    Ok(false)
}

/// Builds the page's skeleton, wires its listeners and starts the loop.
pub fn start() -> Result<(), JsValue> {
    std::panic::set_hook(Box::new(|info| {
        let forced = log_error(ErrorSource::Panic, &format!("playground panic: {info}"));
        if forced && !PANICKED.with(|panicked| panicked.replace(true)) {
            spawn_local(report::send_report(true));
        }
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
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("the page has no window"))?;
    let role = PageRole::of_path(&window.location().pathname()?);
    // The phone kit's gallery is a static page of sample screens: it needs the theme and nothing
    // else, so it takes none of the page's listeners and follows no host.
    if role == PageRole::Phone && draw_phone_gallery(&document, &root)? {
        spawn_local(assets::refresh_assets());
        return Ok(());
    }

    // Listeners on the root serve every control, and they live as long as the page, so they are
    // leaked on purpose: nothing ever removes them.
    let click = Closure::<dyn FnMut(Event)>::new(|event: Event| on_click(&event));
    root.add_event_listener_with_callback("click", click.as_ref().unchecked_ref())?;
    click.forget();
    let change = Closure::<dyn FnMut(Event)>::new(|event: Event| on_change(&event));
    root.add_event_listener_with_callback("change", change.as_ref().unchecked_ref())?;
    change.forget();

    let input = Closure::<dyn FnMut(Event)>::new(|event: Event| on_input(&event));
    root.add_event_listener_with_callback("input", input.as_ref().unchecked_ref())?;
    input.forget();

    // A key press is a gesture too (and Enter in the composer's box sends), and a picture says when
    // it has loaded or failed. Those events do not bubble, so they are caught on the way down.
    let key = Closure::<dyn FnMut(Event)>::new(|event: Event| on_key(&event));
    body.add_event_listener_with_callback("keydown", key.as_ref().unchecked_ref())?;
    key.forget();
    let visibility = Closure::<dyn FnMut(Event)>::new(|_: Event| on_visibility_change());
    document.add_event_listener_with_callback(
        "visibilitychange",
        visibility.as_ref().unchecked_ref(),
    )?;
    visibility.forget();
    let started = Closure::<dyn FnMut(Event)>::new(|event: Event| on_animation_started(&event));
    root.add_event_listener_with_callback("animationstart", started.as_ref().unchecked_ref())?;
    started.forget();
    let finished = Closure::<dyn FnMut(Event)>::new(|event: Event| on_animation_ended(&event));
    root.add_event_listener_with_callback("animationend", finished.as_ref().unchecked_ref())?;
    finished.forget();
    // The fold eases the board's width: once it has settled, the bubbles are placed for its size.
    let settled = Closure::<dyn FnMut(Event)>::new(|event: Event| on_transition_ended(&event));
    root.add_event_listener_with_callback("transitionend", settled.as_ref().unchecked_ref())?;
    settled.forget();
    let loaded = Closure::<dyn FnMut(Event)>::new(|event: Event| on_media_event(&event, false));
    root.add_event_listener_with_callback_and_bool("load", loaded.as_ref().unchecked_ref(), true)?;
    loaded.forget();
    let failed = Closure::<dyn FnMut(Event)>::new(|event: Event| on_media_event(&event, true));
    root.add_event_listener_with_callback_and_bool("error", failed.as_ref().unchecked_ref(), true)?;
    failed.forget();

    // A window that changes size changes the stage's, and the bubbles are placed for its size.
    let resized = Closure::<dyn FnMut(Event)>::new(|_: Event| place_bubbles());
    window.add_event_listener_with_callback("resize", resized.as_ref().unchecked_ref())?;
    resized.forget();

    crate::view::roll::preload_dice()?;
    let client_id = match role {
        PageRole::Phone => kept_phone_name(&window)?,
        PageRole::Playground => role.client_name(random_bits()),
    };
    crate::net::set_client(&client_id);
    with_app_mut(|app| {
        app.client_id = client_id;
        app.role = role;
    });
    // A real phone takes a seat before it plays; the join screens lie over its page until then.
    if role == PageRole::Phone
        && let Err(error) = seat::start(&document, &window)
    {
        log_error(
            ErrorSource::Draw,
            &format!("the phone cannot take a seat: {}", js_message(&error)),
        );
    }
    spawn_local(voice::refresh_host_state());
    spawn_local(frames::run());
    Ok(())
}

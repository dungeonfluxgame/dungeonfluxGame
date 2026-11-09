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
mod sound;
mod state;
mod toolbar;
mod voice;

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::Event;

use crate::dom::{append, document, element};
use crate::timeline::ErrorSource;
use events::{
    on_animation_ended, on_change, on_click, on_input, on_key, on_media_event, on_transition_ended,
    on_visibility_change,
};
use page::place_bubbles;
use state::{ROOT_ID, THEME_ID, log_error, with_app_mut};

thread_local! {
    /// Set by the first panic, so a panic while reporting one does not report again for ever.
    static PANICKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("the page has no window"))?;
    let resized = Closure::<dyn FnMut(Event)>::new(|_: Event| place_bubbles());
    window.add_event_listener_with_callback("resize", resized.as_ref().unchecked_ref())?;
    resized.forget();

    let client_id = format!(
        "page-{:08x}",
        (js_sys::Math::random() * 4_294_967_295.0) as u32
    );
    crate::net::set_client(&client_id);
    with_app_mut(|app| app.client_id = client_id);
    spawn_local(voice::refresh_host_state());
    spawn_local(frames::run());
    Ok(())
}

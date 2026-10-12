//! Spoken narration and NPC lines on the shared display only: the voice player.
//!
//! The browser decides nothing about order. The stage conductor (`stage_conductor`) owns the
//! line and clip machines; this module only does what it is told and reports what happened.
//! `request` fetches and decodes the audio of a line from the server (`/gameplay/voice/<key>`),
//! `play` plays a requested clip, `cancel` drops it, and the player reports `Loaded`,
//! `Started`, `Finished` or `Error` for the line so its machine follows the real audio. A
//! non-200 answer, a decode failure or a blocked play call is an `Error`, and the line then runs
//! on reading time. Phones never reach this module, so a room does not echo.
//!
//! Browsers refuse audio until the page has had a user gesture, so the display shows one
//! small "Enable sound" control until a click or key press, then a mute toggle that is
//! kept only until the page reloads (a reload starts locked and the first press enables
//! sound). The conductor asks for clips only while sound is on.
use super::*;
use crate::sound_switch::{Press, Sound, SoundSwitch};
use crate::stage_conductor::{ClipSignal, LineId};
use crate::voice_line::VoiceLine;
use std::collections::HashMap;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Blob, BlobPropertyBag, Event, EventTarget, HtmlAudioElement, Response, Url};

const VOICE_PATH: &str = "/gameplay/voice";
const MAXIMUM_CLIP_BYTES: u32 = 4 * 1024 * 1024;
/// A zero-length WAV played inside the first gesture so Safari lets this element play later.
const UNLOCK_CLIP: &str =
    "data:audio/wav;base64,UklGRiQAAABXQVZFZm10IBAAAAABAAEAQB8AAEAfAAABAAgAZGF0YQAAAAA=";
// Sized for a TV seen from across a room: the label never drops below 24px.
const CONTROL_STYLE: &str = "position:fixed;right:2.4vw;top:2.6vh;z-index:40;display:flex;\
align-items:center;gap:.6em;min-height:2.4em;padding:.45em 1em;border-radius:999px;\
border:1px solid rgba(223,185,116,.55);background:rgba(8,15,23,.78);color:#efe6ce;\
font:clamp(24px,1.35vw,34px) \"Iowan Old Style\",\"Palatino Linotype\",Palatino,Georgia,serif;\
letter-spacing:.03em;cursor:pointer;opacity:.92";
const ICON_ATTRIBUTES: &str = "viewBox=\"0 0 24 24\" width=\"1.1em\" height=\"1.1em\" aria-hidden=\"true\" \
fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"";
const SPEAKER_PATHS: &str = "<path d=\"M11 5 6 9H3v6h3l5 4z\"/>";
const WAVE_PATHS: &str =
    "<path d=\"M15.5 8.5a5 5 0 0 1 0 7\"/><path d=\"M18.5 5.5a9 9 0 0 1 0 13\"/>";
const MUTE_PATHS: &str = "<path d=\"M16 9l6 6M22 9l-6 6\"/>";

/// A requested clip that is not playing yet.
enum Clip {
    Fetching,
    /// Decoded and waiting for its turn, as an object URL.
    Ready(String),
}

/// The clip on the audio element.
struct Current {
    id: LineId,
    url: String,
    /// `Started` was reported (the `playing` event repeats after buffering).
    started: bool,
}

struct Director {
    audio: HtmlAudioElement,
    control: Element,
    icon: Element,
    label: Element,
    switch: SoundSwitch,
    clips: HashMap<LineId, Clip>,
    current: Option<Current>,
    listeners: Vec<Closure<dyn FnMut(Event)>>,
}

type Shared = Rc<RefCell<Director>>;

thread_local! {static DIRECTOR:RefCell<Option<Shared>>=const{RefCell::new(None)};}

impl Director {
    fn sound(&self) -> Sound {
        self.switch.sound()
    }

    fn render_control(&self) {
        let (state, label, paths, name) = match self.sound() {
            Sound::Locked => ("locked", "Enable sound", WAVE_PATHS, "Enable sound"),
            Sound::On => ("on", "", WAVE_PATHS, "Mute narration"),
            Sound::Muted => ("muted", "", MUTE_PATHS, "Unmute narration"),
        };
        let _marked = self.control.set_attribute("data-voice-state", state);
        let _named = self.control.set_attribute("aria-label", name);
        let _titled = self.control.set_attribute("title", name);
        self.label.set_text_content(Some(label));
        self.icon.set_inner_html(&format!(
            "<svg {ICON_ATTRIBUTES}>{SPEAKER_PATHS}{paths}</svg>"
        ));
    }

    fn plays(&self, url: &str) -> bool {
        self.current
            .as_ref()
            .is_some_and(|current| current.url == url)
    }

    /// Frees an object URL; a refusal only leaves a little memory until the page closes.
    fn release(url: &str) {
        let _revoked = Url::revoke_object_url(url);
    }
}

/// What the music mix reads from the one shared sound state: the unlock gesture and mute
/// choice live here, and music follows them.
#[derive(Clone, Copy)]
pub(super) struct SoundState {
    /// Unlocked by a gesture and not muted.
    pub(super) enabled: bool,
    /// A voice clip is audibly playing right now.
    pub(super) speaking: bool,
}

/// `None` when the voice director does not exist yet or is mid-update; callers keep what
/// they last read.
pub(super) fn sound_state() -> Option<SoundState> {
    let shared = DIRECTOR.with(|slot| slot.borrow().clone())?;
    let state = shared.try_borrow().ok()?;
    let clip_playing = state.current.is_some() && !state.audio.paused() && !state.audio.ended();
    Some(SoundState {
        enabled: state.sound() == Sound::On,
        speaking: clip_playing,
    })
}

/// What the sound-effects engine may do right now.
pub(super) struct EffectsGate {
    /// A narration clip is speaking, so effects play quieter.
    pub(super) narrating: bool,
}

fn with_director<T>(read: impl FnOnce(&Director) -> T) -> Option<T> {
    DIRECTOR.with(|slot| {
        let shared = slot.borrow().clone()?;
        // A mutable borrow in progress means a state change is mid-flight: skip, not panic.
        let state = shared.try_borrow().ok()?;
        Some(read(&state))
    })
}

/// `Some` only while audio is unlocked and unmuted: the one sound control governs effects too.
pub(super) fn effects_gate() -> Option<EffectsGate> {
    with_director(|state| {
        (state.sound() == Sound::On).then(|| EffectsGate {
            narrating: state.current.is_some() && !state.audio.paused(),
        })
    })
    .flatten()
}

/// `None` while audio is still locked, otherwise whether the viewer muted sound.
pub(super) fn effects_muted() -> Option<bool> {
    with_director(|state| {
        state
            .switch
            .is_unlocked()
            .then_some(state.switch.is_muted())
    })
    .flatten()
}

/// Called once per newly applied view: makes sure the sound control exists. Voice is best
/// effort: a failure here marks the display root and never affects rendering.
pub(super) fn observe(client: &Rc<RefCell<Client>>) {
    let (document, root) = {
        let state = client.borrow();
        if state.role != "display" {
            return;
        }
        (state.document.clone(), state.root.clone())
    };
    if director(&document, &root).is_err() {
        let _marked = root.set_attribute("data-voice", "unavailable");
    }
}

/// Creates the shared sound control on `root` without any view, for the battlefield preview:
/// the same unlock gesture and mute toggle then govern the effects there too.
pub(super) fn attach(document: &Document, root: &Element) -> Result<(), JsValue> {
    director(document, root).map(|_| ())
}

fn director(document: &Document, root: &Element) -> Result<Shared, JsValue> {
    let existing = DIRECTOR.with(|slot| slot.borrow().clone());
    let shared = match existing {
        Some(shared) => shared,
        None => {
            let created = create(document)?;
            DIRECTOR.with(|slot| *slot.borrow_mut() = Some(created.clone()));
            created
        }
    };
    let control = shared.borrow().control.clone();
    if !control.is_connected() {
        root.append_child(&control)?;
    }
    Ok(shared)
}

fn create(document: &Document) -> Result<Shared, JsValue> {
    let audio = HtmlAudioElement::new()?;
    let control = document.create_element("button")?;
    control.set_attribute("type", "button")?;
    control.set_attribute("class", "tv-voice-control")?;
    control.set_attribute("style", CONTROL_STYLE)?;
    let icon = node(document, &control, "span", "tv-voice-icon", "")?;
    let label = node(document, &control, "span", "tv-voice-label", "")?;
    let shared = Rc::new(RefCell::new(Director {
        audio: audio.clone(),
        control: control.clone(),
        icon,
        label,
        switch: SoundSwitch::new(),
        clips: HashMap::new(),
        current: None,
        listeners: Vec::new(),
    }));
    shared.borrow().render_control();
    listen(&shared, audio.as_ref(), "playing", on_playing)?;
    listen(&shared, audio.as_ref(), "ended", on_ended)?;
    listen(&shared, audio.as_ref(), "error", on_error)?;
    listen(&shared, control.as_ref(), "click", on_control)?;
    listen(&shared, document.as_ref(), "pointerdown", on_gesture)?;
    listen(&shared, document.as_ref(), "keydown", on_gesture)?;
    Ok(shared)
}

/// The director lives as long as the page, so its listeners are kept, not removed. Each
/// callback holds only a weak reference to avoid a reference cycle.
fn listen(
    shared: &Shared,
    target: &EventTarget,
    name: &'static str,
    handler: fn(&Shared, &Event),
) -> Result<(), JsValue> {
    let weak = Rc::downgrade(shared);
    let callback = Closure::wrap(Box::new(move |event: Event| {
        if let Some(shared) = weak.upgrade() {
            handler(&shared, &event);
        }
    }) as Box<dyn FnMut(Event)>);
    target.add_event_listener_with_callback(name, callback.as_ref().unchecked_ref())?;
    shared.borrow_mut().listeners.push(callback);
    Ok(())
}

/// Any pointer or key press on the page unlocks audio, the control included: the click that
/// follows on the control is part of the same gesture (see `sound_switch`).
fn on_gesture(shared: &Shared, _event: &Event) {
    let first = shared.borrow_mut().switch.gesture(now_ms());
    if first {
        start_unlocked(shared);
    }
}

fn on_control(shared: &Shared, _event: &Event) {
    // Read the state first: a borrow held across the match would still be live when the
    // arms borrow mutably, which aborts the whole client.
    let pressed = shared.borrow_mut().switch.press(now_ms());
    match pressed {
        Press::Unlocked => start_unlocked(shared),
        Press::SameGesture => {}
        Press::Muted => apply_mute(shared, true),
        Press::Unmuted => apply_mute(shared, false),
    }
}

fn now_ms() -> u64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0, |performance| performance.now() as u64)
}

/// Audio was just unlocked by a gesture: play the unlock clip inside it.
fn start_unlocked(shared: &Shared) {
    {
        let state = shared.borrow();
        state.render_control();
        state.audio.set_src(UNLOCK_CLIP);
        // Playing inside the gesture is what unlocks the element; a browser that refuses
        // here is caught later by the real clip's own play call.
        if let Ok(promise) = state.audio.play() {
            spawn_local(async move {
                let _settled = JsFuture::from(promise).await;
            });
        }
    }
    live_sfx::sync();
    live_motion::sound_changed(true);
}

/// The switch already holds the new mute choice; stop the clip on the air to match it. The
/// line it belonged to ends (its machine sees an error) and later lines run on reading time.
fn apply_mute(shared: &Shared, muted: bool) {
    let stopped = {
        let mut state = shared.borrow_mut();
        if muted {
            let _paused = state.audio.pause();
        }
        state.render_control();
        if muted { state.current.take() } else { None }
    };
    live_sfx::sync();
    live_motion::sound_changed(!muted);
    if let Some(current) = stopped {
        Director::release(&current.url);
        live_motion::clip_signal(current.id, ClipSignal::Error);
    }
}

/// Reports a signal after the current event handler has returned, so a signal never nests
/// inside the call that produced it.
fn signal_later(id: LineId, signal: ClipSignal) {
    spawn_local(async move {
        live_motion::clip_signal(id, signal);
    });
}

/// Fetches and decodes the audio of the line `id`, without playing it.
pub(super) fn request(id: LineId, line: &VoiceLine) {
    let Some(shared) = DIRECTOR.with(|slot| slot.borrow().clone()) else {
        signal_later(id, ClipSignal::Error);
        return;
    };
    {
        let mut state = shared.borrow_mut();
        if state.clips.contains_key(&id) {
            return;
        }
        state.clips.insert(id, Clip::Fetching);
    }
    let key = line.key();
    spawn_local(async move {
        let fetched = fetch_clip(&key).await;
        let signal = {
            let mut state = shared.borrow_mut();
            let wanted = matches!(state.clips.get(&id), Some(Clip::Fetching));
            match (wanted, fetched) {
                (true, Ok(url)) => {
                    state.clips.insert(id, Clip::Ready(url));
                    Some(ClipSignal::Loaded)
                }
                (true, Err(_)) => {
                    state.clips.remove(&id);
                    Some(ClipSignal::Error)
                }
                // Cancelled while it was on its way.
                (false, Ok(url)) => {
                    Director::release(&url);
                    None
                }
                (false, Err(_)) => None,
            }
        };
        if let Some(signal) = signal {
            live_motion::clip_signal(id, signal);
        }
    });
}

/// Plays the decoded clip of the line `id`. The player reports `Started` when the browser
/// begins playing it, `Finished` when it ends, or `Error` when the play call is refused.
pub(super) fn play(id: LineId) {
    let Some(shared) = DIRECTOR.with(|slot| slot.borrow().clone()) else {
        signal_later(id, ClipSignal::Error);
        return;
    };
    let started = {
        let mut state = shared.borrow_mut();
        match state.clips.remove(&id) {
            Some(Clip::Ready(url)) => {
                state.audio.set_src(&url);
                state.current = Some(Current {
                    id,
                    url,
                    started: false,
                });
                Some(state.audio.play())
            }
            Some(other) => {
                state.clips.insert(id, other);
                None
            }
            None => None,
        }
    };
    match started {
        Some(Ok(promise)) => spawn_local(async move {
            if let Err(refusal) = JsFuture::from(promise).await {
                refused(&shared, id, &refusal);
            }
        }),
        Some(Err(refusal)) => refused(&shared, id, &refusal),
        None => signal_later(id, ClipSignal::Error),
    }
}

/// Drops the clip of the line `id`; if it is on the air it stops. The line's machine has
/// already moved on, so nothing is reported.
pub(super) fn cancel(id: LineId) {
    let Some(shared) = DIRECTOR.with(|slot| slot.borrow().clone()) else {
        return;
    };
    let mut state = shared.borrow_mut();
    if let Some(Clip::Ready(url)) = state.clips.remove(&id) {
        Director::release(&url);
    }
    if state
        .current
        .as_ref()
        .is_some_and(|current| current.id == id)
    {
        let _paused = state.audio.pause();
        if let Some(current) = state.current.take() {
            Director::release(&current.url);
        }
    }
}

/// A play call the browser refused: if it was the autoplay policy the control comes back and
/// sound is off again; either way the line runs on reading time.
fn refused(shared: &Shared, id: LineId, refusal: &JsValue) {
    let blocked = js_sys::Reflect::get(refusal, &JsValue::from_str("name"))
        .ok()
        .and_then(|name| name.as_string())
        .is_some_and(|name| name == "NotAllowedError");
    let released = {
        let mut state = shared.borrow_mut();
        if !state
            .current
            .as_ref()
            .is_some_and(|current| current.id == id)
        {
            return;
        }
        if blocked {
            state.switch.refused();
            state.render_control();
        }
        state.current.take()
    };
    if let Some(current) = released {
        Director::release(&current.url);
    }
    if blocked {
        live_motion::sound_changed(false);
    }
    live_motion::clip_signal(id, ClipSignal::Error);
}

fn on_playing(shared: &Shared, _event: &Event) {
    let reported = {
        let mut state = shared.borrow_mut();
        let url = state.audio.current_src();
        let seconds = state.audio.duration();
        match state.current.as_mut() {
            Some(current) if current.url == url && !current.started => {
                current.started = true;
                let duration_ms = (seconds.is_finite() && seconds > 0.0)
                    .then(|| (seconds * 1000.0).min(f64::from(u32::MAX)) as u32);
                Some((current.id, duration_ms))
            }
            _ => None,
        }
    };
    if let Some((id, duration_ms)) = reported {
        live_motion::clip_signal(id, ClipSignal::Started { duration_ms });
    }
}

/// `ended` or `error`, but only for the real clip, never for the unlock clip.
fn finish(shared: &Shared, signal: ClipSignal) {
    let over = {
        let mut state = shared.borrow_mut();
        let url = state.audio.current_src();
        if state.plays(&url) {
            state.current.take()
        } else {
            None
        }
    };
    if let Some(current) = over {
        Director::release(&current.url);
        live_motion::clip_signal(current.id, signal);
    }
}

fn on_ended(shared: &Shared, _event: &Event) {
    finish(shared, ClipSignal::Finished);
}

fn on_error(shared: &Shared, _event: &Event) {
    finish(shared, ClipSignal::Error);
}

/// Fetches one clip and returns an object URL for it. Anything but a non-empty, bounded
/// 200 response is an error and the line stays silent.
async fn fetch_clip(key: &str) -> Result<String, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window absent"))?;
    let response: Response = JsFuture::from(window.fetch_with_str(&format!("{VOICE_PATH}/{key}")))
        .await?
        .dyn_into()?;
    if response.status() != 200 {
        return Err(JsValue::from_str("voice unavailable"));
    }
    let buffer = JsFuture::from(response.array_buffer()?).await?;
    let bytes = js_sys::Uint8Array::new(&buffer);
    if bytes.length() == 0 || bytes.length() > MAXIMUM_CLIP_BYTES {
        return Err(JsValue::from_str("voice clip size refused"));
    }
    let options = BlobPropertyBag::new();
    options.set_type("audio/mpeg");
    let blob = Blob::new_with_u8_array_sequence_and_options(&js_sys::Array::of1(&bytes), &options)?;
    Url::create_object_url_with_blob(&blob)
}

//! Spoken narration and NPC lines on the shared display only.
//!
//! The browser decides nothing: for every accepted display view it asks the server for the
//! audio of the lines the view carries (`/gameplay/voice/<key>`) and plays what comes back,
//! one clip at a time. A non-200 answer, a decode failure or a blocked play call just skips
//! the line. Phones never reach this module, so a room does not echo.
//!
//! Browsers refuse audio until the page has had a user gesture, so the display shows one
//! small "Enable sound" control until a click or key press, then a mute toggle that is
//! remembered for the tab. While a clip plays the element holding its text gets the
//! `speaking` class.
use super::*;
use crate::voice_line::{VoiceLine, display_lines};
use std::collections::VecDeque;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Blob, BlobPropertyBag, Event, EventTarget, HtmlAudioElement, Node, Response, Url};

const VOICE_PATH: &str = "/gameplay/voice";
const MUTED_KEY: &str = "df.voice.muted";
const SPEAKING_CLASS: &str = "speaking";
const SPOKEN_MEMORY: usize = 256;
const MAXIMUM_CLIP_BYTES: u32 = 4 * 1024 * 1024;
/// A zero-length WAV played inside the first gesture so Safari lets this element play later.
const UNLOCK_CLIP: &str =
    "data:audio/wav;base64,UklGRiQAAABXQVZFZm10IBAAAAABAAEAQB8AAEAfAAABAAgAZGF0YQAAAAA=";
// Sized for a TV seen from across a room: the label never drops below 24px.
const CONTROL_STYLE: &str = "position:fixed;right:2.4vw;bottom:2.6vh;z-index:40;display:flex;\
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

#[derive(Clone, Copy, Eq, PartialEq)]
enum Sound {
    Locked,
    On,
    Muted,
}

/// The clip being fetched or played.
struct Playing {
    line: VoiceLine,
    speaking: Option<Element>,
    object_url: Option<String>,
}

impl Playing {
    /// Clears the speaking mark and frees the clip's object URL. Both are cosmetic or
    /// memory cleanup of a clip that is already over, so a failure changes nothing visible.
    fn release(self) {
        if let Some(element) = self.speaking {
            let _cleared = element.class_list().remove_1(SPEAKING_CLASS);
        }
        if let Some(url) = self.object_url {
            let _revoked = Url::revoke_object_url(&url);
        }
    }
}

struct Director {
    document: Document,
    audio: HtmlAudioElement,
    control: Element,
    icon: Element,
    label: Element,
    unlocked: bool,
    muted: bool,
    /// Lines of the newest accepted view; older views' queued lines are discarded.
    latest: Vec<VoiceLine>,
    queue: VecDeque<VoiceLine>,
    spoken: VecDeque<String>,
    current: Option<Playing>,
    play_id: u64,
    listeners: Vec<Closure<dyn FnMut(Event)>>,
}

type Shared = Rc<RefCell<Director>>;

thread_local! {static DIRECTOR:RefCell<Option<Shared>>=const{RefCell::new(None)};}

impl Director {
    fn sound(&self) -> Sound {
        match (self.unlocked, self.muted) {
            (false, _) => Sound::Locked,
            (true, false) => Sound::On,
            (true, true) => Sound::Muted,
        }
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

    fn has_spoken(&self, key: &str) -> bool {
        self.spoken.iter().any(|known| known == key)
    }

    fn remember(&mut self, key: String) {
        if self.has_spoken(&key) {
            return;
        }
        if self.spoken.len() >= SPOKEN_MEMORY {
            self.spoken.pop_front();
        }
        self.spoken.push_back(key);
    }

    fn forget(&mut self, key: &str) {
        self.spoken.retain(|known| known != key);
    }

    /// The queue is rebuilt from the newest view only: lines not yet started for an older
    /// scene are dropped, and a clip already playing is left to finish.
    fn refill(&mut self) {
        self.queue.clear();
        for line in &self.latest {
            let playing = self
                .current
                .as_ref()
                .is_some_and(|current| current.line == *line);
            if !playing && !self.has_spoken(&line.key()) {
                self.queue.push_back(line.clone());
            }
        }
    }

    fn plays(&self, url: &str) -> bool {
        self.current
            .as_ref()
            .and_then(|current| current.object_url.as_deref())
            .is_some_and(|expected| expected == url)
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
    let clip_playing = state
        .current
        .as_ref()
        .is_some_and(|current| current.object_url.is_some())
        && !state.audio.paused()
        && !state.audio.ended();
    Some(SoundState {
        enabled: state.sound() == Sound::On,
        speaking: clip_playing,
    })
}

/// Called once per newly applied view. Voice is best effort: a failure here marks the
/// display root and never affects rendering.
pub(super) fn observe(client: &Rc<RefCell<Client>>) {
    let (document, root, view) = {
        let state = client.borrow();
        if state.role != "display" {
            return;
        }
        let Some(view) = state
            .views
            .as_ref()
            .and_then(|views| views.current().cloned())
        else {
            return;
        };
        (state.document.clone(), state.root.clone(), view)
    };
    let Some(rpc::view_message::Audience::Display(display)) = view.audience else {
        return;
    };
    match director(&document, &root) {
        Ok(director) => {
            {
                let mut state = director.borrow_mut();
                state.latest = display_lines(&display);
                state.refill();
            }
            pump(&director);
        }
        Err(_) => {
            let _marked = root.set_attribute("data-voice", "unavailable");
        }
    }
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
        document: document.clone(),
        audio: audio.clone(),
        control: control.clone(),
        icon,
        label,
        unlocked: false,
        muted: stored_mute(),
        latest: Vec::new(),
        queue: VecDeque::new(),
        spoken: VecDeque::new(),
        current: None,
        play_id: 0,
        listeners: Vec::new(),
    }));
    shared.borrow().render_control();
    listen(&shared, audio.as_ref(), "playing", on_playing)?;
    listen(&shared, audio.as_ref(), "ended", on_clip_over)?;
    listen(&shared, audio.as_ref(), "error", on_clip_over)?;
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

/// Any click or key press outside the control unlocks audio; the control handles its own.
fn on_gesture(shared: &Shared, event: &Event) {
    let inside_control = event
        .target()
        .and_then(|target| target.dyn_into::<Node>().ok())
        .is_some_and(|target| shared.borrow().control.contains(Some(&target)));
    if !inside_control {
        unlock(shared);
    }
}

fn on_control(shared: &Shared, _event: &Event) {
    // Read the state first: a borrow held across the match would still be live when the
    // arms borrow mutably, which aborts the whole client.
    let sound = shared.borrow().sound();
    match sound {
        Sound::Locked => unlock(shared),
        Sound::On => set_muted(shared, true),
        Sound::Muted => set_muted(shared, false),
    }
}

fn unlock(shared: &Shared) {
    {
        let mut state = shared.borrow_mut();
        if state.unlocked {
            return;
        }
        state.unlocked = true;
        state.render_control();
        state.audio.set_src(UNLOCK_CLIP);
        // Playing inside the gesture is what unlocks the element; a browser that refuses
        // here is caught later by the real clip's own play call.
        if let Ok(promise) = state.audio.play() {
            spawn_local(async move {
                let _settled = JsFuture::from(promise).await;
            });
        }
        state.refill();
    }
    pump(shared);
}

fn set_muted(shared: &Shared, muted: bool) {
    let stopped = {
        let mut state = shared.borrow_mut();
        state.muted = muted;
        let _stored = store_mute(muted);
        state.play_id += 1;
        if muted {
            let _paused = state.audio.pause();
            state.queue.clear();
        } else {
            state.refill();
        }
        state.render_control();
        state.current.take()
    };
    if let Some(playing) = stopped {
        playing.release();
    }
    pump(shared);
}

/// Starts the next queued line when sound is on and nothing is playing.
fn pump(shared: &Shared) {
    let (line, id) = {
        let mut state = shared.borrow_mut();
        if state.sound() != Sound::On || state.current.is_some() {
            return;
        }
        let Some(line) = state.queue.pop_front() else {
            return;
        };
        // A started line counts as spoken even if its clip fails, so a refused line is
        // not requested again on every view.
        state.remember(line.key());
        state.play_id += 1;
        state.current = Some(Playing {
            line: line.clone(),
            speaking: None,
            object_url: None,
        });
        (line, state.play_id)
    };
    let shared = Rc::clone(shared);
    spawn_local(async move {
        match fetch_clip(&line.key()).await {
            Ok(url) => start(&shared, id, url),
            Err(_) => abandon(&shared, id),
        }
    });
}

fn start(shared: &Shared, id: u64, url: String) {
    let played = {
        let mut state = shared.borrow_mut();
        if state.play_id != id || state.current.is_none() {
            let _revoked = Url::revoke_object_url(&url);
            return;
        }
        state.audio.set_src(&url);
        if let Some(current) = state.current.as_mut() {
            current.object_url = Some(url);
        }
        state.audio.play()
    };
    match played {
        Ok(promise) => {
            let shared = Rc::clone(shared);
            spawn_local(async move {
                if let Err(refusal) = JsFuture::from(promise).await {
                    refused(&shared, id, &refusal);
                }
            });
        }
        Err(refusal) => refused(shared, id, &refusal),
    }
}

/// A play call the browser refused: if it was the autoplay policy the control comes back
/// and the line waits; any other refusal skips the line.
fn refused(shared: &Shared, id: u64, refusal: &JsValue) {
    let blocked = js_sys::Reflect::get(refusal, &JsValue::from_str("name"))
        .ok()
        .and_then(|name| name.as_string())
        .is_some_and(|name| name == "NotAllowedError");
    if !blocked {
        abandon(shared, id);
        return;
    }
    let released = {
        let mut state = shared.borrow_mut();
        if state.play_id != id {
            return;
        }
        state.unlocked = false;
        state.render_control();
        let released = state.current.take();
        if let Some(playing) = &released {
            state.forget(&playing.line.key());
        }
        state.refill();
        released
    };
    if let Some(playing) = released {
        playing.release();
    }
}

/// Gives up on the clip `id` (fetch failed, refused or undecodable) and moves on.
fn abandon(shared: &Shared, id: u64) {
    if shared.borrow().play_id != id {
        return;
    }
    finish(shared);
}

fn finish(shared: &Shared) {
    let over = shared.borrow_mut().current.take();
    if let Some(playing) = over {
        playing.release();
    }
    pump(shared);
}

fn on_playing(shared: &Shared, _event: &Event) {
    let mut state = shared.borrow_mut();
    let url = state.audio.current_src();
    if !state.plays(&url) {
        return;
    }
    let element = state
        .current
        .as_ref()
        .and_then(|current| locate(&state.document, &current.line));
    if let Some(element) = &element {
        let _marked = element.class_list().add_1(SPEAKING_CLASS);
    }
    if let Some(current) = state.current.as_mut() {
        current.speaking = element;
    }
}

/// `ended` or `error`, but only for the real clip, never for the unlock clip.
fn on_clip_over(shared: &Shared, _event: &Event) {
    let url = shared.borrow().audio.current_src();
    if shared.borrow().plays(&url) {
        finish(shared);
    }
}

/// The element showing `line`: an element carrying `data-voice-role` for the line's role
/// if the interface provides one, otherwise the known narration and quote elements, matched
/// by their text so a changed layout simply gets no mark.
fn locate(document: &Document, line: &VoiceLine) -> Option<Element> {
    let attribute = format!(
        "[data-voice-role=\"{}\"]",
        if line.is_npc() { "npc" } else { "narrator" }
    );
    let spoken = line.text.trim();
    [
        attribute.as_str(),
        ".tv-narration",
        ".narration",
        ".tv-quote",
        ".dialogue blockquote",
    ]
    .into_iter()
    .find_map(|selector| {
        let element = document.query_selector(selector).ok().flatten()?;
        element
            .text_content()
            .is_some_and(|text| text.contains(spoken))
            .then_some(element)
    })
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

/// The remembered mute choice of this tab. Unavailable storage means "not muted".
fn stored_mute() -> bool {
    web_sys::window()
        .and_then(|window| window.session_storage().ok().flatten())
        .and_then(|storage| storage.get_item(MUTED_KEY).ok().flatten())
        .is_some_and(|value| value == "1")
}

fn store_mute(muted: bool) -> Result<(), JsValue> {
    let storage = web_sys::window()
        .and_then(|window| window.session_storage().ok().flatten())
        .ok_or_else(|| JsValue::from_str("session storage unavailable"))?;
    storage.set_item(MUTED_KEY, if muted { "1" } else { "0" })
}

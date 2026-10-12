//! The hold-to-talk control next to the dialogue text box.
//!
//! The rules of the control live in [`crate::speech_capture`]; this module carries them out in
//! the browser. The microphone is requested only on a deliberate press, never on page load, and
//! its tracks are stopped as soon as the recording ends. The recording is decoded to 16 kHz mono
//! in the page, uploaded to the server, and the words that come back are only a draft in the
//! dialogue text box: the player edits it and sends it like any typed line.
//!
//! The control is built once and moved into the dialogue panel on every redraw, so a redraw
//! never drops a press in progress: the pointer-up, cancel and Escape listeners sit on the
//! window, not on the control.
use std::{cell::Cell, time::Duration};

use super::*;
use crate::speech_capture::{Capture, Command, Environment, MicrophoneFault, Notice, Phase};
use crate::speech_clip::{
    BINDING_HEADER, CLIP_CONTENT_TYPE, MAXIMUM_RECORDING, OUTCOME_HEADER, SAMPLE_RATE, SPEECH_PATH,
    STATUS_PATH, SpeechOutcome, encode_wav,
};
use gloo_timers::callback::{Interval, Timeout};
use js_sys::{ArrayBuffer, Uint8Array};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    AbortController, AudioBuffer, Blob, BlobEvent, EventTarget, Headers, KeyboardEvent,
    MediaRecorder, MediaStream, MediaStreamConstraints, MediaStreamTrack, OfflineAudioContext,
    PointerEvent, RecordingState, RequestCache, RequestCredentials, RequestInit, RequestMode,
    Response,
};

const STYLE: &str = include_str!("talk_control.css");
const TICK_MILLISECONDS: u32 = 200;
/// How far outside the control a pointer may drift before the press counts as sliding off.
const SLIDE_OFF_MARGIN: f64 = 24.0;
/// The server's own deadline is twelve seconds; the phone gives up a little after.
const UPLOAD_DEADLINE_MILLISECONDS: u32 = 20_000;
/// Decoded audio past the cap by more than this is cut, so a late stop never grows the clip.
const DECODED_SLACK: Duration = Duration::from_millis(500);
const PROBE_ATTEMPTS: u8 = 3;
const REVIEW_HINT: &str = "Check the words, change them if you like, then send.";
const RECORDING_HINT: &str = "Slide off the button or press Escape to cancel.";

type WeakClient = std::rc::Weak<RefCell<Client>>;

/// Whether the server offers speech input, learned once from its status route.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Availability {
    #[default]
    Unknown,
    Available,
    BudgetSpent,
    Unavailable,
}

/// What is holding the control down.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Held {
    Pointer(i32),
    Key,
}

struct Listener {
    target: EventTarget,
    name: &'static str,
    callback: Closure<dyn FnMut(web_sys::Event)>,
}

/// The microphone, the recorder and what it has captured. At most one exists.
struct Recording {
    stream: MediaStream,
    recorder: MediaRecorder,
    chunks: Rc<RefCell<Vec<Blob>>>,
    /// False once the recording was discarded: the recorder's stop event then does nothing.
    keep: Rc<Cell<bool>>,
    _on_data: Closure<dyn FnMut(BlobEvent)>,
    _on_error: Closure<dyn FnMut(web_sys::Event)>,
}

impl Recording {
    fn stop_tracks(&self) {
        stop_tracks(&self.stream);
    }

    fn detach_handlers(&self) {
        self.recorder.set_ondataavailable(None);
        self.recorder.set_onerror(None);
    }
}

struct Row {
    root: Element,
    hold: Element,
    label: Element,
    status: Element,
    actions: Element,
}

#[derive(Default)]
pub(super) struct Talk {
    capture: Capture,
    availability: Availability,
    probing: bool,
    probe_failures: u8,
    row: Option<Row>,
    listeners: Vec<Listener>,
    held: Option<Held>,
    recording: Option<Recording>,
    ticker: Option<Interval>,
    upload: Option<AbortHandle>,
}

impl Talk {
    /// Whether the talk control is being held right now (speech intent is live).
    pub(super) fn is_held(&self) -> bool {
        self.held.is_some()
    }

    /// Releases the microphone and every listener. Used when the page's client is dropped.
    fn dispose(&mut self) {
        self.ticker = None;
        if let Some(abort) = self.upload.take() {
            abort.abort();
        }
        if let Some(recording) = self.recording.take() {
            recording.keep.set(false);
            recording.detach_handlers();
            if recording.recorder.state() != RecordingState::Inactive {
                let _stopped = recording.recorder.stop();
            }
            recording.stop_tracks();
        }
        for listener in self.listeners.drain(..) {
            let _removed = listener.target.remove_event_listener_with_callback(
                listener.name,
                listener.callback.as_ref().unchecked_ref(),
            );
        }
    }
}

impl Drop for Talk {
    fn drop(&mut self) {
        self.dispose();
    }
}

fn now() -> Duration {
    web_sys::window()
        .and_then(|window| window.performance())
        .map(|performance| Duration::from_secs_f64(performance.now() / 1000.0))
        .unwrap_or_default()
}

fn environment() -> Environment {
    let window = web_sys::window();
    let secure_context = window
        .as_ref()
        .is_some_and(web_sys::Window::is_secure_context);
    let can_record = window.is_some_and(|window| {
        js_sys::Reflect::has(&window, &"MediaRecorder".into()).unwrap_or(false)
            && js_sys::Reflect::get(&window.navigator(), &"mediaDevices".into())
                .is_ok_and(|devices| devices.is_object())
    });
    Environment {
        secure_context,
        can_record,
    }
}

/// Puts the control in the dialogue panel, below the text box. Nothing is shown until the
/// server has said it offers speech input.
pub(super) fn mount(client: &Rc<RefCell<Client>>, panel: &Element) -> Result<(), JsValue> {
    if client.borrow().role != "player" {
        return Ok(());
    }
    if client.borrow().dialogue.talk.row.is_none() {
        build(client)?;
    }
    probe(client);
    let (root, offered) = {
        let state = client.borrow();
        let talk = &state.dialogue.talk;
        let root = talk.row.as_ref().map(|row| row.root.clone());
        let offered = matches!(
            talk.availability,
            Availability::Available | Availability::BudgetSpent
        );
        (root, offered)
    };
    if let (Some(root), true) = (root, offered) {
        panel.append_child(&root)?;
        refresh(client)?;
    }
    Ok(())
}

fn build(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let document = client.borrow().document.clone();
    if document
        .query_selector("style[data-talk-control]")?
        .is_none()
    {
        let style = document.create_element("style")?;
        style.set_attribute("data-talk-control", "")?;
        style.set_text_content(Some(STYLE));
        document
            .query_selector("head")?
            .ok_or_else(|| JsValue::from_str("document head absent"))?
            .append_child(&style)?;
    }
    let root = document.create_element("div")?;
    root.set_class_name("talk-row");
    let hold = node(&document, &root, "button", "talk-hold", "")?;
    hold.set_attribute("type", "button")?;
    hold.set_attribute("data-talk", "hold")?;
    hold.set_attribute("aria-keyshortcuts", "Space Enter")?;
    node(&document, &hold, "span", "talk-dot", "")?.set_attribute("aria-hidden", "true")?;
    let label = node(&document, &hold, "span", "talk-label", "Hold to talk")?;
    let status = node(&document, &root, "p", "talk-status", "")?;
    status.set_attribute("role", "status")?;
    let actions = node(&document, &root, "div", "talk-actions", "")?;
    client.borrow_mut().dialogue.talk.row = Some(Row {
        root,
        hold: hold.clone(),
        label,
        status,
        actions,
    });
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window absent"))?;
    let window_target: &EventTarget = window.as_ref();
    let document_target: &EventTarget = document.as_ref();
    let hold_target: &EventTarget = hold.as_ref();
    listen(client, hold_target, "pointerdown", pointer_down)?;
    listen(client, hold_target, "keydown", key_down)?;
    listen(client, hold_target, "keyup", key_up)?;
    listen(client, hold_target, "blur", |client, _| {
        lose_key_hold(client)
    })?;
    // A long press on a phone opens the context menu, which would cancel the press.
    listen(client, hold_target, "contextmenu", |_, event| {
        event.prevent_default();
    })?;
    listen(client, window_target, "pointerup", pointer_up)?;
    listen(client, window_target, "pointercancel", |client, event| {
        if is_held_pointer(&client, event) {
            cancel(&client);
        }
    })?;
    listen(client, window_target, "pointermove", pointer_move)?;
    listen(client, window_target, "keydown", |client, event| {
        if event
            .dyn_ref::<KeyboardEvent>()
            .is_some_and(|key| key.key() == "Escape")
        {
            cancel(&client);
        }
    })?;
    listen(client, document_target, "visibilitychange", |client, _| {
        let hidden = client.borrow().document.hidden();
        if hidden {
            cancel(&client);
        }
    })?;
    Ok(())
}

fn listen(
    client: &Rc<RefCell<Client>>,
    target: &EventTarget,
    name: &'static str,
    mut handler: impl FnMut(Rc<RefCell<Client>>, &web_sys::Event) + 'static,
) -> Result<(), JsValue> {
    let weak = Rc::downgrade(client);
    let callback = Closure::wrap(Box::new(move |event: web_sys::Event| {
        if let Some(client) = weak.upgrade() {
            handler(client, &event);
        }
    }) as Box<dyn FnMut(web_sys::Event)>);
    target.add_event_listener_with_callback(name, callback.as_ref().unchecked_ref())?;
    client.borrow_mut().dialogue.talk.listeners.push(Listener {
        target: target.clone(),
        name,
        callback,
    });
    Ok(())
}

/// Asks the server once whether speech input exists. The answer shows or hides the control;
/// typing never depends on it.
fn probe(client: &Rc<RefCell<Client>>) {
    {
        let mut state = client.borrow_mut();
        let talk = &mut state.dialogue.talk;
        if talk.probing
            || talk.availability != Availability::Unknown
            || talk.probe_failures >= PROBE_ATTEMPTS
        {
            return;
        }
        talk.probing = true;
    }
    let weak = Rc::downgrade(client);
    spawn_local(async move {
        let found = fetch_status().await;
        let Some(client) = weak.upgrade() else {
            return;
        };
        {
            let mut state = client.borrow_mut();
            let talk = &mut state.dialogue.talk;
            talk.probing = false;
            match found {
                Some(availability) => talk.availability = availability,
                None => talk.probe_failures += 1,
            }
        }
        if redraw_current(&client).is_err() {
            status(
                &client.borrow(),
                "Dialogue controls could not be displayed.",
            );
        }
    });
}

async fn fetch_status() -> Option<Availability> {
    let window = web_sys::window()?;
    let options = RequestInit::new();
    options.set_method("GET");
    options.set_mode(RequestMode::SameOrigin);
    options.set_credentials(RequestCredentials::Omit);
    options.set_cache(RequestCache::NoStore);
    let response: Response = JsFuture::from(window.fetch_with_str_and_init(STATUS_PATH, &options))
        .await
        .ok()?
        .dyn_into()
        .ok()?;
    if response.status() != 200 {
        return None;
    }
    let label = response.headers().get(OUTCOME_HEADER).ok()??;
    Some(match SpeechOutcome::from_label(&label)? {
        SpeechOutcome::Available => Availability::Available,
        SpeechOutcome::BudgetSpent => Availability::BudgetSpent,
        _ => Availability::Unavailable,
    })
}

fn pointer_down(client: Rc<RefCell<Client>>, event: &web_sys::Event) {
    let Some(pointer) = event.dyn_ref::<PointerEvent>() else {
        return;
    };
    if pointer.button() != 0 {
        return;
    }
    // No text selection, focus change or emulated mouse events from this press.
    event.prevent_default();
    press(&client, Held::Pointer(pointer.pointer_id()));
}

fn key_down(client: Rc<RefCell<Client>>, event: &web_sys::Event) {
    let Some(key) = event.dyn_ref::<KeyboardEvent>() else {
        return;
    };
    if !matches!(key.key().as_str(), " " | "Enter") {
        return;
    }
    // The click a held key would produce is not wanted either.
    event.prevent_default();
    if !key.repeat() {
        press(&client, Held::Key);
    }
}

fn key_up(client: Rc<RefCell<Client>>, event: &web_sys::Event) {
    let Some(key) = event.dyn_ref::<KeyboardEvent>() else {
        return;
    };
    if !matches!(key.key().as_str(), " " | "Enter") {
        return;
    }
    event.prevent_default();
    if client.borrow().dialogue.talk.held == Some(Held::Key) {
        release(&client, true);
    }
}

fn lose_key_hold(client: Rc<RefCell<Client>>) {
    if client.borrow().dialogue.talk.held == Some(Held::Key) {
        cancel(&client);
    }
}

fn is_held_pointer(client: &Rc<RefCell<Client>>, event: &web_sys::Event) -> bool {
    event.dyn_ref::<PointerEvent>().is_some_and(|pointer| {
        client.borrow().dialogue.talk.held == Some(Held::Pointer(pointer.pointer_id()))
    })
}

fn over_control(client: &Rc<RefCell<Client>>, event: &web_sys::Event) -> bool {
    let Some(pointer) = event.dyn_ref::<PointerEvent>() else {
        return false;
    };
    let state = client.borrow();
    let Some(row) = state.dialogue.talk.row.as_ref() else {
        return false;
    };
    let rect = row.hold.get_bounding_client_rect();
    let (x, y) = (f64::from(pointer.client_x()), f64::from(pointer.client_y()));
    x >= rect.left() - SLIDE_OFF_MARGIN
        && x <= rect.right() + SLIDE_OFF_MARGIN
        && y >= rect.top() - SLIDE_OFF_MARGIN
        && y <= rect.bottom() + SLIDE_OFF_MARGIN
}

fn pointer_up(client: Rc<RefCell<Client>>, event: &web_sys::Event) {
    if is_held_pointer(&client, event) {
        let inside = over_control(&client, event);
        release(&client, inside);
    }
}

fn pointer_move(client: Rc<RefCell<Client>>, event: &web_sys::Event) {
    if is_held_pointer(&client, event) && !over_control(&client, event) {
        cancel(&client);
    }
}

fn press(client: &Rc<RefCell<Client>>, held: Held) {
    let command = {
        let mut state = client.borrow_mut();
        let talk = &mut state.dialogue.talk;
        if talk.held.is_some() {
            return;
        }
        let command = talk.capture.press(environment());
        if command.is_some() {
            talk.held = Some(held);
        }
        command
    };
    let started = command.is_some();
    execute(client, command);
    show(client);
    if started {
        live_presence::signal(client, live_presence::Intent::Speaking);
    }
}

fn release(client: &Rc<RefCell<Client>>, inside: bool) {
    let command = {
        let mut state = client.borrow_mut();
        let talk = &mut state.dialogue.talk;
        talk.held = None;
        talk.capture.release(now(), inside)
    };
    execute(client, command);
    show(client);
    live_presence::signal(client, live_presence::Intent::Stopped);
}

fn cancel(client: &Rc<RefCell<Client>>) {
    let command = {
        let mut state = client.borrow_mut();
        let talk = &mut state.dialogue.talk;
        talk.held = None;
        talk.capture.cancel()
    };
    execute(client, command);
    show(client);
    live_presence::signal(client, live_presence::Intent::Stopped);
}

fn tick(client: &Rc<RefCell<Client>>) {
    let command = client.borrow_mut().dialogue.talk.capture.tick(now());
    execute(client, command);
    show(client);
}

fn execute(client: &Rc<RefCell<Client>>, command: Option<Command>) {
    match command {
        Some(Command::OpenMicrophone) => {
            let weak = Rc::downgrade(client);
            spawn_local(open_microphone(weak));
        }
        Some(Command::FinishRecording) => finish_recording(client),
        Some(Command::Discard) => discard_recording(client),
        Some(Command::AbortUpload) => {
            // The recorder may not have delivered its stop event yet; it must not upload.
            discard_recording(client);
            if let Some(abort) = client.borrow_mut().dialogue.talk.upload.take() {
                abort.abort();
            }
        }
        None => {}
    }
}

fn fault_of(error: &JsValue) -> MicrophoneFault {
    let name = js_sys::Reflect::get(error, &"name".into())
        .ok()
        .and_then(|name| name.as_string())
        .unwrap_or_default();
    match name.as_str() {
        "NotAllowedError" | "SecurityError" | "PermissionDeniedError" => {
            MicrophoneFault::PermissionRefused
        }
        "NotFoundError" | "DevicesNotFoundError" | "OverconstrainedError" => {
            MicrophoneFault::NoMicrophone
        }
        _ => MicrophoneFault::Other,
    }
}

fn stop_tracks(stream: &MediaStream) {
    let tracks = stream.get_tracks();
    for index in 0..tracks.length() {
        if let Ok(track) = tracks.get(index).dyn_into::<MediaStreamTrack>() {
            track.stop();
        }
    }
}

async fn open_microphone(weak: WeakClient) {
    let requested = async {
        let window = web_sys::window().ok_or_else(|| JsValue::from_str("window absent"))?;
        let devices = window.navigator().media_devices()?;
        let constraints = MediaStreamConstraints::new();
        constraints.set_audio_bool(true);
        let stream = JsFuture::from(devices.get_user_media_with_constraints(&constraints)?).await?;
        stream.dyn_into::<MediaStream>()
    }
    .await;
    let Some(client) = weak.upgrade() else {
        if let Ok(stream) = &requested {
            stop_tracks(stream);
        }
        return;
    };
    let stream = match requested {
        Ok(stream) => stream,
        Err(error) => {
            client
                .borrow_mut()
                .dialogue
                .talk
                .capture
                .microphone_failed(fault_of(&error));
            client.borrow_mut().dialogue.talk.held = None;
            show(&client);
            return;
        }
    };
    let command = client
        .borrow_mut()
        .dialogue
        .talk
        .capture
        .microphone_ready(now());
    if command.is_some() {
        // Nobody is holding the button any more; the microphone is not kept.
        stop_tracks(&stream);
        client.borrow_mut().dialogue.talk.held = None;
        show(&client);
        return;
    }
    match start_recorder(&client, stream.clone()) {
        Ok(recording) => {
            let weak = Rc::downgrade(&client);
            let mut state = client.borrow_mut();
            state.dialogue.talk.recording = Some(recording);
            state.dialogue.talk.ticker = Some(Interval::new(TICK_MILLISECONDS, move || {
                if let Some(client) = weak.upgrade() {
                    tick(&client);
                }
            }));
        }
        Err(_) => {
            stop_tracks(&stream);
            let mut state = client.borrow_mut();
            state.dialogue.talk.capture.recorder_failed();
            state.dialogue.talk.held = None;
        }
    }
    show(&client);
}

fn start_recorder(client: &Rc<RefCell<Client>>, stream: MediaStream) -> Result<Recording, JsValue> {
    let recorder = MediaRecorder::new_with_media_stream(&stream)?;
    let chunks = Rc::new(RefCell::new(Vec::new()));
    let keep = Rc::new(Cell::new(true));
    let collected = chunks.clone();
    let on_data = Closure::wrap(Box::new(move |event: BlobEvent| {
        if let Some(blob) = event.data()
            && blob.size() > 0.0
        {
            collected.borrow_mut().push(blob);
        }
    }) as Box<dyn FnMut(BlobEvent)>);
    recorder.set_ondataavailable(Some(on_data.as_ref().unchecked_ref()));
    let weak = Rc::downgrade(client);
    let on_error = Closure::wrap(Box::new(move |_: web_sys::Event| {
        if let Some(client) = weak.upgrade() {
            let command = client.borrow_mut().dialogue.talk.capture.recorder_failed();
            execute(&client, command);
            show(&client);
        }
    }) as Box<dyn FnMut(web_sys::Event)>);
    recorder.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    // The stop event fires exactly once, after the final data event; it hands the capture on.
    let weak = Rc::downgrade(client);
    let wanted = keep.clone();
    let on_stop = Closure::once_into_js(move || {
        if wanted.get()
            && let Some(client) = weak.upgrade()
        {
            recorded(&client);
        }
    });
    recorder.set_onstop(Some(on_stop.unchecked_ref()));
    recorder.start()?;
    Ok(Recording {
        stream,
        recorder,
        chunks,
        keep,
        _on_data: on_data,
        _on_error: on_error,
    })
}

/// Ends the recording; the recorder's stop event delivers what it captured.
fn finish_recording(client: &Rc<RefCell<Client>>) {
    let mut state = client.borrow_mut();
    let talk = &mut state.dialogue.talk;
    talk.ticker = None;
    let Some(recording) = talk.recording.as_ref() else {
        return;
    };
    if recording.recorder.stop().is_err() {
        talk.capture.recorder_failed();
        if let Some(recording) = talk.recording.take() {
            recording.keep.set(false);
            recording.detach_handlers();
            recording.stop_tracks();
        }
    }
}

/// Drops a recording unheard: the recorder stops, its data is dropped and the microphone is
/// released at once.
fn discard_recording(client: &Rc<RefCell<Client>>) {
    let mut state = client.borrow_mut();
    let talk = &mut state.dialogue.talk;
    talk.ticker = None;
    if let Some(recording) = talk.recording.take() {
        recording.keep.set(false);
        recording.detach_handlers();
        if recording.recorder.state() != RecordingState::Inactive {
            let _stopped = recording.recorder.stop();
        }
        recording.stop_tracks();
    }
}

/// The recorder has stopped: release the microphone, then decode and upload the capture.
fn recorded(client: &Rc<RefCell<Client>>) {
    let blob = {
        let mut state = client.borrow_mut();
        let Some(recording) = state.dialogue.talk.recording.take() else {
            return;
        };
        recording.detach_handlers();
        recording.stop_tracks();
        let parts = js_sys::Array::new();
        for chunk in recording.chunks.borrow().iter() {
            parts.push(chunk);
        }
        Blob::new_with_blob_sequence(&parts)
    };
    let (abort, registration) = AbortHandle::new_pair();
    let Ok(blob) = blob else {
        client.borrow_mut().dialogue.talk.capture.unreadable();
        show(client);
        return;
    };
    client.borrow_mut().dialogue.talk.upload = Some(abort);
    let weak = Rc::downgrade(client);
    spawn_local(async move {
        let _finished = Abortable::new(hear(weak, blob), registration).await;
    });
}

/// Mono samples at the clip sample rate from the recording, however the browser encoded it.
async fn decode(blob: &Blob) -> Result<Vec<f32>, JsValue> {
    let encoded: ArrayBuffer = JsFuture::from(blob.array_buffer()).await?.dyn_into()?;
    let context = OfflineAudioContext::new_with_number_of_channels_and_length_and_sample_rate(
        1,
        SAMPLE_RATE,
        SAMPLE_RATE as f32,
    )?;
    let audio: AudioBuffer = JsFuture::from(context.decode_audio_data(&encoded)?)
        .await?
        .dyn_into()?;
    let channels = audio.number_of_channels();
    let mut samples = vec![0.0f32; audio.length() as usize];
    for channel in 0..channels {
        for (sample, value) in samples.iter_mut().zip(audio.get_channel_data(channel)?) {
            *sample += value / channels as f32;
        }
    }
    let longest =
        ((MAXIMUM_RECORDING + DECODED_SLACK).as_secs_f64() * f64::from(SAMPLE_RATE)) as usize;
    samples.truncate(longest);
    Ok(samples)
}

async fn hear(weak: WeakClient, blob: Blob) {
    let decoded = decode(&blob).await;
    drop(blob);
    let Some(client) = weak.upgrade() else {
        return;
    };
    let samples = match decoded {
        Ok(samples) => samples,
        Err(_) => {
            client.borrow_mut().dialogue.talk.capture.unreadable();
            show(&client);
            return;
        }
    };
    let clip = encode_wav(&samples);
    let wanted = client
        .borrow_mut()
        .dialogue
        .talk
        .capture
        .decoded(samples.len(), clip.len());
    drop(samples);
    if !wanted {
        show(&client);
        return;
    }
    let credential = client.borrow().credential.clone();
    let answer = upload(&credential, &clip).await;
    drop(clip);
    let Some(client) = weak.upgrade() else {
        return;
    };
    let (outcome, words) = match answer {
        Ok(answer) => answer,
        Err(_) => {
            client.borrow_mut().dialogue.talk.capture.upload_failed();
            show(&client);
            return;
        }
    };
    let heard = {
        let mut state = client.borrow_mut();
        state.dialogue.talk.upload = None;
        let heard = state.dialogue.talk.capture.heard(outcome, &words);
        if heard && let Some(editor) = state.dialogue.editor() {
            editor.set_value(&words);
        }
        heard
    };
    // The words sit in the text box now; this copy is dropped with the call.
    drop(words);
    show(&client);
    if heard {
        focus_editor(&client);
    }
}

fn focus_editor(client: &Rc<RefCell<Client>>) {
    let editor = client.borrow().dialogue.editor().cloned();
    if let Some(editor) = editor {
        let _focused = editor.focus();
    }
}

/// Sends the clip and returns the server's outcome label and the transcript text.
async fn upload(credential: &str, clip: &[u8]) -> Result<(SpeechOutcome, String), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window absent"))?;
    let controller = AbortController::new()?;
    let aborting = controller.clone();
    let _deadline = Timeout::new(UPLOAD_DEADLINE_MILLISECONDS, move || aborting.abort());
    let headers = Headers::new()?;
    headers.set("content-type", CLIP_CONTENT_TYPE)?;
    headers.set(BINDING_HEADER, credential)?;
    let options = RequestInit::new();
    options.set_method("POST");
    options.set_mode(RequestMode::SameOrigin);
    options.set_credentials(RequestCredentials::Omit);
    options.set_cache(RequestCache::NoStore);
    options.set_headers(&headers);
    options.set_signal(Some(&controller.signal()));
    options.set_body(&Uint8Array::from(clip));
    let response: Response = JsFuture::from(window.fetch_with_str_and_init(SPEECH_PATH, &options))
        .await?
        .dyn_into()?;
    let label = response
        .headers()
        .get(OUTCOME_HEADER)?
        .ok_or_else(|| JsValue::from_str("speech outcome absent"))?;
    let outcome = SpeechOutcome::from_label(&label)
        .ok_or_else(|| JsValue::from_str("speech outcome unknown"))?;
    let words = JsFuture::from(response.text()?)
        .await?
        .as_string()
        .unwrap_or_default();
    Ok((outcome, words))
}

/// Brings the control's text, state and buttons in line with the machine.
fn show(client: &Rc<RefCell<Client>>) {
    if refresh(client).is_err() {
        status(
            &client.borrow(),
            "Dialogue controls could not be displayed.",
        );
    }
}

fn refresh(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let (row, phase, notice, availability, elapsed) = {
        let state = client.borrow();
        let talk = &state.dialogue.talk;
        let Some(row) = talk.row.as_ref() else {
            return Ok(());
        };
        (
            (
                row.root.clone(),
                row.hold.clone(),
                row.label.clone(),
                row.status.clone(),
                row.actions.clone(),
            ),
            talk.capture.phase(),
            talk.capture.notice(),
            talk.availability,
            talk.capture.elapsed_seconds(now()),
        )
    };
    let (root, hold, label, status_line, actions) = row;
    let name = match phase {
        Phase::Idle => "idle",
        Phase::Requesting(_) => "requesting",
        Phase::Recording { .. } => "recording",
        Phase::Listening => "listening",
        Phase::Review => "review",
    };
    root.set_attribute("data-state", name)?;
    label.set_text_content(Some(&match phase {
        Phase::Requesting(_) => "Starting…".to_owned(),
        Phase::Recording { .. } => {
            format!(
                "Recording {}s · release to send",
                elapsed.unwrap_or_default()
            )
        }
        Phase::Listening => "Listening…".to_owned(),
        Phase::Idle | Phase::Review => "Hold to talk".to_owned(),
    }));
    hold.set_attribute(
        "aria-pressed",
        if matches!(phase, Phase::Recording { .. }) {
            "true"
        } else {
            "false"
        },
    )?;
    let disabled = phase == Phase::Listening || availability == Availability::BudgetSpent;
    if disabled {
        hold.set_attribute("disabled", "")?;
    } else {
        hold.remove_attribute("disabled")?;
    }
    status_line.set_text_content(Some(match (phase, notice, availability) {
        (_, Some(notice), _) => notice.message(),
        (Phase::Recording { .. }, ..) => RECORDING_HINT,
        (Phase::Review, ..) => REVIEW_HINT,
        (Phase::Idle, None, Availability::BudgetSpent) => Notice::BudgetSpent.message(),
        _ => "",
    }));
    actions.set_text_content(None);
    match phase {
        Phase::Review => {
            button(client, &actions, "Send", send)?.set_attribute("data-talk", "send")?;
            button(client, &actions, "Clear", clear)?.set_attribute("data-talk", "clear")?;
        }
        Phase::Listening => {
            button(client, &actions, "Cancel", |client| cancel(&client))?
                .set_attribute("data-talk", "cancel")?;
        }
        _ => {}
    }
    Ok(())
}

/// Sends the draft exactly as if it had been typed.
fn send(client: Rc<RefCell<Client>>) {
    client.borrow_mut().dialogue.talk.capture.settle_draft();
    show(&client);
    dialogue_recovery::submit(client);
}

fn clear(client: Rc<RefCell<Client>>) {
    let editor = client.borrow().dialogue.editor().cloned();
    if let Some(editor) = editor {
        editor.set_value("");
    }
    client.borrow_mut().dialogue.talk.capture.settle_draft();
    show(&client);
    live_presence::signal(&client, live_presence::Intent::Stopped);
}

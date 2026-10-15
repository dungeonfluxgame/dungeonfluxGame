//! The tap-to-talk control next to the dialogue text box.
//!
//! The rules of the control live in [`crate::speech_capture`]; this module carries them out in
//! the browser. The microphone is requested only on a deliberate tap, never on page load, and
//! its tracks are stopped as soon as the recording ends. The recording is decoded to 16 kHz mono
//! in the page, uploaded to the server, and the words that come back are only a draft in the
//! dialogue text box: the player edits it and sends it like any typed line.
//!
//! The control is built once and moved into the dialogue panel on every redraw, so a redraw
//! never drops a press in progress: the pointer-up, cancel and Escape listeners sit on the
//! window, not on the control.
use std::{cell::Cell, time::Duration};

use super::*;
use crate::speech_capture::{
    Capture, Command, Environment, Interruption, MicrophoneFault, Notice, Phase,
};
use crate::speech_clip::{
    BINDING_HEADER, CANCEL_PATH, CLIP_CONTENT_TYPE, MAXIMUM_RECORDING, OUTCOME_HEADER, SAMPLE_RATE,
    SPEECH_PATH, STATUS_PATH, SpeechOutcome, UPLOAD_ID_HEADER, encode_wav,
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
const RECORDING_HINT: &str = "Tap again when you are done, or press Escape to cancel.";

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

/// What is pressing the control down.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Held {
    Pointer(i32),
    Key,
}

struct UploadCancellation {
    task: AbortHandle,
    fetch: AbortController,
    identity: UploadIdentity,
}

impl UploadCancellation {
    fn abort(self) {
        self.abort_with(|request| {
            spawn_local(async move {
                // The browser fetch is already aborted; notifying the server is best effort if
                // the page is closing or the connection is gone.
                let _sent = send_cancel(request).await;
            });
        });
    }

    fn abort_with(self, send_cancel: impl FnOnce(CancelRequest)) {
        self.fetch.abort();
        self.task.abort();
        send_cancel(self.identity.cancel_request());
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct UploadIdentity {
    credential: String,
    upload_id: String,
}

impl UploadIdentity {
    fn speech_headers(&self) -> Result<Headers, JsValue> {
        let headers = Headers::new()?;
        headers.set("content-type", CLIP_CONTENT_TYPE)?;
        headers.set(BINDING_HEADER, &self.credential)?;
        headers.set(UPLOAD_ID_HEADER, &self.upload_id)?;
        Ok(headers)
    }

    fn cancel_request(&self) -> CancelRequest {
        CancelRequest {
            path: CANCEL_PATH,
            method: "POST",
            credential: self.credential.clone(),
            upload_id: self.upload_id.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CancelRequest {
    path: &'static str,
    method: &'static str,
    credential: String,
    upload_id: String,
}

impl CancelRequest {
    fn headers(&self) -> Result<Headers, JsValue> {
        let headers = Headers::new()?;
        headers.set(BINDING_HEADER, &self.credential)?;
        headers.set(UPLOAD_ID_HEADER, &self.upload_id)?;
        Ok(headers)
    }
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
    clear: Element,
    cancel: Element,
}

#[derive(Default)]
pub(super) struct Talk {
    capture: Capture,
    capture_revision: Option<rpc::SessionRevision>,
    transcript_revision: Option<rpc::SessionRevision>,
    availability: Availability,
    probing: bool,
    probe_failures: u8,
    row: Option<Row>,
    listeners: Vec<Listener>,
    held: Option<Held>,
    recording: Option<Recording>,
    ticker: Option<Interval>,
    upload: Option<UploadCancellation>,
}

impl Talk {
    pub(super) fn is_busy(&self) -> bool {
        self.capture.is_busy()
    }

    /// Whether the control is listening for the player's words, which keeps the speech-intent
    /// mark live.
    pub(super) fn is_listening(&self) -> bool {
        self.capture.is_listening()
    }

    /// Ends listening at once for an interruption that needs no signal to the table: the text box
    /// went away, the connection was reset, or the player typed. The redraw that follows shows it.
    pub(super) fn interrupt_now(&mut self, why: Interruption) {
        self.held = None;
        if let Some(command) = self.capture.interrupt(why) {
            self.carry_out(command);
        }
    }

    fn carry_out(&mut self, command: Command) {
        match command {
            Command::Discard => self.discard(),
            Command::AbortUpload => {
                // The recorder may not have delivered its stop event yet; it must not upload.
                self.discard();
                if let Some(upload) = self.upload.take() {
                    upload.abort();
                }
            }
            // Only a tap opens or finishes a recording; an interruption never asks for either.
            Command::OpenMicrophone | Command::FinishRecording => {}
        }
    }

    /// Drops a recording unheard: the recorder stops, its data is dropped and the microphone is
    /// released at once.
    fn discard(&mut self) {
        self.ticker = None;
        if let Some(recording) = self.recording.take() {
            recording.keep.set(false);
            recording.detach_handlers();
            if recording.recorder.state() != RecordingState::Inactive {
                let _stopped = recording.recorder.stop();
            }
            recording.stop_tracks();
        }
    }

    /// Releases the microphone and every listener. Used when the page's client is dropped.
    fn dispose(&mut self) {
        self.ticker = None;
        if let Some(upload) = self.upload.take() {
            upload.abort();
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
    ensure_current_scope(client);
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
    let label = node(&document, &hold, "span", "talk-label", "Tap to talk")?;
    let status = node(&document, &root, "p", "talk-status", "")?;
    status.set_attribute("role", "status")?;
    let actions = node(&document, &root, "div", "talk-actions", "")?;
    let clear_button = node(&document, &actions, "button", "", "Clear")?;
    clear_button.set_attribute("type", "button")?;
    listen(client, clear_button.as_ref(), "click", |client, _| {
        clear(client)
    })?;
    clear_button.set_attribute("data-talk", "clear")?;
    let cancel_button = node(&document, &actions, "button", "", "Cancel")?;
    cancel_button.set_attribute("type", "button")?;
    listen(client, cancel_button.as_ref(), "click", |client, _| {
        interrupt(&client, Interruption::Cancelled)
    })?;
    cancel_button.set_attribute("data-talk", "cancel")?;
    client.borrow_mut().dialogue.talk.row = Some(Row {
        root,
        hold: hold.clone(),
        label,
        status,
        actions,
        clear: clear_button,
        cancel: cancel_button,
    });
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window absent"))?;
    let window_target: &EventTarget = window.as_ref();
    let document_target: &EventTarget = document.as_ref();
    let hold_target: &EventTarget = hold.as_ref();
    listen(client, hold_target, "pointerdown", pointer_down)?;
    listen(client, hold_target, "keydown", key_down)?;
    listen(client, hold_target, "keyup", key_up)?;
    listen(client, hold_target, "blur", |client, _| {
        lose_key_press(client)
    })?;
    // A long press on a phone opens the context menu, which would cancel the press.
    listen(client, hold_target, "contextmenu", |_, event| {
        event.prevent_default();
    })?;
    listen(client, window_target, "pointerup", pointer_up)?;
    listen(client, window_target, "pointercancel", |client, event| {
        if is_held_pointer(&client, event) {
            abandon(&client);
        }
    })?;
    listen(client, window_target, "pointermove", pointer_move)?;
    listen(client, window_target, "keydown", |client, event| {
        if event
            .dyn_ref::<KeyboardEvent>()
            .is_some_and(|key| key.key() == "Escape")
        {
            interrupt(&client, Interruption::Cancelled);
        }
    })?;
    listen(client, document_target, "visibilitychange", |client, _| {
        let hidden = client.borrow().document.hidden();
        if hidden {
            interrupt(&client, Interruption::PageHidden);
        }
    })?;
    let editor = client.borrow().dialogue.editor().cloned();
    if let Some(editor) = editor {
        let editor_target: &EventTarget = editor.as_ref();
        listen(client, editor_target, "input", |client, _| typed(&client))?;
    }
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
    press_down(&client, Held::Pointer(pointer.pointer_id()));
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
        press_down(&client, Held::Key);
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

fn lose_key_press(client: Rc<RefCell<Client>>) {
    if client.borrow().dialogue.talk.held == Some(Held::Key) {
        abandon(&client);
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
        abandon(&client);
    }
}

/// Starts a press on the control. Nothing starts until the press is released over the control.
fn press_down(client: &Rc<RefCell<Client>>, held: Held) {
    let mut state = client.borrow_mut();
    let talk = &mut state.dialogue.talk;
    if talk.held.is_some() {
        return;
    }
    talk.held = Some(held);
    talk.capture.press_down();
}

/// The press ended. Released over the control, it is a tap: it starts or stops listening.
fn release(client: &Rc<RefCell<Client>>, inside: bool) {
    if !ensure_current_scope(client) {
        return;
    }
    step(client, |talk| {
        talk.held = None;
        talk.capture.release(now(), inside, environment())
    });
}

/// The press stopped being a tap: the pointer was cancelled or dragged off the control, or the
/// key lost focus. Listening that is already under way goes on.
fn abandon(client: &Rc<RefCell<Client>>) {
    let mut state = client.borrow_mut();
    state.dialogue.talk.held = None;
    state.dialogue.talk.capture.abandon_press();
}

/// Ends listening for a reason other than a tap, such as Escape or a hidden page.
fn interrupt(client: &Rc<RefCell<Client>>, why: Interruption) {
    step(client, |talk| {
        talk.held = None;
        talk.capture.interrupt(why)
    });
}

/// Typing in the text box ends listening. The draft's own signal already tells the table, so
/// this stops the control without sending a stop of its own.
fn typed(client: &Rc<RefCell<Client>>) {
    client.borrow_mut().dialogue.talk.transcript_revision = None;
    if client.borrow().dialogue.talk.capture.is_busy() {
        client
            .borrow_mut()
            .dialogue
            .talk
            .interrupt_now(Interruption::Typing);
        show(client);
    }
}

/// Runs one step of the control, carries out its command, shows the new state, and tells the
/// table when listening starts or stops.
fn step(client: &Rc<RefCell<Client>>, operation: impl FnOnce(&mut Talk) -> Option<Command>) {
    let (command, before, after) = {
        let mut state = client.borrow_mut();
        let talk = &mut state.dialogue.talk;
        let before = talk.capture.is_listening();
        let command = operation(talk);
        (command, before, talk.capture.is_listening())
    };
    execute(client, command);
    show(client);
    if before != after {
        let intent = if after {
            live_presence::Intent::Speaking
        } else {
            live_presence::Intent::Stopped
        };
        live_presence::signal(client, intent);
    }
}

fn tick(client: &Rc<RefCell<Client>>) {
    if !ensure_current_scope(client) {
        return;
    }
    let attached = client
        .borrow()
        .dialogue
        .editor()
        .is_some_and(|editor| editor.is_connected());
    if attached {
        step(client, |talk| talk.capture.tick(now()));
    } else {
        // The scene took the text box away, so there is nothing left to talk into.
        interrupt(client, Interruption::SceneChanged);
    }
}

fn scope_matches(
    capture_revision: Option<rpc::SessionRevision>,
    current_revision: Option<rpc::SessionRevision>,
) -> bool {
    matches!((capture_revision, current_revision), (Some(capture), Some(current)) if capture == current)
}

/// Cancels recordings and drops their transcript when the player moves to another view.
fn ensure_current_scope(client: &Rc<RefCell<Client>>) -> bool {
    let (active, capture_revision, transcript_revision, current_revision) = {
        let state = client.borrow();
        (
            state.dialogue.talk.capture.is_busy(),
            state.dialogue.talk.capture_revision,
            state.dialogue.talk.transcript_revision,
            state.revision,
        )
    };
    let capture_stale = active && !scope_matches(capture_revision, current_revision);
    let transcript_stale = transcript_revision
        .is_some_and(|revision| !scope_matches(Some(revision), current_revision));
    if capture_stale {
        interrupt(client, Interruption::SceneChanged);
    }
    if transcript_stale {
        let mut state = client.borrow_mut();
        if let Some(editor) = state.dialogue.editor().cloned() {
            editor.set_value("");
        }
        state.dialogue.talk.transcript_revision = None;
    }
    !capture_stale && !transcript_stale
}

fn execute(client: &Rc<RefCell<Client>>, command: Option<Command>) {
    match command {
        Some(Command::OpenMicrophone) => {
            let mut state = client.borrow_mut();
            state.dialogue.talk.capture_revision = state.revision;
            drop(state);
            let weak = Rc::downgrade(client);
            spawn_local(open_microphone(weak));
        }
        Some(Command::FinishRecording) => finish_recording(client),
        Some(command @ (Command::Discard | Command::AbortUpload)) => {
            client.borrow_mut().dialogue.talk.carry_out(command);
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
    if !ensure_current_scope(&client) {
        let command = client
            .borrow_mut()
            .dialogue
            .talk
            .capture
            .microphone_ready(now());
        execute(&client, command);
        if let Ok(stream) = &requested {
            stop_tracks(stream);
        }
        show(&client);
        return;
    }
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
        // The start was cancelled while the microphone opened, so the microphone is not kept.
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

/// The recorder has stopped: release the microphone, then decode and upload the capture.
fn recorded(client: &Rc<RefCell<Client>>) {
    if !ensure_current_scope(client) {
        return;
    }
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
    let Ok(blob) = blob else {
        client.borrow_mut().dialogue.talk.capture.unreadable();
        show(client);
        return;
    };
    let controller = match AbortController::new() {
        Ok(controller) => controller,
        Err(_) => {
            client.borrow_mut().dialogue.talk.capture.upload_failed();
            show(client);
            return;
        }
    };
    let upload_id = match new_upload_id() {
        Ok(upload_id) => upload_id,
        Err(_) => {
            client.borrow_mut().dialogue.talk.capture.upload_failed();
            show(client);
            return;
        }
    };
    let identity = UploadIdentity {
        credential: client.borrow().credential.clone(),
        upload_id,
    };
    let (task, registration) = AbortHandle::new_pair();
    client.borrow_mut().dialogue.talk.upload = Some(UploadCancellation {
        task,
        fetch: controller.clone(),
        identity: identity.clone(),
    });
    let weak = Rc::downgrade(client);
    spawn_local(async move {
        let _finished = Abortable::new(hear(weak, blob, controller, identity), registration).await;
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

async fn hear(weak: WeakClient, blob: Blob, controller: AbortController, identity: UploadIdentity) {
    let decoded = decode(&blob).await;
    drop(blob);
    let Some(client) = weak.upgrade() else {
        return;
    };
    if !ensure_current_scope(&client) {
        return;
    }
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
    let answer = upload(&identity, &clip, &controller).await;
    drop(clip);
    let Some(client) = weak.upgrade() else {
        return;
    };
    if !ensure_current_scope(&client) {
        return;
    }
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
        let editor = heard.then(|| state.dialogue.editor().cloned()).flatten();
        if let Some(editor) = editor {
            editor.set_value(&words);
            state.dialogue.talk.transcript_revision = state.dialogue.talk.capture_revision;
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
async fn upload(
    identity: &UploadIdentity,
    clip: &[u8],
    controller: &AbortController,
) -> Result<(SpeechOutcome, String), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window absent"))?;
    let aborting = controller.clone();
    let _deadline = Timeout::new(UPLOAD_DEADLINE_MILLISECONDS, move || aborting.abort());
    let headers = identity.speech_headers()?;
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

/// Notifies the server independently of the aborted speech request.
async fn send_cancel(request: CancelRequest) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window absent"))?;
    let headers = request.headers()?;
    let options = RequestInit::new();
    options.set_method(request.method);
    options.set_mode(RequestMode::SameOrigin);
    options.set_credentials(RequestCredentials::Omit);
    options.set_cache(RequestCache::NoStore);
    options.set_headers(&headers);
    let _response = JsFuture::from(window.fetch_with_str_and_init(request.path, &options)).await?;
    Ok(())
}

fn new_upload_id() -> Result<String, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window absent"))?;
    let crypto = window.crypto()?;
    let mut bytes = [0_u8; 16];
    crypto.get_random_values_with_u8_array(&mut bytes)?;
    let digits = b"0123456789abcdef";
    let mut upload_id = String::with_capacity(32);
    for byte in bytes {
        upload_id.push(char::from(digits[usize::from(byte >> 4)]));
        upload_id.push(char::from(digits[usize::from(byte & 0x0f)]));
    }
    Ok(upload_id)
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
                row.clear.clone(),
                row.cancel.clone(),
            ),
            talk.capture.phase(),
            talk.capture.notice(),
            talk.availability,
            talk.capture.elapsed_seconds(now()),
        )
    };
    let (root, hold, label, status_line, actions, clear_button, cancel_button) = row;
    let name = match phase {
        Phase::Idle => "idle",
        Phase::Requesting(_) => "requesting",
        Phase::Recording { .. } => "recording",
        Phase::Listening => "listening",
        Phase::Review => "review",
    };
    root.set_attribute("data-state", name)?;
    if let Some(panel) = root.closest(".dialogue-recovery")?
        && let Some(send) = panel.query_selector("[data-dialogue=submit]")?
    {
        if dialogue_recovery::submit_disabled(&client.borrow()) {
            send.set_attribute("disabled", "")?;
        } else {
            send.remove_attribute("disabled")?;
        }
    }
    label.set_text_content(Some(&match phase {
        Phase::Requesting(_) => "Starting…".to_owned(),
        Phase::Recording { .. } => {
            format!("Listening… tap to stop · {}s", elapsed.unwrap_or_default())
        }
        Phase::Listening => "Listening…".to_owned(),
        Phase::Idle | Phase::Review => "Tap to talk".to_owned(),
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
    let review = phase == Phase::Review;
    let listening = phase == Phase::Listening;
    for (button, visible) in [(&clear_button, review), (&cancel_button, listening)] {
        if visible {
            button.remove_attribute("hidden")?;
        } else {
            button.set_attribute("hidden", "")?;
        }
    }
    if review || listening {
        actions.remove_attribute("hidden")?;
    } else {
        actions.set_attribute("hidden", "")?;
    }
    Ok(())
}

/// Sends the draft exactly as if it had been typed.
pub(super) fn send(client: Rc<RefCell<Client>>) {
    if client.borrow().dialogue.talk.capture.is_busy() {
        feedback(
            &client.borrow(),
            "Finish recording and review the words before sending.",
        );
        return;
    }
    if !ensure_current_scope(&client) {
        return;
    }
    client.borrow_mut().dialogue.talk.capture.settle_draft();
    show(&client);
    dialogue_recovery::submit(client);
}

fn clear(client: Rc<RefCell<Client>>) {
    let editor = client.borrow().dialogue.editor().cloned();
    if let Some(editor) = editor {
        editor.set_value("");
        if dialogue_recovery::update_typed_draft(&editor).is_err() {
            status(
                &client.borrow(),
                "Your draft controls could not be updated.",
            );
        }
    }
    client.borrow_mut().dialogue.talk.capture.settle_draft();
    client.borrow_mut().dialogue.talk.transcript_revision = None;
    show(&client);
    live_presence::signal(&client, live_presence::Intent::Stopped);
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_shared_send_does_not_submit_an_old_draft_while_speech_is_being_captured() {
        use super::*;
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("section").unwrap();
        let client = new_client(&document, root.clone(), "player").unwrap();
        let editor = node(&document, &root, "textarea", "dialogue-draft", "")
            .unwrap()
            .dyn_into::<web_sys::HtmlTextAreaElement>()
            .unwrap();
        editor.set_value("An earlier draft");
        client.borrow_mut().dialogue.set_test_editor(editor.clone());
        {
            let mut state = client.borrow_mut();
            state.dialogue.talk.capture.press_down();
            assert_eq!(
                state.dialogue.talk.capture.release(
                    Duration::ZERO,
                    true,
                    Environment {
                        secure_context: true,
                        can_record: true
                    },
                ),
                Some(Command::OpenMicrophone)
            );
        }
        send(client.clone());
        assert_eq!(editor.value(), "An earlier draft");
        assert!(!client.borrow().busy);
        assert!(
            root.query_selector(".feedback")
                .unwrap()
                .unwrap()
                .text_content()
                .unwrap()
                .contains("Finish recording")
        );
    }

    #[test]
    fn speech_review_has_one_send_action_and_clear_survives_a_page_redraw() {
        use super::*;
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("section").unwrap();
        let panel = node(&document, &root, "section", "dialogue-recovery", "").unwrap();
        let client = new_client(&document, root, "player").unwrap();
        let editor = node(&document, &panel, "textarea", "dialogue-draft", "")
            .unwrap()
            .dyn_into::<web_sys::HtmlTextAreaElement>()
            .unwrap();
        client.borrow_mut().dialogue.set_test_editor(editor.clone());
        client.borrow_mut().dialogue.talk.availability = Availability::Available;
        mount(&client, &panel).unwrap();
        node(&document, &panel, "button", "", "Say it").unwrap();
        let buttons = panel.query_selector_all("button").unwrap();
        let sends = (0..buttons.length())
            .filter(|index| {
                buttons.item(*index).unwrap().text_content().as_deref() == Some("Say it")
            })
            .count();
        assert_eq!(sends, 1);
        editor.set_value("Words to clear");
        panel.set_attribute("data-typed", "").unwrap();
        clear_callbacks(&mut client.borrow_mut());
        mount(&client, &panel).unwrap();
        panel
            .query_selector("[data-talk=clear]")
            .unwrap()
            .unwrap()
            .dispatch_event(&web_sys::Event::new("click").unwrap())
            .unwrap();
        assert_eq!(editor.value(), "");
        assert!(!panel.has_attribute("data-typed"));
    }

    use super::scope_matches;
    #[cfg(target_arch = "wasm32")]
    use super::{CancelRequest, UploadIdentity};
    use crate::gameplay_browser::rpc;

    fn revision(sequence: u64) -> rpc::SessionRevision {
        rpc::SessionRevision {
            epoch: Some(rpc::RecoveryEpoch { value: Some(1) }),
            sequence: Some(sequence),
        }
    }

    #[test]
    fn capture_scope_matches_only_its_originating_revision() {
        let origin = revision(4);

        assert!(scope_matches(Some(origin), Some(origin)));
        assert!(!scope_matches(Some(origin), Some(revision(5))));
        assert!(!scope_matches(Some(origin), None));
        assert!(!scope_matches(None, Some(origin)));
    }

    #[cfg(target_arch = "wasm32")]
    #[test]
    fn upload_ids_are_fresh_128_bit_lowercase_hex_values() {
        let first = super::new_upload_id().expect("browser crypto should provide an upload id");
        let second = super::new_upload_id().expect("browser crypto should provide an upload id");

        assert_eq!(first.len(), 32);
        assert_eq!(second.len(), 32);
        assert!(first.bytes().all(is_lowercase_hex));
        assert!(second.bytes().all(is_lowercase_hex));
        assert_ne!(first, second);
    }

    #[cfg(target_arch = "wasm32")]
    #[test]
    fn cancelling_observed_upload_aborts_it_and_posts_one_matching_cancel() {
        use super::{SPEECH_PATH, UploadCancellation};
        use web_sys::AbortController;

        let identity = UploadIdentity {
            credential: "member-binding".to_owned(),
            upload_id: "0123456789abcdef0123456789abcdef".to_owned(),
        };
        let controller = AbortController::new().expect("browser should support abort controllers");
        let signal = controller.signal();
        let mut endpoint = FakeSpeechEndpoint::default();
        endpoint.observe_speech_post(
            SPEECH_PATH,
            "POST",
            &identity
                .speech_headers()
                .expect("speech headers should be valid"),
            signal.clone(),
        );
        let (task, _registration) = futures::future::AbortHandle::new_pair();

        UploadCancellation {
            task,
            fetch: controller,
            identity: identity.clone(),
        }
        .abort_with(|request| endpoint.receive_cancel_post(request));

        assert!(signal.aborted());
        assert_eq!(endpoint.cancel_posts.len(), 1);
        assert_eq!(endpoint.cancel_posts[0], identity.cancel_request());
        assert!(!endpoint.uploads[&identity.upload_id].active);
        assert!(endpoint.uploads[&identity.upload_id].signal.aborted());
    }

    #[cfg(target_arch = "wasm32")]
    #[test]
    fn stale_completed_upload_cancel_does_not_cancel_a_later_upload() {
        use super::{CANCEL_PATH, SPEECH_PATH};
        use web_sys::AbortController;

        let completed = UploadIdentity {
            credential: "member-binding".to_owned(),
            upload_id: "0123456789abcdef0123456789abcdef".to_owned(),
        };
        let completed_controller =
            AbortController::new().expect("browser should support abort controllers");
        let later = UploadIdentity {
            credential: "member-binding".to_owned(),
            upload_id: "fedcba9876543210fedcba9876543210".to_owned(),
        };
        let later_controller =
            AbortController::new().expect("browser should support abort controllers");
        let later_signal = later_controller.signal();
        let mut endpoint = FakeSpeechEndpoint::default();
        endpoint.observe_speech_post(
            SPEECH_PATH,
            "POST",
            &completed
                .speech_headers()
                .expect("speech headers should be valid"),
            completed_controller.signal(),
        );
        endpoint.complete(&completed.upload_id);
        endpoint.observe_speech_post(
            SPEECH_PATH,
            "POST",
            &later
                .speech_headers()
                .expect("speech headers should be valid"),
            later_signal.clone(),
        );

        endpoint.receive_cancel_post(completed.cancel_request());

        assert_eq!(endpoint.cancel_posts.len(), 1);
        assert_eq!(endpoint.cancel_posts[0].path, CANCEL_PATH);
        assert_eq!(endpoint.cancel_posts[0].method, "POST");
        assert!(endpoint.uploads[&later.upload_id].active);
        assert!(!later_signal.aborted());
    }

    #[cfg(target_arch = "wasm32")]
    fn is_lowercase_hex(byte: u8) -> bool {
        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
    }

    #[cfg(target_arch = "wasm32")]
    #[derive(Default)]
    struct FakeSpeechEndpoint {
        uploads: std::collections::HashMap<String, FakeUpload>,
        cancel_posts: Vec<CancelRequest>,
    }

    #[cfg(target_arch = "wasm32")]
    struct FakeUpload {
        credential: String,
        signal: web_sys::AbortSignal,
        active: bool,
    }

    #[cfg(target_arch = "wasm32")]
    impl FakeSpeechEndpoint {
        fn observe_speech_post(
            &mut self,
            path: &str,
            method: &str,
            headers: &web_sys::Headers,
            signal: web_sys::AbortSignal,
        ) {
            assert_eq!(path, super::SPEECH_PATH);
            assert_eq!(method, "POST");
            let credential = headers
                .get(super::BINDING_HEADER)
                .expect("binding header should be readable")
                .expect("speech request should carry its binding");
            let upload_id = headers
                .get(super::UPLOAD_ID_HEADER)
                .expect("upload id header should be readable")
                .expect("speech request should carry its upload id");
            assert_eq!(
                headers
                    .get("content-type")
                    .expect("content type should be readable")
                    .as_deref(),
                Some(super::CLIP_CONTENT_TYPE)
            );
            self.uploads.insert(
                upload_id,
                FakeUpload {
                    credential,
                    signal,
                    active: true,
                },
            );
        }

        fn receive_cancel_post(&mut self, request: CancelRequest) {
            assert_eq!(request.path, super::CANCEL_PATH);
            assert_eq!(request.method, "POST");
            let headers = request
                .headers()
                .expect("cancel request headers should be valid");
            let credential = headers
                .get(super::BINDING_HEADER)
                .expect("binding header should be readable")
                .expect("cancel request should carry its binding");
            let upload_id = headers
                .get(super::UPLOAD_ID_HEADER)
                .expect("upload id header should be readable")
                .expect("cancel request should carry its upload id");
            assert_eq!(credential, request.credential);
            assert_eq!(upload_id, request.upload_id);
            if let Some(upload) = self.uploads.get_mut(&upload_id)
                && upload.credential == credential
                && upload.signal.aborted()
            {
                upload.active = false;
            }
            self.cancel_posts.push(request);
        }

        fn complete(&mut self, upload_id: &str) {
            if let Some(upload) = self.uploads.get_mut(upload_id) {
                upload.active = false;
            }
        }
    }
}

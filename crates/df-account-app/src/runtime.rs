//! The browser runtime: owns the `Model`, the current `Node` tree, every listener, request and timer,
//! and feeds messages through `Model::update`.
//!
//! One dispatch loop: `dispatch_all` queues messages, runs `Model::update` for each, performs the
//! returned effects, then renders once, synchronously (a background tab starves
//! `requestAnimationFrame`), and only then moves focus. A message that arrives while a dispatch is
//! running is queued; a DOM event that arrives while a dispatch is running is a consequence of our
//! own DOM changes (a blur while an element is replaced), never of the person, so it is ignored.
//!
//! Ownership: the `Closure`s of the four delegated listeners live in `App::listeners`, the closure
//! of each timer and each request deadline lives in its `Timeout`, and dropping the `App` removes
//! the listeners, clears the timers and aborts the requests. Nothing is leaked with
//! `Closure::forget`. A closure never drops itself while it runs (that is undefined in
//! wasm-bindgen), so a timer that finishes or is cancelled moves to `State::retired` and is dropped
//! at the start of the next browser callback.
//!
//! Events carry no handlers: the runtime listens once on the mount, computes the child-index path
//! of the target (`dom.rs`) and resolves the intent from the current tree (`patch.rs`).

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    rc::{Rc, Weak},
};

use js_sys::{Date, Function, Promise, Reflect};
use wasm_bindgen::{JsCast, JsValue, closure::Closure, prelude::wasm_bindgen};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    AbortController, AbortSignal, Document, Element, Event, EventTarget, Headers, MouseEvent,
    Request, RequestCache, RequestCredentials, RequestInit, Response, Window, console,
};

use crate::{
    api::{ApiCall, Guard, HttpRequest, Method, Reply},
    diagnostics::{self, Event as Log},
    dom::Dom,
    entry::{self, Host, Location},
    node::Node,
    patch,
    route::Route,
    state::{Effect, FocusTarget, Model, Msg, OperationSlot},
    view,
};

/// How long a request may wait for an answer before it counts as lost (plan 11: a mutation then
/// reads as an unknown outcome, a read as unavailable).
const REQUEST_DEADLINE_MS: i32 = 15_000;
/// Requests allowed in flight at once; a seventeenth is refused as lost instead of queued.
const MAXIMUM_IN_FLIGHT: usize = 16;

thread_local! {
    /// The running app. Owned here so its listeners live as long as the page.
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

fn console_error(event: Log<'_>) {
    console::error_1(&JsValue::from_str(&diagnostics::line(event)));
}

fn console_warn(event: Log<'_>) {
    console::warn_1(&JsValue::from_str(&diagnostics::line(event)));
}

fn refused(what: &'static str) {
    console_error(Log::BrowserRefused { what });
}

fn now_ms() -> u64 {
    // `Date.now()` is a whole number of milliseconds, never negative.
    Date::now().max(0.0) as u64
}

/// A `setTimeout` whose closure it owns; dropping it clears the timer.
struct Timeout {
    window: Window,
    handle: i32,
    _closure: Closure<dyn FnMut()>,
}

impl Timeout {
    fn start(
        window: &Window,
        delay_ms: i32,
        callback: impl FnMut() + 'static,
    ) -> Result<Self, JsValue> {
        let closure = Closure::<dyn FnMut()>::new(callback);
        let handle = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            closure.as_ref().unchecked_ref(),
            delay_ms,
        )?;
        Ok(Self {
            window: window.clone(),
            handle,
            _closure: closure,
        })
    }

    fn cancel(&self) {
        self.window.clear_timeout_with_handle(self.handle);
    }
}

impl Drop for Timeout {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// A listener and its closure; dropping it removes the listener.
struct Listener {
    target: EventTarget,
    kind: &'static str,
    closure: Closure<dyn FnMut(Event)>,
}

impl Listener {
    fn add(
        target: &EventTarget,
        kind: &'static str,
        handler: impl FnMut(Event) + 'static,
    ) -> Result<Self, JsValue> {
        let closure = Closure::<dyn FnMut(Event)>::new(handler);
        target.add_event_listener_with_callback(kind, closure.as_ref().unchecked_ref())?;
        Ok(Self {
            target: target.clone(),
            kind,
            closure,
        })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if self
            .target
            .remove_event_listener_with_callback(self.kind, self.closure.as_ref().unchecked_ref())
            .is_err()
        {
            refused("listener");
        }
    }
}

#[derive(Clone, Copy)]
enum Purpose {
    /// The answer is a `Msg::Replied`.
    Reply,
    /// The answer is an issued operation id for this form slot.
    Operation(OperationSlot),
}

/// A request that has been sent and not yet answered, cancelled or timed out.
struct InFlight {
    call: ApiCall,
    method: Method,
    generation: u64,
    purpose: Purpose,
    controller: AbortController,
    deadline: Timeout,
}

enum Outcome {
    Answered {
        status: u16,
        body: String,
    },
    /// A network error: the request never got an answer.
    NoAnswer,
    TimedOut,
}

struct State {
    model: Model,
    /// The tree the DOM currently shows; `None` before the first render.
    tree: Option<Node>,
    queue: VecDeque<Msg>,
    in_flight: HashMap<u64, InFlight>,
    timers: HashMap<u64, Timeout>,
    /// Timers that finished or were cancelled, dropped at the next browser callback.
    retired: Vec<Timeout>,
    next_id: u64,
    /// Path and query of the address as last seen; a `popstate` that changes only the fragment
    /// (the skip link, an error-summary link) is not a navigation.
    location_key: String,
}

impl State {
    fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
}

struct App {
    window: Window,
    dom: Dom,
    state: RefCell<State>,
    listeners: RefCell<Vec<Listener>>,
    /// True from the first message of a dispatch to the focus move after its render.
    is_dispatching: Cell<bool>,
    this: Weak<App>,
}

impl Drop for App {
    fn drop(&mut self) {
        for (_, flight) in self.state.get_mut().in_flight.drain() {
            flight.controller.abort();
        }
    }
}

impl App {
    fn start(window: Window, document: Document, mount: Element) -> Result<Rc<Self>, JsValue> {
        let location_key = location_key(&read_location(&window));
        let app = Rc::new_cyclic(|this| Self {
            dom: Dom::new(document, mount),
            state: RefCell::new(State {
                model: Model::default(),
                tree: None,
                queue: VecDeque::new(),
                in_flight: HashMap::new(),
                timers: HashMap::new(),
                retired: Vec::new(),
                next_id: 0,
                location_key,
            }),
            listeners: RefCell::new(Vec::new()),
            is_dispatching: Cell::new(false),
            this: this.clone(),
            window,
        });
        app.install_listeners()?;
        Ok(app)
    }

    fn install_listeners(&self) -> Result<(), JsValue> {
        let mount: &EventTarget = self.dom.mount().as_ref();
        let window: &EventTarget = self.window.as_ref();
        let listeners = vec![
            self.listen(mount, "click", |app, event| app.on_click(&event))?,
            self.listen(mount, "submit", |app, event| app.on_submit(&event))?,
            self.listen(mount, "input", |app, event| app.on_field(&event))?,
            self.listen(mount, "change", |app, event| app.on_field(&event))?,
            self.listen(window, "popstate", |app, _| app.on_popstate())?,
        ];
        *self.listeners.borrow_mut() = listeners;
        Ok(())
    }

    fn listen(
        &self,
        target: &EventTarget,
        kind: &'static str,
        handler: fn(&App, Event),
    ) -> Result<Listener, JsValue> {
        let this = self.this.clone();
        Listener::add(target, kind, move |event| {
            if let Some(app) = this.upgrade() {
                // A callback starts here, so nothing retired by an earlier one is still running.
                app.drop_retired();
                handler(&app, event);
            }
        })
    }

    fn drop_retired(&self) {
        let retired = std::mem::take(&mut self.state.borrow_mut().retired);
        drop(retired);
    }

    fn retire(&self, timeout: Timeout) {
        timeout.cancel();
        self.state.borrow_mut().retired.push(timeout);
    }

    // ---- events ------------------------------------------------------------------------------

    fn target_path(&self, event: &Event) -> Option<patch::Path> {
        let target = event.target()?.dyn_into::<web_sys::Node>().ok()?;
        self.dom.path_of(&target)
    }

    fn on_click(&self, event: &Event) {
        if self.is_dispatching.get() {
            return;
        }
        let Some(mouse) = event.dyn_ref::<MouseEvent>() else {
            return;
        };
        let Some(path) = self.target_path(event) else {
            return;
        };
        let hit = {
            let state = self.state.borrow();
            state.tree.as_ref().and_then(|tree| {
                patch::click_target(tree, &path).map(|hit| (hit.value, hit.element.tag == "a"))
            })
        };
        let Some((intent, is_link)) = hit else {
            return;
        };
        if is_link {
            // A modified click means "open in a new tab or window": the browser's job.
            if !is_plain_left_click(mouse) {
                return;
            }
            event.prevent_default();
        }
        self.dispatch(Msg::Intent(intent));
    }

    fn on_submit(&self, event: &Event) {
        if event
            .target()
            .and_then(|target| target.dyn_into::<Element>().ok())
            .is_some_and(|element| {
                element.get_attribute("data-df-oauth").as_deref() == Some("google")
            })
        {
            return;
        }
        // Always ours: a page reload would lose the form.
        event.prevent_default();
        if self.is_dispatching.get() {
            return;
        }
        let Some(path) = self.target_path(event) else {
            return;
        };
        let intent = {
            let state = self.state.borrow();
            state
                .tree
                .as_ref()
                .and_then(|tree| patch::submit_target(tree, &path).map(|hit| hit.value))
        };
        if let Some(intent) = intent {
            self.dispatch(Msg::Intent(intent));
        }
    }

    /// `input` and `change`: the same control may fire both; the model takes the repeat as a no-op.
    fn on_field(&self, event: &Event) {
        if self.is_dispatching.get() {
            return;
        }
        let Some(path) = self.target_path(event) else {
            return;
        };
        let found = {
            let state = self.state.borrow();
            state.tree.as_ref().and_then(|tree| {
                patch::field_target(tree, &path)
                    .map(|hit| (hit.value, patch::control(hit.element), hit.path))
            })
        };
        let Some((field, control, element_path)) = found else {
            return;
        };
        let is_change = event.type_() == "change";
        let message = match control {
            patch::Control::Text | patch::Control::Select => self
                .dom
                .value_at(&element_path)
                .map(|value| Msg::Edit(field, value)),
            patch::Control::Checkbox => self
                .dom
                .is_checked_at(&element_path)
                .map(|is_on| Msg::Toggle(field, is_on)),
            // A radio is answered once, when it becomes the checked one of its group.
            patch::Control::Radio if is_change => match self.dom.is_checked_at(&element_path) {
                Some(true) => self
                    .dom
                    .value_at(&element_path)
                    .map(|value| Msg::Edit(field, value)),
                Some(false) | None => None,
            },
            patch::Control::Radio => None,
        };
        if let Some(message) = message {
            self.dispatch(message);
        }
    }

    fn on_popstate(&self) {
        if self.is_dispatching.get() {
            return;
        }
        let location = read_location(&self.window);
        let key = location_key(&location);
        {
            let mut state = self.state.borrow_mut();
            // A fragment that carries a token is a new link opened in this tab: handle it so the token
            // is used and removed from the address bar.
            if state.location_key == key && !location.fragment.contains("token=") {
                return;
            }
            state.location_key = key;
        }
        let route = Route::parse(&location.path, &location.query, &location.fragment);
        self.dispatch(Msg::LocationChanged(route));
    }

    // ---- dispatch ----------------------------------------------------------------------------

    fn dispatch(&self, message: Msg) {
        self.dispatch_all(vec![message]);
    }

    fn dispatch_all(&self, messages: Vec<Msg>) {
        self.state.borrow_mut().queue.extend(messages);
        if self.is_dispatching.replace(true) {
            return;
        }
        loop {
            let mut focus = None;
            loop {
                let next = self.state.borrow_mut().queue.pop_front();
                let Some(message) = next else {
                    break;
                };
                let effects = self.state.borrow_mut().model.update(message);
                for effect in effects {
                    self.perform(effect, &mut focus);
                }
            }
            self.render();
            if let Some(target) = focus
                && self.dom.focus(target).is_err()
            {
                refused("focus");
            }
            if self.state.borrow().queue.is_empty() {
                break;
            }
        }
        self.is_dispatching.set(false);
    }

    fn enqueue(&self, messages: Vec<Msg>) {
        self.state.borrow_mut().queue.extend(messages);
    }

    fn render(&self) {
        let new_tree = view::view(&self.state.borrow().model);
        let ops = {
            let mut state = self.state.borrow_mut();
            let ops = state.tree.as_ref().map(|old| patch::diff(old, &new_tree));
            state.tree = Some(new_tree);
            ops
        };
        match ops.map(|ops| self.dom.apply(&ops).is_ok()) {
            Some(true) => return,
            // An operation the DOM could not take (something outside the app changed it).
            Some(false) => refused("render"),
            // The first render has nothing to patch.
            None => {}
        }
        let tree = self.state.borrow().tree.clone();
        if let Some(tree) = tree
            && self.dom.rebuild(&tree).is_err()
        {
            refused("render");
        }
    }

    // ---- effects -----------------------------------------------------------------------------

    fn perform(&self, effect: Effect, focus: &mut Option<FocusTarget>) {
        match effect {
            Effect::Api { generation, call } => {
                self.start_request(generation, call, Purpose::Reply)
            }
            Effect::IssueOperation { generation, slot } => {
                self.start_request(generation, ApiCall::Operation, Purpose::Operation(slot));
            }
            Effect::PushUrl(path) => self.change_history(&path, true),
            Effect::ReplaceUrl(path) => self.change_history(&path, false),
            Effect::Leave(url) => self.leave(&url),
            Effect::CancelPending => self.cancel_pending(),
            Effect::After {
                generation,
                delay_ms,
            } => self.start_timer(generation, delay_ms),
            Effect::Focus(target) => *focus = Some(target),
            Effect::Title(title) => self.dom.set_title(&title),
            Effect::CopyText(text) => copy_text(&self.window, &text),
            Effect::CopySecret(secret) => copy_text(&self.window, &secret.0),
            Effect::DownloadCsv { filename, content } => {
                download_text(&self.window, &filename, &content)
            }
        }
    }

    fn start_request(&self, generation: u64, call: ApiCall, purpose: Purpose) {
        let http = call.request();
        let method = http.method;
        let (id, csrf, is_full) = {
            let mut state = self.state.borrow_mut();
            (
                state.next_id(),
                state.model.csrf().to_owned(),
                state.in_flight.len() >= MAXIMUM_IN_FLIGHT,
            )
        };
        if is_full {
            refused("request");
            self.enqueue(reply_messages(generation, purpose, call.lost()));
            return;
        }
        match self.send(id, &http, &csrf) {
            Ok((controller, deadline, request)) => {
                self.state.borrow_mut().in_flight.insert(
                    id,
                    InFlight {
                        call,
                        method,
                        generation,
                        purpose,
                        controller,
                        deadline,
                    },
                );
                let this = self.this.clone();
                let window = self.window.clone();
                spawn_local(async move {
                    let outcome = match fetch_answer(&window, &request).await {
                        Some((status, body)) => Outcome::Answered { status, body },
                        None => Outcome::NoAnswer,
                    };
                    if let Some(app) = this.upgrade() {
                        app.drop_retired();
                        app.complete(id, outcome);
                    }
                });
            }
            Err(_) => {
                console_error(Log::RequestFailed { method, status: 0 });
                self.enqueue(reply_messages(generation, purpose, call.lost()));
            }
        }
    }

    /// Builds the request, its abort controller and its deadline timer.
    fn send(
        &self,
        id: u64,
        http: &HttpRequest,
        csrf: &str,
    ) -> Result<(AbortController, Timeout, Request), JsValue> {
        let controller = AbortController::new()?;
        let request = build_request(http, csrf, &controller.signal())?;
        let this = self.this.clone();
        let deadline = Timeout::start(&self.window, REQUEST_DEADLINE_MS, move || {
            if let Some(app) = this.upgrade() {
                app.drop_retired();
                app.complete(id, Outcome::TimedOut);
            }
        })?;
        Ok((controller, deadline, request))
    }

    /// A request ended. Unknown ids were cancelled (navigation), so their late end is dropped.
    fn complete(&self, id: u64, outcome: Outcome) {
        let flight = self.state.borrow_mut().in_flight.remove(&id);
        let Some(flight) = flight else {
            return;
        };
        let InFlight {
            call,
            method,
            generation,
            purpose,
            controller,
            deadline,
        } = flight;
        self.retire(deadline);
        let reply = match outcome {
            Outcome::Answered { status, body } => {
                if !(200..300).contains(&status) {
                    console_warn(Log::RequestFailed { method, status });
                }
                call.decode(status, &body)
            }
            Outcome::NoAnswer => {
                console_error(Log::RequestFailed { method, status: 0 });
                call.lost()
            }
            Outcome::TimedOut => {
                controller.abort();
                console_error(Log::RequestTimedOut { method });
                call.lost()
            }
        };
        self.dispatch_all(reply_messages(generation, purpose, reply));
    }

    fn cancel_pending(&self) {
        let (flights, timers) = {
            let mut state = self.state.borrow_mut();
            (
                std::mem::take(&mut state.in_flight),
                std::mem::take(&mut state.timers),
            )
        };
        for (_, flight) in flights {
            flight.controller.abort();
            self.retire(flight.deadline);
        }
        for (_, timer) in timers {
            self.retire(timer);
        }
    }

    fn start_timer(&self, generation: u64, delay_ms: u32) {
        let id = self.state.borrow_mut().next_id();
        let this = self.this.clone();
        let delay = i32::try_from(delay_ms).unwrap_or(i32::MAX);
        let started = Timeout::start(&self.window, delay, move || {
            if let Some(app) = this.upgrade() {
                app.drop_retired();
                app.timer_fired(id, generation);
            }
        });
        match started {
            Ok(timeout) => {
                self.state.borrow_mut().timers.insert(id, timeout);
            }
            Err(_) => refused("timer"),
        }
    }

    fn timer_fired(&self, id: u64, generation: u64) {
        let timer = self.state.borrow_mut().timers.remove(&id);
        // A cancelled timer is gone from the map; a fire that raced the cancel is dropped.
        let Some(timer) = timer else {
            return;
        };
        self.retire(timer);
        self.dispatch_all(vec![
            Msg::Clock { now_ms: now_ms() },
            Msg::TimerFired { generation },
        ]);
    }

    fn change_history(&self, path: &str, is_push: bool) {
        let result = self.window.history().and_then(|history| {
            if is_push {
                history.push_state_with_url(&JsValue::NULL, "", Some(path))
            } else {
                history.replace_state_with_url(&JsValue::NULL, "", Some(path))
            }
        });
        if result.is_err() {
            refused("history");
            return;
        }
        self.state.borrow_mut().location_key = location_key(&read_location(&self.window));
    }

    fn leave(&self, url: &str) {
        if !patch::is_allowed_destination(url) || self.window.location().assign(url).is_err() {
            refused("navigation");
        }
    }
}

fn is_plain_left_click(event: &MouseEvent) -> bool {
    event.button() == 0
        && !event.ctrl_key()
        && !event.meta_key()
        && !event.shift_key()
        && !event.alt_key()
}

/// The browser's clock, then the delivery: the model's deadlines read the clock.
fn reply_messages(generation: u64, purpose: Purpose, reply: Reply) -> Vec<Msg> {
    let clock = Msg::Clock { now_ms: now_ms() };
    match (purpose, reply) {
        (Purpose::Operation(slot), Reply::Operation(result)) => vec![
            clock,
            Msg::OperationIssued {
                generation,
                slot,
                result,
            },
        ],
        (Purpose::Operation(_), _) => {
            // `ApiCall::Operation` only ever decodes to `Reply::Operation`.
            refused("operation");
            vec![clock]
        }
        (Purpose::Reply, reply) => vec![clock, Msg::Replied { generation, reply }],
    }
}

fn build_request(http: &HttpRequest, csrf: &str, signal: &AbortSignal) -> Result<Request, JsValue> {
    let headers = Headers::new()?;
    headers.set("Accept", "application/json")?;
    if http.body.is_some() {
        headers.set("Content-Type", "application/json")?;
    }
    match http.guard {
        Guard::None => {}
        Guard::Csrf => headers.set("x-df-csrf", csrf)?,
        Guard::Device => headers.set("x-df-device", "1")?,
    }
    let init = RequestInit::new();
    init.set_method(http.method.as_str());
    init.set_headers_headers(&headers);
    init.set_credentials(RequestCredentials::SameOrigin);
    // Reads (the lobby, the pairing status) must never come from a cache.
    init.set_cache(RequestCache::NoStore);
    init.set_signal(Some(signal));
    if let Some(body) = &http.body {
        init.set_body(&JsValue::from_str(body));
    }
    Request::new_with_str_and_init(&http.path, &init)
}

/// The status and body text of the answer, or `None` when no answer arrived.
async fn fetch_answer(window: &Window, request: &Request) -> Option<(u16, String)> {
    let response = JsFuture::from(window.fetch_with_request(request))
        .await
        .ok()?
        .dyn_into::<Response>()
        .ok()?;
    let body = JsFuture::from(response.text().ok()?)
        .await
        .ok()?
        .as_string()?;
    Some((response.status(), body))
}

/// Calls `target[name](argument)` and returns what it returned.
fn call_method(
    target: &JsValue,
    name: &str,
    argument: Option<&JsValue>,
) -> Result<JsValue, JsValue> {
    let function = Reflect::get(target, &JsValue::from_str(name))?
        .dyn_into::<Function>()
        .map_err(|_| JsValue::from_str("not a function"))?;
    match argument {
        Some(argument) => function.call1(target, argument),
        None => function.call0(target),
    }
}

/// Starts `navigator.clipboard.writeText` inside the click that asked for it (the browser only
/// allows it during a user gesture) and logs a refusal when it fails.
fn copy_text(window: &Window, text: &str) {
    let started = Reflect::get(&window.navigator(), &JsValue::from_str("clipboard"))
        .and_then(|clipboard| call_method(&clipboard, "writeText", Some(&JsValue::from_str(text))))
        .and_then(|promise| promise.dyn_into::<Promise>());
    match started {
        Ok(promise) => spawn_local(async move {
            if JsFuture::from(promise).await.is_err() {
                refused("clipboard");
            }
        }),
        Err(_) => refused("clipboard"),
    }
}

/// Downloads a generated text file through a temporary object URL and anchor.
fn download_text(window: &Window, filename: &str, content: &str) {
    let parts = js_sys::Array::new();
    parts.push(&JsValue::from_str(content));
    let Ok(blob) = web_sys::Blob::new_with_str_sequence(&parts) else {
        refused("download");
        return;
    };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else {
        refused("download");
        return;
    };
    if let Some(anchor) = window
        .document()
        .and_then(|document| document.create_element("a").ok())
        .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = anchor.set_attribute("href", &url);
        let _ = anchor.set_attribute("download", filename);
        anchor.click();
    }
    let _ = web_sys::Url::revoke_object_url(&url);
}

fn read_location(window: &Window) -> Location {
    let location = window.location();
    let read = |value: Result<String, JsValue>| {
        value.unwrap_or_else(|_| {
            refused("location");
            String::new()
        })
    };
    let search = read(location.search());
    let hash = read(location.hash());
    Location {
        path: read(location.pathname()),
        query: search.strip_prefix('?').unwrap_or(&search).to_owned(),
        fragment: hash.strip_prefix('#').unwrap_or(&hash).to_owned(),
    }
}

fn location_key(location: &Location) -> String {
    format!("{}?{}", location.path, location.query)
}

struct BrowserHost {
    window: Window,
}

impl Host for BrowserHost {
    fn location(&self) -> Location {
        read_location(&self.window)
    }

    fn offset_east_minutes(&self) -> i32 {
        // `getTimezoneOffset` is minutes west of UTC and a whole number.
        (-Date::new_0().get_timezone_offset()).round() as i32
    }

    fn replace_state(&mut self, path: &str) {
        let result = self
            .window
            .history()
            .and_then(|history| history.replace_state_with_url(&JsValue::NULL, "", Some(path)));
        if result.is_err() {
            refused("history");
        }
    }

    fn install_panic_hook(&mut self) {
        std::panic::set_hook(Box::new(|info| {
            console_error(Log::Panicked {
                message: &info.to_string(),
            });
        }));
    }

    fn start_router(&mut self, started: Msg) {
        if let Msg::Started { route, .. } = &started {
            console::info_1(&JsValue::from_str(&diagnostics::line(Log::Booted {
                route: route.name(),
            })));
        }
        let mounted = self.window.document().and_then(|document| {
            document
                .get_element_by_id("app")
                .map(|mount| (document, mount))
        });
        let Some((document, mount)) = mounted else {
            refused("mount");
            return;
        };
        match App::start(self.window.clone(), document, mount) {
            Ok(app) => {
                APP.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&app)));
                app.dispatch(started);
            }
            Err(_) => refused("listener"),
        }
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    let Some(window) = web_sys::window() else {
        refused("window");
        return;
    };
    entry::boot(&mut BrowserHost { window });
}

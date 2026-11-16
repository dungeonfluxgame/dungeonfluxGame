//! A real phone's way to a seat: the seat key it keeps, the seat machine (`seat`) it follows, the
//! asks it makes of the host's seat routes and the join screens it draws in the phone page's place.
//!
//! The key is 256 random bits from the browser's cryptographic source, kept in the browser's
//! storage apart from the page's name and sent only in `path::SEAT_KEY_HEADER`. The machine moves
//! on the host's answers; a tap only sends the player's choice and draws what comes back. A join
//! screen takes the phone page's place (the page is hidden while it shows, so the phone has one
//! screen and one scrolling body), with its own listeners kept for the page's life; once the
//! seat's table moves on from the lobby it hides and the play screen shows. A failed ask keeps the
//! screen as it is and is noted; the next poll waits longer after each failure (`backoff`).

use std::cell::RefCell;

use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Document, Element, Event, HtmlInputElement, KeyboardEvent, Window};

use super::events::action_of;
use super::frames::wait_ms;
use super::kept_in_storage;
use super::state::{ROOT_ID, log_error};
use crate::backoff::retry_delay_ms;
use crate::dev_reply::Rejection;
use crate::dom::{append, clear, element, js_message};
use crate::limits::SEAT_POLL_MS;
use crate::net::{NetError, fetch_seat_text, post_seat_json};
use crate::path::{
    PLAY_HELLO, PLAY_HERO, PLAY_HEROES, PLAY_JOIN, PLAY_JOIN_STATUS, PLAY_ROOM, PLAY_SEAT_INTENT,
    PLAY_SEAT_VIEW,
};
use crate::seat::screen::{JoinScreen, SeatTap, screen_of};
use crate::seat::wire::{
    HelloReply, HeroChoice, HeroesReply, JoinRequest, JoinStatus, RoomReply, SeatCard, SeatIntent,
    SeatIntentReply, SeatReply, SeatView, hex_digits, is_seat_key,
};
use crate::seat::{SeatEvent, SeatMachine, SeatPoll};
use crate::timeline::ErrorSource;
use crate::view::seat::{NAME_FIELD_ID, build};

/// Where a phone keeps its seat key in the browser's storage, apart from its name.
const SEAT_KEY_STORAGE: &str = "df-phone-seat-key";
/// The join screens of this page.
struct SeatPage {
    machine: SeatMachine,
    key: String,
    layer: Element,
    drawn: Option<Option<JoinScreen>>,
    /// The layer's listeners, kept as long as the page.
    _listeners: [Closure<dyn FnMut(Event)>; 2],
}

thread_local! {
    static SEAT: RefCell<Option<SeatPage>> = const { RefCell::new(None) };
}

/// A new seat key from the browser's cryptographic source.
fn new_seat_key(window: &Window) -> Result<String, JsValue> {
    let mut random = [0_u8; 32];
    window
        .crypto()?
        .get_random_values_with_u8_array(&mut random)?;
    Ok(hex_digits(&random))
}

/// The seat key this phone keeps across reloads, apart from its name (`kept_in_storage`).
fn kept_seat_key(window: &Window) -> Result<String, JsValue> {
    kept_in_storage(window, SEAT_KEY_STORAGE, "seat key", is_seat_key, || {
        new_seat_key(window)
    })
}

fn on_click(event: &Event) {
    if let Some(tap) = action_of(event).and_then(|(_, action)| SeatTap::from_action(&action)) {
        spawn_local(on_tap(tap));
    }
}

/// Enter in the name field joins as a guest.
fn on_key(event: &Event) {
    let Some(key_event) = event.dyn_ref::<KeyboardEvent>() else {
        return;
    };
    let in_name = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
        .is_some_and(|target| target.id() == NAME_FIELD_ID);
    if in_name && key_event.key() == "Enter" {
        spawn_local(on_tap(SeatTap::JoinGuest));
    }
}

/// Starts the join screens over a real phone's page, and the loop that follows the host.
pub fn start(document: &Document, window: &Window) -> Result<(), JsValue> {
    let key = kept_seat_key(window)?;
    // The join screens are the phone's first screen, before the page draws: they need the kit's
    // look from their first draw.
    crate::phone_kit::draw::ensure_style(document)?;
    let body = document
        .body()
        .ok_or_else(|| JsValue::from_str("the page has no body"))?;
    let layer = element(document, "div", "ph-join-layer")?;
    let click = Closure::<dyn FnMut(Event)>::new(|event: Event| on_click(&event));
    layer.add_event_listener_with_callback("click", click.as_ref().unchecked_ref())?;
    let key_down = Closure::<dyn FnMut(Event)>::new(|event: Event| on_key(&event));
    layer.add_event_listener_with_callback("keydown", key_down.as_ref().unchecked_ref())?;
    append(&body, &layer)?;
    SEAT.with(|seat| {
        *seat.borrow_mut() = Some(SeatPage {
            machine: SeatMachine::default(),
            key,
            layer,
            drawn: None,
            _listeners: [click, key_down],
        });
    });
    draw();
    spawn_local(run());
    Ok(())
}

fn with_seat<T>(act: impl FnOnce(&mut SeatPage) -> T) -> Option<T> {
    SEAT.with(|seat| seat.borrow_mut().as_mut().map(act))
}

/// The value of the name field now, if it is shown.
fn typed_name() -> Option<String> {
    name_field().map(|field| field.value())
}

/// Shows `shown` and hides `hidden`, so one screen (and its one scrolling body) is on the phone.
fn swap(shown: &Element, hidden: &Element) -> Result<(), JsValue> {
    hidden.set_attribute("hidden", "")?;
    shown.remove_attribute("hidden")
}

/// Draws the screen of the machine's state when it changed: a join screen in place of the phone
/// page, or the phone page once the seat's table has moved on from the lobby. The name a guest
/// typed is kept.
fn draw() {
    let typed = typed_name();
    let result = with_seat(|page| -> Result<(), JsValue> {
        let screen = screen_of(&page.machine);
        if page.drawn.as_ref() == Some(&screen) {
            return Ok(());
        }
        let document = crate::dom::document()?;
        let phone_page = document
            .get_element_by_id(ROOT_ID)
            .ok_or_else(|| JsValue::from_str("the page has no root"))?;
        clear(&page.layer);
        match &screen {
            None => swap(&phone_page, &page.layer)?,
            Some(join) => {
                append(&page.layer, &build(&document, join)?)?;
                swap(&page.layer, &phone_page)?;
            }
        }
        page.drawn = Some(screen);
        Ok(())
    });
    if let Some(Err(error)) = result {
        log_error(ErrorSource::Draw, &js_message(&error));
    }
    if let (Some(name), Some(field)) = (typed, name_field()) {
        field.set_value(&name);
    }
}

fn name_field() -> Option<HtmlInputElement> {
    let document = web_sys::window()?.document()?;
    document
        .get_element_by_id(NAME_FIELD_ID)?
        .dyn_into::<HtmlInputElement>()
        .ok()
}

/// Applies `event` to the machine and draws what changed.
fn apply(event: SeatEvent) {
    with_seat(|page| page.machine.apply(event));
    draw();
}

/// What an ask of a seat route came back with.
enum Answer<T> {
    Value(T),
    Refused(SeatEvent),
    /// Not answered or not readable: noted, and asked again later.
    Failed,
}

fn read_reply<T: DeserializeOwned>(address: &str, text: Result<String, NetError>) -> Answer<T> {
    let body = match text {
        Ok(body) | Err(NetError::Status { body, .. }) => body,
        Err(error) => {
            log_error(ErrorSource::Net, &error.to_string());
            return Answer::Failed;
        }
    };
    match serde_json::from_str::<SeatReply<T>>(&body).map(SeatReply::into_result) {
        Ok(Ok(value)) => Answer::Value(value),
        Ok(Err(Rejection { kind, message })) => {
            Answer::Refused(SeatEvent::Refused { kind, message })
        }
        Err(cause) => {
            log_error(
                ErrorSource::Net,
                &format!("{address} did not answer as a seat route: {cause}"),
            );
            Answer::Failed
        }
    }
}

fn seat_key() -> Option<String> {
    with_seat(|page| page.key.clone())
}

async fn get<T: DeserializeOwned>(address: &str) -> Answer<T> {
    let Some(key) = seat_key() else {
        return Answer::Failed;
    };
    read_reply(address, fetch_seat_text(address, Some(&key)).await)
}

async fn post<Body: Serialize, T: DeserializeOwned>(address: &str, body: &Body) -> Answer<T> {
    let Some(key) = seat_key() else {
        return Answer::Failed;
    };
    let text = match serde_json::to_string(body) {
        Ok(text) => text,
        Err(cause) => {
            log_error(
                ErrorSource::Net,
                &format!("could not write a seat request: {cause}"),
            );
            return Answer::Failed;
        }
    };
    read_reply(address, post_seat_json(address, Some(&key), &text).await)
}

/// Applies a value as `event`, or the refusal. `false` when the ask failed.
fn settle<T>(answer: Answer<T>, event: impl FnOnce(T) -> SeatEvent) -> bool {
    match answer {
        Answer::Value(value) => apply(event(value)),
        Answer::Refused(refusal) => apply(refusal),
        Answer::Failed => return false,
    }
    true
}

/// The body of `PLAY_HELLO`: nothing but the key in its header.
#[derive(Serialize)]
struct HelloBody {}

/// Asks the host what the machine's state needs. `false` when the ask failed.
async fn poll_once() -> bool {
    match with_seat(|page| page.machine.poll()).flatten() {
        Some(SeatPoll::Room) => {
            if !settle(get::<RoomReply>(PLAY_ROOM).await, SeatEvent::RoomLoaded) {
                return false;
            }
            let still_in_room = with_seat(|page| page.machine.poll() == Some(SeatPoll::Room));
            if still_in_room != Some(true) {
                return true;
            }
            let hello = post::<_, HelloReply>(PLAY_HELLO, &HelloBody {}).await;
            settle(hello, SeatEvent::HelloAnswered)
        }
        Some(SeatPoll::JoinStatus) => settle(
            get::<JoinStatus>(PLAY_JOIN_STATUS).await,
            SeatEvent::JoinAnswered,
        ),
        Some(SeatPoll::Heroes) => settle(get::<HeroesReply>(PLAY_HEROES).await, |reply| {
            SeatEvent::HeroesLoaded(reply.heroes)
        }),
        Some(SeatPoll::SeatView) => {
            settle(get::<SeatView>(PLAY_SEAT_VIEW).await, SeatEvent::ViewLoaded)
        }
        None => true,
    }
}

/// Polls the host, waiting longer after each failure in a row as the frame loop does.
async fn run() {
    let mut failures = 0_u32;
    loop {
        failures = if poll_once().await {
            0
        } else {
            failures.saturating_add(1)
        };
        wait_ms(SEAT_POLL_MS.max(retry_delay_ms(failures))).await;
    }
}

async fn join(request: &JoinRequest) {
    settle(post(PLAY_JOIN, request).await, SeatEvent::JoinAnswered);
}

async fn on_tap(tap: SeatTap) {
    match tap {
        SeatTap::Continue => join(&JoinRequest::Returning).await,
        SeatTap::Guest => apply(SeatEvent::ChoseGuest),
        SeatTap::Friend => apply(SeatEvent::ChoseFriend),
        SeatTap::SignIn => {
            apply(SeatEvent::ChoseSignIn);
            join(&JoinRequest::Account).await;
        }
        SeatTap::Back => apply(SeatEvent::Back),
        SeatTap::JoinGuest => {
            let name = typed_name().unwrap_or_default().trim().to_owned();
            join(&JoinRequest::Guest { name }).await;
        }
        SeatTap::PickFriend(profile) => join(&JoinRequest::Friend { profile }).await,
        SeatTap::PlayHero(hero) => {
            let answer = post::<_, SeatCard>(PLAY_HERO, &HeroChoice { hero }).await;
            settle(answer, SeatEvent::HeroChosen);
        }
        SeatTap::MakeHero => apply(SeatEvent::MakeHero),
        SeatTap::Act(action) => {
            let answer = post::<_, SeatIntentReply>(PLAY_SEAT_INTENT, &SeatIntent { action }).await;
            settle(answer, SeatEvent::IntentAnswered);
        }
        SeatTap::Retry => {
            with_seat(|page| page.machine = SeatMachine::default());
            draw();
            // A failed ask is noted where it failed; the loop asks again.
            poll_once().await;
        }
    }
}

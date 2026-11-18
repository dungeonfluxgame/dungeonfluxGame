//! A real phone's way to a seat: the seat key it keeps, the seat machine (`seat`) it follows, the
//! asks it makes of the host's seat routes and the join screens it draws in the phone page's place.
//!
//! The key is 256 random bits from the browser's cryptographic source, kept in the browser's
//! storage apart from the page's name and sent only in `path::SEAT_KEY_HEADER`. The machine moves
//! on the host's answers; a tap only sends the player's choice and draws what comes back. A join
//! screen takes the phone page's place (the page is hidden while it shows, so the phone has one
//! screen and one scrolling body), with its own listeners kept for the page's life; once the
//! seat's table moves on from the lobby it hides and the play screen shows. Every ask goes through
//! `seat_call` (`net::call` with the seat key). A failed ask keeps the screen as it is and is
//! noted; one that did not reach the host shows the link line until a seat route answers again,
//! and the next poll waits longer after each failure (`backoff`), but no longer than until the page
//! is woken (`frames::wake`: shown again, back online, or the link back). A seat action (`act`)
//! whose answer did not come is never dropped: it is sent again with its own `said` id until the
//! host answers it, so a tap made while the link is down is done once when the link is back.
//!
//! A new way in (Guest, Friend, Sign in) carries the room code the table shows: the phone asks for
//! it first ("Room code", its field drawn here), unless the page's link carried it (`?code=`). Next
//! asks the host whether the code is the table's (`path::PLAY_JOIN_CODE`, under the same wait after
//! a wrong code as a join), so a wrong one is asked again before the player names themselves; a
//! code a join still refuses (`code_mismatch`, a new code since Next) is forgotten and asked again.
//! The name typed is kept while the code screen shows. Coming back to the seat the key holds needs
//! none.

use std::cell::RefCell;

use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Document, Element, Event, HtmlInputElement, KeyboardEvent, Window};

use super::events::action_of;
use super::frames::{seat_link_back, seat_link_lost, wait_or_wake};
use super::kept_in_storage;
use super::page::{redraw_part, render};
use super::state::{ROOT_ID, log_error, with_app_mut};
use crate::backoff::retry_delay_ms;
use crate::dev_reply::ErrorTag;
use crate::dev_reply::Rejection;
use crate::dom::{append, clear, element, js_message, text_element};
use crate::draw_plan::Part;
use crate::limits::{SEAT_POLL_MS, WORK_REQUEST_TIMEOUT_MS};
use crate::lobby::model::room_code;
use crate::net::{Call, CallError, NetError, call};
use crate::path::{
    PLAY_HELLO, PLAY_HEROES, PLAY_JOIN, PLAY_JOIN_CODE, PLAY_JOIN_STATUS, PLAY_ROOM,
    PLAY_SEAT_ACTION, PLAY_SEAT_VIEW, query_value,
};
use crate::phone_kit::shell::PhoneHeader;
use crate::seat::play::{RollSeen, play_of};
use crate::seat::screen::{
    JoinScreen, LobbyTap, SeatTap, code_screen, hero_header, lobby_viewer, screen_of,
};
use crate::seat::wire::{
    CodeCheck, CodeTaken, HelloReply, HeroesReply, JoinRequest, JoinStatus, LobbyStep, RoomReply,
    SEAT_TROUBLE_WORDS, SayKind, SeatAction, SeatActionErrorKind, SeatErrorKind, SeatOutcome,
    SeatView, hex_digits, is_seat_key,
};
use crate::seat::{SeatEvent, SeatMachine, SeatPoll, SeatState, Unsettled};
use crate::timeline::ErrorSource;
use crate::view::seat::{NAME_FIELD_ID, build};

/// Where a phone keeps its seat key in the browser's storage, apart from its name.
const SEAT_KEY_STORAGE: &str = "df-phone-seat-key";
/// The id of the room code field, read when Next is tapped.
const CODE_FIELD_ID: &str = "ph-join-code";
/// The page address's query key of the room code a join link carries.
const CODE_QUERY_KEY: &str = "code";
/// What the player is told when the typed code is not six of the code's letters and digits.
const CODE_SHAPE_WORDS: &str = "A room code is six letters and digits, as on the screen.";
/// The join screens of this page.
struct SeatPage {
    machine: SeatMachine,
    key: String,
    layer: Element,
    drawn: Option<Option<JoinScreen>>,
    /// The look studio's change count when it was last drawn, while it shows.
    look_drawn: Option<u64>,
    /// The picture of the player's hero the lobby shows full screen, when one is open.
    viewing: Option<String>,
    /// The room code a new way in carries: the link's, or the one typed; none until one is.
    code: Option<String>,
    /// Why the last code was not taken.
    code_problem: Option<String>,
    /// The name last typed in the name field, kept while another screen (the code's) shows.
    name: Option<String>,
    /// The lobby step on its way to the host, until the seat's view after it is drawn.
    stepping: Option<LobbyStep>,
    /// The seat's last roll as Act first saw it, so a result clears with the next ask.
    roll_seen: RollSeen,
    /// The layer's listeners, kept as long as the page.
    _listeners: [Closure<dyn FnMut(Event)>; 7],
}

thread_local! {
    static SEAT: RefCell<Option<SeatPage>> = const { RefCell::new(None) };
    /// The actions sent whose fate is not known yet (on their way, sent again while the link is
    /// down, or answered unreadably): sending the same action again sends it with its `said` id,
    /// so the host does it once.
    static UNSETTLED: RefCell<Unsettled> = RefCell::new(Unsettled::default());
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
    if let Some((_, action)) = action_of(event) {
        dispatch(&action);
    }
}

/// Takes a tap's action: the look studio's, the lobby viewer's (the hero's pictures full screen,
/// from the lobby or the play screen's header), or a seat's. Returns whether it was one.
pub(super) fn dispatch(action: &str) -> bool {
    if super::look::on_action(action) {
        return true;
    }
    // The hero build's taps (`pages::build`), drawn on this layer over the lobby.
    if super::pages::dispatch(action) {
        return true;
    }
    if let Some(tap) = LobbyTap::from_action(action) {
        with_seat(|page| {
            page.viewing = match tap {
                LobbyTap::View(picture) => Some(picture),
                LobbyTap::Close => None,
            }
        });
        draw();
        return true;
    }
    on_action(action)
}

/// Typing in the look studio's name field.
fn on_input(event: &Event) {
    let Some(input) = event
        .target()
        .and_then(|target| target.dyn_into::<HtmlInputElement>().ok())
    else {
        return;
    };
    if input.id() == crate::look::studio::NAME_FIELD_ID {
        super::look::typed_name(input.value());
    }
}

/// A look studio slider let go: its action with the value it was left at.
fn on_change(event: &Event) {
    let Some((carrier, action)) = action_of(event) else {
        return;
    };
    if let Ok(input) = carrier.dyn_into::<HtmlInputElement>() {
        super::look::on_action(&format!("{action}:{}", input.value()));
    }
}

/// Takes a tap's action when it is a seat's (a join screen's, or a suggested line of the play
/// screen's Act page). Returns whether it was.
pub fn on_action(action: &str) -> bool {
    let Some(tap) = SeatTap::from_action(action) else {
        return false;
    };
    spawn_local(on_tap(tap));
    true
}

/// Enter in the name field joins as a guest; in the code field it takes the code.
fn on_key(event: &Event) {
    let Some(key_event) = event.dyn_ref::<KeyboardEvent>() else {
        return;
    };
    if key_event.key() != "Enter" {
        return;
    }
    let field = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
        .map(|target| target.id());
    match field.as_deref() {
        Some(NAME_FIELD_ID) => spawn_local(on_tap(SeatTap::JoinGuest)),
        Some(CODE_FIELD_ID) => spawn_local(on_tap(SeatTap::EnterCode)),
        _ => {}
    }
}

/// The room code the page's link carries (`?code=`), when it is one.
fn linked_code(window: &Window) -> Option<String> {
    let query = window.location().search().ok()?;
    room_code(query_value(&query, CODE_QUERY_KEY)?)
}

/// Starts the join screens over a real phone's page, and the loop that follows the host.
pub fn start(document: &Document, window: &Window) -> Result<(), JsValue> {
    let key = kept_seat_key(window)?;
    let code = linked_code(window);
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
    let changed = Closure::<dyn FnMut(Event)>::new(|event: Event| on_change(&event));
    layer.add_event_listener_with_callback("change", changed.as_ref().unchecked_ref())?;
    let typed = Closure::<dyn FnMut(Event)>::new(|event: Event| on_input(&event));
    layer.add_event_listener_with_callback("input", typed.as_ref().unchecked_ref())?;
    let zoom = Closure::<dyn FnMut(Event)>::new(|event: Event| super::look::on_double_tap(&event));
    layer.add_event_listener_with_callback("dblclick", zoom.as_ref().unchecked_ref())?;
    let pressed = Closure::<dyn FnMut(Event)>::new(|event: Event| super::look::on_press(&event));
    layer.add_event_listener_with_callback("pointerdown", pressed.as_ref().unchecked_ref())?;
    let released = Closure::<dyn FnMut(Event)>::new(|event: Event| super::look::on_release(&event));
    layer.add_event_listener_with_callback("pointerup", released.as_ref().unchecked_ref())?;
    append(&body, &layer)?;
    SEAT.with(|seat| {
        *seat.borrow_mut() = Some(SeatPage {
            machine: SeatMachine::default(),
            key,
            layer,
            drawn: None,
            look_drawn: None,
            viewing: None,
            code,
            code_problem: None,
            name: None,
            stepping: None,
            roll_seen: RollSeen::default(),
            _listeners: [click, key_down, changed, typed, zoom, pressed, released],
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
    input_by_id(NAME_FIELD_ID).map(|field| field.value())
}

/// Whether the screen of `page` asks for the room code first: a new way in is chosen (or about to
/// be, with no seat to continue as) and no code is known.
fn needs_code(page: &SeatPage) -> bool {
    page.code.is_none()
        && match page.machine.state() {
            SeatState::WhoAreYou => page.machine.returning().is_none(),
            SeatState::Guest | SeatState::Friend | SeatState::SigningIn { .. } => true,
            _ => false,
        }
}

/// The room code field, put in the code screen's body after its words.
fn code_field(document: &Document, drawn: &Element) -> Result<(), JsValue> {
    let Some(body) = drawn.query_selector(".ph-join-body")? else {
        return Ok(());
    };
    let label = element(document, "label", "ph-join-field")?;
    append(
        &label,
        &text_element(document, "span", "ph-join-field-label", "Code")?,
    )?;
    let input = element(document, "input", "ph-join-name")?;
    for (name, value) in [
        ("id", CODE_FIELD_ID),
        ("type", "text"),
        ("autocomplete", "off"),
        ("autocapitalize", "characters"),
        ("spellcheck", "false"),
        ("enterkeyhint", "go"),
        ("placeholder", "ABC234"),
    ] {
        input.set_attribute(name, value)?;
    }
    append(&label, &input)?;
    append(&body, &label)
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
    let typed_code = input_by_id(CODE_FIELD_ID).map(|field| field.value());
    let result = with_seat(|page| -> Result<(), JsValue> {
        if typed.is_some() {
            page.name.clone_from(&typed);
        }
        let asks_code = needs_code(page);
        let mut screen = if asks_code {
            Some(code_screen(&page.machine, page.code_problem.as_deref()))
        } else {
            screen_of(&page.machine)
        };
        if let (
            Some(picture),
            SeatState::Seated {
                view: Some(view), ..
            },
        ) = (page.viewing.as_deref(), page.machine.state())
        {
            // From the play screen's header the viewer opens over the play page, alone.
            screen.get_or_insert_with(JoinScreen::default).viewer = lobby_viewer(view, picture);
        }
        let document = crate::dom::document()?;
        let phone_page = document
            .get_element_by_id(ROOT_ID)
            .ok_or_else(|| JsValue::from_str("the page has no root"))?;
        // A locked-in hero not yet submitted builds over the lobby (`pages::build`).
        if let SeatState::Seated {
            seat,
            view: Some(view),
        } = page.machine.state()
            && super::pages::shows_build(seat, view)
        {
            let scene = page.machine.room().and_then(|room| room.picture.clone());
            super::pages::draw_build(&page.layer, scene.as_deref())?;
            swap(&page.layer, &phone_page)?;
            page.drawn = None;
            page.look_drawn = None;
            return Ok(());
        }
        if let SeatState::Creation { seat } = page.machine.state() {
            let revision = super::look::revision();
            if page.drawn.as_ref() == Some(&screen) && page.look_drawn == Some(revision) {
                return Ok(());
            }
            let header = PhoneHeader {
                name: seat.name.clone(),
                subtitle: Some("Making your hero".to_owned()),
                portrait: None,
                hp: None,
            };
            let scene = page.machine.room().and_then(|room| room.picture.clone());
            super::look::draw_into(&page.layer, &header, scene.as_deref())?;
            swap(&page.layer, &phone_page)?;
            page.drawn = Some(screen);
            page.look_drawn = Some(revision);
            return Ok(());
        }
        page.look_drawn = None;
        if page.drawn.as_ref() == Some(&screen) {
            return Ok(());
        }
        clear(&page.layer);
        match &screen {
            None => swap(&phone_page, &page.layer)?,
            Some(join) => {
                let drawn = build(&document, join)?;
                if asks_code {
                    code_field(&document, &drawn)?;
                }
                append(&page.layer, &drawn)?;
                swap(&page.layer, &phone_page)?;
            }
        }
        page.drawn = Some(screen);
        Ok(())
    });
    if let Some(Err(error)) = result {
        log_error(ErrorSource::Draw, &js_message(&error));
    }
    let kept_name = with_seat(|page| page.name.clone()).flatten();
    if let (Some(name), Some(field)) = (kept_name, input_by_id(NAME_FIELD_ID)) {
        field.set_value(&name);
    }
    if let (Some(code), Some(field)) = (typed_code, input_by_id(CODE_FIELD_ID)) {
        field.set_value(&code);
    }
}

fn input_by_id(id: &str) -> Option<HtmlInputElement> {
    let document = web_sys::window()?.document()?;
    document
        .get_element_by_id(id)?
        .dyn_into::<HtmlInputElement>()
        .ok()
}

/// Takes the code typed in the code field when the host says it is the table's
/// (`path::PLAY_JOIN_CODE`); else says why not: not a room code's shape, the host's refusal, or
/// that it did not reach the table.
async fn enter_code() {
    let typed = input_by_id(CODE_FIELD_ID)
        .map(|field| field.value())
        .unwrap_or_default();
    let checked = match room_code(&typed) {
        None => Err(CODE_SHAPE_WORDS.to_owned()),
        Some(code) => post::<_, CodeTaken>(PLAY_JOIN_CODE, &CodeCheck { code: code.clone() })
            .await
            .map(|CodeTaken {}| code)
            .map_err(|error| player_words(&error)),
    };
    with_seat(|page| match checked {
        Ok(code) => {
            page.code = Some(code);
            page.code_problem = None;
        }
        Err(problem) => {
            page.code = None;
            page.code_problem = Some(problem);
        }
    });
    draw();
}

/// The room code a new way in carries now (empty when none is known: the host refuses it).
fn code_now() -> String {
    with_seat(|page| page.code.clone())
        .flatten()
        .unwrap_or_default()
}

/// Applies `event` to the machine and draws what changed.
fn apply(event: SeatEvent) {
    change_machine(|machine| {
        machine.apply(event);
    });
}

/// Changes the machine and draws what changed. Entering `creation` opens the look studio; leaving
/// it lets the studio go.
fn change_machine(change: impl FnOnce(&mut SeatMachine)) {
    let creating = || {
        with_seat(|page| matches!(page.machine.state(), SeatState::Creation { .. }))
            .unwrap_or(false)
    };
    let was_creating = creating();
    with_seat(|page| change(&mut page.machine));
    match (was_creating, creating()) {
        (false, true) => super::look::enter(),
        (true, false) => super::look::leave(),
        _ => {}
    }
    draw();
    follow_play();
}

/// The host restarted: what the phone follows is of the old run, so it asks the room, says hello
/// and reads its seat's view again, and only then draws: a phone whose key holds a seat (the host
/// seats it again by itself) goes straight back to its seated view, without the join screens in
/// between. A step that fails leaves the machine where it got to, and the poll loop goes on from
/// there.
pub(super) fn host_restarted() {
    if seat_key().is_none() {
        // Not a phone: there is no seat to follow.
        return;
    }
    spawn_local(async {
        let mut machine = SeatMachine::default();
        if let Ok(room) = get::<RoomReply>(PLAY_ROOM).await {
            machine.apply(SeatEvent::RoomLoaded(room));
        }
        if machine.poll() == Some(SeatPoll::Room)
            && let Ok(hello) = post::<_, HelloReply>(PLAY_HELLO, &HelloBody {}).await
        {
            machine.apply(SeatEvent::HelloAnswered(hello));
        }
        if machine.poll() == Some(SeatPoll::SeatView)
            && let Ok(view) = get::<SeatView>(PLAY_SEAT_VIEW).await
        {
            machine.apply(SeatEvent::ViewLoaded(view));
        }
        change_machine(|kept| *kept = machine);
    });
}

/// Draws the join layer again: the look studio changed.
pub(super) fn redraw() {
    draw();
}

/// Back from the look studio to the player's heroes.
pub(super) fn back() {
    apply(SeatEvent::Back);
}

/// Puts the seat's live story and asks on the phone's play screen when they changed: the Story
/// page through the page's own redraw, the Act page in its slot.
fn follow_play() {
    let play = with_seat(|page| match page.machine.state() {
        SeatState::Seated {
            view: Some(view), ..
        } => {
            let roll_current = page.roll_seen.still_current(view);
            play_of(view).map(|mut play| {
                if !roll_current {
                    play.act.last_roll = None;
                }
                play
            })
        }
        _ => None,
    })
    .flatten();
    let header = with_seat(|page| match page.machine.state() {
        SeatState::Seated {
            view: Some(view), ..
        } => hero_header(view),
        _ => None,
    })
    .flatten();
    let header_changed = with_app_mut(|app| {
        if app.seat_header == header {
            return false;
        }
        app.seat_header = header;
        true
    });
    if header_changed {
        redraw_part(Part::PhoneHead);
    }
    let (changed, act_changed) = with_app_mut(|app| {
        if app.seat_play == play {
            return (false, false);
        }
        let act_changed =
            app.seat_play.as_ref().map(|play| &play.act) != play.as_ref().map(|play| &play.act);
        app.seat_play = play;
        app.reshow();
        (true, act_changed)
    });
    if changed {
        render();
    }
    if act_changed {
        redraw_part(Part::PhoneChoices);
    }
}

/// Says `line` as the seat's character, as written, to the character `to` of the shown scene when
/// one is named: the answer to what the table asks it, or talk of its own (`kind`). A suggested
/// card's line carries the thing the card is about (`thing`, its id), a typed line none. Why it
/// was not taken, when it was not (`player_words` says it to the player).
///
/// Each draft has its own `said` id. A draft sent again after an answer that never came (the same
/// words, to the same character, of the same kind and card) is sent with the same body and id, so
/// the host answers it as before and the character says it once; a new draft gets a new id.
pub async fn say(
    line: &str,
    to: Option<String>,
    kind: SayKind,
    thing: Option<String>,
) -> Result<(), CallError<SeatErrorKind>> {
    // An answer names the ask the phone shows, so one to an ask the table has replaced is refused
    // (`stale_ask`), never taken as the next ask's answer.
    let action = match kind {
        SayKind::Answer => SeatAction::Answer {
            said: 0,
            ask: with_seat(|page| match page.machine.state() {
                SeatState::Seated {
                    view: Some(view), ..
                } => view.ask.as_ref().map(|asked| asked.id),
                _ => None,
            })
            .flatten(),
            text: line.to_owned(),
            to,
            thing,
        },
        SayKind::Talk => SeatAction::Talk {
            said: 0,
            text: line.to_owned(),
            to,
        },
    };
    let said = act(action, None).await;
    // What the table asks next (or that it asks nothing now) shows at once.
    poll_once().await;
    said.map(|_| ()).map_err(seat_error)
}

/// Does `action` as this seat (`path::PLAY_SEAT_ACTION`), within `timeout_ms` when it may wait
/// on work: what it came to, or why not. The action gets a new `said` id, but an action equal to
/// one whose answer has not come gets that one's id, so the host does it once.
///
/// An action that did not reach the host, whose answer was lost on the way, or that the host is
/// still doing (`in_flight`) is sent again with the same id after the backoff's wait, or at once
/// when the link is back (`frames::wake`), until the host answers it: a tap made while the link is
/// down is never dropped, and the link line says why it waits. Done or refused, it is settled; an
/// answer that cannot be read, or a request that cannot be made, is returned as it is and the
/// action stays unsettled for a tap again.
pub(super) async fn act(
    action: SeatAction,
    timeout_ms: Option<u64>,
) -> Result<SeatOutcome, CallError<SeatActionErrorKind>> {
    let action = UNSETTLED.with(|unsettled| unsettled.borrow_mut().take_up(&action, new_said_id()));
    let mut failures = 0_u32;
    loop {
        let request = Call::post(PLAY_SEAT_ACTION, &action);
        let request = match timeout_ms {
            Some(timeout_ms) => request.within(timeout_ms),
            None => request,
        };
        let answer = seat_call::<SeatOutcome, SeatActionErrorKind>(request).await;
        if !answer_lost(&answer) {
            if matches!(answer, Ok(_) | Err(CallError::Refused(_))) {
                UNSETTLED.with(|unsettled| unsettled.borrow_mut().settle(action.said()));
            }
            return answer;
        }
        failures = failures.saturating_add(1);
        wait_or_wake(retry_delay_ms(failures)).await;
    }
}

/// Whether an action's answer never came, so it is sent again: the request did not reach the
/// host, no whole answer came back, or the host is still doing it (`in_flight`).
fn answer_lost(answer: &Result<SeatOutcome, CallError<SeatActionErrorKind>>) -> bool {
    match answer {
        Err(CallError::Net(
            NetError::Unreachable { .. }
            | NetError::TimedOut { .. }
            | NetError::NotAResponse { .. }
            | NetError::Unreadable { .. },
        )) => true,
        Err(CallError::Refused(rejection)) => {
            rejection.kind == Some(SeatActionErrorKind::Seat(SeatErrorKind::InFlight))
        }
        _ => false,
    }
}

/// An action's refusal as the seat routes word it.
pub(super) fn seat_error(error: CallError<SeatActionErrorKind>) -> CallError<SeatErrorKind> {
    match error {
        CallError::Refused(rejection) => CallError::Refused(Rejection {
            kind: rejection.kind.and_then(SeatActionErrorKind::seat),
            message: rejection.message,
        }),
        CallError::Net(error) => CallError::Net(error),
        CallError::Unreadable { address, cause } => CallError::Unreadable { address, cause },
        CallError::Unwritable { address, cause } => CallError::Unwritable { address, cause },
    }
}

/// The part of an action's outcome the caller asked for, or the outcome as unreadable when the
/// host answered with another kind.
pub(super) fn outcome<T, K>(
    answer: Result<SeatOutcome, CallError<K>>,
    pick: impl FnOnce(SeatOutcome) -> Option<T>,
) -> Result<T, CallError<K>> {
    let outcome = answer?;
    let what = format!("{outcome:?}");
    pick(outcome).ok_or(CallError::Unreadable {
        address: PLAY_SEAT_ACTION.to_owned(),
        cause: format!("another outcome than asked: {what}"),
    })
}

/// A new draft's `said` id: 64 random bits, so two drafts of one seat never share one.
fn new_said_id() -> u64 {
    (u64::from(super::random_bits()) << 32) | u64::from(super::random_bits())
}

/// A seat refusal that came through another route (the look studio's): the machine follows it as
/// it follows the seat routes' own, so a key the host lost goes back to "Who are you?".
pub(super) fn seat_refused(kind: SeatErrorKind) {
    apply(SeatEvent::Refused {
        kind: Some(kind),
        message: refusal_words(Some(kind)),
    });
}

/// The player's words for a refusal of kind `kind` (the host's own sentence stays in the log).
pub(super) fn refusal_words(kind: Option<SeatErrorKind>) -> String {
    kind.map_or(SEAT_TROUBLE_WORDS, SeatErrorKind::player_words)
        .to_owned()
}

/// What the player is told when a seat route gave no value: the words for the host's refusal, or
/// that it did not reach the table.
pub(super) fn player_words(error: &CallError<SeatErrorKind>) -> String {
    match error {
        CallError::Refused(rejection) => refusal_words(rejection.kind),
        CallError::Net(_) | CallError::Unreadable { .. } | CallError::Unwritable { .. } => {
            SEAT_TROUBLE_WORDS.to_owned()
        }
    }
}

/// Asks a seat route (or a look route) as this phone's seat key: the value, or why not, noted with
/// its address. An ask that did not reach the host shows the link line; any answer ends it.
pub(super) async fn seat_call<T: DeserializeOwned, K: ErrorTag>(
    request: Call<'_>,
) -> Result<T, CallError<K>> {
    let address = request.address().to_owned();
    let Some(key) = seat_key() else {
        return Err(CallError::Unwritable {
            address,
            cause: "the phone has no seat key".to_owned(),
        });
    };
    let answer = call(request.with_seat_key(&key)).await;
    match &answer {
        Ok(_) => seat_link_back(),
        Err(CallError::Net(error)) => {
            log_error(ErrorSource::Net, &error.to_string());
            seat_link_lost(error.clone());
        }
        Err(error) => {
            seat_link_back();
            log_error(ErrorSource::Net, &format!("{address}: {error}"));
        }
    }
    answer
}

pub(super) fn seat_key() -> Option<String> {
    with_seat(|page| page.key.clone())
}

/// The seat's latest view, while the phone is seated and has one.
pub(super) fn seat_view() -> Option<SeatView> {
    with_seat(|page| match page.machine.state() {
        SeatState::Seated {
            view: Some(view), ..
        } => Some(view.as_ref().clone()),
        _ => None,
    })
    .flatten()
}

/// Reads the seat route at `address`.
pub(super) async fn get<T: DeserializeOwned>(address: &str) -> Result<T, CallError<SeatErrorKind>> {
    seat_call(Call::get(address)).await
}

/// Posts `body` to the seat route at `address`.
pub(super) async fn post<Body: Serialize, T: DeserializeOwned>(
    address: &str,
    body: &Body,
) -> Result<T, CallError<SeatErrorKind>> {
    seat_call(Call::post(address, body)).await
}

/// Applies a value as `event`, or the refusal in the player's words. `false` when the ask failed.
fn settle<T>(
    answer: Result<T, CallError<SeatErrorKind>>,
    event: impl FnOnce(T) -> SeatEvent,
) -> bool {
    match answer {
        Ok(value) => apply(event(value)),
        Err(CallError::Refused(rejection)) => apply(SeatEvent::Refused {
            kind: rejection.kind,
            message: refusal_words(rejection.kind),
        }),
        Err(CallError::Net(_) | CallError::Unreadable { .. } | CallError::Unwritable { .. }) => {
            return false;
        }
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
            let viewed = settle(get::<SeatView>(PLAY_SEAT_VIEW).await, SeatEvent::ViewLoaded);
            // A seat that plays a character has its live pages beside its view.
            if seat_view().is_some_and(|view| view.story.is_some()) {
                super::pages::poll().await;
            }
            // A seat that may build a hero has its build beside its view.
            let building = with_seat(|page| match page.machine.state() {
                SeatState::Seated {
                    seat,
                    view: Some(view),
                } => super::pages::may_build(seat, view),
                _ => false,
            });
            if building == Some(true) {
                super::pages::poll_build().await;
            }
            viewed
        }
        None => true,
    }
}

/// Polls the host, waiting longer after each failure in a row as the frame loop does; a page woken
/// (shown again, back online) asks at once and forgets its failures.
async fn run() {
    let mut failures = 0_u32;
    loop {
        failures = if poll_once().await {
            0
        } else {
            failures.saturating_add(1)
        };
        if wait_or_wake(SEAT_POLL_MS.max(retry_delay_ms(failures))).await {
            failures = 0;
        }
    }
}

/// Asks for a seat the way `request` says. A code the host refuses is forgotten, and the code
/// screen asks again with the reason.
async fn join(request: &JoinRequest) {
    let answer = post(PLAY_JOIN, request).await;
    if let Err(CallError::Refused(rejection)) = &answer
        && matches!(
            rejection.kind,
            Some(SeatErrorKind::CodeMismatch | SeatErrorKind::TooManyGuesses)
        )
    {
        let words = refusal_words(rejection.kind);
        with_seat(|page| {
            page.code = None;
            page.code_problem = Some(words);
        });
        draw();
        return;
    }
    settle(answer, SeatEvent::JoinAnswered);
}

async fn on_tap(tap: SeatTap) {
    match tap {
        SeatTap::Continue => join(&JoinRequest::Returning).await,
        SeatTap::Guest => apply(SeatEvent::ChoseGuest),
        SeatTap::Friend => apply(SeatEvent::ChoseFriend),
        SeatTap::SignIn => {
            apply(SeatEvent::ChoseSignIn);
            join(&JoinRequest::Account { code: code_now() }).await;
        }
        SeatTap::Back => apply(SeatEvent::Back),
        SeatTap::JoinGuest => {
            let name = typed_name().unwrap_or_default().trim().to_owned();
            join(&JoinRequest::Guest {
                name,
                code: code_now(),
            })
            .await;
        }
        SeatTap::PickFriend(profile) => {
            join(&JoinRequest::Friend {
                profile,
                code: code_now(),
            })
            .await;
        }
        SeatTap::EnterCode => enter_code().await,
        SeatTap::PlayHero(hero) => {
            let answer = act(SeatAction::Hero { said: 0, hero }, None).await;
            let card = outcome(answer, |done| match done {
                SeatOutcome::Hero { seat } => Some(seat),
                _ => None,
            });
            settle(card.map_err(seat_error), SeatEvent::HeroChosen);
        }
        SeatTap::MakeHero => apply(SeatEvent::MakeHero),
        SeatTap::Act(action) => {
            // A tap again on the same step while it is on its way is the same press: the screen
            // it answers is not drawn yet, and the button there is about to become another one.
            let first = with_seat(|page| {
                if page.stepping == Some(action) {
                    return false;
                }
                page.stepping = Some(action);
                true
            });
            if first != Some(true) {
                return;
            }
            // Starting the game runs the table's scenario, so a lobby action may wait on work.
            let answer = act(
                SeatAction::Lobby {
                    said: 0,
                    step: action,
                },
                Some(WORK_REQUEST_TIMEOUT_MS),
            )
            .await;
            let reply = outcome(answer, |done| match done {
                SeatOutcome::Lobby { lobby } => Some(lobby),
                _ => None,
            });
            settle(reply.map_err(seat_error), SeatEvent::IntentAnswered);
            // The screen follows the step at once: the seat's view after it, not at the next poll.
            poll_once().await;
            with_seat(|page| page.stepping = None);
        }
        SeatTap::Retry => {
            with_seat(|page| page.machine = SeatMachine::default());
            draw();
            // A failed ask is noted where it failed; the loop asks again.
            poll_once().await;
        }
        SeatTap::Roll => {
            // The tap answers the roll the phone shows; the host rolls the dice.
            let Some(call) = seat_view()
                .and_then(|view| view.pending_roll)
                .map(|roll| roll.roll_id)
            else {
                return;
            };
            let answer = act(SeatAction::Roll { said: 0, call }, None).await;
            if let Err(error) = answer {
                let message = player_words(&seat_error(error));
                log_error(
                    ErrorSource::Net,
                    &format!("the roll was not taken: {message}"),
                );
                super::voice::show_refusal(&message);
            }
            poll_once().await;
        }
        SeatTap::Say(index) => {
            let line = with_seat(|page| match page.machine.state() {
                SeatState::Seated {
                    view: Some(view), ..
                } => view
                    .ask
                    .as_ref()
                    .and_then(|ask| ask.cards.get(index))
                    .map(|card| (card.line.clone(), card.thing.clone())),
                _ => None,
            })
            .flatten();
            let Some((line, thing)) = line else {
                return;
            };
            // A refused card stays where it is, and the player is told why wherever they tapped it:
            // under the Act page's composer and on the live page it came from.
            let said = say(&line, None, SayKind::Answer, thing).await;
            let problem = said.err().map(|error| player_words(&error));
            if let Some(message) = &problem {
                log_error(
                    ErrorSource::Net,
                    &format!("the line was not taken: {message}"),
                );
                super::voice::show_refusal(message);
            }
            super::pages::note_problem(problem);
        }
    }
}

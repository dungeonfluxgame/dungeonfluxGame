//! A real phone's look studio (P3): shown in the join layer while the seat machine is in
//! `creation` (after "Make a new hero"), it asks the host's look routes as the seat's key and draws
//! `look::studio::LookPage`. Every result comes from the host; the phone keeps only the draft, in
//! the browser's storage so a refresh keeps it (REF-049), and its own picks. While it shows, it
//! reads the studio again every `SEAT_POLL_MS`, so a picture being made arrives by itself. A failed
//! ask keeps the screen as it is and is noted.

use std::cell::{Cell, RefCell};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::Element;

use super::frames::wait_ms;
use super::state::log_error;
use crate::dom::{append, clear, js_message};
use crate::limits::{SEAT_POLL_MS, WORK_REQUEST_TIMEOUT_MS};
use crate::look::studio::{LookPage, LookPhase, LookTap, NAME_FIELD_ID, WORDS_FIELD_ID};
use crate::look::wire::{
    LookAgainAsk, LookControls, LookDetails, LookDetailsAsk, LookDraft, LookErrorKind, LookLockAsk,
    LookRefineAsk, LookRollAsk, LookStartAsk, LookStartOptions, LookStudioView, LookVersion,
    LookVersionAsk,
};
use crate::net::{Call, CallError};
use crate::path::{
    PLAY_LOOK, PLAY_LOOK_AGAIN, PLAY_LOOK_CONTROLS, PLAY_LOOK_DETAILS, PLAY_LOOK_DISCARD,
    PLAY_LOOK_KEEP, PLAY_LOOK_LOCK, PLAY_LOOK_REFINE, PLAY_LOOK_ROLL, PLAY_LOOK_START,
};
use crate::phone_kit::shell::PhoneHeader;
use crate::timeline::ErrorSource;

/// Where the phone keeps its look draft in the browser's storage.
const DRAFT_STORAGE: &str = "df-phone-look-draft";

thread_local! {
    static LOOK: RefCell<LookPage> = RefCell::new(LookPage::default());
    /// Counts the changes of the page, so the seat layer redraws only what changed.
    static REVISION: Cell<u64> = const { Cell::new(0) };
    /// The phase last drawn, so the stage is brought into view once when a picture starts or
    /// lands, not on every redraw.
    static SHOWN_PHASE: Cell<Option<LookPhase>> = const { Cell::new(None) };
    /// Whether the studio shows now; its loop stops when it does not.
    static SHOWN: Cell<bool> = const { Cell::new(false) };
}

/// The page's change count.
pub(super) fn revision() -> u64 {
    REVISION.with(Cell::get)
}

fn change(act: impl FnOnce(&mut LookPage)) {
    LOOK.with(|page| act(&mut page.borrow_mut()));
    REVISION.with(|revision| revision.set(revision.get().wrapping_add(1)));
    super::seat::redraw();
}

fn read<T>(look: impl FnOnce(&LookPage) -> T) -> T {
    LOOK.with(|page| look(&page.borrow()))
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// A draft as the browser keeps it: with the hero it is for, so another hero never takes it.
#[derive(Serialize, Deserialize)]
struct KeptDraft {
    hero: u64,
    draft: LookDraft,
}

/// The draft the browser keeps for `hero`; an empty one for any other hero.
fn kept_draft(hero: u64) -> LookDraft {
    storage()
        .and_then(|storage| storage.get_item(DRAFT_STORAGE).ok().flatten())
        .and_then(|text| serde_json::from_str::<KeptDraft>(&text).ok())
        .filter(|kept| kept.hero == hero)
        .map(|kept| kept.draft)
        .unwrap_or_default()
}

/// Keeps the page's draft in the browser for its hero (none until the host has named the hero).
fn keep_draft(page: &LookPage) {
    let Some(controls) = &page.controls else {
        return;
    };
    let kept = KeptDraft {
        hero: controls.hero,
        draft: page.draft.clone(),
    };
    let (Some(storage), Ok(text)) = (storage(), serde_json::to_string(&kept)) else {
        return;
    };
    if let Err(error) = storage.set_item(DRAFT_STORAGE, &text) {
        log_error(
            ErrorSource::Asset,
            &format!("could not keep the look draft: {}", js_message(&error)),
        );
    }
}

/// What an ask of a look route came back with.
type Answer<T> = Result<T, CallError<LookErrorKind>>;

/// Reads the look route at `address`.
async fn get<T: DeserializeOwned>(address: &str) -> Answer<T> {
    super::seat::seat_call(Call::get(address)).await
}

/// Posts `body` to the look route at `address`. A look's change may wait on a model or a picture,
/// so it is given the time of work.
async fn post<Body: Serialize, T: DeserializeOwned>(address: &str, body: &Body) -> Answer<T> {
    super::seat::seat_call(Call::post(address, body).within(WORK_REQUEST_TIMEOUT_MS)).await
}

/// The value, or `None` with the refusal shown (and the studio read again when it moved on).
async fn value<T>(answer: Answer<T>) -> Option<T> {
    match answer {
        Ok(value) => Some(value),
        Err(CallError::Refused(rejection)) => {
            let kind = rejection.kind;
            if let Some(LookErrorKind::Seat(seat_kind)) = kind {
                // The seat itself refused (the host lost its key, say): the player reads the
                // seat's words, and the seat machine follows it as a seat route's refusal.
                change(|page| page.notice = Some(super::seat::refusal_words(Some(seat_kind))));
                super::seat::seat_refused(seat_kind);
                return None;
            }
            let notice = if kind == Some(LookErrorKind::StudioUnavailable) {
                "The creator is not available right now.".to_owned()
            } else {
                rejection.message
            };
            change(|page| page.notice = Some(notice));
            match kind {
                Some(LookErrorKind::KeptChanged) => refresh().await,
                // The seat has no hero (it changed hands, say): the phone starts a new one.
                Some(LookErrorKind::NoHero) => spawn_local(start_form()),
                _ => {}
            }
            None
        }
        Err(CallError::Net(_) | CallError::Unreadable { .. } | CallError::Unwritable { .. }) => {
            change(|page| page.notice = Some("The host could not be reached.".to_owned()));
            None
        }
    }
}

/// Reads the studio again; draws it when it changed.
async fn refresh() {
    if let Ok(studio) = get::<LookStudioView>(PLAY_LOOK).await
        && read(|page| page.studio.as_ref() != Some(&studio))
    {
        change(|page| page.studio = Some(studio));
    }
}

/// Opens the studio as the seat machine enters `creation`: a look in the making goes on, anything
/// else starts a new hero. Starts the loop that follows the host.
pub(super) fn enter() {
    change(|page| *page = LookPage::default());
    spawn_local(open());
    if SHOWN.with(|shown| shown.replace(true)) {
        return;
    }
    spawn_local(async {
        while SHOWN.with(Cell::get) {
            wait_ms(SEAT_POLL_MS).await;
            if SHOWN.with(Cell::get) {
                refresh().await;
            }
        }
    });
}

/// The seat machine left `creation`.
pub(super) fn leave() {
    SHOWN.with(|shown| shown.set(false));
}

async fn open() {
    let Some(studio) = value(get::<LookStudioView>(PLAY_LOOK).await).await else {
        return;
    };
    let going_on = studio.lock.is_none() && !studio.versions.is_empty();
    change(|page| page.studio = Some(studio));
    if going_on {
        load_controls().await;
    } else {
        start_form().await;
    }
}

/// The start form of a new hero: what it may be started as.
async fn start_form() {
    if let Some(options) = value(get::<LookStartOptions>(PLAY_LOOK_START).await).await {
        change(|page| {
            page.controls = None;
            page.options = Some(options);
        });
    }
}

/// The controls of the seat's hero, and the draft the browser kept for that hero.
async fn load_controls() {
    if let Some(controls) = value(get::<LookControls>(PLAY_LOOK_CONTROLS).await).await {
        change(|page| {
            page.draft = kept_draft(controls.hero);
            page.controls = Some(controls);
        });
    }
}

fn field(id: &str) -> Option<Element> {
    web_sys::window()?.document()?.get_element_by_id(id)
}

/// The value of the input or text area `id`, read as its `value` property (both have one).
fn field_value(id: &str) -> Option<String> {
    let element: JsValue = field(id)?.into();
    let value = js_sys::Reflect::get(&element, &JsValue::from_str("value")).ok()?;
    value.as_string()
}

fn set_field(id: &str, text: &str) {
    let Some(element) = field(id) else {
        return;
    };
    // A field that will not take the text keeps what it shows; nothing depends on it.
    let _ = js_sys::Reflect::set(
        &element,
        &JsValue::from_str("value"),
        &JsValue::from_str(text),
    );
}

/// Sends what a tap asks of the host, then sets the page from the answer.
async fn ask(tap: LookTap) {
    change(|page| {
        page.sending = true;
        page.notice = None;
    });
    match tap {
        LookTap::ReadDetails => {
            let words = field_value(WORDS_FIELD_ID).unwrap_or_default();
            let read =
                value(post::<_, LookDetails>(PLAY_LOOK_DETAILS, &LookDetailsAsk { words }).await)
                    .await;
            if let Some(read) = read {
                change(|page| {
                    page.apply_details(read);
                    keep_draft(page);
                });
                set_field(WORDS_FIELD_ID, "");
            }
        }
        LookTap::Refine => {
            // A new roll renders a new person from the base pose; otherwise the kept look is
            // edited with only what changed.
            let ask = read(|page| LookRefineAsk {
                draft: page.draft.clone(),
                kept: page
                    .studio
                    .as_ref()
                    .and_then(|studio| studio.kept)
                    .filter(|_| !page.fresh),
            });
            let made = value(post::<_, LookVersion>(PLAY_LOOK_REFINE, &ask).await).await;
            if let Some(version) = made {
                // The version's draft has every control the player left unset rolled: show it.
                change(|page| {
                    page.draft = version.draft;
                    page.fresh = false;
                    keep_draft(page);
                });
                refresh().await;
            }
        }
        LookTap::Roll => {
            if let Some(draft) =
                value(post::<_, LookDraft>(PLAY_LOOK_ROLL, &LookRollAsk {}).await).await
            {
                change(|page| {
                    page.fresh = page
                        .studio
                        .as_ref()
                        .is_some_and(|studio| studio.kept.is_some());
                    page.draft = draft;
                    keep_draft(page);
                });
            }
        }
        LookTap::Keep(version) => {
            let ask = LookVersionAsk { version };
            settle(post(PLAY_LOOK_KEEP, &ask).await, Some(version)).await;
        }
        LookTap::KeepOld(version) => {
            let ask = LookVersionAsk { version };
            let back_to = read(|page| page.studio.as_ref().and_then(|studio| studio.kept));
            settle(post(PLAY_LOOK_DISCARD, &ask).await, back_to).await;
        }
        LookTap::StartAgain => {
            settle(post(PLAY_LOOK_AGAIN, &LookAgainAsk {}).await, None).await;
        }
        LookTap::LockIn => {
            let ask = read(|page| LookLockAsk {
                draft: page.draft.clone(),
            });
            settle(post(PLAY_LOOK_LOCK, &ask).await, None).await;
        }
        LookTap::Start => {
            let ask = read(|page| LookStartAsk {
                name: page.name.trim().to_owned(),
                species: page.species.clone().unwrap_or_default(),
                class: page.class.clone().unwrap_or_default(),
            });
            if let Some(studio) =
                value(post::<_, LookStudioView>(PLAY_LOOK_START, &ask).await).await
            {
                change(|page| {
                    page.studio = Some(studio);
                    page.options = None;
                    page.draft = LookDraft::default();
                    keep_draft(page);
                });
                load_controls().await;
            }
        }
        _ => {}
    }
    change(|page| page.sending = false);
}

/// Sets the studio from an answer; the draft becomes the one of version `draft_of` when it has one
/// (a version kept, or gone back to).
async fn settle(answer: Answer<LookStudioView>, draft_of: Option<u64>) {
    let Some(studio) = value(answer).await else {
        return;
    };
    change(|page| {
        if let Some(version) = studio
            .versions
            .iter()
            .find(|version| Some(version.id) == draft_of)
        {
            page.draft = version.draft.clone();
            page.fresh = false;
            keep_draft(page);
        }
        page.studio = Some(studio);
    });
}

/// The new hero's name as typed. The page is drawn again only when Start's readiness changes; the
/// field keeps its focus through that (`draw_into`).
pub(super) fn typed_name(name: String) {
    let flips = read(|page| page.name.trim().is_empty() != name.trim().is_empty());
    if flips {
        change(|page| page.name = name);
    } else {
        LOOK.with(|page| page.borrow_mut().name = name);
    }
}

/// Takes a tap's action when it is the look studio's. Returns whether it was.
pub(super) fn on_action(action: &str) -> bool {
    let Some(tap) = LookTap::from_action(action) else {
        return false;
    };
    // An empty details box asks nothing: details are never required.
    if tap == LookTap::ReadDetails
        && field_value(WORDS_FIELD_ID).is_none_or(|words| words.trim().is_empty())
    {
        return true;
    }
    if let LookTap::TakeSeat(hero) = tap {
        leave();
        super::seat::back();
        super::seat::dispatch(&crate::seat::screen::SeatTap::PlayHero(hero).action());
        return true;
    }
    if tap == LookTap::Done {
        leave();
        super::seat::back();
        return true;
    }
    let mut local = false;
    change(|page| {
        local = page.apply_local(&tap);
        if local {
            keep_draft(page);
        }
    });
    if !local && !read(|page| page.sending) {
        spawn_local(ask(tap));
    }
    true
}

/// Draws the studio into `layer`, keeping what was typed and where the body was scrolled.
pub(super) fn draw_into(
    layer: &Element,
    header: &PhoneHeader,
    scene: Option<&str>,
) -> Result<(), JsValue> {
    let words = field_value(WORDS_FIELD_ID);
    let name = field_value(NAME_FIELD_ID);
    let focused = crate::dom::document()?
        .active_element()
        .map(|element| element.id())
        .filter(|id| id == NAME_FIELD_ID || id == WORDS_FIELD_ID);
    let scrolled = layer
        .query_selector(".ph-look-body")?
        .map(|body| body.scroll_top());
    let document = crate::dom::document()?;
    let screen = read(LookPage::screen);
    let phase = read(LookPage::phase);
    let shown = SHOWN_PHASE.with(|last| last.replace(Some(phase)));
    let to_stage =
        shown != Some(phase) && matches!(phase, LookPhase::Refining(_) | LookPhase::Comparing(_));
    clear(layer);
    append(
        layer,
        &crate::view::look::build(&document, &screen, header, scene)?,
    )?;
    if let Some(words) = words {
        set_field(WORDS_FIELD_ID, &words);
    }
    if let Some(name) = name {
        set_field(NAME_FIELD_ID, &name);
    }
    if let (Some(top), Some(body)) = (scrolled, layer.query_selector(".ph-look-body")?) {
        body.set_scroll_top(top);
    }
    if to_stage
        && screen.viewer.is_none()
        && let Some(stage) = layer.query_selector(".ph-stage-wrap, .ph-compare")?
    {
        stage.scroll_into_view();
    }
    if let Some(element) = focused.and_then(|id| field(&id)) {
        // The field typed in before the redraw keeps the focus, so typing goes on.
        if let Ok(field) = element.dyn_into::<web_sys::HtmlElement>() {
            field.focus()?;
        }
    }
    Ok(())
}

thread_local! {
    /// Where a press on the viewer's picture began, across the screen, for a swipe.
    static PRESSED_AT: Cell<Option<f64>> = const { Cell::new(None) };
}

/// How far a press must travel across the viewer's picture to step to the next or previous one.
const SWIPE_PX: f64 = 60.0;

/// The viewer's frame `event` happened in, if any.
fn viewer_frame(event: &web_sys::Event) -> Option<Element> {
    let target = event.target()?.dyn_into::<Element>().ok()?;
    target
        .closest(&format!(".{}", crate::view::look::VIEWER_FRAME_CLASS))
        .ok()?
}

fn client_x(event: &web_sys::Event) -> Option<f64> {
    js_sys::Reflect::get(event, &JsValue::from_str("clientX"))
        .ok()?
        .as_f64()
}

/// Whether the viewer's frame is zoomed in (its class names are the viewer's own).
fn is_zoomed(frame: &Element) -> bool {
    super::events::has_class(frame, crate::view::look::ZOOMED_CLASS)
}

/// A double tap on the viewer's picture zooms it in, or back out: presentation only.
pub(super) fn on_double_tap(event: &web_sys::Event) {
    if let Some(frame) = viewer_frame(event) {
        let zoomed = is_zoomed(&frame);
        let base = crate::view::look::VIEWER_FRAME_CLASS;
        let class = if zoomed {
            base.to_owned()
        } else {
            format!("{base} {}", crate::view::look::ZOOMED_CLASS)
        };
        frame.set_class_name(&class);
    }
}

/// A press on the viewer's picture: where a swipe starts.
pub(super) fn on_press(event: &web_sys::Event) {
    let start = viewer_frame(event).and_then(|_| client_x(event));
    PRESSED_AT.with(|pressed| pressed.set(start));
}

/// A press let go: a swipe across the unzoomed picture steps to the next or previous one.
pub(super) fn on_release(event: &web_sys::Event) {
    let Some(start) = PRESSED_AT.with(|pressed| pressed.take()) else {
        return;
    };
    let zoomed = viewer_frame(event).is_some_and(|frame| is_zoomed(&frame));
    let Some(end) = client_x(event).filter(|_| !zoomed) else {
        return;
    };
    // The viewer's own step control says where a step goes, in the studio and in the lobby.
    let control = match end - start {
        moved if moved <= -SWIPE_PX => ".ph-viewer-next",
        moved if moved >= SWIPE_PX => ".ph-viewer-previous",
        _ => return,
    };
    let action = crate::dom::document()
        .ok()
        .and_then(|document| document.query_selector(control).ok().flatten())
        .and_then(|step| step.get_attribute("data-action"));
    if let Some(action) = action {
        super::seat::dispatch(&action);
    }
}

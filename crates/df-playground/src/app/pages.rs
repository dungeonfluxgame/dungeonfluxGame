//! A seated phone's live pages in play (`pages`): it asks the host's seat pages route beside each
//! seat view, keeps what the player has open (whom they speak to, the drafts, the Notebook's
//! filter), draws the page the kit's navigation is on (`phone_kit::draw::nav`) and sends the taps.
//!
//! A tap that answers the table (a way on the map, a thing to step up to) is the seat's own
//! suggested card (`seat::screen::SeatTap::Say`), so it takes the Act page's one path; a line from
//! Talk is the seat's line (`seat::say`); a note is the host's to keep. A newly painted study opens
//! on the phone once; the day's show opens the paintings. Nothing here decides a rule.
//!
//! Before play, a seat whose look is locked in builds its hero over the lobby (`pages::build`):
//! the build is begun once as a quick hero, each tap sends a pick, and I'm ready submits it, which
//! is the seat's Ready in the lobby.

use std::cell::RefCell;

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlInputElement};

use super::seat::{act, outcome, player_words, refusal_words, seat_call, seat_error, seat_view};
use super::state::try_with_app;
use crate::clues::ClueBoard;
use crate::composer::Target;
use crate::limits::WORK_REQUEST_TIMEOUT_MS;
use crate::media::{MediaKind, media_kind};
use crate::net::{Call, CallError, NetError};
use crate::pages::build::{BuildInput, BuildTap, BuildUi, build_panels};
use crate::pages::creation::{
    BuildChoose, BuildPick, BuildState, BuildStep, CreationErrorKind, CreationView,
};
use crate::pages::screens::{
    ClueViewer, GALLERY_PAGE, LookCard, NOTE_FIELD_ID, PageTap, PagesInput, PagesUi, ROOM_LABEL,
    TALK_FIELD_ID, TALK_PAGE, TableWaits, WayCard, clue_viewer, new_painting, newest_seq,
    painting_page, panels, speakable_of,
};
use crate::pages::sheet::{RollShown, check_label, roll_of};
use crate::pages::wire::{BagAct, NoteAsk, PagesErrorKind, SeatPages};
use crate::path::{
    PLAY_CREATION, PLAY_CREATION_CHOOSE, PLAY_CREATION_SUBMIT, PLAY_CREATION_SURPRISE,
    PLAY_SEAT_PAGES, PLAY_SEAT_ROLL,
};
use crate::phone_kit::PhonePanels;
use crate::phone_kit::draw::nav;
use crate::phone_kit::nav::page_action;
use crate::seat::screen::SeatTap;
use crate::seat::wire::{
    AskKind, SayKind, SeatAction, SeatActionErrorKind, SeatAsk, SeatCard, SeatCheck, SeatErrorKind,
    SeatOutcome, SeatRoll, SeatRolled, SeatStt, SeatView, TableStage,
};
use crate::speaker_card::{Portrait, portrait_crop};

/// The line Talk opens with when the player picks a keeper to trade with; theirs to change.
const TRADE_LINE: &str = "I'd like to buy or trade with you.";

#[derive(Default)]
struct Pages {
    pages: Option<SeatPages>,
    ui: PagesUi,
    /// The newest memory the phone has seen; none until the first answer.
    seen: Option<u64>,
    /// Whether the day's show was up at the last answer.
    showing: bool,
    /// The pages changed while the player was typing: drawn once they leave the field.
    stale: bool,
    /// The seat's talking by voice, as the host last said.
    stt: SeatStt,
    /// What the table waits on from the seat, as its view last said.
    waits: TableWaits,
    /// The roll the shared screen showed at the last answer, by id: Act shows the seat's last roll
    /// in full only while it is up.
    tv_roll: Option<String>,
}

thread_local! {
    static PAGES: RefCell<Pages> = RefCell::new(Pages::default());
}

fn with_pages<T>(act: impl FnOnce(&mut Pages) -> T) -> T {
    PAGES.with(|pages| act(&mut pages.borrow_mut()))
}

/// The ways to go of `ask`, when it asks where the hero goes next.
fn ways_of(ask: Option<&SeatAsk>) -> Vec<WayCard> {
    ask.filter(|ask| ask.kind == AskKind::Move)
        .map(|ask| {
            ask.cards
                .iter()
                .enumerate()
                .map(|(index, card)| WayCard {
                    index,
                    label: card.label.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The things to step up to of `ask`, when it asks what the hero looks at closely.
fn looks_of(ask: Option<&SeatAsk>) -> Vec<LookCard> {
    ask.filter(|ask| ask.kind == AskKind::Look)
        .map(|ask| {
            ask.cards
                .iter()
                .enumerate()
                .filter_map(|(index, card)| {
                    card.thing.clone().map(|thing| LookCard { index, thing })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The faces of the people here cut from the room picture the shared screen shows, as its speaker
/// cards cut them: each by the region its scan has for them (`speaker_card::portrait_crop`).
fn faces_of(pages: &crate::pages::wire::SeatPages) -> Vec<(String, Portrait)> {
    try_with_app(|app| {
        let Some(Ok(fixture)) = app.shown.as_deref() else {
            return Vec::new();
        };
        let Some(layout) = fixture.tv.cast_layout() else {
            return Vec::new();
        };
        let picture = layout.picture();
        if media_kind(&picture.name) != Some(MediaKind::Image) {
            return Vec::new();
        }
        speakable_of(pages)
            .into_iter()
            .filter_map(|person| {
                let region = layout.region(&person.id)?;
                Some((
                    person.id.clone(),
                    Portrait {
                        media: picture.name.clone(),
                        crop: portrait_crop(region, picture),
                    },
                ))
            })
            .collect()
    })
    .unwrap_or_default()
}

/// The clue board the shared screen shows now, and the clue it shows full screen, by id.
fn table_clues() -> (Option<ClueBoard>, Option<String>) {
    try_with_app(|app| match app.shown.as_deref() {
        Some(Ok(fixture)) => (fixture.tv.clues.clone(), fixture.tv.shown_clue.clone()),
        _ => (None, None),
    })
    .unwrap_or_default()
}

/// The panels of the live page `page` from the last answer and the seat's view, or `None` when it
/// is not a live page or nothing has come yet.
pub fn live_panels(page: &str) -> Option<PhonePanels> {
    let view: SeatView = seat_view()?;
    let story = view.story.as_ref()?;
    let mic = super::voice::talk_mic(seat_stt());
    let (clues, on_table) = table_clues();
    with_pages(|state| {
        let pages = state.pages.as_ref()?;
        let ask = view.ask.as_ref();
        let (ways, looks) = (ways_of(ask), looks_of(ask));
        panels(
            page,
            &PagesInput {
                pages,
                hero: &story.hero,
                lines: &story.lines,
                faces: &faces_of(pages),
                ways: &ways,
                looks: &looks,
                talk_waiting: view.talk_waiting,
                mic,
                called: view.pending_roll.as_deref(),
                clues: clues.as_ref(),
                on_table: on_table.as_deref(),
                ui: &state.ui,
            },
        )
    })
}

/// The seat's last roll as the shared screen showed it, for the full card on Roll.
pub fn last_roll() -> Option<crate::fixture::RollView> {
    try_with_app(|app| {
        app.seat_play
            .as_ref()
            .and_then(|play| play.act.last_roll.clone())
    })
    .flatten()
}

/// The clue open full screen on the phone, while the phone is on the Clues page and the clue is
/// still on the shared screen's board.
fn open_clue(on_clues_page: bool) -> Option<ClueViewer> {
    let open = with_pages(|state| state.ui.clue.clone()).filter(|_| on_clues_page)?;
    let (board, on_table) = table_clues();
    let problem = with_pages(|state| state.ui.problem.clone());
    clue_viewer(&board?, &open, on_table.as_deref(), problem.as_deref())
}

/// The attribute of the clue viewer that says what it shows, so an unchanged viewer is left as it
/// is (zoomed, say) when the page is drawn again.
const CLUE_VIEWER_MARK: &str = "data-clue-viewer";

thread_local! {
    /// The clue viewer's gestures, made once: a double tap zooms, a swipe steps (`look`).
    static CLUE_GESTURES: [wasm_bindgen::prelude::Closure<dyn FnMut(web_sys::Event)>; 3] = [
        wasm_bindgen::prelude::Closure::new(|event: web_sys::Event| {
            super::look::on_double_tap(&event);
        }),
        wasm_bindgen::prelude::Closure::new(|event: web_sys::Event| super::look::on_press(&event)),
        wasm_bindgen::prelude::Closure::new(|event: web_sys::Event| {
            super::look::on_release(&event);
        }),
    ];
}

/// Draws the clue open full screen over the live phone's `screen` (the look studio's viewer, with
/// what was found of the clue), or takes it away when none is open. A viewer on the same clue,
/// buttons and words is left as it stands.
pub fn paint_clue_viewer(
    document: &web_sys::Document,
    screen: &Element,
    on_clues_page: bool,
) -> Result<(), JsValue> {
    let open = open_clue(on_clues_page);
    let mark = open.as_ref().map(|open| format!("{open:?}"));
    let shown = screen.query_selector(&format!("[{CLUE_VIEWER_MARK}]"))?;
    if let Some(old) = &shown {
        if old.get_attribute(CLUE_VIEWER_MARK) == mark {
            return Ok(());
        }
        old.remove();
    }
    let (Some(open), Some(mark)) = (open, mark) else {
        return Ok(());
    };
    let overlay = crate::view::look::viewer(document, &open.viewer)?;
    overlay.set_attribute("class", "ph-viewer ph-clue-viewer")?;
    overlay.set_attribute(CLUE_VIEWER_MARK, &mark)?;
    if shown.is_some() {
        // A step or a new button replaces the viewer in place, without its entrance again.
        overlay.set_attribute("data-redrawn", "")?;
    }
    let words = crate::dom::element(document, "div", "ph-clue-words")?;
    if !open.text.is_empty() {
        crate::dom::append(
            &words,
            &crate::dom::text_element(document, "p", "ph-clue-text", &open.text)?,
        )?;
    }
    if let Some(problem) = &open.problem {
        crate::dom::append(
            &words,
            &crate::dom::text_element(document, "p", "ph-clue-problem", problem)?,
        )?;
    }
    match overlay.query_selector(".ph-viewer-dock")? {
        Some(dock) => overlay.insert_before(&words, Some(&dock))?,
        None => overlay.append_child(&words)?,
    };
    CLUE_GESTURES.with(|[double_tap, press, release]| {
        overlay
            .add_event_listener_with_callback("dblclick", double_tap.as_ref().unchecked_ref())?;
        overlay.add_event_listener_with_callback("pointerdown", press.as_ref().unchecked_ref())?;
        overlay.add_event_listener_with_callback("pointerup", release.as_ref().unchecked_ref())
    })?;
    crate::dom::append(screen, &overlay)
}

/// Shows the clue `clue` on the shared screen, or takes the shown one off it (`None`), as the
/// seat; a refusal is shown in the viewer and on the Clues page.
async fn send_clue(clue: Option<String>) {
    let action = match clue {
        Some(clue) => SeatAction::ShowClue { said: 0, clue },
        None => SeatAction::CloseClue { said: 0 },
    };
    let problem = problem_of(act(action, None).await);
    with_pages(|state| state.ui.problem = problem);
    repaint();
}

/// The player's words for why the table did not take an action, or none when it did.
fn problem_of(answer: Result<SeatOutcome, CallError<SeatActionErrorKind>>) -> Option<String> {
    match answer {
        Ok(_) => None,
        // The rules' own reason a cast was refused.
        Err(CallError::Refused(rejection)) => Some(match rejection.kind {
            Some(SeatActionErrorKind::Cast(kind)) => kind.player_words().to_owned(),
            kind => pages_words(kind.map(SeatActionErrorKind::pages)),
        }),
        Err(_) => Some("The table could not be reached. Try again.".to_owned()),
    }
}

/// Casts the spell `spell` at `level`, as a ritual when `as_ritual`, as the seat: the host checks
/// the spell and the slot, spends it and hands the cast to the dungeon master, which may take a
/// while. A refusal is shown on the Hero page; the slots left follow from the next answer.
async fn send_cast(spell: String, level: u8, as_ritual: bool) {
    let action = SeatAction::Cast {
        said: 0,
        spell,
        level,
        as_ritual,
    };
    let problem = problem_of(act(action, Some(WORK_REQUEST_TIMEOUT_MS)).await);
    with_pages(|state| state.ui.problem = problem);
    poll().await;
    repaint();
}

/// Buys `qty` of `item` at the shop `shop`, as the seat: the host moves the coins and the goods
/// together or not at all, and refuses with a reason (no room, past the capacity, short of coin).
/// The refusal shows on the shop's page; the purse and the load follow from the next answer.
async fn send_buy(shop: String, item: String, qty: u32) {
    let action = SeatAction::Buy {
        said: 0,
        shop,
        item,
        qty,
    };
    let problem = problem_of(act(action, None).await);
    with_pages(|state| state.ui.problem = problem);
    poll().await;
    repaint();
}

/// What the table waits on from the seat, as its view last said: for the tab bar's dots and the
/// line on the other pages.
pub fn table_waits() -> TableWaits {
    with_pages(|state| state.waits.clone())
}

/// Whether the seat's pages, as last read, carry paintings (`screens::has_paintings`).
// TODO(DEV-12): test: the More menu lists Paintings with a kept painting or the show up, and omits
// it without.
pub fn has_paintings() -> bool {
    with_pages(|state| {
        state
            .pages
            .as_ref()
            .is_some_and(crate::pages::screens::has_paintings)
    })
}

/// The shops here as cards for the Act page, each opening its page; none before the first answer.
pub fn shop_cards() -> Vec<crate::phone_kit::page::PickCard> {
    with_pages(|state| {
        state
            .pages
            .as_ref()
            .map(crate::pages::screens::shop_cards)
            .unwrap_or_default()
    })
}

/// The id of the room's entry in the composer's picker: the line is said to no one in particular.
pub(super) const ROOM_TARGET: &str = "";

/// Whom the player speaks to, for the composer's picker: the room first, then the people here by
/// the names the character has learned. The seat's own character is never one of them.
pub(super) fn targets() -> Vec<Target> {
    let mut targets = vec![Target {
        id: ROOM_TARGET.to_owned(),
        name: ROOM_LABEL.to_owned(),
    }];
    with_pages(|state| {
        if let Some(pages) = &state.pages {
            targets.extend(speakable_of(pages).into_iter().map(|person| Target {
                id: person.id.clone(),
                name: person.name.clone(),
            }));
        }
    });
    targets
}

/// The seat's talking by voice, as the host last said: its switch, and whether the table writes
/// speech down at all.
pub(super) fn seat_stt() -> SeatStt {
    with_pages(|state| state.stt)
}

/// Puts the words the host wrote down from the player's recording in the Talk field, after what
/// is there: theirs to read and change before they say it.
pub(super) fn take_words(text: &str) {
    let current =
        typed(TALK_FIELD_ID).unwrap_or_else(|| with_pages(|state| state.ui.talk_draft.clone()));
    let words = if current.trim().is_empty() {
        text.to_owned()
    } else {
        format!("{} {text}", current.trim_end())
    };
    if let Some(input) = input_of(TALK_FIELD_ID) {
        input.set_value(&words);
    }
    with_pages(|state| {
        state.ui.talk_draft = words;
        state.ui.problem = None;
    });
    repaint();
}

/// The player speaks to `id` on the Talk page (the room's id for no one in particular); the Act
/// composer keeps its own choice.
pub(super) fn address(id: &str) {
    let person = Some(id.to_owned()).filter(|id| id != ROOM_TARGET);
    with_pages(|state| state.ui.addressing = person);
    repaint();
}

fn input_of(id: &str) -> Option<HtmlInputElement> {
    crate::dom::document()
        .ok()?
        .get_element_by_id(id)?
        .dyn_into::<HtmlInputElement>()
        .ok()
}

/// What the player has typed in the field `id`, when it is on the page.
fn typed(id: &str) -> Option<String> {
    input_of(id).map(|input| input.value())
}

/// Whether the player is typing in one of the pages' fields.
fn typing() -> bool {
    let focused = crate::dom::document()
        .ok()
        .and_then(|document| document.active_element())
        .map(|element| element.id());
    focused.is_some_and(|id| id == TALK_FIELD_ID || id == NOTE_FIELD_ID)
}

/// Keeps what the fields hold, then draws the open page again; while the player types, it waits.
fn repaint() {
    if typing() {
        with_pages(|state| state.stale = true);
        return;
    }
    with_pages(|state| {
        state.stale = false;
        if let Some(text) = typed(TALK_FIELD_ID) {
            state.ui.talk_draft = text;
        }
        if let Some(text) = typed(NOTE_FIELD_ID) {
            state.ui.note_draft = text;
        }
    });
    nav::refresh();
}

/// Asks the host for the seat's pages once, beside the seat's view, and draws them when they
/// changed. A new painting opens on the phone; the start of the show opens the paintings.
pub(super) async fn poll() {
    let pages = match seat_call::<SeatPages, PagesErrorKind>(Call::get(PLAY_SEAT_PAGES)).await {
        Ok(pages) => pages,
        // A refusal is the seat's (it plays no character yet): there is nothing to show.
        Err(_) => return,
    };
    let view = seat_view();
    let stt = view.as_ref().map(|view| view.stt).unwrap_or_default();
    let waits = view.as_ref().map(TableWaits::of).unwrap_or_default();
    let (changed, open, stale, shops_changed) = with_pages(|state| {
        let changed =
            state.pages.as_ref() != Some(&pages) || state.stt != stt || state.waits != waits;
        state.waits = waits;
        let shops_changed = state
            .pages
            .as_ref()
            .is_none_or(|known| known.shops != pages.shops);
        state.stt = stt;
        let mut open = None;
        if let Some(seen) = state.seen
            && let Some(painting) = new_painting(&pages, seen)
        {
            open = Some(painting_page(&painting.id));
        }
        if pages.show && !state.showing {
            open = Some(GALLERY_PAGE.to_owned());
        }
        state.seen = Some(newest_seq(&pages));
        state.showing = pages.show;
        state.pages = Some(pages);
        (changed, open, state.stale, shops_changed)
    });
    if changed {
        super::voice::refresh();
    }
    // Act shows the seat's last roll in full only while the shared screen shows it.
    let tv_roll = try_with_app(|app| match app.shown.as_deref() {
        Some(Ok(fixture)) => fixture.tv.roll.as_ref().map(|shown| shown.view.id.clone()),
        _ => None,
    })
    .flatten();
    let roll_moved = with_pages(|state| {
        let moved = state.tv_roll != tv_roll;
        state.tv_roll = tv_roll;
        moved
    });
    // Act offers the shops here.
    if shops_changed || roll_moved {
        super::page::redraw_part(crate::draw_plan::Part::PhoneChoices);
    }
    match open {
        Some(page) => {
            nav::on_action(&page_action(&page));
        }
        None if changed || stale => repaint(),
        None => {}
    }
}

/// The player's words for a refusal of a pages route of kind `kind`: the seat's own refusals read
/// as the seat routes' do.
fn pages_words(kind: Option<PagesErrorKind>) -> String {
    match kind {
        Some(kind) => kind.player_words().to_owned(),
        None => refusal_words(None),
    }
}

/// Sends what the hands do with a thing the hero carries, then asks again so the bag shows it.
/// Giving and selling go to the one the player speaks to; with nobody, nothing is sent.
async fn send_bag(bag: impl FnOnce(Option<String>) -> Option<BagAct>) {
    let to = with_pages(|state| state.ui.addressing.clone());
    let Some(bag) = bag(to) else {
        return;
    };
    let problem = match act(SeatAction::Bag { said: 0, bag }, None).await {
        Ok(_) => None,
        Err(CallError::Refused(rejection)) => {
            Some(pages_words(rejection.kind.map(SeatActionErrorKind::pages)))
        }
        Err(_) => Some("The table could not be reached. Try again.".to_owned()),
    };
    with_pages(|state| state.ui.problem = problem);
    poll().await;
    repaint();
}

/// Keeps a note and asks again, so it shows at once.
async fn send_note(note: NoteAsk) -> bool {
    let kept = match act(SeatAction::Note { said: 0, note }, None).await {
        Ok(_) => {
            with_pages(|state| state.ui.problem = None);
            true
        }
        Err(CallError::Refused(rejection)) => {
            let message = pages_words(rejection.kind.map(SeatActionErrorKind::pages));
            with_pages(|state| state.ui.problem = Some(message));
            false
        }
        Err(_) => false,
    };
    poll().await;
    repaint();
    kept
}

async fn on_tap(tap: PageTap) {
    match tap {
        PageTap::Say(index) => {
            super::seat::dispatch(&SeatTap::Say(index).action());
        }
        PageTap::Address(person) => {
            address(person.as_deref().unwrap_or(ROOM_TARGET));
            super::voice::refresh();
        }
        PageTap::Send => {
            let line = typed(TALK_FIELD_ID).unwrap_or_default();
            let to = with_pages(|state| state.ui.addressing.clone());
            // A Talk line is a turn of its own, never the answer to what the table asks.
            let said = super::seat::say(&line, to, SayKind::Talk, None).await;
            let taken = said.is_ok();
            with_pages(|state| match said {
                Ok(()) => {
                    state.ui.talk_draft.clear();
                    state.ui.problem = None;
                }
                Err(error) => {
                    state.ui.talk_draft = line;
                    state.ui.problem = Some(player_words(&error));
                }
            });
            if let Some(input) = input_of(TALK_FIELD_ID) {
                // A line the host took leaves the field; a refused one stays to send again.
                if taken {
                    input.set_value("");
                }
                input.blur().ok();
            }
            repaint();
        }
        PageTap::Filter(filter) => {
            with_pages(|state| state.ui.filter = filter);
            repaint();
        }
        PageTap::Write => {
            let text = typed(NOTE_FIELD_ID).unwrap_or_default();
            if let Some(input) = input_of(NOTE_FIELD_ID) {
                input.blur().ok();
            }
            if send_note(NoteAsk::Write { text }).await {
                with_pages(|state| state.ui.note_draft.clear());
                repaint();
            }
        }
        PageTap::Snap => {
            send_note(NoteAsk::Snap).await;
        }
        PageTap::Keep(line) => {
            send_note(NoteAsk::Keep { line }).await;
        }
        PageTap::Refused(place) => {
            let reason = with_pages(|state| {
                state
                    .pages
                    .as_ref()?
                    .places
                    .iter()
                    .find(|known| known.id == place)?
                    .refused
                    .clone()
            });
            with_pages(|state| state.ui.refused = reason);
            repaint();
        }
        PageTap::Trade(keeper) => {
            with_pages(|state| {
                state.ui.addressing = Some(keeper);
                state.ui.talk_draft = TRADE_LINE.to_owned();
            });
            nav::on_action(&page_action(TALK_PAGE));
        }
        PageTap::Use(item) => send_bag(|_| Some(BagAct::Use { item })).await,
        PageTap::Drop(item) => send_bag(|_| Some(BagAct::Drop { item })).await,
        PageTap::Give(item) => send_bag(|to| to.map(|to| BagAct::Give { item, to })).await,
        PageTap::Sell(item) => send_bag(|to| to.map(|to| BagAct::Sell { item, to })).await,
        PageTap::Mic => super::voice::talk_mic_pressed(),
        PageTap::Speaking(on) => turn_voice(on).await,
        PageTap::Clue(id) => {
            with_pages(|state| {
                state.ui.clue = Some(id);
                state.ui.problem = None;
            });
            repaint();
        }
        PageTap::CloseClue => {
            with_pages(|state| {
                state.ui.clue = None;
                state.ui.problem = None;
            });
            repaint();
        }
        PageTap::ShowClue(id) => send_clue(Some(id)).await,
        PageTap::TakeClueOff => send_clue(None).await,
        PageTap::Cast {
            spell,
            level,
            ritual,
        } => send_cast(spell, level, ritual).await,
        PageTap::Buy { shop, item, qty } => send_buy(shop, item, qty).await,
    }
}

/// Turns the seat's talking by voice on or off; off ends a recording not written down yet.
async fn turn_voice(on: bool) {
    if !on {
        super::voice::on_action("mic-cancel");
    }
    let answer = act(SeatAction::Voice { said: 0, on }, None).await;
    let voice = outcome(answer, |done| match done {
        SeatOutcome::Voice { stt } => Some(stt),
        _ => None,
    });
    let problem = match voice {
        Ok(stt) => {
            with_pages(|state| state.stt = stt);
            None
        }
        Err(error @ CallError::Refused(_)) => Some(player_words(&seat_error(error))),
        Err(_) => Some("The table could not be reached. Try again.".to_owned()),
    };
    with_pages(|state| state.ui.problem = problem);
    super::voice::refresh();
    repaint();
}

/// Draws the open page again: the Talk page's microphone changed.
pub(super) fn refresh_talk() {
    repaint();
}

/// Shows on the live pages why the table did not take the last line, or clears it once one was
/// taken.
pub(super) fn note_problem(problem: Option<String>) {
    let changed = with_pages(|state| {
        let changed = state.ui.problem != problem;
        state.ui.problem = problem;
        changed
    });
    if changed {
        repaint();
    }
}

/// Takes a tap on a live page or the hero build; whether it was one.
pub(super) fn dispatch(action: &str) -> bool {
    if let Some(tap) = BuildTap::from_action(action) {
        spawn_local(on_build_tap(tap));
        return true;
    }
    if let Some(check) = roll_of(action) {
        spawn_local(send_roll(check));
        return true;
    }
    let Some(tap) = PageTap::from_action(action) else {
        return false;
    };
    if let PageTap::Say(_) = tap {
        with_pages(|state| state.ui.refused = None);
    }
    spawn_local(on_tap(tap));
    true
}

/// The seat's hero build after Lock in (`pages::build`): its last answer, what the phone has open,
/// and whether the build was asked to begin.
#[derive(Default)]
struct Build {
    view: Option<CreationView>,
    ui: BuildUi,
    asked_to_begin: bool,
}

thread_local! {
    static BUILD: RefCell<Build> = RefCell::new(Build::default());
}

fn with_build<T>(act: impl FnOnce(&mut Build) -> T) -> T {
    BUILD.with(|build| act(&mut build.borrow_mut()))
}

/// The attribute the build's screen carries, so a redraw knows whether it is the one on the layer.
const BUILD_MARK: &str = "data-build";

/// Whether the seat builds a hero: its look is locked in, it plays no character, and the table
/// still gathers in the lobby.
pub(super) fn may_build(seat: &SeatCard, view: &SeatView) -> bool {
    seat.hero_locked
        && seat.plays.is_none()
        && view.story.is_none()
        && view.stage == TableStage::Gathering
}

/// Whether the build shows over the lobby now: it may build and its build is not submitted.
pub(super) fn shows_build(seat: &SeatCard, view: &SeatView) -> bool {
    may_build(seat, view)
        && with_build(|build| {
            build
                .view
                .as_ref()
                .is_none_or(|view| view.state != BuildState::Ready)
        })
}

/// Draws the build into `layer`, over the room's picture `scene`, unless the same screen is up.
pub(super) fn draw_build(layer: &Element, scene: Option<&str>) -> Result<(), JsValue> {
    let panels = with_build(|build| {
        build_panels(&BuildInput {
            view: build.view.as_ref(),
            ui: &build.ui,
            scene,
        })
    });
    let shown = layer.query_selector(&format!("[{BUILD_MARK}]"))?;
    if shown.is_some() && LAST_BUILD.with(|last| last.borrow().as_ref() == Some(&panels)) {
        return Ok(());
    }
    let scrolled = layer
        .query_selector(".ph-body")?
        .filter(|_| shown.is_some())
        .map(|body| body.scroll_top());
    let document = crate::dom::document()?;
    let screen = crate::phone_kit::draw::screen(&document, &panels)?;
    screen.set_attribute(BUILD_MARK, "")?;
    crate::dom::clear(layer);
    crate::dom::append(layer, &screen)?;
    if let (Some(top), Some(body)) = (scrolled, layer.query_selector(".ph-body")?) {
        body.set_scroll_top(top);
    }
    LAST_BUILD.with(|last| *last.borrow_mut() = Some(panels));
    Ok(())
}

thread_local! {
    /// The build screen last drawn.
    static LAST_BUILD: RefCell<Option<PhonePanels>> = const { RefCell::new(None) };
}

/// Keeps a creation route's answer, or why it was refused; draws the join layer again.
fn settle_build(answer: Result<CreationView, CallError<CreationErrorKind>>) {
    let problem = match answer {
        Ok(view) => {
            with_build(|build| build.view = Some(view));
            None
        }
        Err(CallError::Refused(rejection)) => Some(
            rejection
                .kind
                .map_or("The table did not take that. Try again.", |kind| {
                    kind.words()
                })
                .to_owned(),
        ),
        Err(_) => Some("The table could not be reached. Try again.".to_owned()),
    };
    with_build(|build| {
        build.ui.problem = problem;
        build.ui.busy = false;
    });
    super::seat::redraw();
}

/// Asks the host for the seat's build beside its view, and begins it once (a quick hero, every
/// slot chosen at random) when it has not begun.
pub(super) async fn poll_build() {
    let answer = seat_call::<CreationView, CreationErrorKind>(Call::get(PLAY_CREATION)).await;
    let Ok(view) = answer else {
        return;
    };
    // A build that has begun lets the next one begin too: after a change of hero, or at the next
    // start, the host shows the seat's build as not begun again.
    if view.hero.is_some() {
        with_build(|build| build.asked_to_begin = false);
    }
    let begin = view.hero.is_none() && !with_build(|build| build.asked_to_begin);
    let changed = with_build(|build| {
        let changed = build.view.as_ref() != Some(&view);
        build.view = Some(view);
        changed
    });
    if begin {
        with_build(|build| build.asked_to_begin = true);
        let request = Call::post(PLAY_CREATION_CHOOSE, &BuildChoose::default());
        let answer = seat_call(request).await;
        if answer.is_err() {
            // Asked again at the next poll.
            with_build(|build| build.asked_to_begin = false);
        }
        settle_build(answer);
        return;
    }
    if changed {
        super::seat::redraw();
    }
}

async fn on_build_tap(tap: BuildTap) {
    let request = match tap {
        BuildTap::Open(part) => {
            with_build(|build| {
                build.ui.part = part;
                build.ui.problem = None;
            });
            LAST_BUILD.with(|last| *last.borrow_mut() = None);
            super::seat::redraw();
            return;
        }
        BuildTap::Pick { set, clear } => Call::post(
            PLAY_CREATION_CHOOSE,
            &BuildChoose {
                picks: set
                    .into_iter()
                    .map(|(slot, option)| BuildPick { slot, option })
                    .collect(),
                clear,
            },
        ),
        BuildTap::Surprise => Call::post(PLAY_CREATION_SURPRISE, &BuildStep {}),
        BuildTap::Submit => {
            with_build(|build| build.ui.part = None);
            Call::post(PLAY_CREATION_SUBMIT, &BuildStep {})
        }
    };
    with_build(|build| build.ui.busy = true);
    super::seat::redraw();
    let answered = seat_call(request).await;
    let taken = answered.is_ok();
    settle_build(answered);
    if taken {
        reread_build().await;
    }
}

/// Reads the seat's build again and draws the build screen anew from it, as a fresh load does:
/// after a pick or a take back the host may have changed more than the answer said (it fills what
/// is left open), and every tap on the screen must come from what the host holds now.
async fn reread_build() {
    let Ok(view) = seat_call::<CreationView, CreationErrorKind>(Call::get(PLAY_CREATION)).await
    else {
        return;
    };
    with_build(|build| build.view = Some(view));
    LAST_BUILD.with(|last| *last.borrow_mut() = None);
    super::seat::redraw();
}

/// Asks the table to roll `check` from the Roll page, as the seat, and shows the answer: the roll,
/// the table's reason for not calling it, or why it could not be asked. The check judge may take
/// a while, so the ask waits as long as a lobby action does. Until the host serves the route, its
/// absence reads as rolls not being open yet.
async fn send_roll(check: SeatCheck) {
    let label = with_pages(|state| state.pages.as_ref().map(|pages| check_label(pages, &check)))
        .unwrap_or_else(|| "The check".to_owned());
    with_pages(|state| {
        state.ui.roll = Some(RollShown::Rolling {
            check: label.clone(),
        });
    });
    let Some(key) = super::seat::seat_key() else {
        with_pages(|state| {
            state.ui.roll = Some(RollShown::Told {
                check: label,
                words: refusal_words(None),
            });
        });
        repaint();
        return;
    };
    repaint();
    let request = Call::post(PLAY_SEAT_ROLL, &SeatRoll { check, about: None })
        .with_seat_key(&key)
        .within(WORK_REQUEST_TIMEOUT_MS);
    let shown = match crate::net::call::<SeatRolled, SeatErrorKind>(request).await {
        Ok(rolled) => RollShown::Rolled {
            check: label,
            rolled,
        },
        Err(CallError::Net(NetError::Status { status: 404, .. })) => RollShown::Told {
            check: label,
            words: ROLLS_NOT_OPEN.to_owned(),
        },
        Err(CallError::Refused(rejection)) => RollShown::Told {
            check: label,
            words: refusal_words(rejection.kind),
        },
        Err(_) => RollShown::Told {
            check: label,
            words: "The table could not be reached. Try again.".to_owned(),
        },
    };
    with_pages(|state| state.ui.roll = Some(shown));
    repaint();
}

/// What the Roll page says while the host does not serve rolls yet.
const ROLLS_NOT_OPEN: &str = "Rolls are not open yet.";

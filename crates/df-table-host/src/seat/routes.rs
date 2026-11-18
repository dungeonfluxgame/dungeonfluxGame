//! The seat routes: the phone's (`df_playground::path::PLAY_*`, on the player listener and the
//! loopback listener) and the dev routes that manage the players books (`path`, loopback only).
//!
//! Every player route reads the seat key from its header and takes the seat from the book, never
//! from the body; every answer is a `SeatReply`. Each step writes one `SEAT` line with the seat,
//! the kind or the reason, never the key or its hash and never a name.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{ConnectInfo, State};
use axum::http::header::RETRY_AFTER;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Extension, Router};
use df_playground::look::wire::PackStatus;
use df_playground::overlay::Edit;
use df_playground::pages::wire::BuyDone;
use df_playground::path::{
    PLAY_HELLO, PLAY_HEROES, PLAY_JOIN, PLAY_JOIN_CODE, PLAY_JOIN_STATUS, PLAY_ROOM,
    PLAY_SEAT_ACTION, PLAY_SEAT_TRANSCRIBE, PLAY_SEAT_VIEW,
};
use df_playground::scene::SceneChange;
use df_playground::seat::wire::{
    AnsweredBy, CastDone, CodeCheck, CodeTaken, HelloReply, HeroesReply, JoinRequest, JoinStatus,
    LobbyStep, RoomReply, RoomState, SayKind, SeatAction, SeatCard, SeatIntentReply, SeatOutcome,
    SeatStory, SeatStt, SeatView, StoryLine, TableHero, TablePlayer, TableStage,
};
use df_rules::local_journey::valid_character_name;
use df_table::lobby::LobbyAction;
use df_table::ports::Clock;
use serde::Deserialize;

use super::book::{Book, Books, Holder, SeatRecord};
use super::key::{KeyHash, SeatCaller};
use super::request::{
    ApprovalAnswer, Approved, AskClear, AskSet, Asked, FriendAdd, FriendRemove, HeroAdd, HeroAdded,
    Inbox, InboxWait, PersonaBind, PlayersReport, SeatActionError, SeatError, SeatNote, SeatRemove,
    SeatsNamed,
};
use super::room::Missed;
use super::scenario::{self, HeroLaunch, persona_of};
use super::table::{self, ActionResult};
use super::talk::{Asking, Begun, CastNote, InboxEntry, Line};
use crate::blocking::{run_blocking, run_owned};
use crate::dev_http::{read_body, reply};
use crate::engine::request::AudienceSpec;
use crate::limits::LONGEST_INBOX_WAIT;
use crate::observe::WallClock;
use crate::state::AppState;

/// The addresses of the seat dev routes (loopback only).
pub mod path {
    /// The seat book, the friends book and the requests waiting (GET).
    pub const PLAYERS: &str = "/dev/players";
    /// POST a `FriendAdd`.
    pub const FRIEND_ADD: &str = "/dev/players/friends/add";
    /// POST a `FriendRemove`.
    pub const FRIEND_REMOVE: &str = "/dev/players/friends/remove";
    /// POST an `ApprovalAnswer`: the friend takes a seat.
    pub const APPROVE: &str = "/dev/players/approve";
    /// POST an `ApprovalAnswer`: the friend is told no.
    pub const REFUSE: &str = "/dev/players/refuse";
    /// POST a `HeroAdd`.
    pub const HERO_ADD: &str = "/dev/players/heroes/add";
    /// POST a `PersonaBind`: the character the hero at a seat plays.
    pub const PERSONA: &str = "/dev/players/persona";
    /// POST an `AskSet`: what the table asks a seat now.
    pub const ASK: &str = "/dev/players/ask";
    /// POST an `AskClear`: the table stops asking a seat.
    pub const ASK_CLEAR: &str = "/dev/players/ask/clear";
    /// POST a `SeatNote`: a word from the table for a seat, until its next ask.
    pub const NOTE: &str = "/dev/players/note";
    /// POST an `InboxWait`: the next line a seat says (a long poll).
    pub const INBOX: &str = "/dev/players/inbox";
    /// POST a `SeatRemove`: the seat leaves the table and the book.
    pub const SEAT_REMOVE: &str = "/dev/players/seats/remove";
}

/// The body of `PLAY_HELLO`: nothing but the key in its header.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HelloBody {}

/// Milliseconds since the Unix epoch now, from the host's wall clock. Read at the edge.
fn now_ms() -> u64 {
    WallClock.now_ms()
}

fn checked_name(name: &str) -> Result<&str, SeatError> {
    if valid_character_name(name) {
        Ok(name)
    } else {
        Err(SeatError::InvalidName)
    }
}

/// Writes `SEAT refused reason=<key>` for a refusal a phone was given, then answers.
fn answer_phone<T: serde::Serialize>(state: &AppState, result: Result<T, SeatError>) -> Response {
    if let Err(error) = &result {
        state.record_line(format!("SEAT refused reason={}", error.key()));
    }
    reply(result)
}

/// The adventure the TV's landing page shows now: its title and picture.
fn adventure(state: &AppState) -> (Option<String>, Option<String>) {
    let Edit::Set(home) = state.playground().snapshot().scene.home else {
        return (None, None);
    };
    let Some(adventure) = home.adventure else {
        return (None, None);
    };
    let title = Some(adventure.title).filter(|title| !title.is_empty());
    (title, adventure.picture)
}

/// `GET PLAY_ROOM`.
async fn room(State(state): State<Arc<AppState>>) -> Response {
    let result = async {
        let records = state.seats().open().await?.books().records().to_vec();
        // A table that cannot be read has nobody at it: the room then says it is closed.
        let players = table_players(&state, &records, None)
            .await
            .map(|players| {
                players
                    .into_iter()
                    .filter(|player| player.present)
                    .map(|player| player.name)
                    .collect()
            })
            .unwrap_or_default();
        let room = match state.engine() {
            Ok(engine) => match engine.intent_view(AudienceSpec::Tv).await {
                Ok(view) if table::is_gathering(&view) => RoomState::Gathering,
                Ok(_) => RoomState::Playing,
                Err(_) => RoomState::Closed,
            },
            Err(_) => RoomState::Closed,
        };
        let (adventure, picture) = adventure(&state);
        Ok::<_, SeatError>(RoomReply {
            adventure,
            picture,
            players,
            state: room,
        })
    }
    .await;
    reply(result)
}

/// `POST PLAY_HELLO`.
async fn hello(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let result = async {
        let key = KeyHash::from_headers(&headers)?;
        let HelloBody {} = read_body(bytes)?;
        let mut open = state.seats().open().await?;
        let friends = open.books().friend_cards();
        let Some(record) = open.books().seat_of(&key).cloned() else {
            state.record_line("SEAT hello known=no".to_owned());
            return Ok(HelloReply::Unknown { friends });
        };
        let view = table::quiet_view(state.engine()?, record.seat).await?;
        let seated = table::is_seated(&view);
        state.record_line(format!(
            "SEAT hello known=yes seat={} seated={}",
            record.seat,
            if seated { "yes" } else { "no" }
        ));
        // A phone whose key holds a seat goes straight back to it: the table seats it again by
        // itself (the host restarted). Only when that is refused does it ask to continue.
        if !seated {
            drop(open);
            return match rejoin(&state, &key).await {
                Ok(JoinStatus::Seated { seat }) => Ok(HelloReply::Seated { seat }),
                refused => {
                    state.record_line(format!(
                        "SEAT hello rejoin_refused seat={} error={}",
                        record.seat,
                        refused.err().map_or("not_seated", |error| error.key())
                    ));
                    let mut open = state.seats().open().await?;
                    Ok(HelloReply::Returning {
                        seat: open.books().card_of(&record),
                        friends,
                    })
                }
            };
        }
        open.books().touch(&key, now_ms());
        open.save(Book::Seats).await?;
        Ok::<_, SeatError>(HelloReply::Seated {
            seat: open.books().card_of(&record),
        })
    }
    .await;
    answer_phone(&state, result)
}

fn join_kind(request: &JoinRequest) -> &'static str {
    match request {
        JoinRequest::Returning => "returning",
        JoinRequest::Guest { .. } => "guest",
        JoinRequest::Friend { .. } => "friend",
        JoinRequest::Account { .. } => "account",
    }
}

/// The room code a new way in carries; none for coming back to the key's own seat.
fn join_code(request: &JoinRequest) -> Option<&str> {
    match request {
        JoinRequest::Returning => None,
        JoinRequest::Guest { code, .. }
        | JoinRequest::Friend { code, .. }
        | JoinRequest::Account { code } => Some(code),
    }
}

/// Where a request came from, for counting a run of wrong room codes: the phone's address on the
/// player listener; the loopback listener says none.
fn caller_address(connection: Option<Extension<ConnectInfo<SocketAddr>>>) -> String {
    connection.map_or_else(
        || "loopback".to_owned(),
        |Extension(ConnectInfo(address))| address.ip().to_string(),
    )
}

/// Whether `code`, from `address`, is the room's (`room::RoomDoor::check`), answered at once: a
/// wrong one is `code_mismatch` and opens the address's wait; any try inside it is
/// `too_many_guesses`, unchecked. One `SEAT <step> refused reason=code_mismatch misses=
/// delay_ms=` or `SEAT <step> refused reason=too_many_guesses retry_after_ms=` line.
fn admit_code(state: &AppState, step: &str, address: &str, code: &str) -> Result<(), SeatError> {
    match state.seats().check_code(address, code) {
        Ok(()) => Ok(()),
        Err(Missed::Wrong { misses, delay }) => {
            state.record_line(format!(
                "SEAT {step} refused reason=code_mismatch misses={misses} delay_ms={}",
                delay.as_millis()
            ));
            Err(SeatError::CodeMismatch)
        }
        Err(Missed::TooSoon { retry_after }) => {
            let retry_after_ms = retry_after.as_millis();
            state.record_line(format!(
                "SEAT {step} refused reason=too_many_guesses retry_after_ms={retry_after_ms}"
            ));
            Err(SeatError::TooManyGuesses {
                retry_after_s: u64::try_from(retry_after_ms.div_ceil(1_000)).unwrap_or(u64::MAX),
            })
        }
    }
}

/// [`answer_phone`] for a route that checks the room code: `too_many_guesses` also carries
/// `Retry-After`.
fn answer_code_route<T: serde::Serialize>(
    state: &AppState,
    result: Result<T, SeatError>,
) -> Response {
    let retry_after_s = match &result {
        Err(SeatError::TooManyGuesses { retry_after_s }) => Some(*retry_after_s),
        _ => None,
    };
    let mut response = answer_phone(state, result);
    if let Some(seconds) = retry_after_s {
        response
            .headers_mut()
            .insert(RETRY_AFTER, HeaderValue::from(seconds));
    }
    response
}

/// `POST PLAY_JOIN_CODE`: whether the room code typed at Next is the table's, checked as a join
/// checks it (`admit_code`), so the phone asks again before the player names themselves. One
/// `SEAT code taken` line when it is.
async fn join_code_check(
    State(state): State<Arc<AppState>>,
    connection: Option<Extension<ConnectInfo<SocketAddr>>>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let address = caller_address(connection);
    let result = read_body::<CodeCheck>(bytes)
        .map_err(SeatError::from)
        .and_then(|CodeCheck { code }| admit_code(&state, "code", &address, &code))
        .map(|()| {
            state.record_line("SEAT code taken".to_owned());
            CodeTaken {}
        });
    answer_code_route(&state, result)
}

/// `POST PLAY_JOIN`. A new way in must carry the room code: a wrong one, or any inside the
/// address's wait after one, is refused at once (`admit_code`).
async fn join(
    State(state): State<Arc<AppState>>,
    connection: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let address = caller_address(connection);
    let result = async {
        let key = KeyHash::from_headers(&headers)?;
        let request: JoinRequest = read_body(bytes)?;
        state.record_line(format!("SEAT join_requested kind={}", join_kind(&request)));
        if let Some(code) = join_code(&request) {
            admit_code(&state, "join", &address, code)?;
        }
        match request {
            JoinRequest::Account { .. } => Err(SeatError::AccountsNotAvailable),
            JoinRequest::Returning => rejoin(&state, &key).await,
            JoinRequest::Guest { name, .. } => seat_guest(&state, &key, checked_name(&name)?).await,
            JoinRequest::Friend { profile, .. } => {
                let mut open = state.seats().open().await?;
                let request = open.books().ask_as_friend(&key, profile, now_ms())?;
                Ok(JoinStatus::AwaitingApproval { request })
            }
        }
    }
    .await;
    answer_code_route(&state, result)
}

/// The key's own seat again, seated through the lobby if the table lost it (a restart).
async fn rejoin(state: &AppState, key: &KeyHash) -> Result<JoinStatus, SeatError> {
    let mut open = state.seats().open().await?;
    let record = open
        .books()
        .seat_of(key)
        .cloned()
        .ok_or(SeatError::UnknownKey)?;
    table::sit_down(state.engine()?, record.seat).await?;
    open.books().touch(key, now_ms());
    open.save(Book::Seats).await?;
    state.record_line(format!("SEAT rejoined seat={}", record.seat));
    Ok(JoinStatus::Seated {
        seat: open.books().card_of(&record),
    })
}

/// A guest named `name`: the key's own seat when it holds one, else the first free seat.
async fn seat_guest(state: &AppState, key: &KeyHash, name: &str) -> Result<JoinStatus, SeatError> {
    let mut open = state.seats().open().await?;
    let engine = state.engine()?;
    let held = open.books().seat_of(key).map(|record| record.seat);
    let seat = match held {
        Some(seat) => {
            table::sit_down(engine, seat).await?;
            seat
        }
        None => {
            let free = open.books().free_seats();
            let seat = table::take_free_seat(engine, &free).await?;
            state.seats().talk().forget_seat(seat);
            seat
        }
    };
    let card = open.books().seat(key, seat, Holder::Guest, name, now_ms());
    open.save(Book::Seats).await?;
    record_seated(state, &card);
    Ok(JoinStatus::Seated { seat: card })
}

fn record_seated(state: &AppState, card: &SeatCard) {
    state.record_line(format!(
        "SEAT seated seat={} kind={}",
        card.seat,
        card.kind.key()
    ));
}

/// `GET PLAY_JOIN_STATUS`.
async fn join_status(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let result = async {
        let key = KeyHash::from_headers(&headers)?;
        Ok::<_, SeatError>(state.seats().open().await?.books().status(&key))
    }
    .await;
    reply(result)
}

/// `GET PLAY_HEROES`.
async fn heroes(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let result = async {
        let key = KeyHash::from_headers(&headers)?;
        let heroes = state.seats().open().await?.books().heroes_of(&key)?;
        Ok::<_, SeatError>(HeroesReply { heroes })
    }
    .await;
    reply(result)
}

/// Everyone the book seats, with whether the table has them now (a player who has not come back
/// since a restart is away) and whether they are ready, read from each seat's own view.
pub(crate) async fn table_players(
    state: &AppState,
    records: &[SeatRecord],
    own: Option<&KeyHash>,
) -> Result<Vec<TablePlayer>, SeatError> {
    let engine = state.engine()?;
    let heroes = table_heroes(state, records).await?;
    let mut players = Vec::with_capacity(records.len());
    for (record, hero) in records.iter().zip(heroes) {
        let view = table::quiet_view(engine, record.seat).await?;
        players.push(TablePlayer {
            name: record.name.clone(),
            present: table::is_seated(&view),
            ready: table::is_ready(&view),
            you: own == Some(&record.key),
            hero,
        });
    }
    Ok(players)
}

/// The hero each of `records` plays, from the hero library, with its pack.
async fn table_heroes(
    state: &AppState,
    records: &[SeatRecord],
) -> Result<Vec<Option<TableHero>>, SeatError> {
    let mut open = state.seats().open().await?;
    let books = open.books();
    Ok(records
        .iter()
        .map(|record| {
            let hero = books.hero(record.hero?)?;
            Some(TableHero {
                id: hero.id,
                name: hero.name.clone(),
                species: hero.species.clone(),
                class: hero.class.clone(),
                portrait: hero.portrait.clone(),
                pack: hero.pack.clone(),
            })
        })
        .collect())
}

/// `GET PLAY_SEAT_VIEW`: the seat's view (one `ENGINE wire_view` line) as typed data, and who is
/// at the table.
async fn seat_view(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let result = async {
        let SeatCaller { record } = SeatCaller::of(&state, &headers).await?;
        let (key, seat, persona) = (record.key, record.seat, record.persona);
        // Who went away or came back shows on every phone's next look (`presence`).
        super::presence::sweep(&state, now_ms()).await?;
        // The TV lobby follows the table while it gathers.
        super::lobby::show(&state).await;
        let records = state.seats().open().await?.books().records().to_vec();
        let view = state
            .engine()?
            .machine_view(AudienceSpec::Seat { seat })
            .await?;
        let players = table_players(&state, &records, Some(&key)).await?;
        // The play screen shows only once the table started; with a scenario, only while the
        // seat's day runs or after it ended (a binding left from an earlier host never shows).
        let playing = match state.seats().scenario() {
            Some(scenario) => scenario.playing(seat).await,
            None => persona,
        };
        let story = match playing.filter(|_| !table::is_gathering(&view)) {
            Some(persona) => Some(Box::new(story_of(&state, &persona).await)),
            None => None,
        };
        let ask = state.seats().talk().ask_of(seat).cloned();
        // Each read of the seat's talk is its own statement: the talk is locked while one runs.
        let talk_waiting = state.seats().talk().talk_waiting(seat);
        let notice = state.seats().talk().notice_of(seat);
        // The roll called for this seat's hero, and the card the shared screen showed for it: the
        // phone shows both as the screen does.
        let pending_roll = crate::persona::check::pending_roll(&state, seat);
        if let Some(pending) = &pending_roll {
            state.seats().talk().roll_called(seat, &pending.roll_id);
        }
        let shown_roll = match state.playground().snapshot().scene.roll {
            Edit::Set(shown) => Some(shown.view),
            Edit::Keep | Edit::Clear => None,
        };
        let last_roll = state
            .seats()
            .talk()
            .last_roll(seat, shown_roll.as_ref())
            .map(Box::new);
        let stt = stt_of(&state, seat);
        let answered = state.seats().talk().answered_of(seat).cloned();
        let answered_by = match answered {
            Some(answered) => {
                let mut open = state.seats().open().await?;
                Some(AnsweredBy {
                    ask: answered.ask,
                    seat: answered.seat,
                    hero: seat_hero_name(open.books(), answered.seat),
                    said: answered.said,
                })
            }
            None => None,
        };
        Ok::<_, SeatError>(SeatView {
            revision: view
                .revision
                .as_ref()
                .and_then(|revision| revision.sequence)
                .unwrap_or_default(),
            stage: if table::is_gathering(&view) {
                TableStage::Gathering
            } else {
                TableStage::Playing
            },
            ready: table::is_ready(&view),
            can_toggle_ready: table::offers(&view, LobbyAction::Ready) || table::is_ready(&view),
            can_start: table::offers(&view, LobbyAction::Start),
            players,
            ask,
            story,
            day_end: state
                .seats()
                .scenario()
                .and_then(|scenario| scenario.day_end(seat)),
            start_problem: state
                .seats()
                .scenario()
                .and_then(|scenario| scenario.start_problem(seat)),
            talk_waiting,
            stt,
            notice,
            pending_roll: pending_roll.map(Box::new),
            last_roll,
            answered_by,
        })
    }
    .await;
    reply(result)
}

/// The name of the hero `seat` plays, from the hero library; the player's name while it plays
/// none, and the seat's number when nobody holds it now.
fn seat_hero_name(books: &mut Books, seat: u64) -> String {
    let Some(record) = books.record_at(seat).cloned() else {
        return format!("Seat {seat}");
    };
    record
        .hero
        .and_then(|hero| books.hero(hero))
        .map_or(record.name, |hero| hero.name.clone())
}

/// The name of the character `persona`: its persona's own name, else the name of the hero a seat
/// plays as it (before the day made the persona), never its id but as the last resort.
pub(crate) async fn hero_name(state: &Arc<AppState>, persona: &str) -> String {
    if let Ok(own) = crate::persona::generate::load_persona(state, persona).await {
        return own.display_name().to_owned();
    }
    let named = match state.seats().open().await {
        Ok(mut open) => {
            let books = open.books();
            books
                .records()
                .iter()
                .find(|record| record.persona.as_deref() == Some(persona))
                .and_then(|record| record.hero)
                .and_then(|hero| books.hero(hero))
                .map(|hero| hero.name.clone())
        }
        Err(_) => None,
    };
    named.unwrap_or_else(|| persona.to_owned())
}

/// The characters the seats play: the party, who know each other's names from the start.
pub(crate) async fn party_personas(state: &AppState) -> Vec<String> {
    match state.seats().open().await {
        Ok(mut open) => open
            .books()
            .records()
            .iter()
            .filter_map(|record| record.persona.clone())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// The name the character `hero` knows the speaker of `line` by: their own name for their own
/// lines, a name they learned, else what the speaker does ("The innkeeper"); the narrator is "The
/// story". The one place a speaker's label is made (the phone's story and its notebook).
pub(crate) async fn speaker_name(state: &Arc<AppState>, hero: &str, line: &StoryLine) -> String {
    speaker_names(state, hero, std::slice::from_ref(line))
        .await
        .pop()
        .unwrap_or_else(|| "Someone".to_owned())
}

/// [`speaker_name`] of each of `lines`, in order, with the hero's state and the speakers'
/// personas read once for all of them.
async fn speaker_names(state: &Arc<AppState>, hero: &str, lines: &[StoryLine]) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for id in lines
        .iter()
        .filter(|line| !line.you)
        .filter_map(|line| line.speaker.as_ref())
    {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    let summaries = crate::persona::summaries_of(state, ids).await;
    // A state that cannot be read knows no names: the speakers are told by what they do.
    let known = known_to(state, hero).await;
    let own = if lines.iter().any(|line| line.you) {
        Some(hero_name(state, hero).await)
    } else {
        None
    };
    lines
        .iter()
        .map(|line| {
            if line.you {
                return own.clone().unwrap_or_else(|| hero.to_owned());
            }
            let Some(id) = &line.speaker else {
                return "The story".to_owned();
            };
            summaries
                .iter()
                .find(|summary| summary.id == *id)
                .map_or_else(
                    || "Someone".to_owned(),
                    |summary| label_for(summary, &known),
                )
        })
        .collect()
}

/// The ids whose names the character `hero` knows (`persona::known_names`: the names learned, and
/// the party's); none when its state cannot be read.
pub(crate) async fn known_to(state: &Arc<AppState>, hero: &str) -> Vec<String> {
    let party = party_personas(state).await;
    let (reader, hero) = (Arc::clone(state), hero.to_owned());
    run_blocking(
        move || {
            let known =
                crate::persona::known_names(&crate::persona::load_state(&reader, &hero), &party);
            Ok::<_, ()>(known)
        },
        (),
    )
    .await
    .unwrap_or_default()
}

/// What a hero who knows the names `known` calls the persona of `summary`: their name when it is
/// known, else what they do.
pub(crate) fn label_for(summary: &crate::persona::PersonaSummary, known: &[String]) -> String {
    if known.contains(&summary.id) {
        summary.display_name.clone()
    } else {
        unnamed_label(&summary.job)
    }
}

/// What someone whose name the hero has not learned is called: by what they do, as the persona's
/// job says it, without the full stop a job written as a sentence ends with.
fn unnamed_label(job: &str) -> String {
    let job = job.trim();
    format!("The {}", job.strip_suffix('.').unwrap_or(job))
}

/// The story as the character `persona` lives it: their name, the active place, the picture the
/// shared screen shows, the lines heard so far and their own latest thought.
async fn story_of(state: &Arc<AppState>, persona: &str) -> SeatStory {
    let hero = hero_name(state, persona).await;
    let reader = Arc::clone(state);
    // A place that cannot be read is written by `active_place_name`; a stopped task is no place.
    let place = run_blocking(
        move || Ok::<_, ()>(crate::world::active_place_name(&reader)),
        (),
    )
    .await
    .ok()
    .flatten();
    let backdrop = match state.playground().scene_backdrop() {
        Edit::Set(image) => Some(image),
        Edit::Keep | Edit::Clear => None,
    };
    // A close-up cuts away from the room for a while: the thought had on the room stays up.
    let shown = backdrop.as_ref().and_then(|image| match image.plate {
        df_playground::fixture::PlateKind::CloseUp => None,
        _ => Some(image.media.as_str()),
    });
    let thought = state.seats().talk().thought_of(persona, shown);
    let plate = backdrop.map(|image| image.media);
    let heard = state.seats().talk().story_lines(persona);
    // A speaker is shown by the name the hero knows them by, never by an id; the names are read
    // once for the whole story.
    let names = speaker_names(state, persona, &heard).await;
    let lines = heard
        .into_iter()
        .zip(names)
        .map(|(line, name)| {
            let speaker = match (&line.speaker, line.you) {
                (None, false) => None,
                (_, true) => Some(hero.clone()),
                (Some(_), false) => Some(name),
            };
            StoryLine { speaker, ..line }
        })
        .collect();
    SeatStory {
        hero,
        place,
        plate,
        lines,
        thought,
    }
}

/// `seat`'s talking by voice: its own switch, and whether speech to text is set up at all.
fn stt_of(state: &AppState, seat: u64) -> SeatStt {
    SeatStt {
        on: state.seats().talk().stt_on(seat),
        available: state
            .speech()
            .is_some_and(crate::speech::Speech::stt_available),
    }
}

/// The seat's own switch for talking by voice (`SeatAction::Voice`): off until the player turns
/// it on; the host's restart and a seat changing hands turn it off again.
fn set_voice(state: &AppState, seat: u64, on: bool) -> SeatStt {
    state.seats().talk().set_stt(seat, on);
    state.record_line(format!("SEAT stt seat={seat} on={on}"));
    stt_of(state, seat)
}

/// `POST PLAY_SEAT_TRANSCRIBE`: a recording of the seat's player written down through the speech
/// service, with its limits and refusals, while the seat's own switch is on and the playground's
/// STT switch is on too (`Speech::transcribe`, the one gate of a paid transcription: nothing is
/// spent while the dev bar's switch is off). Each one is a `SEAT transcribed
/// seat= ms= chars=` line, so what talking by voice costs shows.
async fn seat_transcribe(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    use crate::speech::{SpeechError, SpeechFeature};
    let seat = match SeatCaller::of(&state, &headers).await {
        Ok(caller) => caller.seat(),
        Err(error) => return answer_phone::<()>(&state, Err(error)),
    };
    let speech = state.speech().cloned();
    let result = async {
        if !state.seats().talk().stt_on(seat) {
            return Err(SpeechError::Disabled {
                feature: SpeechFeature::Stt,
            });
        }
        let speech = speech.ok_or(SpeechError::Stopped)?;
        crate::speech::check_content_type(&headers)?;
        let pcm = Vec::from(body);
        crate::blocking::run_owned(
            async move { speech.transcribe(pcm, Vec::new()).await },
            SpeechError::Stopped,
        )
        .await
    }
    .await;
    match &result {
        Ok(heard) => state.record_line(format!(
            "SEAT transcribed seat={seat} ms={} chars={}",
            heard.audio_ms,
            heard.text.chars().count()
        )),
        Err(error) => state.record_line(format!(
            "SEAT transcribe_refused seat={seat} error={}",
            error.key()
        )),
    }
    reply(result)
}

/// The player's line, as written, for the seat's character (`SeatAction::Answer`, `Talk`): it
/// answers what the table asks the seat, or is talk of its own, and waits in the seat's inbox for
/// the runner. Anyone the Talk page lists may be spoken to: the people of the scene and the keepers
/// of the place, who stay there while the picture cuts to a close-up.
async fn say_line(
    state: &Arc<AppState>,
    caller: &SeatCaller,
    line: Line,
) -> Result<Option<u64>, SeatError> {
    caller.character()?;
    if let Some(listener) = &line.to
        && !crate::seat_pages::listener_ids(state)
            .await
            .contains(listener)
    {
        return Err(SeatError::UnknownListener {
            to: listener.clone(),
        });
    }
    let (kind, carded, chars) = (
        line.kind,
        line.thing.is_some(),
        line.text.trim().chars().count(),
    );
    let ask = state.seats().say(caller.seat(), line)?;
    state.record_line(format!(
        "SEAT said seat={} kind={} ask={} card_thing={carded} chars={chars}",
        caller.seat(),
        match kind {
            SayKind::Answer => "answer",
            SayKind::Talk => "talk",
        },
        ask.map_or_else(|| "-".to_owned(), |ask| ask.to_string()),
    ));
    Ok(ask)
}

/// `POST PLAY_SEAT_ACTION`: everything a seated phone does at the table, one typed action
/// (`SeatAction`) answered with what it came to (`SeatOutcome`) or one refusal of the
/// `SeatActionError` family. Every action carries the phone's id for it (`said`) and is done once
/// (`once`): a repeat is answered as before, a copy sent while it is being done is `in_flight`, the
/// same id with another body is `said_changed`. One `SEAT action seat= action= repeat=` line; a
/// refusal is one `SEAT refused` line.
async fn seat_action(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let result = async {
        let caller = SeatCaller::of(&state, &headers).await?;
        let action: SeatAction = read_body(bytes).map_err(SeatError::from)?;
        let (doer, done) = (Arc::clone(&state), action.clone());
        once(&state, caller.seat(), action, async move {
            act(&doer, &caller, done).await
        })
        .await
    }
    .await;
    if let Err(error) = &result {
        state.record_line(format!("SEAT refused reason={}", error.key()));
    }
    reply(result)
}

/// A seat action being done: its id is free again when it ends, however it ends (done, refused,
/// or its task stopped).
struct Doing {
    state: Arc<AppState>,
    seat: u64,
    said: u64,
}

impl Drop for Doing {
    fn drop(&mut self) {
        self.state.seats().talk().finish(self.seat, self.said);
    }
}

/// Does `seat`'s `action` once by its `said` id, by `work`: marked as being done in the same step
/// as the look for a repeat (`SeatTalk::begin`), then done on a task of its own (`run_owned`), so a
/// request the player listener cut off at its deadline still finishes, writes its lines and keeps
/// its outcome for the phone's retry. One `SEAT action seat= action= repeat=` line when done.
async fn once<Work>(
    state: &Arc<AppState>,
    seat: u64,
    action: SeatAction,
    work: Work,
) -> Result<SeatOutcome, SeatActionError>
where
    Work: Future<Output = Result<SeatOutcome, SeatActionError>> + Send + 'static,
{
    let begun = state.seats().talk().begin(seat, &action);
    if let Begun::Before(before) = begun {
        let outcome = before?;
        state.record_line(format!(
            "SEAT action seat={seat} action={} repeat=yes",
            action.key()
        ));
        return Ok(outcome);
    }
    let doing = Doing {
        state: Arc::clone(state),
        seat,
        said: action.said(),
    };
    run_owned(
        async move {
            let outcome = work.await?;
            doing.state.record_line(format!(
                "SEAT action seat={seat} action={} repeat=no",
                action.key()
            ));
            doing
                .state
                .seats()
                .talk()
                .remember(seat, action, outcome.clone());
            drop(doing);
            Ok(outcome)
        },
        SeatActionError::Seat(SeatError::NotApplied),
    )
    .await
}

/// Does `action` for the seat of `caller`, by its kind.
async fn act(
    state: &Arc<AppState>,
    caller: &SeatCaller,
    action: SeatAction,
) -> Result<SeatOutcome, SeatActionError> {
    let seat = caller.seat();
    Ok(match action {
        SeatAction::Answer {
            ask,
            text,
            to,
            thing,
            ..
        } => {
            let line = Line {
                text,
                to,
                kind: SayKind::Answer,
                thing,
                ask,
            };
            SeatOutcome::Said {
                ask: say_line(state, caller, line).await?,
            }
        }
        SeatAction::Talk { text, to, .. } => {
            let line = Line {
                text,
                to,
                kind: SayKind::Talk,
                thing: None,
                ask: None,
            };
            SeatOutcome::Said {
                ask: say_line(state, caller, line).await?,
            }
        }
        SeatAction::Lobby { step, .. } => SeatOutcome::Lobby {
            lobby: lobby_step(state, seat, step).await?,
        },
        SeatAction::Hero { hero, .. } => SeatOutcome::Hero {
            seat: choose_hero(state, &caller.record.key, hero).await?,
        },
        SeatAction::Voice { on, .. } => SeatOutcome::Voice {
            stt: set_voice(state, seat, on),
        },
        SeatAction::Note { note, .. } => SeatOutcome::Noted {
            note: crate::seat_pages::keep_note(state, caller, note).await?,
        },
        SeatAction::Bag { bag, .. } => SeatOutcome::Bag {
            bag: crate::seat_pages::bag_act(state, caller, bag).await?,
        },
        SeatAction::Roll { call, .. } => {
            crate::persona::check::answer_pending_roll(state, seat, &call)?;
            state.seats().talk().roll_called(seat, &call);
            SeatOutcome::Rolled { call }
        }
        SeatAction::ShowClue { clue, .. } => SeatOutcome::ClueShown {
            clue: show_clue(state, seat, clue).await?,
        },
        SeatAction::CloseClue { .. } => SeatOutcome::ClueClosed {
            clue: close_clue(state, seat).await?,
        },
        SeatAction::Buy {
            shop, item, qty, ..
        } => SeatOutcome::Bought {
            buy: buy(state, caller, &shop, &item, qty).await?,
        },
        SeatAction::Cast {
            spell,
            level,
            as_ritual,
            ..
        } => SeatOutcome::Cast {
            cast: cast(state, caller, &spell, level, as_ritual).await?,
        },
    })
}

/// `SeatAction::Buy`: the seat's character buys `qty` of `item` from the shop `shop` (its keeper's
/// id) through the inventory's one purchase (`world::inventory::buy`, which moves coins and goods
/// together or nothing, and writes its `INVENTORY buy` line). The keeper must be one of the people
/// here the Shop page draws its shops from (`seat_pages::listener_ids`), else `not_here`; one who
/// keeps no shop is the inventory's `not_a_shop`. A repeat of the action (the same `said`) is
/// answered from the seat's memory and never buys twice.
async fn buy(
    state: &Arc<AppState>,
    caller: &SeatCaller,
    shop: &str,
    item: &str,
    qty: u32,
) -> Result<BuyDone, SeatActionError> {
    let buyer = caller.character()?;
    if !crate::seat_pages::listener_ids(state)
        .await
        .iter()
        .any(|here| here == shop)
    {
        return Err(SeatError::NotHere {
            shop: shop.to_owned(),
        }
        .into());
    }
    let bought = crate::world::inventory::buy(state, buyer, shop, item, qty).await?;
    Ok(BuyDone {
        item: bought.item,
        qty: bought.qty,
        price: crate::seat_pages::price_words(bought.price.copper()),
        weight: bought.weight.to_string(),
        capacity: bought.capacity.to_string(),
    })
}

/// The clue board and the clue shown to the table, as the shared screen draws them now.
async fn shown_clues(
    state: &AppState,
) -> Result<(Option<df_playground::clues::ClueBoard>, Option<String>), SeatError> {
    let (mut fixture, _) = crate::playground::shown_fixture(state)
        .await
        .map_err(|_| SeatError::NotApplied)?;
    state.playground().snapshot().scene.apply_to(&mut fixture);
    Ok((fixture.tv.clues, fixture.tv.shown_clue))
}

/// `SeatAction::ShowClue`: the clue `clue` of the board shown goes full screen on the TV, in place
/// of the one shown. One `SEAT clue shown seat= clue=` line.
async fn show_clue(state: &AppState, seat: u64, clue: String) -> Result<String, SeatError> {
    let (board, _) = shown_clues(state).await?;
    let on_board = board.is_some_and(|board| board.notes.iter().any(|note| note.id == clue));
    if !on_board {
        return Err(SeatError::UnknownClue { clue });
    }
    let change = SceneChange::ShowClue { clue: clue.clone() };
    crate::playground::apply_scene_changes(state, vec![change])
        .await
        .map_err(|_| SeatError::NotApplied)?;
    state.record_line(format!("SEAT clue shown seat={seat} clue={clue}"));
    Ok(clue)
}

/// `SeatAction::CloseClue`: the clue shown to the table goes; any seat may close it. One `SEAT
/// clue closed seat= clue=` line.
async fn close_clue(state: &AppState, seat: u64) -> Result<Option<String>, SeatError> {
    let (_, shown) = shown_clues(state).await?;
    let Some(clue) = shown else {
        return Err(SeatError::NoClueShown);
    };
    crate::playground::apply_scene_changes(state, vec![SceneChange::ClearShownClue {}])
        .await
        .map_err(|_| SeatError::NotApplied)?;
    state.record_line(format!("SEAT clue closed seat={seat} clue={clue}"));
    Ok(Some(clue))
}

/// `SeatAction::Cast`: the seat's hero casts `spell` at `level` through the rules
/// (`creation::cast_spell`, which spends the slot and writes its `RULES cast` line), and the cast
/// waits in the seat's inbox for the runner, who hands it to the dungeon master. One `SEAT cast
/// seat= hero= spell= level= ritual=` line.
async fn cast(
    state: &AppState,
    caller: &SeatCaller,
    spell: &str,
    level: u8,
    as_ritual: bool,
) -> Result<CastDone, SeatActionError> {
    let seat = caller.seat();
    let hero = caller.record.hero.ok_or(SeatError::NoHeroToPlay)?;
    let outcome = crate::creation::cast_spell(state, hero, spell, level, as_ritual).await?;
    state.seats().cast(
        seat,
        CastNote {
            spell: outcome.spell.to_owned(),
            name: outcome.name.to_owned(),
            level: outcome.level,
            ritual: outcome.ritual,
            summary: outcome.summary.to_owned(),
        },
    );
    state.record_line(format!(
        "SEAT cast seat={seat} hero={hero} spell={} level={} ritual={}",
        outcome.spell, outcome.level, outcome.ritual
    ));
    Ok(CastDone {
        spell: outcome.spell.to_owned(),
        name: outcome.name.to_owned(),
        level: outcome.level,
        ritual: outcome.ritual,
        slot_spent: outcome.slot_spent,
        slots_left: outcome.slots_left,
        summary: outcome.summary.to_owned(),
    })
}

/// `POST path::PERSONA`: the character the hero at a seat plays, or none.
async fn bind_persona(
    State(state): State<Arc<AppState>>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let result = async {
        let PersonaBind { seat, persona } = read_body(bytes)?;
        if let Some(persona) = &persona {
            crate::persona::generate::load_persona(&state, persona)
                .await
                .map_err(|_| SeatError::UnknownPersona {
                    persona: persona.clone(),
                })?;
        }
        let mut open = state.seats().open().await?;
        open.books().bind_persona(seat, persona.as_deref())?;
        open.save(Book::Seats).await?;
        state.record_line(format!(
            "SEAT persona seat={seat} persona={}",
            persona.as_deref().unwrap_or("-")
        ));
        Ok::<_, SeatError>(open.books().report(state.seats().room_code()))
    }
    .await;
    reply(result)
}

/// `POST path::ASK`: what the table asks a seat now, replacing what it was asked.
/// That a runner's seat call (`ask`, `inbox`, `note`) about `seats` is the current day's: its run
/// token (`DEV_RUN_HEADER`) is the running day's, or it sends none and none of `seats` plays a day
/// (a developer's own call). A runner an earlier host left behind is refused `not_this_run`, with
/// one `SCENARIO stale_runner route= token= seats=` line.
async fn check_run(
    state: &AppState,
    headers: &HeaderMap,
    route: &str,
    seats: &[u64],
) -> Result<(), SeatError> {
    let sent = headers
        .get(super::key::DEV_RUN_HEADER)
        .and_then(|value| value.to_str().ok());
    let running = match state.seats().scenario() {
        Some(scenario) => scenario.run().await,
        None => None,
    };
    let current = match (sent, &running) {
        (Some(sent), Some((token, _))) => sent == token,
        (Some(_), None) => false,
        (None, Some((_, party))) => !seats.iter().any(|seat| party.contains(seat)),
        (None, None) => true,
    };
    if current {
        return Ok(());
    }
    state.record_line_and_echo(format!(
        "SCENARIO stale_runner route={route} token={} seats={}",
        sent.map_or("-", super::scenario::token_prefix),
        scenario::seats_text(seats.iter().copied())
    ));
    Err(SeatError::NotThisRun)
}

async fn set_ask(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let result = async {
        let AskSet {
            to,
            kind,
            prompt,
            cards,
            note,
        } = read_body(bytes)?;
        let seats = seats_named(&state, &to).await?;
        check_run(&state, &headers, "ask", &seats).await?;
        let card_count = cards.len();
        let ask = state.seats().talk().ask(
            &seats,
            Asking {
                kind,
                prompt,
                cards,
                note,
            },
        );
        state.record_line(format!(
            "SEAT asked seat={} ask={ask} kind={} cards={card_count} party={}",
            scenario::seats_text(seats.iter().copied()),
            kind.key(),
            seats.len() > 1
        ));
        Ok::<_, SeatError>(Asked { ask, seats })
    }
    .await;
    reply(result)
}

/// `POST path::ASK_CLEAR`: the table stops asking a seat.
async fn clear_ask(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let result = async {
        let AskClear { to } = read_body(bytes)?;
        let seats = seats_named(&state, &to).await?;
        check_run(&state, &headers, "ask_clear", &seats).await?;
        for seat in seats {
            if state.seats().talk().clear_ask(seat) {
                state.record_line(format!("SEAT ask_cleared seat={seat}"));
            }
        }
        Ok::<_, SeatError>(Inbox::Empty)
    }
    .await;
    reply(result)
}

/// `POST path::NOTE`: a word from the table for a seat, shown on its phone until the next ask. The
/// words are never logged, only their length.
async fn note(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let result = async {
        let SeatNote { seat, text } = read_body(bytes)?;
        check_run(&state, &headers, "note", &[seat]).await?;
        let text = text.trim();
        if text.is_empty() {
            return Err(SeatError::EmptyLine);
        }
        state.seats().talk().note(seat, text);
        state.record_line(format!(
            "SEAT noted seat={seat} chars={}",
            text.chars().count()
        ));
        Ok::<_, SeatError>(Inbox::Empty)
    }
    .await;
    reply(result)
}

/// `POST path::INBOX`: the next line a seat says, waiting for it up to the asked time (at most
/// `LONGEST_INBOX_WAIT`).
async fn inbox(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let result = async {
        let InboxWait { from, wait_ms } = read_body(bytes)?;
        let patience = Duration::from_millis(wait_ms).min(LONGEST_INBOX_WAIT);
        // A seat named is waited on as named, held or not; the party is the day's seats.
        let seats = match &from {
            SeatsNamed::Seats(seats) => seats.clone(),
            SeatsNamed::Party => seats_named(&state, &from).await?,
        };
        check_run(&state, &headers, "inbox", &seats).await?;
        Ok::<_, SeatError>(match state.seats().next_line(&seats, patience).await {
            Some((seat, InboxEntry::Line(line))) => Inbox::Said {
                seat,
                ask: line.ask,
                kind: line.kind,
                thing: line.thing,
                text: line.text,
                to: line.to,
                at_ms: line.at_ms,
            },
            Some((seat, InboxEntry::Cast { note, at_ms })) => Inbox::Cast {
                seat,
                spell: note.spell,
                name: note.name,
                level: note.level,
                ritual: note.ritual,
                summary: note.summary,
                at_ms,
            },
            None => Inbox::Empty,
        })
    }
    .await;
    reply(result)
}

/// The seats `named` names, each once, in the order given: the seats named, each held in the book
/// (`unknown_seat` otherwise); or the party, the seats of the day running now, else every seat that
/// plays a character, in seat order (`no_party` when there is none).
async fn seats_named(state: &AppState, named: &SeatsNamed) -> Result<Vec<u64>, SeatError> {
    let mut open = state.seats().open().await?;
    let books = open.books();
    let seats = match named {
        SeatsNamed::Seats(seats) => {
            let mut unique: Vec<u64> = Vec::with_capacity(seats.len());
            for seat in seats {
                if books.record_at(*seat).is_none() {
                    return Err(SeatError::UnknownSeat { seat: *seat });
                }
                if !unique.contains(seat) {
                    unique.push(*seat);
                }
            }
            unique
        }
        SeatsNamed::Party => {
            let running = match state.seats().scenario() {
                Some(scenario) => scenario.party().await,
                None => Vec::new(),
            };
            if running.is_empty() {
                let mut playing: Vec<u64> = books
                    .records()
                    .iter()
                    .filter(|record| record.persona.is_some())
                    .map(|record| record.seat)
                    .collect();
                playing.sort_unstable();
                playing
            } else {
                running
            }
        }
    };
    if seats.is_empty() {
        return Err(SeatError::NoParty);
    }
    Ok(seats)
}

/// The hero the player plays at their seat, one of their own (`SeatAction::Hero`).
async fn choose_hero(
    state: &Arc<AppState>,
    key: &KeyHash,
    hero: u64,
) -> Result<SeatCard, SeatError> {
    let mut open = state.seats().open().await?;
    let card = open.books().choose_hero(key, hero)?;
    open.save(Book::Seats).await?;
    drop(open);
    state.record_line(format!("SEAT hero_chosen seat={} hero={hero}", card.seat));
    // A hero locked in before a pack item existed gets it made as it comes to the table; an
    // item being made or that failed is not asked again here (`look pack` makes it again).
    if let Some(look) = state.look()
        && let Err(error) = look
            .make_pack(state, hero, None, |status| *status == PackStatus::Missing)
            .await
    {
        state.record_line(format!(
            "LOOK failed hero={hero} stage=pack cause={}",
            error.key()
        ));
    }
    Ok(card)
}

fn lobby_action(action: LobbyStep) -> LobbyAction {
    match action {
        LobbyStep::Ready => LobbyAction::Ready,
        LobbyStep::Unready => LobbyAction::Unready,
        LobbyStep::Leave => LobbyAction::Leave,
        LobbyStep::Start => LobbyAction::Start,
    }
}

/// Takes `seat` off the table (when the table has it) and out of the book.
async fn leave(state: &AppState, seat: u64) -> Result<SeatIntentReply, SeatError> {
    let mut open = state.seats().open().await?;
    let engine = state.engine()?;
    let view = table::quiet_view(engine, seat).await?;
    if table::is_seated(&view) {
        match table::act(engine, seat, LobbyAction::Leave).await? {
            ActionResult::Applied => {}
            ActionResult::Declined(reason) => return Ok(SeatIntentReply::Declined { reason }),
            ActionResult::NotApplied => return Err(SeatError::NotApplied),
        }
    }
    open.books().free(seat);
    open.save(Book::Seats).await?;
    drop(open);
    state.seats().talk().forget_seat(seat);
    // The seat is out of creation; its hero's finished build is kept for the hero (`creation`).
    crate::creation::release_seat(state, seat).await?;
    state.record_line(format!("SEAT left seat={seat}"));
    // The TV lobby follows the table as it gathers: a seat taken off it goes off the TV now.
    super::lobby::show(state).await;
    Ok(SeatIntentReply::Left)
}

/// A step of the seat in the lobby (`SeatAction::Lobby`): ready, not ready, start the day (the
/// table's scenario runs), leave the table.
async fn lobby_step(
    state: &Arc<AppState>,
    seat: u64,
    action: LobbyStep,
) -> Result<SeatIntentReply, SeatError> {
    if action == LobbyStep::Leave {
        return leave(state, seat).await;
    }
    // The party a Start begins the day for, read while the lobby still gathers: once it has
    // moved on, every seat of the book reads as seated.
    let party = if action == LobbyStep::Start {
        // A seat gone away leaves the lobby first: it neither blocks the start nor joins the day.
        let now = now_ms();
        super::presence::sweep(state, now).await?;
        let party = start_party(state, now).await?;
        check_scenario(state, seat, &party).await?;
        party
    } else {
        Vec::new()
    };
    // A seat building a hero is ready when its build is submitted (`creation`).
    if action == LobbyStep::Ready
        && let Some(reason) = crate::creation::ready_waits(state, seat).await?
    {
        return Ok(SeatIntentReply::Declined { reason });
    }
    let reply = match table::act(state.engine()?, seat, lobby_action(action)).await? {
        ActionResult::Applied => SeatIntentReply::Applied,
        ActionResult::Declined(reason) => SeatIntentReply::Declined { reason },
        ActionResult::NotApplied => return Err(SeatError::NotApplied),
    };
    if action == LobbyStep::Unready && reply == SeatIntentReply::Applied {
        crate::creation::reopen_after_unready(state, seat).await?;
    }
    if action == LobbyStep::Start && reply == SeatIntentReply::Applied {
        super::room::close_lobby(state).await;
        start_scenario(state, seat, party).await?;
        crate::creation::start_play(state).await;
    }
    Ok(reply)
}

/// The party a Start begins the day for, read while the lobby gathers: the book's records of every
/// seat the lobby has seated and that is not away at `now_ms` (`presence`), in seat order. A seat
/// away never joins, whatever character it is bound to.
pub(crate) async fn start_party(
    state: &AppState,
    now_ms: u64,
) -> Result<Vec<SeatRecord>, SeatError> {
    let mut records = state.seats().open().await?.books().records().to_vec();
    records.sort_by_key(|record| record.seat);
    let engine = state.engine()?;
    let mut party = Vec::with_capacity(records.len());
    for record in records {
        let view = table::quiet_view(engine, record.seat).await?;
        let here = !super::presence::is_away(record.last_seen_ms, now_ms);
        if table::is_gathering(&view) && table::is_seated(&view) && here {
            party.push(record);
        }
    }
    Ok(party)
}

/// That the table's scenario, when it has one, can start the day for the whole party: every seat
/// at the table has its hero in the library with its look locked in (`no_hero_to_play` for the
/// seat starting, `party_not_ready` naming another). Checked before the lobby starts (which waits
/// for every seat to be ready), so the table never plays with nothing running.
async fn check_scenario(
    state: &AppState,
    seat: u64,
    party: &[SeatRecord],
) -> Result<(), SeatError> {
    if state.seats().scenario().is_none() {
        return Ok(());
    }
    if !party.iter().any(|record| record.seat == seat) {
        return Err(SeatError::UnknownSeat { seat });
    }
    let mut open = state.seats().open().await?;
    let books = open.books();
    match party
        .iter()
        .find(|record| !books.card_of(record).hero_locked)
    {
        None => Ok(()),
        Some(record) if record.seat == seat => Err(SeatError::NoHeroToPlay),
        Some(record) => Err(SeatError::PartyNotReady { seat: record.seat }),
    }
}

/// The game started from a phone: the table's scenario, when it has one, starts one day for the
/// whole party (every seat at the table, in seat order), and each seat now plays its hero's
/// persona.
async fn start_scenario(
    state: &Arc<AppState>,
    seat: u64,
    party: Vec<SeatRecord>,
) -> Result<(), SeatError> {
    let Some(scenario) = state.seats().scenario() else {
        return Ok(());
    };
    let launches = {
        let mut open = state.seats().open().await?;
        let books = open.books();
        let mut launches = Vec::with_capacity(party.len());
        for record in &party {
            let hero = record.hero.and_then(|hero| books.hero(hero));
            let Some(launch) = HeroLaunch::of(record, hero) else {
                let unbound: Vec<(u64, Option<String>)> =
                    party.iter().map(|record| (record.seat, None)).collect();
                drop(open);
                scenario::not_started(state, &unbound, "no_hero").await;
                return Ok(());
            };
            launches.push(launch);
        }
        for launch in &launches {
            books.bind_persona(launch.seat, Some(&persona_of(launch.hero.id)))?;
        }
        open.save(Book::Seats).await?;
        launches
    };
    if launches.is_empty() {
        scenario::not_started(state, &[(seat, None)], "no_party").await;
        return Ok(());
    }
    scenario::launch(state, scenario, launches).await;
    Ok(())
}

/// `GET path::PLAYERS`.
async fn players(State(state): State<Arc<AppState>>) -> Response {
    let result = async {
        Ok::<PlayersReport, SeatError>(
            state
                .seats()
                .open()
                .await?
                .books()
                .report(state.seats().room_code()),
        )
    }
    .await;
    reply(result)
}

/// `POST path::FRIEND_ADD`.
async fn friend_add(
    State(state): State<Arc<AppState>>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let result = async {
        let FriendAdd { name } = read_body(bytes)?;
        let mut open = state.seats().open().await?;
        let friend = open.books().add_friend(checked_name(&name)?)?;
        open.save(Book::Friends).await?;
        state.record_line(format!("SEAT friend_added profile={}", friend.id));
        Ok::<_, SeatError>(friend)
    }
    .await;
    reply(result)
}

/// `POST path::FRIEND_REMOVE`.
async fn friend_remove(
    State(state): State<Arc<AppState>>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let result = async {
        let FriendRemove { profile } = read_body(bytes)?;
        let mut open = state.seats().open().await?;
        open.books().remove_friend(profile)?;
        open.save(Book::Friends).await?;
        state.record_line(format!("SEAT friend_removed profile={profile}"));
        Ok::<_, SeatError>(open.books().report(state.seats().room_code()))
    }
    .await;
    reply(result)
}

/// `POST path::APPROVE`: the friend takes the seat their profile holds (handed over from another
/// phone), the seat this key holds, or the first free one.
async fn approve(
    State(state): State<Arc<AppState>>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let result = async {
        let ApprovalAnswer { request } = read_body(bytes)?;
        let mut open = state.seats().open().await?;
        let engine = state.engine()?;
        let books = open.books();
        let pending = books.waiting(request)?.clone();
        let name = books
            .friend(pending.profile)
            .ok_or(SeatError::UnknownFriend {
                profile: pending.profile,
            })?
            .name
            .clone();
        let own = books.seat_of(&pending.key).map(|record| record.seat);
        let held = books.profile_seat(pending.profile).or(own);
        let free = books.free_seats();
        let seat = match held {
            Some(seat) => {
                table::sit_down(engine, seat).await?;
                seat
            }
            None => table::take_free_seat(engine, &free).await?,
        };
        // The phone moves to the friend's seat: the seat it held leaves the table, and a seat
        // that changes hands forgets what it was asked.
        if let Some(old) = own.filter(|old| *old != seat) {
            let view = table::quiet_view(engine, old).await?;
            if table::is_seated(&view) {
                table::act(engine, old, LobbyAction::Leave).await?;
            }
            state.seats().talk().forget_seat(old);
            state.record_line(format!("SEAT left seat={old} moved_to={seat}"));
        }
        if own != Some(seat) {
            state.seats().talk().forget_seat(seat);
        }
        let books = open.books();
        books.take_waiting(request)?;
        let holder = Holder::Friend {
            profile: pending.profile,
        };
        let card = books.seat(&pending.key, seat, holder, &name, now_ms());
        open.save(Book::Seats).await?;
        state.record_line(format!("SEAT approved request={request} seat={seat}"));
        record_seated(&state, &card);
        Ok::<_, SeatError>(Approved { seat: card })
    }
    .await;
    reply(result)
}

/// `POST path::REFUSE`.
async fn refuse(
    State(state): State<Arc<AppState>>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let result = async {
        let ApprovalAnswer { request } = read_body(bytes)?;
        let mut open = state.seats().open().await?;
        open.books().refuse(request)?;
        state.record_line(format!(
            "SEAT refused reason=refused_by_host request={request}"
        ));
        Ok::<_, SeatError>(open.books().report(state.seats().room_code()))
    }
    .await;
    reply(result)
}

/// `POST path::HERO_ADD`.
async fn hero_add(
    State(state): State<Arc<AppState>>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let result = async {
        let HeroAdd {
            seat,
            name,
            species,
            class,
        } = read_body(bytes)?;
        let mut open = state.seats().open().await?;
        let hero = open.books().add_hero(
            seat,
            checked_name(&name)?,
            checked_name(&species)?,
            checked_name(&class)?,
        )?;
        open.save(Book::Heroes).await?;
        state.record_line(format!("SEAT hero_added seat={seat} hero={}", hero.id));
        Ok::<_, SeatError>(HeroAdded { hero })
    }
    .await;
    reply(result)
}

/// `POST path::SEAT_REMOVE`.
async fn seat_remove(
    State(state): State<Arc<AppState>>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let result = async {
        let SeatRemove { seat } = read_body(bytes)?;
        let known = state
            .seats()
            .open()
            .await?
            .books()
            .record_at(seat)
            .is_some();
        if !known {
            return Err(SeatError::UnknownSeat { seat });
        }
        leave(&state, seat).await?;
        Ok::<_, SeatError>(
            state
                .seats()
                .open()
                .await?
                .books()
                .report(state.seats().room_code()),
        )
    }
    .await;
    reply(result)
}

/// The phone's seat routes, for the player listener and the loopback listener.
pub(crate) fn player_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(PLAY_ROOM, get(room))
        .route(PLAY_HELLO, post(hello))
        .route(PLAY_JOIN, post(join))
        .route(PLAY_JOIN_CODE, post(join_code_check))
        .route(PLAY_JOIN_STATUS, get(join_status))
        .route(PLAY_HEROES, get(heroes))
        .route(PLAY_SEAT_VIEW, get(seat_view))
        .route(PLAY_SEAT_ACTION, post(seat_action))
        // A recording is larger than any other body a phone sends: its route takes the speech
        // service's own bound.
        .route(
            PLAY_SEAT_TRANSCRIBE,
            post(seat_transcribe).layer(axum::extract::DefaultBodyLimit::max(
                crate::guard::recording_limit(PLAY_SEAT_TRANSCRIBE),
            )),
        )
        .with_state(Arc::clone(&state))
        // A seat's hero build after Lock in, on both listeners like the seat's other routes.
        .merge(crate::creation::routes::player_router(Arc::clone(&state)))
        // A seat's roll of its hero sheet's checks, likewise.
        .merge(crate::persona::check::seat_router(state))
}

#[cfg(test)]
mod routes_tests;

/// The dev routes that manage the players books, for the loopback listener only.
pub(crate) fn dev_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(path::PLAYERS, get(players))
        .route(path::FRIEND_ADD, post(friend_add))
        .route(path::FRIEND_REMOVE, post(friend_remove))
        .route(path::APPROVE, post(approve))
        .route(path::REFUSE, post(refuse))
        .route(path::HERO_ADD, post(hero_add))
        .route(path::SEAT_REMOVE, post(seat_remove))
        .route(path::PERSONA, post(bind_persona))
        .route(path::ASK, post(set_ask))
        .route(path::ASK_CLEAR, post(clear_ask))
        .route(path::NOTE, post(note))
        .route(path::INBOX, post(inbox))
        .with_state(state)
}

//! The seat routes: the phone's (`df_playground::path::PLAY_*`, on the player listener and the
//! loopback listener) and the dev routes that manage the players books (`path`, loopback only).
//!
//! Every player route reads the seat key from its header and takes the seat from the book, never
//! from the body; every answer is a `SeatReply`. Each step writes one `SEAT` line with the seat,
//! the kind or the reason, never the key or its hash and never a name.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::extract::rejection::BytesRejection;
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, post};
use df_playground::overlay::Edit;
use df_playground::path::{
    PLAY_HELLO, PLAY_HERO, PLAY_HEROES, PLAY_JOIN, PLAY_JOIN_STATUS, PLAY_ROOM, PLAY_SEAT_INTENT,
    PLAY_SEAT_VIEW,
};
use df_playground::seat::wire::{
    HelloReply, HeroChoice, HeroesReply, JoinRequest, JoinStatus, RoomReply, RoomState, SeatAction,
    SeatCard, SeatIntent, SeatIntentReply, SeatView, TablePlayer, TableStage,
};
use df_rules::local_journey::valid_character_name;
use df_table::lobby::LobbyAction;
use df_table::ports::Clock;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::book::{Book, Holder, SeatRecord};
use super::key::KeyHash;
use super::request::{
    ApprovalAnswer, Approved, FriendAdd, FriendRemove, HeroAdd, HeroAdded, PlayersReport,
    SeatError, SeatRemove,
};
use super::table::{self, ActionResult};
use crate::dev_http::{BodyCause, BodyError, parse_body, reply};
use crate::engine::request::AudienceSpec;
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

fn body<T: DeserializeOwned>(bytes: Result<Bytes, BytesRejection>) -> Result<T, SeatError> {
    let bytes = bytes.map_err(|rejection| {
        SeatError::InvalidBody(BodyError {
            cause: BodyCause::Truncated,
            line: 0,
            column: 0,
            detail: rejection.body_text(),
        })
    })?;
    Ok(parse_body(&bytes)?)
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
        let HelloBody {} = body(bytes)?;
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
        if !seated {
            return Ok(HelloReply::Returning {
                seat: record.card(),
                friends,
            });
        }
        open.books().touch(&key, now_ms());
        open.save(Book::Seats).await?;
        Ok::<_, SeatError>(HelloReply::Seated {
            seat: record.card(),
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
        JoinRequest::Account => "account",
    }
}

/// `POST PLAY_JOIN`.
async fn join(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let result = async {
        let key = KeyHash::from_headers(&headers)?;
        let request: JoinRequest = body(bytes)?;
        state.record_line(format!("SEAT join_requested kind={}", join_kind(&request)));
        match request {
            JoinRequest::Account => Err(SeatError::AccountsNotAvailable),
            JoinRequest::Returning => rejoin(&state, &key).await,
            JoinRequest::Guest { name } => seat_guest(&state, &key, checked_name(&name)?).await,
            JoinRequest::Friend { profile } => {
                let mut open = state.seats().open().await?;
                let request = open.books().ask_as_friend(&key, profile, now_ms())?;
                Ok(JoinStatus::AwaitingApproval { request })
            }
        }
    }
    .await;
    answer_phone(&state, result)
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
        seat: record.card(),
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
            table::take_free_seat(engine, &free).await?
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
async fn table_players(
    state: &AppState,
    records: &[SeatRecord],
    own: Option<&KeyHash>,
) -> Result<Vec<TablePlayer>, SeatError> {
    let engine = state.engine()?;
    let mut players = Vec::with_capacity(records.len());
    for record in records {
        let view = table::quiet_view(engine, record.seat).await?;
        players.push(TablePlayer {
            name: record.name.clone(),
            present: table::is_seated(&view),
            ready: table::is_ready(&view),
            you: own == Some(&record.key),
        });
    }
    Ok(players)
}

/// `GET PLAY_SEAT_VIEW`: the seat's view (one `ENGINE wire_view` line) as typed data, and who is
/// at the table.
async fn seat_view(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let result = async {
        let key = KeyHash::from_headers(&headers)?;
        let (seat, records) = {
            let mut open = state.seats().open().await?;
            let books = open.books();
            let seat = books.seat_of(&key).ok_or(SeatError::UnknownKey)?.seat;
            books.touch(&key, now_ms());
            (seat, books.records().to_vec())
        };
        let view = state
            .engine()?
            .machine_view(AudienceSpec::Seat { seat })
            .await?;
        let players = table_players(&state, &records, Some(&key)).await?;
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
            players,
        })
    }
    .await;
    reply(result)
}

/// `POST PLAY_HERO`: the hero the player plays at their seat, one of their own.
async fn choose_hero(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let result = async {
        let key = KeyHash::from_headers(&headers)?;
        let HeroChoice { hero } = body(bytes)?;
        let mut open = state.seats().open().await?;
        let card = open.books().choose_hero(&key, hero)?;
        open.save(Book::Seats).await?;
        state.record_line(format!("SEAT hero_chosen seat={} hero={hero}", card.seat));
        Ok::<_, SeatError>(card)
    }
    .await;
    answer_phone(&state, result)
}

fn lobby_action(action: SeatAction) -> LobbyAction {
    match action {
        SeatAction::Ready => LobbyAction::Ready,
        SeatAction::Unready => LobbyAction::Unready,
        SeatAction::Leave => LobbyAction::Leave,
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
    state.record_line(format!("SEAT left seat={seat}"));
    Ok(SeatIntentReply::Left)
}

/// `POST PLAY_SEAT_INTENT`.
async fn seat_intent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let result = async {
        let key = KeyHash::from_headers(&headers)?;
        let SeatIntent { action } = body(bytes)?;
        let seat = state
            .seats()
            .open()
            .await?
            .books()
            .seat_of(&key)
            .ok_or(SeatError::UnknownKey)?
            .seat;
        if action == SeatAction::Leave {
            return leave(&state, seat).await;
        }
        Ok(
            match table::act(state.engine()?, seat, lobby_action(action)).await? {
                ActionResult::Applied => SeatIntentReply::Applied,
                ActionResult::Declined(reason) => SeatIntentReply::Declined { reason },
                ActionResult::NotApplied => return Err(SeatError::NotApplied),
            },
        )
    }
    .await;
    reply(result)
}

/// `GET path::PLAYERS`.
async fn players(State(state): State<Arc<AppState>>) -> Response {
    let result =
        async { Ok::<PlayersReport, SeatError>(state.seats().open().await?.books().report()) }
            .await;
    reply(result)
}

/// `POST path::FRIEND_ADD`.
async fn friend_add(
    State(state): State<Arc<AppState>>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let result = async {
        let FriendAdd { name } = body(bytes)?;
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
        let FriendRemove { profile } = body(bytes)?;
        let mut open = state.seats().open().await?;
        open.books().remove_friend(profile)?;
        open.save(Book::Friends).await?;
        state.record_line(format!("SEAT friend_removed profile={profile}"));
        Ok::<_, SeatError>(open.books().report())
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
        let ApprovalAnswer { request } = body(bytes)?;
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
        let held = books
            .profile_seat(pending.profile)
            .or_else(|| books.seat_of(&pending.key).map(|record| record.seat));
        let free = books.free_seats();
        let seat = match held {
            Some(seat) => {
                table::sit_down(engine, seat).await?;
                seat
            }
            None => table::take_free_seat(engine, &free).await?,
        };
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
        let ApprovalAnswer { request } = body(bytes)?;
        let mut open = state.seats().open().await?;
        open.books().refuse(request)?;
        state.record_line(format!(
            "SEAT refused reason=refused_by_host request={request}"
        ));
        Ok::<_, SeatError>(open.books().report())
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
        } = body(bytes)?;
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
        let SeatRemove { seat } = body(bytes)?;
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
        Ok::<_, SeatError>(state.seats().open().await?.books().report())
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
        .route(PLAY_JOIN_STATUS, get(join_status))
        .route(PLAY_HEROES, get(heroes))
        .route(PLAY_HERO, post(choose_hero))
        .route(PLAY_SEAT_VIEW, get(seat_view))
        .route(PLAY_SEAT_INTENT, post(seat_intent))
        .with_state(state)
}

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
        .with_state(state)
}

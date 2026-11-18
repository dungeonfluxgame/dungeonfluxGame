//! The TV lobby while the table gathers (`planning/phone-and-creation.md` P1 "TV lobby" and the
//! look studio's TV bullet): the room code and its QR, and one card per seat of the table, fed
//! from the seat book and creation, drawn on the existing `LobbyScreen`. Nothing private: a seat
//! shows its player's name, then "Making their hero…", then the locked hero's name, class and
//! picture with Ready; a seat gone away (`presence`) says Away.
//!
//! When the lobby opens (boot, and back from a day) and no day runs, the TV goes to the clean
//! fixture (`LOBBY_FIXTURE`, the one a day's runner starts from) through the playground's one
//! change path, without saving it as the owner's choice, so no scene, caption or sample party is
//! behind the lobby. The lobby stands on the adventure's picture, else the lobby hall plate.
//! While the table gathers, every look at the table (a seat view, the room, a phone seen) puts
//! the lobby up again when it changed. Start takes it down (`room::close_lobby`).
//!
//! Lines: `SEAT lobby shown seats= tv=shown|<status>`, `SEAT lobby backdrop tv=`.

use std::sync::Arc;
use std::time::Duration;

use df_playground::lobby::change::{
    JoinEdit, LobbyLine, LobbyPart, PartyEdit, ScreenEdit, SeatEdit,
};
use df_playground::lobby::model::{
    JoinCode, LobbyScreen, LobbySeat, PartySummary, SeatState, SummaryIcon,
};
use df_playground::overlay::Edit;
use df_playground::scene::{ImageSlot, SceneChange};
use df_playground::seat::wire::TablePlayer;

use super::table;
use crate::dev_http::RouteError;
use crate::engine::limits::DEV_SEAT_COUNT;
use crate::engine::report::EngineError;
use crate::engine::request::AudienceSpec;
use crate::state::AppState;

/// The fixture the TV shows behind the lobby: the clean one a day's runner starts from (no
/// figures, caption, card, party or title).
pub(crate) const LOBBY_FIXTURE: &str = "painters_day";
/// The lobby's picture when the adventure has none.
pub(crate) const LOBBY_PLATE: &str = "lobby-hall-plate.webp";
/// What a seat's card says while its player chooses a hero.
const CHOOSING_WORDS: &str = "Choosing a hero";
/// What a seat's card says while its player makes their hero.
const MAKING_WORDS: &str = "Making their hero…";
/// The badge of a seat gone away.
const AWAY_WORD: &str = "Away";
/// The badge of a locked-in hero whose player is not ready yet.
const NOT_READY_WORD: &str = "Not ready";

/// The lobby's look, taken when it opens: the adventure's title and picture, which the clean
/// fixture does not carry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LobbyLook {
    pub(crate) title: Option<String>,
    pub(crate) picture: Option<String>,
}

/// The lobby as last put up, so an unchanged one is not put up again.
#[derive(Debug, Default)]
pub(crate) struct LobbyShown {
    pub(crate) look: LobbyLook,
    last: Option<LobbyScreen>,
}

impl LobbyShown {
    /// Forgets the lobby put up: the next look puts it up again.
    pub(crate) fn forget(&mut self) {
        self.last = None;
    }
}

/// What a seat's card shows: the player, whether they are here and ready, and their hero once
/// they chose one (with whether its look is locked in).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SeatFacts {
    pub(crate) seat: u64,
    pub(crate) player: TablePlayer,
    pub(crate) locked: bool,
}

/// The card of one held seat.
fn seat_card(facts: &SeatFacts) -> LobbySeat {
    let player = &facts.player;
    let hero = player.hero.as_ref().filter(|_| facts.locked);
    let mut card = LobbySeat {
        state: SeatState::Creating,
        name: Some(player.name.clone()),
        class: None,
        glyph: None,
        level: None,
        portrait: None,
        host: false,
        progress_percent: None,
        step: None,
        stage: None,
        badge: None,
        controller: false,
    };
    match hero {
        Some(hero) => {
            card.state = SeatState::LockedIn;
            card.name = Some(hero.name.clone());
            card.class = Some(hero.class.clone());
            card.portrait = hero
                .pack
                .headshot
                .picture()
                .map(str::to_owned)
                .or_else(|| hero.portrait.clone());
            card.badge = (!player.ready).then(|| NOT_READY_WORD.to_owned());
        }
        None if player.hero.is_some() => card.stage = Some(MAKING_WORDS.to_owned()),
        None => card.stage = Some(CHOOSING_WORDS.to_owned()),
    }
    if !player.present {
        card.badge = Some(AWAY_WORD.to_owned());
    }
    card
}

/// The lobby of `seats` (the held seats, by seat) with the look `look` and the join panel `join`:
/// the title, one card per seat of the table (an open one where nobody sits), the join panel and
/// how many are here and ready. No begin button: the phones start the game.
pub(crate) fn lobby_screen(look: &LobbyLook, join: JoinCode, seats: &[SeatFacts]) -> LobbyScreen {
    let cards = (1..=DEV_SEAT_COUNT)
        .map(|seat| match seats.iter().find(|facts| facts.seat == seat) {
            Some(facts) => seat_card(facts),
            None => open_seat(),
        })
        .collect();
    let here = seats.iter().filter(|facts| facts.player.present).count();
    let ready = seats
        .iter()
        .filter(|facts| facts.player.present && facts.player.ready)
        .count();
    let count = |value: usize| u8::try_from(value).unwrap_or(u8::MAX);
    LobbyScreen {
        eyebrow: None,
        title: look.title.clone().unwrap_or_default(),
        subtitle: None,
        seats: cards,
        join: Some(join),
        party: Some(PartySummary {
            joined: count(here),
            capacity: count(usize::try_from(DEV_SEAT_COUNT).unwrap_or(usize::MAX)),
            word: Some("Here".to_owned()),
            detail: Some(format!("{ready} ready")),
            icon: SummaryIcon::People,
        }),
        begin: None,
        focus: None,
    }
}

fn open_seat() -> LobbySeat {
    LobbySeat {
        state: SeatState::Open,
        name: None,
        class: None,
        glyph: None,
        level: None,
        portrait: None,
        host: false,
        progress_percent: None,
        step: None,
        stage: None,
        badge: None,
        controller: false,
    }
}

/// The live changes that put `screen` up whole, from a cleared lobby.
fn screen_edits(screen: &LobbyScreen) -> Vec<SceneChange> {
    let mut edits = vec![ScreenEdit::LobbyClear {}];
    if !screen.title.trim().is_empty() {
        edits.push(ScreenEdit::LobbyLine {
            field: LobbyLine::Title,
            text: screen.title.clone(),
        });
    }
    edits.push(ScreenEdit::LobbySeats {
        count: screen.seats.len(),
    });
    for (index, seat) in screen.seats.iter().enumerate() {
        if seat.state == SeatState::Open {
            continue;
        }
        edits.push(ScreenEdit::LobbySeat {
            number: index + 1,
            seat: SeatEdit {
                state: Some(seat.state),
                name: seat.name.clone(),
                class: seat.class.clone(),
                portrait: seat.portrait.clone(),
                stage: seat.stage.clone(),
                badge: seat.badge.clone(),
                ..SeatEdit::default()
            },
        });
    }
    if let Some(join) = &screen.join {
        edits.push(ScreenEdit::LobbyJoin {
            join: JoinEdit {
                hint: join.hint.clone(),
                join_link: join.join_link.clone(),
                game_code: join.game_code.clone(),
                qr: join.qr.clone(),
            },
        });
    }
    if let Some(party) = &screen.party {
        edits.push(ScreenEdit::LobbyParty {
            party: PartyEdit {
                joined: Some(party.joined),
                capacity: Some(party.capacity),
                word: party.word.clone(),
                detail: party.detail.clone(),
                icon: Some(party.icon),
            },
        });
    }
    edits.push(ScreenEdit::LobbyRemove {
        part: LobbyPart::Begin,
    });
    edits
        .into_iter()
        .map(|edit| SceneChange::Screen { edit })
        .collect()
}

/// Whether the table gathers now (the lobby, no day started).
async fn gathering(state: &AppState) -> bool {
    match state.engine() {
        Ok(engine) => engine
            .intent_view(AudienceSpec::Tv)
            .await
            .is_ok_and(|view| table::is_gathering(&view)),
        Err(_) => false,
    }
}

/// The held seats as their cards need them, by seat.
async fn seat_facts(state: &AppState) -> Option<Vec<SeatFacts>> {
    let records = state.seats().open().await.ok()?.books().records().to_vec();
    let players = super::routes::table_players(state, &records, None)
        .await
        .ok()?;
    let mut open = state.seats().open().await.ok()?;
    let books = open.books();
    Some(
        records
            .iter()
            .zip(players)
            .map(|(record, player)| SeatFacts {
                seat: record.seat,
                locked: books.card_of(record).hero_locked,
                player,
            })
            .collect(),
    )
}

/// How long the lobby's opening at boot waits for the table's engine, which the server starts as
/// it begins to serve.
const ENGINE_WAIT: Duration = Duration::from_secs(10);
/// How often the opening looks for the engine while it waits.
const ENGINE_LOOK: Duration = Duration::from_millis(50);

/// Whether the table's engine runs, waiting up to `ENGINE_WAIT` for it while it is not set yet
/// (the boot: the server starts it as it begins to serve). An engine that failed to start is not
/// waited for.
pub(crate) async fn engine_started(state: &AppState) -> bool {
    let deadline = tokio::time::Instant::now() + ENGINE_WAIT;
    loop {
        match state.engine() {
            Ok(_) => return true,
            Err(EngineError::Stopped) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(ENGINE_LOOK).await;
            }
            Err(_) => return false,
        }
    }
}

/// The lobby opens: unless a day runs, the TV goes to the clean fixture (not saved as the owner's
/// choice), the lobby stands on the adventure's picture (else the hall plate) and is put up. The
/// TV's answer, for the caller's line.
pub(crate) async fn open(state: &Arc<AppState>) -> String {
    if !engine_started(state).await {
        return "no_engine".to_owned();
    }
    let running = match state.seats().scenario() {
        Some(scenario) => !scenario.party().await.is_empty(),
        None => false,
    };
    if running || !gathering(state).await {
        return "playing".to_owned();
    }
    // No seat plays a day left from before: a binding to a hero's day persona goes (a closed tab's
    // half-made hero never reaches a run).
    super::scenario::clear_bindings(state).await;
    // The adventure's title and picture are the landing page's: taken before the clean fixture
    // clears the live scene.
    let look = match state.playground().snapshot().scene.home {
        Edit::Set(home) => home
            .adventure
            .map_or_else(LobbyLook::default, |adventure| LobbyLook {
                title: Some(adventure.title).filter(|title| !title.trim().is_empty()),
                picture: adventure.picture,
            }),
        Edit::Keep | Edit::Clear => LobbyLook::default(),
    };
    if let Err(error) = crate::playground::go_fixture(state, LOBBY_FIXTURE).await {
        state.record_line(format!(
            "SEAT lobby fixture_refused status={}",
            error.status().as_u16()
        ));
    }
    {
        let mut shown = state.seats().lobby();
        shown.look = look.clone();
        shown.forget();
    }
    let backdrop = SceneChange::Image {
        slot: ImageSlot::TvBackdrop,
        media: look.picture.unwrap_or_else(|| LOBBY_PLATE.to_owned()),
        caption: None,
        rotation_degrees: 0,
        slide: None,
        layout: None,
        plate: df_playground::fixture::PlateKind::default(),
    };
    // A picture the media library lacks leaves the lobby on the plain stage, never off the TV.
    let painted = crate::playground::apply_scene_changes(state, vec![backdrop])
        .await
        .map_or_else(
            |error| error.status.as_u16().to_string(),
            |_| "shown".to_owned(),
        );
    state.record_line(format!("SEAT lobby backdrop tv={painted}"));
    show(state).await
}

/// Puts the lobby up while the table gathers, when it changed since it was last put up. The TV's
/// answer, for the caller's line; `unchanged` or `playing` when nothing was put up.
pub(crate) async fn show(state: &AppState) -> String {
    if !gathering(state).await {
        return "playing".to_owned();
    }
    let Some(seats) = seat_facts(state).await else {
        return "unreadable".to_owned();
    };
    let code = state.seats().room_code();
    let join = super::room::join_code(state, &code);
    let screen = {
        let shown = state.seats().lobby();
        let screen = lobby_screen(&shown.look, join, &seats);
        if shown.last.as_ref() == Some(&screen) {
            return "unchanged".to_owned();
        }
        screen
    };
    let shown = crate::playground::apply_scene_changes(state, screen_edits(&screen))
        .await
        .map_or_else(
            |error| error.status.as_u16().to_string(),
            |_| "shown".to_owned(),
        );
    state.record_line(format!("SEAT lobby shown seats={} tv={shown}", seats.len()));
    if shown == "shown" {
        state.seats().lobby().last = Some(screen);
    }
    shown
}

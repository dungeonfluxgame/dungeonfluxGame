//! Seats for phones: who a phone is and which seat it holds, from the room to the table and back
//! (`planning/phone-and-creation.md` P1, "Who is playing" and "Getting heroes back").
//!
//! A phone keeps a random 256-bit seat key (REF-046) and sends it only in
//! `path::SEAT_KEY_HEADER`; the host keeps its SHA-256 hash (`key`) and never logs either. The
//! seat book (`book`) maps the hash to a seat of the table (1 to six, REF-062), the kind of player
//! and their name; it is a file in the playground's directory, so a phone comes back to its seat
//! after a refresh or a host restart. A guest names themselves; a friend picks a profile of the
//! friends book and waits for the host to approve it (`players approve`); signing in to one's own
//! account is refused `accounts_not_available` on the dev host. The phone never decides its seat
//! or kind: every seat comes from the key, never from a body. A new way in carries the room code
//! the TV lobby shows (`room`); a key that holds a seat comes back without it.
//!
//! Joining, readying and leaving go through the lobby machine as that seat, through the wire
//! intent's one path (`table`). The routes (`routes`) answer the phone on the player listener and
//! the loopback listener, and the dev routes and verbs (`players ...`, `cli`) manage friends,
//! approvals, seats and sample heroes. Every step is one `SEAT` line: `hello`, `join_requested`,
//! `approved`, `refused`, `seated`, `rejoined`, `left`.

pub(crate) mod book;
pub mod cli;
pub(crate) mod key;
pub(crate) mod lobby;
pub(crate) mod presence;
pub mod request;
pub(crate) mod room;
pub mod routes;
pub mod scenario;
pub(crate) mod table;
pub(crate) mod talk;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use df_table::ports::Clock;
use tokio::sync::Notify;

use self::book::{Book, Books, write_book};
use self::lobby::LobbyShown;
use self::presence::Presence;
use self::request::SeatError;
use self::room::{Missed, RoomDoor};
use self::scenario::Scenario;
use self::talk::{CastNote, InboxEntry, SeatTalk};
use crate::blocking::run_blocking;
use crate::io_cause::IoCause;
use crate::observe::WallClock;

pub(crate) use self::book::PLAYERS_DIRECTORY;
pub use self::cli::{PlayersCommand, parse};

/// What a blocking file task that stopped without an answer is.
fn stopped() -> SeatError {
    SeatError::Storage {
        cause: IoCause::Other,
    }
}

/// The players books behind one lock, read from their files on first use.
pub struct SeatDesk {
    directory: PathBuf,
    books: tokio::sync::Mutex<Option<Books>>,
    /// The asks, the inboxes and the lines heard. Held briefly, never across an `.await`.
    talk: Mutex<SeatTalk>,
    /// Woken when a seat says a line, so a runner waiting for it hears it at once.
    said: Notify,
    /// The scenario the table plays when a phone starts the game (`serve --scenario`).
    scenario: OnceLock<Scenario>,
    /// The room code a new phone carries, and the misses from each address. Held briefly.
    door: Mutex<RoomDoor>,
    /// The seats away now (`presence`). Held briefly, never across an `.await`.
    presence: Mutex<Presence>,
    /// The TV lobby as last put up (`lobby`). Held briefly, never across an `.await`.
    lobby: Mutex<LobbyShown>,
}

/// The books, locked, for one route's decision and the writes it makes.
pub(crate) struct OpenBooks<'desk> {
    guard: tokio::sync::MutexGuard<'desk, Option<Books>>,
    directory: &'desk Path,
}

impl SeatDesk {
    /// A desk over the books in `directory` (read when first asked for).
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            books: tokio::sync::Mutex::new(None),
            talk: Mutex::new(SeatTalk::seeded(WallClock.now_ms())),
            said: Notify::new(),
            scenario: OnceLock::new(),
            door: Mutex::new(RoomDoor::new()),
            presence: Mutex::new(Presence::default()),
            lobby: Mutex::new(LobbyShown::default()),
        }
    }

    /// The TV lobby as last put up, locked briefly.
    pub(crate) fn lobby(&self) -> MutexGuard<'_, LobbyShown> {
        self.lobby.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The seats away now, locked briefly.
    pub(crate) fn presence(&self) -> MutexGuard<'_, Presence> {
        self.presence.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn door(&self) -> MutexGuard<'_, RoomDoor> {
        self.door.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The room code a new phone carries now.
    pub(crate) fn room_code(&self) -> String {
        self.door().code().to_owned()
    }

    /// A new room code: the table's lobby opens again.
    pub(crate) fn renew_code(&self) {
        self.door().renew();
    }

    /// Whether `typed` is the room code, from `address`; a miss says how long its answer waits.
    pub(crate) fn check_code(&self, address: &str, typed: &str) -> Result<(), Missed> {
        self.door().check(address, typed, WallClock.now_ms())
    }

    /// Names the scenario the table plays when a phone starts the game. A second call keeps the
    /// first: the host has one table.
    pub fn set_scenario(&self, scenario: Scenario) {
        // The first call wins; serving starts once.
        let _ = self.scenario.set(scenario);
    }

    pub(crate) fn scenario(&self) -> Option<&Scenario> {
        self.scenario.get()
    }

    /// The asks, the inboxes and the lines heard, locked briefly.
    pub(crate) fn talk(&self) -> MutexGuard<'_, SeatTalk> {
        self.talk.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Notes a line the table heard: a character's spoken line (`speaker` is its id) or the
    /// narrator's (`None`).
    pub(crate) fn hear(&self, speaker: Option<&str>, text: &str) {
        self.talk().hear(speaker, text);
    }

    /// Takes `text` as `seat`'s answer to what it is asked, and wakes a runner waiting for it.
    pub(crate) fn say(&self, seat: u64, line: talk::Line) -> Result<Option<u64>, SeatError> {
        let ask = self.talk().say(seat, line, WallClock.now_ms())?;
        self.said.notify_waiters();
        Ok(ask)
    }

    /// Puts the spell `note` `seat`'s hero cast in its inbox, and wakes a runner waiting for it.
    pub(crate) fn cast(&self, seat: u64, note: CastNote) {
        self.talk().cast(seat, note, WallClock.now_ms());
        self.said.notify_waiters();
    }

    /// The next entry of one of `seats` (the oldest waiting), with its seat, waiting up to
    /// `patience` for one; `None` when none came.
    pub(crate) async fn next_line(
        &self,
        seats: &[u64],
        patience: Duration,
    ) -> Option<(u64, InboxEntry)> {
        let deadline = tokio::time::Instant::now() + patience;
        loop {
            // Made before looking, so a line said between the look and the wait still wakes it.
            let said = self.said.notified();
            if let Some(line) = self.talk().take_line(seats) {
                return Some(line);
            }
            if tokio::time::timeout_at(deadline, said).await.is_err() {
                return None;
            }
        }
    }

    /// Locks the books, reading them from their files the first time. A file that cannot be read
    /// is refused `Storage` and nothing is cached, so the next call reads again.
    ///
    /// The lock is held across the work of a route (the engine's join and the file write), so two
    /// phones never take one seat. If a request is dropped mid-way, what it already did stays:
    /// a seat the engine took with no book entry is passed over by the next allocation
    /// (`table::take_free_seat`), and a book changed in memory is written by the next change.
    pub(crate) async fn open(&self) -> Result<OpenBooks<'_>, SeatError> {
        let mut guard = self.books.lock().await;
        if guard.is_none() {
            let directory = self.directory.clone();
            let loaded = run_blocking(move || Books::load(&directory), stopped()).await?;
            *guard = Some(loaded);
        }
        Ok(OpenBooks {
            guard,
            directory: &self.directory,
        })
    }
}

impl OpenBooks<'_> {
    pub(crate) fn books(&mut self) -> &mut Books {
        // `open` filled the slot before handing out the guard.
        self.guard.get_or_insert_with(Books::default)
    }

    /// Writes `book` to its file, off the async workers.
    pub(crate) async fn save(&mut self, book: Book) -> Result<(), SeatError> {
        let write = self.books().write_of(book)?;
        let directory = self.directory.to_path_buf();
        run_blocking(move || write_book(&directory, &write), stopped()).await
    }
}

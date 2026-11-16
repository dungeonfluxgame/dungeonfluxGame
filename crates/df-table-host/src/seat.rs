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
//! or kind: every seat comes from the key, never from a body.
//!
//! Joining, readying and leaving go through the lobby machine as that seat, through the wire
//! intent's one path (`table`). The routes (`routes`) answer the phone on the player listener and
//! the loopback listener, and the dev routes and verbs (`players ...`, `cli`) manage friends,
//! approvals, seats and sample heroes. Every step is one `SEAT` line: `hello`, `join_requested`,
//! `approved`, `refused`, `seated`, `rejoined`, `left`.

pub(crate) mod book;
pub mod cli;
pub(crate) mod key;
pub mod request;
pub mod routes;
pub(crate) mod table;

use std::path::{Path, PathBuf};

use self::book::{Book, Books, write_book};
use self::request::SeatError;
use crate::blocking::run_blocking;
use crate::io_cause::IoCause;

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

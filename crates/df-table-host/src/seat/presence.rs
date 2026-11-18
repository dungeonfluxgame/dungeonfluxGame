//! Who is at the table now: a seat is seen whenever its phone calls a seat route (`SeatCaller`),
//! and a seat not seen for `AWAY_AFTER_MS` is away.
//!
//! The book keeps when each seat was last seen, written at most once every `SEEN_EVERY_MS` so a
//! polling phone does not rewrite the file on every poll. While the lobby gathers, an away seat
//! leaves the lobby machine (`sweep`): it is not counted for the party's readiness, a Start leaves
//! it out of the day's party (`DF_RUN_PLAYERS`), and the other phones list it as away (`present`
//! false). It keeps its seat in the book, so its key comes back to it: the next seat route it calls
//! seats it again (`seen`), as a restart's rejoin does. Once the game has started, nobody leaves
//! the table for being away.
//!
//! Lines: `SEAT away seat=` once when a seat goes away, `SEAT back seat=` when it is seen again.

use std::collections::BTreeSet;

use df_table::lobby::LobbyAction;

use super::book::{Book, Books};
use super::key::KeyHash;
use super::request::SeatError;
use super::table::{self, ActionResult};
use crate::state::AppState;

/// How long a seat may go unseen before it is away: two minutes.
pub(crate) const AWAY_AFTER_MS: u64 = 2 * 60 * 1_000;
/// The least time between two writes of when a seat was last seen.
pub(crate) const SEEN_EVERY_MS: u64 = 5_000;

/// Whether a seat last seen at `last_seen_ms` is away at `now_ms`.
pub(crate) fn is_away(last_seen_ms: u64, now_ms: u64) -> bool {
    now_ms.saturating_sub(last_seen_ms) >= AWAY_AFTER_MS
}

impl Books {
    /// Notes that the player at `key` was seen at `now_ms`, unless they were noted less than
    /// `SEEN_EVERY_MS` before: whether the book changed.
    pub(crate) fn seen(&mut self, key: &KeyHash, now_ms: u64) -> bool {
        let fresh = self
            .seat_of(key)
            .is_some_and(|record| now_ms.saturating_sub(record.last_seen_ms) < SEEN_EVERY_MS);
        !fresh && self.touch(key, now_ms)
    }
}

/// The seats away now, and the change since the last look: each seat that went away, and each
/// that came back.
#[derive(Debug, Default)]
pub(crate) struct Presence {
    away: BTreeSet<u64>,
}

impl Presence {
    /// Marks the seats of `last_seen` (seat, last seen) away or present at `now_ms`; the seats
    /// that went away and those that came back since the last call.
    pub(crate) fn update(&mut self, last_seen: &[(u64, u64)], now_ms: u64) -> (Vec<u64>, Vec<u64>) {
        let (mut gone, mut back) = (Vec::new(), Vec::new());
        for (seat, seen_ms) in last_seen {
            if is_away(*seen_ms, now_ms) {
                if self.away.insert(*seat) {
                    gone.push(*seat);
                }
            } else if self.away.remove(seat) {
                back.push(*seat);
            }
        }
        // A seat the book no longer holds is forgotten.
        self.away
            .retain(|seat| last_seen.iter().any(|(held, _)| held == seat));
        (gone, back)
    }
}

/// Notes that the phone at `key` was seen at `now_ms` (written to the book at most every
/// `SEEN_EVERY_MS`), and seats it at the table again when it had gone away while the lobby
/// gathered. One `SEAT back seat=` line when it comes back.
pub(crate) async fn seen(state: &AppState, key: &KeyHash, now_ms: u64) -> Result<(), SeatError> {
    let mut open = state.seats().open().await?;
    if !open.books().seen(key, now_ms) {
        return Ok(());
    }
    open.save(Book::Seats).await?;
    drop(open);
    sweep(state, now_ms).await?;
    // A phone at work elsewhere (the look studio) keeps the TV lobby up to date too.
    super::lobby::show(state).await;
    Ok(())
}

/// Looks at who is away at `now_ms`: each seat that went away leaves the lobby machine while the
/// table gathers (it keeps its seat in the book), and each that came back is seated again. One
/// `SEAT away seat=` or `SEAT back seat=` line for each change.
pub(crate) async fn sweep(state: &AppState, now_ms: u64) -> Result<(), SeatError> {
    let last_seen: Vec<(u64, u64)> = state
        .seats()
        .open()
        .await?
        .books()
        .records()
        .iter()
        .map(|record| (record.seat, record.last_seen_ms))
        .collect();
    let (gone, back) = state.seats().presence().update(&last_seen, now_ms);
    if gone.is_empty() && back.is_empty() {
        return Ok(());
    }
    let engine = state.engine()?;
    for seat in gone {
        state.record_line(format!("SEAT away seat={seat}"));
        let view = table::quiet_view(engine, seat).await?;
        if table::is_gathering(&view) && table::is_seated(&view) {
            let left = table::act(engine, seat, LobbyAction::Leave).await?;
            if left != ActionResult::Applied {
                state.record_line(format!("SEAT away_kept seat={seat}"));
            }
        }
    }
    for seat in back {
        state.record_line(format!("SEAT back seat={seat}"));
        let view = table::quiet_view(engine, seat).await?;
        if table::is_gathering(&view) {
            table::sit_down(engine, seat).await?;
        }
    }
    Ok(())
}

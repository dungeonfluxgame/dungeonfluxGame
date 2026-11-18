//! The room code: six characters from an unambiguous alphabet (`lobby::model::ROOM_CODE_ALPHABET`)
//! that a new phone must carry to take a seat (`JoinRequest`), shown on the shared screen's lobby
//! with a QR code of the join link `http://<lan>:<players port>/play?code=<code>`.
//!
//! The host makes a code when the table's lobby opens (`SeatDesk::new` at boot, and `renew` when a
//! day ends and the table gathers again) and puts it on the TV lobby's join panel
//! (`open_lobby`). A wrong code is refused `code_mismatch` at once, and opens a wait for its
//! address that grows with each miss in a row (`MISS_FIRST_DELAY` doubling to
//! `MISS_LONGEST_DELAY`): any code from that address inside its wait, right or wrong, is refused
//! `too_many_guesses` at once and unchecked, so guessing is slow however many connections ask,
//! and no answer is held open. The check and the wait are taken under one lock, so one check per
//! address is in flight. The addresses remembered are bounded. A phone whose key holds a seat
//! comes back without a code (`JoinRequest::Returning`). The code itself is never logged: it is on
//! the screen, and `players` prints it for the developer.
//!
//! Lines: `SEAT code made chars= link=yes|no qr=yes|no tv=shown|<error>`, `SEAT join|code refused
//! reason=code_mismatch misses= delay_ms=`, `SEAT join|code refused reason=too_many_guesses
//! retry_after_ms=`, `SEAT lobby closed tv=closed|<error>`.

use std::collections::BTreeMap;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::Arc;
use std::time::Duration;

use df_playground::lobby::change::ScreenEdit;
use df_playground::lobby::model::{
    JoinCode, QrModules, ROOM_CODE_ALPHABET, ROOM_CODE_CHARACTERS, room_code,
};
use df_playground::scene::SceneChange;
use qrcode::{Color, QrCode};

use crate::state::AppState;

/// How long the address of a first miss waits before its next try.
const MISS_FIRST_DELAY: Duration = Duration::from_millis(250);
/// The longest wait a run of misses reaches.
const MISS_LONGEST_DELAY: Duration = Duration::from_secs(4);
/// A run of misses is forgotten after this long without another.
const MISS_MEMORY_MS: u64 = 10 * 60 * 1_000;
/// The most addresses whose misses are remembered; the one missed longest ago goes first.
const MOST_MISSING_ADDRESSES: usize = 256;

/// A run of misses from one address: how many, when the last was, and until when the address
/// waits before its next try.
#[derive(Debug, Clone, Copy)]
struct Misses {
    count: u32,
    last_ms: u64,
    wait_until_ms: u64,
}

/// A code not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Missed {
    /// The code is not the room's: how many misses in a row the address has made, and how long it
    /// waits before its next try.
    Wrong { misses: u32, delay: Duration },
    /// The address tried inside its wait: the code was not looked at, and it may try again after
    /// `retry_after`.
    TooSoon { retry_after: Duration },
}

/// The room's code and the misses from each address.
#[derive(Debug)]
pub(crate) struct RoomDoor {
    code: String,
    misses: BTreeMap<String, Misses>,
}

/// A new room code: `ROOM_CODE_CHARACTERS` of the alphabet, from the operating system's seed of
/// `RandomState` (new for every call), so no new dependency is needed. It only has to be hard to
/// guess at the slowed pace of a miss, not secret.
fn new_code() -> String {
    let alphabet: Vec<char> = ROOM_CODE_ALPHABET.chars().collect();
    let mut bits = random_u64(0x5EA7);
    let base = alphabet.len() as u64;
    (0..ROOM_CODE_CHARACTERS)
        .map(|_| {
            // The index is below the alphabet's length, so it fits.
            let character = alphabet[(bits % base) as usize];
            bits /= base;
            character
        })
        .collect()
}

/// 64 random bits from the operating system's seed of `RandomState` (new for every call), mixed
/// with `salt`: the room code's and a run's token's.
pub(crate) fn random_u64(salt: u64) -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(salt);
    hasher.finish()
}

impl RoomDoor {
    /// A door with a new code.
    pub(crate) fn new() -> Self {
        Self {
            code: new_code(),
            misses: BTreeMap::new(),
        }
    }

    pub(crate) fn code(&self) -> &str {
        &self.code
    }

    /// A new code, for a lobby that opens again; the misses are forgotten with the old one.
    pub(crate) fn renew(&mut self) {
        self.code = new_code();
        self.misses.clear();
    }

    /// Whether `typed` is the room's code, from `address` at `now_ms`. A try inside the address's
    /// wait is `TooSoon`, unchecked; a miss is counted for the address and opens its next wait; a
    /// match forgets the address's misses.
    pub(crate) fn check(&mut self, address: &str, typed: &str, now_ms: u64) -> Result<(), Missed> {
        if let Some(run) = self.misses.get(address)
            && now_ms < run.wait_until_ms
        {
            return Err(Missed::TooSoon {
                retry_after: Duration::from_millis(run.wait_until_ms - now_ms),
            });
        }
        if room_code(typed).as_deref() == Some(self.code.as_str()) {
            self.misses.remove(address);
            return Ok(());
        }
        let earlier = self
            .misses
            .get(address)
            .filter(|run| now_ms.saturating_sub(run.last_ms) < MISS_MEMORY_MS)
            .map_or(0, |run| run.count);
        if !self.misses.contains_key(address) && self.misses.len() >= MOST_MISSING_ADDRESSES {
            let oldest = self
                .misses
                .iter()
                .min_by_key(|(_, run)| run.last_ms)
                .map(|(oldest, _)| oldest.clone());
            if let Some(oldest) = oldest {
                self.misses.remove(&oldest);
            }
        }
        let count = earlier.saturating_add(1);
        let doublings = count.saturating_sub(1).min(8);
        let delay = MISS_FIRST_DELAY
            .saturating_mul(1 << doublings)
            .min(MISS_LONGEST_DELAY);
        let delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
        self.misses.insert(
            address.to_owned(),
            Misses {
                count,
                last_ms: now_ms,
                wait_until_ms: now_ms.saturating_add(delay_ms),
            },
        );
        Err(Missed::Wrong {
            misses: count,
            delay,
        })
    }
}

/// The modules of a QR code of `text`, row by row (`QrModules`): the one place the host makes a
/// QR code, for the TV lobby and the phone link page alike.
pub(crate) fn qr_modules(text: &str) -> Result<QrModules, qrcode::types::QrError> {
    let code = QrCode::new(text.as_bytes())?;
    let width = u16::try_from(code.width()).map_err(|_| qrcode::types::QrError::DataTooLong)?;
    let dark = code
        .to_colors()
        .into_iter()
        .map(|color| if color == Color::Dark { '1' } else { '0' })
        .collect();
    Ok(QrModules { width, dark })
}

/// The join link of `code` on the player listener's first address, when it listens.
fn join_link(state: &AppState, code: &str) -> Option<String> {
    state
        .player_links()
        .first()
        .map(|link| format!("{link}?code={code}"))
}

/// The TV lobby's join panel for `code`: the code, its join link and the link's QR code.
pub(crate) fn join_code(state: &AppState, code: &str) -> JoinCode {
    let join_link = join_link(state, code);
    let qr = join_link.as_deref().and_then(|link| qr_modules(link).ok());
    JoinCode {
        hint: None,
        join_link,
        game_code: Some(code.to_owned()),
        qr,
    }
}

/// The table's lobby opens: its code goes on the TV lobby (`lobby::open`), with the join link and
/// its QR code. One `SEAT code made` line.
pub(crate) async fn open_lobby(state: &Arc<AppState>) {
    let code = state.seats().room_code();
    let join = join_code(state, &code);
    let (linked, drawn) = (join.join_link.is_some(), join.qr.is_some());
    let shown = super::lobby::open(state).await;
    state.record_line(format!(
        "SEAT code made chars={ROOM_CODE_CHARACTERS} link={} qr={} tv={shown}",
        yes(linked),
        yes(drawn)
    ));
}

/// Takes the lobby off the TV: the game has started. One `SEAT lobby closed` line.
pub(crate) async fn close_lobby(state: &AppState) {
    let edit = ScreenEdit::LobbyClear {};
    let closed = crate::playground::apply_scene_changes(state, vec![SceneChange::Screen { edit }])
        .await
        .map_or_else(
            |error| error.status.as_u16().to_string(),
            |_| "closed".to_owned(),
        );
    state.seats().lobby().forget();
    state.record_line(format!("SEAT lobby closed tv={closed}"));
}

fn yes(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

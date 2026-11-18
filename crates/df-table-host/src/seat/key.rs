//! A phone's seat key as the host keeps it: only its SHA-256 hash (REF-046). The key arrives in
//! `path::SEAT_KEY_HEADER` and is hashed at once; neither the key nor the hash is ever written to
//! a log or put in an address.

use axum::http::HeaderMap;
use df_playground::path::SEAT_KEY_HEADER;
use df_playground::seat::wire::{hex_digits, is_seat_key};
use df_table::ports::Clock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::book::SeatRecord;
use super::request::SeatError;
use crate::observe::WallClock;
use crate::state::AppState;

/// The SHA-256 hash of a seat key, as lowercase hexadecimal digits.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct KeyHash(String);

impl KeyHash {
    /// The hash of `key`, a key `is_seat_key` accepts.
    pub(crate) fn of(key: &str) -> Self {
        Self(hex_digits(&Sha256::digest(key.as_bytes())))
    }

    /// The hash of the key a request carries, or why it carries none that reads.
    pub(crate) fn from_headers(headers: &HeaderMap) -> Result<Self, SeatError> {
        let value = headers.get(SEAT_KEY_HEADER).ok_or(SeatError::MissingKey)?;
        let key = value.to_str().map_err(|_| SeatError::InvalidKey)?;
        if !is_seat_key(key) {
            return Err(SeatError::InvalidKey);
        }
        Ok(Self::of(key))
    }
}

/// The start of the developer headers: the guard refuses every one of them on the player listener
/// (`guard::GuardError::DevHeader`), so only the loopback listener's callers may use them.
pub(crate) const DEV_HEADER_PREFIX: &str = "x-df-dev-";
/// The `dev` command line acts as the seat of this number on the player routes, in place of a
/// phone's key.
pub(crate) const DEV_SEAT_HEADER: &str = "x-df-dev-seat";
/// The run token of the day's runner (`DF_RUN_TOKEN`), which the `dev` command line sends with
/// the seat calls a runner makes (`players ask`, `inbox`, `note`): the host refuses a runner that
/// is not the current day's (`not_this_run`).
pub(crate) const DEV_RUN_HEADER: &str = "x-df-dev-run";
/// The environment variable a runner's run token is in.
pub(crate) const RUN_TOKEN_VARIABLE: &str = "DF_RUN_TOKEN";
/// `true`: the `dev` command line waits for a picture a look route starts (`wait=true`).
pub(crate) const DEV_WAIT_HEADER: &str = "x-df-dev-wait";

/// Whether the developer asked a look route to wait for its picture.
pub(crate) fn dev_wait(headers: &HeaderMap) -> bool {
    headers
        .get(DEV_WAIT_HEADER)
        .is_some_and(|value| value.as_bytes() == b"true")
}

/// Who calls a player route: the seat the request's key holds, read the one way every seat, look
/// and pages route reads it, so a lost key refuses alike on all of them (`unknown_key`). On the
/// loopback listener the `dev` command line may name the seat instead (`DEV_SEAT_HEADER`).
#[derive(Debug, Clone)]
pub(crate) struct SeatCaller {
    pub(crate) record: SeatRecord,
}

impl SeatCaller {
    /// The seat the key of `headers` holds.
    pub(crate) async fn of(state: &AppState, headers: &HeaderMap) -> Result<Self, SeatError> {
        if let Some(value) = headers.get(DEV_SEAT_HEADER) {
            let seat = value
                .to_str()
                .ok()
                .and_then(|text| text.parse::<u64>().ok())
                .ok_or(SeatError::InvalidKey)?;
            let mut open = state.seats().open().await?;
            let record = open
                .books()
                .records()
                .iter()
                .find(|record| record.seat == seat)
                .cloned()
                .ok_or(SeatError::UnknownSeat { seat })?;
            return Ok(Self { record });
        }
        let key = KeyHash::from_headers(headers)?;
        let record = state
            .seats()
            .open()
            .await?
            .books()
            .seat_of(&key)
            .cloned()
            .ok_or(SeatError::UnknownKey)?;
        // Every seat route the phone calls says it is here; a failure to note it never refuses
        // the route.
        if let Err(error) = super::presence::seen(state, &key, WallClock.now_ms()).await {
            state.record_line(format!(
                "SEAT seen_failed seat={} cause={}",
                record.seat,
                error.key()
            ));
        }
        Ok(Self { record })
    }

    pub(crate) fn seat(&self) -> u64 {
        self.record.seat
    }

    /// The character the seat plays, by persona id; a seat that plays none has nothing to say.
    pub(crate) fn character(&self) -> Result<&str, SeatError> {
        self.record.persona.as_deref().ok_or(SeatError::NoPersona)
    }
}

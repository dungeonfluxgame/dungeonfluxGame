//! A phone's seat key as the host keeps it: only its SHA-256 hash (REF-046). The key arrives in
//! `path::SEAT_KEY_HEADER` and is hashed at once; neither the key nor the hash is ever written to
//! a log or put in an address.

use axum::http::HeaderMap;
use df_playground::path::SEAT_KEY_HEADER;
use df_playground::seat::wire::{hex_digits, is_seat_key};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::request::SeatError;

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

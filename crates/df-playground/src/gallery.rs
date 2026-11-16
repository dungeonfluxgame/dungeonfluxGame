//! The shared part of the development galleries (`kit::gallery`, `phone_kit::gallery`): boards of
//! sample panels a reviewer opens with a query on the page address (`?kit=3`, `?phone_kit=2`).
//! Each gallery has its own board type and its own `gallery.json`; reading them, counting them
//! from 1 and finding the number in the address is the same for both.

use std::fmt;

use serde::Deserialize;

use crate::path::query_value;

/// Every board of one gallery.
#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Gallery<B> {
    pub boards: Vec<B>,
}

/// The gallery's boards could not be read.
#[derive(Debug)]
pub struct GalleryError {
    cause: serde_json::Error,
}

impl fmt::Display for GalleryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "the gallery could not be read: {}", self.cause)
    }
}

impl std::error::Error for GalleryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

impl<B: for<'de> Deserialize<'de>> Gallery<B> {
    pub fn parse(text: &str) -> Result<Self, GalleryError> {
        serde_json::from_str(text).map_err(|cause| GalleryError { cause })
    }
}

impl<B> Gallery<B> {
    /// Board `number`, counted from 1.
    pub fn board(&self, number: usize) -> Option<&B> {
        number
            .checked_sub(1)
            .and_then(|index| self.boards.get(index))
    }
}

/// The board number the address's query names under `key`, or `None` when it names none.
pub fn board_number(query: &str, key: &str) -> Option<usize> {
    query_value(query, key)
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|number| *number >= 1)
}

//! The words the player sees for each status, and how long a line stays up.
//!
//! This is the table experience contract of the plan (section K.5, K.12 and H.15) as code, so a
//! phrase or a timing is decided in one place and tested. It never draws and never reads state.

use serde::{Deserialize, Serialize};

use crate::fixture::Status;
use crate::limits::{DWELL_MS_PER_WORD, MINIMUM_CAPTION_DWELL_MS, MINIMUM_RESULT_CARD_DWELL_MS};

/// How fast the table likes to read: scales every dwell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadingPace {
    Slow,
    Normal,
    Fast,
}

impl ReadingPace {
    pub const ALL: [Self; 3] = [Self::Slow, Self::Normal, Self::Fast];

    /// The pace as a share of the normal dwell, in percent.
    fn percent(self) -> u32 {
        match self {
            Self::Slow => 150,
            Self::Normal => 100,
            Self::Fast => 75,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Slow => "slow",
            Self::Normal => "normal",
            Self::Fast => "fast",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }
}

/// Scales a normal-pace dwell to the pace, held at the largest `u64` so an enormous caption stays
/// enormous instead of wrapping to something short.
fn scaled(normal_ms: u64, pace: ReadingPace) -> u64 {
    normal_ms.saturating_mul(u64::from(pace.percent())) / 100
}

/// How long a caption of `words` words stays up at the given pace, in milliseconds.
pub fn caption_dwell_ms(words: u32, pace: ReadingPace) -> u64 {
    let reading_ms = u64::from(words).saturating_mul(DWELL_MS_PER_WORD);
    scaled(MINIMUM_CAPTION_DWELL_MS.max(reading_ms), pace)
}

/// How long a result card stays up at the given pace, in milliseconds.
pub fn result_card_dwell_ms(pace: ReadingPace) -> u64 {
    scaled(MINIMUM_RESULT_CARD_DWELL_MS, pace)
}

/// The number of whitespace-separated words in a caption.
pub fn word_count(text: &str) -> u32 {
    u32::try_from(text.split_whitespace().count()).unwrap_or(u32::MAX)
}

/// A status chosen by key in the playground's controls, for trying each wait on screen. The key
/// of a status with a person in it gets a sample person.
pub fn status_from_key(key: &str) -> Option<Status> {
    Some(match key {
        "ok" => Status::Ok,
        "reading" => Status::Reading,
        "rolling" => Status::Rolling,
        "telling" => Status::Telling,
        "art_delayed" => Status::ArtDelayed,
        "degraded" => Status::Degraded,
        "read_only" => Status::ReadOnly,
        "reconnecting" => Status::Reconnecting,
        "waiting_for" => Status::WaitingFor {
            who: "Pip".to_owned(),
            what: "to answer a reaction".to_owned(),
        },
        _ => return None,
    })
}

/// The status keys the playground offers, in the order of its menu.
pub const STATUS_KEYS: [&str; 9] = [
    "ok",
    "reading",
    "rolling",
    "telling",
    "art_delayed",
    "degraded",
    "read_only",
    "reconnecting",
    "waiting_for",
];

/// The plain sentence for a status, or `None` when nothing needs saying.
pub fn status_copy(status: &Status) -> Option<String> {
    match status {
        Status::Ok => None,
        Status::Reading => Some("Reading your move".to_owned()),
        Status::Rolling => Some("Rolling".to_owned()),
        Status::Telling => Some("Telling it".to_owned()),
        Status::ArtDelayed => Some("Picture catching up".to_owned()),
        Status::Degraded => Some("Some pictures are unavailable. Play continues.".to_owned()),
        Status::ReadOnly => Some("Could not save. Try again.".to_owned()),
        Status::Reconnecting => Some("Reconnecting".to_owned()),
        Status::WaitingFor { who, what } => Some(format!("Waiting for {who} {what}")),
    }
}

/// Whether a status stops the table from taking input, so the screen should say so loudly.
pub fn status_blocks_input(status: &Status) -> bool {
    matches!(
        status,
        Status::ReadOnly | Status::Reconnecting | Status::WaitingFor { .. }
    )
}

//! The typed error of the seat routes, and the bodies and replies of their dev routes. The player
//! routes' bodies and replies are the phone's (`df_playground::seat::wire`).

use std::fmt;

use axum::http::StatusCode;
use df_playground::seat::wire::{FriendCard, HeroCard, PlayerKind, SeatCard};
use serde::{Deserialize, Serialize};

use crate::dev_http::{BodyError, DevReply, RouteError};
use crate::engine::report::EngineError;
use crate::io_cause::IoCause;

/// Why a seat route refused. The tags the phone words are `SeatErrorKind`'s keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum SeatError {
    /// A player route was called without `path::SEAT_KEY_HEADER`.
    MissingKey,
    /// The seat key is not 64 lowercase hexadecimal digits.
    InvalidKey,
    /// The key holds no seat.
    UnknownKey,
    /// The hero is not one of this player's.
    UnknownHero {
        hero: u64,
    },
    /// The name breaks the hero name rule (REF-061).
    InvalidName,
    UnknownFriend {
        profile: u64,
    },
    /// Signing in to one's own account: the dev host has no accounts.
    AccountsNotAvailable,
    /// Every seat of the table is held.
    TableFull,
    /// The lobby has handed the table on; no new seat is taken now.
    GameStarted,
    PendingFull,
    /// The table's engine is not running.
    TableClosed,
    /// The table answered a seat's action without applying or declining it.
    NotApplied,
    UnknownRequest {
        request: u64,
    },
    UnknownSeat {
        seat: u64,
    },
    FriendsFull,
    HeroesFull,
    InvalidBody(BodyError),
    /// A players file could not be read or written.
    Storage {
        cause: IoCause,
    },
    /// The engine refused the call that would have seated or moved the seat.
    Engine {
        cause: EngineError,
    },
}

impl SeatError {
    pub fn key(&self) -> &'static str {
        match self {
            Self::MissingKey => "missing_key",
            Self::InvalidKey => "invalid_key",
            Self::UnknownKey => "unknown_key",
            Self::UnknownHero { .. } => "unknown_hero",
            Self::InvalidName => "invalid_name",
            Self::UnknownFriend { .. } => "unknown_friend",
            Self::AccountsNotAvailable => "accounts_not_available",
            Self::TableFull => "table_full",
            Self::GameStarted => "game_started",
            Self::PendingFull => "pending_full",
            Self::TableClosed => "table_closed",
            Self::NotApplied => "not_applied",
            Self::UnknownRequest { .. } => "unknown_request",
            Self::UnknownSeat { .. } => "unknown_seat",
            Self::FriendsFull => "friends_full",
            Self::HeroesFull => "heroes_full",
            Self::InvalidBody(_) => "invalid_body",
            Self::Storage { .. } => "storage",
            Self::Engine { .. } => "engine",
        }
    }
}

impl From<BodyError> for SeatError {
    fn from(error: BodyError) -> Self {
        Self::InvalidBody(error)
    }
}

impl From<EngineError> for SeatError {
    fn from(cause: EngineError) -> Self {
        match cause {
            EngineError::NotStarted { .. } | EngineError::Stopped | EngineError::OwnerPanicked => {
                Self::TableClosed
            }
            cause => Self::Engine { cause },
        }
    }
}

impl fmt::Display for SeatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingKey => formatter.write_str("this phone sent no seat key"),
            Self::InvalidKey => formatter.write_str("this phone's seat key is not one"),
            Self::UnknownKey => formatter.write_str("this phone holds no seat here"),
            Self::UnknownHero { hero } => write!(formatter, "hero {hero} is not one of yours"),
            Self::InvalidName => formatter.write_str(
                "a name is letters, with single spaces, apostrophes, hyphens or periods between",
            ),
            Self::UnknownFriend { profile } => {
                write!(formatter, "there is no friend profile {profile}")
            }
            Self::AccountsNotAvailable => {
                formatter.write_str("signing in comes later on this table")
            }
            Self::TableFull => formatter.write_str("the table is full"),
            Self::GameStarted => formatter.write_str("the game has started without a new seat"),
            Self::PendingFull => {
                formatter.write_str("too many players are waiting for the host; try again soon")
            }
            Self::TableClosed => formatter.write_str("the table is not running"),
            Self::NotApplied => formatter.write_str("the table did not take that yet; try again"),
            Self::UnknownRequest { request } => {
                write!(formatter, "there is no request {request} waiting")
            }
            Self::UnknownSeat { seat } => write!(formatter, "nobody holds seat {seat}"),
            Self::FriendsFull => formatter.write_str("the friends book is full"),
            Self::HeroesFull => formatter.write_str("the hero library is full"),
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::Storage { cause } => write!(formatter, "a players file failed: {cause}"),
            Self::Engine { cause } => write!(formatter, "the table refused: {cause}"),
        }
    }
}

impl std::error::Error for SeatError {}

impl RouteError for SeatError {
    fn status(&self) -> StatusCode {
        match self {
            Self::MissingKey | Self::InvalidKey | Self::InvalidName | Self::InvalidBody(_) => {
                StatusCode::BAD_REQUEST
            }
            Self::UnknownKey
            | Self::UnknownHero { .. }
            | Self::UnknownFriend { .. }
            | Self::UnknownRequest { .. }
            | Self::UnknownSeat { .. } => StatusCode::NOT_FOUND,
            Self::AccountsNotAvailable => StatusCode::NOT_IMPLEMENTED,
            Self::TableFull
            | Self::GameStarted
            | Self::PendingFull
            | Self::NotApplied
            | Self::FriendsFull
            | Self::HeroesFull => StatusCode::CONFLICT,
            Self::TableClosed => StatusCode::SERVICE_UNAVAILABLE,
            Self::Storage { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Engine { cause } => cause.status(),
        }
    }
}

/// The answer of a seat route.
pub type SeatReply<T> = DevReply<T, SeatError>;

/// `path::FRIEND_ADD`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FriendAdd {
    pub name: String,
}

/// `path::FRIEND_REMOVE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FriendRemove {
    pub profile: u64,
}

/// `path::APPROVE` and `path::REFUSE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalAnswer {
    pub request: u64,
}

/// `path::HERO_ADD`: a sample hero for the player at `seat`, until creation writes real ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeroAdd {
    pub seat: u64,
    pub name: String,
    pub species: String,
    pub class: String,
}

/// `path::SEAT_REMOVE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatRemove {
    pub seat: u64,
}

/// One held seat, as `players` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatLine {
    pub seat: u64,
    pub name: String,
    pub kind: PlayerKind,
    /// The friend profile the seat plays under, for a friend.
    pub profile: Option<u64>,
    pub joined_ms: u64,
    pub last_seen_ms: u64,
}

/// Whether a request for a seat as a friend still waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingState {
    Waiting,
    /// Refused, and the phone has not read it yet.
    Refused,
}

impl PendingState {
    pub fn key(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Refused => "refused",
        }
    }
}

/// One request for a seat as a friend, as `players pending` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingLine {
    pub request: u64,
    pub profile: u64,
    pub name: String,
    pub asked_ms: u64,
    pub state: PendingState,
}

/// `path::PLAYERS`: the seat book, the friends book and the requests waiting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayersReport {
    pub seats: Vec<SeatLine>,
    pub friends: Vec<FriendCard>,
    pub pending: Vec<PendingLine>,
    pub heroes: usize,
}

/// `path::APPROVE`: the seat the friend now holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approved {
    pub seat: SeatCard,
}

/// `path::HERO_ADD`: the hero as the phone lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeroAdded {
    pub hero: HeroCard,
}

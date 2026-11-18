//! The creation routes' typed refusal, and the bodies and replies of the dev routes (the table,
//! the start and a seat's sheet file). A seat's own routes take and answer the phone's wire values
//! (`df_playground::pages::creation`).
use std::fmt;

use axum::http::StatusCode;
use df_table::creation::{CreationError, DesignMark, HeroStart, SeatCreationState};
use serde::{Deserialize, Serialize};

use crate::dev_http::{BodyError, DevReply, QueryError, RouteError};
use crate::io_cause::IoCause;
use crate::seat::request::SeatError;

/// The reply of every creation route.
pub type CreationReply<T> = DevReply<T, CreationRouteError>;

/// A seat, as a dev route's query names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatAsk {
    pub seat: u64,
}

/// The body of the start route: nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableAsk {}

/// Every seat in creation, as the TV sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableView {
    pub started: bool,
    pub seats: Vec<PublicSeatView>,
}

/// One seat as the TV sees it: the hero's name once chosen, and the state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicSeatView {
    pub seat: u64,
    pub name: Option<String>,
    pub state: SeatCreationState,
}

/// One finished hero handed to play.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartedHero {
    pub seat: u64,
    pub hero: HeroStart,
    pub sheet: serde_json::Value,
    pub marks: Vec<DesignMark>,
}

/// What starting play handed on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartView {
    pub next_machine: String,
    pub heroes: Vec<StartedHero>,
}

/// Why a creation route refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum CreationRouteError {
    InvalidBody(BodyError),
    InvalidQuery(QueryError),
    /// No seat has begun its build yet.
    NotOpen,
    /// No seat is seated to hand to creation.
    NoSeats,
    UnknownSeat {
        seat: u64,
    },
    /// The seat plays no hero yet: the look studio starts one.
    NoHero {
        seat: u64,
    },
    /// The seat is ready: it reopens before its build changes.
    SeatReady {
        seat: u64,
    },
    /// Only a build the rules derived in full is submitted.
    BuildIncomplete {
        seat: u64,
        issues: usize,
    },
    NotReady {
        seat: u64,
    },
    /// Play has started: creation is over.
    Started,
    /// Play starts when every seat is ready; the first that is not.
    SeatsNotReady {
        seat: u64,
    },
    TooManyPicks,
    /// No seat has built this hero.
    UnknownHero {
        hero: u64,
    },
    /// The rules refused the change of the hero's vitals.
    InvalidVitals {
        cause: df_rules::hero::VitalsError,
    },
    /// The lobby did not take the seat's ready (or not ready), with its sentence.
    LobbyDeclined {
        reason: String,
    },
    /// The hand-off to play could not be built.
    HandOffRefused,
    /// The saved record of creation could not be read or written.
    Storage {
        cause: IoCause,
    },
    /// A task stopped without an answer.
    Stopped,
    /// The seat books refused, in the seat routes' own shape.
    #[serde(untagged)]
    Seat(SeatError),
}

impl CreationRouteError {
    /// The key the audit line of a refusal names.
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::InvalidQuery(_) => "invalid_query",
            Self::NotOpen => "not_open",
            Self::NoSeats => "no_seats",
            Self::UnknownSeat { .. } => "unknown_seat",
            Self::NoHero { .. } => "no_hero",
            Self::SeatReady { .. } => "seat_ready",
            Self::BuildIncomplete { .. } => "build_incomplete",
            Self::NotReady { .. } => "not_ready",
            Self::Started => "started",
            Self::SeatsNotReady { .. } => "seats_not_ready",
            Self::TooManyPicks => "too_many_picks",
            Self::LobbyDeclined { .. } => "lobby_declined",
            Self::UnknownHero { .. } => "unknown_hero",
            Self::InvalidVitals { .. } => "invalid_vitals",
            Self::HandOffRefused => "hand_off_refused",
            Self::Storage { .. } => "storage",
            Self::Stopped => "stopped",
            Self::Seat(cause) => cause.key(),
        }
    }
}

impl From<CreationError> for CreationRouteError {
    fn from(error: CreationError) -> Self {
        match error {
            CreationError::UnknownSeat { seat } => Self::UnknownSeat { seat: seat.get() },
            CreationError::NoHero { seat } => Self::NoHero { seat: seat.get() },
            CreationError::SeatReady { seat } => Self::SeatReady { seat: seat.get() },
            CreationError::BuildIncomplete { seat, issues } => Self::BuildIncomplete {
                seat: seat.get(),
                issues,
            },
            CreationError::NotReady { seat } => Self::NotReady { seat: seat.get() },
            CreationError::Started => Self::Started,
            CreationError::SeatsNotReady { seat } => Self::SeatsNotReady { seat: seat.get() },
            CreationError::NoSeats => Self::NoSeats,
            CreationError::TooManyPicks => Self::TooManyPicks,
            CreationError::HandOff(_) | CreationError::Name(_) => Self::HandOffRefused,
        }
    }
}

impl From<SeatError> for CreationRouteError {
    fn from(cause: SeatError) -> Self {
        Self::Seat(cause)
    }
}

impl fmt::Display for CreationRouteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::InvalidQuery(error) => write!(formatter, "{error}"),
            Self::NotOpen => formatter.write_str("no seat has begun its build yet"),
            Self::NoSeats => formatter.write_str("no seat is seated to make a hero"),
            Self::UnknownSeat { seat } => write!(formatter, "seat {seat} is not in creation"),
            Self::NoHero { seat } => write!(
                formatter,
                "seat {seat} plays no hero yet: start one in the look studio"
            ),
            Self::SeatReady { seat } => {
                write!(
                    formatter,
                    "seat {seat} is ready: reopen it to change the build"
                )
            }
            Self::BuildIncomplete { seat, issues } => write!(
                formatter,
                "seat {seat}'s build is not complete ({issues} problems)"
            ),
            Self::NotReady { seat } => write!(formatter, "seat {seat} is not ready"),
            Self::Started => formatter.write_str("play has started: creation is over"),
            Self::SeatsNotReady { seat } => {
                write!(
                    formatter,
                    "every seat must be ready first; seat {seat} is not"
                )
            }
            Self::TooManyPicks => formatter.write_str("the build draft holds too many picks"),
            Self::LobbyDeclined { reason } => {
                write!(formatter, "the lobby did not take it: {reason}")
            }
            Self::UnknownHero { hero } => write!(formatter, "no seat has built hero {hero}"),
            Self::InvalidVitals { cause } => write!(formatter, "{cause}"),
            Self::HandOffRefused => formatter.write_str("the hand-off to play could not be built"),
            Self::Storage { cause } => {
                write!(formatter, "the creation record could not be kept: {cause}")
            }
            Self::Stopped => formatter.write_str("the creation task stopped without an answer"),
            Self::Seat(cause) => write!(formatter, "{cause}"),
        }
    }
}

impl std::error::Error for CreationRouteError {}

impl RouteError for CreationRouteError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_) | Self::InvalidQuery(_) | Self::TooManyPicks => {
                StatusCode::BAD_REQUEST
            }
            Self::UnknownSeat { .. } | Self::UnknownHero { .. } => StatusCode::NOT_FOUND,
            Self::InvalidVitals { .. } => StatusCode::CONFLICT,
            Self::NotOpen
            | Self::NoSeats
            | Self::NoHero { .. }
            | Self::SeatReady { .. }
            | Self::BuildIncomplete { .. }
            | Self::NotReady { .. }
            | Self::Started
            | Self::SeatsNotReady { .. }
            | Self::LobbyDeclined { .. } => StatusCode::CONFLICT,
            Self::HandOffRefused | Self::Storage { .. } | Self::Stopped => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::Seat(cause) => cause.status(),
        }
    }
}

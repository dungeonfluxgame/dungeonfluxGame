//! The bodies and replies of the world routes, and why they refuse.
//!
//! Every route answers a `WorldReply<T>`: `{"outcome":"done","value":..}` or
//! `{"outcome":"rejected","error":{"error":"<key>",..},"message":".."}`.

use std::fmt;

use axum::http::StatusCode;
use df_content::world_bible::{BibleError, PriceQuote, QuoteError};
use serde::{Deserialize, Serialize};

use crate::dev_http::{BodyError, DevReply, RouteError};
use crate::io_cause::IoCause;

/// The answer of a world route: its value, or why it was refused.
pub type WorldReply<T> = DevReply<T, WorldError>;

/// `POST /dev/world/place`: the place to be at, or none to be nowhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceRequest {
    pub place: Option<String>,
}

/// The query of `GET /dev/world/quote`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuoteQuery {
    pub item: String,
    pub qty: u32,
}

/// `{id, description}` of a voice pack, or `{id, name}` of a place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listed {
    pub id: String,
    pub text: String,
}

/// What `GET /dev/world` answers: the bible's contents by id, the active place, and the facts the
/// persona prompts receive for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldView {
    pub place: Option<String>,
    pub items: Vec<String>,
    pub places: Vec<Listed>,
    pub voice_packs: Vec<Listed>,
    /// `null` when no place is active, or the active place is not in the bible.
    pub facts: serde_json::Value,
}

/// What `POST /dev/world/place` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceSet {
    pub place: Option<String>,
}

/// What `GET /dev/world/quote` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quoted {
    pub place: String,
    pub item: String,
    pub quote: PriceQuote,
}

/// What `GET /dev/world/validate` answers when the bible is good.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Validated {
    pub items: usize,
    pub places: usize,
    pub stalls: usize,
    pub voice_packs: usize,
}

/// Why a world route refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum WorldError {
    InvalidBody(BodyError),
    InvalidQuery { detail: String },
    /// `world/bible.json` could not be read.
    BibleUnreadable { cause: IoCause },
    /// `world/bible.json` is not a valid bible; `cause` says where and why.
    BibleInvalid { cause: BibleError },
    /// `world/state.json` could not be read.
    StateUnreadable { cause: IoCause },
    /// `world/state.json` is not the state file.
    StateInvalid { cause: BodyError },
    StateUnwritable { cause: IoCause },
    /// The place is not an id, or not in the bible.
    UnknownPlace { id: String },
    /// A quote needs a place and none is active (`world place <id>`).
    NoActivePlace,
    /// The quote cannot be made.
    Quote { cause: QuoteError },
    Stopped,
}

impl WorldError {
    pub(crate) fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::InvalidQuery { .. } => "invalid_query",
            Self::BibleUnreadable { .. } => "bible_unreadable",
            Self::BibleInvalid { .. } => "bible_invalid",
            Self::StateUnreadable { .. } => "state_unreadable",
            Self::StateInvalid { .. } => "state_invalid",
            Self::StateUnwritable { .. } => "state_unwritable",
            Self::UnknownPlace { .. } => "unknown_place",
            Self::NoActivePlace => "no_active_place",
            Self::Quote { cause } => cause.key(),
            Self::Stopped => "stopped",
        }
    }
}

impl From<BodyError> for WorldError {
    fn from(error: BodyError) -> Self {
        Self::InvalidBody(error)
    }
}

impl fmt::Display for WorldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::InvalidQuery { detail } => write!(formatter, "the query is not valid: {detail}"),
            Self::BibleUnreadable { cause } => {
                write!(formatter, "world/bible.json could not be read: {cause}")
            }
            Self::BibleInvalid { cause } => write!(formatter, "world/bible.json: {cause}"),
            Self::StateUnreadable { cause } => {
                write!(formatter, "world/state.json could not be read: {cause}")
            }
            Self::StateInvalid { cause } => write!(formatter, "world/state.json: {cause}"),
            Self::StateUnwritable { cause } => {
                write!(formatter, "world/state.json could not be written: {cause}")
            }
            Self::UnknownPlace { id } => write!(
                formatter,
                "the bible has no place {id:?} (see: world show)"
            ),
            Self::NoActivePlace => formatter
                .write_str("no place is active (set one with: world place <id>)"),
            Self::Quote { cause } => write!(formatter, "{cause}"),
            Self::Stopped => formatter.write_str("the host stopped before the work finished"),
        }
    }
}

impl std::error::Error for WorldError {}

impl RouteError for WorldError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_) | Self::InvalidQuery { .. } => StatusCode::BAD_REQUEST,
            Self::UnknownPlace { .. } => StatusCode::NOT_FOUND,
            Self::NoActivePlace | Self::Quote { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::BibleUnreadable { .. }
            | Self::BibleInvalid { .. }
            | Self::StateUnreadable { .. }
            | Self::StateInvalid { .. }
            | Self::StateUnwritable { .. }
            | Self::Stopped => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

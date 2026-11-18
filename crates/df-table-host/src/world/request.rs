//! The bodies and replies of the world routes, and why they refuse.
//!
//! Every route answers a `WorldReply<T>`: `{"outcome":"done","value":..}` or
//! `{"outcome":"rejected","error":{"error":"<key>",..},"message":".."}`.

use std::fmt;

use axum::http::StatusCode;
use df_content::world_bible::{BibleError, Place, PriceQuote, QuoteError};
use serde::{Deserialize, Serialize};

use super::inventory::ShopStock;
use crate::dev_http::{BodyError, DevReply, QueryError, RouteError};
use crate::io_cause::IoCause;

/// The answer of a world route: its value, or why it was refused.
pub type WorldReply<T> = DevReply<T, WorldError>;

/// `POST /dev/world/place`: the place to be at, or none to be nowhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceRequest {
    pub place: Option<String>,
}

/// `POST /dev/world/arrive`: a place designed in play (a move's destination, with the things in
/// sight there worth a close look) that becomes the active place. It is checked as a place of the
/// bible is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArriveRequest {
    pub place: Place,
    /// The one feature that sets the place apart from the others designed (the design's
    /// `feature`): kept with the places the party has been to, so the next design is told it.
    #[serde(default)]
    pub feature: Option<String>,
    /// The traders of the place and their stock: each stocked as a shop once the party arrives.
    #[serde(default)]
    pub shops: Vec<ShopStock>,
}

/// `POST /dev/world/picture`: keeps `picture` as the close-up of the thing `item` at the active
/// place (a place designed in play), its reference sheet from then on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThingPicture {
    pub item: String,
    pub picture: String,
}

/// `path::WORLD_THING_ADD`: a thing in sight at the active place (a place designed in play), with
/// the id the look path names it by, its name and its visible look.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThingAdd {
    pub id: String,
    pub name: String,
    pub look: String,
}

/// `path::WORLD_THING_NOTE`: a note on a thing of the active place, of a kind (`label`: its state,
/// what it reads, what is inside) that replaces the note of that kind before, and the thing's
/// name now when it changed with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThingNote {
    pub id: String,
    pub label: String,
    pub text: String,
    #[serde(default)]
    pub name: Option<String>,
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
    InvalidQuery(QueryError),
    /// `world/bible.json` could not be read.
    BibleUnreadable {
        cause: IoCause,
    },
    /// `world/bible.json` is not a valid bible; `cause` says where and why.
    BibleInvalid {
        cause: BibleError,
    },
    /// `world/state.json` could not be read.
    StateUnreadable {
        cause: IoCause,
    },
    /// `world/state.json` is not the state file.
    StateInvalid {
        cause: BodyError,
    },
    StateUnwritable {
        cause: IoCause,
    },
    /// The place is not an id, or not in the bible.
    UnknownPlace {
        id: String,
    },
    /// A place designed in play is not a valid place; `cause` says where and why.
    PlaceInvalid {
        cause: BibleError,
    },
    /// A thing's picture is kept only at a place designed in play.
    NotDesigned,
    /// The active place has no such thing.
    UnknownThing {
        id: String,
    },
    /// A walk's stage note names a place or a stage that is not an id.
    InvalidWalkStage {
        id: String,
    },
    /// A quote needs a place and none is active (`world place <id>`).
    NoActivePlace,
    /// The quote cannot be made.
    Quote {
        cause: QuoteError,
    },
    /// A quest, a board or a taking of one does not stand; `reason` says why.
    InvalidQuest {
        reason: String,
    },
    /// No quest of that id is offered or taken.
    UnknownQuest {
        id: String,
    },
    /// No quest is taken yet.
    NoQuestTaken,
    /// A region does not stand; `reason` says why.
    InvalidRegion {
        reason: String,
    },
    Stopped,
}

impl WorldError {
    pub(crate) fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::InvalidQuery(_) => "invalid_query",
            Self::BibleUnreadable { .. } => "bible_unreadable",
            Self::BibleInvalid { .. } => "bible_invalid",
            Self::StateUnreadable { .. } => "state_unreadable",
            Self::StateInvalid { .. } => "state_invalid",
            Self::StateUnwritable { .. } => "state_unwritable",
            Self::UnknownPlace { .. } => "unknown_place",
            Self::PlaceInvalid { .. } => "place_invalid",
            Self::NotDesigned => "not_designed",
            Self::UnknownThing { .. } => "unknown_thing",
            Self::InvalidWalkStage { .. } => "invalid_walk_stage",
            Self::NoActivePlace => "no_active_place",
            Self::Quote { cause } => cause.key(),
            Self::InvalidQuest { .. } => "invalid_quest",
            Self::UnknownQuest { .. } => "unknown_quest",
            Self::NoQuestTaken => "no_quest_taken",
            Self::InvalidRegion { .. } => "invalid_region",
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
            Self::InvalidQuery(error) => write!(formatter, "{error}"),
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
            Self::UnknownPlace { id } => {
                write!(formatter, "the bible has no place {id:?} (see: world show)")
            }
            Self::PlaceInvalid { cause } => write!(formatter, "the designed place: {cause}"),
            Self::NotDesigned => formatter
                .write_str("the active place was not designed in play, so it keeps no picture"),
            Self::UnknownThing { id } => write!(formatter, "the active place has no thing {id:?}"),
            Self::InvalidWalkStage { id } => {
                write!(
                    formatter,
                    "{id:?} is not an id (lowercase letters, digits, '_' or '-')"
                )
            }
            Self::NoActivePlace => {
                formatter.write_str("no place is active (set one with: world place <id>)")
            }
            Self::Quote { cause } => write!(formatter, "{cause}"),
            Self::InvalidQuest { reason } => write!(formatter, "the quest: {reason}"),
            Self::UnknownQuest { id } => write!(formatter, "no quest {id:?} is on the board"),
            Self::NoQuestTaken => formatter.write_str("no quest is taken yet"),
            Self::InvalidRegion { reason } => write!(formatter, "the region: {reason}"),
            Self::Stopped => formatter.write_str("the host stopped before the work finished"),
        }
    }
}

impl std::error::Error for WorldError {}

impl RouteError for WorldError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_)
            | Self::InvalidQuery(_)
            | Self::InvalidWalkStage { .. }
            | Self::PlaceInvalid { .. }
            | Self::InvalidQuest { .. }
            | Self::InvalidRegion { .. } => StatusCode::BAD_REQUEST,
            Self::UnknownPlace { .. } | Self::UnknownThing { .. } | Self::UnknownQuest { .. } => {
                StatusCode::NOT_FOUND
            }
            Self::NoActivePlace | Self::NotDesigned | Self::Quote { .. } | Self::NoQuestTaken => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            Self::BibleUnreadable { .. }
            | Self::BibleInvalid { .. }
            | Self::StateUnreadable { .. }
            | Self::StateInvalid { .. }
            | Self::StateUnwritable { .. }
            | Self::Stopped => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

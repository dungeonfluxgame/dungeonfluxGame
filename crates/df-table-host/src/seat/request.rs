//! The typed error of the seat routes, and the bodies and replies of their dev routes. The player
//! routes' bodies and replies are the phone's (`df_playground::seat::wire`).

use std::fmt;

use axum::http::StatusCode;
use df_playground::pages::wire::PagesErrorKind;
use df_playground::seat::wire::{
    AskKind, CastErrorKind, FriendCard, HeroCard, PlayerKind, SayKind, SeatCard, SeatErrorKind,
    SuggestedLine,
};
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
    /// The seat plays no character, so it has nothing to say.
    NoPersona,
    /// The table is not asking this seat anything now.
    NothingAsked,
    /// The line has no words.
    EmptyLine,
    /// The character is played from another seat: only one seat speaks for it.
    PersonaTaken {
        seat: u64,
    },
    /// No stored persona has this id.
    UnknownPersona {
        persona: String,
    },
    /// The table's scenario needs this seat's hero, chosen and locked in, to start the day.
    NoHeroToPlay,
    /// The answer is to ask `ask`, which the table no longer asks.
    StaleAsk {
        ask: u64,
    },
    /// The answer names a thing none of the open ask's cards is about.
    UnknownThing {
        thing: String,
    },
    /// A line was taken before with the draft id `said` and other words.
    SaidChanged {
        said: u64,
    },
    /// The line is said to someone the shown scene does not have.
    UnknownListener {
        to: String,
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
    /// The room code a new phone carried is not the table's.
    CodeMismatch,
    /// The table cannot do this kind of seat action yet: its owner has not landed.
    NotReady {
        action: String,
    },
    /// The clue to show is not on the clue board shown.
    UnknownClue {
        clue: String,
    },
    /// No clue is shown to the table, so there is none to close.
    NoClueShown,
    /// A party ask with no seat playing the day.
    NoParty,
    /// The seat at the table has no hero locked in, so the party's day cannot start.
    PartyNotReady {
        seat: u64,
    },
    /// The shop (its keeper's id) is not one of the shops where the party is.
    NotHere {
        shop: String,
    },
    /// The action with the phone's id `said` is still being done; a retry gets its answer.
    InFlight {
        said: u64,
    },
    /// The address tried a room code inside its wait after a wrong one; it may try again in
    /// `retry_after_s` seconds.
    TooManyGuesses {
        retry_after_s: u64,
    },
    /// A runner's seat call with no run token, or another run's, for a seat of the day running now
    /// (or any run token while no day runs): a runner an earlier host left behind.
    NotThisRun,
}

impl SeatError {
    /// The refusal as the phone reads it: one of the shared `SeatErrorKind`s, whose key is the
    /// error's tag.
    pub fn kind(&self) -> SeatErrorKind {
        match self {
            Self::MissingKey => SeatErrorKind::MissingKey,
            Self::InvalidKey => SeatErrorKind::InvalidKey,
            Self::UnknownKey => SeatErrorKind::UnknownKey,
            Self::UnknownHero { .. } => SeatErrorKind::UnknownHero,
            Self::NoPersona => SeatErrorKind::NoPersona,
            Self::NothingAsked => SeatErrorKind::NothingAsked,
            Self::EmptyLine => SeatErrorKind::EmptyLine,
            Self::PersonaTaken { .. } => SeatErrorKind::PersonaTaken,
            Self::UnknownPersona { .. } => SeatErrorKind::UnknownPersona,
            Self::UnknownListener { .. } => SeatErrorKind::UnknownListener,
            Self::NoHeroToPlay => SeatErrorKind::NoHeroToPlay,
            Self::UnknownThing { .. } => SeatErrorKind::UnknownThing,
            Self::StaleAsk { .. } => SeatErrorKind::StaleAsk,
            Self::SaidChanged { .. } => SeatErrorKind::SaidChanged,
            Self::InvalidName => SeatErrorKind::InvalidName,
            Self::UnknownFriend { .. } => SeatErrorKind::UnknownFriend,
            Self::AccountsNotAvailable => SeatErrorKind::AccountsNotAvailable,
            Self::TableFull => SeatErrorKind::TableFull,
            Self::GameStarted => SeatErrorKind::GameStarted,
            Self::PendingFull => SeatErrorKind::PendingFull,
            Self::TableClosed => SeatErrorKind::TableClosed,
            Self::NotApplied => SeatErrorKind::NotApplied,
            Self::UnknownRequest { .. } => SeatErrorKind::UnknownRequest,
            Self::UnknownSeat { .. } => SeatErrorKind::UnknownSeat,
            Self::FriendsFull => SeatErrorKind::FriendsFull,
            Self::HeroesFull => SeatErrorKind::HeroesFull,
            Self::InvalidBody(_) => SeatErrorKind::InvalidBody,
            Self::Storage { .. } => SeatErrorKind::Storage,
            Self::Engine { .. } => SeatErrorKind::Engine,
            Self::CodeMismatch => SeatErrorKind::CodeMismatch,
            Self::NotReady { .. } => SeatErrorKind::NotReady,
            Self::UnknownClue { .. } => SeatErrorKind::UnknownClue,
            Self::NoClueShown => SeatErrorKind::NoClueShown,
            Self::NoParty => SeatErrorKind::NoParty,
            Self::PartyNotReady { .. } => SeatErrorKind::PartyNotReady,
            Self::NotHere { .. } => SeatErrorKind::NotHere,
            Self::InFlight { .. } => SeatErrorKind::InFlight,
            Self::TooManyGuesses { .. } => SeatErrorKind::TooManyGuesses,
            Self::NotThisRun => SeatErrorKind::NotThisRun,
        }
    }

    pub fn key(&self) -> &'static str {
        self.kind().key()
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
            Self::NoPersona => formatter.write_str("this seat plays no character yet"),
            Self::NothingAsked => formatter.write_str("nothing is being asked of you right now"),
            Self::EmptyLine => formatter.write_str("say something first"),
            Self::PersonaTaken { seat } => {
                write!(formatter, "that character is played from seat {seat}")
            }
            Self::UnknownPersona { persona } => write!(formatter, "there is no persona {persona}"),
            Self::UnknownListener { to } => write!(formatter, "{to} is not here to hear it"),
            Self::StaleAsk { ask } => write!(formatter, "ask {ask} is no longer open"),
            Self::SaidChanged { said } => {
                write!(formatter, "line {said} was already sent with other words")
            }
            Self::UnknownThing { thing } => {
                write!(formatter, "{thing} is not a thing the open ask offers")
            }
            Self::NoHeroToPlay => {
                formatter.write_str("the day starts once your hero is chosen and locked in")
            }
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
            Self::CodeMismatch => formatter.write_str("that is not this table's room code"),
            Self::NotReady { action } => {
                write!(formatter, "the table cannot {action} yet")
            }
            Self::UnknownClue { clue } => write!(formatter, "the clue board has no clue {clue}"),
            Self::NoClueShown => formatter.write_str("no clue is shown to the table"),
            Self::NoParty => formatter.write_str("no seat plays the day, so there is no party"),
            Self::PartyNotReady { seat } => {
                write!(formatter, "seat {seat} has no hero locked in yet")
            }
            Self::NotHere { shop } => write!(formatter, "the shop of {shop} is not here"),
            Self::InFlight { said } => write!(formatter, "action {said} is still being done"),
            Self::TooManyGuesses { retry_after_s } => write!(
                formatter,
                "too many wrong room codes; try again in {retry_after_s} s"
            ),
            Self::NotThisRun => {
                formatter.write_str("this runner is not the table's current run; stop it")
            }
        }
    }
}

impl std::error::Error for SeatError {}

impl RouteError for SeatError {
    fn status(&self) -> StatusCode {
        match self {
            Self::MissingKey | Self::InvalidKey | Self::InvalidName | Self::EmptyLine => {
                StatusCode::BAD_REQUEST
            }
            // A body too large is 413, as every route answers it.
            Self::InvalidBody(error) => error.status(),
            Self::UnknownKey
            | Self::UnknownHero { .. }
            | Self::UnknownPersona { .. }
            | Self::UnknownListener { .. }
            | Self::UnknownThing { .. }
            | Self::UnknownFriend { .. }
            | Self::UnknownRequest { .. }
            | Self::UnknownSeat { .. }
            | Self::UnknownClue { .. }
            | Self::NotHere { .. } => StatusCode::NOT_FOUND,
            Self::CodeMismatch => StatusCode::FORBIDDEN,
            Self::TooManyGuesses { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::NotReady { .. } => StatusCode::NOT_IMPLEMENTED,
            Self::AccountsNotAvailable => StatusCode::NOT_IMPLEMENTED,
            Self::TableFull
            | Self::GameStarted
            | Self::PendingFull
            | Self::NoPersona
            | Self::NoHeroToPlay
            | Self::NothingAsked
            | Self::StaleAsk { .. }
            | Self::SaidChanged { .. }
            | Self::PersonaTaken { .. }
            | Self::NotApplied
            | Self::FriendsFull
            | Self::HeroesFull
            | Self::NoClueShown
            | Self::NoParty
            | Self::PartyNotReady { .. }
            | Self::InFlight { .. }
            | Self::NotThisRun => StatusCode::CONFLICT,
            Self::TableClosed => StatusCode::SERVICE_UNAVAILABLE,
            Self::Storage { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Engine { cause } => cause.status(),
        }
    }
}

/// The answer of a seat route.
pub type SeatReply<T> = DevReply<T, SeatError>;

/// Every refusal of `PLAY_SEAT_ACTION`, one family: each written in its own route error's shape
/// (`{"error": "<kind>", ..}`), so the tag is always the leaf kind the phone reads
/// (`SeatActionErrorKind`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SeatActionError {
    Seat(SeatError),
    Pages(crate::seat_pages::PagesError),
    Roll(crate::persona::check::PendingRollError),
    Cast(CastRefusal),
    Buy(BuyRefusal),
}

/// A purchase by a tap refused (`world::inventory::buy`; nothing moved), in the phone's pages kind
/// (`error`, `InventoryError::pages_kind`) and the inventory's own sentence (`detail`, for the
/// log).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuyRefusal {
    pub error: PagesErrorKind,
    pub detail: String,
}

impl fmt::Display for BuyRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl From<crate::world::InventoryError> for SeatActionError {
    fn from(error: crate::world::InventoryError) -> Self {
        Self::Buy(BuyRefusal {
            error: error.pages_kind(),
            detail: error.to_string(),
        })
    }
}

/// A cast the rules refused (`creation::cast_spell`), in the phone's words for it
/// (`CastErrorKind`); `detail` is the rules' own sentence, for the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CastRefusal {
    pub error: CastErrorKind,
    pub detail: String,
}

impl fmt::Display for CastRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl From<crate::creation::CastError> for SeatActionError {
    fn from(error: crate::creation::CastError) -> Self {
        use crate::creation::CastError;
        use df_rules::hero::CastError as Rules;
        let refused = |kind: CastErrorKind, detail: String| {
            Self::Cast(CastRefusal {
                error: kind,
                detail,
            })
        };
        match error {
            CastError::Refused { cause } => {
                let kind = match &cause {
                    Rules::NotACaster => CastErrorKind::NotACaster,
                    Rules::UnknownSpell { .. } => CastErrorKind::UnknownSpell,
                    Rules::NotKnown { .. } => CastErrorKind::NotKnown,
                    Rules::NotPrepared { .. } => CastErrorKind::NotPrepared,
                    Rules::NotARitual { .. } => CastErrorKind::NotARitual,
                    Rules::InvalidLevel { .. } => CastErrorKind::InvalidLevel,
                    Rules::NoSlot { .. } => CastErrorKind::NoSlot,
                };
                refused(kind, cause.to_string())
            }
            CastError::InvalidVitals { cause } => {
                refused(CastErrorKind::InvalidVitals, cause.to_string())
            }
            CastError::UnknownHero { hero } => Self::Seat(SeatError::UnknownHero { hero }),
            CastError::Storage { cause } => Self::Seat(SeatError::Storage { cause }),
            CastError::Stopped => Self::Seat(SeatError::Storage {
                cause: IoCause::Other,
            }),
            CastError::Creation { cause } => Self::Seat(crate::creation::seat_error(cause)),
        }
    }
}

impl SeatActionError {
    /// The error's tag, for the audit line.
    pub fn key(&self) -> &'static str {
        match self {
            Self::Seat(error) => error.key(),
            Self::Pages(error) => error.kind().key(),
            Self::Roll(error) => error.key(),
            Self::Cast(refusal) => refusal.error.key(),
            Self::Buy(refusal) => refusal.error.key(),
        }
    }
}

impl From<SeatError> for SeatActionError {
    fn from(error: SeatError) -> Self {
        Self::Seat(error)
    }
}

impl From<crate::persona::check::PendingRollError> for SeatActionError {
    fn from(error: crate::persona::check::PendingRollError) -> Self {
        Self::Roll(error)
    }
}

impl From<crate::seat_pages::PagesError> for SeatActionError {
    fn from(error: crate::seat_pages::PagesError) -> Self {
        Self::Pages(error)
    }
}

impl fmt::Display for SeatActionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Seat(error) => error.fmt(formatter),
            Self::Pages(error) => error.fmt(formatter),
            Self::Roll(error) => error.fmt(formatter),
            Self::Cast(refusal) => refusal.fmt(formatter),
            Self::Buy(refusal) => refusal.fmt(formatter),
        }
    }
}

impl std::error::Error for SeatActionError {}

impl RouteError for SeatActionError {
    fn status(&self) -> StatusCode {
        match self {
            Self::Seat(error) => error.status(),
            Self::Pages(error) => error.status(),
            // A roll no longer waiting is the table having moved on.
            Self::Roll(_) => StatusCode::CONFLICT,
            // The rules refused the cast as the hero stands now.
            Self::Cast(_) => StatusCode::CONFLICT,
            // The purchase does not stand as the bag, the purse and the shop are now.
            Self::Buy(_) => StatusCode::CONFLICT,
        }
    }
}

/// The answer of `PLAY_SEAT_ACTION`.
pub type SeatActionReply = DevReply<df_playground::seat::wire::SeatOutcome, SeatActionError>;

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

/// `path::PERSONA`: the character `seat` plays, or none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaBind {
    pub seat: u64,
    #[serde(default)]
    pub persona: Option<String>,
}

/// Which seats a dev route is about: the seats named, or the party (every seat of the day running
/// now, else every seat that plays a character).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatsNamed {
    Seats(Vec<u64>),
    Party,
}

/// `path::ASK`: what the table asks the seats `to` now. It replaces what each was asked. A party
/// ask (more than one seat) is one ask: the first seat to answer it answers for all, and each
/// other seat is told who did (`SeatView::answered_by`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskSet {
    pub to: SeatsNamed,
    #[serde(default)]
    pub kind: AskKind,
    pub prompt: String,
    #[serde(default)]
    pub cards: Vec<SuggestedLine>,
    #[serde(default)]
    pub note: Option<String>,
}

/// `path::ASK`'s answer: the ask's id and the seats asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asked {
    pub ask: u64,
    pub seats: Vec<u64>,
}

/// `path::ASK_CLEAR`: stop asking the seats `to`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskClear {
    pub to: SeatsNamed,
}

/// `path::NOTE`: a word from the table for `seat`, shown above what it is asked until the next ask.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatNote {
    pub seat: u64,
    pub text: String,
}

/// `path::INBOX`: the next line one of the seats `from` says (the oldest waiting), waiting up to
/// `wait_ms` for one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InboxWait {
    pub from: SeatsNamed,
    #[serde(default)]
    pub wait_ms: u64,
}

/// `path::INBOX`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Inbox {
    /// The hero of seat `seat` cast `spell` (its catalog id, named `name`) at `level` (0 for a
    /// cantrip), as a Ritual or not: the slot is spent, and the dungeon master narrates what it
    /// does (`summary`).
    Cast {
        seat: u64,
        spell: String,
        name: String,
        level: u8,
        ritual: bool,
        summary: String,
        at_ms: u64,
    },
    /// The seat `seat` said `text`, in answer to ask `ask` or as talk (`kind`), to the character
    /// `to` when it named one.
    Said {
        seat: u64,
        #[serde(default)]
        ask: Option<u64>,
        #[serde(default)]
        kind: SayKind,
        /// The thing of the ask's card the answer is, by id.
        #[serde(default)]
        thing: Option<String>,
        text: String,
        #[serde(default)]
        to: Option<String>,
        /// When the host took the line, in milliseconds since the Unix epoch.
        #[serde(default)]
        at_ms: u64,
    },
    /// Nothing came before the wait ran out.
    Empty,
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
    /// The character the seat plays, when it plays one.
    pub persona: Option<String>,
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
    /// The room code a new phone carries now (shown on the TV lobby).
    pub room_code: String,
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

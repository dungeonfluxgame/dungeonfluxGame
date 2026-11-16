//! The bodies and replies of the player routes (`path::PLAY_ROOM` to `path::PLAY_SEAT_INTENT`),
//! shared by the host that answers them and the phone that reads them, so neither can drift.
//!
//! Every reply travels in the dev envelope (`dev_reply::DevReply`); a refusal's `error` is the
//! host's typed seat error, of which the phone reads the tag (`SeatErrorKind`). Nothing here
//! carries a seat key or its hash: the key travels only in `path::SEAT_KEY_HEADER`.

use serde::{Deserialize, Serialize};

use crate::dev_reply::{ErrorTag, HostReply};

/// How many hexadecimal digits a seat key has: 256 random bits (REF-046).
pub const SEAT_KEY_DIGITS: usize = 64;

/// Whether `text` is a seat key as a phone makes one: `SEAT_KEY_DIGITS` lowercase hexadecimal
/// digits.
pub fn is_seat_key(text: &str) -> bool {
    text.len() == SEAT_KEY_DIGITS
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// `bytes` as lowercase hexadecimal digits, two to a byte: a seat key from 32 random bytes, and the
/// host's hash of one.
pub fn hex_digits(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Which kind of player holds a seat. The host decides it from the key or the approval; the phone
/// never does. The TV is never told it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerKind {
    /// Anyone at the table, known by the key kept in this phone's browser.
    Guest,
    /// A friend on the host's account, approved by the host.
    Friend,
}

impl PlayerKind {
    pub const ALL: [Self; 2] = [Self::Guest, Self::Friend];

    pub fn key(self) -> &'static str {
        match self {
            Self::Guest => "guest",
            Self::Friend => "friend",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }
}

/// Whether the room takes players.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomState {
    /// The lobby gathers: a phone may take a seat.
    Gathering,
    /// The game has started: only a phone that holds a seat comes back.
    Playing,
    /// The table is not running.
    Closed,
}

impl RoomState {
    pub const ALL: [Self; 3] = [Self::Gathering, Self::Playing, Self::Closed];

    pub fn key(self) -> &'static str {
        match self {
            Self::Gathering => "gathering",
            Self::Playing => "playing",
            Self::Closed => "closed",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.key() == key)
    }
}

/// `path::PLAY_ROOM`: the room as the phone shows it before anyone says who they are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomReply {
    /// Tonight's adventure's title, when the table shows one.
    pub adventure: Option<String>,
    /// The adventure's picture in the media library, when it has one.
    pub picture: Option<String>,
    /// The names of the players who hold a seat, by seat.
    pub players: Vec<String>,
    pub state: RoomState,
}

/// A friend profile the host keeps on the account, as "Who are you?" lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendCard {
    pub id: u64,
    pub name: String,
}

/// A seat the host gave this phone's key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatCard {
    pub seat: u64,
    pub name: String,
    pub kind: PlayerKind,
    /// The hero the player chose for this seat, or none until they choose one.
    pub hero: Option<u64>,
}

/// `path::PLAY_HELLO`: what the host knows of this phone's key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum HelloReply {
    /// The key holds a seat the table has now: the phone goes straight to it.
    Seated { seat: SeatCard },
    /// The key holds a seat the table does not have now (the host restarted, say): "Continue as
    /// <name>" first, and the other ways in.
    Returning {
        seat: SeatCard,
        friends: Vec<FriendCard>,
    },
    /// The key is new here: "Who are you?".
    Unknown { friends: Vec<FriendCard> },
}

/// `path::PLAY_JOIN`: the way in the player chose. The seat always comes from the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum JoinRequest {
    /// Back to the seat the key holds.
    Returning,
    /// A guest with the name they typed (REF-061 name rule).
    Guest { name: String },
    /// A friend profile of the account; the host approves it.
    Friend { profile: u64 },
    /// Signing in to one's own account.
    Account,
}

/// `path::PLAY_JOIN` and `path::PLAY_JOIN_STATUS`: where this phone's request for a seat stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum JoinStatus {
    Seated {
        seat: SeatCard,
    },
    /// The host has not answered the request numbered `request` yet.
    AwaitingApproval {
        request: u64,
    },
    /// The host refused the request.
    Refused,
    /// The key has no seat and no request.
    NotAsked,
}

/// A hero the player may play, as the hero list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeroCard {
    pub id: u64,
    pub name: String,
    pub species: String,
    pub class: String,
    /// The portrait in the media library, once one is made.
    pub portrait: Option<String>,
    /// When the hero last played, in milliseconds since the Unix epoch.
    pub last_played_ms: Option<u64>,
}

/// `path::PLAY_HEROES`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeroesReply {
    pub heroes: Vec<HeroCard>,
}

/// `path::PLAY_HERO`'s body: the hero the player plays at their seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeroChoice {
    pub hero: u64,
}

/// Where the seat's table stands, as the host reads it from the seat's view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableStage {
    /// The lobby gathers the players.
    Gathering,
    /// The table has left the lobby: the game is on.
    Playing,
}

/// A player who holds a seat, as the seat's view lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TablePlayer {
    pub name: String,
    /// At the table now; a player who holds the seat but has not come back is away.
    pub present: bool,
    pub ready: bool,
    /// This phone's own seat.
    pub you: bool,
}

/// `path::PLAY_SEAT_VIEW`: the seat's view of the table, as typed data the phone words itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatView {
    pub revision: u64,
    pub stage: TableStage,
    /// Whether this seat said it is ready.
    pub ready: bool,
    /// Whether the lobby offers this seat to say it is ready, or not ready.
    pub can_toggle_ready: bool,
    /// Everyone who holds a seat, by seat.
    pub players: Vec<TablePlayer>,
}

/// A lobby action a seat sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatAction {
    Ready,
    Unready,
    Leave,
}

impl SeatAction {
    pub const ALL: [Self; 3] = [Self::Ready, Self::Unready, Self::Leave];

    /// The lobby's offer id for the action (`df_table::lobby::LobbyAction`'s key).
    pub fn key(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Unready => "unready",
            Self::Leave => "leave",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.key() == key)
    }
}

/// `path::PLAY_SEAT_INTENT`'s body. The seat comes from the key, never from the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatIntent {
    pub action: SeatAction,
}

/// What the table did with a seat's action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SeatIntentReply {
    Applied,
    /// The table declined it, with the sentence the seat is shown.
    Declined {
        reason: String,
    },
    /// The seat left the table; the key no longer holds it.
    Left,
}

/// The refusals of the player routes the phone words, by the tag of the host's typed error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatErrorKind {
    MissingKey,
    InvalidKey,
    /// The key holds no seat.
    UnknownKey,
    /// The hero is not one this player may play.
    UnknownHero,
    InvalidName,
    UnknownFriend,
    /// Signing in to one's own account comes later on this table.
    AccountsNotAvailable,
    TableFull,
    /// The game has started; a new player cannot take a seat now.
    GameStarted,
    /// The host has too many requests waiting.
    PendingFull,
    /// The table is not running.
    TableClosed,
}

impl SeatErrorKind {
    pub const ALL: [Self; 11] = [
        Self::MissingKey,
        Self::InvalidKey,
        Self::UnknownKey,
        Self::UnknownHero,
        Self::InvalidName,
        Self::UnknownFriend,
        Self::AccountsNotAvailable,
        Self::TableFull,
        Self::GameStarted,
        Self::PendingFull,
        Self::TableClosed,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::MissingKey => "missing_key",
            Self::InvalidKey => "invalid_key",
            Self::UnknownKey => "unknown_key",
            Self::UnknownHero => "unknown_hero",
            Self::InvalidName => "invalid_name",
            Self::UnknownFriend => "unknown_friend",
            Self::AccountsNotAvailable => "accounts_not_available",
            Self::TableFull => "table_full",
            Self::GameStarted => "game_started",
            Self::PendingFull => "pending_full",
            Self::TableClosed => "table_closed",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }
}

impl ErrorTag for SeatErrorKind {
    fn from_tag(tag: &str) -> Option<Self> {
        Self::from_key(tag)
    }
}

/// A player route's answer as the phone reads it: a refusal reads as the `SeatErrorKind` it
/// names, or none for another tag (a body the host could not read, a guard refusal).
pub type SeatReply<T> = HostReply<T, SeatErrorKind>;

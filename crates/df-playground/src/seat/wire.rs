//! The bodies and replies of the player routes (`path::PLAY_ROOM` to `path::PLAY_SEAT_ACTION`),
//! shared by the host that answers them and the phone that reads them, so neither can drift.
//!
//! Every reply travels in the dev envelope (`dev_reply::DevReply`); a refusal's `error` is the
//! host's typed seat error, of which the phone reads the tag (`SeatErrorKind`). Nothing here
//! carries a seat key or its hash: the key travels only in `path::SEAT_KEY_HEADER`.

use serde::{Deserialize, Serialize};

use crate::dev_reply::{ErrorTag, HostReply};
use crate::fixture::{RollMode, RollOutcome, RollTarget};
use crate::look::wire::HeroPack;

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
    /// Whether the chosen hero's look is locked in (its approved portrait made). Until it is, the
    /// player belongs in the look studio.
    #[serde(default)]
    pub hero_locked: bool,
    /// The character the seat's hero plays (`danny`), when the host bound one to it.
    #[serde(default)]
    pub plays: Option<String>,
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

/// `path::PLAY_JOIN`: the way in the player chose. The seat always comes from the key. A new way
/// in carries the room code the table shows (`code`, as `lobby::model::room_code` writes it): a
/// code that is not the table's is refused `code_mismatch`. Coming back to the seat the key holds
/// needs none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum JoinRequest {
    /// Back to the seat the key holds.
    Returning,
    /// A guest with the name they typed (REF-061 name rule).
    Guest { name: String, code: String },
    /// A friend profile of the account; the host approves it.
    Friend { profile: u64, code: String },
    /// Signing in to one's own account.
    Account { code: String },
}

/// `path::PLAY_JOIN_CODE`'s body: the room code typed at Next, checked before the join choices
/// as a join checks it (`code_mismatch`, `too_many_guesses`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeCheck {
    pub code: String,
}

/// `path::PLAY_JOIN_CODE`'s answer: the code is the table's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeTaken {}

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
    /// The pictures made at lock-in (the headshot the hero list shows).
    #[serde(default)]
    pub pack: HeroPack,
    /// When the hero last played, in milliseconds since the Unix epoch.
    pub last_played_ms: Option<u64>,
}

/// `path::PLAY_HEROES`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeroesReply {
    pub heroes: Vec<HeroCard>,
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

/// The hero a player plays at the table, as the lobby shows them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableHero {
    pub id: u64,
    pub name: String,
    pub species: String,
    pub class: String,
    /// The approved portrait: the version kept at lock-in.
    pub portrait: Option<String>,
    /// The pictures made from it at lock-in: sheet, headshot, half-body portrait, ready pose.
    pub pack: HeroPack,
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
    /// The hero the player plays, once they chose one.
    #[serde(default)]
    pub hero: Option<TableHero>,
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
    /// Whether the lobby offers this seat to start the game.
    #[serde(default)]
    pub can_start: bool,
    /// Everyone who holds a seat, by seat.
    pub players: Vec<TablePlayer>,
    /// What the table asks this seat now, when it asks something.
    pub ask: Option<SeatAsk>,
    /// The story as this seat's hero lives it, when the seat plays a character (boxed: the view
    /// travels in the seat machine's events).
    pub story: Option<Box<SeatStory>>,
    /// How the table's day for this seat ended, once it ended.
    #[serde(default)]
    pub day_end: Option<DayEnd>,
    /// Why the game did not start, in the player's words ("The game couldn't start: ..."), when a
    /// Start failed; the lobby is gathering again with nobody ready.
    #[serde(default)]
    pub start_problem: Option<String>,
    /// The Talk lines this seat sent that the table has not taken yet: queued, not said. A day
    /// that ends drops them, and this is none again.
    #[serde(default)]
    pub talk_waiting: u32,
    /// Whether this seat talks by voice now, and whether the table can write speech down at all.
    #[serde(default)]
    pub stt: SeatStt,
    /// A word from the table for this seat (the day's runner), shown until its next ask.
    #[serde(default)]
    pub notice: Option<String>,
    /// The roll the dungeon master called for this seat's hero and waits on: the phone shows the
    /// call as the shared screen does, and its Roll button answers it (`SeatAction::Roll`).
    #[serde(default)]
    pub pending_roll: Option<Box<PendingRoll>>,
    /// The last roll of this seat's hero as the shared screen showed it: the same card, drawn by
    /// the same renderer (`kit::roll_card`).
    #[serde(default)]
    pub last_roll: Option<Box<crate::fixture::RollView>>,
    /// The party's ask this seat was asked too, answered from another seat: who answered it. Shown
    /// until the seat's next ask.
    #[serde(default)]
    pub answered_by: Option<AnsweredBy>,
}

/// A party ask answered from another seat: the ask, the seat that answered first, its hero's name
/// and the words it answered with (a card's line, or its own words), as written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnsweredBy {
    pub ask: u64,
    pub seat: u64,
    pub hero: String,
    pub said: String,
}

/// A seat's talking by voice: its own switch (off until the player turns it on; off again when
/// the host restarts or the seat changes hands), and whether the table has speech to text set up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatStt {
    pub on: bool,
    pub available: bool,
}

/// How the day the table played for a seat ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DayEnd {
    /// The day could not start.
    NotStarted,
    /// The day stopped before its end.
    Failed,
    /// The day played to its end.
    Finished,
}

impl DayEnd {
    /// What the phone tells the player: what happened, and what to do now.
    pub fn player_words(self) -> (&'static str, &'static str) {
        match self {
            Self::NotStarted => (
                "The day could not start.",
                "You are back at the table: get ready to try again, or ask the host to look.",
            ),
            Self::Failed => (
                "The day stopped early.",
                "You are back at the table: get ready to try again, or ask the host to look.",
            ),
            Self::Finished => (
                "The day is over.",
                "You are back at the table: change your hero, get ready and start a new day.",
            ),
        }
    }
}

/// A suggested answer to an ask: what its card says, and the line it sends, as written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuggestedLine {
    pub label: String,
    pub line: String,
    /// The thing of the place the card is about (a look's), by its id.
    #[serde(default)]
    pub thing: Option<String>,
}

/// What kind of answer an ask wants, so the phone never reads the prompt to know.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskKind {
    /// A line in the character's own words.
    #[default]
    Line,
    /// Where the character goes next.
    Move,
    /// What the character steps up to and looks at closely.
    Look,
}

impl AskKind {
    pub const ALL: [Self; 3] = [Self::Line, Self::Move, Self::Look];

    pub fn key(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Move => "move",
            Self::Look => "look",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }
}

/// What the table asks a seat now: a short prompt, suggested answers, and a note (why the last
/// answer could not be taken, say).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatAsk {
    pub id: u64,
    #[serde(default)]
    pub kind: AskKind,
    pub prompt: String,
    pub cards: Vec<SuggestedLine>,
    pub note: Option<String>,
}

/// A line the table heard: a character's (by its id) or the narrator's (no speaker).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryLine {
    /// The line's own number, the same for as long as the host keeps it (a kept line names it).
    pub id: u64,
    pub speaker: Option<String>,
    /// Said by this seat's own character.
    pub you: bool,
    pub text: String,
}

/// The story as a seat's character lives it: where they are, the picture on the shared screen,
/// the lines heard so far (oldest first) and their own latest thought.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatStory {
    /// The character's name.
    pub hero: String,
    pub place: Option<String>,
    /// The picture the shared screen shows, in the media library.
    pub plate: Option<String>,
    pub lines: Vec<StoryLine>,
    /// What they last thought on seeing the screen: private to this seat. A thought the model
    /// reported as the character's private side is never here.
    pub thought: Option<SeatThought>,
}

/// A character's thought on seeing the screen, as its parts: the thought, what they saw (others,
/// never themself) and what they want.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatThought {
    pub thought: String,
    pub saw: Vec<String>,
    pub wanted: Vec<String>,
}

/// What a seat's line is: the answer to what the table asks now (a card, the Act page's
/// composer), or talk (the Talk page): a turn of its own that never answers the ask.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SayKind {
    #[default]
    Answer,
    Talk,
}

/// A lobby action a seat sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LobbyStep {
    Ready,
    Unready,
    Leave,
    /// Start the game: the lobby takes it once every seat at the table is ready.
    Start,
}

impl LobbyStep {
    pub const ALL: [Self; 4] = [Self::Ready, Self::Unready, Self::Leave, Self::Start];

    /// The lobby's offer id for the action (`df_table::lobby::LobbyAction`'s key).
    pub fn key(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Unready => "unready",
            Self::Leave => "leave",
            Self::Start => "start",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.key() == key)
    }
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

/// Everything a seated phone does at the table, as one typed action (`path::PLAY_SEAT_ACTION`).
/// Every action carries the phone's own id for it, `said`: made once per action and sent again
/// with a retry, so the same id and body is answered as before and done once, and the same id with
/// another body is refused `said_changed`. The seat comes from the key, never from the body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum SeatAction {
    /// The player's line answers what the table asks (`ask`, the ask the phone shows: an answer to
    /// an ask no longer open is refused `stale_ask`); a card's line names the thing it is about.
    Answer {
        said: u64,
        #[serde(default)]
        ask: Option<u64>,
        text: String,
        #[serde(default)]
        to: Option<String>,
        #[serde(default)]
        thing: Option<String>,
    },
    /// The player's line as a turn of its own, to someone here or the room.
    Talk {
        said: u64,
        text: String,
        #[serde(default)]
        to: Option<String>,
    },
    /// A step in the lobby: ready, not ready, start the day, leave the table.
    Lobby { said: u64, step: LobbyStep },
    /// The hero the player plays at their seat, one of their own.
    Hero { said: u64, hero: u64 },
    /// The seat's own switch for talking by voice.
    Voice { said: u64, on: bool },
    /// Keep something in the seat's notebook.
    Note {
        said: u64,
        note: crate::pages::wire::NoteAsk,
    },
    /// What the seat's hands do with a thing it carries.
    Bag {
        said: u64,
        bag: crate::pages::wire::BagAct,
    },
    /// Roll the dice the dungeon master called for (`SeatView::pending_roll`), by the call's id:
    /// the host rolls them; the phone never names a number.
    Roll { said: u64, call: String },
    /// Show the clue of the clue board `clue` (its id) to the whole table, full screen, in place
    /// of the one shown.
    ShowClue { said: u64, clue: String },
    /// Take the clue shown to the whole table away; any seat may.
    CloseClue { said: u64 },
    /// Buy `qty` of the item `item` (its catalog id) at the shop `shop` (its id).
    Buy {
        said: u64,
        shop: String,
        item: String,
        qty: u32,
    },
    /// Cast the spell `spell` (its catalog id) at `level`: 0 for a cantrip, the spell's own level
    /// as a Ritual (`as_ritual`), else the level of the slot to spend.
    Cast {
        said: u64,
        spell: String,
        level: u8,
        #[serde(default)]
        as_ritual: bool,
    },
}

impl SeatAction {
    /// The phone's id for the action.
    pub fn said(&self) -> u64 {
        match self {
            Self::Answer { said, .. }
            | Self::Talk { said, .. }
            | Self::Lobby { said, .. }
            | Self::Hero { said, .. }
            | Self::Voice { said, .. }
            | Self::Note { said, .. }
            | Self::Bag { said, .. }
            | Self::Roll { said, .. }
            | Self::ShowClue { said, .. }
            | Self::CloseClue { said }
            | Self::Buy { said, .. }
            | Self::Cast { said, .. } => *said,
        }
    }

    /// The action with the id `said` in place of its own: two actions are the same when they are
    /// equal but for their ids.
    pub fn with_said(&self, id: u64) -> Self {
        let mut action = self.clone();
        match &mut action {
            Self::Answer { said, .. }
            | Self::Talk { said, .. }
            | Self::Lobby { said, .. }
            | Self::Hero { said, .. }
            | Self::Voice { said, .. }
            | Self::Note { said, .. }
            | Self::Bag { said, .. }
            | Self::Roll { said, .. }
            | Self::ShowClue { said, .. }
            | Self::CloseClue { said }
            | Self::Buy { said, .. }
            | Self::Cast { said, .. } => *said = id,
        }
        action
    }

    /// The action's closed key, its `"action"` tag.
    pub fn key(&self) -> &'static str {
        match self {
            Self::Answer { .. } => "answer",
            Self::Talk { .. } => "talk",
            Self::Lobby { .. } => "lobby",
            Self::Hero { .. } => "hero",
            Self::Voice { .. } => "voice",
            Self::Note { .. } => "note",
            Self::Bag { .. } => "bag",
            Self::Roll { .. } => "roll",
            Self::ShowClue { .. } => "show_clue",
            Self::CloseClue { .. } => "close_clue",
            Self::Buy { .. } => "buy",
            Self::Cast { .. } => "cast",
        }
    }
}

/// What an action came to (`path::PLAY_SEAT_ACTION`'s answer), one variant per kind of action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "done", rename_all = "snake_case")]
pub enum SeatOutcome {
    /// An answer or talk was taken: the ask it answered, none for talk.
    Said { ask: Option<u64> },
    /// What the lobby did with the step.
    Lobby { lobby: SeatIntentReply },
    /// The seat plays the hero now.
    Hero { seat: SeatCard },
    /// The seat's switch for talking by voice, as it is now.
    Voice { stt: SeatStt },
    /// The note kept.
    Noted { note: crate::pages::wire::NoteKept },
    /// What the bag holds of the item now, and whether terms wait for an answer.
    Bag { bag: crate::pages::wire::BagDone },
    /// The called roll was answered: the host rolls it, and the seat's view shows the card.
    Rolled { call: String },
    /// The clue shows to the whole table now.
    ClueShown { clue: String },
    /// The clue shown to the whole table, when one was, is taken away.
    ClueClosed { clue: Option<String> },
    /// The spell was cast: the slot is spent and the dungeon master narrates it.
    Cast { cast: CastDone },
    /// The purchase was made: coins and goods moved together.
    Bought { buy: crate::pages::wire::BuyDone },
}

/// A spell the seat's hero cast, as the phone shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CastDone {
    /// The spell's catalog id.
    pub spell: String,
    pub name: String,
    /// The level it was cast at: 0 for a cantrip.
    pub level: u8,
    pub ritual: bool,
    /// The level of the slot spent; none for a cantrip or a Ritual.
    pub slot_spent: Option<u8>,
    /// The spell slots of every level left.
    pub slots_left: u8,
    /// What the spell does.
    pub summary: String,
}

/// Why the rules refused a cast; nothing was spent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CastErrorKind {
    NotACaster,
    UnknownSpell,
    NotKnown,
    NotPrepared,
    NotARitual,
    InvalidLevel,
    NoSlot,
    /// The hero's vitals refused the slot for another reason.
    InvalidVitals,
}

impl CastErrorKind {
    pub const ALL: [Self; 8] = [
        Self::NotACaster,
        Self::UnknownSpell,
        Self::NotKnown,
        Self::NotPrepared,
        Self::NotARitual,
        Self::InvalidLevel,
        Self::NoSlot,
        Self::InvalidVitals,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::NotACaster => "not_a_caster",
            Self::UnknownSpell => "unknown_spell",
            Self::NotKnown => "not_known",
            Self::NotPrepared => "not_prepared",
            Self::NotARitual => "not_a_ritual",
            Self::InvalidLevel => "invalid_level",
            Self::NoSlot => "no_slot",
            Self::InvalidVitals => "invalid_vitals",
        }
    }

    /// What the phone tells the player.
    pub fn player_words(self) -> &'static str {
        match self {
            Self::NotACaster => "Your hero casts no spells.",
            Self::UnknownSpell => "There is no such spell.",
            Self::NotKnown => "You don't know that spell.",
            Self::NotPrepared => "That spell isn't prepared.",
            Self::NotARitual => "That spell can't be cast as a ritual.",
            Self::InvalidLevel => "That spell can't be cast at that level.",
            Self::NoSlot => "No spell slot of that level is left.",
            Self::InvalidVitals => SEAT_TROUBLE_WORDS,
        }
    }
}

/// Why a tap on Roll did not answer a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollTapErrorKind {
    /// No roll waits for the seat.
    NoPendingRoll,
    /// The roll tapped is not the one waiting (it was rolled, or the wait ran out).
    StaleRoll,
}

impl RollTapErrorKind {
    pub const ALL: [Self; 2] = [Self::NoPendingRoll, Self::StaleRoll];

    pub fn key(self) -> &'static str {
        match self {
            Self::NoPendingRoll => "no_pending_roll",
            Self::StaleRoll => "stale_roll",
        }
    }

    /// What the phone tells the player.
    pub fn player_words(self) -> &'static str {
        match self {
            Self::NoPendingRoll => "No roll is waiting on you right now.",
            Self::StaleRoll => "That roll was already made.",
        }
    }
}

/// Every refusal of `path::PLAY_SEAT_ACTION`, by the tag of the host's typed error: one family
/// shared with the host (`SeatActionError`), so the phone's list cannot drift from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatActionErrorKind {
    /// The seat, its key, its hero, its line or the lobby refused.
    Seat(SeatErrorKind),
    /// A notebook or bag action refused.
    Pages(crate::pages::wire::PagesErrorKind),
    /// A tap on Roll refused.
    Roll(RollTapErrorKind),
    /// A cast the rules refused.
    Cast(CastErrorKind),
}

impl SeatActionErrorKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Seat(kind) => kind.key(),
            Self::Pages(kind) => kind.key(),
            Self::Roll(kind) => kind.key(),
            Self::Cast(kind) => kind.key(),
        }
    }

    /// The refusal as a pages route words it (a seat refusal is the pages routes' seat kind).
    pub fn pages(self) -> crate::pages::wire::PagesErrorKind {
        match self {
            Self::Seat(kind) => crate::pages::wire::PagesErrorKind::Seat(kind),
            Self::Pages(kind) => kind,
            Self::Roll(_) | Self::Cast(_) => {
                crate::pages::wire::PagesErrorKind::Seat(SeatErrorKind::NothingAsked)
            }
        }
    }

    /// The seat kind of the refusal, when it is one (a lost key, a stale ask).
    pub fn seat(self) -> Option<SeatErrorKind> {
        match self {
            Self::Seat(kind) | Self::Pages(crate::pages::wire::PagesErrorKind::Seat(kind)) => {
                Some(kind)
            }
            Self::Pages(_) | Self::Roll(_) | Self::Cast(_) => None,
        }
    }
}

impl ErrorTag for SeatActionErrorKind {
    fn from_tag(tag: &str) -> Option<Self> {
        SeatErrorKind::from_key(tag)
            .map(Self::Seat)
            .or_else(|| {
                crate::pages::wire::PagesErrorKind::ALL
                    .into_iter()
                    .find(|kind| kind.key() == tag)
                    .map(Self::Pages)
            })
            .or_else(|| {
                RollTapErrorKind::ALL
                    .into_iter()
                    .find(|kind| kind.key() == tag)
                    .map(Self::Roll)
            })
            .or_else(|| {
                CastErrorKind::ALL
                    .into_iter()
                    .find(|kind| kind.key() == tag)
                    .map(Self::Cast)
            })
    }
}

/// Every refusal of the seat routes, by the tag of the host's typed error: the host names each of
/// its `SeatError`s by one of these (`SeatError::kind`), so the phone's list cannot drift from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatErrorKind {
    MissingKey,
    InvalidKey,
    /// The key holds no seat.
    UnknownKey,
    /// The hero is not one this player may play.
    UnknownHero,
    /// The seat plays no character, so it has nothing to say.
    NoPersona,
    /// The table is not asking this seat anything now.
    NothingAsked,
    /// The line has no words.
    EmptyLine,
    /// The character is played from another seat.
    PersonaTaken,
    /// No stored persona has the id.
    UnknownPersona,
    /// The table's day starts once the seat's hero is chosen and locked in.
    NoHeroToPlay,
    /// The answer is to an ask the table no longer asks: a newer one replaced it.
    StaleAsk,
    /// The answer names a thing none of the open ask's cards is about.
    UnknownThing,
    /// A line was sent before with this draft's id and other words.
    SaidChanged,
    /// The line is said to someone who is not here.
    UnknownListener,
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
    /// The table answered without applying or declining the action.
    NotApplied,
    UnknownRequest,
    UnknownSeat,
    FriendsFull,
    HeroesFull,
    /// The host could not read the request's body.
    InvalidBody,
    /// A players file could not be read or written.
    Storage,
    /// The table's engine refused.
    Engine,
    /// The room code is not this table's.
    CodeMismatch,
    /// The table cannot do this kind of action yet.
    NotReady,
    /// The clue is not on the clue board shown.
    UnknownClue,
    /// No clue is shown to the table.
    NoClueShown,
    /// No seat plays the day, so the party has nobody to ask.
    NoParty,
    /// Another seat at the table has no hero locked in.
    PartyNotReady,
    /// The shop is not one of the shops where the party is.
    NotHere,
    /// The same action (its `said` id) is still being done; its answer comes to a retry.
    InFlight,
    /// The address tried a room code again inside its wait after a wrong one.
    TooManyGuesses,
    /// A runner that is not the table's current run (one an earlier host left behind).
    NotThisRun,
}

impl SeatErrorKind {
    pub const ALL: [Self; 39] = [
        Self::MissingKey,
        Self::InvalidKey,
        Self::UnknownKey,
        Self::UnknownHero,
        Self::NoPersona,
        Self::NothingAsked,
        Self::EmptyLine,
        Self::PersonaTaken,
        Self::UnknownPersona,
        Self::NoHeroToPlay,
        Self::StaleAsk,
        Self::UnknownThing,
        Self::SaidChanged,
        Self::UnknownListener,
        Self::InvalidName,
        Self::UnknownFriend,
        Self::AccountsNotAvailable,
        Self::TableFull,
        Self::GameStarted,
        Self::PendingFull,
        Self::TableClosed,
        Self::NotApplied,
        Self::UnknownRequest,
        Self::UnknownSeat,
        Self::FriendsFull,
        Self::HeroesFull,
        Self::InvalidBody,
        Self::Storage,
        Self::Engine,
        Self::CodeMismatch,
        Self::NotReady,
        Self::UnknownClue,
        Self::NoClueShown,
        Self::NoParty,
        Self::PartyNotReady,
        Self::NotHere,
        Self::InFlight,
        Self::TooManyGuesses,
        Self::NotThisRun,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::MissingKey => "missing_key",
            Self::InvalidKey => "invalid_key",
            Self::UnknownKey => "unknown_key",
            Self::UnknownHero => "unknown_hero",
            Self::NoPersona => "no_persona",
            Self::NothingAsked => "nothing_asked",
            Self::EmptyLine => "empty_line",
            Self::PersonaTaken => "persona_taken",
            Self::UnknownPersona => "unknown_persona",
            Self::NoHeroToPlay => "no_hero_to_play",
            Self::StaleAsk => "stale_ask",
            Self::UnknownThing => "unknown_thing",
            Self::SaidChanged => "said_changed",
            Self::UnknownListener => "unknown_listener",
            Self::InvalidName => "invalid_name",
            Self::UnknownFriend => "unknown_friend",
            Self::AccountsNotAvailable => "accounts_not_available",
            Self::TableFull => "table_full",
            Self::GameStarted => "game_started",
            Self::PendingFull => "pending_full",
            Self::TableClosed => "table_closed",
            Self::NotApplied => "not_applied",
            Self::UnknownRequest => "unknown_request",
            Self::UnknownSeat => "unknown_seat",
            Self::FriendsFull => "friends_full",
            Self::HeroesFull => "heroes_full",
            Self::InvalidBody => "invalid_body",
            Self::Storage => "storage",
            Self::Engine => "engine",
            Self::CodeMismatch => "code_mismatch",
            Self::NotReady => "not_ready",
            Self::UnknownClue => "unknown_clue",
            Self::NoClueShown => "no_clue_shown",
            Self::NoParty => "no_party",
            Self::PartyNotReady => "party_not_ready",
            Self::NotHere => "not_here",
            Self::InFlight => "in_flight",
            Self::TooManyGuesses => "too_many_guesses",
            Self::NotThisRun => "not_this_run",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }

    /// What the phone tells the player; the host's own sentence stays in the logs.
    pub fn player_words(self) -> &'static str {
        match self {
            Self::MissingKey | Self::InvalidKey | Self::UnknownKey => {
                "This phone is not at the table. Join again."
            }
            Self::UnknownHero => "That hero is not one you can play.",
            Self::NoPersona => "Your hero is not in the story yet.",
            Self::NothingAsked => "The table is not waiting on you right now.",
            Self::EmptyLine => "Say something first.",
            Self::PersonaTaken => "Someone else at the table plays that character.",
            Self::UnknownPersona => "The table does not know that character.",
            Self::NoHeroToPlay => "Choose your hero and lock in their look first.",
            Self::StaleAsk => "The table has moved on to a new question. Look again.",
            Self::UnknownThing => "That is not something you can step up to now.",
            Self::SaidChanged => "That line was already sent. Send your new words again.",
            Self::UnknownListener => "They are not here to hear you. Pick someone who is.",
            Self::InvalidName => "That name will not do. Try another.",
            Self::UnknownFriend => "The host does not know that friend.",
            Self::AccountsNotAvailable => "Signing in comes later. Join as a guest for now.",
            Self::TableFull => "The table is full.",
            Self::GameStarted => "The game has started without new players.",
            Self::PendingFull => "The host has too many requests. Try again soon.",
            Self::TableClosed => "The table has closed.",
            Self::NotApplied => "The table did not take that yet. Try again.",
            Self::UnknownRequest => "That request is no longer waiting.",
            Self::UnknownSeat => "Nobody holds that seat.",
            Self::FriendsFull => "The host's friends book is full.",
            Self::HeroesFull => "The host's hero library is full.",
            Self::CodeMismatch => "That is not this table's code. Check the code on the screen.",
            Self::NotReady => "The table cannot do that yet.",
            Self::UnknownClue => "That clue is not on the board.",
            Self::NoClueShown => "No clue is on the screen.",
            Self::NoParty => "Nobody is playing the day yet.",
            Self::PartyNotReady => "Everyone at the table needs a hero locked in first.",
            Self::NotHere => "That shop isn't here.",
            Self::InFlight => "That is still going through. Try again in a moment.",
            Self::TooManyGuesses => "Too many wrong codes. Wait a moment, then try again.",
            Self::NotThisRun => "That is not the table's current game.",
            Self::InvalidBody | Self::Storage | Self::Engine => SEAT_TROUBLE_WORDS,
        }
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

/// What the phone tells the player when the host refused for a reason it does not word.
pub const SEAT_TROUBLE_WORDS: &str = "That didn't reach the table; try again.";

/// `path::PLAY_SEAT_ROLL`'s body: the check of the seat's hero sheet the player taps on the Roll
/// page, and the thing in sight it is about, when it is about one (`about`, a thing id of the
/// place). The host's check judge sets the difficulty from the scene and the host rolls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatRoll {
    pub check: SeatCheck,
    #[serde(default)]
    pub about: Option<String>,
}

/// A check of the hero sheet: an ability check or a saving throw by the sheet's short ability key
/// (`str`, as `SheetAbility::key`), or a skill check by the rules' skill key (`athletics`,
/// `animal_handling`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SeatCheck {
    Ability { ability: String },
    Skill { skill: String },
    Save { ability: String },
}

/// `path::PLAY_SEAT_ROLL`'s answer: the roll as the host made it, or why the dungeon master called
/// none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum SeatRolled {
    /// The d20 kept, what the sheet adds to it, the total, the difficulty it is set against and
    /// the verdict.
    Rolled {
        roll: i32,
        modifier: i32,
        total: i32,
        dc: Option<i32>,
        outcome: Option<RollOutcome>,
    },
    /// The check judge found the check does not fit the scene now; `reason` is its reason key.
    NotCalled { reason: String },
}

/// A roll the dungeon master called for the seat's hero, waiting for the player to roll it: the
/// table is paused on it until the seat answers (or the wait runs out and the host rolls), and the
/// host rolls the dice when it does. The phone names nothing but `roll_id` back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingRoll {
    pub roll_id: String,
    /// The test in the dungeon master's words ("Athletics, Strength").
    pub title: String,
    /// The roll's kind by its rules key (`ability_check`, `saving_throw`, `attack_roll`,
    /// `initiative`, `death_save`, `damage`, ...).
    pub kind: String,
    /// The ability by its rules key (`strength`), when the roll names one.
    #[serde(default)]
    pub ability: Option<String>,
    /// The skill by its rules key (`athletics`), when the roll names one.
    #[serde(default)]
    pub skill: Option<String>,
    #[serde(default)]
    pub mode: RollMode,
    /// The difficulty or Armor Class the total is set against, when the roll has one.
    #[serde(default)]
    pub target: Option<RollTarget>,
}

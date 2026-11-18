//! The creation routes of a seated phone (`planning/phone-and-creation.md` P3): the seat's build,
//! as the host's creation machine holds it, and what the phone sends to change it. Shared by the
//! host that answers them and the phone that reads them; the `dev` command line calls the same
//! routes as a seat (`x-df-dev-seat`).
//!
//! The phone never sends a number the rules derive: a pick is a slot and an option id, and the
//! host derives the sheet. Nothing here carries a seat key.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::wire::HeroSheetView;
use crate::dev_reply::{ErrorTag, HostReply};
use crate::seat::wire::SeatErrorKind;

/// Where the seat's build stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildState {
    /// Not a legal build yet (or not started).
    #[default]
    Drafting,
    /// The rules derived a full sheet: shown for review.
    Reviewing,
    /// Submitted: the seat is ready.
    Ready,
}

/// The hero the build is for, as the look studio started it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildHero {
    pub name: String,
    /// The species and class ids the look studio chose.
    pub species: String,
    pub class: String,
    #[serde(default)]
    pub portrait: Option<String>,
}

/// One option of a choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildOption {
    pub id: String,
    pub label: String,
    /// "Not in play yet", with why, for an option the rules cannot play yet (it may still be
    /// chosen: it is designed and marked, never played as another).
    #[serde(default)]
    pub not_in_play: Option<String>,
    /// One line the host words about the option, for a choice that needs more than its name: a
    /// spell's level, school and what it does, a weapon's mastery.
    #[serde(default)]
    pub detail: Option<String>,
}

/// One group of choices the build asks for: its slots and its options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildGroup {
    pub id: String,
    pub label: String,
    /// One line on why the group matters.
    #[serde(default)]
    pub because: Option<String>,
    pub slots: Vec<String>,
    pub options: Vec<BuildOption>,
}

/// `PLAY_CREATION`'s answer, and every creation step's.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CreationView {
    pub state: BuildState,
    /// None until the seat's build is started.
    #[serde(default)]
    pub hero: Option<BuildHero>,
    pub picks: BTreeMap<String, String>,
    pub groups: Vec<BuildGroup>,
    /// What still stops the build, in sentences.
    pub issues: Vec<String>,
    /// The derived sheet, once the build is legal.
    #[serde(default)]
    pub sheet: Option<HeroSheetView>,
}

/// One pick: a slot and the option id for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildPick {
    pub slot: String,
    pub option: String,
}

/// `PLAY_CREATION_CHOOSE`'s body: picks to set and slots to clear, in order.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildChoose {
    #[serde(default)]
    pub picks: Vec<BuildPick>,
    #[serde(default)]
    pub clear: Vec<String>,
}

/// The body of the surprise, submit and reopen routes: nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildStep {}

/// Every refusal of the creation routes, by the tag of the host's typed error. A seat refusal
/// reads as the seat routes' own kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreationErrorKind {
    InvalidBody,
    NoHero,
    SeatReady,
    BuildIncomplete,
    NotReady,
    Started,
    TooManyPicks,
    LobbyDeclined,
    Storage,
    Seat(SeatErrorKind),
}

impl CreationErrorKind {
    /// The creation routes' own kinds (a seat refusal is `SeatErrorKind`'s).
    pub const ALL: [Self; 9] = [
        Self::InvalidBody,
        Self::NoHero,
        Self::SeatReady,
        Self::BuildIncomplete,
        Self::NotReady,
        Self::Started,
        Self::TooManyPicks,
        Self::LobbyDeclined,
        Self::Storage,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::InvalidBody => "invalid_body",
            Self::NoHero => "no_hero",
            Self::SeatReady => "seat_ready",
            Self::BuildIncomplete => "build_incomplete",
            Self::NotReady => "not_ready",
            Self::Started => "started",
            Self::TooManyPicks => "too_many_picks",
            Self::LobbyDeclined => "lobby_declined",
            Self::Storage => "storage",
            Self::Seat(kind) => kind.key(),
        }
    }

    /// The player's words for the refusal.
    pub fn words(self) -> &'static str {
        match self {
            Self::InvalidBody | Self::TooManyPicks => "That change could not be read. Try again.",
            Self::NoHero => "Make your hero's look first.",
            Self::SeatReady => "You are ready. Choose Not ready to change your hero.",
            Self::BuildIncomplete => "Your hero is not finished yet.",
            Self::NotReady => "You are not ready yet.",
            Self::Started => "The game has started.",
            Self::LobbyDeclined => "The table did not take that just now. Try again.",
            Self::Storage => "The table could not keep your hero just now. Try again.",
            Self::Seat(_) => "The table did not take that. Try again.",
        }
    }
}

impl ErrorTag for CreationErrorKind {
    fn from_tag(tag: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.key() == tag)
            .or_else(|| SeatErrorKind::from_key(tag).map(Self::Seat))
    }
}

/// A creation route's answer as the phone reads it.
pub type CreationReply<T> = HostReply<T, CreationErrorKind>;

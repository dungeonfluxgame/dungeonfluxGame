//! The lobby's own live changes (`lobby_change` holds the rest): the title lines, a seat, the join
//! code, the party summary, the begin button and the parts of the lobby that can be taken away,
//! with how each is applied to the lobby and what its audit line carries.
//!
//! This is the part of `lobby_change` that belongs to the player lobby alone; `ScreenOverlay::apply`
//! calls it. A change is checked at the edge: a percent is 0 to 100, a step is one of the three,
//! the players who joined are not more than the seats there are, the focus names the begin button
//! or a seat in range, and the lists are held to very large named limits.

use serde::{Deserialize, Serialize};

use super::{MAXIMUM_LOBBY_SEATS, ScreenError, ScreenOverlay, drawn, percent, plain, present, put};
use crate::fixture::TvScene;
use crate::home_model::is_focus_id;
use crate::lobby_model::{
    BEGIN_FOCUS_ID, CHARACTER_STEPS, JoinCode, LobbyScreen, LobbySeat, PartySummary, SeatState,
    SummaryIcon, seat_key,
};
use crate::overlay::Glyph;
use crate::scene::SceneError;

/// Which line of the lobby's title block a text change is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LobbyLine {
    Eyebrow,
    Title,
    Subtitle,
}

/// A change to one lobby seat. A field left out stays as it is; a text left empty is taken away.
/// Setting the state to open or closed first empties the seat.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SeatEdit {
    pub state: Option<SeatState>,
    pub name: Option<String>,
    pub class: Option<String>,
    pub glyph: Option<Glyph>,
    pub level: Option<u8>,
    pub portrait: Option<String>,
    pub host: Option<bool>,
    pub progress_percent: Option<u8>,
    pub step: Option<u8>,
    pub stage: Option<String>,
    pub badge: Option<String>,
    pub controller: Option<bool>,
}

/// A change to the join panel: the game code and the words around it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JoinEdit {
    pub hint: Option<String>,
    pub join_link: Option<String>,
    pub game_code: Option<String>,
}

/// A change to the party summary: who is at the table and what they are doing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PartyEdit {
    pub joined: Option<u8>,
    pub capacity: Option<u8>,
    pub word: Option<String>,
    pub detail: Option<String>,
    pub icon: Option<SummaryIcon>,
}

/// The parts of the lobby that can be taken away, by the name the dev CLI uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LobbyPart {
    Title,
    Seats,
    Join,
    Party,
    Begin,
}

impl LobbyPart {
    /// Every part, in the order the lobby draws them.
    pub const ALL: [Self; 5] = [
        Self::Title,
        Self::Seats,
        Self::Join,
        Self::Party,
        Self::Begin,
    ];

    /// The name the dev CLI and the audit line use.
    pub fn key(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Seats => "seats",
            Self::Join => "join",
            Self::Party => "party",
            Self::Begin => "begin",
        }
    }

    /// The part of that name.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|part| part.key() == key)
    }

    /// The names of every part, comma separated.
    pub fn keys_text() -> String {
        Self::ALL.map(Self::key).join(", ")
    }
}

/// A change to the begin button.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BeginEdit {
    pub enabled: Option<bool>,
    pub label: Option<String>,
    pub note: Option<String>,
}

pub(super) fn seat_index(number: usize) -> Result<usize, ScreenError> {
    if (1..=MAXIMUM_LOBBY_SEATS).contains(&number) {
        Ok(number - 1)
    } else {
        Err(ScreenError::BadSeatNumber { number })
    }
}

pub(super) fn empty_seat(state: SeatState) -> LobbySeat {
    LobbySeat {
        state,
        name: None,
        class: None,
        glyph: None,
        level: None,
        portrait: None,
        host: false,
        progress_percent: None,
        step: None,
        stage: None,
        badge: None,
        controller: false,
    }
}

impl SeatEdit {
    pub(super) fn apply(self, seat: &mut LobbySeat) -> Result<(), SceneError> {
        if let Some(state) = self.state {
            if matches!(state, SeatState::Open | SeatState::Closed) {
                *seat = empty_seat(state);
            }
            seat.state = state;
        }
        put(&mut seat.name, self.name, "a seat's name")?;
        put(&mut seat.class, self.class, "a class")?;
        put(&mut seat.portrait, self.portrait, "a media name")?;
        put(&mut seat.stage, self.stage, "a seat's stage")?;
        put(&mut seat.badge, self.badge, "a seat's badge")?;
        if let Some(glyph) = self.glyph {
            seat.glyph = Some(glyph);
        }
        if let Some(level) = self.level {
            seat.level = Some(level);
        }
        if let Some(host) = self.host {
            seat.host = host;
        }
        if let Some(controller) = self.controller {
            seat.controller = controller;
        }
        if let Some(progress) = self.progress_percent {
            seat.progress_percent = Some(percent("a seat's progress", progress)?);
        }
        if let Some(step) = self.step {
            if usize::from(step) >= CHARACTER_STEPS.len() {
                return Err(ScreenError::BadStep(step).into());
            }
            seat.step = Some(step);
        }
        Ok(())
    }
}

pub(super) fn lobby_of(overlay: &ScreenOverlay, base: &TvScene) -> LobbyScreen {
    drawn(&overlay.lobby, &base.lobby).unwrap_or_else(|| LobbyScreen {
        eyebrow: None,
        title: String::new(),
        subtitle: None,
        seats: Vec::new(),
        join: None,
        party: None,
        begin: None,
        focus: None,
    })
}

impl JoinEdit {
    pub(super) fn apply(self, join: &mut JoinCode) -> Result<(), SceneError> {
        put(&mut join.hint, self.hint, "the join hint")?;
        put(&mut join.join_link, self.join_link, "a join link")?;
        put(&mut join.game_code, self.game_code, "a game code")?;
        Ok(())
    }
}

impl PartyEdit {
    pub(super) fn apply(self, party: &mut PartySummary) -> Result<(), SceneError> {
        put(&mut party.word, self.word, "the count's word")?;
        put(&mut party.detail, self.detail, "the count's detail")?;
        if let Some(joined) = self.joined {
            party.joined = joined;
        }
        if let Some(capacity) = self.capacity {
            party.capacity = capacity;
        }
        if let Some(icon) = self.icon {
            party.icon = icon;
        }
        if party.joined > party.capacity {
            return Err(ScreenError::JoinedOverCapacity {
                joined: party.joined,
                capacity: party.capacity,
            }
            .into());
        }
        Ok(())
    }
}

/// Whether `id` names an element the lobby can focus: the begin button or a seat in range.
pub(super) fn focus_id_ok(id: &str) -> bool {
    if !is_focus_id(id) {
        return false;
    }
    if id == BEGIN_FOCUS_ID {
        return true;
    }
    id.strip_prefix("seat:")
        .and_then(|number| number.parse::<usize>().ok())
        .is_some_and(|number| (1..=MAXIMUM_LOBBY_SEATS).contains(&number))
}

pub(super) fn seat_pairs(number: usize, seat: &SeatEdit) -> Vec<(&'static str, String)> {
    let mut pairs = vec![("seat", number.to_string())];
    if let Some(state) = seat.state {
        pairs.push(("state", seat_key(state).to_owned()));
    }
    present(&mut pairs, "name", &seat.name);
    present(&mut pairs, "class", &seat.class);
    if let Some(media) = &seat.portrait {
        pairs.push(("portrait", plain(media)));
    }
    if let Some(host) = seat.host {
        pairs.push(("host", host.to_string()));
    }
    if let Some(progress) = seat.progress_percent {
        pairs.push(("progress", progress.to_string()));
    }
    if let Some(step) = seat.step {
        pairs.push(("step", step.to_string()));
    }
    present(&mut pairs, "stage", &seat.stage);
    present(&mut pairs, "badge", &seat.badge);
    if let Some(controller) = seat.controller {
        pairs.push(("controller", controller.to_string()));
    }
    pairs
}

pub(super) fn join_pairs(join: &JoinEdit) -> Vec<(&'static str, String)> {
    let mut pairs = Vec::new();
    present(&mut pairs, "link", &join.join_link);
    present(&mut pairs, "code", &join.game_code);
    present(&mut pairs, "hint", &join.hint);
    pairs
}

pub(super) fn party_pairs(party: &PartyEdit) -> Vec<(&'static str, String)> {
    let mut pairs = Vec::new();
    if let Some(joined) = party.joined {
        pairs.push(("joined", joined.to_string()));
    }
    if let Some(capacity) = party.capacity {
        pairs.push(("capacity", capacity.to_string()));
    }
    present(&mut pairs, "word", &party.word);
    present(&mut pairs, "detail", &party.detail);
    present(&mut pairs, "icon", &party.icon);
    pairs
}

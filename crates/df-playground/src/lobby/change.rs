//! Live changes to the lobby, the loading screen, the roleplay card and the top bar: what the dev
//! CLI's `scene lobby`, `scene loading`, `scene roleplay` and `scene pill` verbs send, and what
//! each one does to the scene.
//!
//! A change is a `ScreenEdit`. It is applied to a copy of the overlay (`scene_apply`), so a
//! refused change leaves everything as it was, and the overlay holds the resulting component as a
//! whole in its `lobby`, `loading`, `roleplay` or `pills` slot (an `Edit` drawn over the fixture's
//! own), and `scene reset` drops it. A text left empty (`name=`) takes that field away. Every
//! range is checked here, at the edge: a percent is 0 to 100, a step is one of the three, the
//! players who joined are not more than the seats there are, and the lists are held to very
//! large named limits that only keep memory bounded. The audit line of a change carries the
//! component and the field, plus numbers, state keys and lengths, never free text.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::audit_fields::present;
use crate::checks;
use crate::fixture::{Fixture, TvScene};
use crate::limits::{
    MAXIMUM_CAPTION_CHARACTERS, MAXIMUM_CHARACTER_MARKERS, MAXIMUM_LOBBY_SEATS,
    MAXIMUM_NAME_CHARACTERS, MAXIMUM_PERCENT, MAXIMUM_SPEECH_BUBBLES,
};
use crate::lobby::model::{
    BEGIN_FOCUS_ID, BeginButton, CHARACTER_STEPS, LoadingScreen, LogoStyle, NarratorChip,
    PartyCount, PartySummary, ReadyChip, SceneTitle, SeatState, SummaryIcon,
};
use crate::overlay::{Edit, Glyph};
use crate::scene::{SceneError, SceneOverlay};

mod roleplay;

pub use self::roleplay::RoleplayEdit;
use self::roleplay::{new_bubble, roleplay_pairs};
mod player_lobby;

pub use self::player_lobby::{BeginEdit, JoinEdit, LobbyLine, LobbyPart, PartyEdit, SeatEdit};
use self::player_lobby::{
    empty_seat, focus_id_ok, join_pairs, lobby_of, party_pairs, seat_index, seat_pairs,
};

/// The picture of a roleplay title when the change gives none.
pub const DEFAULT_TITLE_GLYPH: Glyph = Glyph::Book;
/// Where a bubble made by a change is put until the change says: the box's left edge, its top,
/// its width, and the point its tail aims at, all in percent of the screen.
pub const DEFAULT_BUBBLE: (u8, u8, u8, u8, u8) = (20, 12, 24, 30, 40);
/// The seats a lobby made by a change has until it says otherwise.
pub const DEFAULT_CAPACITY: u8 = 6;

/// Why a change to a screen component was refused, beyond the shared range checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenError {
    /// A share of the screen or of a bar is outside 0 to 100.
    InvalidPercent { what: &'static str, value: u8 },
    /// A seat number is 0 or past the most seats.
    InvalidSeatNumber { number: usize },
    /// A bubble number is 0 or past the most bubbles.
    InvalidBubbleNumber { number: usize },
    /// A marker number is 0, past the markers there are plus one, or past the most markers.
    InvalidMarkerNumber { number: usize },
    /// A cue names no valid character, or a spoken line is empty.
    InvalidCue,
    /// A new marker needs both its character and its text.
    NewMarkerNeeds,
    /// A bubble's anchor point or shift was changed on a bubble with no anchor.
    NoBubbleAnchor,
    /// A character step is not one of the three.
    InvalidStep(u8),
    /// More players joined than there are seats.
    JoinedOverCapacity { joined: u8, capacity: u8 },
    /// A focus id that is neither `begin` nor `seat:<n>` for a seat number in range.
    InvalidFocus(String),
}

impl fmt::Display for ScreenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPercent { what, value } => {
                write!(
                    formatter,
                    "{what} is a percent from 0 to {MAXIMUM_PERCENT}, not {value}"
                )
            }
            Self::InvalidSeatNumber { number } => write!(
                formatter,
                "a seat is numbered from 1 to {MAXIMUM_LOBBY_SEATS}, not {number}"
            ),
            Self::InvalidBubbleNumber { number } => write!(
                formatter,
                "a bubble is numbered from 1 to {MAXIMUM_SPEECH_BUBBLES}, not {number}"
            ),
            Self::InvalidMarkerNumber { number } => write!(
                formatter,
                "marker {number} is not on the card: a marker is numbered from 1 to one past the last, at most {}",
                MAXIMUM_CHARACTER_MARKERS
            ),
            Self::InvalidCue => formatter.write_str(
                "a cue names a character (letters, digits, - or _) and a spoken line has words",
            ),
            Self::NewMarkerNeeds => formatter.write_str(
                "a new marker needs its character and its text (mark-at=<region> mark-text=<text>)",
            ),
            Self::NoBubbleAnchor => formatter
                .write_str("the bubble has no anchor yet: give it one with anchor=<region> first"),
            Self::InvalidStep(step) => write!(
                formatter,
                "a character step is 0 to {} ({}), not {step}",
                CHARACTER_STEPS.len() - 1,
                CHARACTER_STEPS.join(", ")
            ),
            Self::JoinedOverCapacity { joined, capacity } => write!(
                formatter,
                "{joined} players cannot have joined a table of {capacity} seats"
            ),
            Self::InvalidFocus(id) => write!(
                formatter,
                "the lobby's focus is `{BEGIN_FOCUS_ID}` or `seat:<n>` for a seat from 1 to {MAXIMUM_LOBBY_SEATS}, not `{id}`"
            ),
        }
    }
}

impl std::error::Error for ScreenError {}

impl From<ScreenError> for SceneError {
    fn from(error: ScreenError) -> Self {
        Self::Screen(error)
    }
}

/// A change to the loading screen's own words, its bar and its narrator.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoadingEdit {
    pub kicker: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub label: Option<String>,
    pub percent: Option<u8>,
    pub narrator_name: Option<String>,
    pub narrator_status: Option<String>,
    pub narrator_portrait: Option<String>,
}

/// A change to one seat chip of the loading screen.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ChipEdit {
    pub name: Option<String>,
    pub portrait: Option<String>,
    pub ready: Option<bool>,
}

/// A change to the top bar.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PillEdit {
    pub logo: Option<LogoStyle>,
    pub session: Option<String>,
    pub party: Option<PartyCount>,
    pub balance: Option<String>,
    pub rate: Option<String>,
    pub profile: Option<bool>,
}

/// A part of the top bar that can be taken away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PillPart {
    Logo,
    Session,
    Party,
    Balance,
    Rate,
    Profile,
}

impl PillPart {
    pub const ALL: [Self; 6] = [
        Self::Logo,
        Self::Session,
        Self::Party,
        Self::Balance,
        Self::Rate,
        Self::Profile,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Logo => "logo",
            Self::Session => "session",
            Self::Party => "party",
            Self::Balance => "balance",
            Self::Rate => "rate",
            Self::Profile => "profile",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|part| part.key() == key)
    }
}

/// One change to a screen component, as the dev CLI sends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScreenEdit {
    LobbyLine {
        field: LobbyLine,
        text: String,
    },
    /// Seats are numbered from 1; a seat past the end adds open seats up to it.
    LobbySeat {
        number: usize,
        seat: SeatEdit,
    },
    /// Makes the table have exactly this many seats: seats past the end are added open, the ones
    /// beyond are taken away.
    LobbySeats {
        count: usize,
    },
    LobbyJoin {
        join: JoinEdit,
    },
    LobbyParty {
        party: PartyEdit,
    },
    LobbyBegin {
        begin: BeginEdit,
    },
    /// Puts the focus on one element by its id (`begin`, `seat:2`); `None` returns it to the
    /// begin button.
    LobbyFocus {
        id: Option<String>,
    },
    /// Takes one part of the lobby away.
    LobbyRemove {
        part: LobbyPart,
    },
    LobbyClear {},
    LoadingSet {
        loading: LoadingEdit,
    },
    LoadingSeat {
        number: usize,
        chip: ChipEdit,
    },
    LoadingClear {},
    PillSet {
        pill: PillEdit,
    },
    PillClear {
        part: PillPart,
    },
    RoleplaySet {
        roleplay: RoleplayEdit,
    },
    RoleplayClear {},
    /// A chat system's cue about one character by name: thinking, speaking a line, or quiet.
    /// The host checks the name against the shot's layout before it sends the edit.
    Cue {
        cue: crate::image_layout::CharacterCue,
    },
}

fn put(
    slot: &mut Option<String>,
    value: Option<String>,
    what: &'static str,
) -> Result<(), SceneError> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.trim().is_empty() {
        *slot = None;
        return Ok(());
    }
    checks::labelled(what, &value)?;
    *slot = Some(value);
    Ok(())
}

fn put_text(
    slot: &mut String,
    value: Option<String>,
    what: &'static str,
) -> Result<(), SceneError> {
    if let Some(value) = value {
        checks::text(what, &value, MAXIMUM_NAME_CHARACTERS)?;
        *slot = value;
    }
    Ok(())
}

/// `value` when it is a percent from 0 to `MAXIMUM_PERCENT`, the one range check of a screen
/// percent for live edits and scene files alike.
pub(crate) fn percent(what: &'static str, value: u8) -> Result<u8, ScreenError> {
    if value > MAXIMUM_PERCENT {
        Err(ScreenError::InvalidPercent { what, value })
    } else {
        Ok(value)
    }
}

fn loading_of(overlay: &SceneOverlay, base: &TvScene) -> LoadingScreen {
    overlay
        .loading
        .drawn(base.loading.as_ref())
        .unwrap_or_else(|| LoadingScreen {
            kicker: None,
            title: String::new(),
            subtitle: None,
            narrator: None,
            label: None,
            percent: 0,
            seats: Vec::new(),
        })
}

impl SceneOverlay {
    /// Makes one change to a screen component, starting from what is drawn now (the overlay's, or
    /// the fixture's). The caller holds a copy of the overlay, so a refusal changes nothing.
    pub(crate) fn apply_screen(
        &mut self,
        edit: ScreenEdit,
        base: &Fixture,
    ) -> Result<(), SceneError> {
        let tv = &base.tv;
        match edit {
            ScreenEdit::LobbyLine { field, text } => {
                let mut lobby = lobby_of(self, tv);
                checks::text("a lobby line", &text, MAXIMUM_CAPTION_CHARACTERS)?;
                match field {
                    LobbyLine::Title => lobby.title = text,
                    LobbyLine::Subtitle => lobby.subtitle = Some(text),
                    LobbyLine::Eyebrow => lobby.eyebrow = Some(text),
                }
                self.lobby = Edit::Set(lobby);
            }
            ScreenEdit::LobbySeat { number, seat } => {
                let index = seat_index(number)?;
                let mut lobby = lobby_of(self, tv);
                while lobby.seats.len() <= index {
                    lobby.seats.push(empty_seat(SeatState::Open));
                }
                seat.apply(&mut lobby.seats[index])?;
                self.lobby = Edit::Set(lobby);
            }
            ScreenEdit::LobbySeats { count } => {
                if count > MAXIMUM_LOBBY_SEATS {
                    return Err(ScreenError::InvalidSeatNumber { number: count }.into());
                }
                let mut lobby = lobby_of(self, tv);
                lobby.seats.truncate(count);
                while lobby.seats.len() < count {
                    lobby.seats.push(empty_seat(SeatState::Open));
                }
                self.lobby = Edit::Set(lobby);
            }
            ScreenEdit::LobbyJoin { join } => {
                let mut lobby = lobby_of(self, tv);
                let mut current = lobby.join.take().unwrap_or_default();
                join.apply(&mut current)?;
                lobby.join = Some(current);
                self.lobby = Edit::Set(lobby);
            }
            ScreenEdit::LobbyParty { party } => {
                let mut lobby = lobby_of(self, tv);
                let capacity = u8::try_from(lobby.seats.len()).unwrap_or(DEFAULT_CAPACITY);
                let mut current = lobby.party.take().unwrap_or(PartySummary {
                    joined: 0,
                    capacity: if capacity == 0 {
                        DEFAULT_CAPACITY
                    } else {
                        capacity
                    },
                    word: None,
                    detail: None,
                    icon: SummaryIcon::default(),
                });
                party.apply(&mut current)?;
                lobby.party = Some(current);
                self.lobby = Edit::Set(lobby);
            }
            ScreenEdit::LobbyBegin { begin } => {
                let mut lobby = lobby_of(self, tv);
                let mut current = lobby.begin.take().unwrap_or(BeginButton {
                    label: None,
                    enabled: false,
                    note: None,
                });
                put(&mut current.label, begin.label, "the begin label")?;
                put(&mut current.note, begin.note, "the begin note")?;
                if let Some(enabled) = begin.enabled {
                    current.enabled = enabled;
                }
                lobby.begin = Some(current);
                self.lobby = Edit::Set(lobby);
            }
            ScreenEdit::LobbyFocus { id } => {
                if let Some(id) = &id
                    && !focus_id_ok(id)
                {
                    return Err(ScreenError::InvalidFocus(id.clone()).into());
                }
                let mut lobby = lobby_of(self, tv);
                lobby.focus = id;
                self.lobby = Edit::Set(lobby);
            }
            ScreenEdit::LobbyRemove { part } => {
                let mut lobby = lobby_of(self, tv);
                match part {
                    LobbyPart::Title => {
                        lobby.eyebrow = None;
                        lobby.title = String::new();
                        lobby.subtitle = None;
                    }
                    LobbyPart::Seats => lobby.seats.clear(),
                    LobbyPart::Join => lobby.join = None,
                    LobbyPart::Party => lobby.party = None,
                    LobbyPart::Begin => lobby.begin = None,
                }
                self.lobby = Edit::Set(lobby);
            }
            ScreenEdit::LobbyClear {} => self.lobby = Edit::Clear,
            ScreenEdit::LoadingSet { loading } => self.set_loading(loading, tv)?,
            ScreenEdit::LoadingSeat { number, chip } => {
                let index = seat_index(number)?;
                let mut screen = loading_of(self, tv);
                while screen.seats.len() <= index {
                    screen.seats.push(ReadyChip {
                        name: format!("Seat {}", screen.seats.len() + 1),
                        portrait: None,
                        ready: false,
                    });
                }
                let current = &mut screen.seats[index];
                put_text(&mut current.name, chip.name, "a seat's name")?;
                put(&mut current.portrait, chip.portrait, "a media name")?;
                if let Some(ready) = chip.ready {
                    current.ready = ready;
                }
                self.loading = Edit::Set(screen);
            }
            ScreenEdit::LoadingClear {} => self.loading = Edit::Clear,
            ScreenEdit::PillSet { pill } => self.set_pills(pill, tv)?,
            ScreenEdit::PillClear { part } => {
                let mut bar = self.pills.drawn(tv.pills.as_ref()).unwrap_or_default();
                match part {
                    PillPart::Logo => bar.logo = None,
                    PillPart::Session => bar.session = None,
                    PillPart::Party => bar.party = None,
                    PillPart::Balance => bar.balance = None,
                    PillPart::Rate => bar.rate = None,
                    PillPart::Profile => bar.profile = false,
                }
                self.pills = Edit::Set(bar);
            }
            ScreenEdit::RoleplaySet { roleplay } => self.set_roleplay(roleplay, tv)?,
            ScreenEdit::RoleplayClear {} => self.roleplay = Edit::Clear,
            ScreenEdit::Cue { cue } => {
                let mut card = self
                    .roleplay
                    .drawn(tv.roleplay.as_ref())
                    .unwrap_or_default();
                roleplay::apply_cue(&mut card, &cue)?;
                self.roleplay = Edit::Set(card);
            }
        }
        Ok(())
    }

    fn set_loading(&mut self, edit: LoadingEdit, tv: &TvScene) -> Result<(), SceneError> {
        let mut screen = loading_of(self, tv);
        put(&mut screen.kicker, edit.kicker, "the kicker")?;
        put_text(&mut screen.title, edit.title, "the loading title")?;
        put(&mut screen.subtitle, edit.subtitle, "the loading words")?;
        put(&mut screen.label, edit.label, "the loading label")?;
        if let Some(value) = edit.percent {
            screen.percent = percent("the loading bar", value)?;
        }
        if edit.narrator_name.is_some()
            || edit.narrator_status.is_some()
            || edit.narrator_portrait.is_some()
        {
            let mut narrator = screen.narrator.take().unwrap_or(NarratorChip {
                name: String::new(),
                status: None,
                portrait: None,
            });
            put_text(
                &mut narrator.name,
                edit.narrator_name,
                "the narrator's name",
            )?;
            put(
                &mut narrator.status,
                edit.narrator_status,
                "the narrator's status",
            )?;
            put(
                &mut narrator.portrait,
                edit.narrator_portrait,
                "a media name",
            )?;
            screen.narrator = Some(narrator);
        }
        self.loading = Edit::Set(screen);
        Ok(())
    }

    fn set_pills(&mut self, edit: PillEdit, tv: &TvScene) -> Result<(), SceneError> {
        let mut bar = self.pills.drawn(tv.pills.as_ref()).unwrap_or_default();
        put(&mut bar.session, edit.session, "the session's name")?;
        put(&mut bar.balance, edit.balance, "the balance")?;
        put(&mut bar.rate, edit.rate, "the rate")?;
        if let Some(logo) = edit.logo {
            bar.logo = Some(logo);
        }
        if let Some(profile) = edit.profile {
            bar.profile = profile;
        }
        if let Some(count) = edit.party {
            if count.joined > count.capacity {
                return Err(ScreenError::JoinedOverCapacity {
                    joined: count.joined,
                    capacity: count.capacity,
                }
                .into());
            }
            bar.party = Some(count);
        }
        self.pills = Edit::Set(bar);
        Ok(())
    }

    fn set_roleplay(&mut self, edit: RoleplayEdit, tv: &TvScene) -> Result<(), SceneError> {
        let mut card = self
            .roleplay
            .drawn(tv.roleplay.as_ref())
            .unwrap_or_default();
        let wants_bubble =
            edit.speaker.is_some() || edit.line.is_some() || edit.has_bubble_fields();
        if edit.title.is_some() || edit.icon.is_some() {
            let mut title = card.title.take().unwrap_or(SceneTitle {
                glyph: DEFAULT_TITLE_GLYPH,
                text: String::new(),
            });
            put_text(&mut title.text, edit.title.clone(), "a scene's title")?;
            if let Some(icon) = edit.icon {
                title.glyph = icon;
            }
            card.title = Some(title);
        }
        put(&mut card.caption, edit.caption.clone(), "a scene's caption")?;
        if wants_bubble {
            let number = edit.bubble.unwrap_or(1);
            if !(1..=MAXIMUM_SPEECH_BUBBLES).contains(&number) {
                return Err(ScreenError::InvalidBubbleNumber { number }.into());
            }
            while card.bubbles.len() < number {
                card.bubbles.push(new_bubble());
            }
            edit.apply_to_bubble(&mut card.bubbles[number - 1])?;
        }
        edit.apply_to_markers(&mut card.markers)?;
        self.roleplay = Edit::Set(card);
        Ok(())
    }
}

fn plain(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_whitespace() || character == '=' || character == '"' {
                '_'
            } else {
                character
            }
        })
        .collect()
}

impl ScreenEdit {
    /// The component and field of the change, for the audit line: `component=lobby field=seat`,
    /// then numbers, state keys, flags, media names and the length of each text, never the text.
    pub fn audit_fields(&self) -> (&'static str, Vec<(&'static str, String)>) {
        let (component, field, mut pairs): (&str, &str, Vec<(&'static str, String)>) = match self {
            Self::LobbyLine { field, text } => (
                "lobby",
                field.key(),
                vec![("chars", text.chars().count().to_string())],
            ),
            Self::LobbySeat { number, seat } => ("lobby", "seat", seat_pairs(*number, seat)),
            Self::LobbySeats { count } => ("lobby", "seats", vec![("count", count.to_string())]),
            Self::LobbyJoin { join } => ("lobby", "join", join_pairs(join)),
            Self::LobbyParty { party } => ("lobby", "party", party_pairs(party)),
            Self::LobbyFocus { id } => (
                "lobby",
                "focus",
                vec![("id", id.clone().unwrap_or_else(|| "default".to_owned()))],
            ),
            Self::LobbyRemove { part } => {
                ("lobby", "remove", vec![("part", part.key().to_owned())])
            }
            Self::LobbyBegin { begin } => {
                let mut pairs = Vec::new();
                if let Some(enabled) = begin.enabled {
                    pairs.push(("enabled", enabled.to_string()));
                }
                present(&mut pairs, "label", begin.label.as_ref());
                present(&mut pairs, "note", begin.note.as_ref());
                ("lobby", "begin", pairs)
            }
            Self::LobbyClear {} => ("lobby", "clear", Vec::new()),
            Self::LoadingSet { loading } => ("loading", "set", loading_pairs(loading)),
            Self::LoadingSeat { number, chip } => {
                let mut pairs = vec![("seat", number.to_string())];
                present(&mut pairs, "name", chip.name.as_ref());
                if let Some(media) = &chip.portrait {
                    pairs.push(("portrait", plain(media)));
                }
                if let Some(ready) = chip.ready {
                    pairs.push(("ready", ready.to_string()));
                }
                ("loading", "seat", pairs)
            }
            Self::LoadingClear {} => ("loading", "clear", Vec::new()),
            Self::PillSet { pill } => ("pill", "set", pill_pairs(pill)),
            Self::PillClear { part } => ("pill", "clear", vec![("part", part.key().to_owned())]),
            Self::RoleplaySet { roleplay } => ("roleplay", "set", roleplay_pairs(roleplay)),
            Self::RoleplayClear {} => ("roleplay", "clear", Vec::new()),
            Self::Cue { cue } => ("roleplay", "cue", roleplay::cue_pairs(cue)),
        };
        pairs.insert(0, ("field", field.to_owned()));
        pairs.insert(0, ("component", component.to_owned()));
        ("screen", pairs)
    }

    /// The media names the change shows, so the host can check each is a picture it has.
    pub fn media_names(&self) -> Vec<&str> {
        let names = match self {
            Self::LobbySeat { seat, .. } => vec![seat.portrait.as_deref()],
            Self::LoadingSeat { chip, .. } => vec![chip.portrait.as_deref()],
            Self::LoadingSet { loading } => vec![loading.narrator_portrait.as_deref()],
            Self::RoleplaySet { roleplay } => vec![roleplay.portrait.as_deref()],
            _ => Vec::new(),
        };
        names
            .into_iter()
            .flatten()
            .filter(|name| !name.trim().is_empty())
            .collect()
    }
}

fn loading_pairs(loading: &LoadingEdit) -> Vec<(&'static str, String)> {
    let mut pairs = Vec::new();
    present(&mut pairs, "kicker", loading.kicker.as_ref());
    present(&mut pairs, "title", loading.title.as_ref());
    present(&mut pairs, "subtitle", loading.subtitle.as_ref());
    present(&mut pairs, "label", loading.label.as_ref());
    if let Some(percent) = loading.percent {
        pairs.push(("percent", percent.to_string()));
    }
    present(&mut pairs, "narrator", loading.narrator_name.as_ref());
    present(
        &mut pairs,
        "narrator_status",
        loading.narrator_status.as_ref(),
    );
    if let Some(media) = &loading.narrator_portrait {
        pairs.push(("narrator_portrait", plain(media)));
    }
    pairs
}

fn pill_pairs(pill: &PillEdit) -> Vec<(&'static str, String)> {
    let mut pairs = Vec::new();
    if let Some(logo) = pill.logo {
        pairs.push(("logo", logo.key().to_owned()));
    }
    present(&mut pairs, "session", pill.session.as_ref());
    if let Some(count) = pill.party {
        pairs.push(("party", format!("{}/{}", count.joined, count.capacity)));
    }
    present(&mut pairs, "balance", pill.balance.as_ref());
    present(&mut pairs, "rate", pill.rate.as_ref());
    if let Some(profile) = pill.profile {
        pairs.push(("profile", profile.to_string()));
    }
    pairs
}

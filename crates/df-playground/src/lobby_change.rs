//! Live changes to the lobby, the loading screen, the roleplay card and the top bar: what the dev
//! CLI's `scene lobby`, `scene loading`, `scene roleplay` and `scene pill` verbs send, and what
//! each one does to the scene.
//!
//! A change is a `ScreenEdit`. It is applied to a copy of the overlay (`scene_apply`), so a
//! refused change leaves everything as it was, and the overlay holds the resulting component as a
//! whole: `ScreenOverlay` is drawn over the fixture's own lobby, loading screen, roleplay card or
//! top bar, and `scene reset` drops it. A text left empty (`name=`) takes that field away. Every
//! range is checked here, at the edge: a percent is 0 to 100, a step is one of the three, the
//! players who joined are not more than the seats there are, and the lists are held to very
//! large named limits that only keep memory bounded. The audit line of a change carries the
//! component and the field, plus numbers, state keys and lengths, never free text.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::checks;
use crate::fixture::{Fixture, TvScene};
use crate::limits::{MAXIMUM_CAPTION_CHARACTERS, MAXIMUM_NAME_CHARACTERS};
use crate::lobby_model::{
    BeginButton, Bubble, BubbleTone, CHARACTER_STEPS, Invite, LoadingScreen, LobbyScreen,
    LobbySeat, LogoStyle, NarratorChip, PartyCount, ReadyChip, RoleplayScene, SceneTitle,
    SeatState, SummaryIcon, TopBar, seat_key,
};
use crate::overlay::Glyph;
use crate::scene::{Override, SceneError};

/// Most seats a lobby or chips a loading screen holds: a very large number that only bounds memory.
pub const MAXIMUM_LOBBY_SEATS: usize = 1_000;
/// Most speech bubbles on a roleplay card.
pub const MAXIMUM_BUBBLES: usize = 1_000;
/// The picture of a roleplay title when the change gives none.
pub const DEFAULT_TITLE_GLYPH: Glyph = Glyph::Book;
/// Where a bubble made by a change is put until the change says: the box's left edge, its top,
/// its width, and the point its tail aims at, all in percent of the screen.
pub const DEFAULT_BUBBLE: (u8, u8, u8, u8, u8) = (20, 12, 24, 30, 40);
/// The seats a lobby made by a change has until it says otherwise.
pub const DEFAULT_CAPACITY: u8 = 6;
/// The highest a percent is.
pub const MAXIMUM_PERCENT: u8 = 100;

/// Why a change to a screen component was refused, beyond the shared range checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenError {
    /// A share of the screen or of a bar is outside 0 to 100.
    BadPercent { what: &'static str, value: u8 },
    /// A seat number is 0 or past the most seats.
    BadSeatNumber { number: usize },
    /// A bubble number is 0 or past the most bubbles.
    BadBubbleNumber { number: usize },
    /// A character step is not one of the three.
    BadStep(u8),
    /// More players joined than there are seats.
    JoinedOverCapacity { joined: u8, capacity: u8 },
}

impl fmt::Display for ScreenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadPercent { what, value } => {
                write!(
                    formatter,
                    "{what} is a percent from 0 to {MAXIMUM_PERCENT}, not {value}"
                )
            }
            Self::BadSeatNumber { number } => write!(
                formatter,
                "a seat is numbered from 1 to {MAXIMUM_LOBBY_SEATS}, not {number}"
            ),
            Self::BadBubbleNumber { number } => write!(
                formatter,
                "a bubble is numbered from 1 to {MAXIMUM_BUBBLES}, not {number}"
            ),
            Self::BadStep(step) => write!(
                formatter,
                "a character step is 0 to {} ({}), not {step}",
                CHARACTER_STEPS.len() - 1,
                CHARACTER_STEPS.join(", ")
            ),
            Self::JoinedOverCapacity { joined, capacity } => write!(
                formatter,
                "{joined} players cannot have joined a table of {capacity} seats"
            ),
        }
    }
}

impl From<ScreenError> for SceneError {
    fn from(error: ScreenError) -> Self {
        Self::Screen(error)
    }
}

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

/// A change to the invite panel.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InviteEdit {
    pub heading: Option<String>,
    pub hint: Option<String>,
    pub join_link: Option<String>,
    pub game_code: Option<String>,
    pub joined: Option<u8>,
    pub capacity: Option<u8>,
    pub word: Option<String>,
    pub detail: Option<String>,
    pub icon: Option<SummaryIcon>,
}

/// A change to the begin button.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BeginEdit {
    pub enabled: Option<bool>,
    pub label: Option<String>,
    pub note: Option<String>,
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
        [
            Self::Logo,
            Self::Session,
            Self::Party,
            Self::Balance,
            Self::Rate,
            Self::Profile,
        ]
        .into_iter()
        .find(|part| part.key() == key)
    }
}

/// A change to the roleplay card: its title bar, one bubble (number 1 when none is named) and the
/// caption.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RoleplayEdit {
    pub title: Option<String>,
    pub icon: Option<Glyph>,
    pub bubble: Option<usize>,
    pub speaker: Option<String>,
    pub line: Option<String>,
    pub emphasis: Option<String>,
    pub speaker_icon: Option<Glyph>,
    pub portrait: Option<String>,
    pub tone: Option<BubbleTone>,
    pub left_percent: Option<u8>,
    pub top_percent: Option<u8>,
    pub width_percent: Option<u8>,
    pub tail_x_percent: Option<u8>,
    pub tail_y_percent: Option<u8>,
    pub caption: Option<String>,
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
    LobbyInvite {
        invite: InviteEdit,
    },
    LobbyBegin {
        begin: BeginEdit,
    },
    LobbyLegend {
        shown: bool,
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
}

/// The screen components the overlay draws over the fixture's own.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenOverlay {
    pub lobby: Option<Override<LobbyScreen>>,
    pub loading: Option<Override<LoadingScreen>>,
    pub roleplay: Option<Override<RoleplayScene>>,
    pub pills: Option<Override<TopBar>>,
}

fn drawn<T: Clone>(change: &Option<Override<T>>, fixture: &Option<T>) -> Option<T> {
    match change {
        Some(Override::Set(value)) => Some(value.clone()),
        Some(Override::Clear) => None,
        None => fixture.clone(),
    }
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

fn percent(what: &'static str, value: u8) -> Result<u8, ScreenError> {
    if value > MAXIMUM_PERCENT {
        Err(ScreenError::BadPercent { what, value })
    } else {
        Ok(value)
    }
}

fn seat_index(number: usize) -> Result<usize, ScreenError> {
    if (1..=MAXIMUM_LOBBY_SEATS).contains(&number) {
        Ok(number - 1)
    } else {
        Err(ScreenError::BadSeatNumber { number })
    }
}

fn empty_seat(state: SeatState) -> LobbySeat {
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
    fn apply(self, seat: &mut LobbySeat) -> Result<(), SceneError> {
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

fn lobby_of(overlay: &ScreenOverlay, base: &TvScene) -> LobbyScreen {
    drawn(&overlay.lobby, &base.lobby).unwrap_or_else(|| LobbyScreen {
        eyebrow: None,
        title: String::new(),
        subtitle: None,
        seats: Vec::new(),
        invite: None,
        begin: None,
        legend: false,
    })
}

fn loading_of(overlay: &ScreenOverlay, base: &TvScene) -> LoadingScreen {
    drawn(&overlay.loading, &base.loading).unwrap_or_else(|| LoadingScreen {
        kicker: None,
        title: String::new(),
        subtitle: None,
        narrator: None,
        label: None,
        percent: 0,
        seats: Vec::new(),
    })
}

impl InviteEdit {
    fn apply(self, invite: &mut Invite) -> Result<(), SceneError> {
        put(&mut invite.heading, self.heading, "the invite heading")?;
        put(&mut invite.hint, self.hint, "the invite hint")?;
        put(&mut invite.join_link, self.join_link, "a join link")?;
        put(&mut invite.game_code, self.game_code, "a game code")?;
        put(&mut invite.word, self.word, "the count's word")?;
        put(&mut invite.detail, self.detail, "the count's detail")?;
        if let Some(joined) = self.joined {
            invite.joined = joined;
        }
        if let Some(capacity) = self.capacity {
            invite.capacity = capacity;
        }
        if let Some(icon) = self.icon {
            invite.icon = icon;
        }
        if invite.joined > invite.capacity {
            return Err(ScreenError::JoinedOverCapacity {
                joined: invite.joined,
                capacity: invite.capacity,
            }
            .into());
        }
        Ok(())
    }
}

impl ScreenOverlay {
    /// Draws the overlay onto the shared screen of a copy of the fixture.
    pub fn apply_to(&self, tv: &mut TvScene) {
        tv.lobby = drawn(&self.lobby, &tv.lobby);
        tv.loading = drawn(&self.loading, &tv.loading);
        tv.roleplay = drawn(&self.roleplay, &tv.roleplay);
        tv.pills = drawn(&self.pills, &tv.pills);
    }

    /// Makes one change to a component, starting from what is drawn now (the overlay's, or the
    /// fixture's). The caller holds a copy of the overlay, so a refusal changes nothing.
    pub fn apply(&mut self, edit: ScreenEdit, base: &Fixture) -> Result<(), SceneError> {
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
                self.lobby = Some(Override::Set(lobby));
            }
            ScreenEdit::LobbySeat { number, seat } => {
                let index = seat_index(number)?;
                let mut lobby = lobby_of(self, tv);
                while lobby.seats.len() <= index {
                    lobby.seats.push(empty_seat(SeatState::Open));
                }
                seat.apply(&mut lobby.seats[index])?;
                self.lobby = Some(Override::Set(lobby));
            }
            ScreenEdit::LobbyInvite { invite } => {
                let mut lobby = lobby_of(self, tv);
                let capacity = u8::try_from(lobby.seats.len()).unwrap_or(DEFAULT_CAPACITY);
                let mut current = lobby.invite.take().unwrap_or(Invite {
                    heading: None,
                    hint: None,
                    join_link: None,
                    game_code: None,
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
                invite.apply(&mut current)?;
                lobby.invite = Some(current);
                self.lobby = Some(Override::Set(lobby));
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
                self.lobby = Some(Override::Set(lobby));
            }
            ScreenEdit::LobbyLegend { shown } => {
                let mut lobby = lobby_of(self, tv);
                lobby.legend = shown;
                self.lobby = Some(Override::Set(lobby));
            }
            ScreenEdit::LobbyClear {} => self.lobby = Some(Override::Clear),
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
                self.loading = Some(Override::Set(screen));
            }
            ScreenEdit::LoadingClear {} => self.loading = Some(Override::Clear),
            ScreenEdit::PillSet { pill } => self.set_pills(pill, tv)?,
            ScreenEdit::PillClear { part } => {
                let mut bar = drawn(&self.pills, &tv.pills).unwrap_or_default();
                match part {
                    PillPart::Logo => bar.logo = None,
                    PillPart::Session => bar.session = None,
                    PillPart::Party => bar.party = None,
                    PillPart::Balance => bar.balance = None,
                    PillPart::Rate => bar.rate = None,
                    PillPart::Profile => bar.profile = false,
                }
                self.pills = Some(Override::Set(bar));
            }
            ScreenEdit::RoleplaySet { roleplay } => self.set_roleplay(roleplay, tv)?,
            ScreenEdit::RoleplayClear {} => self.roleplay = Some(Override::Clear),
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
        self.loading = Some(Override::Set(screen));
        Ok(())
    }

    fn set_pills(&mut self, edit: PillEdit, tv: &TvScene) -> Result<(), SceneError> {
        let mut bar = drawn(&self.pills, &tv.pills).unwrap_or_default();
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
        self.pills = Some(Override::Set(bar));
        Ok(())
    }

    fn set_roleplay(&mut self, edit: RoleplayEdit, tv: &TvScene) -> Result<(), SceneError> {
        let mut card = drawn(&self.roleplay, &tv.roleplay).unwrap_or_default();
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
            if !(1..=MAXIMUM_BUBBLES).contains(&number) {
                return Err(ScreenError::BadBubbleNumber { number }.into());
            }
            while card.bubbles.len() < number {
                card.bubbles.push(new_bubble());
            }
            edit.apply_to_bubble(&mut card.bubbles[number - 1])?;
        }
        self.roleplay = Some(Override::Set(card));
        Ok(())
    }
}

fn new_bubble() -> Bubble {
    let (left, top, width, tail_x, tail_y) = DEFAULT_BUBBLE;
    Bubble {
        speaker: String::new(),
        line: String::new(),
        emphasis: None,
        glyph: None,
        portrait: None,
        tone: BubbleTone::default(),
        left_percent: left,
        top_percent: top,
        width_percent: width,
        tail_x_percent: tail_x,
        tail_y_percent: tail_y,
    }
}

impl RoleplayEdit {
    fn has_bubble_fields(&self) -> bool {
        self.emphasis.is_some()
            || self.speaker_icon.is_some()
            || self.portrait.is_some()
            || self.tone.is_some()
            || self.left_percent.is_some()
            || self.top_percent.is_some()
            || self.width_percent.is_some()
            || self.tail_x_percent.is_some()
            || self.tail_y_percent.is_some()
    }

    fn apply_to_bubble(&self, bubble: &mut Bubble) -> Result<(), SceneError> {
        put_text(&mut bubble.speaker, self.speaker.clone(), "a speaker")?;
        if let Some(line) = self.line.clone() {
            checks::text("a bubble's line", &line, MAXIMUM_CAPTION_CHARACTERS)?;
            bubble.line = line;
        }
        put(
            &mut bubble.emphasis,
            self.emphasis.clone(),
            "a bubble's emphasis",
        )?;
        put(&mut bubble.portrait, self.portrait.clone(), "a media name")?;
        if let Some(icon) = self.speaker_icon {
            bubble.glyph = Some(icon);
        }
        if let Some(tone) = self.tone {
            bubble.tone = tone;
        }
        for (what, value, slot) in [
            (
                "a bubble's left edge",
                self.left_percent,
                &mut bubble.left_percent,
            ),
            ("a bubble's top", self.top_percent, &mut bubble.top_percent),
            (
                "a bubble's width",
                self.width_percent,
                &mut bubble.width_percent,
            ),
            (
                "a bubble's tail x",
                self.tail_x_percent,
                &mut bubble.tail_x_percent,
            ),
            (
                "a bubble's tail y",
                self.tail_y_percent,
                &mut bubble.tail_y_percent,
            ),
        ] {
            if let Some(value) = value {
                *slot = percent(what, value)?;
            }
        }
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

fn present<T>(pairs: &mut Vec<(&'static str, String)>, key: &'static str, value: &Option<T>) {
    if value.is_some() {
        pairs.push((key, "set".to_owned()));
    }
}

impl ScreenEdit {
    /// The component and field of the change, for the audit line: `component=lobby field=seat`,
    /// then numbers, state keys, flags, media names and the length of each text, never the text.
    pub fn audit_fields(&self) -> (&'static str, Vec<(&'static str, String)>) {
        let (component, field, mut pairs): (&str, &str, Vec<(&'static str, String)>) = match self {
            Self::LobbyLine { field, text } => {
                let key = match field {
                    LobbyLine::Eyebrow => "eyebrow",
                    LobbyLine::Title => "title",
                    LobbyLine::Subtitle => "subtitle",
                };
                (
                    "lobby",
                    key,
                    vec![("chars", text.chars().count().to_string())],
                )
            }
            Self::LobbySeat { number, seat } => ("lobby", "seat", seat_pairs(*number, seat)),
            Self::LobbyInvite { invite } => ("lobby", "invite", invite_pairs(invite)),
            Self::LobbyBegin { begin } => {
                let mut pairs = Vec::new();
                if let Some(enabled) = begin.enabled {
                    pairs.push(("enabled", enabled.to_string()));
                }
                present(&mut pairs, "label", &begin.label);
                present(&mut pairs, "note", &begin.note);
                ("lobby", "begin", pairs)
            }
            Self::LobbyLegend { shown } => ("lobby", "legend", vec![("shown", shown.to_string())]),
            Self::LobbyClear {} => ("lobby", "clear", Vec::new()),
            Self::LoadingSet { loading } => ("loading", "set", loading_pairs(loading)),
            Self::LoadingSeat { number, chip } => {
                let mut pairs = vec![("seat", number.to_string())];
                present(&mut pairs, "name", &chip.name);
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

fn seat_pairs(number: usize, seat: &SeatEdit) -> Vec<(&'static str, String)> {
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

fn invite_pairs(invite: &InviteEdit) -> Vec<(&'static str, String)> {
    let mut pairs = Vec::new();
    present(&mut pairs, "link", &invite.join_link);
    present(&mut pairs, "code", &invite.game_code);
    if let Some(joined) = invite.joined {
        pairs.push(("joined", joined.to_string()));
    }
    if let Some(capacity) = invite.capacity {
        pairs.push(("capacity", capacity.to_string()));
    }
    present(&mut pairs, "heading", &invite.heading);
    present(&mut pairs, "hint", &invite.hint);
    present(&mut pairs, "word", &invite.word);
    present(&mut pairs, "detail", &invite.detail);
    present(&mut pairs, "icon", &invite.icon);
    pairs
}

fn loading_pairs(loading: &LoadingEdit) -> Vec<(&'static str, String)> {
    let mut pairs = Vec::new();
    present(&mut pairs, "kicker", &loading.kicker);
    present(&mut pairs, "title", &loading.title);
    present(&mut pairs, "subtitle", &loading.subtitle);
    present(&mut pairs, "label", &loading.label);
    if let Some(percent) = loading.percent {
        pairs.push(("percent", percent.to_string()));
    }
    present(&mut pairs, "narrator", &loading.narrator_name);
    present(&mut pairs, "narrator_status", &loading.narrator_status);
    if let Some(media) = &loading.narrator_portrait {
        pairs.push(("narrator_portrait", plain(media)));
    }
    pairs
}

fn pill_pairs(pill: &PillEdit) -> Vec<(&'static str, String)> {
    let mut pairs = Vec::new();
    if let Some(logo) = pill.logo {
        let key = match logo {
            LogoStyle::Stacked => "stacked",
            LogoStyle::Inline => "inline",
        };
        pairs.push(("logo", key.to_owned()));
    }
    present(&mut pairs, "session", &pill.session);
    if let Some(count) = pill.party {
        pairs.push(("party", format!("{}/{}", count.joined, count.capacity)));
    }
    present(&mut pairs, "balance", &pill.balance);
    present(&mut pairs, "rate", &pill.rate);
    if let Some(profile) = pill.profile {
        pairs.push(("profile", profile.to_string()));
    }
    pairs
}

fn roleplay_pairs(edit: &RoleplayEdit) -> Vec<(&'static str, String)> {
    let mut pairs = Vec::new();
    present(&mut pairs, "title", &edit.title);
    present(&mut pairs, "icon", &edit.icon);
    if let Some(bubble) = edit.bubble {
        pairs.push(("bubble", bubble.to_string()));
    }
    present(&mut pairs, "speaker", &edit.speaker);
    present(&mut pairs, "line", &edit.line);
    present(&mut pairs, "emphasis", &edit.emphasis);
    present(&mut pairs, "caption", &edit.caption);
    if let Some(media) = &edit.portrait {
        pairs.push(("portrait", plain(media)));
    }
    for (key, value) in [
        ("left", edit.left_percent),
        ("top", edit.top_percent),
        ("width", edit.width_percent),
        ("tail_x", edit.tail_x_percent),
        ("tail_y", edit.tail_y_percent),
    ] {
        if let Some(value) = value {
            pairs.push((key, value.to_string()));
        }
    }
    pairs
}

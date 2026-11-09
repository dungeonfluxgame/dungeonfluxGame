//! The data of the lobby, the loading screen, the roleplay card and the top-bar pills of the
//! shared screen, and the plain decisions that turn that data into what the page draws.
//!
//! These are presentation values only. Every field is optional in a scene file (`TvScene` holds
//! them with a default), so a scene that names none of them reads as before. Nothing here decides
//! a rule: whether a seat is locked in, how many players joined, what the table pays and whether
//! the table may begin is what the server projected, and the page draws it. A balance or a rate is
//! drawn only when the scene carries one: the public pricing shows what tables pay, so the page
//! never works a figure out. `lobby_checks` says which values are out of range and the views
//! (`view::lobby`, `view::pills`, `view::roleplay`) only turn the answers into elements.

use serde::{Deserialize, Serialize};

use crate::overlay::Glyph;

/// The three steps of making a character, in the order the seat card's dots show them.
pub const CHARACTER_STEPS: [&str; 3] = ["Class", "Race", "Finalize"];
/// The word on a locked-in seat's seal when the scene does not give one.
pub const LOCKED_IN_WORD: &str = "Ready";
/// What a seat that is making a character says when the scene does not say.
pub const CREATING_WORD: &str = "Creating character";
/// The word under an open seat's name.
pub const OPEN_WORD: &str = "Open";
/// The word under a closed seat's name.
pub const CLOSED_WORD: &str = "Closed";
/// What the join panel says over the game code when the scene does not say.
pub const JOIN_HINT: &str = "Game code";
/// The most seats the lobby draws: a television density rule (fewer, bigger things), not a limit
/// on the data. The seats after these stay in the data.
pub const SEATS_SHOWN: usize = 6;
/// The most seats a lobby draws at the wide card size; one more draws all at the narrow size.
pub const WIDE_SEATS: usize = 5;
/// The longest title, in characters, drawn at the full size; longer ones step down.
pub const TITLE_FULL_CHARACTERS: usize = 26;
/// The longest title, in characters, drawn at the middle size.
pub const TITLE_MIDDLE_CHARACTERS: usize = 40;
/// The id of the begin button for the remote's focus.
pub const BEGIN_FOCUS_ID: &str = "begin";
/// The word after the count of players when the scene does not say.
pub const JOINED_WORD: &str = "Joined";
/// What the begin button says when the scene does not say.
pub const BEGIN_LABEL: &str = "Begin adventure";
/// The line over a lobby's title.
pub const LOBBY_EYEBROW: &str = "Session lobby";
/// What the loading screen's progress is called when the scene does not say.
pub const LOADING_LABEL: &str = "Initializing adventure";
/// The word on a ready chip of the loading screen.
pub const READY_WORD: &str = "Ready";
/// The side of the square of cells the QR-like placeholder block draws.
pub const QR_CELLS: usize = 21;
/// The side of a finder square of that block.
pub const QR_FINDER: usize = 7;
/// How far a speech bubble's tail keeps from the corners of its box, in percent of the width.
pub const TAIL_INSET_PERCENT: u8 = 12;

/// What a lobby seat is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatState {
    /// The player's hero is locked in.
    LockedIn,
    /// The player is making a character.
    Creating,
    /// Nobody is in the seat.
    Open,
    /// The table has closed the seat.
    Closed,
}

impl SeatState {
    pub const ALL: [Self; 4] = [Self::LockedIn, Self::Creating, Self::Open, Self::Closed];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::LockedIn => "locked_in",
            Self::Creating => "creating",
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }
}

/// One seat of the lobby.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LobbySeat {
    pub state: SeatState,
    /// The player's or hero's name; an open seat is called "Seat N" when this is left out.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub class: Option<String>,
    /// The small picture beside the class.
    #[serde(default)]
    pub glyph: Option<Glyph>,
    #[serde(default)]
    pub level: Option<u8>,
    /// A picture in the media library.
    #[serde(default)]
    pub portrait: Option<String>,
    /// Whether this seat's player started the table; it draws a crown.
    #[serde(default)]
    pub host: bool,
    /// How far a player making a character has got, in percent.
    #[serde(default)]
    pub progress_percent: Option<u8>,
    /// Which step of `CHARACTER_STEPS` the player is on, counted from 0.
    #[serde(default)]
    pub step: Option<u8>,
    /// What a player making a character is doing, such as "Selecting character".
    #[serde(default)]
    pub stage: Option<String>,
    /// The word on the seat's badge, when it is not the state's own.
    #[serde(default)]
    pub badge: Option<String>,
    /// Whether the player is on a controller, which draws its mark.
    #[serde(default)]
    pub controller: bool,
}

/// The picture beside the count of players.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryIcon {
    #[default]
    Check,
    People,
    Spinner,
}

impl SummaryIcon {
    pub const ALL: [Self; 3] = [Self::Check, Self::People, Self::Spinner];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::People => "people",
            Self::Spinner => "spinner",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }
}

/// How a phone joins: the game code, the link it belongs to and the words over the code.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JoinCode {
    /// The words over the code; "Game code" when left out.
    pub hint: Option<String>,
    /// The address of the join page. It is data for the phone's side: the television draws the
    /// code and its QR picture only.
    pub join_link: Option<String>,
    pub game_code: Option<String>,
}

/// How many players are at the table and what they are doing: "4 of 6 Ready".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartySummary {
    pub joined: u8,
    pub capacity: u8,
    /// The word after the count, such as "Joined", "Ready" or "Locked in".
    #[serde(default)]
    pub word: Option<String>,
    /// A second line under the count, such as "2 still choosing".
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub icon: SummaryIcon,
}

/// The button that starts the adventure. Whether it may be pressed is the server's word.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeginButton {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    /// What a button that may not be pressed says, such as "Waiting for 2 players".
    #[serde(default)]
    pub note: Option<String>,
}

/// The player lobby: the session's name, the seats, how a phone joins, how many are ready and the
/// begin button. Each is its own component and is drawn only when the scene has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LobbyScreen {
    #[serde(default)]
    pub eyebrow: Option<String>,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub seats: Vec<LobbySeat>,
    #[serde(default)]
    pub join: Option<JoinCode>,
    #[serde(default)]
    pub party: Option<PartySummary>,
    #[serde(default)]
    pub begin: Option<BeginButton>,
    /// The one element the remote is on: `begin` or `seat:<n>` (counted from 1). Without one the
    /// focus is on the begin button.
    #[serde(default)]
    pub focus: Option<String>,
}

/// The focus id of the seat numbered `number`, counted from 1.
pub fn seat_focus_id(number: usize) -> String {
    format!("seat:{number}")
}

impl LobbyScreen {
    /// The id of the element that has focus: the scene's own, or else the begin button.
    pub fn focus_id(&self) -> Option<String> {
        match &self.focus {
            Some(id) => Some(id.clone()),
            None => self.begin.as_ref().map(|_| BEGIN_FOCUS_ID.to_owned()),
        }
    }

    /// The seats the lobby draws: the first `SEATS_SHOWN`.
    pub fn shown_seats(&self) -> &[LobbySeat] {
        &self.seats[..self.seats.len().min(SEATS_SHOWN)]
    }
}

/// How the title is sized: the full size for a short name and a step down for a long one, so a
/// long name stays on one or two lines above the seats.
pub fn title_size_key(title: &str) -> &'static str {
    match title.trim().chars().count() {
        0..=TITLE_FULL_CHARACTERS => "full",
        count if count <= TITLE_MIDDLE_CHARACTERS => "middle",
        _ => "long",
    }
}

/// How wide the seat cards are: the wide size for up to `WIDE_SEATS` seats, the narrow for more.
pub fn seat_size_key(drawn: usize) -> &'static str {
    if drawn <= WIDE_SEATS {
        "wide"
    } else {
        "narrow"
    }
}

/// The narrator's chip of the loading screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarratorChip {
    pub name: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub portrait: Option<String>,
}

/// One seat of the loading screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadyChip {
    pub name: String,
    #[serde(default)]
    pub portrait: Option<String>,
    #[serde(default)]
    pub ready: bool,
}

/// The loading screen: where the party is going, the narrator, a bar and the seats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadingScreen {
    #[serde(default)]
    pub kicker: Option<String>,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub narrator: Option<NarratorChip>,
    #[serde(default)]
    pub label: Option<String>,
    pub percent: u8,
    #[serde(default)]
    pub seats: Vec<ReadyChip>,
}

/// Who a speech bubble belongs to; it sets the colour of the bubble's rim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BubbleTone {
    Hero,
    #[default]
    Ally,
    Mystic,
    Npc,
    Narrator,
    Danger,
    /// A thinking indicator: a rounded box of pulsing dots with a trail of dots for a tail. It
    /// draws no words; the speaker names it for a screen reader and the line is not shown.
    Thought,
}

impl BubbleTone {
    pub const ALL: [Self; 7] = [
        Self::Hero,
        Self::Ally,
        Self::Mystic,
        Self::Npc,
        Self::Narrator,
        Self::Danger,
        Self::Thought,
    ];

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tone| tone.key() == key)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Hero => "hero",
            Self::Ally => "ally",
            Self::Mystic => "mystic",
            Self::Npc => "npc",
            Self::Narrator => "narrator",
            Self::Danger => "danger",
            Self::Thought => "thought",
        }
    }
}

/// The title bar of a roleplay card: a small picture and the scene's name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneTitle {
    pub glyph: Glyph,
    pub text: String,
}

/// One person's line, in a box whose tail points at them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bubble {
    pub speaker: String,
    pub line: String,
    /// A last line in italics, such as the narrator's warning.
    #[serde(default)]
    pub emphasis: Option<String>,
    #[serde(default)]
    pub glyph: Option<Glyph>,
    /// A small picture of the speaker, drawn instead of the glyph.
    #[serde(default)]
    pub portrait: Option<String>,
    #[serde(default)]
    pub tone: BubbleTone,
    /// The box's left edge, in percent of the screen's width.
    pub left_percent: u8,
    /// The box's top edge, in percent of the screen's height.
    pub top_percent: u8,
    /// The box's width, in percent of the screen's width.
    pub width_percent: u8,
    /// Where the tail points, in percent of the screen's width and height.
    pub tail_x_percent: u8,
    pub tail_y_percent: u8,
    /// A region of the backdrop's scanned layout to sit over; when it resolves, it places the box
    /// and the tail instead of the five numbers above, which stay as the fallback.
    #[serde(default)]
    pub anchor: Option<crate::image_layout::BubbleAnchor>,
}

/// The roleplay card: a title bar, the bubbles of whoever is speaking, and a caption.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RoleplayScene {
    pub title: Option<SceneTitle>,
    pub bubbles: Vec<Bubble>,
    pub caption: Option<String>,
    /// The name of the item a close-up shows, which titles the caption when the picture on screen
    /// is a close-up (`closeup::scene_closeup`); a caption on any other picture ignores it.
    pub caption_title: Option<String>,
    /// Small items tied to characters of the backdrop's layout (name tags, status marks), placed
    /// by the character anchor API.
    pub markers: Vec<crate::image_layout::CharacterMarker>,
}

/// Which wordmark the top bar draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogoStyle {
    /// The compass star above and below the name, as on the lobby and the loading screen.
    Stacked,
    /// The star and the name on one line, as on the narrator and the roleplay screens.
    Inline,
}

impl LogoStyle {
    pub const ALL: [Self; 2] = [Self::Stacked, Self::Inline];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Stacked => "stacked",
            Self::Inline => "inline",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }
}

/// How many players are in the party, drawn "Party: 4 / 6".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartyCount {
    pub joined: u8,
    pub capacity: u8,
}

/// The top bar: the wordmark at the left and the pills at the right. Every part is optional and
/// is drawn only when present; the balance and the rate are text the server worded.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TopBar {
    pub logo: Option<LogoStyle>,
    /// The session's name, drawn "Session: The Ember Roads".
    pub session: Option<String>,
    pub party: Option<PartyCount>,
    /// What the table has to spend, such as "$28.40".
    pub balance: Option<String>,
    /// What the table is paying now, such as "$3.90/hr".
    pub rate: Option<String>,
    /// Whether the round profile mark is drawn.
    pub profile: bool,
}

/// What a lobby seat's card shows, decided from its data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatView {
    pub key: &'static str,
    pub name: String,
    /// The line under the name: the class, the stage or the seat's word.
    pub sub_line: Option<String>,
    pub badge: Option<String>,
    pub progress_percent: Option<u8>,
    pub step: Option<usize>,
}

/// The short word of a seat's state, used for its class and its audit line (`locked`, where the
/// serde key `SeatState::key` is `locked_in`).
pub fn seat_marker(state: SeatState) -> &'static str {
    match state {
        SeatState::LockedIn => "locked",
        SeatState::Creating => "creating",
        SeatState::Open => "open",
        SeatState::Closed => "closed",
    }
}

fn clean(text: Option<&str>) -> Option<&str> {
    text.map(|text| text.trim()).filter(|text| !text.is_empty())
}

/// "Paladin, level 5", "Paladin" or "Level 5"; `None` when there is neither.
pub fn class_line(class: Option<&str>, level: Option<u8>) -> Option<String> {
    let class = class.map(str::trim).filter(|class| !class.is_empty());
    match (class, level) {
        (Some(class), Some(level)) => Some(format!("{class} \u{2022} Lv {level}")),
        (Some(class), None) => Some(class.to_owned()),
        (None, Some(level)) => Some(format!("Level {level}")),
        (None, None) => None,
    }
}

/// What the card of the seat at `index` (counted from 0) shows.
pub fn seat_view(seat: &LobbySeat, index: usize) -> SeatView {
    let name =
        clean(seat.name.as_deref()).map_or_else(|| format!("Seat {}", index + 1), str::to_owned);
    let sub_line = match seat.state {
        SeatState::Open => Some(OPEN_WORD.to_owned()),
        SeatState::Closed => Some(CLOSED_WORD.to_owned()),
        SeatState::Creating => Some(
            clean(seat.stage.as_deref()).map_or_else(|| CREATING_WORD.to_owned(), str::to_owned),
        ),
        SeatState::LockedIn => class_line(seat.class.as_deref(), seat.level),
    };
    let badge = match seat.state {
        SeatState::LockedIn => Some(
            clean(seat.badge.as_deref()).map_or_else(|| LOCKED_IN_WORD.to_owned(), str::to_owned),
        ),
        SeatState::Creating => clean(seat.badge.as_deref()).map(str::to_owned),
        SeatState::Open | SeatState::Closed => None,
    };
    SeatView {
        key: seat_marker(seat.state),
        name,
        sub_line,
        badge,
        progress_percent: match seat.state {
            SeatState::Creating => seat.progress_percent.map(|percent| percent.min(100)),
            _ => None,
        },
        step: match seat.state {
            SeatState::Creating => seat
                .step
                .map(|step| usize::from(step).min(CHARACTER_STEPS.len() - 1)),
            _ => None,
        },
    }
}

/// "4 of 6", the count the party summary states.
pub fn count_text(joined: u8, capacity: u8) -> String {
    format!("{joined} of {capacity}")
}

/// The word after the count.
pub fn summary_word(party: &PartySummary) -> &str {
    clean(party.word.as_deref()).unwrap_or(JOINED_WORD)
}

/// The text of the party pill, "Party: 4 / 6".
pub fn party_text(count: PartyCount) -> String {
    format!("Party: {} / {}", count.joined, count.capacity)
}

/// The text of the session pill, "Session: The Ember Roads".
pub fn session_text(name: &str) -> String {
    format!("Session: {}", name.trim())
}

/// A placeholder for the join panel's QR picture: a square of cells that look like one (a finder
/// square in three corners and a pattern between them) and are the same every time for the same
/// code. It is NOT a code anyone can scan: the real one comes from the server. Row by row, `true`
/// is dark.
pub fn qr_like_cells(code: &str) -> Vec<bool> {
    let size = QR_CELLS;
    let mut state = code.bytes().fold(0x9E37_79B9_u32, |state, byte| {
        state.rotate_left(5) ^ u32::from(byte).wrapping_mul(0x0100_0193)
    }) | 1;
    let far = size - QR_FINDER;
    let mut cells = Vec::with_capacity(size * size);
    for row in 0..size {
        for column in 0..size {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let inside = |from_row: usize, from_column: usize| {
                row >= from_row
                    && row < from_row + QR_FINDER
                    && column >= from_column
                    && column < from_column + QR_FINDER
            };
            let dark = if inside(0, 0) {
                finder_cell(row, column)
            } else if inside(0, far) {
                finder_cell(row, column - far)
            } else if inside(far, 0) {
                finder_cell(row - far, column)
            } else if row == QR_FINDER - 1 || column == QR_FINDER - 1 {
                // the quiet line beside a finder square
                false
            } else if row == QR_FINDER + 1 && column < far {
                column % 2 == 0
            } else if column == QR_FINDER + 1 && row < far {
                row % 2 == 0
            } else {
                state % 5 < 2
            };
            cells.push(dark);
        }
    }
    cells
}

/// A cell of a finder square: a dark ring, a light ring and a dark centre.
fn finder_cell(row: usize, column: usize) -> bool {
    let edge = QR_FINDER - 1;
    let outer = row == 0 || row == edge || column == 0 || column == edge;
    let centre = (2..=edge - 2).contains(&row) && (2..=edge - 2).contains(&column);
    outer || centre
}

/// Where a bubble's tail sits on its box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tail {
    /// Whether the tail hangs below the box (the speaker is lower) or rises above it.
    pub below: bool,
    /// Where the tail's root sits along the box's width, in percent of the box.
    pub root_percent: u8,
    /// Where the tail's tip sits along the box's width, in percent of the box.
    pub tip_percent: u8,
}

/// Which side of its box a bubble's tail is on, and where along that side it sits, so that the
/// tail points at (`tail_x_percent`, `tail_y_percent`). The root is held away from the corners.
pub fn tail_of(bubble: &Bubble) -> Tail {
    let width = i16::from(bubble.width_percent).max(1);
    let along = (i16::from(bubble.tail_x_percent) - i16::from(bubble.left_percent)) * 100 / width;
    let inset = i16::from(TAIL_INSET_PERCENT);
    let root = along.clamp(inset, 100 - inset);
    let tip = along.clamp(0, 100);
    Tail {
        below: i16::from(bubble.tail_y_percent) >= i16::from(bubble.top_percent),
        root_percent: u8::try_from(root).unwrap_or(TAIL_INSET_PERCENT),
        tip_percent: u8::try_from(tip).unwrap_or(TAIL_INSET_PERCENT),
    }
}

/// The loading bar's fill, held at the full bar.
pub fn bar_percent(percent: u8) -> u8 {
    percent.min(100)
}

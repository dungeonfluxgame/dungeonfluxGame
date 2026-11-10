//! The data a playground scene is drawn from, read from a JSON fixture file.
//!
//! It owns the shape of a fixture and its strict reading: a misspelt key is an error with the
//! position of the mistake, so an author sees it on the page instead of a blank panel. It never
//! draws anything and never decides what a status or a choice means: `copy` and `choices` do.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::overlay::{NarrationBox, SeatStrip};
use crate::scene::Track;

/// Where the stage gets its elements from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ElementSource {
    /// The elements listed in the fixture.
    Inline(Vec<StageElement>),
    /// The elements on the running canvas, as the `df-table` host serves them.
    Live(LiveMarker),
}

/// The only text accepted where a fixture asks for the live canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveMarker {
    Live,
}

impl LiveMarker {
    pub const ALL: [Self; 1] = [Self::Live];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Live => "live",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementShape {
    Circle,
    Rect,
}

impl ElementShape {
    pub const ALL: [Self; 2] = [Self::Circle, Self::Rect];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Circle => "circle",
            Self::Rect => "rect",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }
}

/// One thing drawn on the stage. The host's `/canvas.json` elements have this shape (plus fields
/// the stage ignores), so a live element and an inline one are drawn the same way.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageElement {
    pub id: String,
    pub kind: String,
    pub label: String,
    /// Centre of the element, in canvas units (x from 0 to 100, y from 0 to 56.25).
    pub x: f32,
    pub y: f32,
    pub color: Option<String>,
    pub shape: ElementShape,
    pub width_units: f32,
    pub height_units: f32,
    /// The element's fields, as the host serves them. A `portrait` field naming a picture in the
    /// media library draws that picture in the figure.
    #[serde(default)]
    pub fields: serde_json::Map<String, serde_json::Value>,
    /// Clockwise turn in degrees, 0 to 359 (180 shows it upside down). Defaults to 0.
    #[serde(default)]
    pub rotation_degrees: u16,
    /// The point the figure turns about. Defaults to its centre.
    #[serde(default)]
    pub pivot: StagePivot,
}

/// The point a figure turns about, in whole percent of its own width and height from its top left
/// corner: 50 and 50 is the centre, 0 and 50 a hinge on the left edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StagePivot {
    pub x_percent: u8,
    pub y_percent: u8,
}

impl Default for StagePivot {
    fn default() -> Self {
        Self {
            x_percent: 50,
            y_percent: 50,
        }
    }
}

impl StageElement {
    /// The media file of the element's portrait, when it has one.
    pub fn portrait(&self) -> Option<&str> {
        self.fields
            .get("portrait")
            .and_then(serde_json::Value::as_str)
            .filter(|name| !name.is_empty())
    }
}

/// What the table is waiting for or showing. Each value has fixed plain-language copy (`copy`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Status {
    Ok,
    Reading,
    Rolling,
    Telling,
    ArtDelayed,
    Degraded,
    ReadOnly,
    Reconnecting,
    WaitingFor { who: String, what: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Caption {
    pub speaker: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartyMember {
    pub name: String,
    pub hit_points: u32,
    pub maximum_hit_points: u32,
    #[serde(default)]
    pub conditions: Vec<String>,
    pub color: Option<String>,
}

/// What the shared screen shows.
/// A picture that slides sideways across the shared screen: it starts at `from_percent` of the
/// screen's width and ends at `to_percent`, taking `duration_ms`, at an even pace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slide {
    pub from_percent: u8,
    pub to_percent: u8,
    pub duration_ms: u64,
    /// Counts the slides of a session, so a page can tell a new slide from one it has already
    /// begun and carry on from where it was after a redraw. The host sets it; leave it out of a
    /// scene file.
    #[serde(default)]
    pub serial: u64,
    /// The host's clock, in milliseconds since it started, when this slide began. The host sets
    /// it; leave it out of a scene file.
    #[serde(default)]
    pub began_ms: u64,
    /// How long ago the slide began, as the host's clock read when it built the answer, so a page
    /// that opens in the middle of the slide carries on from there instead of starting again. The
    /// host sets it; leave it out of a scene file.
    #[serde(default)]
    pub elapsed_ms: u64,
}

/// Where the shared screen's camera looks: in on one element, or out to the whole stage.
///
/// The camera moves from wherever it was to this framing in `duration_ms`, at an even pace. Only
/// the picture moves: the caption, the party strip and the status stay where they are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Camera {
    /// The element to keep in the middle of the screen, or `None` for the whole stage.
    pub target: Option<String>,
    /// How far in, in percent: 100 shows the whole stage, 200 shows it twice as large.
    pub zoom_percent: u16,
    /// How long the move takes, in milliseconds. Zero cuts straight to the framing.
    pub duration_ms: u64,
    /// Counts the camera moves of a session, so a page can tell a new move from one it has
    /// already begun and carry on from where it was after a redraw. The host sets it; leave it out
    /// of a scene file.
    #[serde(default)]
    pub serial: u64,
    /// The host's clock, in milliseconds since it started, when this move began. The host sets
    /// it; leave it out of a scene file.
    #[serde(default)]
    pub began_ms: u64,
    /// How long ago the move began, as the host's clock read when it built the answer, so a page
    /// that opens in the middle of the move carries on from there instead of starting again. The
    /// host sets it; leave it out of a scene file.
    #[serde(default)]
    pub elapsed_ms: u64,
}

/// A picture from the media library, by file name, with an optional line under it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageRef {
    pub media: String,
    pub caption: Option<String>,
    /// Clockwise turn in degrees, 0 to 359 (180 shows it upside down). Defaults to 0.
    #[serde(default)]
    pub rotation_degrees: u16,
    /// A slide across the shared screen, for a card.
    #[serde(default)]
    pub slide: Option<Slide>,
    /// Where a segmentation scan found people in this picture (`media locate`); bubbles anchor to
    /// its regions. A different picture drops it.
    #[serde(default)]
    pub layout: Option<crate::image_layout::ImageLayout>,
    /// What a backdrop picture is, as the host that put it up says; only the backdrop's is read.
    /// Defaults to a room plate.
    #[serde(default)]
    pub plate: PlateKind,
}

/// What a backdrop picture is, as the host that put it up knows it: the page reads this and never
/// guesses it from the picture's name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlateKind {
    /// A picture of the room with its own scan, or with none: the cast stand where its layout says.
    #[default]
    Room,
    /// A room picture shown with the cast of the room before it, as its own scan missed someone:
    /// the cast may speak, but nobody stands where that layout says.
    RoomKeptCast,
    /// An insert: a cut to an item filling the screen, shown with the cast of the room it cuts
    /// away from, so the cast may speak while nobody is in the picture.
    CloseUp,
    /// The end-of-day show: a room picture with its own scan, like `Room` in every way, that the
    /// runner marks so the inventory panel steps aside while it is up (`inventory::picture_is_up`).
    Show,
}

impl PlateKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Room => "room",
            Self::RoomKeptCast => "room_kept_cast",
            Self::CloseUp => "close_up",
            Self::Show => "show",
        }
    }

    /// Whether the backdrop's scan is of that very picture, so the cast stand where it says: a
    /// room plate, and the show plate that is one.
    pub fn has_own_layout(self) -> bool {
        matches!(self, Self::Room | Self::Show)
    }
}

/// What a roll tests or rolls for. The page words it as the card's kicker and decides nothing
/// else from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollKind {
    AbilityCheck,
    SkillCheck,
    PassivePerception,
    SavingThrow,
    MeleeAttack,
    RangedAttack,
    ContestedCheck,
    Initiative,
    DeathSavingThrow,
    Damage,
    Healing,
    HitDie,
    BonusDie,
    TableRoll,
}

impl RollKind {
    pub const ALL: [Self; 14] = [
        Self::AbilityCheck,
        Self::SkillCheck,
        Self::PassivePerception,
        Self::SavingThrow,
        Self::MeleeAttack,
        Self::RangedAttack,
        Self::ContestedCheck,
        Self::Initiative,
        Self::DeathSavingThrow,
        Self::Damage,
        Self::Healing,
        Self::HitDie,
        Self::BonusDie,
        Self::TableRoll,
    ];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::AbilityCheck => "ability_check",
            Self::SkillCheck => "skill_check",
            Self::PassivePerception => "passive_perception",
            Self::SavingThrow => "saving_throw",
            Self::MeleeAttack => "melee_attack",
            Self::RangedAttack => "ranged_attack",
            Self::ContestedCheck => "contested_check",
            Self::Initiative => "initiative",
            Self::DeathSavingThrow => "death_saving_throw",
            Self::Damage => "damage",
            Self::Healing => "healing",
            Self::HitDie => "hit_die",
            Self::BonusDie => "bonus_die",
            Self::TableRoll => "table_roll",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }
}

/// How the d20 of a test was rolled.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollMode {
    #[default]
    Normal,
    Advantage,
    Disadvantage,
}

impl RollMode {
    pub const ALL: [Self; 3] = [Self::Normal, Self::Advantage, Self::Disadvantage];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Advantage => "advantage",
            Self::Disadvantage => "disadvantage",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }
}

/// What a die of a roll is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DieRole {
    Test,
    Bonus,
    Damage,
    Healing,
    HitDie,
    Table,
}

impl DieRole {
    pub const ALL: [Self; 6] = [
        Self::Test,
        Self::Bonus,
        Self::Damage,
        Self::Healing,
        Self::HitDie,
        Self::Table,
    ];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Test => "test",
            Self::Bonus => "bonus",
            Self::Damage => "damage",
            Self::Healing => "healing",
            Self::HitDie => "hit_die",
            Self::Table => "table",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }
}

/// Which natural face of the d20 earns a mark on the card. The server says so only for the test
/// types the rules give a natural 1 or 20 an effect.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NaturalMark {
    One,
    Twenty,
    #[default]
    #[serde(rename = "none")]
    Unmarked,
}

impl NaturalMark {
    pub const ALL: [Self; 3] = [Self::One, Self::Twenty, Self::Unmarked];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::One => "one",
            Self::Twenty => "twenty",
            Self::Unmarked => "none",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }
}

/// The verdict a test earns, as the server worded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollOutcome {
    Success,
    Failure,
    Hit,
    Miss,
    CriticalHit,
    Won,
    Lost,
    Tied,
}

impl RollOutcome {
    pub const ALL: [Self; 8] = [
        Self::Success,
        Self::Failure,
        Self::Hit,
        Self::Miss,
        Self::CriticalHit,
        Self::Won,
        Self::Lost,
        Self::Tied,
    ];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Hit => "hit",
            Self::Miss => "miss",
            Self::CriticalHit => "critical_hit",
            Self::Won => "won",
            Self::Lost => "lost",
            Self::Tied => "tied",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }
}

/// One die of a roll with the face the server rolled. A percentile roll is one die of 100 sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollDie {
    pub sides: u8,
    pub face: u8,
    pub role: DieRole,
    /// Whether the die counts in the sum: the d20 an advantage or disadvantage left out, and a
    /// die dropped from a pool, are not kept.
    #[serde(default = "kept_by_default")]
    pub kept: bool,
}

fn kept_by_default() -> bool {
    true
}

/// A number added to the dice, with the name the table knows it by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollModifier {
    pub label: String,
    pub value: i32,
}

/// What the total is set against: a difficulty class, an armour class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollTarget {
    pub label: String,
    pub value: i32,
}

/// The die a Heroic Inspiration reroll replaced: `index` is its place in `RollView::dice`, where
/// the new face stands, and `was` the face it had.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspirationReroll {
    pub index: usize,
    pub was: u8,
}

/// One resolved roll as the shared screen shows it: every face, modifier, total and verdict is the
/// server's, and the page works none of them out (`roll`, `kit::roll_card`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollView {
    /// A plain name (letters, digits, `_`, `-` and `.`): it names the roll in the timeline.
    pub id: String,
    /// What is tested, such as "Athletics (Strength)".
    pub title: String,
    pub kind: RollKind,
    #[serde(default)]
    pub mode: RollMode,
    pub dice: Vec<RollDie>,
    #[serde(default)]
    pub modifiers: Vec<RollModifier>,
    pub total: i32,
    #[serde(default)]
    pub target: Option<RollTarget>,
    /// The verdict; a roll that is not a test (damage, a table) has none.
    #[serde(default)]
    pub outcome: Option<RollOutcome>,
    #[serde(default)]
    pub natural: NaturalMark,
    #[serde(default)]
    pub inspiration_reroll: Option<InspirationReroll>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TvScene {
    pub place: String,
    pub status: Status,
    pub caption: Option<Caption>,
    /// A picture drawn behind the figures, filling the stage.
    pub backdrop: Option<ImageRef>,
    /// A picture shown in the middle of the screen, such as a handout or a portrait.
    pub card: Option<ImageRef>,
    /// Where the camera looks; without one the whole stage shows.
    #[serde(default)]
    pub camera: Option<Camera>,
    #[serde(default)]
    pub party: Vec<PartyMember>,
    /// The narrator's box. It takes the caption's place, so a scene has a caption or a narration
    /// box, not both. Optional and additive (see `overlay`).
    #[serde(default)]
    pub narration: Option<NarrationBox>,
    /// How the party strip's heroes look, and how many open seats follow them.
    #[serde(default)]
    pub seats: Option<SeatStrip>,
    /// Whether the DungeonFlux logo is drawn at the top left; the place name moves under it.
    #[serde(default)]
    pub brand: bool,
    /// The lobby: title block, six seats, invite panel and begin button (see `lobby::model`).
    #[serde(default)]
    pub lobby: Option<crate::lobby::model::LobbyScreen>,
    /// The loading screen: where the party is going, the narrator and a progress bar.
    #[serde(default)]
    pub loading: Option<crate::lobby::model::LoadingScreen>,
    /// The roleplay card: title bar, speech bubbles and caption.
    #[serde(default)]
    pub roleplay: Option<crate::lobby::model::RoleplayScene>,
    /// The top bar: wordmark, session and party pills, and the optional balance and rate.
    #[serde(default)]
    pub pills: Option<crate::lobby::model::TopBar>,
    /// The landing page: brand, title, navigation, actions, settings, adventure and campaigns
    /// (see `home::model`).
    #[serde(default)]
    pub home: Option<crate::home::model::HomeScene>,
    /// The clue board: every clue found so far (see `clues`).
    #[serde(default)]
    pub clues: Option<crate::clues::ClueBoard>,
    /// The story being played: its title and the place the party has reached.
    #[serde(default)]
    pub story: Option<crate::overlay::StoryMark>,
    /// The inventory panel: what one owner holds and what the newest change moved (see
    /// `inventory`).
    #[serde(default)]
    pub inventory: Option<crate::inventory::InventoryPanel>,
    /// The roll card: one resolved roll, staged over the screen (see `roll`).
    #[serde(default)]
    pub roll: Option<crate::roll::ShownRoll>,
    pub elements: ElementSource,
}

impl TvScene {
    /// The scanned layout of the backdrop picture, when the host says it is that very picture's
    /// (a room plate, `PlateKind::has_own_layout`): a close-up or a room shown with the room's
    /// cast before it has nobody in it where that layout's figures stood, so nothing is drawn
    /// round them, no bubble is pinned to them and the camera frames none of them (owner,
    /// 2026-11-08: an outline only round a character who is visible).
    pub fn backdrop_layout(&self) -> Option<&crate::image_layout::ImageLayout> {
        self.backdrop
            .as_ref()
            .filter(|backdrop| backdrop.plate.has_own_layout())
            .and_then(|backdrop| backdrop.layout.as_ref())
    }

    /// The layout the backdrop carries, whatever kind of plate it is: the cast's names and the
    /// room picture their faces are cut from. A close-up keeps it so the cast may speak, but
    /// nothing is drawn on the close-up from it (`backdrop_layout` is that rule).
    pub fn cast_layout(&self) -> Option<&crate::image_layout::ImageLayout> {
        self.backdrop
            .as_ref()
            .and_then(|backdrop| backdrop.layout.as_ref())
    }

    /// Whether the host put the backdrop up as a close-up (`PlateKind::CloseUp`).
    pub fn is_close_up(&self) -> bool {
        self.backdrop
            .as_ref()
            .is_some_and(|backdrop| backdrop.plate == PlateKind::CloseUp)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub label: String,
    pub explain: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceGroup {
    pub name: String,
    pub choices: Vec<Choice>,
}

/// A choice the hero cannot take now, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnavailableChoice {
    pub label: String,
    pub reason: String,
}

/// A private note shown to one seat only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteCard {
    pub text: String,
}

/// What one player's phone shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhoneScene {
    pub hero: String,
    pub goal: Option<String>,
    pub status: Status,
    #[serde(default)]
    pub narration: Vec<String>,
    #[serde(default)]
    pub suggested: Vec<Choice>,
    #[serde(default)]
    pub groups: Vec<ChoiceGroup>,
    #[serde(default)]
    pub unavailable: Vec<UnavailableChoice>,
    pub note: Option<NoteCard>,
    /// A picture shown at the top of the story, such as a handout or a portrait.
    pub image: Option<ImageRef>,
}

/// One complete scenario: the shared screen and one phone, drawn together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub tv: TvScene,
    pub phone: PhoneScene,
    /// Music that plays while this scene is shown, from the media library.
    #[serde(default)]
    pub music: Option<Track>,
}

impl Fixture {
    /// Reads a fixture, naming the line and column of the first mistake. A value outside the range
    /// the overlay and the command line enforce is a mistake too: all or nothing, checked at the
    /// edge.
    pub fn parse(text: &str) -> Result<Self, FixtureError> {
        let fixture: Self = serde_json::from_str(text)
            .map_err(|error| crate::fixture_checks::serde_error(text, &error))?;
        match crate::fixture_checks::first_problem(&fixture, text) {
            None => Ok(fixture),
            Some(error) => Err(error),
        }
    }
}

/// A fixture file and the name of the scenario in it, for the scenario menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureTitle {
    pub file: String,
    /// `None` when the file is not a valid scenario.
    pub title: Option<String>,
}

impl FixtureTitle {
    /// The title of the fixture file `file` whose text is `text`.
    pub fn read(file: String, text: &str) -> Self {
        let title = Fixture::parse(text).ok().map(|fixture| fixture.name);
        Self { file, title }
    }
}

/// A fixture that could not be read, with where the reader stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for FixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "line {}, column {}: {}",
            self.line, self.column, self.message
        )
    }
}

impl std::error::Error for FixtureError {}

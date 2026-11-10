//! Live changes to the scene a fixture describes: the caption, the party strip, the phone's
//! choices, pictures and sound, made from the dev CLI while the page is open.
//!
//! A fixture is a file; a change here is not saved to it. The changes are kept as an overlay that
//! is drawn on top of the fixture, so `scene reset` (or choosing another fixture) returns to the
//! file. This module holds the overlay's shape and how it is drawn; `scene_apply` decides what a
//! change means and `checks` what is too large to accept. It does not enforce the
//! table-experience contract: a label of six words is accepted so the playground can show the
//! break, exactly as a fixture that breaks it is.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::clues::{ClueBoard, ClueNote};
use crate::controls::Device;
use crate::fixture::{
    Camera, Caption, Choice, ChoiceGroup, Fixture, ImageRef, PartyMember, PlateKind, RollView,
    Slide, UnavailableChoice,
};
use crate::inventory::InventoryPanel;
use crate::limits::{
    MAXIMUM_CAMERA_MS, MAXIMUM_PARTY_MEMBERS, MAXIMUM_PIVOT_PERCENT, MAXIMUM_ROTATION_DEGREES,
    MAXIMUM_SLIDE_MS, MAXIMUM_VOLUME_PERCENT, MAXIMUM_ZOOM_PERCENT, MINIMUM_ZOOM_PERCENT,
};
use crate::lobby::model::{LoadingScreen, LobbyScreen, RoleplayScene, TopBar};
use crate::overlay::{
    Edit, Glyph, NarrationBox, NarrationPlacement, Progress, SeatStrip, StoryMark,
};
use crate::roll::ShownRoll;

/// Which screens a sound is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Both,
    Tv,
    Phone,
}

impl Target {
    pub const ALL: [Self; 3] = [Self::Both, Self::Tv, Self::Phone];

    pub fn key(self) -> &'static str {
        match self {
            Self::Both => "both",
            Self::Tv => "tv",
            Self::Phone => "phone",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }

    /// Whether a page showing `device` should play a sound meant for this target.
    pub fn plays_on(self, device: Device) -> bool {
        match self {
            Self::Both => true,
            Self::Tv => device.shows_tv(),
            Self::Phone => device.shows_phone(),
        }
    }
}

/// A picture slot on the screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageSlot {
    /// Behind the figures on the shared screen.
    TvBackdrop,
    /// In the middle of the shared screen.
    TvCard,
    /// At the top of a phone's story.
    PhoneImage,
}

impl ImageSlot {
    pub const ALL: [Self; 3] = [Self::TvBackdrop, Self::TvCard, Self::PhoneImage];

    pub fn key(self) -> &'static str {
        match self {
            Self::TvBackdrop => "tv_backdrop",
            Self::TvCard => "tv_card",
            Self::PhoneImage => "phone_image",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.key() == key)
    }
}

/// Music that plays until it is stopped or replaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub media: String,
    pub target: Target,
    /// Percent, 0 to 100.
    pub volume_percent: u8,
    pub looping: bool,
}

/// A sound played once. Pages play each cue they have not played yet, by its id. A key this type
/// does not know is refused, like every slot of the overlay that holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cue {
    pub id: u64,
    pub target: Target,
    pub media: String,
    pub volume_percent: u8,
}

/// The changes drawn on top of a fixture: one `Edit` per slot, where `Keep` leaves the fixture's
/// own. A kept slot is left out of the JSON, and a key this type does not know is refused, so a
/// reader of another version fails loudly instead of dropping a change.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SceneOverlay {
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub caption: Edit<Caption>,
    /// The party strip's members; `Clear` draws no one.
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub party: Edit<Vec<PartyMember>>,
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub backdrop: Edit<ImageRef>,
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub card: Edit<ImageRef>,
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub camera: Edit<Camera>,
    /// The narration box: set, or cleared so the fixture's own (or the caption) shows.
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub narration: Edit<NarrationBox>,
    /// The looks of the party strip's heroes and its open seats.
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub seats: Edit<SeatStrip>,
    /// Whether the logo is drawn.
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub brand: Edit<bool>,
    /// The player lobby the dev CLI changed (`lobby::change`).
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub lobby: Edit<LobbyScreen>,
    /// The loading screen the dev CLI changed (`lobby::change`).
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub loading: Edit<LoadingScreen>,
    /// The roleplay card the dev CLI changed (`lobby::change`).
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub roleplay: Edit<RoleplayScene>,
    /// The top bar the dev CLI changed (`lobby::change`).
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub pills: Edit<TopBar>,
    /// The landing page: set whole, or cleared (see `home::change`).
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub home: Edit<crate::home::model::HomeScene>,
    /// The phone's suggested choices; `Clear` empties the list.
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub suggested: Edit<Vec<Choice>>,
    /// The phone's choice groups; `Clear` removes them all.
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub groups: Edit<Vec<ChoiceGroup>>,
    /// The phone's unavailable choices; `Clear` empties the list.
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub unavailable: Edit<Vec<UnavailableChoice>>,
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub phone_image: Edit<ImageRef>,
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub music: Edit<Track>,
    /// The clue board on the shared screen: set whole, or cleared.
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub clues: Edit<ClueBoard>,
    /// The story's title and the place reached, at the top right of the shared screen: set whole,
    /// or cleared.
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub story: Edit<StoryMark>,
    /// The inventory panel on the shared screen: set whole by the host, or cleared.
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub inventory: Edit<InventoryPanel>,
    /// The roll card on the shared screen: set by the host, which stamps it, or cleared.
    #[serde(skip_serializing_if = "Edit::is_keep")]
    pub roll: Edit<ShownRoll>,
    /// How many cues there have ever been. It only rises, so a page can tell a new cue from one it
    /// has played even after a reset.
    pub cue_count: u64,
    /// How many slides there have ever been; it only rises, like `cue_count`.
    pub slide_count: u64,
    /// How many camera moves there have ever been; it only rises, like `slide_count`.
    pub camera_count: u64,
    /// How many rolls there have ever been; it only rises, like `slide_count`.
    pub roll_count: u64,
    /// The newest cues, oldest first.
    pub cues: Vec<Cue>,
}

/// One change to the scene, as the dev CLI sends it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case", deny_unknown_fields)]
pub enum SceneChange {
    Caption {
        speaker: Option<String>,
        text: String,
    },
    ClearCaption {},
    PartyAdd {
        member: PartyMember,
    },
    /// Changes the named member; a field left out stays as it is.
    PartySet {
        name: String,
        hit_points: Option<u32>,
        maximum_hit_points: Option<u32>,
        conditions: Option<Vec<String>>,
    },
    PartyRemove {
        name: String,
    },
    PartyClear {},
    /// Replaces the suggested list.
    Suggested {
        choices: Vec<Choice>,
    },
    /// Replaces one group's choices; an empty list removes the group.
    Group {
        name: String,
        choices: Vec<Choice>,
    },
    /// Replaces the unavailable list.
    Unavailable {
        choices: Vec<UnavailableChoice>,
    },
    /// Empties the suggested list, every group and the unavailable list.
    ClearChoices {},
    /// Shows a picture from the media library in a slot.
    Image {
        slot: ImageSlot,
        media: String,
        caption: Option<String>,
        /// Clockwise turn in degrees, 0 to 359.
        #[serde(default)]
        rotation_degrees: u16,
        /// Slides the card across the shared screen. Only a `tv_card` may slide.
        #[serde(default)]
        slide: Option<Slide>,
        /// The picture's scanned layout; the host adds it for a picture that has one, so a live
        /// change of backdrop keeps its characters (anchors, the SAM layer, camera targets).
        #[serde(default)]
        layout: Option<crate::image_layout::ImageLayout>,
        /// What a backdrop picture is: the host says a room plate, a room shown with the cast
        /// before it, a close-up, or the day's show. Defaults to a room plate; only the backdrop's
        /// is read.
        #[serde(default)]
        plate: PlateKind,
    },
    ClearImage {
        slot: ImageSlot,
    },
    /// Starts music, replacing any that was playing.
    Music {
        media: String,
        target: Target,
        volume_percent: u8,
        looping: bool,
    },
    StopMusic {},
    /// Sets the clue board on the shared screen, replacing the one shown: every clue found so far,
    /// oldest first, the newest last.
    Clues {
        notes: Vec<ClueNote>,
    },
    /// Takes the clue board away.
    ClearClues {},
    /// Sets the story's mark on the shared screen (its title and the place reached), replacing the
    /// one shown.
    Story {
        mark: StoryMark,
    },
    /// Takes the story's mark away.
    ClearStory {},
    /// Sets the inventory panel on the shared screen (one owner's rows and the newest change),
    /// replacing the one shown.
    Inventory {
        panel: InventoryPanel,
    },
    /// Takes the inventory panel away.
    ClearInventory {},
    /// Puts one resolved roll on the shared screen as a card, replacing the one shown. The card
    /// stays at least `min_hold_ms` after its verdict appears, and never under six seconds.
    Roll {
        view: RollView,
        #[serde(default)]
        min_hold_ms: u64,
    },
    /// Takes the roll card away at once.
    ClearRoll {},
    /// Plays a sound once.
    Sound {
        media: String,
        target: Target,
        volume_percent: u8,
    },
    /// Moves the camera in on an element, or out to the whole stage when `target` is `None`.
    Camera {
        target: Option<String>,
        zoom_percent: u16,
        duration_ms: u64,
    },
    /// Sets the narration box, or changes its fields: a field left out stays as it is, and the
    /// text is needed only when there is no narration box yet.
    Narration {
        text: Option<String>,
        #[serde(default)]
        speaker: Edit<String>,
        #[serde(default)]
        badge: Edit<Glyph>,
        #[serde(default)]
        continue_label: Edit<String>,
        voice: Option<bool>,
        placement: Option<NarrationPlacement>,
        #[serde(default)]
        progress: Edit<Progress>,
    },
    /// Takes the narration box away (the caption, if there is one, shows instead).
    ClearNarration {},
    /// Sets how a hero of the party strip looks, and their hit points. A hero who is not in the
    /// party yet is added (it needs `hit_points`); a field left out stays as it is.
    SeatSet {
        name: String,
        #[serde(default)]
        class: Edit<String>,
        #[serde(default)]
        level: Edit<u8>,
        #[serde(default)]
        portrait: Edit<String>,
        #[serde(default)]
        glyph: Edit<Glyph>,
        #[serde(default)]
        accent: Edit<String>,
        hit_points: Option<u32>,
        maximum_hit_points: Option<u32>,
        conditions: Option<Vec<String>>,
    },
    /// Takes a hero out of the party strip (their look and their place in the party).
    SeatRemove {
        name: String,
    },
    /// Sets how many open seats follow the heroes.
    SeatsOpen {
        open: u8,
    },
    /// Takes every look and every open seat away; the party's hit points stay.
    SeatsClear {},
    /// Draws or hides the logo at the top left.
    Brand {
        on: bool,
    },
    /// Changes the lobby, the loading screen, the roleplay card or the top bar (`lobby::change`).
    Screen {
        edit: crate::lobby::change::ScreenEdit,
    },
    /// Changes the landing page (`home::change`).
    Home {
        edit: crate::home::change::HomeEdit,
    },
    /// Drops every change and returns to the fixture. Cues already made stay counted.
    Reset {},
}

/// Why a change was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneError {
    Empty(&'static str),
    TooLong {
        what: &'static str,
        limit: usize,
    },
    DuplicateMember(String),
    DuplicateClue(String),
    UnknownMember(String),
    TooManyMembers,
    TooManyChoices {
        limit: usize,
    },
    InvalidMaximum,
    InvalidVolume(u8),
    InvalidRotation(u16),
    InvalidSlide,
    SlideNeedsTheCard,
    InvalidZoom(u16),
    InvalidCameraTime(u64),
    /// A scenario file's camera looks at an element its own stage does not have.
    UnknownCameraTarget(String),
    /// A scenario file's figure turns about a point outside its own size.
    InvalidPivot(u8),
    /// A scenario file sets a number only the host sets.
    HostOwned(&'static str),
    /// A list of overlay parts is longer than its very large limit.
    TooMany {
        what: &'static str,
        limit: usize,
    },
    /// The inventory panel has a row twice.
    DuplicateRow(String),
    /// The inventory panel moved a row it does not have.
    UnknownRow(String),
    /// A roll's id is not a plain name.
    InvalidRollId(String),
    /// A roll holds a die the page cannot draw, or a face the die does not have.
    InvalidDie {
        sides: u8,
        face: u8,
    },
    /// A roll's Heroic Inspiration reroll names no die of the roll, or a face it cannot have.
    InvalidReroll,
    /// A roll's total is not its kept dice and modifiers added up.
    RollTotalMismatch {
        sum: i64,
        total: i32,
    },
    /// A scene's number is 0 or past its total.
    InvalidProgress {
        index: u16,
        total: u16,
    },
    /// A scene has both a caption and a narration box, which share one place.
    NarrationAndCaption,
    /// A hero who is not in the party was given no hit points.
    NeedsHitPoints(String),
    /// A narration box was asked for with no text and there is none to change.
    NoNarrationYet,
    /// A change to the lobby, loading screen, roleplay card or top bar is out of range.
    Screen(crate::lobby::change::ScreenError),
    /// A change to the landing page, or a value of it, is out of range.
    Home(crate::home::checks::HomeError),
}

impl fmt::Display for SceneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(what) => write!(formatter, "{what} cannot be empty"),
            Self::TooLong { what, limit } => {
                write!(formatter, "{what} is longer than {limit} characters")
            }
            Self::DuplicateMember(name) => write!(formatter, "'{name}' is already in the party"),
            Self::DuplicateClue(id) => write!(formatter, "the clue board has '{id}' twice"),
            Self::DuplicateRow(id) => write!(formatter, "the inventory has '{id}' twice"),
            Self::UnknownRow(id) => {
                write!(
                    formatter,
                    "the inventory moved '{id}', which is not a row of it"
                )
            }
            Self::InvalidRollId(id) => write!(
                formatter,
                "a roll's id is a plain name of letters, digits, '_', '-' and '.', not '{id}'"
            ),
            Self::InvalidDie { sides, face } => write!(
                formatter,
                "a roll holds a d4, d6, d8, d10, d12, d20 or d100, with a face from 1 to its \
                 sides, not face {face} of {sides} sides"
            ),
            Self::InvalidReroll => formatter.write_str(
                "the Heroic Inspiration reroll must name a die of the roll and a face it has",
            ),
            Self::RollTotalMismatch { sum, total } => write!(
                formatter,
                "the roll's kept dice and modifiers add up to {sum}, not its total {total}"
            ),
            Self::UnknownMember(name) => write!(formatter, "'{name}' is not in the party"),
            Self::TooManyMembers => write!(
                formatter,
                "the party strip holds at most {MAXIMUM_PARTY_MEMBERS} members"
            ),
            Self::TooManyChoices { limit } => {
                write!(formatter, "a list holds at most {limit} choices")
            }
            Self::InvalidMaximum => formatter.write_str("maximum hit points must be at least 1"),
            Self::InvalidSlide => write!(
                formatter,
                "a slide goes from and to a share of the screen from 0 to 100 percent and takes 1 to {MAXIMUM_SLIDE_MS} ms"
            ),
            Self::SlideNeedsTheCard => {
                formatter.write_str("only the card on the shared screen can slide")
            }
            Self::InvalidZoom(percent) => write!(
                formatter,
                "a zoom is from {MINIMUM_ZOOM_PERCENT} to {MAXIMUM_ZOOM_PERCENT} percent, not {percent}"
            ),
            Self::InvalidCameraTime(ms) => write!(
                formatter,
                "a camera move takes 0 to {MAXIMUM_CAMERA_MS} ms, not {ms}"
            ),
            Self::UnknownCameraTarget(target) => {
                write!(
                    formatter,
                    "the camera looks at '{target}', which is not on the stage"
                )
            }
            Self::HostOwned(field) => write!(
                formatter,
                "'{field}' is set by the host; leave it out of a scene file"
            ),
            Self::InvalidPivot(percent) => write!(
                formatter,
                "a pivot is from 0 to {MAXIMUM_PIVOT_PERCENT} percent of the figure, not {percent}"
            ),
            Self::InvalidRotation(degrees) => write!(
                formatter,
                "a rotation is from 0 to {MAXIMUM_ROTATION_DEGREES} degrees, not {degrees}"
            ),
            Self::TooMany { what, limit } => {
                write!(formatter, "{what} holds at most {limit} entries")
            }
            Self::InvalidProgress { index, total } => write!(
                formatter,
                "scene {index} of {total} does not exist: a scene is counted from 1 up to the total"
            ),
            Self::NarrationAndCaption => formatter.write_str(
                "a scene has a caption or a narration box, not both: they share one place",
            ),
            Self::NeedsHitPoints(name) => write!(
                formatter,
                "'{name}' is not in the party yet: give hp=N to add them"
            ),
            Self::NoNarrationYet => formatter
                .write_str("there is no narration box to change: give the text to make one"),
            Self::Screen(error) => write!(formatter, "{error}"),
            Self::Home(error) => write!(formatter, "{error}"),
            Self::InvalidVolume(volume_percent) => {
                write!(
                    formatter,
                    "a volume is from 0 to {MAXIMUM_VOLUME_PERCENT} percent, not {volume_percent}"
                )
            }
        }
    }
}

impl std::error::Error for SceneError {}

impl SceneOverlay {
    /// The overlay with every change dropped but the counts, which only rise. Used when a scene is
    /// reset or another fixture is chosen, so a page never mistakes a new cue for an old one.
    pub fn cleared(&self) -> Self {
        Self {
            cue_count: self.cue_count,
            slide_count: self.slide_count,
            camera_count: self.camera_count,
            roll_count: self.roll_count,
            ..Self::default()
        }
    }

    /// True when the overlay changes nothing about what is drawn. Past cues do not count.
    pub fn is_empty(&self) -> bool {
        self.changed_slots().is_empty()
    }

    /// The JSON names of the slots this overlay changes (every one that is not `Keep`), in field
    /// order. Every slot is named, so a new one cannot be left out of the report.
    pub fn changed_slots(&self) -> Vec<&'static str> {
        let Self {
            caption,
            party,
            backdrop,
            card,
            camera,
            narration,
            seats,
            brand,
            lobby,
            loading,
            roleplay,
            pills,
            home,
            suggested,
            groups,
            unavailable,
            phone_image,
            music,
            clues,
            story,
            inventory,
            roll,
            cue_count: _,
            slide_count: _,
            camera_count: _,
            roll_count: _,
            cues: _,
        } = self;
        [
            ("caption", caption.is_keep()),
            ("party", party.is_keep()),
            ("backdrop", backdrop.is_keep()),
            ("card", card.is_keep()),
            ("camera", camera.is_keep()),
            ("narration", narration.is_keep()),
            ("seats", seats.is_keep()),
            ("brand", brand.is_keep()),
            ("lobby", lobby.is_keep()),
            ("loading", loading.is_keep()),
            ("roleplay", roleplay.is_keep()),
            ("pills", pills.is_keep()),
            ("home", home.is_keep()),
            ("suggested", suggested.is_keep()),
            ("groups", groups.is_keep()),
            ("unavailable", unavailable.is_keep()),
            ("phone_image", phone_image.is_keep()),
            ("music", music.is_keep()),
            ("clues", clues.is_keep()),
            ("story", story.is_keep()),
            ("inventory", inventory.is_keep()),
            ("roll", roll.is_keep()),
        ]
        .into_iter()
        .filter_map(|(name, kept)| (!kept).then_some(name))
        .collect()
    }

    /// This overlay as a reader should see it `now_ms` after the host started: each move that is
    /// part of it says how long ago it began, so a page that opens mid-move carries on from there.
    pub fn at(&self, now_ms: u64) -> Self {
        let mut stamped = self.clone();
        if let Edit::Set(camera) = &mut stamped.camera {
            camera.elapsed_ms = now_ms.saturating_sub(camera.began_ms);
        }
        for slot in [
            &mut stamped.backdrop,
            &mut stamped.card,
            &mut stamped.phone_image,
        ] {
            if let Edit::Set(image) = slot
                && let Some(slide) = &mut image.slide
            {
                slide.elapsed_ms = now_ms.saturating_sub(slide.began_ms);
            }
        }
        if let Edit::Set(roll) = &mut stamped.roll {
            roll.elapsed_ms = now_ms.saturating_sub(roll.began_ms);
        }
        stamped
    }

    /// The element the overlay's own camera looks at, if it set a camera that looks at one.
    pub fn camera_target(&self) -> Option<&str> {
        self.camera.set_value()?.target.as_deref()
    }

    /// Draws the overlay onto a copy of the fixture. Every slot is named, so a new one cannot be
    /// left undrawn.
    pub fn apply_to(&self, fixture: &mut Fixture) {
        let Self {
            caption,
            party,
            backdrop,
            card,
            camera,
            narration,
            seats,
            brand,
            lobby,
            loading,
            roleplay,
            pills,
            home,
            suggested,
            groups,
            unavailable,
            phone_image,
            music,
            clues,
            story,
            inventory,
            roll,
            cue_count: _,
            slide_count: _,
            camera_count: _,
            roll_count: _,
            cues: _,
        } = self;
        let tv = &mut fixture.tv;
        caption.apply_to(&mut tv.caption);
        party.apply_to_value(&mut tv.party);
        backdrop.apply_to(&mut tv.backdrop);
        card.apply_to(&mut tv.card);
        camera.apply_to(&mut tv.camera);
        narration.apply_to(&mut tv.narration);
        seats.apply_to(&mut tv.seats);
        brand.apply_to_value(&mut tv.brand);
        lobby.apply_to(&mut tv.lobby);
        loading.apply_to(&mut tv.loading);
        roleplay.apply_to(&mut tv.roleplay);
        pills.apply_to(&mut tv.pills);
        home.apply_to(&mut tv.home);
        let phone = &mut fixture.phone;
        suggested.apply_to_value(&mut phone.suggested);
        groups.apply_to_value(&mut phone.groups);
        unavailable.apply_to_value(&mut phone.unavailable);
        phone_image.apply_to(&mut phone.image);
        music.apply_to(&mut fixture.music);
        clues.apply_to(&mut fixture.tv.clues);
        story.apply_to(&mut fixture.tv.story);
        inventory.apply_to(&mut fixture.tv.inventory);
        roll.apply_to(&mut fixture.tv.roll);
    }
}

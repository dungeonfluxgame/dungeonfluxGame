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

use crate::controls::Device;
use crate::fixture::{
    Camera, Caption, Choice, ChoiceGroup, Fixture, ImageRef, PartyMember, Slide, UnavailableChoice,
};
use crate::limits::MAXIMUM_PIVOT_PERCENT;
use crate::overlay::{Edit, Glyph, NarrationBox, NarrationPlacement, Progress, SeatStrip};

pub use crate::limits::{
    MAXIMUM_CAMERA_MS, MAXIMUM_CAPTION_CHARACTERS, MAXIMUM_CHOICES_PER_LIST, MAXIMUM_CUES_KEPT,
    MAXIMUM_NAME_CHARACTERS, MAXIMUM_PARTY_MEMBERS, MAXIMUM_ROTATION_DEGREES, MAXIMUM_SLIDE_MS,
    MAXIMUM_VOLUME, MAXIMUM_ZOOM_PERCENT, MINIMUM_ZOOM_PERCENT,
};

/// What the overlay does to one part of the scene: put something there, or take what is there away.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Override<T> {
    Set(T),
    Clear,
}

/// What the overlay does to the caption.
pub type CaptionOverride = Override<Caption>;

/// Which screens a sound is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Both,
    Tv,
    Phone,
}

impl Target {
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "both" => Some(Self::Both),
            "tv" => Some(Self::Tv),
            "phone" => Some(Self::Phone),
            _ => None,
        }
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
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "tv-backdrop" => Some(Self::TvBackdrop),
            "tv-card" => Some(Self::TvCard),
            "phone" => Some(Self::PhoneImage),
            _ => None,
        }
    }
}

/// Music that plays until it is stopped or replaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub media: String,
    pub target: Target,
    /// Percent, 0 to 100.
    pub volume: u8,
    pub looping: bool,
}

/// A sound played once. Pages play each cue they have not played yet, by its id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cue {
    pub id: u64,
    pub target: Target,
    pub media: String,
    pub volume: u8,
}

/// The changes drawn on top of a fixture. A field that is `None` leaves the fixture's own.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneOverlay {
    pub caption: Option<CaptionOverride>,
    pub party: Option<Vec<PartyMember>>,
    pub suggested: Option<Vec<Choice>>,
    pub groups: Option<Vec<ChoiceGroup>>,
    pub unavailable: Option<Vec<UnavailableChoice>>,
    pub backdrop: Option<Override<ImageRef>>,
    pub card: Option<Override<ImageRef>>,
    pub phone_image: Option<Override<ImageRef>>,
    pub music: Option<Override<Track>>,
    pub camera: Option<Camera>,
    /// The narration box: set, or cleared so the fixture's own (or the caption) shows.
    #[serde(default)]
    pub narration: Option<Override<NarrationBox>>,
    /// The looks of the party strip's heroes and its open seats.
    #[serde(default)]
    pub seats: Option<SeatStrip>,
    /// Whether the logo is drawn.
    #[serde(default)]
    pub brand: Option<bool>,
    /// The lobby, loading screen, roleplay card and top bar the dev CLI changed (`lobby_change`).
    #[serde(default)]
    pub screens: crate::lobby_change::ScreenOverlay,
    /// The landing page: set whole, or cleared (see `home_change`).
    #[serde(default)]
    pub home: Option<Override<crate::home_model::HomeScene>>,
    /// How many cues there have ever been. It only rises, so a page can tell a new cue from one it
    /// has played even after a reset.
    #[serde(default)]
    pub cue_count: u64,
    /// How many slides there have ever been; it only rises, like `cue_count`.
    #[serde(default)]
    pub slide_count: u64,
    /// How many camera moves there have ever been; it only rises, like `slide_count`.
    #[serde(default)]
    pub camera_count: u64,
    /// The newest cues, oldest first.
    #[serde(default)]
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
    },
    ClearImage {
        slot: ImageSlot,
    },
    /// Starts music, replacing any that was playing.
    Music {
        media: String,
        target: Target,
        volume: u8,
        looping: bool,
    },
    StopMusic {},
    /// Plays a sound once.
    Sound {
        media: String,
        target: Target,
        volume: u8,
    },
    /// Moves the camera in on an element, or out to the whole stage when `target` is `None`.
    Camera {
        target: Option<String>,
        zoom_percent: u16,
        duration_ms: u32,
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
    /// Changes the lobby, the loading screen, the roleplay card or the top bar (`lobby_change`).
    Screen {
        edit: crate::lobby_change::ScreenEdit,
    },
    /// Changes the landing page (`home_change`).
    Home {
        edit: crate::home_change::HomeEdit,
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
    UnknownMember(String),
    TooManyMembers,
    TooManyChoices {
        limit: usize,
    },
    BadMaximum,
    BadVolume(u8),
    BadRotation(u16),
    BadSlide,
    SlideNeedsTheCard,
    BadZoom(u16),
    BadCameraTime(u32),
    /// A scenario file's camera looks at an element its own stage does not have.
    UnknownCameraTarget(String),
    /// A scenario file's figure turns about a point outside its own size.
    BadPivot(u8),
    /// A scenario file sets a number only the host sets.
    HostOwned(&'static str),
    /// A list of overlay parts is longer than its very large limit.
    TooMany {
        what: &'static str,
        limit: usize,
    },
    /// A scene's number is 0 or past its total.
    BadProgress {
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
    Screen(crate::lobby_change::ScreenError),
    /// A change to the landing page, or a value of it, is out of range.
    Home(crate::home_checks::HomeError),
}

impl fmt::Display for SceneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(what) => write!(formatter, "{what} cannot be empty"),
            Self::TooLong { what, limit } => {
                write!(formatter, "{what} is longer than {limit} characters")
            }
            Self::DuplicateMember(name) => write!(formatter, "'{name}' is already in the party"),
            Self::UnknownMember(name) => write!(formatter, "'{name}' is not in the party"),
            Self::TooManyMembers => write!(
                formatter,
                "the party strip holds at most {MAXIMUM_PARTY_MEMBERS} members"
            ),
            Self::TooManyChoices { limit } => {
                write!(formatter, "a list holds at most {limit} choices")
            }
            Self::BadMaximum => formatter.write_str("maximum hit points must be at least 1"),
            Self::BadSlide => write!(
                formatter,
                "a slide goes from and to a share of the screen from 0 to 100 percent and takes 1 to {MAXIMUM_SLIDE_MS} ms"
            ),
            Self::SlideNeedsTheCard => {
                formatter.write_str("only the card on the shared screen can slide")
            }
            Self::BadZoom(percent) => write!(
                formatter,
                "a zoom is from {MINIMUM_ZOOM_PERCENT} to {MAXIMUM_ZOOM_PERCENT} percent, not {percent}"
            ),
            Self::BadCameraTime(ms) => write!(
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
            Self::BadPivot(percent) => write!(
                formatter,
                "a pivot is from 0 to {MAXIMUM_PIVOT_PERCENT} percent of the figure, not {percent}"
            ),
            Self::BadRotation(degrees) => write!(
                formatter,
                "a rotation is from 0 to {MAXIMUM_ROTATION_DEGREES} degrees, not {degrees}"
            ),
            Self::TooMany { what, limit } => {
                write!(formatter, "{what} holds at most {limit} entries")
            }
            Self::BadProgress { index, total } => write!(
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
            Self::BadVolume(volume) => {
                write!(
                    formatter,
                    "a volume is from 0 to {MAXIMUM_VOLUME} percent, not {volume}"
                )
            }
        }
    }
}

impl SceneOverlay {
    /// The overlay with every change dropped but the counts, which only rise. Used when a scene is
    /// reset or another fixture is chosen, so a page never mistakes a new cue for an old one.
    pub fn cleared(&self) -> Self {
        Self {
            cue_count: self.cue_count,
            slide_count: self.slide_count,
            camera_count: self.camera_count,
            ..Self::default()
        }
    }

    /// True when the overlay changes nothing about what is drawn. Past cues do not count.
    pub fn is_empty(&self) -> bool {
        Self {
            cue_count: 0,
            slide_count: 0,
            camera_count: 0,
            cues: Vec::new(),
            ..self.clone()
        } == Self::default()
    }

    /// This overlay as a reader should see it `now_ms` after the host started: each move that is
    /// part of it says how long ago it began, so a page that opens mid-move carries on from there.
    pub fn at(&self, now_ms: u64) -> Self {
        let mut stamped = self.clone();
        if let Some(camera) = &mut stamped.camera {
            camera.elapsed_ms = now_ms.saturating_sub(camera.began_ms);
        }
        for slot in [
            &mut stamped.backdrop,
            &mut stamped.card,
            &mut stamped.phone_image,
        ] {
            if let Some(Override::Set(image)) = slot
                && let Some(slide) = &mut image.slide
            {
                slide.elapsed_ms = now_ms.saturating_sub(slide.began_ms);
            }
        }
        stamped
    }

    /// Draws the overlay onto a copy of the fixture.
    pub fn apply_to(&self, fixture: &mut Fixture) {
        match &self.caption {
            Some(CaptionOverride::Set(caption)) => fixture.tv.caption = Some(caption.clone()),
            Some(CaptionOverride::Clear) => fixture.tv.caption = None,
            None => {}
        }
        if let Some(party) = &self.party {
            fixture.tv.party.clone_from(party);
        }
        if let Some(suggested) = &self.suggested {
            fixture.phone.suggested.clone_from(suggested);
        }
        if let Some(groups) = &self.groups {
            fixture.phone.groups.clone_from(groups);
        }
        if let Some(unavailable) = &self.unavailable {
            fixture.phone.unavailable.clone_from(unavailable);
        }
        match &self.music {
            Some(Override::Set(track)) => fixture.music = Some(track.clone()),
            Some(Override::Clear) => fixture.music = None,
            None => {}
        }
        if let Some(camera) = &self.camera {
            fixture.tv.camera = Some(camera.clone());
        }
        match &self.narration {
            Some(Override::Set(narration)) => fixture.tv.narration = Some(narration.clone()),
            Some(Override::Clear) => fixture.tv.narration = None,
            None => {}
        }
        if let Some(seats) = &self.seats {
            fixture.tv.seats = Some(seats.clone());
        }
        if let Some(on) = self.brand {
            fixture.tv.brand = on;
        }
        self.screens.apply_to(&mut fixture.tv);
        match &self.home {
            Some(Override::Set(home)) => fixture.tv.home = Some(home.clone()),
            Some(Override::Clear) => fixture.tv.home = None,
            None => {}
        }
        apply_image(&self.backdrop, &mut fixture.tv.backdrop);
        apply_image(&self.card, &mut fixture.tv.card);
        apply_image(&self.phone_image, &mut fixture.phone.image);
    }
}

fn apply_image(change: &Option<Override<ImageRef>>, slot: &mut Option<ImageRef>) {
    match change {
        Some(Override::Set(image)) => *slot = Some(image.clone()),
        Some(Override::Clear) => *slot = None,
        None => {}
    }
}

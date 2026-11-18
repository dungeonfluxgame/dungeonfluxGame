//! What the look studio's routes take and answer, shared by the host and the phone. The host
//! defines every control (`LookControls`); the phone draws them and sends its draft back with each
//! refine, never a control of its own.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::dev_reply::{ErrorTag, HostReply};
use crate::seat::wire::SeatErrorKind;

/// The body frame a species stands on: every hero of a frame starts from its one base pose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyFrame {
    /// Halfling and Gnome.
    Small,
    Medium,
    /// Goliath.
    Tall,
}

impl BodyFrame {
    pub const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Tall];

    pub fn key(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Tall => "tall",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|frame| frame.key() == key)
    }
}

/// One option of a swatch or a chip. A swatch's `colour` is the CSS colour the phone paints it in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookOption {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<String>,
}

/// How a control is set, tagged `"type"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlKind {
    /// A whole number from `min` to `max`. `words` names each value from `min` up, when the
    /// values are words (a build); otherwise the value is a number in `unit`.
    Slider {
        min: u32,
        max: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        words: Vec<String>,
    },
    /// One colour of `options`.
    Swatch { options: Vec<LookOption> },
    /// One choice of `options`.
    Chip { options: Vec<LookOption> },
}

/// One value of a control: a slider's number, or the id of a swatch's or a chip's option.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ControlValue {
    Number(u32),
    Choice(String),
}

/// One control the phone draws.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookControl {
    pub id: String,
    pub label: String,
    pub kind: ControlKind,
    pub default: ControlValue,
}

/// The controls of the seat's hero: those of every hero, then the species' own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookControls {
    /// The hero the controls are for: a draft the phone kept for another hero is not used.
    pub hero: u64,
    pub frame: BodyFrame,
    pub controls: Vec<LookControl>,
}

/// What a detail is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetailKind {
    Scar,
    Tattoo,
    Piercing,
    Jewellery,
    Marking,
    /// The body itself: a missing finger, freckles, a prosthesis.
    Body,
    Clothing,
    Other,
}

impl DetailKind {
    pub const ALL: [Self; 8] = [
        Self::Scar,
        Self::Tattoo,
        Self::Piercing,
        Self::Jewellery,
        Self::Marking,
        Self::Body,
        Self::Clothing,
        Self::Other,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Scar => "scar",
            Self::Tattoo => "tattoo",
            Self::Piercing => "piercing",
            Self::Jewellery => "jewellery",
            Self::Marking => "marking",
            Self::Body => "body",
            Self::Clothing => "clothing",
            Self::Other => "other",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }

    /// The kind in plain words, for a chip and a picture's prompt.
    pub fn words(self) -> &'static str {
        match self {
            Self::Scar => "scar",
            Self::Tattoo => "tattoo",
            Self::Piercing => "piercing",
            Self::Jewellery => "jewellery",
            Self::Marking => "marking",
            Self::Body => "body detail",
            Self::Clothing => "clothing",
            Self::Other => "detail",
        }
    }
}

/// Where on the body a detail is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyRegion {
    Head,
    Face,
    Ears,
    Neck,
    Shoulders,
    Chest,
    Back,
    Belly,
    LeftArm,
    RightArm,
    LeftHand,
    RightHand,
    Hips,
    LeftLeg,
    RightLeg,
    Feet,
    WholeBody,
}

impl BodyRegion {
    pub const ALL: [Self; 17] = [
        Self::Head,
        Self::Face,
        Self::Ears,
        Self::Neck,
        Self::Shoulders,
        Self::Chest,
        Self::Back,
        Self::Belly,
        Self::LeftArm,
        Self::RightArm,
        Self::LeftHand,
        Self::RightHand,
        Self::Hips,
        Self::LeftLeg,
        Self::RightLeg,
        Self::Feet,
        Self::WholeBody,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Face => "face",
            Self::Ears => "ears",
            Self::Neck => "neck",
            Self::Shoulders => "shoulders",
            Self::Chest => "chest",
            Self::Back => "back",
            Self::Belly => "belly",
            Self::LeftArm => "left_arm",
            Self::RightArm => "right_arm",
            Self::LeftHand => "left_hand",
            Self::RightHand => "right_hand",
            Self::Hips => "hips",
            Self::LeftLeg => "left_leg",
            Self::RightLeg => "right_leg",
            Self::Feet => "feet",
            Self::WholeBody => "whole_body",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|region| region.key() == key)
    }

    /// The region in plain words, for a chip and a picture's prompt.
    pub fn words(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Face => "face",
            Self::Ears => "ears",
            Self::Neck => "neck",
            Self::Shoulders => "shoulders",
            Self::Chest => "chest",
            Self::Back => "back",
            Self::Belly => "belly",
            Self::LeftArm => "left arm",
            Self::RightArm => "right arm",
            Self::LeftHand => "left hand",
            Self::RightHand => "right hand",
            Self::Hips => "hips",
            Self::LeftLeg => "left leg",
            Self::RightLeg => "right leg",
            Self::Feet => "feet",
            Self::WholeBody => "whole body",
        }
    }
}

/// One detail of the hero's look in the player's words: a chip the player can remove or change.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookDetail {
    pub kind: DetailKind,
    #[serde(rename = "where")]
    pub place: BodyRegion,
    pub what: String,
}

/// The phone's draft: a value per control (a control left out has its default) and the details.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookDraft {
    #[serde(default)]
    pub values: BTreeMap<String, ControlValue>,
    #[serde(default)]
    pub details: Vec<LookDetail>,
}

/// Why a picture of the studio was not made, as closed keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PictureFailure {
    /// The image service is not set up, or refused or failed the picture.
    Backend,
    /// The delivered bytes were not a picture, or the media library did not take them.
    NotStored,
    /// A picture the request follows could not be read.
    Unreadable,
    /// The job stopped without an answer.
    Stopped,
    /// The host restarted while the picture was being made.
    Interrupted,
    /// No template to paint the sheet on: the frame's three-view pose is not chosen yet.
    NoTemplate,
    /// No portrait to paint the sheet from: the one rendered at lock-in failed.
    NoPortrait,
    /// The studio file gives no picture for this item (a class without a ready pose).
    NoPrompt,
    /// The hero library could not be read or written.
    Library,
    /// Jev held the look back from the image service (REF-039).
    Held,
}

impl PictureFailure {
    pub fn key(self) -> &'static str {
        match self {
            Self::Backend => "backend",
            Self::NotStored => "not_stored",
            Self::Unreadable => "unreadable",
            Self::Stopped => "stopped",
            Self::Interrupted => "interrupted",
            Self::NoTemplate => "no_template",
            Self::NoPortrait => "no_portrait",
            Self::NoPrompt => "no_prompt",
            Self::Library => "library",
            Self::Held => "held",
        }
    }
}

/// How a version stands, tagged `"state"`: `pending` > `ready` | `failed`; a result that arrived
/// after a newer refine began is `stale` and never becomes the kept version (REF-032).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum VersionState {
    Pending,
    Ready,
    /// Shown beside the kept one, and the player kept the old one.
    Discarded,
    Stale,
    Failed {
        cause: PictureFailure,
    },
}

impl VersionState {
    pub fn key(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Discarded => "discarded",
            Self::Stale => "stale",
            Self::Failed { .. } => "failed",
        }
    }

    /// Whether the player may keep this version, or go back to it.
    pub fn keepable(self) -> bool {
        matches!(self, Self::Ready | Self::Discarded)
    }
}

/// One version of the look: the draft it was made from, what changed since the version it edits
/// (`parent`; none when it was rendered from the base pose) and its picture once there is one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookVersion {
    pub id: u64,
    pub parent: Option<u64>,
    pub draft: LookDraft,
    pub changes: Vec<String>,
    pub picture: Option<String>,
    #[serde(flatten)]
    pub state: VersionState,
    pub asked_ms: u64,
    pub took_ms: Option<u64>,
}

/// How locking the look in stands, tagged `"state"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LockState {
    /// The design had no kept version: its one portrait is being made (REF-059).
    Portrait,
    /// The reference sheet is being made from the portrait.
    Sheet { portrait: Option<String> },
    /// The hero is locked in with this portrait and sheet. Either may be missing when its picture
    /// failed; the hero locks in all the same (REF-059).
    Locked {
        portrait: Option<String>,
        sheet: Option<String>,
    },
}

/// Locking the look in for `hero`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookLock {
    pub hero: u64,
    #[serde(flatten)]
    pub state: LockState,
}

/// The seat's studio: the kept version, every version (oldest first) and the lock-in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookStudioView {
    pub kept: Option<u64>,
    pub versions: Vec<LookVersion>,
    pub lock: Option<LookLock>,
}

/// The body of the details route: what the player wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookDetailsAsk {
    pub words: String,
}

/// The details route's answer: the controls the words set (a value per control id, as a draft
/// holds them: the phone moves its sliders, swatches and chips to them, as a roll does) and one
/// chip per detail no control covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookDetails {
    #[serde(default)]
    pub values: BTreeMap<String, ControlValue>,
    pub details: Vec<LookDetail>,
}

/// The body of the refine route: the draft, and the version the phone shows as kept (none: render
/// from the base pose). A kept version the host no longer holds as kept is refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookRefineAsk {
    pub draft: LookDraft,
    #[serde(default)]
    pub kept: Option<u64>,
}

/// The body of the keep and the discard routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookVersionAsk {
    pub version: u64,
}

/// The body of the roll route: a random look for the seat's hero, answered as a `LookDraft`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookRollAsk {}

/// The body of the start-again route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookAgainAsk {}

/// One species or class on the start form: its example picture (none when the media library has
/// not got it yet) and one line for the player.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartCard {
    pub id: String,
    pub label: String,
    pub line: String,
    pub picture: Option<String>,
}

/// What a new hero may be started as: the species and the classes of the studio file. Public
/// visual context for the look until the hero's build is made (P2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookStartOptions {
    pub species: Vec<StartCard>,
    pub classes: Vec<StartCard>,
}

/// The body of the start route: a new hero for the seat, its look studio started afresh. The name
/// is held to the hero name rule and never reaches an image prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookStartAsk {
    pub name: String,
    pub species: String,
    pub class: String,
}

/// The body of the lock-in route: the draft, rendered once when no version is kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookLockAsk {
    pub draft: LookDraft,
}

/// Every refusal of the look routes, by the tag of the host's typed error: the host names each of
/// its `LookError`s by one of these (`LookError::kind`), so the phone's list cannot drift from it.
/// A seat refusal reads as the seat routes' own kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookErrorKind {
    InvalidBody,
    InvalidQuery,
    /// The seat plays no hero yet: a new one is started.
    NoHero,
    UnknownSpecies,
    UnknownClass,
    /// The host cannot use its studio file (unreadable, invalid or rejected): the creator is not
    /// available.
    StudioUnavailable,
    PosesUnreadable,
    PosesInvalid,
    InvalidDraft,
    PoseNotChosen,
    EmptyWords,
    TextServiceMissing,
    InvalidModel,
    TextService,
    AnswerRefused,
    InvalidOperationKey,
    Held,
    /// The kept version changed under the phone: it reads the studio again.
    KeptChanged,
    NothingChanged,
    UnknownVersion,
    NotKeepable,
    Locked,
    LockBusy,
    Generate,
    InvalidPicture,
    UnknownPicture,
    InvalidCount,
    Storage,
    StoreFailed,
    StudioClosed,
    Stopped,
    /// The seat, its key or its hero refused, as every seat route says it.
    Seat(SeatErrorKind),
}

impl LookErrorKind {
    /// The look routes' own kinds (a seat refusal is `SeatErrorKind`'s).
    pub const ALL: [Self; 31] = [
        Self::InvalidBody,
        Self::InvalidQuery,
        Self::NoHero,
        Self::UnknownSpecies,
        Self::UnknownClass,
        Self::StudioUnavailable,
        Self::PosesUnreadable,
        Self::PosesInvalid,
        Self::InvalidDraft,
        Self::PoseNotChosen,
        Self::EmptyWords,
        Self::TextServiceMissing,
        Self::InvalidModel,
        Self::TextService,
        Self::AnswerRefused,
        Self::InvalidOperationKey,
        Self::Held,
        Self::KeptChanged,
        Self::NothingChanged,
        Self::UnknownVersion,
        Self::NotKeepable,
        Self::Locked,
        Self::LockBusy,
        Self::Generate,
        Self::InvalidPicture,
        Self::UnknownPicture,
        Self::InvalidCount,
        Self::Storage,
        Self::StoreFailed,
        Self::StudioClosed,
        Self::Stopped,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::InvalidBody => "invalid_body",
            Self::InvalidQuery => "invalid_query",
            Self::NoHero => "no_hero",
            Self::UnknownSpecies => "unknown_species",
            Self::UnknownClass => "unknown_class",
            Self::StudioUnavailable => "studio_unavailable",
            Self::PosesUnreadable => "poses_unreadable",
            Self::PosesInvalid => "poses_invalid",
            Self::InvalidDraft => "invalid_draft",
            Self::PoseNotChosen => "pose_not_chosen",
            Self::EmptyWords => "empty_words",
            Self::TextServiceMissing => "text_service_missing",
            Self::InvalidModel => "invalid_model",
            Self::TextService => "text_service",
            Self::AnswerRefused => "answer_refused",
            Self::InvalidOperationKey => "invalid_operation_key",
            Self::Held => "held",
            Self::KeptChanged => "kept_changed",
            Self::NothingChanged => "nothing_changed",
            Self::UnknownVersion => "unknown_version",
            Self::NotKeepable => "not_keepable",
            Self::Locked => "locked",
            Self::LockBusy => "lock_busy",
            Self::Generate => "generate",
            Self::InvalidPicture => "invalid_picture",
            Self::UnknownPicture => "unknown_picture",
            Self::InvalidCount => "invalid_count",
            Self::Storage => "storage",
            Self::StoreFailed => "store_failed",
            Self::StudioClosed => "studio_closed",
            Self::Stopped => "stopped",
            Self::Seat(kind) => kind.key(),
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.key() == key)
            .or_else(|| SeatErrorKind::from_key(key).map(Self::Seat))
    }
}

impl ErrorTag for LookErrorKind {
    /// Every refusal over the studio file the host cannot use reads as `StudioUnavailable`.
    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "studio_unreadable" | "studio_invalid" | "studio_rejected" => {
                Some(Self::StudioUnavailable)
            }
            tag => Self::from_key(tag),
        }
    }
}

/// A look route's answer as the phone reads it.
pub type LookReply<T> = HostReply<T, LookErrorKind>;

/// One picture of a hero's asset pack, made at lock-in from the kept version, with the reference
/// sheet as the identity reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackItem {
    /// The reference sheet: a front close-up, a three-quarter view and a full-length view.
    Sheet,
    /// A square head-and-shoulders picture: the header medallion, seat cards, the party strip.
    Headshot,
    /// The half-body portrait.
    Portrait,
    /// The hero standing ready for their class.
    Ready,
}

impl PackItem {
    /// Every item, in the order a pack is made: the sheet first, as the others follow it.
    pub const ALL: [Self; 4] = [Self::Sheet, Self::Headshot, Self::Portrait, Self::Ready];

    pub fn key(self) -> &'static str {
        match self {
            Self::Sheet => "sheet",
            Self::Headshot => "headshot",
            Self::Portrait => "portrait",
            Self::Ready => "ready",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|item| item.key() == key)
    }
}

/// How one item of a pack stands, tagged `"state"`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PackStatus {
    /// Not made yet (a hero locked in before the item existed).
    #[default]
    Missing,
    Pending,
    Ready {
        picture: String,
    },
    /// Not made; the hero is locked in all the same, and the item can be made again.
    Failed {
        cause: PictureFailure,
    },
}

impl PackStatus {
    pub fn key(&self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Pending => "pending",
            Self::Ready { .. } => "ready",
            Self::Failed { .. } => "failed",
        }
    }

    /// The picture, once the item is made.
    pub fn picture(&self) -> Option<&str> {
        match self {
            Self::Ready { picture } => Some(picture),
            _ => None,
        }
    }
}

/// A locked-in hero's asset pack.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HeroPack {
    pub sheet: PackStatus,
    pub headshot: PackStatus,
    pub portrait: PackStatus,
    pub ready: PackStatus,
}

impl HeroPack {
    pub fn get(&self, item: PackItem) -> &PackStatus {
        match item {
            PackItem::Sheet => &self.sheet,
            PackItem::Headshot => &self.headshot,
            PackItem::Portrait => &self.portrait,
            PackItem::Ready => &self.ready,
        }
    }

    pub fn set(&mut self, item: PackItem, status: PackStatus) {
        let slot = match item {
            PackItem::Sheet => &mut self.sheet,
            PackItem::Headshot => &mut self.headshot,
            PackItem::Portrait => &mut self.portrait,
            PackItem::Ready => &mut self.ready,
        };
        *slot = status;
    }
}

/// The body of the pack route: make one item again, or every item not made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookPackAsk {
    #[serde(default)]
    pub item: Option<PackItem>,
}

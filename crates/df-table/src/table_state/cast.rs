//! The cast: the heroes and creatures of a scene, as the table state keeps them (D.5).
//!
//! It owns `CastEntry` (a creature's id, kind, controller, scene, name, vitals, visibility,
//! placement and conditions), the small checked values it is made of (`DisplayName`, `Vitals`,
//! `Placement`, `ConditionName`) and `CastChange`, the one way an entry changes after it entered.
//! Every value is checked where it is made, so a hit-point total above its maximum or a place off
//! the canvas cannot exist, and every type is plain data that an effect can carry and a saved
//! record can hold.
//!
//! Visibility is derived by the rules owner from active effects and handed in, never typed by a
//! machine; the table only stores it. It never decides who sees an entry: `projection` does, with
//! the one rule that every hero is always drawn (K.13, M2-D17, R06).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::element::{CANVAS_HEIGHT_UNITS, CANVAS_WIDTH_UNITS, Color, ElementId, name_type};
use crate::identity::{SceneId, SeatId};
use crate::limits::MAXIMUM_NAME_LENGTH;
use crate::listed::listed_enum;
use crate::private_data::{LengthClass, Measured};
use crate::table_state::digest::Digest;

name_type!(
    /// The name of a condition on a creature, such as `poisoned`.
    ConditionName
);

/// Why a display name was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayNameError {
    Blank,
    TooLong,
    ControlCharacter,
}

impl fmt::Display for DisplayNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Blank => formatter.write_str("a name cannot be blank"),
            Self::TooLong => write!(
                formatter,
                "a name is at most {MAXIMUM_NAME_LENGTH} characters"
            ),
            Self::ControlCharacter => formatter.write_str("a name has no control characters"),
        }
    }
}

impl std::error::Error for DisplayNameError {}

/// What a creature is called on screen. Players choose hero names, so its `Debug` says only a
/// length class, like every other private text.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DisplayName(String);

impl DisplayName {
    pub fn new(text: &str) -> Result<Self, DisplayNameError> {
        if text.trim().is_empty() {
            return Err(DisplayNameError::Blank);
        }
        if text.chars().any(char::is_control) {
            return Err(DisplayNameError::ControlCharacter);
        }
        if text.chars().count() > MAXIMUM_NAME_LENGTH {
            return Err(DisplayNameError::TooLong);
        }
        Ok(Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for DisplayName {
    type Error = DisplayNameError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(&text)
    }
}

impl From<DisplayName> for String {
    fn from(name: DisplayName) -> Self {
        name.0
    }
}

impl fmt::Debug for DisplayName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let length = LengthClass::of_length(self.0.length_in_chars());
        write!(formatter, "DisplayName(length={})", length.key())
    }
}

/// Why a media name was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaNameError {
    Blank,
    TooLong,
    NotAFileName,
}

impl fmt::Display for MediaNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Blank => formatter.write_str("a media name cannot be blank"),
            Self::TooLong => write!(
                formatter,
                "a media name is at most {MAXIMUM_NAME_LENGTH} characters"
            ),
            Self::NotAFileName => formatter
                .write_str("a media name is a file name, with no path and no control character"),
        }
    }
}

impl std::error::Error for MediaNameError {}

/// The file name of a picture in the media library, such as a hero's portrait (K.13).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct MediaName(String);

impl MediaName {
    pub fn new(text: &str) -> Result<Self, MediaNameError> {
        if text.trim().is_empty() {
            return Err(MediaNameError::Blank);
        }
        if text.chars().count() > MAXIMUM_NAME_LENGTH {
            return Err(MediaNameError::TooLong);
        }
        if text
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\'))
        {
            return Err(MediaNameError::NotAFileName);
        }
        Ok(Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for MediaName {
    type Error = MediaNameError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(&text)
    }
}

impl From<MediaName> for String {
    fn from(name: MediaName) -> Self {
        name.0
    }
}

/// The colour a hero is drawn in, as `#rrggbb` (K.13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct HeroColor(Color);

/// A colour that is not `#rrggbb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeroColorError;

impl fmt::Display for HeroColorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a colour is six hexadecimal digits after a #")
    }
}

impl std::error::Error for HeroColorError {}

impl HeroColor {
    pub fn parse(text: &str) -> Result<Self, HeroColorError> {
        Color::parse(text).map(Self).ok_or(HeroColorError)
    }

    pub fn to_hex(self) -> String {
        self.0.to_hex()
    }
}

impl TryFrom<String> for HeroColor {
    type Error = HeroColorError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl From<HeroColor> for String {
    fn from(color: HeroColor) -> Self {
        color.to_hex()
    }
}

listed_enum! {
    /// What a cast entry is to the table (D.5).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum CastKind {
        /// A hero: always visible to the party (K.13).
        Party,
        Npc,
        Summon,
        Mount,
        Companion,
        Hireling,
    }
}

impl CastKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Party => "party",
            Self::Npc => "npc",
            Self::Summon => "summon",
            Self::Mount => "mount",
            Self::Companion => "companion",
            Self::Hireling => "hireling",
        }
    }
}

/// Who plays a cast entry: a seat or the engine (D.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case", deny_unknown_fields)]
pub enum Controller {
    Seat { seat: SeatId },
    Engine,
}

listed_enum! {
    /// Whether the party can see a creature, as the rules owner derived it (D.5, M2-D17).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Visibility {
        Seen,
        /// Hiding, or otherwise not noticed.
        Hidden,
        Invisible,
        /// The party knows something is there and cannot see it (heard, tracked, told of): the
        /// shared screen shows a marker for a threat and never the figure (question 66).
        Sensed,
    }
}

impl Visibility {
    pub fn key(self) -> &'static str {
        match self {
            Self::Seen => "seen",
            Self::Hidden => "hidden",
            Self::Invisible => "invisible",
            Self::Sensed => "sensed",
        }
    }

    /// The chip a hero wears while the party cannot see it: hiding and invisibility look the same
    /// to the rest of the party (K.13, M2-D17). A seen creature wears none.
    pub fn chip(self) -> Option<&'static str> {
        match self {
            Self::Seen => None,
            Self::Hidden | Self::Invisible | Self::Sensed => Some("hidden"),
        }
    }
}

/// Why a hit-point record was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VitalsError {
    AboveMaximum,
}

impl fmt::Display for VitalsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("hit points cannot be above the maximum")
    }
}

impl std::error::Error for VitalsError {}

/// A creature's hit points, which are never above its maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "VitalsRecord")]
pub struct Vitals {
    hit_points: u32,
    maximum_hit_points: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VitalsRecord {
    hit_points: u32,
    maximum_hit_points: u32,
}

impl Vitals {
    pub fn new(hit_points: u32, maximum_hit_points: u32) -> Result<Self, VitalsError> {
        if hit_points > maximum_hit_points {
            return Err(VitalsError::AboveMaximum);
        }
        Ok(Self {
            hit_points,
            maximum_hit_points,
        })
    }

    pub fn hit_points(self) -> u32 {
        self.hit_points
    }

    pub fn maximum_hit_points(self) -> u32 {
        self.maximum_hit_points
    }
}

impl TryFrom<VitalsRecord> for Vitals {
    type Error = VitalsError;

    fn try_from(record: VitalsRecord) -> Result<Self, Self::Error> {
        Self::new(record.hit_points, record.maximum_hit_points)
    }
}

/// How many placement units make one canvas unit: a place is kept in thousandths of a canvas unit
/// so it is exact and can be compared, which a floating point number cannot.
const MILLI_UNITS_PER_UNIT: f32 = 1000.0;
/// The canvas width in thousandths of a canvas unit (`CANVAS_WIDTH_UNITS`).
pub(crate) const CANVAS_WIDTH_MILLI_UNITS: u32 = 100_000;
/// The canvas height in thousandths of a canvas unit (`CANVAS_HEIGHT_UNITS`).
pub(crate) const CANVAS_HEIGHT_MILLI_UNITS: u32 = 56_250;

/// Why a placement was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementError {
    OffCanvas,
}

impl fmt::Display for PlacementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "a place is on the canvas: x up to {CANVAS_WIDTH_UNITS}, y up to {CANVAS_HEIGHT_UNITS}"
        )
    }
}

impl std::error::Error for PlacementError {}

/// Where a creature stands on the 100 by 56.25 canvas, in thousandths of a canvas unit. In the
/// engine the rules owner's cells are the truth and this is the presentation mirror written from
/// them (O.13); the table keeps it so the shared screen can draw the scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PlacementRecord")]
pub struct Placement {
    x_milli_units: u32,
    y_milli_units: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlacementRecord {
    x_milli_units: u32,
    y_milli_units: u32,
}

impl Placement {
    pub fn from_milli_units(
        x_milli_units: u32,
        y_milli_units: u32,
    ) -> Result<Self, PlacementError> {
        if x_milli_units > CANVAS_WIDTH_MILLI_UNITS || y_milli_units > CANVAS_HEIGHT_MILLI_UNITS {
            return Err(PlacementError::OffCanvas);
        }
        Ok(Self {
            x_milli_units,
            y_milli_units,
        })
    }

    /// The centre's x, in canvas units. A placement is at most 100,000 thousandths, which a
    /// 32-bit float holds exactly.
    pub fn x_units(self) -> f32 {
        self.x_milli_units as f32 / MILLI_UNITS_PER_UNIT
    }

    /// The centre's y, in canvas units.
    pub fn y_units(self) -> f32 {
        self.y_milli_units as f32 / MILLI_UNITS_PER_UNIT
    }
}

impl TryFrom<PlacementRecord> for Placement {
    type Error = PlacementError;

    fn try_from(record: PlacementRecord) -> Result<Self, Self::Error> {
        Self::from_milli_units(record.x_milli_units, record.y_milli_units)
    }
}

/// One creature of a scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CastEntry {
    id: ElementId,
    kind: CastKind,
    controller: Controller,
    scene: SceneId,
    name: DisplayName,
    vitals: Vitals,
    visibility: Visibility,
    placement: Option<Placement>,
    conditions: Vec<ConditionName>,
    portrait: Option<MediaName>,
    color: Option<HeroColor>,
}

impl CastEntry {
    /// A seen creature with no place on the stage and no conditions.
    pub fn new(
        id: ElementId,
        kind: CastKind,
        controller: Controller,
        scene: SceneId,
        name: DisplayName,
        vitals: Vitals,
    ) -> Self {
        Self {
            id,
            kind,
            controller,
            scene,
            name,
            vitals,
            visibility: Visibility::Seen,
            placement: None,
            conditions: Vec::new(),
            portrait: None,
            color: None,
        }
    }

    pub fn with_visibility(mut self, visibility: Visibility) -> Self {
        self.visibility = visibility;
        self
    }

    pub fn at(mut self, placement: Placement) -> Self {
        self.placement = Some(placement);
        self
    }

    pub fn with_conditions(mut self, conditions: Vec<ConditionName>) -> Self {
        self.conditions = conditions;
        self
    }

    /// The picture the hero is drawn from in the party strip and on its own phone.
    pub fn with_portrait(mut self, portrait: MediaName) -> Self {
        self.portrait = Some(portrait);
        self
    }

    pub fn with_color(mut self, color: HeroColor) -> Self {
        self.color = Some(color);
        self
    }

    pub fn id(&self) -> &ElementId {
        &self.id
    }

    pub fn kind(&self) -> CastKind {
        self.kind
    }

    pub fn controller(&self) -> Controller {
        self.controller
    }

    pub fn scene(&self) -> SceneId {
        self.scene
    }

    pub fn name(&self) -> &DisplayName {
        &self.name
    }

    pub fn vitals(&self) -> Vitals {
        self.vitals
    }

    pub fn visibility(&self) -> Visibility {
        self.visibility
    }

    pub fn placement(&self) -> Option<Placement> {
        self.placement
    }

    pub fn conditions(&self) -> &[ConditionName] {
        &self.conditions
    }

    pub fn portrait(&self) -> Option<&MediaName> {
        self.portrait.as_ref()
    }

    pub fn color(&self) -> Option<HeroColor> {
        self.color
    }

    pub(super) fn digest_into(&self, digest: &mut Digest) {
        let Self {
            id,
            kind,
            controller,
            scene,
            name,
            vitals,
            visibility,
            placement,
            conditions,
            portrait,
            color,
        } = self;
        digest.text(id.as_str());
        digest.text(kind.key());
        match controller {
            Controller::Seat { seat } => {
                digest.byte(1);
                digest.number(seat.get());
            }
            Controller::Engine => digest.byte(0),
        }
        digest.number(scene.get());
        digest.text(name.as_str());
        digest.number(u64::from(vitals.hit_points()));
        digest.number(u64::from(vitals.maximum_hit_points()));
        digest.text(visibility.key());
        digest.option(placement.as_ref(), |digest, placement| {
            digest.number(u64::from(placement.x_milli_units));
            digest.number(u64::from(placement.y_milli_units));
        });
        digest.count(conditions.len());
        for condition in conditions {
            digest.text(condition.as_str());
        }
        digest.option(portrait.as_ref(), |digest, portrait| {
            digest.text(portrait.as_str())
        });
        digest.option(color.as_ref(), |digest, color| digest.text(&color.to_hex()));
    }

    pub(super) fn moved_to(&mut self, scene: SceneId) {
        self.scene = scene;
    }

    pub(super) fn set_visibility(&mut self, visibility: Visibility) {
        self.visibility = visibility;
    }

    pub(super) fn set_placement(&mut self, placement: Option<Placement>) {
        self.placement = placement;
    }

    pub(super) fn set_hit_points(&mut self, vitals: Vitals) {
        self.vitals = vitals;
    }

    pub(super) fn set_conditions(&mut self, conditions: Vec<ConditionName>) {
        self.conditions = conditions;
    }
}

/// How a cast entry changes after it entered: one field at a time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case", deny_unknown_fields)]
pub enum CastChange {
    Visibility { visibility: Visibility },
    Placement { placement: Option<Placement> },
    HitPoints { hit_points: u32 },
    Conditions { conditions: Vec<ConditionName> },
}

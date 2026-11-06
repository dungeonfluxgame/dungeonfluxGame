//! Who is where in a scene picture: the shared record of a segmentation scan, the character anchor
//! API, the cast, and the cues a chat system sends about a character.
//!
//! This crate is the one contract between the parts that touch a scene's characters:
//!
//! - the host's `locate` feature **produces** a [`SceneLayout`] from a SAM segmentation (through
//!   the imagery API) and names its regions (by hand or by the text service's identification);
//! - the playground **draws** it (the SAM layer, anchored bubbles, markers, the camera);
//! - a chat or dialogue system **reads** the [`cast`] (who is in the shot and where) and **sends**
//!   [`CharacterCue`]s by character name (thinking, speaking a line, quiet), which the display
//!   resolves onto the character's region.
//!
//! It is pure data and arithmetic with no I/O, so it builds for the native host and for the
//! browser alike. All positions are thousandths (permille) of the picture's width and height,
//! which are the shared screen's, since the backdrop fills the stage.
//!
//! A layout is bound to its picture by name and SHA-256 digest: a reader must not use a layout
//! whose binding does not match the picture it shows ([`SceneLayout::is_for`]). Regions carry a
//! stable id derived from their mask, a position number (left to right) and an optional name.

use serde::{Deserialize, Serialize};

/// The layout record's version. A reader refuses any other.
pub const SCHEMA_VERSION: u32 = 1;
/// The whole picture, in thousandths.
pub const FULL_PERMILLE: u16 = 1000;
/// How far above the head point an anchored tail stops, so it does not touch the hair.
pub const HEAD_GAP_PERMILLE: u16 = 15;
/// The camera's widest zoom, in percent (the playground's limit): the whole picture.
pub const MINIMUM_ZOOM_PERCENT: u16 = 100;
/// The camera's closest zoom, in percent (the playground's limit).
pub const MAXIMUM_ZOOM_PERCENT: u16 = 10_000;
/// The prefix of a camera target that names a character rather than a stage element:
/// `region:<name>` or `region:<name>:<shot>`.
pub const REGION_TARGET_PREFIX: &str = "region:";
/// The longest character name, in characters.
pub const MAXIMUM_NAME_CHARS: usize = 256;

/// The picture a layout was made from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PictureBinding {
    /// The picture's file name in the media library.
    pub name: String,
    /// The SHA-256 of the picture's bytes, as lowercase hex.
    pub sha256: String,
    /// The picture's width, in pixels.
    pub width_px: u32,
    /// The picture's height, in pixels.
    pub height_px: u32,
}

/// The regions a scan found in one picture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneLayout {
    /// Always [`SCHEMA_VERSION`].
    pub schema: u32,
    /// The picture the regions were found in; see [`SceneLayout::is_for`].
    pub picture: PictureBinding,
    /// The model that made the layout, such as `sam-3.1`.
    pub model: String,
    /// The noun prompt the scan was asked with, such as `person`.
    pub prompt: String,
    /// The instances, left to right; their position in this list is their number.
    pub regions: Vec<SceneRegion>,
}

/// One instance of a scan: its id, its name if it has one, its box, its head point, its score and
/// its mask.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneRegion {
    /// Stable for the same mask: `r` and 8 hex digits ([`region_id`]).
    pub id: String,
    /// The character this region is, such as `innkeeper`.
    #[serde(default)]
    pub label: Option<String>,
    /// The box's left edge, in thousandths of the picture's width.
    pub left_permille: u16,
    /// The box's top edge, in thousandths of the picture's height.
    pub top_permille: u16,
    /// The box's right edge, in thousandths of the picture's width.
    pub right_permille: u16,
    /// The box's bottom edge, in thousandths of the picture's height.
    pub bottom_permille: u16,
    /// The middle of the mask's topmost rows: where the top of the head is, across.
    pub head_x_permille: u16,
    /// Where the top of the head is, down.
    pub head_y_permille: u16,
    /// How sure the scan is of this instance, in thousandths.
    pub score_permille: u16,
    /// The mask as a PNG data URL the size of the picture, opaque where the instance is.
    #[serde(default)]
    pub mask: Option<String>,
}

/// The 64-bit FNV-1a offset basis.
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// The 64-bit FNV-1a prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The stable id of a region whose mask is described by `bytes` (for example its run lengths):
/// `r` and the first 8 hex digits of an FNV-1a hash, so the same mask gets the same id on every
/// machine and every scan.
pub fn region_id(bytes: &[u8]) -> String {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("r{:08x}", hash >> 32)
}

/// A reference to one region: its number (from 0, left to right), its id or its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionRef<'a> {
    /// The region's position, from 0, left to right.
    Number(usize),
    /// The region's id ([`region_id`]).
    Id(&'a str),
    /// The region's label.
    Name(&'a str),
}

impl<'a> RegionRef<'a> {
    /// Reads a reference as a person types it: digits are a number, `r` and 8 hex digits an id,
    /// anything else a name.
    pub fn parse(text: &'a str) -> Self {
        if let Ok(number) = text.parse::<usize>() {
            return Self::Number(number);
        }
        let is_id = text.len() == 9
            && text.starts_with('r')
            && text[1..].bytes().all(|byte| byte.is_ascii_hexdigit());
        if is_id {
            Self::Id(text)
        } else {
            Self::Name(text)
        }
    }
}

impl SceneLayout {
    /// Whether this layout belongs to the picture `name` with the digest `sha256`, in a version
    /// this reader understands.
    pub fn is_for(&self, name: &str, sha256: &str) -> bool {
        self.schema == SCHEMA_VERSION
            && self.picture.name == name
            && self.picture.sha256.eq_ignore_ascii_case(sha256)
    }

    /// The region named `label`, if any.
    pub fn region(&self, label: &str) -> Option<&SceneRegion> {
        let position = self.find(RegionRef::Name(label))?;
        self.regions.get(position)
    }

    /// The position of the region `reference` points at, if any.
    pub fn find(&self, reference: RegionRef<'_>) -> Option<usize> {
        match reference {
            RegionRef::Number(number) => (number < self.regions.len()).then_some(number),
            RegionRef::Id(id) => self.regions.iter().position(|region| region.id == id),
            RegionRef::Name(name) => self
                .regions
                .iter()
                .position(|region| region.label.as_deref() == Some(name)),
        }
    }

    /// The names given in this layout, left to right.
    pub fn names(&self) -> Vec<&str> {
        self.regions
            .iter()
            .filter_map(|region| region.label.as_deref())
            .collect()
    }
}

/// Whether `name` can name a character: 1 to [`MAXIMUM_NAME_CHARS`] letters, digits, `-` or `_`.
pub fn is_character_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAXIMUM_NAME_CHARS
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

/// A named point of a character, taken from its region's box and head point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterPoint {
    /// The top of the head.
    #[default]
    Head,
    /// Where the face is: a little below the top of the head.
    Face,
    /// The middle of the box.
    Centre,
    /// The bottom of the box, under the head.
    Feet,
    /// The left edge of the box, at chest height.
    Left,
    /// The right edge of the box, at chest height.
    Right,
}

impl CharacterPoint {
    /// Every point, head first.
    pub const ALL: [Self; 6] = [
        Self::Head,
        Self::Face,
        Self::Centre,
        Self::Feet,
        Self::Left,
        Self::Right,
    ];

    /// The point's wire and command name, the same as its serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Face => "face",
            Self::Centre => "centre",
            Self::Feet => "feet",
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    /// The point named `key`, if any.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|point| point.key() == key)
    }
}

/// How much of a character the camera frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shot {
    /// The head and face.
    Face,
    /// Head to waist.
    Upper,
    /// The whole figure.
    Full,
}

impl Shot {
    /// Every shot, closest first.
    pub const ALL: [Self; 3] = [Self::Face, Self::Upper, Self::Full];

    /// The shot's wire and command name, the same as its serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Face => "face",
            Self::Upper => "upper",
            Self::Full => "full",
        }
    }

    /// The shot named `key`, if any.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|shot| shot.key() == key)
    }

    /// The share of the character's height the shot shows, in thousandths.
    fn span_permille(self) -> u32 {
        match self {
            Self::Face => 300,
            Self::Upper => 550,
            Self::Full => 1050,
        }
    }
}

/// The share of the screen's height a shot fills, in thousandths.
const SHOT_FILL_PERMILLE: u32 = 900;
/// Where the face is below the top of the head, in thousandths of the character's height.
const FACE_DROP_PERMILLE: u32 = 110;
/// Where the chest is below the top of the head, in thousandths of the character's height.
const CHEST_DROP_PERMILLE: u32 = 300;

/// The character's height from the top of the head to the bottom of the box, at least 1, in
/// thousandths of the screen's height.
fn figure_height_permille(region: &SceneRegion) -> u32 {
    u32::from(
        region
            .bottom_permille
            .saturating_sub(region.head_y_permille),
    )
    .max(1)
}

/// The height `share_permille` of the character's height below the top of the head, held to the
/// picture.
fn y_below_head_permille(region: &SceneRegion, share_permille: u32) -> u16 {
    let drop_permille = figure_height_permille(region) * share_permille / 1000;
    let y_permille = u32::from(region.head_y_permille) + drop_permille;
    // Saturates: a point past the bottom edge is drawn on it.
    u16::try_from(y_permille)
        .unwrap_or(FULL_PERMILLE)
        .min(FULL_PERMILLE)
}

/// The point halfway between `low_permille` and `high_permille`, without overflowing.
fn midpoint_permille(low_permille: u16, high_permille: u16) -> u16 {
    let middle_permille = (u32::from(low_permille) + u32::from(high_permille)) / 2;
    // The mean of two u16 values always fits a u16.
    u16::try_from(middle_permille).unwrap_or(u16::MAX)
}

/// Where `point` of the character in `region` is, in thousandths of the screen (x, y).
pub fn point_of(region: &SceneRegion, point: CharacterPoint) -> (u16, u16) {
    match point {
        CharacterPoint::Head => (region.head_x_permille, region.head_y_permille),
        CharacterPoint::Face => (
            region.head_x_permille,
            y_below_head_permille(region, FACE_DROP_PERMILLE),
        ),
        CharacterPoint::Centre => (
            midpoint_permille(region.left_permille, region.right_permille),
            midpoint_permille(region.top_permille, region.bottom_permille),
        ),
        CharacterPoint::Feet => (region.head_x_permille, region.bottom_permille),
        CharacterPoint::Left => (
            region.left_permille,
            y_below_head_permille(region, CHEST_DROP_PERMILLE),
        ),
        CharacterPoint::Right => (
            region.right_permille,
            y_below_head_permille(region, CHEST_DROP_PERMILLE),
        ),
    }
}

/// Where the camera looks for `shot` of the character in `region`, in thousandths of the screen.
pub fn shot_focus(region: &SceneRegion, shot: Shot) -> (u16, u16) {
    (
        region.head_x_permille,
        y_below_head_permille(region, shot.span_permille() / 2),
    )
}

/// The zoom, in percent, at which `shot` of the character in `region` fills most of the screen's
/// height, within the camera's range. The page still holds it to what the picture's pixels bear.
pub fn shot_zoom_percent(region: &SceneRegion, shot: Shot) -> u16 {
    let span_permille = (figure_height_permille(region) * shot.span_permille() / 1000).max(1);
    u16::try_from(100 * SHOT_FILL_PERMILLE / span_permille)
        .unwrap_or(MAXIMUM_ZOOM_PERCENT)
        .clamp(MINIMUM_ZOOM_PERCENT, MAXIMUM_ZOOM_PERCENT)
}

/// The camera target of the character `name`, with or without a shot: `region:innkeeper:face`.
pub fn region_target(name: &str, shot: Option<Shot>) -> String {
    match shot {
        Some(shot) => format!("{REGION_TARGET_PREFIX}{name}:{}", shot.key()),
        None => format!("{REGION_TARGET_PREFIX}{name}"),
    }
}

/// Every camera target the named regions of `layout` offer, with the point each looks at:
/// `region:<name>` (the upper shot's focus) and one per shot.
pub fn region_targets(layout: &SceneLayout) -> Vec<(String, (u16, u16))> {
    let mut targets = Vec::new();
    for region in &layout.regions {
        let Some(name) = region.label.as_deref() else {
            continue;
        };
        targets.push((region_target(name, None), shot_focus(region, Shot::Upper)));
        for shot in Shot::ALL {
            targets.push((region_target(name, Some(shot)), shot_focus(region, shot)));
        }
    }
    targets
}

/// Thousandths as a CSS percent with one decimal, such as `84.5`.
pub fn percent_text(permille: u16) -> String {
    format!("{}.{}", permille / 10, permille % 10)
}

/// A named character of the shot and where it is, for a chat or dialogue system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CastMember {
    /// The region's label.
    pub name: String,
    /// The region's id ([`region_id`]).
    pub region_id: String,
    /// The region's position number (from 0, left to right).
    pub number: usize,
    /// The top of the head, in thousandths of the screen (x, y).
    pub head: (u16, u16),
    /// The face, in thousandths of the screen (x, y).
    pub face: (u16, u16),
    /// The middle of the box, in thousandths of the screen (x, y).
    pub centre: (u16, u16),
    /// How sure the scan is of the region, in thousandths.
    pub score_permille: u16,
}

/// The named characters of `layout`, left to right.
pub fn cast(layout: &SceneLayout) -> Vec<CastMember> {
    layout
        .regions
        .iter()
        .enumerate()
        .filter_map(|(number, region)| {
            Some(CastMember {
                name: region.label.clone()?,
                region_id: region.id.clone(),
                number,
                head: point_of(region, CharacterPoint::Head),
                face: point_of(region, CharacterPoint::Face),
                centre: point_of(region, CharacterPoint::Centre),
                score_permille: region.score_permille,
            })
        })
        .collect()
}

/// What a character is doing in the conversation, as a chat system says it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CueKind {
    /// Thinking: a wordless indicator over the character.
    Thinking,
    /// Saying a line: a bubble over the character with the line.
    Speaking {
        /// The words the character says.
        line: String,
    },
    /// Nothing over the character.
    Quiet,
}

/// One cue about one character, by name. The display resolves the name against the shot's
/// layout; a name the layout does not have is refused, never drawn somewhere else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterCue {
    /// The character's name, as the layout labels it.
    pub character: String,
    /// What the character is doing.
    #[serde(flatten)]
    pub kind: CueKind,
}

#[cfg(test)]
mod lib_tests;

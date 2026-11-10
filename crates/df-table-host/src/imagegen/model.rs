//! What a picture is asked for with and what a backend hands back: the purpose, the shape, the
//! checked request, the backend's request and the picture it delivered.
//!
//! Nothing here holds a credential, and an audit line never holds the prompt: it holds the
//! prompt's length.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use df_providers::FORGE_IMAGE_MAXIMUM_STEPS;
use serde::{Deserialize, Serialize};

use super::backend::BackendLimits;
use super::limits::{
    MAXIMUM_ASPECT_PART, MAXIMUM_ASPECT_RATIO, MAXIMUM_NAME_CHARS, MAXIMUM_PICTURES,
    MAXIMUM_SENT_TEXT_CHARS, REQUESTED_LONG_SIDE_PX,
};

/// How urgent a picture is. An emergency picture (an action a character declares they are doing
/// right now) goes first wherever the backend has a queue of its own to put it in; every other
/// picture waits its turn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    #[default]
    Normal,
    Emergency,
}

impl Priority {
    pub fn key(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Emergency => "emergency",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        [Self::Normal, Self::Emergency]
            .into_iter()
            .find(|priority| priority.key() == key)
    }

    /// For `skip_serializing_if`: a normal request does not write its priority.
    pub fn is_normal(&self) -> bool {
        *self == Self::Normal
    }
}

/// What the picture is for. It sets the default shape and adds one line of direction to the
/// prompt, so the owner's words stay the first thing the backend reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Plate,
    Background,
    Portrait,
}

impl Purpose {
    pub const ALL: [Self; 3] = [Self::Plate, Self::Background, Self::Portrait];

    pub fn key(self) -> &'static str {
        match self {
            Self::Plate => "plate",
            Self::Background => "background",
            Self::Portrait => "portrait",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|purpose| purpose.key() == key)
    }

    /// The shape a picture of this purpose has when none is asked for.
    pub fn default_aspect(self) -> Aspect {
        match self {
            Self::Plate | Self::Background => Aspect::WIDE,
            Self::Portrait => Aspect::PORTRAIT,
        }
    }

    /// One line of direction added after the prompt.
    pub fn direction(self) -> &'static str {
        match self {
            Self::Plate => concat!(
                "Intended use: a finished scene plate for a tabletop role-playing game shown on a ",
                "television, one wide picture of the place."
            ),
            Self::Background => concat!(
                "Intended use: a full-bleed background picture behind a game interface; ",
                "environment only, with a calm area in the middle for panels."
            ),
            Self::Portrait => concat!(
                "Intended use: a character portrait, one figure from the chest up, centred, in a ",
                "plain setting."
            ),
        }
    }
}

/// A picture shape as two whole numbers, such as 16:9: each from 1 to `MAXIMUM_ASPECT_PART`, and
/// neither more than `MAXIMUM_ASPECT_RATIO` times the other. On the wire it is the text `W:H`,
/// read through the same check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Aspect {
    width: u32,
    height: u32,
}

/// Why a text is not a picture shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AspectError {
    /// It is not two whole numbers joined by ':'.
    NotWidthByHeight,
    /// A number is 0 or more than `MAXIMUM_ASPECT_PART`.
    PartOutOfRange,
    /// One side is more than `MAXIMUM_ASPECT_RATIO` times the other.
    TooElongated,
}

impl fmt::Display for AspectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotWidthByHeight => {
                formatter.write_str("a shape is two whole numbers as W:H, such as 16:9")
            }
            Self::PartOutOfRange => write!(
                formatter,
                "each number of a shape is from 1 to {MAXIMUM_ASPECT_PART}"
            ),
            Self::TooElongated => write!(
                formatter,
                "neither side of a shape may be more than {MAXIMUM_ASPECT_RATIO} times the other"
            ),
        }
    }
}

impl std::error::Error for AspectError {}

impl Aspect {
    pub const WIDE: Self = Self {
        width: 16,
        height: 9,
    };

    pub const PORTRAIT: Self = Self {
        width: 3,
        height: 4,
    };

    pub fn new(width: u32, height: u32) -> Result<Self, AspectError> {
        let in_range = |part: u32| (1..=MAXIMUM_ASPECT_PART).contains(&part);
        if !in_range(width) || !in_range(height) {
            return Err(AspectError::PartOutOfRange);
        }
        if width > height.saturating_mul(MAXIMUM_ASPECT_RATIO)
            || height > width.saturating_mul(MAXIMUM_ASPECT_RATIO)
        {
            return Err(AspectError::TooElongated);
        }
        Ok(Self { width, height })
    }

    /// The shape of a picture of `size`: the sides in lowest terms, scaled down together when a
    /// part would pass `MAXIMUM_ASPECT_PART`; none when the sides are more elongated than a shape
    /// may be.
    pub fn of_size(size: PictureSize) -> Option<Self> {
        let (width, height) = (size.width_px(), size.height_px());
        let divisor = gcd(width, height);
        let (width, height) = (width / divisor, height / divisor);
        let longest = width.max(height);
        if longest <= MAXIMUM_ASPECT_PART {
            return Self::new(width, height).ok();
        }
        let scaled = |side: u32| {
            let part = (u64::from(side) * u64::from(MAXIMUM_ASPECT_PART) + u64::from(longest) / 2)
                / u64::from(longest);
            u32::try_from(part).unwrap_or(MAXIMUM_ASPECT_PART).max(1)
        };
        Self::new(scaled(width), scaled(height)).ok()
    }

    /// The size asked of the backend: the long side is `REQUESTED_LONG_SIDE_PX`.
    pub fn requested_size(self) -> (u32, u32) {
        let (long, short) = if self.width >= self.height {
            (self.width, self.height)
        } else {
            (self.height, self.width)
        };
        // The short side is at most the long side, so the rounded quotient fits a `u32`.
        let short_px = (u64::from(REQUESTED_LONG_SIDE_PX) * u64::from(short) + u64::from(long) / 2)
            / u64::from(long);
        let short_px = u32::try_from(short_px).unwrap_or(REQUESTED_LONG_SIDE_PX);
        if self.width >= self.height {
            (REQUESTED_LONG_SIDE_PX, short_px)
        } else {
            (short_px, REQUESTED_LONG_SIDE_PX)
        }
    }
}

/// The greatest common divisor of two sides, each at least 1.
fn gcd(first: u32, second: u32) -> u32 {
    if second == 0 {
        first
    } else {
        gcd(second, first % second)
    }
}

impl FromStr for Aspect {
    type Err = AspectError;

    /// `W:H` with both whole numbers.
    fn from_str(text: &str) -> Result<Self, AspectError> {
        let (width, height) = text.split_once(':').ok_or(AspectError::NotWidthByHeight)?;
        let part = |part: &str| {
            part.parse::<u32>()
                .map_err(|_| AspectError::NotWidthByHeight)
        };
        Self::new(part(width)?, part(height)?)
    }
}

impl TryFrom<String> for Aspect {
    type Error = AspectError;

    fn try_from(text: String) -> Result<Self, AspectError> {
        text.parse()
    }
}

impl From<Aspect> for String {
    fn from(aspect: Aspect) -> Self {
        aspect.to_string()
    }
}

impl fmt::Display for Aspect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.width, self.height)
    }
}

/// The most either side of a picture size may be, in pixels, against a mistyped size. The Forge
/// backend rounds and clamps what is given to the sides Forge takes.
pub const MAXIMUM_SIZE_SIDE_PX: u32 = 16_384;

/// A picture size in pixels that replaces the shape's own size, such as 512x512: each side from 1
/// to `MAXIMUM_SIZE_SIDE_PX`. On the wire it is the text `WxH`, read through the same check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PictureSize {
    width_px: u32,
    height_px: u32,
}

/// Why a text is not a picture size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureSizeError {
    /// It is not two whole numbers joined by 'x'.
    NotWidthByHeight,
    /// A side is 0 or more than `MAXIMUM_SIZE_SIDE_PX`.
    SideOutOfRange,
}

impl fmt::Display for PictureSizeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotWidthByHeight => {
                formatter.write_str("a size is two whole numbers as WxH, such as 512x512")
            }
            Self::SideOutOfRange => write!(
                formatter,
                "each side of a size is from 1 to {MAXIMUM_SIZE_SIDE_PX} pixels"
            ),
        }
    }
}

impl std::error::Error for PictureSizeError {}

impl PictureSize {
    pub fn new(width_px: u32, height_px: u32) -> Result<Self, PictureSizeError> {
        let in_range = |side: u32| (1..=MAXIMUM_SIZE_SIDE_PX).contains(&side);
        if !in_range(width_px) || !in_range(height_px) {
            return Err(PictureSizeError::SideOutOfRange);
        }
        Ok(Self {
            width_px,
            height_px,
        })
    }

    pub fn width_px(self) -> u32 {
        self.width_px
    }

    pub fn height_px(self) -> u32 {
        self.height_px
    }
}

impl FromStr for PictureSize {
    type Err = PictureSizeError;

    /// `WxH` with both whole numbers.
    fn from_str(text: &str) -> Result<Self, PictureSizeError> {
        let (width, height) = text
            .split_once('x')
            .ok_or(PictureSizeError::NotWidthByHeight)?;
        let side = |side: &str| {
            side.parse::<u32>()
                .map_err(|_| PictureSizeError::NotWidthByHeight)
        };
        Self::new(side(width)?, side(height)?)
    }
}

impl TryFrom<String> for PictureSize {
    type Error = PictureSizeError;

    fn try_from(text: String) -> Result<Self, PictureSizeError> {
        text.parse()
    }
}

impl From<PictureSize> for String {
    fn from(size: PictureSize) -> Self {
        size.to_string()
    }
}

impl fmt::Display for PictureSize {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}x{}", self.width_px, self.height_px)
    }
}

/// How many denoising steps a Forge image job takes in place of the studio's own default: a whole
/// number from 1 to `FORGE_IMAGE_MAXIMUM_STEPS` (Forge's request contract). On the wire it is the
/// number, read through the same check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct InferenceSteps(u32);

/// Why a number is not a count of inference steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InferenceStepsError;

impl fmt::Display for InferenceStepsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "steps are a whole number from 1 to {FORGE_IMAGE_MAXIMUM_STEPS}"
        )
    }
}

impl std::error::Error for InferenceStepsError {}

impl InferenceSteps {
    pub fn new(steps: u32) -> Result<Self, InferenceStepsError> {
        if (1..=FORGE_IMAGE_MAXIMUM_STEPS).contains(&steps) {
            Ok(Self(steps))
        } else {
            Err(InferenceStepsError)
        }
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

impl FromStr for InferenceSteps {
    type Err = InferenceStepsError;

    /// A whole number of steps.
    fn from_str(text: &str) -> Result<Self, InferenceStepsError> {
        Self::new(text.parse().map_err(|_| InferenceStepsError)?)
    }
}

impl TryFrom<u32> for InferenceSteps {
    type Error = InferenceStepsError;

    fn try_from(steps: u32) -> Result<Self, InferenceStepsError> {
        Self::new(steps)
    }
}

impl From<InferenceSteps> for u32 {
    fn from(steps: InferenceSteps) -> Self {
        steps.0
    }
}

impl fmt::Display for InferenceSteps {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Why a request is not one a generation can start from. On the wire it is tagged `"cause"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum GenerateRequestError {
    EmptyPrompt,
    /// The prompt and its purpose's direction are more than the backend takes.
    PromptTooLong {
        chars: usize,
        maximum_chars: usize,
    },
    /// Not 1 to `MAXIMUM_NAME_CHARS` letters, digits, '-' or '_'.
    InvalidName {
        name: String,
    },
    /// The picture to edit is not a plain picture file name of the media library.
    InvalidBase {
        base: String,
    },
    /// A reference is not a plain picture file name of the media library.
    InvalidReference {
        reference: String,
    },
    /// The picture to edit and the references are more pictures than the request or the backend
    /// takes.
    TooManyPictures {
        count: usize,
        maximum: usize,
    },
}

impl GenerateRequestError {
    /// The serde name of the variant.
    pub fn key(&self) -> &'static str {
        match self {
            Self::EmptyPrompt => "empty_prompt",
            Self::PromptTooLong { .. } => "prompt_too_long",
            Self::InvalidName { .. } => "invalid_name",
            Self::InvalidBase { .. } => "invalid_base",
            Self::InvalidReference { .. } => "invalid_reference",
            Self::TooManyPictures { .. } => "too_many_pictures",
        }
    }
}

impl fmt::Display for GenerateRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPrompt => formatter.write_str("the prompt is empty"),
            Self::PromptTooLong {
                chars,
                maximum_chars,
            } => write!(
                formatter,
                "the prompt is {chars} characters; at most {maximum_chars} fit this purpose"
            ),
            Self::InvalidName { name } => write!(
                formatter,
                concat!(
                    "'{}' is not a picture name: use 1 to {} letters, digits, '-' or '_', ",
                    "with no extension"
                ),
                name, MAXIMUM_NAME_CHARS
            ),
            Self::InvalidBase { base } => write!(
                formatter,
                "'{base}' is not a picture file of the media library, such as bar.png"
            ),
            Self::InvalidReference { reference } => write!(
                formatter,
                "the reference '{reference}' is not a picture file of the media library, such as \
                 ref-warrior.png"
            ),
            Self::TooManyPictures { count, maximum } => write!(
                formatter,
                "the request attaches {count} pictures (base and references); at most {maximum} fit"
            ),
        }
    }
}

impl std::error::Error for GenerateRequestError {}

pub(crate) fn plain_name(name: &str) -> bool {
    let count = name.chars().count();
    (1..=MAXIMUM_NAME_CHARS).contains(&count)
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

/// What sits between the prompt and the purpose's direction in the text the backend reads.
const DIRECTION_SEPARATOR: &str = "\n\n";

/// The line the sent text gains when references of people are attached, before the purpose's
/// direction.
const REFERENCES_DIRECTION: &str = concat!(
    "The reference pictures of people set only how they look: keep each person in one exactly ",
    "as shown (face, hair, build, age, costume), never its place, framing or light."
);

/// What comes before what a picture must not show, when the backend reads it in the text.
const AVOID_LEAD: &str = "Never show: ";

/// A picture of the media library that shows how one thing in the new picture looks (an item's
/// close-up), and what it shows, in a few words (`the brass cipher wheel`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThingReference {
    pub picture: String,
    pub shows: String,
}

/// One checked request: a prompt, the name the picture is stored under (without its extension),
/// what it is for, its shape, the picture of the media library it edits, when it is an edit, the
/// pictures of the media library it follows for the people's looks and for the looks of things in
/// it, what it must not show, and the pictures its caller left out for want of room (for the audit
/// line only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRequest {
    prompt: String,
    name: String,
    purpose: Purpose,
    aspect: Aspect,
    base: Option<String>,
    references: Vec<String>,
    things: Vec<ThingReference>,
    avoid: Vec<String>,
    left_out: Vec<String>,
    priority: Priority,
}

impl ImageRequest {
    /// Checks a request. Without a purpose it is a background; without a shape it has its
    /// purpose's default shape.
    pub fn new(
        prompt: &str,
        name: &str,
        purpose: Option<Purpose>,
        aspect: Option<Aspect>,
    ) -> Result<Self, GenerateRequestError> {
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return Err(GenerateRequestError::EmptyPrompt);
        }
        let purpose = purpose.unwrap_or(Purpose::Background);
        let maximum_chars = maximum_prompt_chars(purpose);
        let chars = prompt.chars().count();
        if chars > maximum_chars {
            return Err(GenerateRequestError::PromptTooLong {
                chars,
                maximum_chars,
            });
        }
        if !plain_name(name) {
            return Err(GenerateRequestError::InvalidName {
                name: name.to_owned(),
            });
        }
        Ok(Self {
            prompt: prompt.to_owned(),
            name: name.to_owned(),
            purpose,
            aspect: aspect.unwrap_or_else(|| purpose.default_aspect()),
            base: None,
            references: Vec::new(),
            things: Vec::new(),
            avoid: Vec::new(),
            left_out: Vec::new(),
            priority: Priority::Normal,
        })
    }

    /// This request as an edit of `base`, a picture file of the media library (`bar.png`): the
    /// new picture keeps it and changes what the prompt says.
    pub fn editing(mut self, base: &str) -> Result<Self, GenerateRequestError> {
        if crate::locate::check_picture_name(base).is_err() {
            return Err(GenerateRequestError::InvalidBase {
                base: base.to_owned(),
            });
        }
        self.base = Some(base.to_owned());
        self.check_picture_count()?;
        Ok(self)
    }

    /// This request following `references`, pictures of the media library (`ref-warrior.png`)
    /// that show how the people look, attached in order after the picture to edit.
    pub fn following(mut self, references: &[String]) -> Result<Self, GenerateRequestError> {
        if let Some(reference) = references
            .iter()
            .find(|reference| crate::locate::check_picture_name(reference).is_err())
        {
            return Err(GenerateRequestError::InvalidReference {
                reference: reference.clone(),
            });
        }
        self.references = references.to_vec();
        self.check_picture_count()?;
        Ok(self)
    }

    /// This request following `things`, pictures of the media library that show how things in it
    /// look, attached in order after the people's references; the text names each by its number.
    pub fn showing(mut self, things: &[ThingReference]) -> Result<Self, GenerateRequestError> {
        if let Some(thing) = things
            .iter()
            .find(|thing| crate::locate::check_picture_name(&thing.picture).is_err())
        {
            return Err(GenerateRequestError::InvalidReference {
                reference: thing.picture.clone(),
            });
        }
        self.things = things.to_vec();
        self.check_picture_count()?;
        Ok(self)
    }

    /// This request with what the picture must not show, a few plain words each (`painting`): the
    /// backend's negative prompt when it takes one, otherwise a line of the text.
    pub fn avoiding(mut self, avoid: &[String]) -> Self {
        self.avoid = avoid.to_vec();
        self
    }

    /// This request with the pictures its caller left out for want of room: recorded in the audit
    /// line of the job, never sent.
    pub fn leaving_out(mut self, left_out: &[String]) -> Self {
        self.left_out = left_out.to_vec();
        self
    }

    /// This request with its urgency: an emergency picture is put first in the line of a backend
    /// that has one.
    pub fn prioritised(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    fn check_picture_count(&self) -> Result<(), GenerateRequestError> {
        self.check_pictures(MAXIMUM_PICTURES)
    }

    /// Whether the picture to edit and the references are at most `maximum` pictures.
    pub fn check_pictures(&self, maximum: usize) -> Result<(), GenerateRequestError> {
        let count = usize::from(self.base.is_some()) + self.references.len() + self.things.len();
        if count > maximum {
            return Err(GenerateRequestError::TooManyPictures { count, maximum });
        }
        Ok(())
    }

    /// The pictures of the media library this request follows for the people's looks, in order.
    pub fn references(&self) -> &[String] {
        &self.references
    }

    /// The pictures of the media library this request follows for the looks of things, in order.
    pub fn things(&self) -> &[ThingReference] {
        &self.things
    }

    /// The picture of the media library this request edits, when it is an edit.
    pub fn base(&self) -> Option<&str> {
        self.base.as_deref()
    }

    /// The pictures its caller left out for want of room.
    pub fn left_out(&self) -> &[String] {
        &self.left_out
    }

    pub fn priority(&self) -> Priority {
        self.priority
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn purpose(&self) -> Purpose {
        self.purpose
    }

    pub fn aspect(&self) -> Aspect {
        self.aspect
    }

    /// The text a backend with `limits` is given: the prompt, the people's references' line when
    /// there are any, one line per thing's reference naming its number among the pictures
    /// attached (the picture to edit first), what the picture must not show when the backend
    /// takes no negative prompt, then the purpose's direction.
    pub fn sent_text(&self, limits: &BackendLimits) -> String {
        let mut text = self.prompt.clone();
        if !self.references.is_empty() {
            text.push_str(DIRECTION_SEPARATOR);
            text.push_str(REFERENCES_DIRECTION);
        }
        if !self.things.is_empty() {
            let before = usize::from(self.base.is_some()) + self.references.len();
            let lines: Vec<String> = self
                .things
                .iter()
                .enumerate()
                .map(|(index, thing)| {
                    format!(
                        "Reference {}: {} as it looks.",
                        before + index + 1,
                        thing.shows
                    )
                })
                .collect();
            text.push_str(DIRECTION_SEPARATOR);
            text.push_str(&lines.join(" "));
        }
        if limits.maximum_negative_characters.is_none() && !self.avoid.is_empty() {
            text.push_str(DIRECTION_SEPARATOR);
            text.push_str(AVOID_LEAD);
            text.push_str(&self.avoid.join(", "));
            text.push('.');
        }
        text.push_str(DIRECTION_SEPARATOR);
        text.push_str(self.purpose.direction());
        text
    }

    /// The negative prompt a backend with `limits` is given: what the picture must not show, as
    /// plain terms, when the backend takes a negative prompt and there is something to avoid.
    pub fn negative_text(&self, limits: &BackendLimits) -> Option<String> {
        (limits.maximum_negative_characters.is_some() && !self.avoid.is_empty())
            .then(|| self.avoid.join(", "))
    }
}

/// The longest prompt of `purpose` whose sent text the backend still takes, with references
/// attached (their line is always counted, so a prompt that fits fits either way).
pub fn maximum_prompt_chars(purpose: Purpose) -> usize {
    MAXIMUM_SENT_TEXT_CHARS
        .saturating_sub(DIRECTION_SEPARATOR.chars().count() * 2)
        .saturating_sub(REFERENCES_DIRECTION.chars().count())
        .saturating_sub(purpose.direction().chars().count())
}

/// What a backend is asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendRequest {
    pub text: String,
    /// What the picture must not show, for a backend that takes a negative prompt.
    pub negative: Option<String>,
    pub width_px: u32,
    pub height_px: u32,
    /// The bytes of the picture to edit, when it is an edit.
    pub base: Option<Vec<u8>>,
    /// The bytes of the pictures to follow for the looks of the people, then of the things, in
    /// order.
    pub references: Vec<Vec<u8>>,
    /// How urgent the picture is: a backend with a queue of its own puts an emergency one first.
    pub priority: Priority,
}

/// A picture a backend delivered, with what it can say about how it was made.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedImage {
    pub bytes: Vec<u8>,
    pub mime: String,
    pub width_px: u32,
    pub height_px: u32,
    pub sha256_hex: String,
    pub duration: Duration,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Why the attempt's scratch folder was left behind, when it was. The picture stands.
    pub cleanup: Option<df_imagery::oneshot::CleanupCause>,
}

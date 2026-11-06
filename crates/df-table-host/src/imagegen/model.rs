//! What a picture is asked for with and what a backend hands back: the purpose, the shape, the
//! checked request, the backend's request and the picture it delivered.
//!
//! Nothing here holds a credential, and an audit line never holds the prompt: it holds the
//! prompt's length.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::limits::{
    MAXIMUM_ASPECT_PART, MAXIMUM_ASPECT_RATIO, MAXIMUM_NAME_CHARS, MAXIMUM_SENT_TEXT_CHARS,
    REQUESTED_LONG_SIDE_PX,
};

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
}

impl GenerateRequestError {
    /// The serde name of the variant.
    pub fn key(&self) -> &'static str {
        match self {
            Self::EmptyPrompt => "empty_prompt",
            Self::PromptTooLong { .. } => "prompt_too_long",
            Self::InvalidName { .. } => "invalid_name",
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
        }
    }
}

impl std::error::Error for GenerateRequestError {}

fn plain_name(name: &str) -> bool {
    let count = name.chars().count();
    (1..=MAXIMUM_NAME_CHARS).contains(&count)
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

/// What sits between the prompt and the purpose's direction in the text the backend reads.
const DIRECTION_SEPARATOR: &str = "\n\n";

/// One checked request: a prompt, the name the picture is stored under (without its extension),
/// what it is for and its shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRequest {
    prompt: String,
    name: String,
    purpose: Purpose,
    aspect: Aspect,
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
        })
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

    /// The text the backend is given: the prompt, then the purpose's direction.
    pub fn sent_text(&self) -> String {
        format!(
            "{}{DIRECTION_SEPARATOR}{}",
            self.prompt,
            self.purpose.direction()
        )
    }
}

/// The longest prompt of `purpose` whose sent text the backend still takes.
pub fn maximum_prompt_chars(purpose: Purpose) -> usize {
    MAXIMUM_SENT_TEXT_CHARS
        .saturating_sub(DIRECTION_SEPARATOR.chars().count())
        .saturating_sub(purpose.direction().chars().count())
}

/// What a backend is asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendRequest {
    pub text: String,
    pub width_px: u32,
    pub height_px: u32,
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

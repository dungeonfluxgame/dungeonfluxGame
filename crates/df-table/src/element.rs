//! Typed names, positions and values for things placed on the canvas.
//!
//! It owns the validation of names (and the `name_type!` macro the rest of the crate uses for
//! every registered name: machines, settings, actions, purposes), the geometry of the canvas and
//! the closed set of value shapes. It never knows what a kind means (kinds are data: `kind.rs`)
//! and never reads or writes JSON: the wire form belongs to the host.

use std::collections::BTreeMap;

use crate::limits::MAXIMUM_NAME_LENGTH;
use crate::motion::Motion;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Width of the canvas in canvas units.
pub const CANVAS_WIDTH_UNITS: f32 = 100.0;
/// Height of the canvas in canvas units (a 16 by 9 screen).
pub const CANVAS_HEIGHT_UNITS: f32 = 56.25;

/// Why an identifier was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong,
    MustStartWithLetter,
    InvalidCharacter(char),
}

impl fmt::Display for NameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("a name cannot be empty"),
            Self::TooLong => write!(
                formatter,
                "a name is at most {MAXIMUM_NAME_LENGTH} characters"
            ),
            Self::MustStartWithLetter => formatter.write_str("a name starts with a letter a to z"),
            Self::InvalidCharacter(character) => write!(
                formatter,
                "a name uses only a to z, 0 to 9, '_' and '-', not '{character}'"
            ),
        }
    }
}

impl std::error::Error for NameError {}

pub(crate) fn validate_name(text: &str) -> Result<(), NameError> {
    let first = text.chars().next().ok_or(NameError::Empty)?;
    if !first.is_ascii_lowercase() {
        return Err(NameError::MustStartWithLetter);
    }
    if text.len() > MAXIMUM_NAME_LENGTH {
        return Err(NameError::TooLong);
    }
    match text
        .chars()
        .find(|character| !matches!(character, 'a'..='z' | '0'..='9' | '_' | '-'))
    {
        Some(character) => Err(NameError::InvalidCharacter(character)),
        None => Ok(()),
    }
}

macro_rules! name_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// Accepts a lowercase identifier of letters, digits, '_' and '-' that starts with a
            /// letter. Because only ASCII is allowed, a name is safe in an attribute, a log line
            /// and a URL.
            pub fn new(text: &str) -> Result<Self, $crate::element::NameError> {
                $crate::element::validate_name(text)?;
                Ok(Self(text.to_owned()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: ::serde::Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: ::serde::Deserializer<'de>,
            {
                let text = <String as ::serde::Deserialize>::deserialize(deserializer)?;
                Self::new(&text).map_err(::serde::de::Error::custom)
            }
        }
    };
}

pub(crate) use name_type;

name_type!(
    /// The identity of one element on the canvas.
    ElementId
);
name_type!(
    /// The name of a kind of element, defined by data.
    KindName
);
name_type!(
    /// The name of one field of a kind.
    FieldName
);

/// A point on the canvas, in canvas units: x from 0 to `CANVAS_WIDTH_UNITS`, y from 0 to
/// `CANVAS_HEIGHT_UNITS`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
}

impl Position {
    /// The middle of the canvas.
    pub const CENTER: Self = Self {
        x: CANVAS_WIDTH_UNITS / 2.0,
        y: CANVAS_HEIGHT_UNITS / 2.0,
    };

    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Whether the point is finite and on the canvas, edges included.
    pub fn is_on_canvas(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && (0.0..=CANVAS_WIDTH_UNITS).contains(&self.x)
            && (0.0..=CANVAS_HEIGHT_UNITS).contains(&self.y)
    }
}

/// Largest rotation, in degrees: a full turn is the same as none.
pub const MAXIMUM_ROTATION_DEGREES: u16 = 359;
/// Largest share, in percent.
pub const MAXIMUM_PERCENT: u8 = 100;

/// A clockwise turn in whole degrees, 0 to 359 (180 is upside down). It is the value of an
/// element's rotation component and is checked once, where it is made.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "i64", into = "u16")]
pub struct Degrees(u16);

/// A rotation that is not a whole number of degrees from 0 to 359.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DegreesError(pub i64);

impl fmt::Display for DegreesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "a rotation is a whole number of degrees from 0 to {MAXIMUM_ROTATION_DEGREES} \
             clockwise (180 is upside down), not {}",
            self.0
        )
    }
}

impl Degrees {
    pub fn new(degrees: i64) -> Result<Self, DegreesError> {
        u16::try_from(degrees)
            .ok()
            .filter(|degrees| *degrees <= MAXIMUM_ROTATION_DEGREES)
            .map(Self)
            .ok_or(DegreesError(degrees))
    }

    pub fn get(self) -> u16 {
        self.0
    }
}

impl TryFrom<i64> for Degrees {
    type Error = DegreesError;

    fn try_from(degrees: i64) -> Result<Self, DegreesError> {
        Self::new(degrees)
    }
}

impl From<Degrees> for u16 {
    fn from(degrees: Degrees) -> Self {
        degrees.0
    }
}

impl fmt::Display for Degrees {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// A whole share from 0 to 100 percent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "i64", into = "u8")]
pub struct Percent(u8);

/// A share that is not a whole number of percent from 0 to 100.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PercentError(pub i64);

impl fmt::Display for PercentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "a share is a whole number of percent from 0 to {MAXIMUM_PERCENT}, not {}",
            self.0
        )
    }
}

impl Percent {
    pub fn new(percent: i64) -> Result<Self, PercentError> {
        u8::try_from(percent)
            .ok()
            .filter(|percent| *percent <= MAXIMUM_PERCENT)
            .map(Self)
            .ok_or(PercentError(percent))
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<i64> for Percent {
    type Error = PercentError;

    fn try_from(percent: i64) -> Result<Self, PercentError> {
        Self::new(percent)
    }
}

impl fmt::Display for Percent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl From<Percent> for u8 {
    fn from(percent: Percent) -> Self {
        percent.0
    }
}

/// The point of an element that its rotation turns about, as a share of the element's own width
/// and height from its top left corner: 50 and 50 is the centre, 0 and 50 a hinge on the left edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pivot {
    pub x: Percent,
    pub y: Percent,
}

impl Pivot {
    pub const CENTER: Self = Self {
        x: Percent(MAXIMUM_PERCENT / 2),
        y: Percent(MAXIMUM_PERCENT / 2),
    };
}

impl Default for Pivot {
    fn default() -> Self {
        Self::CENTER
    }
}

/// How an element is drawn, which no rule reads (O.13): a snapshot of its appearance components.
/// An element with none set has the default, which draws it unturned.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Appearance {
    pub rotation: Degrees,
    pub pivot: Pivot,
}

/// A color as red, green and blue bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(u32);

impl Color {
    /// Parses `#rrggbb` (six hexadecimal digits).
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('#')?;
        if digits.len() != 6 || !digits.chars().all(|digit| digit.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(digits, 16).ok().map(Self)
    }

    /// The `#rrggbb` form.
    pub fn to_hex(self) -> String {
        format!("#{:06x}", self.0)
    }
}

/// A field value. The shape is decided by the kind's field type, never guessed from text.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    Text(String),
    /// Always finite: `FieldType::check` refuses anything else before a value is stored.
    Number(f64),
    Flag(bool),
    Color(Color),
}

impl FieldValue {
    /// The text shown for this value in a label or a list.
    pub fn display_text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Number(number) => number.to_string(),
            Self::Flag(flag) => flag.to_string(),
            Self::Color(color) => color.to_hex(),
        }
    }
}

/// One thing on the canvas, as a snapshot of its components (`world.rs`). Its fields are checked
/// against its kind when it is made and changed.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub id: ElementId,
    pub kind: KindName,
    /// Centre of the element.
    pub position: Position,
    pub fields: BTreeMap<FieldName, FieldValue>,
    /// Where the element is heading, while it travels.
    pub motion: Option<Motion>,
    /// How the element is turned, drawn by the viewer and never read by a rule.
    pub appearance: Appearance,
}

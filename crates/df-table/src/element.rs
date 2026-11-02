//! Typed names, positions and values for things placed on the canvas.
//!
//! It owns the validation of names, the geometry of the canvas and the closed set of value
//! shapes. It never knows what a kind means (kinds are data: `kind.rs`) and never reads or writes
//! JSON: the wire form belongs to the host.

use std::collections::BTreeMap;
use std::fmt;

/// Longest identifier accepted for an element, a kind or a field.
pub const MAXIMUM_NAME_LENGTH: usize = 48;
/// Longest text value accepted for a field, in characters.
pub const MAXIMUM_TEXT_LENGTH: usize = 200;
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

fn validate_name(text: &str) -> Result<(), NameError> {
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
            pub fn new(text: &str) -> Result<Self, NameError> {
                validate_name(text)?;
                Ok(Self(text.to_owned()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

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

/// One thing on the canvas. Its fields are checked against its kind when it is made and changed.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub id: ElementId,
    pub kind: KindName,
    /// Centre of the element.
    pub position: Position,
    pub fields: BTreeMap<FieldName, FieldValue>,
}

//! The types a field can have, and the one place that checks a value against its type.
//!
//! It owns the rule for what a valid value is. It never reads a file or a command line: a kind
//! file's defaults arrive as JSON (`from_json`), and the host parses its own text.

use std::fmt;

use crate::element::{Color, FieldValue};

/// Why a value was refused for a field.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueError {
    WrongShape { expected: &'static str },
    TextTooLong { maximum: usize },
    ControlCharacter,
    OutOfRange { minimum: f64, maximum: f64 },
    NotAChoice,
    InvalidColor,
}

impl fmt::Display for ValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongShape { expected } => write!(formatter, "expected {expected}"),
            Self::TextTooLong { maximum } => {
                write!(formatter, "text is at most {maximum} characters")
            }
            Self::ControlCharacter => formatter.write_str("text cannot contain control characters"),
            Self::OutOfRange { minimum, maximum } => {
                write!(formatter, "number must be from {minimum} to {maximum}")
            }
            Self::NotAChoice => formatter.write_str("not one of the listed choices"),
            Self::InvalidColor => formatter.write_str("a color is #rrggbb"),
        }
    }
}

impl std::error::Error for ValueError {}

/// The type of one field.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldType {
    Text { maximum_length: usize },
    Number { minimum: f64, maximum: f64 },
    Flag,
    Color,
    Choice { choices: Vec<String> },
}

impl FieldType {
    /// What this type expects, for an error message.
    pub fn expected(&self) -> &'static str {
        match self {
            Self::Text { .. } => "text",
            Self::Number { .. } => "a number",
            Self::Flag => "true or false",
            Self::Color => "a color #rrggbb",
            Self::Choice { .. } => "one of the listed choices",
        }
    }

    /// Reads a JSON value (a kind file's default) as this type, and checks it.
    pub fn from_json(&self, json: &serde_json::Value) -> Result<FieldValue, ValueError> {
        let wrong = || ValueError::WrongShape {
            expected: self.expected(),
        };
        let value = match self {
            Self::Text { .. } | Self::Choice { .. } => {
                FieldValue::Text(json.as_str().ok_or_else(wrong)?.to_owned())
            }
            Self::Number { .. } => FieldValue::Number(json.as_f64().ok_or_else(wrong)?),
            Self::Flag => FieldValue::Flag(json.as_bool().ok_or_else(wrong)?),
            Self::Color => FieldValue::Color(
                Color::parse(json.as_str().ok_or_else(wrong)?).ok_or(ValueError::InvalidColor)?,
            ),
        };
        self.check(&value)?;
        Ok(value)
    }

    /// Confirms a value has this type and is inside its limits.
    pub fn check(&self, value: &FieldValue) -> Result<(), ValueError> {
        let wrong = || ValueError::WrongShape {
            expected: self.expected(),
        };
        match self {
            Self::Text { maximum_length } => {
                let FieldValue::Text(text) = value else {
                    return Err(wrong());
                };
                if text.chars().any(char::is_control) {
                    return Err(ValueError::ControlCharacter);
                }
                if text.chars().count() > *maximum_length {
                    return Err(ValueError::TextTooLong {
                        maximum: *maximum_length,
                    });
                }
                Ok(())
            }
            Self::Number { minimum, maximum } => {
                let FieldValue::Number(number) = value else {
                    return Err(wrong());
                };
                if number.is_finite() && number >= minimum && number <= maximum {
                    Ok(())
                } else {
                    Err(ValueError::OutOfRange {
                        minimum: *minimum,
                        maximum: *maximum,
                    })
                }
            }
            Self::Flag => matches!(value, FieldValue::Flag(_))
                .then_some(())
                .ok_or_else(wrong),
            Self::Color => matches!(value, FieldValue::Color(_))
                .then_some(())
                .ok_or_else(wrong),
            Self::Choice { choices } => {
                let FieldValue::Text(text) = value else {
                    return Err(wrong());
                };
                if choices.iter().any(|choice| choice == text) {
                    Ok(())
                } else {
                    Err(ValueError::NotAChoice)
                }
            }
        }
    }
}

//! The check of a model name a request asks the text service for.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The longest model name, in bytes. The name goes into a profile label (`responses:<model>`),
/// which `df_ai::text::TextProfile` holds to 128 bytes, so this leaves room for the prefix.
pub const MAXIMUM_MODEL_BYTES: usize = 100;

/// Why a model was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCause {
    /// The name is empty or only blank.
    Blank,
    /// The name is longer than [`MAXIMUM_MODEL_BYTES`].
    TooLong,
    /// The name has a character other than a letter, a digit, `.`, `_`, `:`, `/` or `-`.
    InvalidCharacter,
    /// The name starts with `-`, which a command line would read as an option.
    LeadingDash,
    /// The runtime already serves its limit of distinct models.
    TooManyModels,
}

impl ModelCause {
    pub const ALL: [Self; 5] = [
        Self::Blank,
        Self::TooLong,
        Self::InvalidCharacter,
        Self::LeadingDash,
        Self::TooManyModels,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Blank => "blank",
            Self::TooLong => "too_long",
            Self::InvalidCharacter => "invalid_character",
            Self::LeadingDash => "leading_dash",
            Self::TooManyModels => "too_many_models",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

impl fmt::Display for ModelCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Blank => formatter.write_str("the model name is blank"),
            Self::TooLong => write!(
                formatter,
                "the model name is longer than {MAXIMUM_MODEL_BYTES} bytes"
            ),
            Self::InvalidCharacter => formatter.write_str(
                "the model name may have only letters, digits, '.', '_', ':', '/' and '-'",
            ),
            Self::LeadingDash => formatter.write_str("the model name may not start with '-'"),
            Self::TooManyModels => {
                formatter.write_str("the text service already serves its limit of distinct models")
            }
        }
    }
}

/// `model` when it is a usable model name: 1 to [`MAXIMUM_MODEL_BYTES`] letters, digits, `.`,
/// `_`, `:`, `/` or `-`, not starting with `-`. A dot is fine (`gpt-6.1-sol`).
pub fn check_model(model: &str) -> Result<&str, ModelCause> {
    if model.trim().is_empty() {
        Err(ModelCause::Blank)
    } else if model.len() > MAXIMUM_MODEL_BYTES {
        Err(ModelCause::TooLong)
    } else if model.starts_with('-') {
        Err(ModelCause::LeadingDash)
    } else if !model
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || ".:/_-".contains(character))
    {
        Err(ModelCause::InvalidCharacter)
    } else {
        Ok(model)
    }
}

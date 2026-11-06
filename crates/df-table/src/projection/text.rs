//! The text a projection shows, which is never blank (H.15).
//!
//! It owns `NonEmptyText`, the only type the narration, caption and place slots of a view accept,
//! so a blank slot cannot be built (O.7), and `or_readout`, which puts the structured readout
//! where a model's narration is blank (C.7). Its `Debug` says only a length class, like every
//! other text that may belong to one audience. It never judges what the text means: that is for
//! whoever wrote it.

use std::borrow::Cow;
use std::fmt;

use serde::Serialize;

use crate::machine::MachineReadout;
use crate::private_data::{LengthClass, Measured, Narration};

/// A blank text was offered where a view needs words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlankTextError;

impl fmt::Display for BlankTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a projection text cannot be blank")
    }
}

impl std::error::Error for BlankTextError {}

/// Text with at least one character that is not white space.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct NonEmptyText(Cow<'static, str>);

impl NonEmptyText {
    /// What stands in when generated text could not be turned into words.
    const FALLBACK: &'static str = "Something happened.";

    pub fn new(text: &str) -> Result<Self, BlankTextError> {
        if text.trim().is_empty() {
            return Err(BlankTextError);
        }
        Ok(Self(Cow::Owned(text.to_owned())))
    }

    /// The text, or the plain fallback sentence if it is blank: for words built from typed fields
    /// whose template always has some, so the fallback is never seen in practice.
    pub fn or_fallback(text: &str) -> Self {
        Self::new(text).unwrap_or(Self(Cow::Borrowed(Self::FALLBACK)))
    }

    /// The model's narration, or the structured readout when the narration is blank (C.7, H.15).
    pub fn narration_or_readout(narration: &Narration, readout: &Self) -> Self {
        Self::new(narration.expose()).unwrap_or_else(|_| readout.clone())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&MachineReadout> for NonEmptyText {
    fn from(readout: &MachineReadout) -> Self {
        Self(Cow::Owned(readout.as_str().to_owned()))
    }
}

impl fmt::Debug for NonEmptyText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let length = LengthClass::of_length(self.0.length_in_chars());
        write!(formatter, "NonEmptyText(length={})", length.key())
    }
}

impl Measured for NonEmptyText {
    fn length_in_chars(&self) -> usize {
        self.0.chars().count()
    }
}

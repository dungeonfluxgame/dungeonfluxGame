//! Text and facts that must never reach a log, an event or a prompt by accident.
//!
//! It owns `PlayerText` (what a player typed), `Narration` (what the storyteller wrote) and
//! `Secret<T>` (a fact an audience may not know), plus the `LengthClass` that is all their `Debug`
//! output says. None of them implements `Display` or serde, and none implements `EventField`
//! (`event::field`), so none can be written into a `TableEvent`: records carry an opaque
//! evidence reference instead (RES36). It never decides who may read a value: a caller that holds
//! one calls `expose` on purpose, and a reviewer can search for that word.
//!
//! The first example does not compile because a private type has no `Display`:
//!
//! ```compile_fail,E0277
//! use df_table::private_data::PlayerText;
//!
//! let line = PlayerText::new("an attacker's words");
//! let _ = format!("{line}");
//! ```
//!
//! The second does not compile because it is not an event field:
//!
//! ```compile_fail,E0277
//! use df_table::event::field::EventField;
//! use df_table::private_data::Narration;
//!
//! fn must_be_an_event_field<T: EventField>(_: &T) {}
//! must_be_an_event_field(&Narration::new("the storyteller's words"));
//! ```
//!
//! The third does not compile because a `TableEvent` field has a fixed type:
//!
//! ```compile_fail,E0308
//! use df_table::event::TableEvent;
//! use df_table::identity::{Correlation, OperationId, RecoveryEpoch, SessionRevision};
//! use df_table::identity::{TableId, TableScope, TurnSeq};
//! use df_table::private_data::PlayerText;
//!
//! let epoch = RecoveryEpoch::new(1).unwrap();
//! let correlation = Correlation {
//!     operation_id: OperationId::from_bytes(&[1; 16]).unwrap(),
//!     turn_seq: TurnSeq::new(1),
//!     revision: SessionRevision::new(epoch, 1),
//! };
//! let _ = TableEvent::InputDeclined {
//!     scope: TableScope::of_table(TableId::new(1)),
//!     correlation,
//!     code: PlayerText::new("an attacker's words"),
//! };
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};

/// Up to this many characters a value is called short. A class boundary, not a limit: nothing is
/// ever refused for its length here.
pub(crate) const SHORT_TEXT_MAXIMUM_CHARS: usize = 40;
/// Up to this many characters a value is called medium (a full phone line, C.7). A class
/// boundary, not a limit.
pub(crate) const MEDIUM_TEXT_MAXIMUM_CHARS: usize = 280;

/// How long a private value is, which is everything a log may say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LengthClass {
    Empty,
    /// 1 to `SHORT_TEXT_MAXIMUM_CHARS` characters.
    Short,
    /// Up to `MEDIUM_TEXT_MAXIMUM_CHARS` characters.
    Medium,
    Long,
}

impl LengthClass {
    pub fn of_length(characters: usize) -> Self {
        if characters == 0 {
            Self::Empty
        } else if characters <= SHORT_TEXT_MAXIMUM_CHARS {
            Self::Short
        } else if characters <= MEDIUM_TEXT_MAXIMUM_CHARS {
            Self::Medium
        } else {
            Self::Long
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Short => "short",
            Self::Medium => "medium",
            Self::Long => "long",
        }
    }
}

/// A value whose size can be classed without showing it.
pub(crate) trait Measured {
    fn length_in_chars(&self) -> usize;
}

impl Measured for str {
    fn length_in_chars(&self) -> usize {
        self.chars().count()
    }
}

impl Measured for String {
    fn length_in_chars(&self) -> usize {
        self.as_str().length_in_chars()
    }
}

impl<T: Measured + ?Sized> Measured for &T {
    fn length_in_chars(&self) -> usize {
        (**self).length_in_chars()
    }
}

fn write_debug(
    formatter: &mut fmt::Formatter<'_>,
    type_name: &str,
    value: &(impl Measured + ?Sized),
) -> fmt::Result {
    let length = LengthClass::of_length(value.length_in_chars());
    write!(formatter, "{type_name}(length={})", length.key())
}

/// What a player typed. Untrusted data in every prompt, and never part of a record.
#[derive(Clone, PartialEq, Eq)]
pub struct PlayerText(String);

impl PlayerText {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PlayerText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_debug(formatter, "PlayerText", &self.0)
    }
}

/// What the storyteller wrote about a committed outcome. Text from a model, never a record field.
#[derive(Clone, PartialEq, Eq)]
pub struct Narration(String);

impl Narration {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Narration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_debug(formatter, "Narration", &self.0)
    }
}

/// A fact that only some audiences may know.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T: Measured> fmt::Debug for Secret<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_debug(formatter, "Secret", &self.0)
    }
}

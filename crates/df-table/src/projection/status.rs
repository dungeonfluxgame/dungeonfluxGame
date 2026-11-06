//! What a projection says about the table's state: the nine statuses of H.15.
//!
//! It owns `ProjectionStatus` (`ok`, `reading`, `rolling`, `telling`, `art_delayed`, `degraded`,
//! `read_only`, `reconnecting` and `waiting_for{who, what}`, the nine keys of `STATUS_KEYS` in the
//! playground's `copy.rs`) and `StatusKind`, its payload-free form for events and counters. It
//! serializes as the playground's `Status`: an object with a `kind` key and, for `waiting_for`,
//! `who` and `what`. Every status other than `ok` has one plain sentence naming the cause, and the
//! words that name a person are never blank (K.5). It never decides which status holds: the
//! pipeline's activity arrives as an attachment, and `projection` adds the pause.

use serde::{Deserialize, Serialize};

use crate::listed::listed_enum;
use crate::projection::text::{BlankTextError, NonEmptyText};

/// What the table is doing or waiting for, as a view says it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProjectionStatus {
    Ok,
    Reading,
    Rolling,
    Telling,
    ArtDelayed,
    Degraded,
    ReadOnly,
    Reconnecting,
    WaitingFor {
        who: NonEmptyText,
        what: NonEmptyText,
    },
}

impl ProjectionStatus {
    /// A wait on a person, named in words: "Waiting for {who} {what}".
    pub fn waiting_for(who: &str, what: &str) -> Result<Self, BlankTextError> {
        Ok(Self::WaitingFor {
            who: NonEmptyText::new(who)?,
            what: NonEmptyText::new(what)?,
        })
    }

    /// The plain sentence that says why the status is not `ok`, in the words of the playground's
    /// `status_copy` (K.5), or `None` for `ok`. The wire view carries it as `reason`, present
    /// exactly when the status is not ok.
    pub fn reason(&self) -> Option<NonEmptyText> {
        let sentence = match self {
            Self::Ok => return None,
            Self::Reading => "Reading your move".to_owned(),
            Self::Rolling => "Rolling".to_owned(),
            Self::Telling => "Telling it".to_owned(),
            Self::ArtDelayed => "Picture catching up".to_owned(),
            Self::Degraded => "Some pictures are unavailable. Play continues.".to_owned(),
            Self::ReadOnly => "Could not save. Try again.".to_owned(),
            Self::Reconnecting => "Reconnecting".to_owned(),
            Self::WaitingFor { who, what } => {
                format!("Waiting for {} {}", who.as_str(), what.as_str())
            }
        };
        Some(NonEmptyText::or_fallback(&sentence))
    }

    pub fn kind(&self) -> StatusKind {
        match self {
            Self::Ok => StatusKind::Ok,
            Self::Reading => StatusKind::Reading,
            Self::Rolling => StatusKind::Rolling,
            Self::Telling => StatusKind::Telling,
            Self::ArtDelayed => StatusKind::ArtDelayed,
            Self::Degraded => StatusKind::Degraded,
            Self::ReadOnly => StatusKind::ReadOnly,
            Self::Reconnecting => StatusKind::Reconnecting,
            Self::WaitingFor { .. } => StatusKind::WaitingFor,
        }
    }
}

listed_enum! {
    /// The status without its words, for an event line.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum StatusKind {
        Ok,
        Reading,
        Rolling,
        Telling,
        ArtDelayed,
        Degraded,
        ReadOnly,
        Reconnecting,
        WaitingFor,
    }
}

impl StatusKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Reading => "reading",
            Self::Rolling => "rolling",
            Self::Telling => "telling",
            Self::ArtDelayed => "art_delayed",
            Self::Degraded => "degraded",
            Self::ReadOnly => "read_only",
            Self::Reconnecting => "reconnecting",
            Self::WaitingFor => "waiting_for",
        }
    }
}

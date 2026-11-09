//! The quest journal (component 12): an open parchment book, not glass. The left page lists the
//! quests with gold diamond markers, the current one in a glowing rim; the right page holds the
//! chosen quest's title, body and ink sketch, with a red ribbon down the gutter (renders 20, 36).
//!
//! The entries and the "+N" line are the host's: past the page's room the host pages the list,
//! sending at most `JOURNAL_ROWS_AT_MOST` entries with its "+N" line. Rows sent past the page's
//! room without such a line are counted ("+4"), so no quest vanishes unannounced. The page shows
//! whole rows only, never a row cut at the page's foot. The right page's body is always shown
//! whole: the sketch under it gives way to it, which the stylesheet decides from the laid-out
//! page, never from a letter count.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::home::glyph::HomeIcon;
use crate::kit::shown;

/// The heading of the left page when the journal names none.
pub const DEFAULT_JOURNAL_TITLE: &str = "Quests";

/// The most entries the left page draws: six whole rows, a long title wrapping to two lines, fit
/// over the section label (render 36). Rows past them are the host's to page.
pub const JOURNAL_ROWS_AT_MOST: usize = 6;

/// Where a quest stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestState {
    #[default]
    Active,
    /// The quest the right page shows, in the glowing rim.
    Current,
    Done,
    Failed,
}

impl QuestState {
    /// The word that ends the CSS class, the same as the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Current => "current",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

/// One quest on the left page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestEntry {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub state: QuestState,
    /// A drawing in the diamond instead of the plain gold mark.
    #[serde(default)]
    pub marker: Option<HomeIcon>,
}

/// The right page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestDetail {
    pub title: String,
    pub body: String,
    /// An ink sketch from the media library.
    #[serde(default)]
    pub sketch: Option<String>,
}

/// The journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    #[serde(default)]
    pub title: Option<String>,
    pub entries: Vec<QuestEntry>,
    /// The host's line for entries it did not send, such as "+3 more".
    #[serde(default)]
    pub more: Option<String>,
    #[serde(default)]
    pub detail: Option<QuestDetail>,
}

impl Journal {
    pub fn title(&self) -> &str {
        shown(self.title.as_deref()).unwrap_or(DEFAULT_JOURNAL_TITLE)
    }

    /// The entries that have a title, in the host's order, at most `JOURNAL_ROWS_AT_MOST`.
    pub fn entries(&self) -> Vec<&QuestEntry> {
        self.titled().take(JOURNAL_ROWS_AT_MOST).collect()
    }

    fn titled(&self) -> impl Iterator<Item = &QuestEntry> {
        self.entries
            .iter()
            .filter(|entry| shown(Some(&entry.title)).is_some())
    }

    /// The host's line for the entries past the page, or, when the host sent more titled entries
    /// than the page draws and no line, the count of those it does not draw ("+4").
    pub fn more(&self) -> Option<Cow<'_, str>> {
        if let Some(line) = shown(self.more.as_deref()) {
            return Some(Cow::Borrowed(line));
        }
        let hidden = self.titled().count().saturating_sub(JOURNAL_ROWS_AT_MOST);
        (hidden > 0).then(|| Cow::Owned(format!("+{hidden}")))
    }

    /// Whether the book is drawn: it has an entry or a page of detail.
    pub fn is_drawn(&self) -> bool {
        !self.entries().is_empty() || self.detail().is_some()
    }

    /// The right page, when it has a title or a body.
    pub fn detail(&self) -> Option<&QuestDetail> {
        self.detail.as_ref().filter(|detail| {
            shown(Some(&detail.title)).is_some() || shown(Some(&detail.body)).is_some()
        })
    }
}

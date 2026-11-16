//! The kit gallery, a development tool only: boards of sample panels a reviewer opens on the
//! shared screen to see every component at once, each board after a concept render.
//!
//! The boards are `gallery.json` beside this file. They are compiled into a development build
//! only (`debug_assertions`); a release build has neither the boards nor this module. A page of a
//! development build shows board `N` as its whole stage when a reviewer opens it with `?kit=N`
//! (counted from 1), over the quiet `ui_kit_gallery` scene; without the query it draws no board.
//! The boards are sample data, not a scene the host plays, so their speech lines are never a
//! fixture's. Until `TvScene` has a `kit` field the host fills (inventory F8), this is the only
//! way kit panels reach the screen.

use serde::{Deserialize, Serialize};

use crate::gallery::{Gallery, GalleryError, board_number as number_in};
use crate::kit::KitPanels;

/// The query parameter that names the board.
pub const GALLERY_QUERY_KEY: &str = "kit";

const GALLERY_TEXT: &str = include_str!("gallery.json");

/// One board.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KitBoard {
    pub name: String,
    /// The concept renders the board follows.
    #[serde(default)]
    pub after: Option<String>,
    /// A picture from the media library drawn full-bleed under the panels.
    #[serde(default)]
    pub plate: Option<String>,
    pub panels: KitPanels,
}

/// Every board.
pub type KitGallery = Gallery<KitBoard>;

impl Gallery<KitBoard> {
    /// The compiled-in boards.
    pub fn read() -> Result<Self, GalleryError> {
        Self::parse(GALLERY_TEXT)
    }
}

/// The board number a page address's query names (`?kit=3`), or `None` when it names none.
pub fn board_number(query: &str) -> Option<usize> {
    number_in(query, GALLERY_QUERY_KEY)
}

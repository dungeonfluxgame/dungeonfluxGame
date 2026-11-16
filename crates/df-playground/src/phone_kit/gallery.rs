//! The phone gallery, a development tool only: boards of sample screens a reviewer opens on the
//! phone page with `?phone_kit=N` (counted from 1) to see every component of the phone kit.
//!
//! The boards are `gallery.json` beside this file, compiled into a development build only
//! (`debug_assertions`). They are sample data, not a scene the host plays.

use serde::{Deserialize, Serialize};

use crate::gallery::{Gallery, GalleryError, board_number as number_in};
use crate::phone_kit::PhoneScreen;

/// The query parameter that names the board.
pub const GALLERY_QUERY_KEY: &str = "phone_kit";

const GALLERY_TEXT: &str = include_str!("gallery.json");

/// One board: the screens that show one family of components.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhoneBoard {
    pub name: String,
    /// What the board follows in the concept art.
    #[serde(default)]
    pub after: Option<String>,
    pub screens: Vec<PhoneScreen>,
}

/// Every board.
pub type PhoneGallery = Gallery<PhoneBoard>;

impl Gallery<PhoneBoard> {
    /// The compiled-in boards.
    pub fn read() -> Result<Self, GalleryError> {
        Self::parse(GALLERY_TEXT)
    }
}

/// The board number a page address's query names (`?phone_kit=3`), or `None` when it names none.
pub fn board_number(query: &str) -> Option<usize> {
    number_in(query, GALLERY_QUERY_KEY)
}

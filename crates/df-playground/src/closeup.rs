//! The title card of a clue close-up: the item's name over its words, as in the concept art
//! (`assets/concept-art/01-fullscreen-sword-inspection.webp`).
//!
//! A close-up is a backdrop picture the host put up as one (`PlateKind::CloseUp`), shown with the
//! room's layout so the cast may still speak. The caption such a scene carries is the clue's
//! words, and the page draws it as this card, not as the caption bar, so the item stays clear. How
//! the card is split into title and description is decided here; the page only draws it
//! (`kit::title_card`). The card's show and hide are recorded by the timeline as
//! the caption's line (`TimelineKind::Line`).

use crate::fixture::TvScene;

/// The title card of a close-up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseUp {
    /// The item's name; none when the host sent no name, and the description stands alone.
    pub title: Option<String>,
    pub description: String,
}

impl CloseUp {
    /// Says whether a later draw shows the same card again, so it does not fade in a second time.
    pub fn card_identity(&self) -> String {
        format!(
            "{}\n{}",
            self.title.as_deref().unwrap_or_default(),
            self.description
        )
    }
}

/// The title card `scene` shows, when its backdrop is a close-up and its roleplay card has a
/// caption.
pub fn scene_closeup(scene: &TvScene) -> Option<CloseUp> {
    if !scene.is_close_up() {
        return None;
    }
    let roleplay = scene.roleplay.as_ref()?;
    let description = roleplay.caption.as_deref()?.trim();
    if description.is_empty() {
        return None;
    }
    let title = roleplay
        .caption_title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_owned);
    Some(CloseUp {
        title,
        description: description.to_owned(),
    })
}

//! The story's mark at the top right of the shared screen: the title chip (`kit::title_chip`)
//! with the progress line (`kit::progress_line`) under it, showing the story's title and the place
//! the party has reached.
//!
//! The host words both (`TvScene::story`); this only mounts the kit's drawings. The mark is its
//! own piece of the screen (`Part::TvStory`), redrawn only when it changes, and it fades out while
//! it would cover a face, a bubble, the narration, the caption bar or the clue board
//! (`view::bubble_layout`).

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::fixture::TvScene;
use crate::kit::KitPanels;
use crate::kit::progress_line::ProgressLine;
use crate::kit::title_chip::TitleChip;

/// The mark of `scene`, or `None` when the scene has no story.
pub fn mark(document: &Document, scene: &TvScene) -> Result<Option<Element>, JsValue> {
    let Some(story) = &scene.story else {
        return Ok(None);
    };
    let layer = crate::kit::draw::panels(
        document,
        &KitPanels {
            chips: vec![TitleChip::from(story)],
            ..KitPanels::default()
        },
        None,
    )?;
    layer.set_attribute("class", "kit tv-story")?;
    layer.set_attribute("aria-label", "The story")?;
    if let Some(line) = ProgressLine::of_story(story)
        && let Some(drawn) = crate::kit::draw::progress_line::draw(document, &line)?
        && let Some(right) = layer.query_selector(".kit-top-right")?
    {
        crate::dom::append(&right, &drawn)?;
    }
    Ok(Some(layer))
}

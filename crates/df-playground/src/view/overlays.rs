//! The overlay layer of the shared screen: one slot, above the world layer and outside the
//! camera's transform, where the panels are drawn.
//!
//! The stage's fixed layers are backdrop, grid, figures, card, then this layer, then the place
//! name, status, caption and party strip (O.13 rule 12). A panel is laid out inside the layer's
//! safe area, which stops above the caption and the strip, so no panel is ever drawn over them.
//! This module only puts each component's element in the layer; the components are
//! `view::lobby` and `view::roleplay`, each a function of the scene's data.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element};
use crate::fixture::TvScene;

/// The overlay layer with the panels the scene has, or `None` when it has none.
pub fn panels(document: &Document, scene: &TvScene) -> Result<Option<Element>, JsValue> {
    let layer = element(document, "div", "tv-panels")?;
    let mut any = false;
    for panel in [
        crate::view::lobby::build(document, scene)?,
        crate::view::roleplay::build(document, scene)?,
        crate::view::home::build(document, scene)?,
    ]
    .into_iter()
    .flatten()
    {
        append(&layer, &panel)?;
        any = true;
    }
    Ok(any.then_some(layer))
}

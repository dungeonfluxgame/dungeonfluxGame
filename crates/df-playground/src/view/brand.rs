//! The top left of the shared screen: the DungeonFlux brand mark with the place name under it when
//! the scene asks for the logo (otherwise the place name alone), and under that the scene's title
//! banner.
//!
//! The mark and the banner are the kit's (`kit::brand`, `kit::banner`); the place name alone stays
//! a plain label until the kit's place label replaces it. The column is laid out by the page's
//! heading (`.tv-heading`), which also holds the clue board, so the board lies under whatever this
//! piece draws and never over it. Nothing here moves with the camera.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::fixture::TvScene;
use crate::kit::KitPanels;
use crate::kit::banner::Banner;
use crate::kit::brand::BrandMark;

/// The place and title column of `scene`: the brand mark with the place name when the scene has
/// `brand` set or its top bar asks for the logo, otherwise the place name alone, then the roleplay
/// card's title banner.
pub fn place(document: &Document, scene: &TvScene) -> Result<Element, JsValue> {
    let column = element(document, "div", "tv-place-column")?;
    // The landing page's top bar (`view::home::chrome::top`) draws its own logo.
    let has_home_top = scene.home.as_ref().is_some_and(|home| home.top.is_some());
    let logo = scene
        .pills
        .as_ref()
        .and_then(|bar| bar.logo)
        .filter(|_| !has_home_top);
    let brand = BrandMark::for_scene(scene.brand, logo, &scene.place);
    let banners: Vec<Banner> = scene
        .roleplay
        .as_ref()
        .and_then(|card| card.title.as_ref())
        .map(Banner::from)
        .into_iter()
        .collect();
    if brand.is_none() {
        append(
            &column,
            &text_element(document, "div", "tv-place", &scene.place)?,
        )?;
    }
    if brand.is_some() || !banners.is_empty() {
        append(
            &column,
            &crate::kit::draw::panels(
                document,
                &KitPanels {
                    brand,
                    banners,
                    ..KitPanels::default()
                },
                None,
            )?,
        )?;
    }
    Ok(column)
}

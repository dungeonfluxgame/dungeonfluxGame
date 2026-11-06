//! The landing page's layer on the shared screen: one box over the whole stage in which each
//! component of the page sits in its own slot.
//!
//! This module only places each component's element in its slot; the components are
//! `view::home::chrome` (brand, title, navigation, top bar), `view::home::actions`,
//! `view::home::configure`, `view::home::adventure` and `view::home::campaigns`. A slot is a box
//! with the share of the screen render 19 gives the component (the CSS block `component: home`);
//! the component fills it. A part the scene does not name leaves its slot out.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

pub mod actions;
pub mod adventure;
pub mod campaigns;
pub mod chrome;
pub mod configure;
pub mod kit;

use crate::dom::{append, element};
use crate::fixture::TvScene;

/// A slot named `name` holding `content`, put in `layer`.
fn put(document: &Document, layer: &Element, name: &str, content: &Element) -> Result<(), JsValue> {
    let slot = element(document, "div", &format!("hm-slot hm-slot-{name}"))?;
    append(&slot, content)?;
    append(layer, &slot)
}

/// Gives the one element whose `data-focus-id` is `id` the focus class. An id the page has no
/// element for leaves nothing focused; the id was checked to hold no character that needs quoting.
fn mark_focus(layer: &Element, id: Option<&str>) -> Result<(), JsValue> {
    let Some(id) = id.filter(|id| crate::home::model::is_focus_id(id)) else {
        return Ok(());
    };
    if let Some(target) = layer.query_selector(&format!("[data-focus-id=\"{id}\"]"))? {
        let class = target.get_attribute("class").unwrap_or_default();
        target.set_attribute("class", &format!("{class} hm-focused"))?;
    }
    Ok(())
}

/// The landing page of `scene`, or `None` when it has none.
pub fn build(document: &Document, scene: &TvScene) -> Result<Option<Element>, JsValue> {
    let Some(home) = &scene.home else {
        return Ok(None);
    };
    let layer = element(document, "div", "hm")?;
    if let Some(brand) = &home.brand {
        put(
            document,
            &layer,
            "brand",
            &crate::view::home::chrome::brand(document, brand)?,
        )?;
    }
    if let Some(top) = &home.top {
        put(
            document,
            &layer,
            "top",
            &crate::view::home::chrome::top(document, top, scene)?,
        )?;
    }
    if let Some(title) = &home.title {
        put(
            document,
            &layer,
            "title",
            &crate::view::home::chrome::title(document, title)?,
        )?;
    }
    if let Some(nav) = &home.nav {
        put(
            document,
            &layer,
            "nav",
            &crate::view::home::chrome::nav(document, nav)?,
        )?;
    }
    if let Some(actions) = &home.actions {
        put(
            document,
            &layer,
            "actions",
            &crate::view::home::actions::actions(document, actions)?,
        )?;
    }
    if let Some(data) = &home.configure {
        put(
            document,
            &layer,
            "configure",
            &crate::view::home::configure::build(document, data)?,
        )?;
    }
    if let Some(data) = &home.adventure {
        put(
            document,
            &layer,
            "adventure",
            &crate::view::home::adventure::build(document, data)?,
        )?;
    }
    if let Some(data) = &home.campaigns {
        put(
            document,
            &layer,
            "campaigns",
            &crate::view::home::campaigns::build(document, data)?,
        )?;
    }
    mark_focus(&layer, home.focus_id().as_deref())?;
    Ok(Some(layer))
}

//! The overlay layer of the shared screen: one slot, above the world layer and outside the
//! camera's transform, where the panels are drawn.
//!
//! The stage's fixed layers are backdrop, grid, figures, card, then this layer, then the place
//! name, status, caption and party strip (O.13 rule 12). A panel is laid out inside the layer's
//! safe area, which stops above the caption and the strip, so no panel is ever drawn over them.
//! This module only puts each component's element in the layer; the components are
//! `view::lobby` and `view::roleplay`, each a function of the scene's data. It also keeps the
//! kit cards that leave (`carry_over`).

use wasm_bindgen::{JsCast, JsValue};
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
        crate::kit::draw::gallery(document)?,
    ]
    .into_iter()
    .flatten()
    {
        append(&layer, &panel)?;
        any = true;
    }
    Ok(any.then_some(layer))
}

/// A kit card that fades out when the scene stops drawing it, and how the page keeps it until then.
struct Leaver {
    /// The card on screen: not a copy already leaving, not a hidden twin.
    live: &'static str,
    /// The attribute that holds the card's key, which says whether a later draw shows it again.
    key: &'static str,
    /// The class of a card shown again, which does not fade in a second time.
    settled: &'static str,
    /// The class of the copy that fades out.
    out: &'static str,
    /// The class of the kit's stack the copy joins at the top, or `None` to join the kit layer.
    stack: Option<&'static str>,
}

/// The off-picture speaker's card (`kit::speaker`).
const SPEAKER: Leaver = Leaver {
    live: ".kit-speaker:not(.kit-speaker-out):not(.kit-speaker-twin)",
    key: "data-card",
    settled: "kit-speaker-settled",
    out: "kit-speaker-out",
    stack: Some("kit-bottom"),
};

/// The title card of a close-up (`kit::title_card`).
const TITLE: Leaver = Leaver {
    live: ".kit-tc:not(.kit-tc-out)",
    key: "data-closeup",
    settled: "kit-tc-settled",
    out: "kit-tc-out",
    stack: None,
};

/// Settles a draw of the panels layer against the speaker card and the close-up title card on
/// screen in `slot`, and returns what to put in the slot. A card shown again is marked settled so
/// it does not fade in a second time; a card that is gone or replaced stays as a copy that fades
/// out, over the new one, so a card never cuts.
pub fn carry_over(
    document: &Document,
    slot: &Element,
    built: Option<Element>,
) -> Result<Option<Element>, JsValue> {
    let built = carry(document, slot, built, &SPEAKER)?;
    carry(document, slot, built, &TITLE)
}

fn carry(
    document: &Document,
    slot: &Element,
    built: Option<Element>,
    card: &Leaver,
) -> Result<Option<Element>, JsValue> {
    let Some(old) = slot.query_selector(card.live)? else {
        return Ok(built);
    };
    let old_key = old.get_attribute(card.key);
    let new = match &built {
        Some(layer) => layer.query_selector(card.live)?,
        None => None,
    };
    if let Some(new) = &new
        && new.get_attribute(card.key) == old_key
    {
        new.set_attribute("class", &format!("{} {}", new.class_name(), card.settled))?;
        return Ok(built);
    }
    let leaving = old
        .clone_node_with_deep(true)?
        .dyn_into::<Element>()
        .map_err(|_| JsValue::from_str("the copy of the card is not an element"))?;
    leaving.set_attribute("class", &format!("{} {}", old.class_name(), card.out))?;
    leaving.set_attribute("aria-hidden", "true")?;
    leaving.remove_attribute("role")?;
    let layer = match built {
        Some(layer) => layer,
        None => element(document, "div", "tv-panels")?,
    };
    let kit = match layer.query_selector(".kit")? {
        Some(kit) => kit,
        None => {
            let kit = element(document, "div", "kit")?;
            match layer.query_selector(".rp")? {
                Some(panel) => append(&panel, &kit)?,
                None => append(&layer, &kit)?,
            }
            kit
        }
    };
    let Some(stack_class) = card.stack else {
        append(&kit, &leaving)?;
        return Ok(Some(layer));
    };
    let stack = match kit.query_selector(&format!(".{stack_class}"))? {
        Some(stack) => stack,
        None => {
            let stack = element(document, "div", stack_class)?;
            append(&kit, &stack)?;
            stack
        }
    };
    stack.insert_before(&leaving, stack.first_child().as_ref())?;
    Ok(Some(layer))
}

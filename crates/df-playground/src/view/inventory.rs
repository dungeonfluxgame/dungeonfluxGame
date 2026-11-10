//! The inventory panel at the lower right of the shared screen: the kit's list (`kit::ledger`) of
//! what one owner holds, with a supply that ran out struck through and each recent change flashed.
//!
//! The host builds the rows and the moves (`TvScene::inventory`); this only mounts the kit's
//! drawing and puts each row's look on it (`InventoryPanel::look_of`). The panel is its own piece
//! of the screen (`Part::TvInventory`), redrawn only when it changes. A move flashes for
//! `FLASH_MS` from when the page first showed it (`FlashClock`): a panel drawn again carries each
//! flash on from where it was (`--inv-age`), a newer push does not clear the flashes of an older
//! one, and a move that arrives while a full-frame picture hides the panel is shown for the whole
//! time once the panel returns (`settle_cover` asks for the panel to be drawn again then). A panel
//! drawn again for the same owner keeps its glass still (`inv-redrawn`). It is anchored to the
//! stage, not to the picture, and fades out while it would cover a face, a bubble or another
//! panel (`view::bubble_layout`).

use std::cell::{Cell, RefCell};

use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element};

use crate::controls::Controls;
use crate::dom::{append, element, set_style, text_element};
use crate::fixture::TvScene;
use crate::inventory::{FlashClock, Flashing, InventoryPanel, picture_is_up};
use crate::kit::draw::ledger;
use crate::page_log::now_ms;

/// The class that fades the panel out while a full-frame picture is up.
const COVERED: &str = "inventory-covered";

thread_local! {
    /// The owner of the panel drawn last, or none while no panel is shown.
    static DRAWN: RefCell<Option<String>> = const { RefCell::new(None) };
    /// When the page first showed each move.
    static CLOCK: RefCell<FlashClock> = RefCell::new(FlashClock::default());
    /// Whether the panel was faded out for a full-frame picture when last settled.
    static COVER: Cell<bool> = const { Cell::new(false) };
}

/// The panel of `scene`, or `None` when the scene has none.
pub fn panel(
    document: &Document,
    scene: &TvScene,
    controls: &Controls,
) -> Result<Option<Element>, JsValue> {
    let Some(inventory) = &scene.inventory else {
        DRAWN.with(|drawn| drawn.borrow_mut().take());
        CLOCK.with(|clock| clock.take());
        return Ok(None);
    };
    // A panel drawn again for the same owner does not fade its glass in a second time.
    let before = DRAWN.with(|drawn| drawn.borrow_mut().replace(inventory.owner.clone()));
    let redrawn = before.as_ref() == Some(&inventory.owner);
    let covered = picture_is_up(
        &controls.plate,
        scene.backdrop.as_ref().map(|image| image.plate),
    );
    let returning = COVER.with(|cover| cover.replace(covered)) && !covered;
    let flashing = CLOCK.with(|clock| clock.borrow_mut().flashing(inventory, now_ms(), !covered));
    let mut class = String::from("kit tv-inventory");
    if redrawn {
        class.push_str(" inv-redrawn");
    }
    if covered {
        class.push(' ');
        class.push_str(COVERED);
    }
    if returning {
        class.push_str(" inv-return");
    }
    let layer = element(document, "div", &class)?;
    layer.set_attribute("aria-label", "What is held")?;
    let list = inventory.ledger();
    if let Some(drawn) = ledger::draw(document, &list)? {
        look(document, inventory, &flashing, &drawn)?;
        append(&layer, &drawn)?;
    }
    Ok(Some(layer))
}

/// Puts the look of each row of `inventory` on its element in `drawn`: the kit draws one `li` for
/// each named row of `inventory.ledger()`, in order.
fn look(
    document: &Document,
    inventory: &InventoryPanel,
    flashing: &[Flashing],
    drawn: &Element,
) -> Result<(), JsValue> {
    let rows = drawn.query_selector_all(".kit-row")?;
    for (index, row) in (0..rows.length()).zip(inventory.rows.iter()) {
        let Some(item) = rows
            .item(index)
            .and_then(|node| node.dyn_into::<Element>().ok())
        else {
            continue;
        };
        item.set_attribute("data-row", &row.id)?;
        let look = inventory.look_of(&row.id, flashing);
        let classes = look.classes();
        if !classes.is_empty() {
            item.set_attribute("class", &format!("{} {classes}", item.class_name()))?;
        }
        if let Some(flash) = look.flash {
            set_style(&item, &format!("--inv-age:{}ms", look.age_ms))?;
            let tag = text_element(document, "span", "inv-tag", &flash.tag())?;
            append(&item, &tag)?;
        }
    }
    Ok(())
}

/// Fades the panel on screen out while a full-frame picture is up (`covered`): the panel's element
/// stays where it is, so its opacity eases (400 ms, never a slide). Returns whether the panel is to
/// be drawn again, which it is when the room returns: moves that came while the picture was up
/// start their flash then, and the panel fades back in (`inv-return`).
pub fn settle_cover(document: &Document, covered: bool) -> Result<bool, JsValue> {
    let was = COVER.with(Cell::get);
    let Some(layer) = document.query_selector(".tv-inventory")? else {
        return Ok(false);
    };
    if covered {
        COVER.with(|cover| cover.set(true));
        let classes = layer.class_name();
        if !classes.split_whitespace().any(|class| class == COVERED) {
            layer.set_attribute("class", &format!("{classes} {COVERED}"))?;
        }
    }
    Ok(was && !covered)
}

//! The clue board down the left of the shared screen: a small card for each clue found so far, the
//! newest at the top with what was found, the older ones under it, smaller when the board is
//! crowded. A clue that is worked out is marked as resolved.
//!
//! What the board holds and how its notes are laid out is `clues`; the look is the kit's
//! (`kit::clue_board`), and this mounts it. The board is its own piece of the screen
//! (`Part::TvClues`), redrawn only when the board changes. It opens for `CLUE_BOARD_OPEN_MS` when
//! it changes and then folds to a slim strip of pictures under the logo; the fold is the theme's
//! (`.tv-clues.clue-shut`). A redraw keeps the notes that were already on screen still
//! (`carry_over`): only a note that is new fades in.

use std::cell::RefCell;

use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element};

use crate::clues::{BoardState, ClueBoard, Observed, Phase};
use crate::dom::set_style;
use crate::fixture::TvScene;
use crate::kit::KitPanels;
use crate::kit::clue_board::KitClues;
use crate::limits::CLUE_BOARD_OPEN_MS;
use crate::page_log::{now_ms, with_log};
use crate::timeline::{EndReason, TimelineKind};

/// The page's memory of the board between draws, and the timeline record that follows it (its
/// expanded stretch, then its shut one).
#[derive(Default)]
struct Driver {
    state: BoardState,
    record: Option<u64>,
}

thread_local! {
    static DRIVER: RefCell<Driver> = RefCell::new(Driver::default());
}

fn with_driver<R>(change: impl FnOnce(&mut Driver) -> R) -> R {
    DRIVER.with(|driver| change(&mut driver.borrow_mut()))
}

fn record_id(key: &str, phase: Phase) -> String {
    format!("{key}-{}", phase.suffix())
}

/// Closes the board's open record for `reason`.
fn close_record(driver: &mut Driver, now: u64, reason: EndReason) {
    if let Some(seq) = driver.record.take() {
        with_log(|log| log.close(seq, now, reason));
    }
}

/// Notes the board now drawn: a changed board opens, and its record is opened to match.
fn observe(key: Option<String>) -> (Option<Phase>, u64) {
    let now = now_ms();
    with_driver(|driver| {
        match driver.state.observe(key.clone(), now) {
            Observed::Changed => {
                close_record(driver, now, EndReason::Replaced);
                if let Some(key) = key {
                    driver.record = with_log(|log| {
                        log.open(
                            TimelineKind::Clues,
                            &record_id(&key, Phase::Open),
                            now,
                            Some(CLUE_BOARD_OPEN_MS),
                            None,
                        )
                    });
                }
            }
            Observed::Cleared => close_record(driver, now, EndReason::Removed),
            Observed::Continuing | Observed::Absent => {}
        }
        (driver.state.phase(), driver.state.open_for_ms(now))
    })
}

/// The board of `scene`, or `None` when it has none. A board that has just changed is open and
/// stays so for `CLUE_BOARD_OPEN_MS`; the same board drawn again is as it was left. The kit draws
/// every note whatever the phase: a shut board is the same elements folded by the theme.
pub fn board(document: &Document, scene: &TvScene) -> Result<Option<Element>, JsValue> {
    let (phase, open_for_ms) = observe(scene.clues.as_ref().map(ClueBoard::board_identity));
    let (Some(clues), Some(phase)) = (&scene.clues, phase) else {
        return Ok(None);
    };
    let layer = crate::kit::draw::panels(
        document,
        &KitPanels {
            clues: Some(KitClues {
                board: clues.clone(),
                open: true,
            }),
            ..KitPanels::default()
        },
        None,
    )?;
    layer.set_attribute(
        "class",
        match phase {
            Phase::Open => "kit tv-clues clue-open",
            Phase::Shut => "kit tv-clues clue-shut",
        },
    )?;
    layer.set_attribute("aria-label", "Clues found")?;
    layer.set_attribute("data-board", &clues.board_identity())?;
    if phase == Phase::Open {
        // The hold is a CSS animation whose end shuts the board (`on_animation_end`); a redraw
        // carries on from where it was instead of starting it again.
        set_style(
            &layer,
            &format!("--clue-hold:{CLUE_BOARD_OPEN_MS}ms;animation-delay:-{open_for_ms}ms;"),
        )?;
    }
    // The kit draws the notes in the order `placed` gives them; each is marked with its clue, so a
    // later draw can tell a note it showed already from a new one.
    let drawn = layer.query_selector_all(".kit-note")?;
    for (index, placed) in (0..drawn.length()).zip(clues.placed()) {
        if let Some(note) = drawn
            .item(index)
            .and_then(|node| node.dyn_into::<Element>().ok())
        {
            note.set_attribute("data-clue-id", &placed.note.id)?;
        }
    }
    Ok(Some(layer))
}

/// Shuts the board when its hold has run out. Returns whether `target` was the board.
pub fn on_animation_end(target: &Element) -> Result<bool, JsValue> {
    if !target
        .class_name()
        .split_whitespace()
        .any(|name| name == "tv-clues")
        || target.closest(".scene-leaving")?.is_some()
    {
        return Ok(false);
    }
    let now = now_ms();
    let shut = with_driver(|driver| {
        let shut = driver.state.shut();
        if shut {
            close_record(driver, now, EndReason::Ended);
            if let Some(key) = driver.state.key().map(str::to_owned) {
                driver.record = with_log(|log| {
                    log.open(
                        TimelineKind::Clues,
                        &record_id(&key, Phase::Shut),
                        now,
                        None,
                        None,
                    )
                });
            }
        }
        shut
    });
    if shut {
        // Whether the strip is clear of the people is decided when the bubbles are placed.
        target.set_attribute("class", "kit tv-clues clue-shut")?;
    }
    Ok(true)
}

/// Settles a draw of the board against the one on screen in `slot`: a note that was already shown
/// is marked settled so it does not fade in again, and a board redrawn while it was open keeps its
/// glass still (`clue-redrawn`) instead of fading in a second time.
pub fn carry_over(slot: &Element, built: Option<Element>) -> Result<Option<Element>, JsValue> {
    let Some(root) = &built else {
        return Ok(built);
    };
    if root.class_name().contains("clue-open")
        && slot.query_selector(".tv-clues.clue-open")?.is_some()
    {
        root.set_attribute("class", &format!("{} clue-redrawn", root.class_name()))?;
    }
    let shown = slot.query_selector_all(".kit-note")?;
    for index in 0..shown.length() {
        let Some(id) = shown
            .item(index)
            .and_then(|node| node.dyn_into::<Element>().ok())
            .and_then(|old| old.get_attribute("data-clue-id"))
        else {
            continue;
        };
        if let Some(new) = find_note(root, &id)? {
            new.set_attribute("class", &format!("{} clue-settled", new.class_name()))?;
        }
    }
    Ok(built)
}

fn find_note(root: &Element, id: &str) -> Result<Option<Element>, JsValue> {
    let notes = root.query_selector_all(".kit-note")?;
    for index in 0..notes.length() {
        if let Some(note) = notes
            .item(index)
            .and_then(|node| node.dyn_into::<Element>().ok())
            && note.get_attribute("data-clue-id").as_deref() == Some(id)
        {
            return Ok(Some(note));
        }
    }
    Ok(None)
}

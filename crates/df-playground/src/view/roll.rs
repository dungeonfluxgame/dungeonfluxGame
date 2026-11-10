//! The roll card in the middle of the shared screen: one resolved roll staged as a call, a tumble,
//! the sum and the verdict, then taken away (`roll`, `kit::roll_card`).
//!
//! The card is its own piece of the screen (`Part::TvRoll`), drawn once for each roll and never
//! redrawn while it runs: the animations are CSS, begun with the card, and a page that opens
//! mid-roll carries on from how far along the host says the roll is. The page rolls no die: every
//! face is the server's, and the tumble's randomness only decides how the dice move. Each stage's
//! start and end is a timeline record, written from the browser's own animation events
//! (`on_animation_start`, `on_animation_end`), so the host's log shows when the call, the tumble,
//! the sum and the card's leaving happened. While the card is up the screen's other panels step
//! aside (`covers_panels`, `is_up`).

use std::cell::RefCell;

use wasm_bindgen::JsValue;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Document, Element, HtmlImageElement};

use crate::dom::{append, element, set_style};
use crate::fixture::TvScene;
use crate::kit::draw::roll_card::draw;
use crate::kit::roll_card::{DieArt, RollCard, staging_vars};
use crate::page_log::{now_ms, with_log};
use crate::roll::{RollSchedule, ShownRoll, TumblePlan, covers_panels};
use crate::timeline::{EndReason, ErrorSource, TimelineKind};

/// The class of the element that holds the card.
const CARD: &str = "tv-roll";
/// The class that marks the card as leaving, so the other panels fade back.
const LEFT: &str = "roll-left";
/// The class that takes a finished card off the screen.
const GONE: &str = "roll-gone";

/// The timeline records a roll keeps open, by sequence number.
#[derive(Debug, Default)]
struct Records {
    card: Option<u64>,
    call: Option<u64>,
    tumble: Option<u64>,
    sum: Option<u64>,
}

/// What the page remembers of the roll it is showing.
#[derive(Debug, Default)]
struct Driver {
    /// The roll being shown, by serial, and when the page first drew it.
    shown: Option<(u64, u64)>,
    plan: Option<TumblePlan>,
    schedule: Option<RollSchedule>,
    records: Records,
    /// The roll that has begun to leave, and the roll that is gone.
    left: Option<u64>,
    gone: Option<u64>,
}

thread_local! {
    static DRIVER: RefCell<Driver> = RefCell::new(Driver::default());
    /// The pictures of the dice, kept so the browser holds them decoded: the first card draws
    /// them without a wait.
    static PRELOADED: RefCell<Vec<HtmlImageElement>> = const { RefCell::new(Vec::new()) };
}

fn with_driver<R>(change: impl FnOnce(&mut Driver) -> R) -> R {
    DRIVER.with(|driver| change(&mut driver.borrow_mut()))
}

/// The page's own randomness, for how the dice move.
fn fresh_seed() -> u64 {
    (js_sys::Math::random() * 9_007_199_254_740_992.0) as u64
}

fn record_id(roll: &ShownRoll, stage: &str) -> String {
    if stage.is_empty() {
        roll.view.id.clone()
    } else {
        format!("{}.{stage}", roll.view.id)
    }
}

impl Driver {
    /// Closes every record that is open, for `reason`.
    fn close_records(&mut self, now: u64, reason: EndReason) {
        let records = std::mem::take(&mut self.records);
        for seq in [records.sum, records.tumble, records.call, records.card]
            .into_iter()
            .flatten()
        {
            with_log(|log| log.close(seq, now, reason));
        }
    }

    /// Takes up `roll` when it is not the one shown: its tumble is chosen, the records of the
    /// roll it replaces are closed and the call and card records open.
    fn begin(&mut self, roll: &ShownRoll, now: u64) {
        if self.shown.map(|(serial, _)| serial) == Some(roll.serial) {
            return;
        }
        let reason = if self.shown.is_some() {
            EndReason::Replaced
        } else {
            EndReason::Removed
        };
        self.close_records(now, reason);
        self.left = None;
        self.gone = None;
        let plan = TumblePlan::new(fresh_seed(), roll.view.dice.len() * 2 + 1);
        let schedule = RollSchedule::new(plan.tumble_ms, roll.min_hold_ms);
        self.shown = Some((roll.serial, now));
        self.plan = Some(plan);
        self.schedule = Some(schedule);
        self.records.card = with_log(|log| {
            log.open(
                TimelineKind::Roll,
                &record_id(roll, ""),
                now,
                Some(schedule.gone_ms()),
                None,
            )
        });
        self.records.call = with_log(|log| {
            log.open(
                TimelineKind::Roll,
                &record_id(roll, "call"),
                now,
                Some(schedule.tumble_start_ms()),
                None,
            )
        });
    }
}

// TODO(DEV-12): a browser-target test of this draw on the real package: the stage timers' animation
// events open and close the `roll` records, the card leaves and the inventory panel returns, and
// the first card after page load draws without a stall now that `preload_dice` decodes the art, and
// the card's frame rate in a browser with a compositor (median requestAnimationFrame delta, first
// animated frame within 150 ms of the card's insertion).
/// The card of `scene`, or `None` when it has no roll, or the roll is over.
pub fn card(document: &Document, scene: &TvScene) -> Result<Option<Element>, JsValue> {
    let now = now_ms();
    let Some(roll) = &scene.roll else {
        with_driver(|driver| {
            driver.close_records(now, EndReason::Removed);
            driver.shown = None;
        });
        return Ok(None);
    };
    let (plan, schedule, age_ms) = with_driver(|driver| {
        driver.begin(roll, now);
        let first_drawn = driver.shown.map_or(now, |(_, first)| first);
        let (Some(plan), Some(schedule)) = (driver.plan.clone(), driver.schedule) else {
            return None;
        };
        Some((
            plan,
            schedule,
            roll.elapsed_ms.max(now.saturating_sub(first_drawn)),
        ))
    })
    .ok_or_else(|| JsValue::from_str("the roll card has no plan"))?;
    let over =
        with_driver(|driver| driver.gone == Some(roll.serial)) || age_ms >= schedule.gone_ms();
    if over {
        with_driver(|driver| {
            driver.gone = Some(roll.serial);
            driver.left = Some(roll.serial);
            driver.close_records(now, EndReason::Ended);
        });
        return Ok(None);
    }
    let layer = element(document, "div", &format!("kit {CARD}"))?;
    set_style(&layer, &staging_vars(schedule, age_ms))?;
    layer.set_attribute("data-roll", &roll.view.id)?;
    layer.set_attribute("data-serial", &roll.serial.to_string())?;
    append(&layer, &draw(document, &RollCard::from(&roll.view), &plan)?)?;
    Ok(Some(layer))
}

/// Whether the card is up and has not begun to leave, so the screen's other panels stay aside.
pub fn is_up(scene: &TvScene) -> bool {
    with_driver(|driver| covers_panels(scene.roll.as_ref(), driver.left))
}

/// The stage of the card an invisible stage timer (`roll-clock-<stage>`) belongs to.
fn stage_of(target: &Element) -> Option<String> {
    target
        .class_name()
        .split_whitespace()
        .find_map(|name| name.strip_prefix("roll-clock-").map(str::to_owned))
}

/// The card's wrapper and the serial of its roll, when `target` is one of its stage timers on
/// the live screen (not on a copy that is leaving).
fn card_of(target: &Element) -> Result<Option<(Element, u64)>, JsValue> {
    if target.closest(".scene-leaving")?.is_some() {
        return Ok(None);
    }
    let Some(layer) = target.closest(&format!(".{CARD}"))? else {
        return Ok(None);
    };
    let serial = layer
        .get_attribute("data-serial")
        .and_then(|text| text.parse::<u64>().ok());
    Ok(serial.map(|serial| (layer, serial)))
}

/// A stage's animation has begun: the stage's record opens, and the card that begins to leave is
/// marked so the other panels return. Returns whether the card began to leave.
pub fn on_animation_start(target: &Element) -> Result<bool, JsValue> {
    let Some(stage) = stage_of(target) else {
        return Ok(false);
    };
    let Some((layer, serial)) = card_of(target)? else {
        return Ok(false);
    };
    let now = now_ms();
    let leaving = stage == "exit";
    with_driver(|driver| {
        let (Some((shown, _)), Some(schedule)) = (driver.shown, driver.schedule) else {
            return;
        };
        let (Some(id), true) = (layer.get_attribute("data-roll"), shown == serial) else {
            return;
        };
        let (slot, planned) = match stage.as_str() {
            "tumble" => (&mut driver.records.tumble, schedule.tumble_ms()),
            "sum" => (&mut driver.records.sum, schedule.sum_ms()),
            "exit" => {
                driver.left = Some(serial);
                return;
            }
            _ => return,
        };
        if slot.is_none() {
            *slot = with_log(|log| {
                log.open(
                    TimelineKind::Roll,
                    &format!("{id}.{stage}"),
                    now,
                    Some(planned),
                    None,
                )
            });
        }
    });
    if leaving {
        layer.set_attribute("class", &format!("kit {CARD} {LEFT}"))?;
    }
    Ok(leaving)
}

/// A stage's animation has ended: the stage's record closes, and a card whose leaving has ended is
/// taken off the screen. Returns whether `target` was a stage timer of the card.
pub fn on_animation_end(target: &Element) -> Result<bool, JsValue> {
    let Some(stage) = stage_of(target) else {
        return Ok(false);
    };
    let Some((layer, serial)) = card_of(target)? else {
        return Ok(true);
    };
    let now = now_ms();
    let gone = stage == "exit";
    with_driver(|driver| {
        if driver.shown.map(|(shown, _)| shown) != Some(serial) {
            return;
        }
        let record = match stage.as_str() {
            "call" => driver.records.call.take(),
            "tumble" => driver.records.tumble.take(),
            "sum" => driver.records.sum.take(),
            _ => {
                driver.gone = Some(serial);
                driver.records.card.take()
            }
        };
        if let Some(seq) = record {
            with_log(|log| log.close(seq, now, EndReason::Ended));
        }
    });
    if gone {
        layer.set_attribute("class", &format!("kit {CARD} {LEFT} {GONE}"))?;
    }
    Ok(true)
}

/// Fetches and decodes the pictures of the dice at page load, so the first roll card does not
/// stall while the browser reads and decodes six large pictures. A picture that cannot be fetched
/// is noted as an asset error; the card then draws its dice on plain shapes.
pub fn preload_dice() -> Result<(), JsValue> {
    for art in DieArt::ALL {
        let image = HtmlImageElement::new()?;
        image.set_src(&crate::path::dice_file(&format!("{}.png", art.key())));
        PRELOADED.with(|kept| kept.borrow_mut().push(image.clone()));
        spawn_local(async move {
            if JsFuture::from(image.decode()).await.is_err() {
                crate::page_log::note_error(
                    ErrorSource::Asset,
                    &format!("dice picture '{}' did not load", art.key()),
                );
            }
        });
    }
    Ok(())
}

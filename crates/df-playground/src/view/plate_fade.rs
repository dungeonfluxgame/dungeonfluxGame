//! The layers of a plate crossfade. The state machine is `plate_fade`; this keeps the DOM
//! to match it: the old picture stays in the backdrop slot, the new one is added over it invisible,
//! starts to fade in once its image has loaded and the layers under it are removed when the fade
//! ends. A change that arrives mid-fade holds the picture fading in at the opacity it reached and
//! fades the newest one in over both, so what is on screen never jumps.
//!
//! The camera moves the slot's parent, so every layer follows it together.

use std::cell::RefCell;

use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Element, HtmlImageElement};

use crate::controls::Transition;
use crate::dom::{append, clear, element, set_style};
use crate::page_log::{now_ms, with_log};
use crate::plate_fade::{FadeStage, PlateFade, crossfade_ms};
use crate::plate_fit::{FIT_ATTRIBUTE, Fit, PlateFit};
use crate::timeline::{EndReason, TimelineKind};

/// A layer whose picture is fading in or waiting to load.
const INCOMING: &str = "tv-backdrop-incoming";
/// An incoming layer whose image has loaded and whose opacity is animating.
const FADING: &str = "tv-backdrop-fading";
/// The stage while a plate change is under way: the bubbles over the figures give way to the
/// lower third of the line (`theme.css`).
const STAGE_FADING: &str = "tv-fading";

/// The machine and the timeline records of the fade under way: the wait for the image, and the
/// fade itself.
#[derive(Default)]
struct Driver {
    fade: PlateFade,
    load_seq: Option<u64>,
    fade_seq: Option<u64>,
    duration_ms: u64,
    /// The stage while a fade is under way, which carries `tv-fading`.
    stage: Option<Element>,
    /// The records of the speakers' lower thirds that stand in for their bubbles during the fade.
    third_seqs: Vec<u64>,
}

thread_local! {
    static DRIVER: RefCell<Driver> = RefCell::new(Driver::default());
}

fn with_driver<R>(change: impl FnOnce(&mut Driver) -> R) -> R {
    DRIVER.with(|driver| change(&mut driver.borrow_mut()))
}

/// The timeline id of the wait for a picture's image (`start` to `loaded`) and of its fade
/// (`loaded` to `end`). Both are `fade` timeline records (and a speaker's lower third a `third`
/// one), which the host writes as TIMING lines.
fn load_id(media: &str) -> String {
    format!("fade-load-{media}")
}

fn fade_id(media: &str) -> String {
    format!("fade-{media}")
}

fn close_records(driver: &mut Driver, reason: EndReason) {
    let now = now_ms();
    let thirds = std::mem::take(&mut driver.third_seqs);
    for seq in [driver.load_seq.take(), driver.fade_seq.take()]
        .into_iter()
        .flatten()
        .chain(thirds)
    {
        with_log(|log| log.close(seq, now, reason));
    }
    if let Some(stage) = driver.stage.take() {
        set_class(&stage, STAGE_FADING, false);
    }
}

fn media_of(layer: &Element) -> Result<Option<String>, JsValue> {
    Ok(layer
        .query_selector("img[data-media]")?
        .and_then(|image| image.get_attribute("data-media")))
}

/// The slot's layers, bottom first.
fn layers(slot: &Element) -> Vec<Element> {
    let children = slot.children();
    (0..children.length())
        .filter_map(|index| children.item(index))
        .collect()
}

fn plain(slot: &Element, built: Option<Element>) -> Result<(), JsValue> {
    clear(slot);
    match built {
        Some(built) => append(slot, &built),
        None => Ok(()),
    }
}

/// Puts `built`, the new plate's frame, in `slot`: faded in over the pictures there
/// when `transition` fades and `reduced_motion` is not asked for, swapped at once otherwise (also
/// for the first picture, a picture without an image, and a change of caption or turn only).
pub fn swap(
    slot: &Element,
    built: Option<Element>,
    transition: Transition,
    reduced_motion: bool,
) -> Result<(), JsValue> {
    let duration = crossfade_ms(transition).filter(|_| !reduced_motion);
    let wanted = built.as_ref().map(media_of).transpose()?.flatten();
    let shown = layers(slot);
    let (Some(built), Some(duration), Some(media), false) =
        (built.clone(), duration, wanted, shown.is_empty())
    else {
        reset();
        return plain(slot, built);
    };
    let top_media = match shown.last() {
        Some(top) => media_of(top)?,
        None => None,
    };
    if top_media.as_deref() == Some(media.as_str()) {
        // The same picture again, with another caption or turn: while it is still arriving it is
        // left alone, and otherwise the frame is replaced where it stands.
        if with_driver(|driver| driver.fade.stage()) == FadeStage::Idle {
            return plain(slot, Some(built));
        }
        return Ok(());
    }
    let prior = with_driver(|driver| driver.fade.begin(&media));
    let now = now_ms();
    match (prior, shown.last()) {
        (FadeStage::Loading, Some(top)) => {
            // Never seen: dropped, and the wait for its image ends.
            top.remove();
            with_driver(|driver| {
                if let Some(seq) = driver.load_seq.take() {
                    with_log(|log| log.close(seq, now, EndReason::Replaced));
                }
            });
        }
        (FadeStage::Fading, Some(top)) => {
            hold(top)?;
            with_driver(|driver| {
                if let Some(seq) = driver.fade_seq.take() {
                    with_log(|log| log.close(seq, now, EndReason::Replaced));
                }
            });
        }
        _ => {}
    }
    // The pictures under the new one no longer count as the pictures shown.
    for layer in layers(slot) {
        let images = layer.query_selector_all("img[data-media]")?;
        for index in 0..images.length() {
            if let Some(image) = images
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            {
                image.remove_attribute("data-media")?;
            }
        }
    }
    enter_lower_third(slot)?;
    built.set_attribute("class", &format!("{} {INCOMING}", built.class_name()))?;
    let existing = built.get_attribute("style").unwrap_or_default();
    set_style(&built, &format!("{existing}--fade-ms:{duration}ms;"))?;
    append(slot, &built)?;
    with_driver(|driver| {
        driver.duration_ms = duration;
        driver.load_seq =
            with_log(|log| log.open(TimelineKind::Fade, &load_id(&media), now, None, None));
    });
    // A picture the browser already has is loaded now; the others say so with `load`.
    if let Some(image) = built
        .query_selector("img")?
        .and_then(|image| image.dyn_into::<HtmlImageElement>().ok())
        && image.complete()
        && image.natural_width() > 0
    {
        fit_loaded(&image)?;
        start_fade(&built, &media)?;
    }
    Ok(())
}

/// Draws a backdrop `layer` for the way its picture lies in the frame (`plate_fit`): a picture
/// that is not near the frame's shape is shown whole, over a blurred copy of itself that the theme
/// draws from the second image added here (`.tv-backdrop-fill`, under `data-fit=contain`). The
/// fit is written as an attribute, which a layer keeps when its classes are reset.
pub fn fit_backdrop(layer: &Element, fit: PlateFit) -> Result<(), JsValue> {
    let key = fit.fit().key();
    if layer.get_attribute(FIT_ATTRIBUTE).as_deref() != Some(key) {
        layer.set_attribute(FIT_ATTRIBUTE, key)?;
    }
    if fit.fit() != Fit::Contain || layer.query_selector(".tv-backdrop-fill")?.is_some() {
        return Ok(());
    }
    let (Some(picture), Some(document)) = (
        layer.query_selector(".tv-backdrop-image")?,
        layer.owner_document(),
    ) else {
        return Ok(());
    };
    let Some(source) = picture.get_attribute("src") else {
        return Ok(());
    };
    let fill = element(&document, "img", "tv-backdrop-fill")?;
    fill.set_attribute("src", &source)?;
    fill.set_attribute("alt", "")?;
    fill.set_attribute("aria-hidden", "true")?;
    append(layer, &fill)
}

/// Fits the backdrop layer of a picture that has loaded to the picture's own pixel size.
fn fit_loaded(image: &HtmlImageElement) -> Result<(), JsValue> {
    match image.closest(".tv-backdrop")? {
        Some(layer) => fit_backdrop(
            &layer,
            PlateFit::of_size(image.natural_width(), image.natural_height()),
        ),
        None => Ok(()),
    }
}

/// Holds a layer that was fading in at the opacity it has now, so a newer picture can fade in
/// over it without the screen changing.
fn hold(layer: &Element) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("the page has no window"))?;
    let opacity = window
        .get_computed_style(layer)?
        .map(|style| style.get_property_value("opacity"))
        .transpose()?
        .and_then(|text| text.trim().parse::<f32>().ok())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    layer.set_attribute("class", "tv-backdrop")?;
    layer.set_attribute("style", &format!("opacity:{opacity:.3};animation:none;"))
}

fn start_fade(layer: &Element, media: &str) -> Result<(), JsValue> {
    if !with_driver(|driver| driver.fade.loaded(media)) {
        return Ok(());
    }
    layer.set_attribute("class", &format!("{} {FADING}", layer.class_name()))?;
    let now = now_ms();
    with_driver(|driver| {
        if let Some(seq) = driver.load_seq.take() {
            with_log(|log| log.close(seq, now, EndReason::Ended));
        }
        driver.fade_seq = with_log(|log| {
            log.open(
                TimelineKind::Fade,
                &fade_id(media),
                now,
                Some(driver.duration_ms),
                None,
            )
        });
    });
    Ok(())
}

/// The picture on screen is `layer` alone: the layers under it go, and it is a plain layer.
fn settle(layer: &Element) -> Result<(), JsValue> {
    while let Some(under) = layer.previous_element_sibling() {
        under.remove();
    }
    layer.set_attribute("class", "tv-backdrop")?;
    layer.remove_attribute("style")
}

/// An image of the page loaded or failed. An incoming picture's image that loaded starts its
/// fade; one that failed is shown at once, as the broken picture it is, so the failure is seen.
pub fn on_image(image: &HtmlImageElement, failed: bool) -> Result<(), JsValue> {
    if !failed {
        fit_loaded(image)?;
    }
    let Some(layer) = image.closest(&format!(".{INCOMING}"))? else {
        return Ok(());
    };
    let Some(media) = image.get_attribute("data-media") else {
        return Ok(());
    };
    if !failed {
        return start_fade(&layer, &media);
    }
    if with_driver(|driver| driver.fade.finished(&media)) {
        with_driver(|driver| close_records(driver, EndReason::Failed));
        settle(&layer)?;
    }
    Ok(())
}

/// An animation ended on `target`. True when it was a plate fade, which is now complete.
pub fn on_animation_end(target: &Element) -> Result<bool, JsValue> {
    if !target
        .class_name()
        .split_whitespace()
        .any(|name| name == FADING)
    {
        return Ok(false);
    }
    let Some(media) = media_of(target)? else {
        return Ok(false);
    };
    if with_driver(|driver| driver.fade.finished(&media)) {
        with_driver(|driver| close_records(driver, EndReason::Ended));
        settle(target)?;
    }
    Ok(true)
}

/// Lets go of a fade under way, because the layers were replaced without one.
pub fn reset() {
    with_driver(|driver| {
        if driver.fade.abandon() != FadeStage::Idle {
            close_records(driver, EndReason::Removed);
        }
    });
}

/// Puts the stage of `slot` in its fading state and opens the timeline record of each speaker's
/// lower third, if it is not in that state already: the bubbles give way to the lower third.
fn enter_lower_third(slot: &Element) -> Result<(), JsValue> {
    let Some(stage) = slot.closest(".tv-stage")? else {
        return Ok(());
    };
    if with_driver(|driver| driver.stage.is_some()) {
        return Ok(());
    }
    set_class(&stage, STAGE_FADING, true);
    let now = now_ms();
    let twins = stage.query_selector_all(".kit-speaker-twin[data-character]")?;
    let mut seqs = Vec::new();
    for index in 0..twins.length() {
        let Some(character) = twins
            .item(index)
            .and_then(|node| node.dyn_into::<Element>().ok())
            .and_then(|twin| twin.get_attribute("data-character"))
        else {
            continue;
        };
        let id = format!("third-{character}");
        seqs.extend(with_log(|log| {
            log.open(TimelineKind::Third, &id, now, None, None)
        }));
    }
    with_driver(|driver| {
        driver.stage = Some(stage);
        driver.third_seqs = seqs;
    });
    Ok(())
}

/// Keeps the items tied to characters of `slot` while a plate change is under way and `built`
/// has none: they are marked stale and fade out with the stage, instead of vanishing. True when
/// that was done and the slot is to be left as it is.
pub fn hold_anchors(slot: &Element, built: &Option<Element>) -> Result<bool, JsValue> {
    let fading = slot
        .closest(".tv-stage")?
        .is_some_and(|stage| has_class(&stage, STAGE_FADING));
    if built.is_some() || !fading || slot.first_element_child().is_none() {
        return Ok(false);
    }
    for layer in layers(slot) {
        layer.set_attribute("data-stale", "true")?;
    }
    Ok(true)
}

fn has_class(element: &Element, class: &str) -> bool {
    element
        .class_name()
        .split_whitespace()
        .any(|name| name == class)
}

/// Adds or takes away one class of `element`, leaving the others.
fn set_class(element: &Element, class: &str, on: bool) {
    let mut names: Vec<String> = element
        .class_name()
        .split_whitespace()
        .filter(|name| *name != class)
        .map(str::to_owned)
        .collect();
    if on {
        names.push(class.to_owned());
    }
    element.set_class_name(&names.join(" "));
}

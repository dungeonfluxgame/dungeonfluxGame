//! Presentation rules for every live screen, installed once per page. The HTML shell carries
//! no styling: `live_legacy.css` keeps the rules the combat and aftermath rendering still use,
//! `live_player.css` styles the phone column, `live_tv.css` the shared display, `live_fx.css` its
//! motion and `live_stage.css` where the cast stands and what is said beneath them.
//!
//! The shared display (`.tv-stage`) is a fixed 16:9 canvas sized from `--u`, one hundredth
//! of the width of the largest 16:9 rectangle that fits the window, so type and panels
//! scale together on a 1080p or 4K television and never scroll. Optional generated art is
//! layered above its authored fallback in CSS, so an absent file leaves the fallback.
use super::*;

const LEGACY: &str = include_str!("live_legacy.css");
const PLAYER: &str = include_str!("live_player.css");
const TV: &str = include_str!("live_tv.css");
const MOTION: &str = include_str!("live_fx.css");
const STAGE: &str = include_str!("live_stage.css");
// PLAY-L01: the previous cut-out fading out above a character's new expression.
const IMAGERY: &str = include_str!("live_imagery.css");

/// Adds the live screen rules once; a second call on the same page is a no-op.
pub(super) fn install(document: &Document) -> Result<(), JsValue> {
    if document.query_selector("style[data-live-theme]")?.is_some() {
        return Ok(());
    }
    let style = document.create_element("style")?;
    style.set_attribute("data-live-theme", "")?;
    style.set_text_content(Some(&format!(
        "{LEGACY}\n{PLAYER}\n{TV}\n{MOTION}\n{STAGE}\n{IMAGERY}"
    )));
    let host = match document.query_selector("head")? {
        Some(head) => head,
        None => document
            .document_element()
            .ok_or_else(|| JsValue::from_str("document element absent"))?,
    };
    host.append_child(&style)?;
    Ok(())
}

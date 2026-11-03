//! The top bar of the shared screen: the wordmark at the left and the pills at the right.
//!
//! The session's name, the party's count, the balance and the rate are drawn only when the scene
//! carries them, and as the words the server gave: the public pricing shows what tables pay, so
//! this page never works a figure out. A pure function of the scene's data (`lobby_model`).

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::fixture::TvScene;
use crate::lobby_model::{LogoStyle, TopBar, party_text, session_text};
use crate::overlay::Glyph;
use crate::view_icons::{Icon, icon};
use crate::view_kit::glyph;

/// The wordmark's name.
const WORDMARK: &str = "DungeonFlux";

/// The top bar of `scene`, or `None` when it has none.
pub fn build(document: &Document, scene: &TvScene) -> Result<Option<Element>, JsValue> {
    let Some(bar) = &scene.pills else {
        return Ok(None);
    };
    // The landing page's top bar (`view_home_chrome::top`) draws these same pills with its bell.
    if scene.home.as_ref().is_some_and(|home| home.top.is_some()) {
        return Ok(None);
    }
    let root = element(document, "div", "tb")?;
    // The scene's own `brand` logo (`view_brand`) and this one are the same mark: draw one.
    if let Some(style) = bar.logo.filter(|_| !scene.brand) {
        append(&root, &logo(document, style)?)?;
    }
    append(&root, &pills(document, bar, None)?)?;
    Ok(Some(root))
}

fn logo(document: &Document, style: LogoStyle) -> Result<Element, JsValue> {
    let class = match style {
        LogoStyle::Stacked => "tb-logo tb-logo-stacked",
        LogoStyle::Inline => "tb-logo tb-logo-inline",
    };
    let mark = element(document, "div", class)?;
    mark.set_attribute("role", "img")?;
    mark.set_attribute("aria-label", WORDMARK)?;
    append(&mark, &icon(document, Icon::Compass, "tb-compass")?)?;
    append(
        &mark,
        &text_element(document, "span", "tb-wordmark", WORDMARK)?,
    )?;
    if style == LogoStyle::Stacked {
        append(
            &mark,
            &icon(document, Icon::Compass, "tb-compass tb-compass-under")?,
        )?;
    }
    Ok(mark)
}

/// A plain pill: a picture and a line.
fn pill(
    document: &Document,
    picture: Element,
    words: &str,
    class: &str,
) -> Result<Element, JsValue> {
    let pill = element(document, "div", &format!("tb-pill {class}"))?;
    append(&pill, &picture)?;
    append(
        &pill,
        &text_element(document, "span", "tb-pill-text", words)?,
    )?;
    Ok(pill)
}

/// The pills of `bar`, with `bell` (the landing page's) before the profile mark.
pub(crate) fn pills(
    document: &Document,
    bar: &TopBar,
    bell: Option<Element>,
) -> Result<Element, JsValue> {
    let group = element(document, "div", "tb-pills")?;
    if let Some(session) = &bar.session {
        let picture = glyph(document, Glyph::Book, "tb-pill-icon")?;
        append(
            &group,
            &pill(document, picture, &session_text(session), "tb-session")?,
        )?;
    }
    if let Some(count) = bar.party {
        let picture = icon(document, Icon::People, "tb-pill-icon")?;
        append(
            &group,
            &pill(document, picture, &party_text(count), "tb-party")?,
        )?;
    }
    if bar.balance.is_some() || bar.rate.is_some() {
        let money = element(document, "div", "tb-pill tb-money")?;
        if let Some(balance) = &bar.balance {
            append(
                &money,
                &icon(document, Icon::Coins, "tb-pill-icon tb-coins")?,
            )?;
            let words = element(document, "span", "tb-pill-text")?;
            append(&words, &text_element(document, "span", "", "Balance: ")?)?;
            append(
                &words,
                &text_element(document, "span", "tb-balance", balance)?,
            )?;
            append(&money, &words)?;
        }
        if let Some(rate) = &bar.rate {
            append(&money, &icon(document, Icon::Bolt, "tb-pill-icon tb-bolt")?)?;
            append(
                &money,
                &text_element(document, "span", "tb-pill-text tb-rate", rate)?,
            )?;
        }
        append(&group, &money)?;
    }
    if let Some(bell) = bell {
        append(&group, &bell)?;
    }
    if bar.profile {
        let profile = element(document, "div", "tb-profile")?;
        profile.set_attribute("role", "img")?;
        profile.set_attribute("aria-label", "Your profile")?;
        append(&profile, &icon(document, Icon::User, "tb-profile-icon")?)?;
        append(&group, &profile)?;
    }
    Ok(group)
}

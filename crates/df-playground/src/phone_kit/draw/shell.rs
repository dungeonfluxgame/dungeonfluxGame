//! The phone's frame as elements: the link line, the header, the pills, the health bar, the
//! buttons, the toast, the empty and loading states and the bottom bar.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, set_action, set_style, text_element};
use crate::phone_kit::draw::glyph;
use crate::phone_kit::glyph::PhoneGlyph;
use crate::phone_kit::shell::{
    ButtonSpec, EmptyState, HpBar, LoadingState, PhoneHeader, Pill, PillTone, TabBar, TabKey, Toast,
};
use crate::view::kit::portrait;

const OFF_TAB_WORDS: &str = "Not open yet";

/// The line that says the link to the host is down. It sits in the column above the header, so it
/// pushes the screen down and never covers the hero's name.
pub fn link_bar(document: &Document, words: &str) -> Result<Element, JsValue> {
    let bar = element(document, "div", "ph-link")?;
    bar.set_attribute("role", "status")?;
    bar.set_attribute("aria-live", "polite")?;
    append(&bar, &glyph(document, PhoneGlyph::Wifi, "ph-link-glyph")?)?;
    append(
        &bar,
        &text_element(document, "span", "ph-link-words", words)?,
    )?;
    Ok(bar)
}

/// A health bar with its words under it.
pub fn hp_bar(document: &Document, hp: &HpBar) -> Result<Element, JsValue> {
    let bar = element(document, "div", "ph-hp")?;
    bar.set_attribute("data-tier", hp.tier().dom_marker())?;
    bar.set_attribute("role", "img")?;
    bar.set_attribute("aria-label", &hp.text())?;
    let track = element(document, "div", "ph-hp-track")?;
    let fill = element(document, "div", "ph-hp-fill")?;
    set_style(&fill, &format!("width:{:.1}%", hp.percent()))?;
    append(&track, &fill)?;
    if hp.temporary > 0 {
        let temporary = element(document, "div", "ph-hp-temp")?;
        set_style(
            &temporary,
            &format!(
                "left:{:.1}%;width:{:.1}%",
                hp.percent(),
                hp.temporary_percent()
            ),
        )?;
        append(&track, &temporary)?;
    }
    append(&bar, &track)?;
    append(
        &bar,
        &text_element(document, "span", "ph-hp-text", &hp.text())?,
    )?;
    Ok(bar)
}

/// The hero: portrait, name, the place and, when sent, the health bar.
pub fn header(document: &Document, header: &PhoneHeader) -> Result<Element, JsValue> {
    let head = element(document, "header", "ph-head")?;
    append(
        &head,
        &portrait(
            document,
            "ph-portrait",
            header.portrait.as_deref(),
            &header.name,
        )?,
    )?;
    let text = element(document, "div", "ph-head-text")?;
    append(
        &text,
        &text_element(document, "div", "ph-name", &header.name)?,
    )?;
    if let Some(subtitle) = &header.subtitle {
        append(
            &text,
            &text_element(document, "div", "ph-subtitle", subtitle)?,
        )?;
    }
    append(&head, &text)?;
    if let Some(hp) = &header.hp {
        let wrap = element(document, "div", "ph-head-hp")?;
        append(&wrap, &hp_bar(document, hp)?)?;
        append(&head, &wrap)?;
    }
    Ok(head)
}

/// A small rounded mark.
pub fn pill(document: &Document, pill: &Pill) -> Result<Element, JsValue> {
    let mark = element(document, "span", &pill_class(pill.tone))?;
    if let Some(drawing) = pill.glyph {
        append(&mark, &glyph(document, drawing, "ph-pill-glyph")?)?;
    }
    append(&mark, &text_element(document, "span", "", &pill.text)?)?;
    Ok(mark)
}

fn pill_class(tone: PillTone) -> String {
    format!("ph-pill ph-pill-{}", tone.key())
}

/// The status row under the header: one pill, on its own line.
pub fn status(document: &Document, status: &Pill) -> Result<Element, JsValue> {
    let row = element(document, "div", "ph-status")?;
    row.set_attribute("role", "status")?;
    append(&row, &pill(document, status)?)?;
    Ok(row)
}

/// A button of the kit; off, it says why under itself.
pub fn button(document: &Document, spec: &ButtonSpec) -> Result<Element, JsValue> {
    let wrap = element(document, "div", "ph-button-wrap")?;
    let control = text_element(
        document,
        "button",
        &format!("ph-btn ph-btn-{}", spec.kind.key()),
        &spec.label,
    )?;
    control.set_attribute("type", "button")?;
    if let Some(action) = &spec.action {
        set_action(&control, action)?;
    }
    if !spec.enabled {
        control.set_attribute("disabled", "")?;
    }
    append(&wrap, &control)?;
    if let Some(reason) = spec.reason.as_ref().filter(|_| !spec.enabled) {
        append(&wrap, &text_element(document, "p", "ph-reason", reason)?)?;
    }
    Ok(wrap)
}

/// The pop-up with its Keep button.
pub fn toast(document: &Document, toast: &Toast) -> Result<Element, JsValue> {
    let card = element(document, "div", "ph-toast")?;
    card.set_attribute("role", "status")?;
    append(
        &card,
        &glyph(document, PhoneGlyph::Inbox, "ph-toast-glyph")?,
    )?;
    let text = element(document, "div", "ph-toast-text")?;
    append(
        &text,
        &text_element(document, "div", "ph-toast-kind", &toast.kind)?,
    )?;
    append(
        &text,
        &text_element(document, "div", "ph-toast-line", &toast.text)?,
    )?;
    append(&card, &text)?;
    let keep = text_element(
        document,
        "button",
        "ph-btn ph-btn-primary ph-btn-small",
        &toast.keep_label,
    )?;
    keep.set_attribute("type", "button")?;
    append(&card, &keep)?;
    let close = element(document, "button", "ph-icon-btn")?;
    close.set_attribute("type", "button")?;
    close.set_attribute("aria-label", "Dismiss")?;
    append(
        &close,
        &glyph(document, PhoneGlyph::Close, "ph-icon-glyph")?,
    )?;
    append(&card, &close)?;
    Ok(card)
}

/// What a panel shows when it has nothing yet.
pub fn empty(document: &Document, state: &EmptyState) -> Result<Element, JsValue> {
    let wrap = element(document, "div", "ph-empty")?;
    let medallion = element(document, "div", "ph-medallion")?;
    append(
        &medallion,
        &glyph(
            document,
            state.glyph.unwrap_or(PhoneGlyph::Inbox),
            "ph-medallion-glyph",
        )?,
    )?;
    append(&wrap, &medallion)?;
    append(
        &wrap,
        &text_element(document, "h2", "ph-empty-title", &state.title)?,
    )?;
    append(
        &wrap,
        &text_element(document, "p", "ph-empty-text", &state.text)?,
    )?;
    Ok(wrap)
}

/// A turning ring over the hero's silhouette, for whatever the host is making.
pub fn loader(document: &Document) -> Result<Element, JsValue> {
    let loader = element(document, "div", "ph-loader")?;
    loader.set_attribute("aria-hidden", "true")?;
    append(&loader, &element(document, "span", "ph-loader-ring")?)?;
    append(
        &loader,
        &element(document, "span", "ph-loader-ring ph-loader-ring-back")?,
    )?;
    append(
        &loader,
        &glyph(document, PhoneGlyph::Hero, "ph-loader-glyph")?,
    )?;
    Ok(loader)
}

/// What a panel shows while the host answers.
pub fn loading(document: &Document, state: &LoadingState) -> Result<Element, JsValue> {
    let wrap = element(document, "div", "ph-loading")?;
    wrap.set_attribute("role", "status")?;
    append(&wrap, &loader(document)?)?;
    append(
        &wrap,
        &text_element(document, "p", "ph-loading-text", &state.text)?,
    )?;
    Ok(wrap)
}

fn tab_button(
    document: &Document,
    bar: &TabBar,
    tab: TabKey,
    class: &str,
) -> Result<Element, JsValue> {
    let active = bar.active == tab;
    let button = element(
        document,
        "button",
        &format!("{class}{}", if active { " ph-tab-active" } else { "" }),
    )?;
    button.set_attribute("type", "button")?;
    button.set_attribute("data-tab", tab.key())?;
    if active {
        button.set_attribute("aria-current", "page")?;
    }
    if !bar.works(tab) {
        button.set_attribute("disabled", "")?;
        button.set_attribute("title", OFF_TAB_WORDS)?;
    }
    append(&button, &glyph(document, tab.glyph(), "ph-tab-glyph")?)?;
    append(
        &button,
        &text_element(document, "span", "ph-tab-label", tab.label())?,
    )?;
    if let Some(count) = bar.badge(tab) {
        append(
            &button,
            &text_element(document, "span", "ph-badge", &count.to_string())?,
        )?;
    }
    Ok(button)
}

/// The bottom bar, with the "more" slot and its sheet when the panels do not all fit.
pub fn tab_bar(document: &Document, bar: &TabBar) -> Result<Element, JsValue> {
    let wrap = element(document, "div", "ph-tabs-wrap")?;
    let layout = bar.layout();
    if bar.more_open && !layout.more.is_empty() {
        let sheet = element(document, "div", "ph-more-sheet")?;
        for tab in &layout.more {
            append(&sheet, &tab_button(document, bar, *tab, "ph-more-item")?)?;
        }
        append(&wrap, &sheet)?;
    }
    let nav = element(document, "nav", "ph-tabs")?;
    nav.set_attribute("aria-label", "Panels")?;
    for tab in &layout.bar {
        append(&nav, &tab_button(document, bar, *tab, "ph-tab")?)?;
    }
    if !layout.more.is_empty() {
        let more = element(
            document,
            "button",
            if bar.more_open {
                "ph-tab ph-tab-more ph-tab-active"
            } else {
                "ph-tab ph-tab-more"
            },
        )?;
        more.set_attribute("type", "button")?;
        if !layout.more.iter().any(|tab| bar.works(*tab)) {
            more.set_attribute("disabled", "")?;
            more.set_attribute("title", OFF_TAB_WORDS)?;
        }
        more.set_attribute(
            "aria-expanded",
            if bar.more_open { "true" } else { "false" },
        )?;
        append(&more, &glyph(document, PhoneGlyph::More, "ph-tab-glyph")?)?;
        append(
            &more,
            &text_element(document, "span", "ph-tab-label", "More")?,
        )?;
        append(&nav, &more)?;
    }
    append(&wrap, &nav)?;
    Ok(wrap)
}

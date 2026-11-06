//! The landing page's ConfigurePanel and its setting cards, built from `ConfigureData`, drawn for a
//! television (render 19 for the materials, the TV type scale for the sizes).
//!
//! `build` is called by `view::home` and returns one element that fills the slot box the page gives
//! it, `width: 100%; height: 100%`, sized in `cqw` by the `home-configure` block of `theme.css`.
//! Every card, every drawn row and the preset selector carries a `data-focus-id`
//! (`configure:<card>`, `configure:<card>:<row>`, `configure:preset`; `ConfigureData::focus_ids`
//! lists them), and `view::home` gives the one whose id is `home.focus` the class `hm-focused`,
//! which draws the ring. Each card kind has its own builder (`select_card`, `sliders_card`,
//! `mixed_card`, `budget_card`) over shared row builders.
//!
//! Nothing is a mouse control. A choice is drawn as `< value >`, with an arrow each side that glows
//! when its card has the focus, and a slider is a wide bar of ten segments with its value in large
//! type at its right. Texts are set as text, and a banner is drawn only when it names a picture.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, text_element};
use crate::home::configure::{
    CardKind, ConfigureData, PanelLayout, PresetSelect, Row, SLIDER_SEGMENTS, SelectRow,
    SettingCard, SliderRow, StatRow,
};
use crate::view::kit::photo;

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
/// The inside of the left and right arrows of a choice, in a 24 by 24 box.
const ARROW_LEFT: &str = r#"<path d="M15 4.5l-7.5 7.5 7.5 7.5"/>"#;
const ARROW_RIGHT: &str = r#"<path d="M9 4.5l7.5 7.5-7.5 7.5"/>"#;
/// The inside of the round info mark, in a 24 by 24 box.
const INFO: &str = r#"<circle cx="12" cy="12" r="9.5"/><path d="M12 11v6M12 7.2v.2"/>"#;

/// The configure component, or an error when the page cannot be built.
pub fn build(document: &Document, data: &ConfigureData) -> Result<Element, JsValue> {
    let tiles = data.layout == PanelLayout::Tiles;
    let wrap = element(document, "div", if tiles { "cfg cfg-tiles" } else { "cfg" })?;
    let panel = element(document, "section", "cfg-panel ui-lit")?;
    panel.set_attribute("aria-label", &data.title)?;
    append(&panel, &head(document, data)?)?;
    let grid = element(document, "div", "cfg-grid")?;
    for (index, card) in data.cards.iter().enumerate() {
        let number = index + 1;
        let view = if tiles {
            tile_view(document, card, number, &data.currency)?
        } else {
            card_view(document, card, number, &data.currency)?
        };
        append(&grid, &view)?;
    }
    append(&panel, &grid)?;
    append(&wrap, &panel)?;
    Ok(wrap)
}

/// A drawing in the current text colour as an `svg` of class `class`.
fn mark(document: &Document, markup: &str, class: &str) -> Result<Element, JsValue> {
    let svg = document.create_element_ns(Some(SVG_NAMESPACE), "svg")?;
    for (name, value) in [
        ("viewBox", "0 0 24 24"),
        ("fill", "none"),
        ("stroke", "currentColor"),
        ("stroke-width", "1.7"),
        ("stroke-linecap", "round"),
        ("stroke-linejoin", "round"),
        ("aria-hidden", "true"),
        ("focusable", "false"),
        ("class", class),
    ] {
        svg.set_attribute(name, value)?;
    }
    svg.set_inner_html(markup);
    Ok(svg)
}

/// Marks `target` as something the remote can rest on.
fn focusable(target: &Element, id: &str) -> Result<(), JsValue> {
    target.set_attribute("data-focus-id", id)
}

/// The title and, when there is one, the preset selector.
fn head(document: &Document, data: &ConfigureData) -> Result<Element, JsValue> {
    let head = element(document, "header", "cfg-head")?;
    append(
        &head,
        &text_element(document, "h2", "cfg-title", &data.title)?,
    )?;
    if let Some(presets) = &data.presets {
        append(&head, &preset(document, presets)?)?;
    }
    Ok(head)
}

fn preset(document: &Document, presets: &PresetSelect) -> Result<Element, JsValue> {
    let group = element(document, "div", "cfg-preset")?;
    append(
        &group,
        &text_element(document, "span", "cfg-preset-label", &presets.label)?,
    )?;
    let selector = choice(
        document,
        &presets.label,
        &presets.value,
        "cfg-choice cfg-choice-preset",
    )?;
    focusable(&selector, ConfigureData::PRESET_FOCUS_ID)?;
    append(&group, &selector)?;
    Ok(group)
}

/// A choice drawn as `< value >`: the chosen value between two arrows, no control behind it.
fn choice(document: &Document, label: &str, value: &str, class: &str) -> Result<Element, JsValue> {
    let choice = element(document, "div", class)?;
    choice.set_attribute("role", "group")?;
    choice.set_attribute("aria-label", &format!("{label} {value}"))?;
    append(&choice, &mark(document, ARROW_LEFT, "cfg-arrow")?)?;
    append(
        &choice,
        &text_element(document, "span", "cfg-choice-value", value)?,
    )?;
    append(&choice, &mark(document, ARROW_RIGHT, "cfg-arrow")?)?;
    Ok(choice)
}

fn card_view(
    document: &Document,
    card: &SettingCard,
    number: usize,
    currency: &str,
) -> Result<Element, JsValue> {
    let section = element(
        document,
        "section",
        &format!(
            "cfg-card ui-lit-quiet cfg-kind-{} cfg-{}",
            card.kind.key(),
            card.accent.key()
        ),
    )?;
    section.set_attribute("aria-label", &card.label)?;
    section.set_attribute("data-card", &card.key)?;
    focusable(&section, &ConfigureData::card_focus_id(number))?;
    if card.kind == CardKind::Select {
        append(&section, &banner(document, card)?)?;
    }
    append(&section, &card_head(document, card)?)?;
    match card.kind {
        CardKind::Select => select_card(document, card, &section)?,
        CardKind::Sliders => sliders_card(document, card, number, &section)?,
        CardKind::Mixed => mixed_card(document, card, number, &section)?,
        CardKind::Budget => budget_card(document, card, number, currency, &section)?,
    }
    Ok(section)
}

/// A card as a tile, for a small slot: its heading and its first drawn row in the same large type.
/// What the tile leaves out stays in the data.
fn tile_view(
    document: &Document,
    card: &SettingCard,
    number: usize,
    currency: &str,
) -> Result<Element, JsValue> {
    let section = element(
        document,
        "section",
        &format!("cfg-card ui-lit-quiet cfg-tile cfg-{}", card.accent.key()),
    )?;
    section.set_attribute("aria-label", &card.label)?;
    section.set_attribute("data-card", &card.key)?;
    focusable(&section, &ConfigureData::card_focus_id(number))?;
    append(
        &section,
        &text_element(document, "h3", "cfg-card-label", &card.label)?,
    )?;
    match card.drawn_rows().first() {
        Some(Row::Select(select)) => append(
            &section,
            &choice(
                document,
                &select.label,
                &select.value,
                "cfg-choice cfg-choice-tile",
            )?,
        )?,
        Some(Row::Slider(slider)) => append(&section, &slider_control(document, slider)?)?,
        Some(Row::Stat(stat)) => {
            if let Some(value) = &stat.value {
                let class = if stat.highlight {
                    "cfg-stat-value cfg-stat-total"
                } else {
                    "cfg-stat-value"
                };
                append(
                    &section,
                    &text_element(document, "span", class, &value.text(currency))?,
                )?;
            }
        }
        None => {}
    }
    Ok(section)
}

/// The scene art behind a select card's top, with a mask so it fades into the card.
fn banner(document: &Document, card: &SettingCard) -> Result<Element, JsValue> {
    let frame = element(document, "div", "cfg-banner")?;
    if let Some(picture) = photo(
        document,
        "cfg-banner-image",
        card.banner.as_deref(),
        &card.label,
    )? {
        append(&frame, &picture)?;
    }
    Ok(frame)
}

fn card_head(document: &Document, card: &SettingCard) -> Result<Element, JsValue> {
    let head = element(document, "header", "cfg-card-head")?;
    if let Some(icon) = card.icon {
        let badge = element(document, "span", "cfg-badge")?;
        append(
            &badge,
            &mark(
                document,
                icon.markup(),
                &format!("cfg-icon cfg-icon-{}", icon.key()),
            )?,
        )?;
        append(&head, &badge)?;
    }
    append(
        &head,
        &text_element(document, "h3", "cfg-card-label", &card.label)?,
    )?;
    Ok(head)
}

/// The SelectCard: the banner, the title and one choice in large type.
fn select_card(document: &Document, card: &SettingCard, section: &Element) -> Result<(), JsValue> {
    for row in card.drawn_rows() {
        if let Row::Select(select) = row {
            append(
                section,
                &choice(
                    document,
                    &select.label,
                    &select.value,
                    "cfg-choice cfg-choice-card",
                )?,
            )?;
        }
    }
    Ok(())
}

/// The SlidersCard: a labelled bar per drawn row.
fn sliders_card(
    document: &Document,
    card: &SettingCard,
    number: usize,
    section: &Element,
) -> Result<(), JsValue> {
    mixed_card(document, card, number, section)
}

/// The MixedCard: choices and bars in one card, each with its label above it.
fn mixed_card(
    document: &Document,
    card: &SettingCard,
    number: usize,
    section: &Element,
) -> Result<(), JsValue> {
    let rows = element(document, "div", "cfg-rows")?;
    for (index, row) in card.drawn_rows().into_iter().enumerate() {
        let line = element(document, "div", "cfg-row")?;
        append(
            &line,
            &text_element(document, "span", "cfg-row-label", row.label())?,
        )?;
        match row {
            Row::Select(select) => append(&line, &select_control(document, select)?)?,
            Row::Slider(slider) => append(&line, &slider_control(document, slider)?)?,
            Row::Stat(_) => continue,
        }
        focusable(&line, &ConfigureData::row_focus_id(number, index + 1))?;
        append(&rows, &line)?;
    }
    append(section, &rows)
}

fn select_control(document: &Document, select: &SelectRow) -> Result<Element, JsValue> {
    choice(
        document,
        &select.label,
        &select.value,
        "cfg-choice cfg-choice-row",
    )
}

/// A slider as a read-out: a bar of ten segments, the lit ones counted from the left, and the
/// value.
fn slider_control(document: &Document, slider: &SliderRow) -> Result<Element, JsValue> {
    let control = element(document, "div", "cfg-slider")?;
    control.set_attribute("role", "meter")?;
    control.set_attribute("aria-label", &slider.label)?;
    control.set_attribute("aria-valuemin", "0")?;
    control.set_attribute("aria-valuemax", "100")?;
    control.set_attribute("aria-valuenow", &slider.value.to_string())?;
    control.set_attribute("aria-valuetext", &slider.read_out())?;
    let bar = element(document, "span", "cfg-bar")?;
    let lit = slider.lit_segments();
    for segment in 0..SLIDER_SEGMENTS {
        let class = if segment < lit {
            "cfg-seg cfg-seg-on"
        } else {
            "cfg-seg"
        };
        append(&bar, &element(document, "span", class)?)?;
    }
    append(&control, &bar)?;
    append(
        &control,
        &text_element(document, "span", "cfg-value", &slider.read_out())?,
    )?;
    Ok(control)
}

/// The BudgetCard: labelled amounts, only the ones that have a value.
fn budget_card(
    document: &Document,
    card: &SettingCard,
    number: usize,
    currency: &str,
    section: &Element,
) -> Result<(), JsValue> {
    let rows = element(document, "div", "cfg-rows")?;
    for (index, row) in card.drawn_rows().into_iter().enumerate() {
        if let Row::Stat(stat) = row {
            let line = stat_row(document, stat, currency)?;
            focusable(&line, &ConfigureData::row_focus_id(number, index + 1))?;
            append(&rows, &line)?;
        }
    }
    append(section, &rows)
}

fn stat_row(document: &Document, stat: &StatRow, currency: &str) -> Result<Element, JsValue> {
    let line = element(document, "div", "cfg-row cfg-stat")?;
    let label = element(document, "span", "cfg-row-label cfg-stat-label")?;
    append(&label, &text_element(document, "span", "", &stat.label)?)?;
    if stat.info {
        let info = mark(document, INFO, "cfg-info")?;
        info.set_attribute("role", "img")?;
        info.set_attribute("aria-label", &format!("About {}", stat.label))?;
        info.remove_attribute("aria-hidden")?;
        append(&label, &info)?;
    }
    append(&line, &label)?;
    if let Some(value) = &stat.value {
        let class = if stat.highlight {
            "cfg-stat-value cfg-stat-total"
        } else {
            "cfg-stat-value"
        };
        append(
            &line,
            &text_element(document, "span", class, &value.text(currency))?,
        )?;
    }
    Ok(line)
}

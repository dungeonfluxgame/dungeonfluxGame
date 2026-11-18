//! The blocks of a phone page as elements (`phone_kit::page`): each turns one block's input into
//! page elements and decides nothing; the decisions are the model's.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, set_action, set_style, text_element};
use crate::kit::Rarity;
use crate::overlay::Glyph;
use crate::phone_kit::draw::{glyph, shell};
use crate::phone_kit::glyph::PhoneGlyph;
use crate::phone_kit::page::{
    Block, Chip, Grid, MapNode, MapWay, Mark, NodeState, PickCard, Row, SliderRange, StageState,
    Stat, StripItem, Swatch, TALLY_MARKS, Token, bar_share, tally_text,
};
use crate::phone_kit::shell::{ButtonSpec, Pill, PillTone};
use crate::speaker_card::PortraitCrop;
use crate::style::css_color;
use crate::view::icons::{Icon, icon};
use crate::view::kit::{glyph as overlay_glyph, photo};

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

fn rarity_class(rarity: Option<Rarity>) -> String {
    rarity.map_or_else(String::new, |rarity| {
        format!(" ph-rarity ph-rarity-{}", rarity.key())
    })
}

/// A picture of the media library, else a glyph, else the first letter of `name`, in a frame.
fn art(
    document: &Document,
    class: &str,
    media: Option<&str>,
    drawing: Option<Glyph>,
    name: &str,
) -> Result<Element, JsValue> {
    let frame = element(document, "span", class)?;
    if let Some(image) = photo(document, "ph-art-image", media, name)? {
        append(&frame, &image)?;
    } else if let Some(drawing) = drawing {
        append(&frame, &overlay_glyph(document, drawing, "ph-art-glyph")?)?;
    } else {
        let initial: String = name.chars().take(1).collect();
        append(
            &frame,
            &text_element(document, "span", "ph-art-initial", &initial)?,
        )?;
    }
    Ok(frame)
}

/// A card's face cut from a room picture: the whole picture, sized and moved inside the frame so
/// only `crop` shows (`PortraitCrop::frame`), as the shared screen's speaker card cuts it.
fn cut_art(
    document: &Document,
    media: &str,
    crop: PortraitCrop,
    name: &str,
) -> Result<Element, JsValue> {
    let frame = element(document, "span", "ph-pcard-art ph-art-cut")?;
    let image = element(document, "img", "ph-art-cut-image")?;
    image.set_attribute("src", &crate::path::media_file(media))?;
    image.set_attribute("alt", name)?;
    let (width, height, left, top) = crop.frame();
    set_style(
        &image,
        &format!("width:{width:.2}%;height:{height:.2}%;left:{left:.2}%;top:{top:.2}%;"),
    )?;
    append(&frame, &image)?;
    Ok(frame)
}

fn mark(document: &Document, mark: &Mark) -> Result<Element, JsValue> {
    shell::pill(
        document,
        &Pill {
            text: mark.text.clone(),
            tone: mark.tone,
            glyph: None,
        },
    )
}

fn card(document: &Document, card: &PickCard, small: bool) -> Result<Element, JsValue> {
    let mut class = String::from("ph-glass ph-pcard");
    if small {
        class.push_str(" ph-pcard-small");
    }
    if card.selected {
        class.push_str(" ph-lit ph-pcard-on");
    }
    if card.off {
        class.push_str(" ph-pcard-off");
    }
    class.push_str(&rarity_class(card.rarity));
    let button = element(document, "button", &class)?;
    button.set_attribute("type", "button")?;
    if let Some(action) = &card.action {
        set_action(&button, action)?;
    }
    let picture = match (card.picture.as_deref(), card.crop) {
        (Some(media), Some(crop)) => cut_art(document, media, crop, &card.title)?,
        (media, _) => art(document, "ph-pcard-art", media, card.glyph, &card.title)?,
    };
    append(&button, &picture)?;
    append(
        &button,
        &text_element(document, "span", "ph-pcard-title", &card.title)?,
    )?;
    if let Some(sub) = &card.sub {
        append(
            &button,
            &text_element(document, "span", "ph-pcard-sub", sub)?,
        )?;
    }
    if let Some(note) = &card.mark {
        append(&button, &mark(document, note)?)?;
    }
    Ok(button)
}

fn row(document: &Document, row: &Row) -> Result<Element, JsValue> {
    let line = element(document, "div", "ph-glass ph-row")?;
    if let Some(picture) = &row.picture {
        append(
            &line,
            &art(document, "ph-row-art", Some(picture), None, &row.label)?,
        )?;
    }
    let text = element(document, "div", "ph-row-text")?;
    append(
        &text,
        &text_element(document, "div", "ph-row-label", &row.label)?,
    )?;
    if let Some(value) = &row.value {
        append(
            &text,
            &text_element(document, "div", "ph-row-value", value)?,
        )?;
    }
    if let Some(because) = &row.because {
        append(
            &text,
            &text_element(document, "div", "ph-row-because", because)?,
        )?;
    }
    if let Some(note) = &row.mark {
        append(&text, &mark(document, note)?)?;
    }
    append(&line, &text)?;
    if let Some(change) = &row.change {
        let button = text_element(
            document,
            "button",
            "ph-btn ph-btn-secondary ph-btn-small",
            change,
        )?;
        button.set_attribute("type", "button")?;
        if let Some(action) = &row.action {
            set_action(&button, action)?;
        }
        append(&line, &button)?;
    }
    Ok(line)
}

fn chip(document: &Document, chip: &Chip) -> Result<Element, JsValue> {
    let class = if chip.selected {
        "ph-chip ph-lit"
    } else {
        "ph-chip"
    };
    let button = text_element(document, "button", class, &chip.text)?;
    button.set_attribute("type", "button")?;
    if let Some(action) = &chip.action {
        set_action(&button, action)?;
    }
    if chip.removable {
        append(
            &button,
            &glyph(document, PhoneGlyph::Close, "ph-chip-remove")?,
        )?;
        button.set_attribute("aria-label", &format!("Remove {}", chip.text))?;
    }
    Ok(button)
}

fn labelled(document: &Document, class: &str, label: &str) -> Result<Element, JsValue> {
    let wrap = element(document, "section", class)?;
    append(
        &wrap,
        &text_element(document, "h3", "ph-group-name", label)?,
    )?;
    Ok(wrap)
}

fn slider(
    document: &Document,
    label: &str,
    percent: u8,
    (low, high): (&str, &str),
    range: Option<&SliderRange>,
) -> Result<Element, JsValue> {
    let wrap = labelled(document, "ph-slider", label)?;
    let track = element(document, "div", "ph-slider-track")?;
    track.set_attribute("role", "img")?;
    track.set_attribute(
        "aria-label",
        &format!("{label}: {} of 100", bar_share(percent)),
    )?;
    let fill = element(document, "div", "ph-slider-fill")?;
    set_style(&fill, &format!("width:{}%", bar_share(percent)))?;
    append(&track, &fill)?;
    let thumb = element(document, "div", "ph-slider-thumb")?;
    set_style(&thumb, &format!("left:{}%", bar_share(percent)))?;
    append(&track, &thumb)?;
    if let Some(range) = range {
        let input = element(document, "input", "ph-slider-input")?;
        input.set_attribute("type", "range")?;
        input.set_attribute("min", &range.min.to_string())?;
        input.set_attribute("max", &range.max.to_string())?;
        input.set_attribute("step", "1")?;
        input.set_attribute("value", &range.value.to_string())?;
        input.set_attribute("aria-label", label)?;
        set_action(&input, &range.action)?;
        append(&track, &input)?;
    }
    append(&wrap, &track)?;
    let ends = element(document, "div", "ph-slider-ends")?;
    append(&ends, &text_element(document, "span", "", low)?)?;
    append(&ends, &text_element(document, "span", "", high)?)?;
    append(&wrap, &ends)?;
    Ok(wrap)
}

fn swatch(document: &Document, swatch: &Swatch) -> Result<Element, JsValue> {
    let class = if swatch.selected {
        "ph-swatch ph-swatch-on"
    } else {
        "ph-swatch"
    };
    let button = element(document, "button", class)?;
    button.set_attribute("type", "button")?;
    button.set_attribute("aria-label", &swatch.name)?;
    if let Some(color) = css_color(&swatch.color) {
        set_style(&button, &format!("--swatch:{color}"))?;
    }
    if let Some(action) = &swatch.action {
        set_action(&button, action)?;
    }
    Ok(button)
}

fn field(
    document: &Document,
    label: &str,
    value: &str,
    placeholder: &str,
    message: Option<&str>,
    (tall, id): (bool, Option<&str>),
) -> Result<Element, JsValue> {
    let wrap = labelled(document, "ph-field", label)?;
    let input = element(
        document,
        if tall { "textarea" } else { "input" },
        "ph-input",
    )?;
    input.set_attribute("aria-label", label)?;
    input.set_attribute("placeholder", placeholder)?;
    if let Some(id) = id {
        input.set_attribute("id", id)?;
    }
    if tall {
        input.set_attribute("rows", "3")?;
        input.set_text_content(Some(value));
    } else {
        input.set_attribute("type", "text")?;
        input.set_attribute("value", value)?;
    }
    append(&wrap, &input)?;
    if let Some(words) = message {
        append(
            &wrap,
            &text_element(document, "p", "ph-field-message", words)?,
        )?;
    }
    Ok(wrap)
}

fn stat(document: &Document, stat: &Stat) -> Result<Element, JsValue> {
    let class = if stat.spent {
        "ph-glass ph-stat ph-stat-spent"
    } else {
        "ph-glass ph-stat"
    };
    let cell = element(document, "div", class)?;
    append(
        &cell,
        &text_element(document, "div", "ph-stat-value", &stat.value)?,
    )?;
    append(
        &cell,
        &text_element(document, "div", "ph-stat-label", &stat.label)?,
    )?;
    if let Some(sub) = &stat.sub {
        append(&cell, &text_element(document, "div", "ph-stat-sub", sub)?)?;
    }
    Ok(cell)
}

fn meter(
    document: &Document,
    label: &str,
    percent: u8,
    tone: PillTone,
) -> Result<Element, JsValue> {
    let wrap = element(
        document,
        "div",
        &format!("ph-meter ph-meter-{}", tone.key()),
    )?;
    append(
        &wrap,
        &text_element(document, "span", "ph-meter-label", label)?,
    )?;
    let track = element(document, "div", "ph-meter-track")?;
    let fill = element(document, "div", "ph-meter-fill")?;
    set_style(&fill, &format!("width:{}%", bar_share(percent)))?;
    append(&track, &fill)?;
    append(&wrap, &track)?;
    Ok(wrap)
}

fn stage(
    document: &Document,
    (picture, state): (Option<&str>, StageState),
    caption: &str,
    action: Option<&str>,
) -> Result<Element, JsValue> {
    let wrap = element(document, "figure", "ph-stage-wrap")?;
    let stage = element(document, "div", "ph-stage")?;
    if let Some(action) = action {
        set_action(&stage, action)?;
    }
    stage.set_attribute(
        "data-state",
        match state {
            StageState::Ready => "ready",
            StageState::Generating => "generating",
            StageState::Failed => "failed",
        },
    )?;
    match photo(document, "ph-stage-image", picture, caption)? {
        Some(image) => append(&stage, &image)?,
        None => append(
            &stage,
            &icon(document, Icon::Silhouette, "ph-stage-silhouette")?,
        )?,
    }
    if state == StageState::Generating {
        append(&stage, &shell::loader(document)?)?;
    }
    append(&wrap, &stage)?;
    append(
        &wrap,
        &text_element(document, "figcaption", "ph-stage-caption", caption)?,
    )?;
    Ok(wrap)
}

fn compare(
    document: &Document,
    (kept, fresh): (&str, &str),
    caption: &str,
    actions: Option<&[String; 2]>,
) -> Result<Element, JsValue> {
    let wrap = element(document, "figure", "ph-compare")?;
    let pair = element(document, "div", "ph-compare-pair")?;
    for (index, (label, media, class)) in [
        ("Kept", kept, "ph-compare-old"),
        ("New", fresh, "ph-compare-new"),
    ]
    .into_iter()
    .enumerate()
    {
        let side = element(
            document,
            "div",
            &format!("ph-glass ph-compare-side {class}"),
        )?;
        if let Some(action) = actions.and_then(|actions| actions.get(index)) {
            set_action(&side, action)?;
        }
        append(
            &side,
            &art(document, "ph-compare-art", Some(media), None, label)?,
        )?;
        append(
            &side,
            &text_element(document, "div", "ph-compare-label", label)?,
        )?;
        append(&pair, &side)?;
    }
    append(&wrap, &pair)?;
    append(
        &wrap,
        &text_element(document, "figcaption", "ph-stage-caption", caption)?,
    )?;
    Ok(wrap)
}

fn strip(document: &Document, label: &str, items: &[StripItem]) -> Result<Element, JsValue> {
    let wrap = labelled(document, "ph-strip", label)?;
    let row = element(document, "div", "ph-strip-row")?;
    for item in items {
        let class = if item.selected {
            "ph-strip-item ph-strip-on"
        } else {
            "ph-strip-item"
        };
        let cell = element(document, "button", class)?;
        cell.set_attribute("type", "button")?;
        if let Some(action) = &item.action {
            set_action(&cell, action)?;
        }
        append(
            &cell,
            &art(
                document,
                "ph-strip-art",
                item.picture.as_deref(),
                None,
                &item.label,
            )?,
        )?;
        append(
            &cell,
            &text_element(document, "span", "ph-strip-label", &item.label)?,
        )?;
        append(&row, &cell)?;
    }
    append(&wrap, &row)?;
    Ok(wrap)
}

fn notice(document: &Document, tone: PillTone, text: &str) -> Result<Element, JsValue> {
    let wrap = element(
        document,
        "div",
        &format!("ph-notice ph-notice-{}", tone.key()),
    )?;
    wrap.set_attribute("role", "status")?;
    let drawing = match tone {
        PillTone::Warn | PillTone::Danger => PhoneGlyph::Warning,
        _ => PhoneGlyph::Inbox,
    };
    append(&wrap, &glyph(document, drawing, "ph-notice-glyph")?)?;
    append(&wrap, &text_element(document, "span", "", text)?)?;
    Ok(wrap)
}

fn map_line(
    document: &Document,
    from: [u8; 2],
    to: [u8; 2],
    blocked: bool,
) -> Result<Element, JsValue> {
    let line = document.create_element_ns(Some(SVG_NAMESPACE), "line")?;
    for (name, value) in [
        ("x1", from[0].to_string()),
        ("y1", from[1].to_string()),
        ("x2", to[0].to_string()),
        ("y2", to[1].to_string()),
        ("vector-effect", "non-scaling-stroke".to_owned()),
        (
            "class",
            if blocked {
                "ph-way ph-way-blocked"
            } else {
                "ph-way"
            }
            .to_owned(),
        ),
    ] {
        line.set_attribute(name, &value)?;
    }
    Ok(line)
}

fn map(document: &Document, nodes: &[MapNode], ways: &[MapWay]) -> Result<Element, JsValue> {
    let wrap = element(document, "div", "ph-glass ph-map")?;
    let lines = document.create_element_ns(Some(SVG_NAMESPACE), "svg")?;
    lines.set_attribute("viewBox", "0 0 100 100")?;
    lines.set_attribute("preserveAspectRatio", "none")?;
    lines.set_attribute("class", "ph-ways")?;
    lines.set_attribute("aria-hidden", "true")?;
    for way in ways {
        if let Some((from, to)) = way.ends(nodes) {
            append(&lines, &map_line(document, from, to, way.blocked)?)?;
        }
    }
    append(&wrap, &lines)?;
    for place in nodes {
        let node = element(
            document,
            "button",
            &format!("ph-node ph-node-{}", place.state.key()),
        )?;
        node.set_attribute("type", "button")?;
        if let Some(action) = &place.action {
            set_action(&node, action)?;
        }
        set_style(
            &node,
            &format!("left:{}%;top:{}%", place.x.min(100), place.y.min(100)),
        )?;
        append(
            &node,
            &art(
                document,
                "ph-node-art",
                place.plate.as_deref(),
                None,
                &place.name,
            )?,
        )?;
        append(
            &node,
            &text_element(document, "span", "ph-node-name", &place.name)?,
        )?;
        match place.state {
            NodeState::Here => append(
                &node,
                &text_element(document, "span", "ph-node-note", "You are here")?,
            )?,
            NodeState::Unvisited => append(
                &node,
                &text_element(document, "span", "ph-node-note", "Not visited")?,
            )?,
            NodeState::Visited => {}
        }
        append(&wrap, &node)?;
    }
    Ok(wrap)
}

fn token(document: &Document, token: &Token) -> Result<Element, JsValue> {
    let figure = element(
        document,
        "span",
        &format!("ph-token ph-token-{}", token.side.key()),
    )?;
    figure.set_attribute("title", &token.name)?;
    append(
        &figure,
        &art(
            document,
            "ph-token-art",
            token.portrait.as_deref(),
            None,
            &token.name,
        )?,
    )?;
    Ok(figure)
}

fn grid(document: &Document, grid: &Grid) -> Result<Element, JsValue> {
    let wrap = element(document, "div", "ph-glass ph-grid-wrap")?;
    let (columns, rows) = grid.size();
    let board = element(document, "div", "ph-grid")?;
    set_style(&board, &format!("--cols:{columns}"))?;
    for row in 0..rows {
        for column in 0..columns {
            let mut class = format!("ph-sq ph-sq-{}", grid.square(column, row).key());
            if grid.is_lit(column, row) {
                class.push_str(" ph-sq-lit");
            }
            if grid.path_step(column, row).is_some() {
                class.push_str(" ph-sq-path");
            }
            if grid.is_target(column, row) {
                class.push_str(" ph-sq-target");
            }
            let square = element(document, "div", &class)?;
            if let Some(figure) = grid.token_at(column, row) {
                append(&square, &token(document, figure)?)?;
            } else if let Some(step) = grid.path_step(column, row) {
                append(
                    &square,
                    &text_element(document, "span", "ph-step", &step.to_string())?,
                )?;
            }
            append(&board, &square)?;
        }
    }
    append(&wrap, &board)?;
    Ok(wrap)
}

fn inspect(
    document: &Document,
    picture: &str,
    title: &str,
    rarity: Option<Rarity>,
    text: &str,
) -> Result<Element, JsValue> {
    let wrap = element(
        document,
        "figure",
        &format!("ph-inspect{}", rarity_class(rarity)),
    )?;
    append(
        &wrap,
        &art(document, "ph-inspect-art", Some(picture), None, title)?,
    )?;
    let block = element(document, "figcaption", "ph-inspect-block")?;
    append(
        &block,
        &text_element(document, "h2", "ph-inspect-title", title)?,
    )?;
    if let Some(rarity) = rarity {
        append(
            &block,
            &text_element(document, "div", "ph-inspect-rarity", rarity.word())?,
        )?;
    }
    append(
        &block,
        &text_element(document, "p", "ph-inspect-text", text)?,
    )?;
    append(&wrap, &block)?;
    Ok(wrap)
}

fn tally(
    document: &Document,
    label: &str,
    successes: u8,
    failures: u8,
) -> Result<Element, JsValue> {
    let wrap = element(document, "div", "ph-tally")?;
    wrap.set_attribute("role", "img")?;
    wrap.set_attribute("aria-label", &tally_text(successes, failures))?;
    append(
        &wrap,
        &text_element(document, "span", "ph-tally-label", label)?,
    )?;
    for (kind, count) in [("success", successes), ("failure", failures)] {
        let row = element(document, "span", &format!("ph-tally-row ph-tally-{kind}"))?;
        for index in 0..TALLY_MARKS {
            let class = if index < count {
                "ph-pip ph-pip-on"
            } else {
                "ph-pip"
            };
            append(&row, &element(document, "i", class)?)?;
        }
        append(&wrap, &row)?;
    }
    Ok(wrap)
}

fn buttons(document: &Document, specs: &[ButtonSpec]) -> Result<Element, JsValue> {
    let row = element(document, "div", "ph-button-row")?;
    for spec in specs {
        append(&row, &shell::button(document, spec)?)?;
    }
    Ok(row)
}

/// One block of a page.
pub fn block(document: &Document, block: &Block) -> Result<Element, JsValue> {
    match block {
        Block::Heading { text, caption } => {
            let wrap = element(document, "header", "ph-heading")?;
            append(
                &wrap,
                &text_element(document, "h2", "ph-heading-title", text)?,
            )?;
            if let Some(caption) = caption {
                append(
                    &wrap,
                    &text_element(document, "p", "ph-heading-caption", caption)?,
                )?;
            }
            Ok(wrap)
        }
        Block::Caption { text } => {
            let band = element(document, "div", "ph-narration")?;
            append(&band, &text_element(document, "p", "ph-line", text)?)?;
            Ok(band)
        }
        Block::Paragraph { text } => text_element(document, "p", "ph-paragraph", text),
        Block::Cards { cards, small } => {
            let grid = element(document, "div", "ph-pcards")?;
            for item in cards {
                append(&grid, &card(document, item, *small)?)?;
            }
            Ok(grid)
        }
        Block::Rows { rows } => {
            let list = element(document, "div", "ph-rows")?;
            for item in rows {
                append(&list, &row(document, item)?)?;
            }
            Ok(list)
        }
        Block::Chips { label, chips } => {
            let wrap = labelled(document, "ph-group", label)?;
            let row = element(document, "div", "ph-chips")?;
            for item in chips {
                append(&row, &chip(document, item)?)?;
            }
            append(&wrap, &row)?;
            Ok(wrap)
        }
        Block::Slider {
            label,
            percent,
            low,
            high,
            range,
        } => slider(document, label, *percent, (low, high), range.as_ref()),
        Block::Swatches { label, swatches } => {
            let wrap = labelled(document, "ph-group", label)?;
            let row = element(document, "div", "ph-swatches")?;
            for item in swatches {
                append(&row, &swatch(document, item)?)?;
            }
            append(&wrap, &row)?;
            Ok(wrap)
        }
        Block::Field {
            label,
            value,
            placeholder,
            message,
            tall,
            id,
        } => field(
            document,
            label,
            value,
            placeholder,
            message.as_deref(),
            (*tall, id.as_deref()),
        ),
        Block::Stats { stats } => {
            let grid = element(document, "div", "ph-stats")?;
            for item in stats {
                append(&grid, &stat(document, item)?)?;
            }
            Ok(grid)
        }
        Block::Meter {
            label,
            percent,
            tone,
        } => meter(document, label, *percent, *tone),
        Block::Stage {
            picture,
            state,
            caption,
            action,
        } => stage(
            document,
            (picture.as_deref(), *state),
            caption,
            action.as_deref(),
        ),
        Block::Compare {
            kept,
            fresh,
            caption,
            actions,
        } => compare(document, (kept, fresh), caption, actions.as_ref()),
        Block::Strip { label, items } => strip(document, label, items),
        Block::Buttons { buttons: specs } => buttons(document, specs),
        Block::Notice { tone, text } => notice(document, *tone, text),
        Block::Map { nodes, ways } => map(document, nodes, ways),
        Block::Grid(board) => grid(document, board),
        Block::Inspect {
            picture,
            title,
            rarity,
            text,
        } => inspect(document, picture, title, *rarity, text),
        Block::Tally {
            label,
            successes,
            failures,
        } => tally(document, label, *successes, *failures),
    }
}

//! The browser drawing of the kit: each child turns one component's input into elements and
//! decides nothing (the decisions are the component modules'). This module holds the pieces they
//! share: the kit's stylesheet, the glass panel and the small marks.
//!
//! The glass follows the measured recipe of inventory F2: a panel is a root holding three
//! siblings, a wide blurred halo ring outside the outline, a narrow bloom ring hugging the rim and
//! the pane that blurs the plate (`backdrop-filter`) and carries the crisp rim. The pane is shaped
//! by a mask on itself, which also cuts its blurred backdrop, never by a clip. No filter, clip,
//! mask or opacity is ever set on the root or any ancestor of a pane, or the blur would sample
//! nothing (F1); a fade animates the halo, the bloom and the pane themselves (F3).
//!
//! The screen is laid out in two stacks, so panels push each other instead of covering: the top
//! bar (brand, headers and clue board at the left, the encounter banner in the centre, the chips
//! and the lists at the right, the twist card under them) and the bottom stack (speaker card,
//! narration, choices, caption, nameplate, section label, party, progress line), each above the
//! next.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, set_style};
use crate::home::glyph::HomeIcon;
use crate::kit::banner::BannerPlace;
use crate::kit::speech::TailSide;
use crate::kit::{Fill, KitPanels, Step};
use crate::view::icons::Icon;

pub mod banner;
pub mod brand;
pub mod caption_bar;
pub mod choices;
pub mod clue_board;
pub mod journal;
pub mod ledger;
pub mod nameplate;
pub mod narration;
pub mod party;
pub mod place_label;
pub mod progress_line;
pub mod section_label;
pub mod speaker;
pub mod speech;
pub mod status;
pub mod title_card;
pub mod title_chip;

const KIT_STYLE_ID: &str = "kit-style";
/// The kit's stylesheet. The host serves only `theme.css` of the playground's styles, so the kit's
/// rules travel in the package and are put in the page once.
const KIT_CSS: &str = include_str!("../../../../assets/playground/kit.css");

/// The outline of a glass panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Chamfered corners: the narration box, the party bar, the lists.
    Octagon,
    /// Pointed ends: the choice rows and the encounter banner.
    Hexagon,
    /// Round ends: the title chip and the status.
    Capsule,
    /// A chamfered left end and a feathered right: the scene header.
    Tab,
    /// A plain band with hairline rules top and bottom: the caption bar.
    Band,
    /// An octagon with a tail: a speech bubble.
    Tailed(TailSide),
}

impl Shape {
    fn dom_marker(self) -> &'static str {
        match self {
            Self::Octagon => "oct",
            Self::Hexagon => "hex",
            Self::Capsule => "capsule",
            Self::Tab => "tab",
            Self::Band => "band",
            Self::Tailed(TailSide::Below) => "tail-below",
            Self::Tailed(TailSide::Above) => "tail-above",
            Self::Tailed(TailSide::Left) => "tail-left",
            Self::Tailed(TailSide::Right) => "tail-right",
        }
    }
}

/// Puts the kit's stylesheet in the page once.
fn ensure_style(document: &Document) -> Result<(), JsValue> {
    if document.get_element_by_id(KIT_STYLE_ID).is_some() {
        return Ok(());
    }
    let body = document
        .body()
        .ok_or_else(|| JsValue::from_str("the page has no body"))?;
    let style = element(document, "style", "")?;
    style.set_attribute("id", KIT_STYLE_ID)?;
    style.set_text_content(Some(KIT_CSS));
    append(&body, &style)
}

/// A glass panel of `shape` and `fill` with the classes `class`. Returns `(root, pane)`: lay out
/// and tone the root, put the content in the pane.
pub fn glass(
    document: &Document,
    shape: Shape,
    fill: Fill,
    class: &str,
) -> Result<(Element, Element), JsValue> {
    let solid = match fill {
        Fill::Blurred => "",
        Fill::Fine => " kit-fine",
        Fill::Solid => " kit-solid",
    };
    let root = element(
        document,
        "div",
        &format!("kit-glass kit-shape-{}{solid} {class}", shape.dom_marker()),
    )?;
    for (wrapper, ring) in [
        ("kit-halo", "kit-halo-ring"),
        ("kit-bloom", "kit-bloom-ring"),
    ] {
        let glow = element(document, "div", wrapper)?;
        glow.set_attribute("aria-hidden", "true")?;
        append(&glow, &element(document, "div", ring)?)?;
        append(&root, &glow)?;
    }
    let pane = element(document, "div", "kit-pane")?;
    append(&root, &pane)?;
    Ok((root, pane))
}

/// The eight-point compass rose of the brand crest and the title cards: the four-point compass
/// drawn twice, the second turned an eighth and smaller behind the first.
pub fn compass_rose(document: &Document, class: &str) -> Result<Element, JsValue> {
    let rose = element(document, "span", &format!("kit-rose {class}"))?;
    rose.set_attribute("aria-hidden", "true")?;
    for part in ["kit-rose-back", "kit-rose-front"] {
        append(
            &rose,
            &crate::view::icons::icon(document, Icon::Compass, part)?,
        )?;
    }
    Ok(rose)
}

/// A drawing of either icon set.
pub fn icon(document: &Document, which: HomeIcon, class: &str) -> Result<Element, JsValue> {
    crate::view::home::kit::icon(document, which, class)
}

/// The hairline rule with a centre diamond.
pub fn diamond_rule(document: &Document, class: &str) -> Result<Element, JsValue> {
    crate::view::home::kit::diamond_rule(document, &format!("kit-rule {class}"))
}

/// A row of track marks, one per step, the current one with a diamond when `diamond` is set.
pub fn track(
    document: &Document,
    class: &str,
    mark_class: &str,
    steps: &[Step],
    diamond: bool,
) -> Result<Element, JsValue> {
    let row = element(document, "div", class)?;
    for step in steps {
        let mark = element(
            document,
            "span",
            &format!("{mark_class} kit-step-{}", step.dom_marker()),
        )?;
        if diamond && *step == Step::Current {
            append(&mark, &element(document, "span", "kit-step-diamond")?)?;
        }
        append(&row, &mark)?;
    }
    Ok(row)
}

/// The heights of the voice wave's bars in percent of its height: a waveform that swells to the
/// middle and tapers to its ends.
const WAVE_BAR_HEIGHTS: [u8; 13] = [22, 36, 50, 66, 84, 100, 90, 76, 60, 46, 34, 24, 16];

/// The voice wave: bars that move while a line is spoken and stand still when it is silent.
pub fn voice_wave(document: &Document, speaking: bool) -> Result<Element, JsValue> {
    let wave = element(
        document,
        "span",
        if speaking {
            "kit-wave kit-wave-on"
        } else {
            "kit-wave"
        },
    )?;
    wave.set_attribute("aria-hidden", "true")?;
    for height in WAVE_BAR_HEIGHTS {
        let bar = element(document, "span", "kit-wave-bar")?;
        set_style(&bar, &format!("height:{height}%"))?;
        append(&wave, &bar)?;
    }
    Ok(wave)
}

/// The thinking cue's three dots.
pub fn thinking_dots(document: &Document) -> Result<Element, JsValue> {
    let dots = element(document, "span", "kit-dots")?;
    dots.set_attribute("aria-hidden", "true")?;
    for _ in 0..3 {
        append(&dots, &element(document, "span", "kit-dot")?)?;
    }
    Ok(dots)
}

/// Appends `child` when there is one.
pub fn put(parent: &Element, child: Option<Element>) -> Result<(), JsValue> {
    match child {
        Some(child) => append(parent, &child),
        None => Ok(()),
    }
}

/// Appends `container` to `parent` when anything was put in it.
fn put_filled(parent: &Element, container: &Element) -> Result<(), JsValue> {
    if container.child_element_count() == 0 {
        return Ok(());
    }
    append(parent, container)
}

/// The top bar: the brand, the scene headers and the clue board at the left, the encounter
/// banner in the centre, the status and chips over the lists at the right, and the twist card in
/// a row of its own under the bar, so a long title wraps instead of running into its neighbour.
fn top(document: &Document, panels: &KitPanels) -> Result<Element, JsValue> {
    let top = element(document, "div", "kit-top")?;
    let row = element(document, "div", "kit-top-row")?;
    let left = element(document, "div", "kit-top-left")?;
    if let Some(mark) = &panels.brand {
        append(&left, &brand::draw(document, mark)?)?;
    }
    let centre = element(document, "div", "kit-top-centre")?;
    for banner in &panels.banners {
        let slot = match banner.placement() {
            BannerPlace::Header => &left,
            BannerPlace::TopCenter => &centre,
        };
        put(slot, banner::draw(document, banner)?)?;
    }
    if let Some(clues) = &panels.clues {
        put(&left, clue_board::draw(document, clues)?)?;
    }
    append(&row, &left)?;
    append(&row, &centre)?;
    let right = element(document, "div", "kit-top-right")?;
    let chips = element(document, "div", "kit-chips")?;
    if let Some(status) = &panels.status {
        put(&chips, status::draw(document, status)?)?;
    }
    for chip in &panels.chips {
        put(&chips, title_chip::draw(document, chip)?)?;
    }
    put_filled(&right, &chips)?;
    let column = element(document, "div", "kit-column")?;
    for ledger in &panels.ledgers {
        put(&column, ledger::draw(document, ledger)?)?;
    }
    put_filled(&right, &column)?;
    append(&row, &right)?;
    append(&top, &row)?;
    if let Some(twist) = panels
        .narration
        .as_ref()
        .filter(|twist| twist.sits_on_top())
    {
        append(&top, &narration::draw(document, twist)?)?;
    }
    Ok(top)
}

/// The bottom stack, top to bottom.
fn bottom(document: &Document, panels: &KitPanels) -> Result<Element, JsValue> {
    let stack = element(document, "div", "kit-bottom")?;
    if let Some(speaker) = &panels.speaker {
        append(&stack, &speaker::draw(document, speaker)?)?;
    }
    if let Some(told) = panels.narration.as_ref().filter(|told| !told.sits_on_top()) {
        append(&stack, &narration::draw(document, told)?)?;
    }
    if let Some(choices) = &panels.choices {
        put(&stack, choices::draw(document, choices)?)?;
    }
    if let Some(caption) = &panels.caption {
        put(&stack, caption_bar::draw(document, caption)?)?;
    }
    if let Some(nameplate) = &panels.nameplate {
        put(&stack, nameplate::draw(document, nameplate)?)?;
    }
    if let Some(section) = panels
        .section
        .as_ref()
        .filter(|_| !panels.section_under_journal())
    {
        put(&stack, section_label::draw(document, section)?)?;
    }
    if let Some(party) = &panels.party {
        put(&stack, party::draw(document, party)?)?;
    }
    if let Some(progress) = &panels.progress {
        put(&stack, progress_line::draw(document, progress)?)?;
    }
    Ok(stack)
}

/// Every panel of `panels` on one layer, over `plate` when given: the free panels (title cards,
/// journal, bubbles), the two stacks and the place labels over them.
pub fn panels(
    document: &Document,
    panels: &KitPanels,
    plate: Option<&str>,
) -> Result<Element, JsValue> {
    ensure_style(document)?;
    let layer = element(document, "div", "kit")?;
    if let Some(picture) = crate::view::kit::photo(document, "kit-plate", plate, "")? {
        append(&layer, &picture)?;
    }
    let stack = bottom(document, panels)?;
    if stack.child_element_count() > 0 {
        append(&layer, &element(document, "div", "kit-vignette")?)?;
    }
    for card in &panels.title_cards {
        put(&layer, title_card::draw(document, card)?)?;
    }
    // The journal and the section label under it share one column, bounded by the stage, so the
    // book never grows over the label.
    if let Some(book) = panels
        .journal
        .as_ref()
        .map(|journal| journal::draw(document, journal))
        .transpose()?
        .flatten()
    {
        let column = element(document, "div", "kit-journal-column")?;
        append(&column, &book)?;
        if let Some(section) = panels
            .section
            .as_ref()
            .filter(|_| panels.section_under_journal())
        {
            put(&column, section_label::draw(document, section)?)?;
        }
        append(&layer, &column)?;
    }
    for bubble in &panels.bubbles {
        append(&layer, &speech::draw(document, bubble)?)?;
    }
    put_filled(&layer, &top(document, panels)?)?;
    put_filled(&layer, &stack)?;
    // A place label stands over the stacks, so the map's current destination is never hidden
    // under a list; keeping it clear of them is the caller's, like a bubble's.
    for place in &panels.places {
        put(&layer, place_label::draw(document, place)?)?;
    }
    Ok(layer)
}

/// The gallery board the page's address names (`?kit=N`), drawn as the whole stage, or `None`
/// when the address names no board. The gallery is a development tool (`kit::gallery`).
#[cfg(debug_assertions)]
pub fn gallery(document: &Document) -> Result<Option<Element>, JsValue> {
    use crate::dom::text_element;
    use crate::kit::gallery::{KitGallery, board_number};

    let window = web_sys::window().ok_or_else(|| JsValue::from_str("the page has no window"))?;
    let Some(number) = board_number(&window.location().search()?) else {
        return Ok(None);
    };
    let gallery = KitGallery::read().map_err(|error| JsValue::from_str(&error.to_string()))?;
    let board = gallery.board(number).ok_or_else(|| {
        JsValue::from_str(&format!(
            "the kit gallery has boards 1 to {}, not {number}",
            gallery.boards.len()
        ))
    })?;
    let layer = panels(document, &board.panels, board.plate.as_deref())?;
    layer.set_attribute("class", "kit kit-gallery")?;
    let mut tag = format!(
        "Kit board {number} of {}: {}",
        gallery.boards.len(),
        board.name
    );
    if let Some(after) = &board.after {
        tag.push_str(&format!(" (after {after})"));
    }
    append(
        &layer,
        &text_element(document, "div", "kit-gallery-tag", &tag)?,
    )?;
    Ok(Some(layer))
}

/// A release build has no gallery, so it draws no board.
#[cfg(not(debug_assertions))]
pub fn gallery(_document: &Document) -> Result<Option<Element>, JsValue> {
    Ok(None)
}

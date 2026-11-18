//! The join screens as elements (`seat::screen::JoinScreen`), in the phone kit's regions and look:
//! the header, the status, a scrolling body (the room, the heading, the cards, the name field, the
//! notice) and a dock of buttons low under the thumb. It decides nothing; every tap is the
//! screen's `SeatTap`, carried as `data-action`.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, set_action, text_element};
use crate::phone_kit::draw::{act as kit_act, regions, shell as kit_shell};
use crate::phone_kit::shell::{ButtonSpec, Pill, PillTone};
use crate::seat::play::SeatAct;
use crate::seat::screen::{
    JoinButton, JoinCard, JoinScreen, RoomBanner, SeatTap, Showcase, TableCard,
};
use crate::view::icons::{Icon, icon};
use crate::view::kit::photo;

/// The id of the guest's name field, read when Join the table is tapped.
pub const NAME_FIELD_ID: &str = "ph-join-name";

fn room(document: &Document, banner: &RoomBanner) -> Result<Element, JsValue> {
    let card = element(document, "section", "ph-glass ph-join-room")?;
    if let Some(image) = photo(
        document,
        "ph-join-room-picture",
        banner.picture.as_deref(),
        &banner.title,
    )? {
        append(&card, &image)?;
    }
    append(
        &card,
        &text_element(document, "h1", "ph-join-room-title", &banner.title)?,
    )?;
    let joined = if banner.players.is_empty() {
        "Nobody has joined yet.".to_owned()
    } else {
        format!("At the table: {}", banner.players.join(", "))
    };
    append(
        &card,
        &text_element(document, "p", "ph-join-room-players", &joined)?,
    )?;
    Ok(card)
}

/// The kit's action card, carrying its tap, with its face in the mark and its pill after the
/// words.
fn card(document: &Document, join: &JoinCard) -> Result<Element, JsValue> {
    let item = kit_act::card(document, &join.card)?;
    set_action(&item, &join.tap.action())?;
    if let (Some(face), Some(mark)) = (
        photo(
            document,
            "ph-join-face",
            join.picture.as_deref(),
            &join.card.label,
        )?,
        item.query_selector(".ph-card-mark")?,
    ) {
        mark.set_class_name("ph-card-mark ph-join-face-mark");
        append(&mark, &face)?;
    }
    if let Some(mark) = &join.mark {
        append(&item, &kit_shell::pill(document, mark)?)?;
    }
    Ok(item)
}

/// The Act page of a seat that plays a character: what the table asks now, why the last answer
/// was not taken, and the suggested lines as cards that send them. Nothing while it asks nothing:
/// the composer says why. The seat's last roll is the full card while the shared screen still
/// shows it (`tv_roll`, the id of the roll it shows) and nothing is asked; otherwise it is one
/// line under the question, which opens the full card on Roll.
pub fn act_block(
    document: &Document,
    act: &SeatAct,
    tv_roll: Option<&str>,
) -> Result<Option<Element>, JsValue> {
    if act.prompt.is_none()
        && act.notice.is_none()
        && act.pending_roll.is_none()
        && act.last_roll.is_none()
    {
        return Ok(None);
    }
    let block = element(document, "section", "ph-seat-ask")?;
    // The roll the dungeon master calls for comes first, as the shared screen calls it.
    if let Some(call) = &act.pending_roll {
        append(&block, &roll_call(document, call)?)?;
    }
    // The seat's last roll: the shared screen's own card, by the same renderer, while it is the
    // moment's; a line after the question once the table has moved on.
    let roll_line = match &act.last_roll {
        Some(roll) if crate::pages::sheet::roll_card_up(act.prompt.is_some(), roll, tv_roll) => {
            append(&block, &super::roll::phone_card(document, roll)?)?;
            None
        }
        Some(roll) => Some(roll_summary_line(document, roll)?),
        None => None,
    };
    // A word from the table comes first, above what it asks (or alone while it asks nothing).
    if let Some(notice) = &act.notice {
        let pill = Pill {
            text: notice.clone(),
            tone: PillTone::Neutral,
            glyph: None,
        };
        append(&block, &kit_shell::status(document, &pill)?)?;
    }
    let Some(prompt) = &act.prompt else {
        if let Some(line) = roll_line {
            append(&block, &line)?;
        }
        return Ok(Some(block));
    };
    append(
        &block,
        &text_element(document, "h2", "ph-join-heading", prompt)?,
    )?;
    if let Some(note) = &act.note {
        let pill = Pill {
            text: note.clone(),
            tone: PillTone::Warn,
            glyph: None,
        };
        append(&block, &kit_shell::status(document, &pill)?)?;
    }
    let cards = element(document, "div", "ph-cards ph-join-cards")?;
    for item in &act.cards {
        append(&cards, &card(document, item)?)?;
    }
    append(&block, &cards)?;
    if let Some(line) = roll_line {
        append(&block, &line)?;
    }
    Ok(Some(block))
}

/// The seat's last roll in one line ("History 17 vs DC 10: success"), a tap that opens the full
/// card on Roll.
fn roll_summary_line(
    document: &Document,
    roll: &crate::fixture::RollView,
) -> Result<Element, JsValue> {
    let line = text_element(
        document,
        "button",
        "ph-roll-summary",
        &crate::pages::sheet::roll_summary(roll),
    )?;
    line.set_attribute("type", "button")?;
    set_action(
        &line,
        &crate::phone_kit::nav::page_action(crate::pages::sheet::ROLL_PAGE),
    )?;
    Ok(line)
}

/// The call of a roll as the shared screen shows it (the kind, the title, advantage or
/// disadvantage, the difficulty) with a big Roll button that answers it.
fn roll_call(
    document: &Document,
    call: &crate::seat::wire::PendingRoll,
) -> Result<Element, JsValue> {
    let card = element(document, "div", "ph-roll-call")?;
    append(
        &card,
        &text_element(
            document,
            "span",
            "ph-roll-call-kicker",
            &call.kind.replace('_', " "),
        )?,
    )?;
    append(
        &card,
        &text_element(document, "h2", "ph-roll-call-title", &call.title)?,
    )?;
    if let Some(mode) = crate::kit::roll_card::mode_word(call.mode) {
        append(
            &card,
            &text_element(document, "span", "ph-roll-call-mode", mode)?,
        )?;
    }
    if let Some(target) = &call.target {
        let text = format!("{} {}", target.label.trim(), target.value);
        append(
            &card,
            &text_element(document, "span", "ph-roll-call-target", &text)?,
        )?;
    }
    let button = element(document, "button", "ph-roll-call-button")?;
    button.set_text_content(Some("Roll"));
    set_action(&button, &SeatTap::Roll.action())?;
    append(&card, &button)?;
    Ok(card)
}

fn name_field(document: &Document) -> Result<Element, JsValue> {
    let label = element(document, "label", "ph-join-field")?;
    append(
        &label,
        &text_element(document, "span", "ph-join-field-label", "Name")?,
    )?;
    let input = element(document, "input", "ph-join-name")?;
    input.set_attribute("id", NAME_FIELD_ID)?;
    input.set_attribute("type", "text")?;
    input.set_attribute("autocomplete", "nickname")?;
    input.set_attribute("enterkeyhint", "go")?;
    input.set_attribute("placeholder", "Your name")?;
    append(&label, &input)?;
    Ok(label)
}

fn button(document: &Document, spec: &JoinButton) -> Result<Element, JsValue> {
    kit_shell::button(
        document,
        &ButtonSpec {
            label: spec.label.clone(),
            kind: spec.kind,
            enabled: true,
            reason: None,
            action: Some(spec.tap.action()),
        },
    )
}

/// The player's own hero, large: the picture fitted whole (the portrait with the loader while the
/// ready picture is made), name, species and class, and the ready state, lit when ready.
fn showcase(document: &Document, hero: &Showcase) -> Result<Element, JsValue> {
    let class = if hero.ready {
        "ph-glass ph-showcase ph-lit ph-showcase-ready"
    } else {
        "ph-glass ph-showcase"
    };
    let card = element(document, "figure", class)?;
    let stage = element(document, "div", "ph-showcase-stage")?;
    if let Some(action) = &hero.action {
        set_action(&stage, action)?;
    }
    match photo(
        document,
        "ph-showcase-image",
        hero.picture.as_deref(),
        &hero.name,
    )? {
        Some(image) => append(&stage, &image)?,
        None => append(
            &stage,
            &icon(document, Icon::Silhouette, "ph-stage-silhouette")?,
        )?,
    }
    if hero.making {
        append(&stage, &kit_shell::loader(document)?)?;
    }
    append(&card, &stage)?;
    let caption = element(document, "figcaption", "ph-showcase-caption")?;
    append(
        &caption,
        &text_element(document, "h2", "ph-showcase-name", &hero.name)?,
    )?;
    append(
        &caption,
        &text_element(document, "p", "ph-showcase-build", &hero.build)?,
    )?;
    let pill = if hero.ready {
        Pill {
            text: "Ready".to_owned(),
            tone: PillTone::Good,
            glyph: None,
        }
    } else {
        Pill {
            text: "Not ready".to_owned(),
            tone: PillTone::Warn,
            glyph: None,
        }
    };
    append(&caption, &kit_shell::pill(document, &pill)?)?;
    append(&card, &caption)?;
    Ok(card)
}

/// The other players at the table, small: picture, name and ready state; away players dimmed.
fn table(document: &Document, cards: &[TableCard]) -> Result<Element, JsValue> {
    let row = element(document, "div", "ph-table-cards")?;
    for player in cards {
        let mut class = String::from("ph-glass ph-table-card");
        if player.ready {
            class.push_str(" ph-lit");
        }
        if player.away {
            class.push_str(" ph-table-away");
        }
        let card = element(document, "div", &class)?;
        match photo(
            document,
            "ph-table-image",
            player.picture.as_deref(),
            &player.name,
        )? {
            Some(image) => append(&card, &image)?,
            None => append(
                &card,
                &icon(document, Icon::Silhouette, "ph-table-silhouette")?,
            )?,
        }
        append(
            &card,
            &text_element(document, "span", "ph-table-name", &player.name)?,
        )?;
        let state = match (player.away, player.ready) {
            (true, _) => "Away",
            (false, true) => "Ready",
            (false, false) => "Not ready",
        };
        append(
            &card,
            &text_element(document, "span", "ph-table-state", state)?,
        )?;
        append(&row, &card)?;
    }
    Ok(row)
}

fn body(document: &Document, screen: &JoinScreen) -> Result<Element, JsValue> {
    let body = element(document, "main", "ph-body ph-join-body")?;
    if let Some(banner) = &screen.room {
        append(&body, &room(document, banner)?)?;
    }
    if let Some(hero) = &screen.showcase {
        append(&body, &showcase(document, hero)?)?;
    }
    if let Some(heading) = &screen.heading {
        append(
            &body,
            &text_element(document, "h2", "ph-join-heading", heading)?,
        )?;
    }
    if let Some(lead) = &screen.lead {
        append(&body, &text_element(document, "p", "ph-join-lead", lead)?)?;
    }
    if let Some(notice) = &screen.notice {
        append(&body, &kit_shell::status(document, notice)?)?;
    }
    if screen.name_field {
        append(&body, &name_field(document)?)?;
    }
    if !screen.cards.is_empty() {
        let cards = element(document, "div", "ph-cards ph-join-cards")?;
        for item in &screen.cards {
            append(&cards, &card(document, item)?)?;
        }
        append(&body, &cards)?;
    }
    if !screen.table.is_empty() {
        append(&body, &table(document, &screen.table)?)?;
    }
    if !screen.roster.is_empty() {
        let row = element(document, "div", "ph-pill-row")?;
        for player in &screen.roster {
            append(&row, &kit_shell::pill(document, player)?)?;
        }
        append(&body, &row)?;
    }
    if let Some(empty) = &screen.empty {
        append(&body, &kit_shell::empty(document, empty)?)?;
    }
    if let Some(loading) = &screen.loading {
        append(&body, &kit_shell::loading(document, loading)?)?;
    }
    Ok(body)
}

/// One join screen, in the kit's regions (`phone_kit::draw::regions`).
pub fn build(document: &Document, screen: &JoinScreen) -> Result<Element, JsValue> {
    let dock = element(document, "div", "ph-dock ph-join-dock")?;
    for spec in &screen.buttons {
        append(&dock, &button(document, spec)?)?;
    }
    let drawn = regions(
        document,
        "ph-live ph-join",
        false,
        screen.header.as_ref(),
        screen.status.as_ref(),
        &body(document, screen)?,
        &dock,
    )?;
    if let Some(open) = &screen.viewer {
        append(&drawn, &crate::view::look::viewer(document, open)?)?;
    }
    Ok(drawn)
}

/// The join board of the phone gallery (`?phone_kit=join`): every join screen side by side, each
/// in a phone frame with its name, or `None` when the address names another board. A development
/// tool only.
#[cfg(debug_assertions)]
pub fn gallery_page(document: &Document) -> Result<Option<Element>, JsValue> {
    use crate::phone_kit::draw::ensure_style;
    use crate::phone_kit::draw::gallery::figure;
    use crate::seat::samples::{join_samples, names_join_board};
    use crate::seat::screen::screen_of;

    let window = web_sys::window().ok_or_else(|| JsValue::from_str("the page has no window"))?;
    if !names_join_board(&window.location().search()?) {
        return Ok(None);
    }
    ensure_style(document)?;
    let page = element(document, "div", "ph-gallery")?;
    append(
        &page,
        &text_element(
            document,
            "h1",
            "ph-gallery-tag",
            "Phone kit board: joining, from the room to a seat",
        )?,
    )?;
    let figures = element(document, "div", "ph-figures")?;
    for (title, machine) in join_samples() {
        let drawn = screen_of(&machine)
            .map(|screen| build(document, &screen))
            .transpose()?;
        append(&figures, &figure(document, title, drawn.as_ref())?)?;
    }
    append(&page, &figures)?;
    Ok(Some(page))
}

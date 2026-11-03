//! The lobby and the loading screen: the title block, six seat cards in three states, the invite
//! panel, the begin button, the key to the states, and the loading screen's bar and ready chips.
//!
//! The lobby is a pure function of the scene's data (`lobby_model`): this module turns the answers
//! of the model into elements and decides nothing. The shared screen takes no input, so the begin
//! button, the copy marks and the seats are pictures of what the phones do, never controls.
//! Text from a scene is set as text, a picture name becomes an address only when it names an
//! image of the media library, and a number reaches a style only as a number.

use wasm_bindgen::JsValue;
use web_sys::{Document, Element};

use crate::dom::{append, element, set_style, text_element};
use crate::fixture::TvScene;
use crate::lobby_model::{
    BEGIN_LABEL, CHARACTER_STEPS, GAME_CODE_LABEL, INVITE_HEADING, INVITE_HINT, LEGEND,
    LOADING_LABEL, LOBBY_EYEBROW, LoadingScreen, LobbyScreen, LobbySeat, READY_WORD, SeatState,
    SummaryIcon, bar_percent, count_text, seat_view, spaced_code, summary_word,
};
use crate::media::{MediaKind, media_kind};
use crate::overlay::Glyph;
use crate::view_icons::{Icon, icon, qr_placeholder};
use crate::view_kit::{glass, glyph};

/// The lobby or the loading screen of `scene`, or `None` when it has neither.
pub fn build(document: &Document, scene: &TvScene) -> Result<Option<Element>, JsValue> {
    if let Some(lobby) = &scene.lobby {
        return lobby_screen(document, lobby).map(Some);
    }
    if let Some(loading) = &scene.loading {
        return loading_screen(document, loading).map(Some);
    }
    Ok(None)
}

/// An element of `tag` and `class` holding `text` as text.
fn text(document: &Document, tag: &str, class: &str, words: &str) -> Result<Element, JsValue> {
    text_element(document, tag, class, words)
}

/// A picture of the media library, or `None` when `media` does not name an image.
pub(crate) fn photo(
    document: &Document,
    class: &str,
    media: Option<&str>,
    alt: &str,
) -> Result<Option<Element>, JsValue> {
    let Some(name) = media.filter(|name| media_kind(name) == Some(MediaKind::Image)) else {
        return Ok(None);
    };
    let image = element(document, "img", class)?;
    image.set_attribute("src", &format!("/playground/media/{name}"))?;
    image.set_attribute("data-media", name)?;
    image.set_attribute("alt", alt)?;
    Ok(Some(image))
}

/// A line with a diamond at each end and a rule beyond it, as over the lobby's title.
pub(crate) fn eyebrow(document: &Document, words: &str, class: &str) -> Result<Element, JsValue> {
    let row = element(document, "div", &format!("lb-eyebrow {class}"))?;
    for part in ["lb-rule", "lb-diamond"] {
        append(&row, &element(document, "span", part)?)?;
    }
    append(&row, &text(document, "span", "lb-eyebrow-text", words)?)?;
    for part in ["lb-diamond", "lb-rule"] {
        append(&row, &element(document, "span", part)?)?;
    }
    Ok(row)
}

/// The thin rule with a diamond in the middle, under the subtitle.
fn flourish(document: &Document) -> Result<Element, JsValue> {
    let row = element(document, "div", "lb-flourish")?;
    for part in ["lb-rule", "lb-diamond lb-diamond-open", "lb-rule"] {
        append(&row, &element(document, "span", part)?)?;
    }
    Ok(row)
}

fn lobby_screen(document: &Document, lobby: &LobbyScreen) -> Result<Element, JsValue> {
    let root = element(document, "div", "lb lb-lobby")?;
    let head = element(document, "header", "lb-head")?;
    let eyebrow_words = lobby.eyebrow.as_deref().unwrap_or(LOBBY_EYEBROW);
    append(&head, &eyebrow(document, eyebrow_words, "")?)?;
    append(&head, &text(document, "h2", "lb-title", &lobby.title)?)?;
    if let Some(subtitle) = &lobby.subtitle {
        append(&head, &text(document, "p", "lb-subtitle", subtitle)?)?;
    }
    append(&head, &flourish(document)?)?;
    append(&root, &head)?;
    let seats = element(document, "div", "lb-seats")?;
    seats.set_attribute("role", "list")?;
    seats.set_attribute("aria-label", "Seats at the table")?;
    set_style(&seats, &format!("--lb-seats:{};", lobby.seats.len().max(1)))?;
    for (index, seat) in lobby.seats.iter().enumerate() {
        append(&seats, &seat_card(document, seat, index)?)?;
    }
    append(&root, &seats)?;
    if let Some(invite) = &lobby.invite {
        append(&root, &invite_panel(document, invite)?)?;
    }
    if let Some(begin) = &lobby.begin {
        append(&root, &begin_button(document, begin)?)?;
    }
    if lobby.legend {
        append(&root, &legend(document)?)?;
    }
    Ok(root)
}

fn seat_card(document: &Document, seat: &LobbySeat, index: usize) -> Result<Element, JsValue> {
    let view = seat_view(seat, index);
    let (wrapper, panel) = glass(
        document,
        &format!("lb-seat lb-seat-{}", view.key),
        "lb-seat-panel",
    )?;
    wrapper.set_attribute("role", "listitem")?;
    match seat.state {
        SeatState::LockedIn | SeatState::Creating => {
            let art = element(document, "div", "lb-art")?;
            match photo(document, "lb-photo", seat.portrait.as_deref(), &view.name)? {
                Some(image) => append(&art, &image)?,
                None => append(&art, &icon(document, Icon::Silhouette, "lb-silhouette")?)?,
            }
            append(&panel, &art)?;
            if seat.controller {
                append(&panel, &icon(document, Icon::Controller, "lb-controller")?)?;
            }
            append(&panel, &element(document, "div", "lb-fade")?)?;
            let body = element(document, "div", "lb-seat-text")?;
            if seat.host {
                append(&body, &glyph(document, Glyph::Crown, "lb-crown")?)?;
            }
            if seat.state == SeatState::Creating
                && seat.step.is_none()
                && (seat.portrait.is_some() || seat.progress_percent.is_none())
            {
                append(&body, &element(document, "span", "lb-ring")?)?;
            }
            append(&body, &text(document, "div", "lb-seat-name", &view.name)?)?;
            if let Some(sub_line) = &view.sub_line {
                append(&body, &seat_sub_line(document, seat, sub_line)?)?;
            }
            if seat.state == SeatState::Creating
                && let Some(class) = seat.class.as_deref()
            {
                append(&body, &text(document, "div", "lb-seat-class-plain", class)?)?;
            }
            if let Some(percent) = view.progress_percent {
                append(&body, &progress_bar(document, percent)?)?;
            }
            if let Some(step) = view.step {
                append(&body, &steps(document, step)?)?;
            }
            if let Some(badge) = &view.badge {
                append(&body, &badge_pill(document, seat, badge)?)?;
            }
            append(&panel, &body)?;
        }
        SeatState::Open | SeatState::Closed => {
            let ring = element(document, "div", "lb-open-ring")?;
            let mark = if seat.state == SeatState::Open {
                Glyph::Plus
            } else {
                Glyph::Lock
            };
            append(&ring, &glyph(document, mark, "lb-open-mark")?)?;
            append(&panel, &ring)?;
            let body = element(document, "div", "lb-seat-text lb-seat-text-open")?;
            append(&body, &text(document, "div", "lb-seat-name", &view.name)?)?;
            if let Some(sub_line) = &view.sub_line {
                append(&body, &text(document, "div", "lb-open-word", sub_line)?)?;
            }
            append(&panel, &body)?;
        }
    }
    Ok(wrapper)
}

/// The line under a seat's name: the class with its glyph for a hero, the stage for a creator.
fn seat_sub_line(document: &Document, seat: &LobbySeat, words: &str) -> Result<Element, JsValue> {
    let line = element(document, "div", "lb-seat-class")?;
    match seat.state {
        SeatState::LockedIn => {
            if let Some(mark) = seat.glyph {
                append(&line, &glyph(document, mark, "lb-class-glyph")?)?;
            }
        }
        _ => {
            if seat.progress_percent.is_some() || seat.step.is_some() {
                line.set_attribute("class", "lb-seat-class lb-seat-stage")?;
            } else {
                line.set_attribute("class", "lb-seat-class lb-seat-stage lb-seat-stage-plain")?;
            }
        }
    }
    append(&line, &text(document, "span", "", words)?)?;
    Ok(line)
}

fn badge_pill(document: &Document, seat: &LobbySeat, words: &str) -> Result<Element, JsValue> {
    let class = if seat.state == SeatState::LockedIn {
        "lb-badge lb-badge-locked"
    } else {
        "lb-badge lb-badge-creating"
    };
    let badge = element(document, "div", class)?;
    let mark = if seat.state == SeatState::LockedIn {
        Glyph::Check
    } else {
        Glyph::Book
    };
    append(&badge, &glyph(document, mark, "lb-badge-glyph")?)?;
    append(&badge, &text(document, "span", "", words)?)?;
    Ok(badge)
}

fn progress_bar(document: &Document, percent: u8) -> Result<Element, JsValue> {
    let row = element(document, "div", "lb-progress")?;
    let bar = element(document, "div", "lb-bar")?;
    bar.set_attribute("role", "progressbar")?;
    bar.set_attribute("aria-valuemin", "0")?;
    bar.set_attribute("aria-valuemax", "100")?;
    bar.set_attribute("aria-valuenow", &percent.to_string())?;
    bar.set_attribute("aria-label", "Character creation progress")?;
    let fill = element(document, "div", "lb-bar-fill")?;
    set_style(&fill, &format!("width:{percent}%;"))?;
    append(&bar, &fill)?;
    append(&row, &bar)?;
    append(
        &row,
        &text(document, "span", "lb-bar-percent", &format!("{percent}%"))?,
    )?;
    Ok(row)
}

/// The dots of the three steps of making a character, joined by a line; the current one is lit.
fn steps(document: &Document, current: usize) -> Result<Element, JsValue> {
    let row = element(document, "ol", "lb-steps")?;
    for (index, name) in CHARACTER_STEPS.iter().enumerate() {
        let class = match index.cmp(&current) {
            std::cmp::Ordering::Less => "lb-step lb-step-done",
            std::cmp::Ordering::Equal => "lb-step lb-step-now",
            std::cmp::Ordering::Greater => "lb-step",
        };
        let item = element(document, "li", class)?;
        append(&item, &element(document, "span", "lb-step-dot")?)?;
        append(&item, &text(document, "span", "lb-step-name", name)?)?;
        append(&row, &item)?;
    }
    Ok(row)
}

fn invite_panel(
    document: &Document,
    invite: &crate::lobby_model::Invite,
) -> Result<Element, JsValue> {
    let (wrapper, panel) = glass(document, "lb-invite-wrap", "lb-invite")?;
    let intro = element(document, "div", "lb-invite-intro")?;
    append(&intro, &icon(document, Icon::People, "lb-invite-icon")?)?;
    let words = element(document, "div", "lb-invite-words")?;
    let heading = invite.heading.as_deref().unwrap_or(INVITE_HEADING);
    append(&words, &text(document, "h3", "lb-invite-heading", heading)?)?;
    let hint = invite.hint.as_deref().unwrap_or(INVITE_HINT);
    append(&words, &text(document, "p", "lb-invite-hint", hint)?)?;
    append(&intro, &words)?;
    append(&panel, &intro)?;
    append(&panel, &element(document, "span", "lb-vrule")?)?;
    let code_source = invite.game_code.as_deref().or(invite.join_link.as_deref());
    if let Some(source) = code_source {
        let block = element(document, "div", "lb-qr")?;
        append(&block, &qr_placeholder(document, source, "lb-qr-cells")?)?;
        append(&panel, &block)?;
        append(&panel, &text(document, "span", "lb-or", "or")?)?;
    }
    let links = element(document, "div", "lb-links")?;
    if let Some(link) = &invite.join_link {
        let field = element(document, "div", "lb-field")?;
        append(&field, &icon(document, Icon::Link, "lb-field-icon")?)?;
        append(&field, &text(document, "span", "lb-field-text", link)?)?;
        append(
            &field,
            &icon(document, Icon::Copy, "lb-field-icon lb-field-copy")?,
        )?;
        append(&links, &field)?;
    }
    if let Some(code) = &invite.game_code {
        let field = element(document, "div", "lb-field")?;
        append(&field, &icon(document, Icon::Controller, "lb-field-icon")?)?;
        append(
            &field,
            &text(document, "span", "lb-field-label", GAME_CODE_LABEL)?,
        )?;
        append(
            &field,
            &text(document, "span", "lb-code", &spaced_code(code))?,
        )?;
        append(
            &field,
            &icon(document, Icon::Copy, "lb-field-icon lb-field-copy")?,
        )?;
        append(&links, &field)?;
    }
    append(&panel, &links)?;
    append(&panel, &element(document, "span", "lb-vrule")?)?;
    append(&panel, &summary(document, invite)?)?;
    Ok(wrapper)
}

fn summary(document: &Document, invite: &crate::lobby_model::Invite) -> Result<Element, JsValue> {
    let block = element(document, "div", "lb-summary")?;
    let which = match invite.icon {
        SummaryIcon::Check => (Icon::CheckRing, "lb-summary-icon lb-summary-check"),
        SummaryIcon::People => (Icon::People, "lb-summary-icon lb-summary-people"),
        SummaryIcon::Spinner => (Icon::Spinner, "lb-summary-icon lb-summary-spinner"),
    };
    append(&block, &icon(document, which.0, which.1)?)?;
    let words = element(document, "div", "lb-summary-words")?;
    let count = format!(
        "{} {}",
        count_text(invite.joined, invite.capacity),
        summary_word(invite)
    );
    append(&words, &text(document, "div", "lb-summary-count", &count)?)?;
    if let Some(detail) = &invite.detail {
        append(&words, &text(document, "div", "lb-summary-detail", detail)?)?;
    }
    append(&block, &words)?;
    Ok(block)
}

fn begin_button(
    document: &Document,
    begin: &crate::lobby_model::BeginButton,
) -> Result<Element, JsValue> {
    let class = if begin.enabled {
        "lb-begin-wrap"
    } else {
        "lb-begin-wrap lb-begin-off"
    };
    let (wrapper, panel) = glass(document, class, "lb-begin")?;
    let row = element(document, "div", "lb-begin-row")?;
    append(&row, &icon(document, Icon::Play, "lb-begin-play")?)?;
    let label = begin.label.as_deref().unwrap_or(BEGIN_LABEL);
    append(&row, &text(document, "span", "lb-begin-label", label)?)?;
    append(&panel, &row)?;
    if !begin.enabled {
        panel.set_attribute("aria-disabled", "true")?;
        if let Some(note) = &begin.note {
            append(&panel, &text(document, "div", "lb-begin-note", note)?)?;
        }
    }
    Ok(wrapper)
}

fn legend(document: &Document) -> Result<Element, JsValue> {
    let (wrapper, panel) = glass(document, "lb-legend-wrap", "lb-legend")?;
    panel.set_attribute("role", "group")?;
    panel.set_attribute("aria-label", "What the seat colours mean")?;
    for entry in LEGEND {
        let item = element(
            document,
            "div",
            &format!("lb-legend-item lb-legend-{}", entry.key),
        )?;
        let mark = match entry.key {
            "locked" => icon(document, Icon::CheckRing, "lb-legend-icon")?,
            "creating" => icon(document, Icon::Spinner, "lb-legend-icon")?,
            _ => icon(document, Icon::PlusRing, "lb-legend-icon")?,
        };
        append(&item, &mark)?;
        let words = element(document, "div", "lb-legend-words")?;
        append(
            &words,
            &text(document, "div", "lb-legend-label", entry.label)?,
        )?;
        append(
            &words,
            &text(document, "div", "lb-legend-hint", entry.hint)?,
        )?;
        append(&item, &words)?;
        append(&panel, &item)?;
    }
    Ok(wrapper)
}

fn loading_screen(document: &Document, loading: &LoadingScreen) -> Result<Element, JsValue> {
    let root = element(document, "div", "lb lb-loading")?;
    let head = element(document, "header", "lb-load-head")?;
    if let Some(kicker) = &loading.kicker {
        append(&head, &eyebrow(document, kicker, "lb-eyebrow-load")?)?;
    }
    append(
        &head,
        &text(document, "h2", "lb-title lb-title-load", &loading.title)?,
    )?;
    append(&head, &flourish(document)?)?;
    if let Some(subtitle) = &loading.subtitle {
        append(
            &head,
            &text(document, "p", "lb-subtitle lb-subtitle-load", subtitle)?,
        )?;
    }
    append(&root, &head)?;
    if let Some(narrator) = &loading.narrator {
        let (wrapper, panel) = glass(document, "lb-narrator-wrap", "lb-narrator")?;
        let face = element(document, "div", "lb-narrator-face")?;
        if let Some(image) = photo(
            document,
            "lb-photo",
            narrator.portrait.as_deref(),
            &narrator.name,
        )? {
            append(&face, &image)?;
        }
        append(&panel, &face)?;
        let words = element(document, "div", "lb-narrator-words")?;
        append(
            &words,
            &text(document, "div", "lb-narrator-name", &narrator.name)?,
        )?;
        if let Some(status) = &narrator.status {
            append(
                &words,
                &text(document, "div", "lb-narrator-status", status)?,
            )?;
        }
        append(&panel, &words)?;
        append(&panel, &element(document, "span", "lb-vrule")?)?;
        append(&panel, &icon(document, Icon::Waveform, "lb-wave")?)?;
        append(&root, &wrapper)?;
    }
    let label = loading.label.as_deref().unwrap_or(LOADING_LABEL);
    append(&root, &eyebrow(document, label, "lb-eyebrow-bar")?)?;
    append(&root, &loading_bar(document, bar_percent(loading.percent))?)?;
    let chips = element(document, "div", "lb-chips")?;
    chips.set_attribute("role", "list")?;
    set_style(
        &chips,
        &format!("--lb-chips:{};", loading.seats.len().max(1)),
    )?;
    for chip in &loading.seats {
        let (wrapper, panel) = glass(
            document,
            if chip.ready {
                "lb-chip lb-chip-ready"
            } else {
                "lb-chip"
            },
            "lb-chip-panel",
        )?;
        wrapper.set_attribute("role", "listitem")?;
        let face = element(document, "div", "lb-chip-face")?;
        match photo(document, "lb-photo", chip.portrait.as_deref(), &chip.name)? {
            Some(image) => append(&face, &image)?,
            None => append(&face, &icon(document, Icon::Silhouette, "lb-silhouette")?)?,
        }
        append(&panel, &face)?;
        let words = element(document, "div", "lb-chip-words")?;
        append(&words, &text(document, "div", "lb-chip-name", &chip.name)?)?;
        if chip.ready {
            let ready = element(document, "div", "lb-chip-ready-word")?;
            append(&ready, &glyph(document, Glyph::Check, "lb-badge-glyph")?)?;
            append(&ready, &text(document, "span", "", READY_WORD)?)?;
            append(&words, &ready)?;
        }
        append(&panel, &words)?;
        append(&chips, &wrapper)?;
    }
    append(&root, &chips)?;
    Ok(root)
}

fn loading_bar(document: &Document, percent: u8) -> Result<Element, JsValue> {
    let (wrapper, panel) = glass(document, "lb-loadbar-wrap", "lb-loadbar")?;
    panel.set_attribute("role", "progressbar")?;
    panel.set_attribute("aria-valuemin", "0")?;
    panel.set_attribute("aria-valuemax", "100")?;
    panel.set_attribute("aria-valuenow", &percent.to_string())?;
    panel.set_attribute("aria-label", "Loading progress")?;
    let track = element(document, "div", "lb-loadbar-track")?;
    let fill = element(document, "div", "lb-loadbar-fill")?;
    set_style(&fill, &format!("width:{percent}%;"))?;
    append(&track, &fill)?;
    append(&panel, &track)?;
    append(
        &panel,
        &text(
            document,
            "span",
            "lb-loadbar-percent",
            &format!("{percent}%"),
        )?,
    )?;
    Ok(wrapper)
}

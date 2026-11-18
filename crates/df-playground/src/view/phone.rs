//! One player's phone: the hero and goal, what is happening, a private note, the choice list and
//! the composer (`composer`).
//!
//! It turns a `PhoneScene` and the chosen controls into page elements. The choice rules (how many,
//! which groups, what is hidden) come from `choices`; this module only draws what it returns, and
//! it shows every warning, so a fixture that breaks the contract is visible and not repaired.
//!
//! A real phone (`PageRole::Phone`) is drawn with the phone kit (`phone_kit::draw`): the same
//! slots, the kit's look, and a tab bar under the Act panel. The playground's mock phone keeps its
//! own plain look, so the studio page does not change.

use wasm_bindgen::prelude::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element};

use crate::choices::{ChoiceListView, build};
use crate::controls::Controls;
use crate::copy::{PRIVATE_NOTE_TITLE, UNAVAILABLE_TITLE, status_blocks_input, status_copy};
use crate::dom::{append, element, picture, set_action, text_element};
use crate::draw_plan::Part;
use crate::fixture::{Choice, PhoneScene, PlateKind, Status, TvScene};
use crate::phone_kit::act::ActChoices;
use crate::phone_kit::draw::nav::{live_tabs, paint_live};
use crate::phone_kit::draw::{act as kit_act, set_scene, shell as kit_shell};
use crate::phone_kit::shell::{PhoneHeader, Pill, PillTone};
use crate::role::PageRole;
use crate::seat::play::SeatAct;
use crate::seat::screen::HeroHeader;
use crate::view::tv::slot;

fn choice_button(document: &Document, choice: &Choice) -> Result<Element, JsValue> {
    let button = element(document, "button", "choice")?;
    button.set_attribute("type", "button")?;
    append(
        &button,
        &text_element(document, "span", "choice-label", &choice.label)?,
    )?;
    if let Some(explain) = &choice.explain {
        append(
            &button,
            &text_element(document, "span", "choice-explain", explain)?,
        )?;
    }
    Ok(button)
}

fn choice_list(document: &Document, view: &ChoiceListView<'_>) -> Result<Element, JsValue> {
    let list = element(document, "div", "choices")?;
    if !view.suggested.is_empty() {
        let suggested = element(document, "div", "choice-suggested")?;
        for choice in view.suggested {
            append(&suggested, &choice_button(document, choice)?)?;
        }
        if view.left_out > 0 {
            append(
                &suggested,
                &text_element(
                    document,
                    "div",
                    "choice-more",
                    &format!("{} more not shown", view.left_out),
                )?,
            )?;
        }
        append(&list, &suggested)?;
    }
    for group in &view.groups {
        let section = element(document, "div", "choice-group")?;
        append(
            &section,
            &text_element(document, "div", "choice-group-name", &group.name)?,
        )?;
        let row = element(document, "div", "choice-row")?;
        for choice in &group.choices {
            append(&row, &choice_button(document, choice)?)?;
        }
        append(&section, &row)?;
        append(&list, &section)?;
    }
    if !view.unavailable.is_empty() {
        let section = element(document, "div", "choice-unavailable")?;
        append(
            &section,
            &text_element(document, "div", "choice-group-name", UNAVAILABLE_TITLE)?,
        )?;
        for choice in view.unavailable {
            let row = element(document, "div", "choice-off")?;
            append(
                &row,
                &text_element(document, "span", "choice-label", &choice.label)?,
            )?;
            append(
                &row,
                &text_element(document, "span", "choice-explain", &choice.reason)?,
            )?;
            append(&section, &row)?;
        }
        append(&list, &section)?;
    }
    Ok(list)
}

fn status_chip(document: &Document, status: &Status) -> Result<Option<Element>, JsValue> {
    let Some(copy) = status_copy(status) else {
        return Ok(None);
    };
    let class = if status_blocks_input(status) {
        "phone-status phone-status-blocking"
    } else {
        "phone-status"
    };
    text_element(document, "div", class, &copy).map(Some)
}

/// The parts of a phone, in the order they are drawn.
pub const PARTS: [Part; 7] = [
    Part::PhoneHead,
    Part::PhoneStatus,
    Part::PhoneImage,
    Part::PhoneNarration,
    Part::PhoneNote,
    Part::PhoneChoices,
    Part::PhoneComposer,
];

/// A phone with its slots empty.
pub struct PhoneShell {
    pub section: Element,
    pub slots: Vec<(Part, Element)>,
}

/// A real phone's shell: the header and status, one scrolling story (picture, narration, note and
/// choices together), and a dock low on the screen for the composer and the tab bar.
fn player_shell(document: &Document) -> Result<PhoneShell, JsValue> {
    let section = element(document, "section", "device device-phone")?;
    section.set_attribute("aria-label", "Your hero")?;
    let screen = element(document, "div", "ph-screen ph-live")?;
    let story = element(document, "main", "ph-body ph-story")?;
    let dock = element(document, "div", "ph-dock")?;
    let mut slots = Vec::new();
    for part in PARTS {
        let holder = slot(document, part)?;
        match part {
            Part::PhoneImage | Part::PhoneNarration | Part::PhoneNote | Part::PhoneChoices => {
                append(&story, &holder)?;
            }
            Part::PhoneComposer => append(&dock, &holder)?,
            _ => append(&screen, &holder)?,
        }
        slots.push((part, holder));
    }
    append(&dock, &kit_shell::tab_bar(document, &live_tabs())?)?;
    // A page of the bar other than Story and Act is a sample page, drawn here while it is open.
    let sample = element(document, "main", "ph-body ph-sample")?;
    sample.set_attribute("hidden", "")?;
    append(&screen, &story)?;
    append(&screen, &sample)?;
    append(&screen, &dock)?;
    paint_live(document, &screen)?;
    append(&section, &screen)?;
    Ok(PhoneShell { section, slots })
}

/// Builds one phone's shell: the screen with a slot for each piece. The story is one scrolling
/// column that holds the picture and the narration, so a redraw of one keeps the other.
pub fn shell(document: &Document, role: PageRole) -> Result<PhoneShell, JsValue> {
    if role == PageRole::Phone {
        return player_shell(document);
    }
    let section = element(document, "section", "device device-phone")?;
    section.set_attribute("aria-label", "A player's phone")?;
    let screen = element(document, "div", "phone-screen")?;
    let story = element(document, "div", "phone-story")?;
    let mut slots = Vec::new();
    for part in PARTS {
        let holder = slot(document, part)?;
        match part {
            Part::PhoneImage | Part::PhoneNarration => append(&story, &holder)?,
            _ => {
                // The story sits between the status and the note.
                if part == Part::PhoneNote {
                    append(&screen, &story)?;
                }
                append(&screen, &holder)?;
            }
        }
        slots.push((part, holder));
    }
    append(&section, &screen)?;
    Ok(PhoneShell { section, slots })
}

/// The attribute of a real phone's screen that names the scene behind it.
const SCENE_ATTRIBUTE: &str = "data-scene";

/// The picture a real phone shows behind its screen for the shared screen `tv`: its backdrop, but
/// never a close look the shared screen cuts to (that stays on the shared screen, and on Look
/// closer), so the place stays behind the phone.
pub fn scene_of(tv: &TvScene) -> Option<&str> {
    tv.backdrop
        .as_ref()
        .filter(|image| image.plate != PlateKind::CloseUp)
        .map(|image| image.media.as_str())
}

/// Puts the shared screen's scene (the backdrop picture) behind a real phone's screen, dimmed by
/// the kit's own fades. The studio's mock phone has no scene.
pub fn paint_scene(section: &Element, role: PageRole, media: Option<&str>) -> Result<(), JsValue> {
    if role != PageRole::Phone {
        return Ok(());
    }
    let (Some(screen), Some(media)) = (section.query_selector(".ph-screen")?, media) else {
        return Ok(());
    };
    set_scene(&screen, Some(media))?;
    screen.set_attribute(SCENE_ATTRIBUTE, media)
}

/// Moves a real phone's scene on to `media` when the shared screen has: the old picture is laid
/// over the new one and faded away, so the scene behind crossfades and nothing reloads.
pub fn fade_scene(section: &Element, media: &str) -> Result<(), JsValue> {
    let Some(screen) = section.query_selector(".ph-live")? else {
        return Ok(());
    };
    let before = screen.get_attribute(SCENE_ATTRIBUTE);
    if before.as_deref() == Some(media) {
        return Ok(());
    }
    if let Some(old) = screen.query_selector(":scope > .ph-scene-was")? {
        old.remove();
    }
    if let Some(before) = before {
        let was = element(&crate::dom::document()?, "div", "ph-scene-was")?;
        was.set_attribute(
            "style",
            &format!(
                "--ph-scene-was:url(\"{}\")",
                crate::path::media_file(&before)
            ),
        )?;
        screen.prepend_with_node_1(&was)?;
    }
    set_scene(&screen, Some(media))?;
    screen.set_attribute(SCENE_ATTRIBUTE, media)
}

/// The line a real phone (`role::PageRole::Phone`) shows above its screen while its link to the
/// host is down, in the words of the reconnecting status. It starts hidden; the page keeps it and
/// only shows or hides it (`show_link_line`), so a redraw never makes it flicker.
pub fn link_line(document: &Document) -> Result<Element, JsValue> {
    let words = status_copy(&Status::Reconnecting)
        .ok_or_else(|| JsValue::from_str("the reconnecting status has no words"))?;
    let line = kit_shell::link_bar(document, &words)?;
    line.set_attribute("hidden", "")?;
    Ok(line)
}

/// Shows the link line while the link is down, and hides it once the host answers again.
pub fn show_link_line(line: &Element, down: bool) -> Result<(), JsValue> {
    if down {
        line.remove_attribute("hidden")
    } else {
        line.set_attribute("hidden", "")
    }
}

fn head(document: &Document, scene: &PhoneScene) -> Result<Element, JsValue> {
    let head = element(document, "header", "phone-head")?;
    append(
        &head,
        &text_element(document, "div", "phone-hero", &scene.hero)?,
    )?;
    if let Some(goal) = &scene.goal {
        append(&head, &text_element(document, "div", "phone-goal", goal)?)?;
    }
    Ok(head)
}

fn narration(document: &Document, scene: &PhoneScene) -> Result<Option<Element>, JsValue> {
    if scene.narration.is_empty() {
        return Ok(None);
    }
    let lines = element(document, "div", "phone-lines")?;
    for line in &scene.narration {
        append(&lines, &text_element(document, "p", "phone-line", line)?)?;
    }
    Ok(Some(lines))
}

fn note(document: &Document, scene: &PhoneScene) -> Result<Option<Element>, JsValue> {
    let Some(note) = &scene.note else {
        return Ok(None);
    };
    let card = element(document, "div", "note-card")?;
    append(
        &card,
        &text_element(document, "div", "note-title", PRIVATE_NOTE_TITLE)?,
    )?;
    append(
        &card,
        &text_element(document, "div", "note-text", &note.text)?,
    )?;
    Ok(Some(card))
}

// TODO(DEV-12): browser test: Story opens at its newest line, follows new lines at the bottom, and
// leaves a reader who scrolled up alone.
/// The story's mark of whether it follows new lines: set to `off` once the reader scrolls up from
/// the bottom, `on` when they are back at it or the page has just opened.
const FOLLOW_ATTRIBUTE: &str = "data-follow";
/// The mark that the story's scroll listener is bound.
const FOLLOW_BOUND_ATTRIBUTE: &str = "data-follow-bound";
/// How far from the bottom, in pixels, still counts as reading the newest line.
const FOLLOW_SLACK: i32 = 24;

/// Scrolls the story to its newest line: when `reopen` is set (the page has just opened), or while
/// the reader has not scrolled up to read earlier lines. The narration is attached after this
/// returns, so the scroll waits for the next frame. The story is the page's one scroller.
pub fn follow_story(reopen: bool) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("the page has no window"))?;
    let follow = Closure::once_into_js(move || {
        let Some(story) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.query_selector(".ph-story").ok().flatten())
        else {
            return;
        };
        if story.get_attribute(FOLLOW_BOUND_ATTRIBUTE).is_none() {
            let watched = story.clone();
            let on_scroll = Closure::<dyn FnMut()>::new(move || {
                let gap = watched.scroll_height() - watched.scroll_top() - watched.client_height();
                let mark = if gap <= FOLLOW_SLACK { "on" } else { "off" };
                // A story that is hidden (another tab is open) has no height to judge by.
                if watched.client_height() > 0 {
                    let _ = watched.set_attribute(FOLLOW_ATTRIBUTE, mark);
                }
            });
            let bound = story
                .add_event_listener_with_callback("scroll", on_scroll.as_ref().unchecked_ref())
                .and_then(|()| story.set_attribute(FOLLOW_BOUND_ATTRIBUTE, ""));
            if bound.is_ok() {
                on_scroll.forget();
            }
        }
        if reopen {
            let _ = story.set_attribute(FOLLOW_ATTRIBUTE, "on");
        }
        if story.get_attribute(FOLLOW_ATTRIBUTE).as_deref() != Some("off") {
            story.set_scroll_top(story.scroll_height());
        }
    });
    window.request_animation_frame(follow.unchecked_ref())?;
    Ok(())
}

/// What a seated real phone draws from its seat in place of the scenario's: the Act page of a seat
/// that plays a character, and the header of the player's own hero.
#[derive(Debug, Clone, Copy, Default)]
pub struct LivePhone<'a> {
    pub act: Option<&'a SeatAct>,
    pub header: Option<&'a HeroHeader>,
    /// What the character saw when they last looked, under their private note.
    pub seen: &'a [String],
    /// The shared screen as it is now, while the phone plays a seat: what the phone mirrors of it
    /// (the party strip, the handout), drawn by the screen's own renderers.
    pub tv: Option<&'a TvScene>,
    /// A party ask another seat answered first (`SeatView::answered_by`), said on Act.
    pub answered_by: Option<&'a crate::seat::wire::AnsweredBy>,
}

/// What goes in the slot of `part` on a real phone, drawn with the phone kit. `live` is what the
/// seat puts in place of the scene: what the table asks it, and its player's own hero.
fn player_part(
    document: &Document,
    part: Part,
    scene: &PhoneScene,
    controls: &Controls,
    live: LivePhone<'_>,
) -> Result<Option<Element>, JsValue> {
    let act = live.act;
    match part {
        Part::PhoneHead if live.header.is_some() => {
            let Some(hero) = live.header else {
                return Ok(None);
            };
            let head = kit_shell::header(document, &hero.header)?;
            if let Some(action) = &hero.action {
                set_action(&head, action)?;
            }
            Ok(Some(head))
        }
        Part::PhoneHead => {
            let header = PhoneHeader {
                name: scene.hero.clone(),
                subtitle: scene.goal.clone(),
                portrait: None,
                hp: None,
            };
            kit_shell::header(document, &header).map(Some)
        }
        Part::PhoneStatus => {
            let status = controls.status_for(&scene.status);
            let Some(words) = status_copy(&status) else {
                return Ok(None);
            };
            let tone = if status_blocks_input(&status) {
                PillTone::Warn
            } else {
                PillTone::Accent
            };
            let pill = Pill {
                text: words,
                tone,
                glyph: None,
            };
            kit_shell::status(document, &pill).map(Some)
        }
        Part::PhoneImage => {
            // A handout the shared screen holds up is the Story page's picture too, full.
            let shown = match live.tv.and_then(|tv| tv.card.as_ref()) {
                Some(handout) => Some(picture(document, "ph-picture ph-handout", handout)?),
                None => scene
                    .image
                    .as_ref()
                    .map(|image| picture(document, "ph-picture", image))
                    .transpose()?,
            };
            // The chapter title the shared screen shows goes over it, drawn by the same renderer.
            let Some(card) = live.tv.and_then(|tv| tv.title_card.as_ref()) else {
                return Ok(shown);
            };
            let mirror = element(document, "div", "ph-mirror-title")?;
            let layer = crate::kit::draw::panels(
                document,
                &crate::kit::KitPanels {
                    title_cards: vec![card.clone()],
                    ..crate::kit::KitPanels::default()
                },
                None,
            )?;
            // TODO(DEV-12): browser test: the mirrored title card draws over the scene picture.
            // The card sits over the picture the phone already has, as the shared screen draws it
            // over its plate.
            if let Some(shown) = shown {
                append(&mirror, &shown)?;
            }
            append(&mirror, &layer)?;
            let block = element(document, "div", "ph-story-picture")?;
            append(&block, &mirror)?;
            Ok(Some(block))
        }
        Part::PhoneNarration => {
            let lines = kit_act::narration(document, &scene.narration)?;
            if lines.is_some() {
                follow_story(false)?;
            }
            Ok(lines)
        }
        Part::PhoneNote => scene
            .note
            .as_ref()
            .map(|note| kit_act::private_note_seen(document, &note.text, live.seen))
            .transpose(),
        Part::PhoneChoices if act.is_some() => {
            // The party as the shared screen shows it (the screen's own strip, smaller), then
            // what the table asks.
            let party = match live.tv {
                Some(tv) => crate::view::seats::strip(document, tv)?,
                None => None,
            };
            let tv_roll = live
                .tv
                .and_then(|tv| tv.roll.as_ref())
                .map(|shown| shown.view.id.as_str());
            let asked = act.map_or(Ok(None), |act| {
                crate::view::seat::act_block(document, act, tv_roll)
            })?;
            // Another seat answered the party's ask first: the party moved on its words.
            let answered = live
                .answered_by
                .map(|answered| {
                    let pill = Pill {
                        text: format!("{} chose: {}", answered.hero, answered.said),
                        tone: PillTone::Accent,
                        glyph: None,
                    };
                    kit_shell::status(document, &pill)
                })
                .transpose()?;
            // The shops here, each a card that opens its page.
            let shop_cards = crate::app::pages::shop_cards();
            let shops = if shop_cards.is_empty() {
                None
            } else {
                Some(crate::phone_kit::draw::page::block(
                    document,
                    &crate::phone_kit::page::Block::Cards {
                        cards: shop_cards,
                        small: true,
                    },
                )?)
            };
            if party.is_none() && answered.is_none() && shops.is_none() {
                return Ok(asked);
            }
            let block = element(document, "div", "ph-act-live")?;
            if let Some(party) = party {
                let mirror = element(document, "div", "ph-mirror-party")?;
                append(&mirror, &party)?;
                append(&block, &mirror)?;
            }
            if let Some(answered) = answered {
                append(&block, &answered)?;
            }
            if let Some(asked) = asked {
                append(&block, &asked)?;
            }
            if let Some(shops) = shops {
                append(&block, &shops)?;
            }
            Ok(Some(block))
        }
        Part::PhoneChoices => {
            let view = build(scene, controls.show_unavailable);
            let choices = ActChoices::from(&view);
            if choices.is_empty() {
                return Ok(None);
            }
            kit_act::choices(document, &choices).map(Some)
        }
        Part::PhoneComposer => Err(JsValue::from_str(
            "the composer is drawn by view::composer, which needs the composer's state",
        )),
        other => Err(JsValue::from_str(&format!(
            "{} is not a piece of a phone",
            other.dom_marker()
        ))),
    }
}

/// What goes in the slot of `part`, or `None` when the piece is empty. A part that is not of a
/// phone is an error: the caller asked the wrong view. `live` is what a seated real phone draws
/// from its seat.
pub fn build_part(
    document: &Document,
    part: Part,
    scene: &PhoneScene,
    controls: &Controls,
    role: PageRole,
    live: LivePhone<'_>,
) -> Result<Option<Element>, JsValue> {
    if role == PageRole::Phone {
        return player_part(document, part, scene, controls, live);
    }
    let status = controls.status_for(&scene.status);
    match part {
        Part::PhoneHead => head(document, scene).map(Some),
        Part::PhoneStatus => status_chip(document, &status),
        Part::PhoneImage => scene
            .image
            .as_ref()
            .map(|image| picture(document, "phone-picture", image))
            .transpose(),
        Part::PhoneNarration => narration(document, scene),
        Part::PhoneNote => note(document, scene),
        Part::PhoneChoices => {
            let view = build(scene, controls.show_unavailable);
            choice_list(document, &view).map(Some)
        }
        Part::PhoneComposer => Err(JsValue::from_str(
            "the composer is drawn by view::composer, which needs the composer's state",
        )),
        other => Err(JsValue::from_str(&format!(
            "{} is not a piece of a phone",
            other.dom_marker()
        ))),
    }
}

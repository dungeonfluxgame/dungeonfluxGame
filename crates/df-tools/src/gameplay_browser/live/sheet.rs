//! The player's accepted character sheet: a compact hero card in the scene, and the designed
//! `CharacterSheetSurface` in a full-screen overlay opened from it. Every value shown is a
//! `CharacterSheet` field or fact exactly as the server sent it; nothing is derived here, so
//! ability modifiers (which the wire does not carry) are not displayed.

use crate::gameplay_browser::{Client, button, feedback, node};
use df_protocol::common as rpc;
use df_ui::{
    CharacterSheetSurface, CharacterSheetView, SheetLabels, SheetOwnerGeneration, SheetRow,
    SheetSection, SheetTab,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{Element, Event};

const MOUNT_ID: &str = "live-sheet";
type CloseListener = Closure<dyn FnMut(Event)>;
const UNRECORDED: &str = "Not recorded";

const ABILITIES: [(&str, &str); 6] = [
    ("STR", "Strength"),
    ("DEX", "Dexterity"),
    ("CON", "Constitution"),
    ("INT", "Intelligence"),
    ("WIS", "Wisdom"),
    ("CHA", "Charisma"),
];
const IDENTITY: [&str; 10] = [
    "Species",
    "Class",
    "Background",
    "Alignment",
    "First language",
    "Second language",
    "Base language",
    "First class skill",
    "Second class skill",
    "Gaming set",
];
const FEATURES: [&str; 2] = ["Origin feat", "Masteries"];
const EQUIPMENT: [&str; 4] = ["Armor", "Weapon", "Starting equipment", "Greatsword"];

/// One open sheet. Dropping it removes its overlay, close listener and sheet surface.
pub(super) struct SheetOverlay {
    host: Element,
    surface: CharacterSheetSurface,
    close_button: Element,
    on_close: RefCell<Option<CloseListener>>,
    disposed: Cell<bool>,
}

impl SheetOverlay {
    fn dispose(&self) -> Result<(), JsValue> {
        if self.disposed.replace(true) {
            return Ok(());
        }
        let listener = match self.on_close.borrow().as_ref() {
            Some(callback) => self
                .close_button
                .remove_event_listener_with_callback("click", callback.as_ref().unchecked_ref()),
            None => Ok(()),
        };
        if listener.is_ok() {
            self.on_close.borrow_mut().take();
        }
        let surface = self.surface.dispose();
        self.host.remove();
        listener?;
        surface.map_err(|_| JsValue::from_str("character sheet cleanup failed"))
    }
}
impl Drop for SheetOverlay {
    fn drop(&mut self) {
        // A listener that could not be removed must stay callable for the page's lifetime.
        if self.dispose().is_err()
            && let Some(callback) = self.on_close.borrow_mut().take()
        {
            callback.forget();
        }
    }
}

struct OwnedRow {
    key: String,
    title: String,
    value: String,
    summary: &'static str,
}
struct OwnedSection {
    key: &'static str,
    tab: SheetTab,
    title: &'static str,
    caption: &'static str,
    rows: Vec<OwnedRow>,
}

fn nonblank(value: &str) -> &str {
    if value.trim().is_empty() {
        UNRECORDED
    } else {
        value
    }
}

fn fact<'a>(sheet: &'a rpc::CharacterSheet, label: &str) -> Option<&'a str> {
    sheet
        .facts
        .iter()
        .find(|fact| fact.label == label)
        .map(|fact| fact.value.as_str())
}

fn identity_line(sheet: &rpc::CharacterSheet) -> String {
    match (fact(sheet, "Species"), fact(sheet, "Class")) {
        (Some(species), Some(class)) => format!("{species} · {class}"),
        (None, Some(class)) => class.to_owned(),
        (Some(species), None) => species.to_owned(),
        (None, None) => "Adventurer".to_owned(),
    }
}

fn labelled_rows(
    section: &'static str,
    sheet: &rpc::CharacterSheet,
    labels: &[&str],
    summary: &'static str,
    used: &mut Vec<String>,
) -> Vec<OwnedRow> {
    let mut rows = Vec::new();
    for label in labels {
        if let Some(value) = fact(sheet, label) {
            used.push((*label).to_owned());
            rows.push(OwnedRow {
                key: format!("{section}-{}", rows.len()),
                title: (*label).to_owned(),
                value: nonblank(value).to_owned(),
                summary,
            });
        }
    }
    rows
}

fn sections(sheet: &rpc::CharacterSheet) -> Vec<OwnedSection> {
    let mut used = Vec::new();
    let mut vitals = vec![
        OwnedRow {
            key: "vitals-hit-points".to_owned(),
            title: "Hit points".to_owned(),
            value: format!("{} / {}", sheet.hit_points, sheet.maximum_hit_points),
            summary: "Current and maximum, as recorded by the server",
        },
        OwnedRow {
            key: "vitals-armor-class".to_owned(),
            title: "Armor Class".to_owned(),
            value: sheet.armor_class.to_string(),
            summary: "Recorded by the server",
        },
    ];
    if let Some(uses) = fact(sheet, "Second Wind uses") {
        used.push("Second Wind uses".to_owned());
        vitals.push(OwnedRow {
            key: "vitals-second-wind".to_owned(),
            title: "Second Wind".to_owned(),
            value: nonblank(uses).to_owned(),
            summary: "Uses remaining",
        });
    }
    if let Some(condition) = fact(sheet, "Condition") {
        used.push("Condition".to_owned());
        vitals.push(OwnedRow {
            key: "vitals-condition".to_owned(),
            title: "Condition".to_owned(),
            value: nonblank(condition).to_owned(),
            summary: "Current condition",
        });
    }
    let mut abilities = Vec::new();
    for (short, long) in ABILITIES {
        if let Some(score) = fact(sheet, short) {
            used.push(short.to_owned());
            abilities.push(OwnedRow {
                key: format!("abilities-{short}"),
                title: long.to_owned(),
                value: nonblank(score).to_owned(),
                summary: "Ability score",
            });
        }
    }
    let identity = labelled_rows(
        "identity",
        sheet,
        &IDENTITY,
        "Character identity",
        &mut used,
    );
    let features = labelled_rows(
        "features",
        sheet,
        &FEATURES,
        "Granted at creation",
        &mut used,
    );
    let equipment = labelled_rows("equipment", sheet, &EQUIPMENT, "Carried gear", &mut used);
    let other: Vec<OwnedRow> = sheet
        .facts
        .iter()
        .filter(|fact| !used.contains(&fact.label))
        .enumerate()
        .map(|(index, fact)| OwnedRow {
            key: format!("other-{index}"),
            title: nonblank(&fact.label).to_owned(),
            value: nonblank(&fact.value).to_owned(),
            summary: "Recorded fact",
        })
        .collect();
    [
        (
            "vitals",
            SheetTab::Character,
            "Combat statistics",
            "Hit points, armor and conditions",
            vitals,
        ),
        (
            "abilities",
            SheetTab::Character,
            "Ability scores",
            "As accepted at creation",
            abilities,
        ),
        (
            "identity",
            SheetTab::Character,
            "Who you are",
            "Ancestry, class, background and training",
            identity,
        ),
        (
            "features",
            SheetTab::Character,
            "Features",
            "Origin feat and weapon masteries",
            features,
        ),
        (
            "other",
            SheetTab::Character,
            "More details",
            "Other accepted facts",
            other,
        ),
        (
            "equipment",
            SheetTab::Equipment,
            "Equipment",
            "What you carry into the story",
            equipment,
        ),
    ]
    .into_iter()
    .filter(|(_, _, _, _, rows)| !rows.is_empty())
    .map(|(key, tab, title, caption, rows)| OwnedSection {
        key,
        tab,
        title,
        caption,
        rows,
    })
    .collect()
}

fn with_view<T>(
    sheet: &rpc::CharacterSheet,
    owner: u64,
    revision: u64,
    operation: impl FnOnce(&CharacterSheetView<'_>) -> T,
) -> T {
    let owned = sections(sheet);
    let rows: Vec<Vec<SheetRow<'_>>> = owned
        .iter()
        .map(|section| {
            section
                .rows
                .iter()
                .map(|row| SheetRow {
                    key: &row.key,
                    title: &row.title,
                    value: &row.value,
                    summary: row.summary,
                    details: &[],
                    offers: &[],
                })
                .collect()
        })
        .collect();
    let sections: Vec<SheetSection<'_>> = owned
        .iter()
        .zip(&rows)
        .map(|(section, rows)| SheetSection {
            key: section.key,
            tab: section.tab,
            title: section.title,
            caption: section.caption,
            rows,
        })
        .collect();
    let identity = identity_line(sheet);
    let subtitle = format!(
        "HP {} / {} · Armor Class {}",
        sheet.hit_points, sheet.maximum_hit_points, sheet.armor_class
    );
    let notice = format!(
        "Accepted by the server · rules {}",
        nonblank(&sheet.source_revision)
    );
    operation(&CharacterSheetView {
        owner: SheetOwnerGeneration(owner),
        revision,
        name: nonblank(&sheet.name),
        identity: &identity,
        subtitle: &subtitle,
        connection: "Connected · your private sheet",
        notice: &notice,
        labels: SheetLabels {
            tabs: ["Character", "Equipment", "Spells", "Journal", "Progression"],
            navigation: "Character sheet sections",
            filter: "Search this sheet",
            no_matches: "Nothing recorded here yet.",
            art_fallback: "Artwork unavailable",
        },
        sections: &sections,
    })
}

/// The accepted sheet and the presentation fence for the current retained view, if any.
fn current(client: &Client) -> Option<(rpc::CharacterSheet, u64, u64)> {
    let views = client.views.as_ref()?;
    let revision = views.current_revision()?;
    let Some(rpc::view_message::Audience::Player(player)) = &views.current()?.audience else {
        return None;
    };
    let sheet = player.journey.as_ref()?.own_character.clone()?;
    Some((sheet, revision.epoch().get(), revision.sequence()))
}

fn hit_point_bar(
    client: &Client,
    parent: &Element,
    current_points: u32,
    maximum: u32,
) -> Result<(), JsValue> {
    let bar = node(&client.document, parent, "div", "combat-hp", "")?;
    bar.set_attribute("role", "meter")?;
    bar.set_attribute("aria-label", "Hit points")?;
    bar.set_attribute("aria-valuemin", "0")?;
    bar.set_attribute("aria-valuemax", &maximum.to_string())?;
    bar.set_attribute("aria-valuenow", &current_points.to_string())?;
    // Presentation ratio only; the supplied numbers are shown unchanged.
    let percent = if maximum == 0 {
        0
    } else {
        u64::from(current_points.min(maximum)) * 100 / u64::from(maximum)
    };
    bar.set_attribute("style", &format!("--hp:{percent}%"))?;
    bar.set_attribute(
        "data-state",
        if current_points == 0 {
            "down"
        } else if percent <= 35 {
            "low"
        } else {
            "healthy"
        },
    )?;
    node(&client.document, &bar, "span", "combat-hp-fill", "")?;
    node(
        &client.document,
        &bar,
        "span",
        "combat-hp-text",
        &format!("{current_points} / {maximum} HP"),
    )?;
    Ok(())
}

/// Compact hero card used where the scene shows the player's own hero. It replaces the plain
/// collapsed list and opens the designed sheet.
pub(in crate::gameplay_browser) fn summary(
    client: &Rc<RefCell<Client>>,
    parent: &Element,
    sheet: &rpc::CharacterSheet,
) -> Result<(), JsValue> {
    super::ensure_style(&client.borrow().document)?;
    let document = client.borrow().document.clone();
    let card = node(&document, parent, "div", "live-hero", "")?;
    node(
        &document,
        &card,
        "strong",
        "live-hero-name",
        nonblank(&sheet.name),
    )?;
    node(
        &document,
        &card,
        "span",
        "live-hero-line",
        &format!(
            "{} · Armor Class {}",
            identity_line(sheet),
            sheet.armor_class
        ),
    )?;
    hit_point_bar(
        &client.borrow(),
        &card,
        sheet.hit_points,
        sheet.maximum_hit_points,
    )?;
    if let Some(uses) = fact(sheet, "Second Wind uses") {
        node(
            &document,
            &card,
            "span",
            "live-hero-line wind-uses",
            &format!("Second Wind · {} uses remaining", nonblank(uses)),
        )?;
    }
    let open = button(client, &card, "Open character sheet", open_from_button)?;
    open.set_class_name("live-sheet-open");
    Ok(())
}

/// The sheet button alone, for screens that already show the hero's state.
pub(super) fn summary_button(
    client: &Rc<RefCell<Client>>,
    parent: &Element,
    _sheet: &rpc::CharacterSheet,
) -> Result<(), JsValue> {
    super::ensure_style(&client.borrow().document)?;
    let open = button(client, parent, "Open character sheet", open_from_button)?;
    open.set_class_name("live-sheet-open");
    Ok(())
}

fn open_from_button(client: Rc<RefCell<Client>>) {
    if open_sheet(&client).is_err() {
        feedback(&client.borrow(), "The character sheet could not be opened.");
    }
}

/// Opens the designed sheet over the page for the current accepted view.
pub(super) fn open_sheet(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    if client.borrow().live.sheet.is_some() {
        return Ok(());
    }
    let (document, root, sheet, owner, revision) = {
        let state = client.borrow();
        let (sheet, owner, revision) =
            current(&state).ok_or_else(|| JsValue::from_str("accepted sheet absent"))?;
        (
            state.document.clone(),
            state.root.clone(),
            sheet,
            owner,
            revision,
        )
    };
    super::ensure_style(&document)?;
    let surface = with_view(&sheet, owner, revision, |view| {
        CharacterSheetSurface::create(&document, MOUNT_ID, view, |_| {})
    })
    .map_err(|_| JsValue::from_str("character sheet could not be displayed"))?;
    let host = document.create_element("div")?;
    host.set_class_name("live-sheet-overlay");
    host.set_attribute("role", "dialog")?;
    host.set_attribute("aria-modal", "true")?;
    host.set_attribute("aria-label", "Character sheet")?;
    host.append_child(surface.root())?;
    let close_button = node(
        &document,
        &host,
        "button",
        "live-sheet-close",
        "Close sheet",
    )?;
    let weak = Rc::downgrade(client);
    let on_close = Closure::wrap(Box::new(move |_: Event| {
        if let Some(client) = weak.upgrade()
            && close_sheet(&client).is_err()
        {
            feedback(&client.borrow(), "The character sheet could not be closed.");
        }
    }) as Box<dyn FnMut(Event)>);
    close_button.add_event_listener_with_callback("click", on_close.as_ref().unchecked_ref())?;
    root.append_child(&host)?;
    if let Ok(focusable) = close_button.clone().dyn_into::<web_sys::HtmlElement>() {
        focusable.focus()?;
    }
    client.borrow_mut().live.sheet = Some(SheetOverlay {
        host,
        surface,
        close_button,
        on_close: RefCell::new(Some(on_close)),
        disposed: Cell::new(false),
    });
    Ok(())
}

pub(super) fn close_sheet(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let overlay = client.borrow_mut().live.sheet.take();
    overlay.map_or(Ok(()), |overlay| overlay.dispose())
}

/// Keeps an open sheet current. It closes when the sheet is no longer available or when the
/// surface refuses the new view (for example a changed recovery epoch).
pub(in crate::gameplay_browser) fn sync(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let updated = {
        let state = client.borrow();
        if state.live.sheet.is_none() {
            return Ok(());
        }
        current(&state).map(|(sheet, owner, revision)| {
            with_view(&sheet, owner, revision, |view| {
                state
                    .live
                    .sheet
                    .as_ref()
                    .map(|open| open.surface.update(view))
            })
        })
    };
    match updated {
        Some(Some(Ok(()))) => Ok(()),
        _ => {
            close_sheet(client)?;
            feedback(
                &client.borrow(),
                "The character sheet was closed because your view changed.",
            );
            Ok(())
        }
    }
}

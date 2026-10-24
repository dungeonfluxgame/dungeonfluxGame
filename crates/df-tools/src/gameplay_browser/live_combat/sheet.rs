//! The player's accepted character sheet: a compact hero card in the scene, and the designed
//! `CharacterSheetSurface` in a full-screen overlay opened from it. All character facts come
//! from the server; the compact sheet also derives each ability modifier from its score.

use crate::{
    combat_text::{
        SHEET_ABILITIES_AND_SAVES, SHEET_COMBAT, SHEET_EQUIPMENT, SHEET_FEATURES_AND_TRAITS,
        SHEET_SKILLS,
    },
    gameplay_browser::{Client, button, feedback, node},
};
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
    "Gaming set",
    "First class skill",
    "Second class skill",
];
const FEATURES: [&str; 7] = [
    "Origin feat",
    "Masteries",
    "Size",
    "Darkvision",
    "Dwarven Resilience",
    "Dwarven Toughness",
    "Stonecunning",
];
const SKILLS: [&str; 3] = [
    "Soldier skill proficiencies",
    "First class skill",
    "Second class skill",
];
const COMBAT_FACTS: [&str; 5] = [
    "Proficiency Bonus",
    "Speed",
    "Initiative",
    "Passive Perception",
    "Hit Point Dice",
];
const SAVES: [&str; 1] = ["Saving throw proficiencies"];
const EQUIPMENT: [&str; 8] = [
    "Armor",
    "Weapon",
    "Starting equipment",
    "Greatsword",
    "Armor training",
    "Weapon proficiencies",
    "In hand",
    "Dropped weapons",
];

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
    let mut abilities_and_saves = Vec::new();
    for (short, long) in ABILITIES {
        if let Some(score) = fact(sheet, short) {
            used.push(short.to_owned());
            abilities_and_saves.push(OwnedRow {
                key: format!("abilities-{short}"),
                title: long.to_owned(),
                value: nonblank(score).to_owned(),
                summary: "Ability score",
            });
        }
    }
    abilities_and_saves.extend(labelled_rows(
        "saves",
        sheet,
        &SAVES,
        "Saving throw proficiency",
        &mut used,
    ));
    let skills = labelled_rows("skills", sheet, &SKILLS, "Skill proficiency", &mut used);
    let mut combat = labelled_rows(
        "combat-facts",
        sheet,
        &COMBAT_FACTS,
        "Combat statistic",
        &mut used,
    );
    combat.extend(vitals);
    let mut features_and_traits = labelled_rows(
        "identity",
        sheet,
        &IDENTITY[..8],
        "Character identity",
        &mut used,
    );
    features_and_traits.extend(labelled_rows(
        "features",
        sheet,
        &FEATURES,
        "Feature or trait",
        &mut used,
    ));
    let mut equipment = labelled_rows("equipment", sheet, &EQUIPMENT, "Carried gear", &mut used);
    equipment.extend(sheet.owned_items.iter().map(|item| {
        let holder_status = if item.holder_id.is_empty() {
            "Owned · current place".to_owned()
        } else if item.holder == sheet.name {
            "Owned · with you".to_owned()
        } else {
            format!("Owned · held by {}", nonblank(&item.holder))
        };
        let detail = if item.detail.trim().is_empty() {
            holder_status.clone()
        } else {
            format!("{} · {holder_status}", item.detail)
        };
        OwnedRow {
            key: format!("equipment-item-{}", item.thing_id),
            title: nonblank(&item.name).to_owned(),
            value: detail,
            summary: "Permanently owned item; holder is shown separately",
        }
    }));
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
            "abilities-and-saves",
            SheetTab::Character,
            SHEET_ABILITIES_AND_SAVES,
            "Scores and saving throw proficiencies",
            abilities_and_saves,
        ),
        (
            "skills",
            SheetTab::Character,
            SHEET_SKILLS,
            "Background and class proficiencies",
            skills,
        ),
        (
            "combat",
            SheetTab::Character,
            SHEET_COMBAT,
            "Hit points, armor, movement and resources",
            combat,
        ),
        (
            "equipment",
            SheetTab::Equipment,
            SHEET_EQUIPMENT,
            "What you carry into the story",
            equipment,
        ),
        (
            "features-and-traits",
            SheetTab::Character,
            SHEET_FEATURES_AND_TRAITS,
            "Character, origin and species features",
            features_and_traits,
        ),
        (
            "other",
            SheetTab::Character,
            "More details",
            "Other accepted facts",
            other,
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

#[cfg(test)]
mod tests {
    use super::*;

    // SRD 5.2.1 pp. 47, 83–84: Fighter training, Soldier skills, and Dwarf traits.
    #[test]
    fn srd_p47_p83_p84_character_facts_have_readable_sections() {
        let facts = [
            "STR",
            "Saving throw proficiencies",
            "Soldier skill proficiencies",
            "Proficiency Bonus",
            "Armor training",
            "Size",
        ]
        .into_iter()
        .map(|label| rpc::SheetFact {
            label: label.to_owned(),
            value: "recorded".to_owned(),
        })
        .collect();
        let sheet = rpc::CharacterSheet {
            facts,
            ..Default::default()
        };

        let sections = sections(&sheet);
        for title in [
            "Abilities and saves",
            "Skills",
            "Combat",
            "Equipment",
            "Features and traits",
        ] {
            assert!(sections.iter().any(|section| section.title == title));
        }
    }
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
    Some((
        crate::player_sheet::player_sheet_view(&sheet),
        revision.epoch().get(),
        revision.sequence(),
    ))
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

fn initial(name: &str) -> String {
    name.chars()
        .next()
        .map_or_else(|| "?".to_owned(), |first| first.to_uppercase().collect())
}

fn class_line(sheet: &rpc::CharacterSheet) -> String {
    let species = fact(sheet, "Species").unwrap_or("Adventurer");
    match fact(sheet, "Class") {
        Some(class) => format!("{species} {class}").replace("level", "Level"),
        None => species.to_owned(),
    }
}

/// The own-sheet image is the unique accepted party hero's server-published personal image.
pub(in crate::gameplay_browser) fn portrait_token<'a>(
    party: &'a [rpc::PartyMemberView],
    own: &rpc::CharacterSheet,
) -> &'a str {
    let mut matches = party
        .iter()
        .enumerate()
        .filter(|(_, member)| member.name == own.name);
    match (matches.next(), matches.next()) {
        (Some((_, member)), None) => {
            df_render::art_contract::player_hero_image(&member.portrait).unwrap_or("")
        }
        _ => "",
    }
}

/// A bounded df-ui decoration key, independent of stock art and accepted image availability.
pub(in crate::gameplay_browser) fn player_portrait_token(name: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(name.as_bytes()));
    format!("player-{}", &digest[..24])
}

pub(in crate::gameplay_browser) fn sync_personal_portraits(
    root: &Element,
    journey: &rpc::JourneyView,
) -> Result<(), JsValue> {
    let nodes = root.query_selector_all("[data-portrait]")?;
    for index in 0..nodes.length() {
        let Some(node) = nodes
            .item(index)
            .and_then(|node| node.dyn_into::<Element>().ok())
        else {
            continue;
        };
        let reference = node.get_attribute("data-portrait").unwrap_or_default();
        let personal = df_render::art_contract::player_hero_image(&reference).or_else(|| {
            journey
                .party
                .iter()
                .find(|member| player_portrait_token(&member.name) == reference)
                .and_then(|member| df_render::art_contract::player_hero_image(&member.portrait))
        });
        if let Some(url) = personal {
            node.set_attribute("data-personal-art", "")?;
            node.set_attribute("style", &format!("--personal-portrait:url({url})"))?;
        } else if node.has_attribute("data-personal-art") {
            node.remove_attribute("data-personal-art")?;
            node.remove_attribute("style")?;
        }
    }
    Ok(())
}

/// The modifier printed beside an ability score on a character sheet. This is display
/// formatting of the published ability-score table only; the wire carries scores alone.
fn modifier_text(score: &str) -> Option<String> {
    let score: i32 = score.trim().parse().ok()?;
    Some(format!("{:+}", (score - 10).div_euclid(2)))
}

fn portrait(
    client: &Rc<RefCell<Client>>,
    parent: &Element,
    token: &str,
    name: &str,
) -> Result<(), JsValue> {
    let document = client.borrow().document.clone();
    let portrait = node(&document, parent, "div", "live-portrait", "")?;
    portrait.set_attribute("data-portrait", token)?;
    if let Some(url) = df_render::art_contract::player_hero_image(token) {
        portrait.set_attribute("data-personal-art", "")?;
        portrait.set_attribute("style", &format!("--personal-portrait:url({url})"))?;
    }
    node(&document, &portrait, "span", "", &initial(name))?;
    Ok(())
}

/// The hero's portrait, name, class, hit points and Second Wind, shown above a player screen.
pub(super) fn mount_head(
    client: &Rc<RefCell<Client>>,
    host: &Element,
    sheet: &rpc::CharacterSheet,
    token: &str,
) -> Result<(), JsValue> {
    let document = client.borrow().document.clone();
    if let Some(previous) = host.query_selector(":scope > .live-hero-head")? {
        previous.remove();
    }
    let head = node(&document, host, "div", "live-hero-head", "")?;
    if df_render::art_contract::player_hero_image(&sheet.portrait_uri).is_some() {
        let image = node(&document, &head, "img", "live-portrait", "")?;
        image.set_attribute("src", &sheet.portrait_uri)?;
        image.set_attribute("alt", &format!("{} portrait", sheet.name))?;
        image.set_attribute("style", "object-fit:cover;object-position:center top")?;
    } else {
        portrait(client, &head, token, &sheet.name)?;
    }
    let meta = node(&document, &head, "div", "live-hero-meta", "")?;
    node(
        &document,
        &meta,
        "strong",
        "live-hero-name",
        nonblank(&sheet.name),
    )?;
    node(
        &document,
        &meta,
        "span",
        "live-hero-line",
        &class_line(sheet),
    )?;
    hit_point_bar(
        &client.borrow(),
        &meta,
        sheet.hit_points,
        sheet.maximum_hit_points,
    )?;
    if let Some(uses) = fact(sheet, "Second Wind uses") {
        node(
            &document,
            &meta,
            "span",
            "live-hero-line wind-uses",
            &format!("Second Wind · {} remaining", nonblank(uses)),
        )?;
    }
    host.insert_before(&head, host.first_child().as_ref())?;
    Ok(())
}

fn list(
    client: &Rc<RefCell<Client>>,
    parent: &Element,
    heading: &str,
    items: &[(&str, String)],
) -> Result<(), JsValue> {
    if items.is_empty() {
        return Ok(());
    }
    let document = client.borrow().document.clone();
    let group = node(&document, parent, "div", "", "")?;
    node(&document, &group, "h3", "", heading)?;
    let list = node(&document, &group, "ul", "", "")?;
    for (label, value) in items {
        let item = node(&document, &list, "li", "", "")?;
        node(&document, &item, "b", "", label)?;
        item.append_with_str_1(&format!(" {}", nonblank(value)))?;
    }
    Ok(())
}

fn joined(sheet: &rpc::CharacterSheet, labels: &[&str]) -> String {
    labels
        .iter()
        .filter_map(|label| fact(sheet, label))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The accepted character sheet as an inline `#hero-sheet` section: portrait, name, class,
/// hit points, the six ability scores, then proficiencies, equipment and features.
pub(in crate::gameplay_browser) fn mount_inline(
    client: &Rc<RefCell<Client>>,
    parent: &Element,
    sheet: &rpc::CharacterSheet,
    token: &str,
) -> Result<(), JsValue> {
    let view = crate::player_sheet::player_sheet_view(sheet);
    let sheet = &view;
    let document = client.borrow().document.clone();
    super::ensure_style(&document)?;
    if let Some(previous) = parent.query_selector("#hero-sheet")? {
        previous.remove();
    }
    let section = node(&document, parent, "section", "live-sheet-inline", "")?;
    section.set_id("hero-sheet");
    section.set_attribute("aria-label", "Character sheet")?;
    let head = node(&document, &section, "div", "live-sheet-head", "")?;
    if df_render::art_contract::player_hero_image(&sheet.portrait_uri).is_some() {
        let image = node(&document, &head, "img", "live-portrait", "")?;
        image.set_attribute("src", &sheet.portrait_uri)?;
        image.set_attribute("alt", &format!("{} portrait", sheet.name))?;
        image.set_attribute("style", "object-fit:cover;object-position:center top")?;
    } else {
        portrait(client, &head, token, &sheet.name)?;
    }
    let meta = node(&document, &head, "div", "live-hero-meta", "")?;
    node(
        &document,
        &meta,
        "strong",
        "live-hero-name",
        nonblank(&sheet.name),
    )?;
    node(
        &document,
        &meta,
        "span",
        "live-hero-line",
        &class_line(sheet),
    )?;
    hit_point_bar(
        &client.borrow(),
        &meta,
        sheet.hit_points,
        sheet.maximum_hit_points,
    )?;
    node(
        &document,
        &meta,
        "span",
        "live-hero-line",
        &format!("Armor Class {}", sheet.armor_class),
    )?;
    if let Some(coin) = fact(sheet, "Coin") {
        node(
            &document,
            &meta,
            "span",
            "live-hero-line live-coin-balance",
            &format!("Coin · {}", nonblank(coin)),
        )?;
    }
    node(&document, &section, "h2", "", SHEET_ABILITIES_AND_SAVES)?;
    let grid = node(&document, &section, "div", "live-abilities", "")?;
    for (short, long) in ABILITIES {
        let Some(score) = fact(sheet, short) else {
            continue;
        };
        let cell = node(&document, &grid, "div", "live-ability", "")?;
        cell.set_attribute("title", long)?;
        node(&document, &cell, "span", "live-ability-label", short)?;
        node(
            &document,
            &cell,
            "span",
            "live-ability-score",
            nonblank(score),
        )?;
        node(
            &document,
            &cell,
            "span",
            "live-ability-mod",
            &modifier_text(score).unwrap_or_default(),
        )?;
    }
    let saves: Vec<(&str, String)> = SAVES
        .iter()
        .filter_map(|label| fact(sheet, label).map(|value| (*label, value.to_owned())))
        .collect();
    list(client, &section, "Saving throws", &saves)?;
    let mut skills = Vec::new();
    if let Some(soldier) = fact(sheet, "Soldier skill proficiencies") {
        skills.push(("Soldier skills", soldier.to_owned()));
    }
    let class_skills = joined(sheet, &["First class skill", "Second class skill"]);
    if !class_skills.is_empty() {
        skills.push(("Fighter skills", class_skills));
    }
    list(client, &section, SHEET_SKILLS, &skills)?;
    let combat: Vec<(&str, String)> = COMBAT_FACTS
        .iter()
        .filter_map(|label| fact(sheet, label).map(|value| (*label, value.to_owned())))
        .collect();
    list(client, &section, SHEET_COMBAT, &combat)?;
    let mut proficiencies = Vec::new();
    if let Some(set) = fact(sheet, "Gaming set") {
        proficiencies.push(("Gaming set", set.to_owned()));
    }
    let languages = joined(
        sheet,
        &["Base language", "First language", "Second language"],
    );
    if !languages.is_empty() {
        proficiencies.push(("Languages", languages));
    }
    list(client, &section, "Other proficiencies", &proficiencies)?;
    let mut gear: Vec<(String, String)> = EQUIPMENT
        .iter()
        .filter_map(|label| fact(sheet, label).map(|value| ((*label).to_owned(), value.to_owned())))
        .collect();
    gear.extend(sheet.owned_items.iter().map(|item| {
        let mut value = item.detail.clone();
        if !item.holder_id.is_empty() {
            let holder = if item.holder == sheet.name {
                "with you".to_owned()
            } else {
                format!("held by {}", nonblank(&item.holder))
            };
            if value.is_empty() {
                value = holder;
            } else {
                value.push_str(" · ");
                value.push_str(&holder);
            }
        }
        (item.name.clone(), value)
    }));
    let gear_rows: Vec<(&str, String)> = gear
        .iter()
        .map(|(label, value)| (label.as_str(), value.clone()))
        .collect();
    list(client, &section, SHEET_EQUIPMENT, &gear_rows)?;
    let features: Vec<(&str, String)> = [
        "Origin feat",
        "Masteries",
        "Alignment",
        "Size",
        "Darkvision",
        "Dwarven Resilience",
        "Dwarven Toughness",
        "Stonecunning",
    ]
    .iter()
    .filter_map(|label| fact(sheet, label).map(|value| (*label, value.to_owned())))
    .collect();
    list(client, &section, SHEET_FEATURES_AND_TRAITS, &features)?;
    let open = button(client, &section, "Full character sheet", open_from_button)?;
    open.set_class_name("live-sheet-open");
    Ok(())
}

/// The inline sheet for the generic scene, where the hero has no party portrait context.
pub(in crate::gameplay_browser) fn summary(
    client: &Rc<RefCell<Client>>,
    parent: &Element,
    sheet: &rpc::CharacterSheet,
) -> Result<(), JsValue> {
    mount_inline(
        client,
        parent,
        sheet,
        df_render::art_contract::player_hero_image(&sheet.portrait_uri).unwrap_or(""),
    )
}

fn open_from_button(client: Rc<RefCell<Client>>) {
    if open_sheet(&client).is_err() {
        feedback(&client.borrow(), "The character sheet could not be opened.");
    }
}

/// Opens the designed sheet over the page for the current accepted view.
pub(super) fn open_sheet(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    if client.borrow().combat.sheet.is_some() {
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
    client.borrow_mut().combat.sheet = Some(SheetOverlay {
        host,
        surface,
        close_button,
        on_close: RefCell::new(Some(on_close)),
        disposed: Cell::new(false),
    });
    Ok(())
}

pub(super) fn close_sheet(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let overlay = client.borrow_mut().combat.sheet.take();
    overlay.map_or(Ok(()), |overlay| overlay.dispose())
}

/// Keeps an open sheet current. It closes when the sheet is no longer available or when the
/// surface refuses the new view (for example a changed recovery epoch).
pub(in crate::gameplay_browser) fn sync(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let updated = {
        let state = client.borrow();
        if state.combat.sheet.is_none() {
            return Ok(());
        }
        current(&state).map(|(sheet, owner, revision)| {
            with_view(&sheet, owner, revision, |view| {
                state
                    .combat
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

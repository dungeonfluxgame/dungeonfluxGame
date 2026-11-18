//! The text the `creation` verbs print: the offers with their status, a seat's state and what its
//! build still asks for or the derived numbers, the table as the TV sees it, and the start.
use df_playground::pages::creation::{BuildState, CreationView};
use df_rules::hero::{HeroOffer, HeroOffers, OfferStatus};

use super::request::{StartView, TableView};

fn offer_line(offer: &HeroOffer) -> String {
    match &offer.status {
        OfferStatus::Playable => format!("  {} ({}): playable", offer.id, offer.title),
        OfferStatus::DesignOnly { reason, note } => format!(
            "  {} ({}): not in play yet [{}] {note}",
            offer.id,
            offer.title,
            reason.key()
        ),
    }
}

pub(super) fn format_offers(offers: &HeroOffers) -> String {
    let mut lines = vec!["species:".to_owned()];
    lines.extend(offers.species.iter().map(offer_line));
    lines.push("classes:".to_owned());
    lines.extend(offers.classes.iter().map(offer_line));
    lines.push("backgrounds:".to_owned());
    lines.extend(offers.backgrounds.iter().map(offer_line));
    lines.join("\n")
}

pub(super) fn format_table(view: &TableView) -> String {
    let mut lines = vec![if view.started {
        "creation: play has started".to_owned()
    } else {
        "creation: open".to_owned()
    }];
    for seat in &view.seats {
        let name = seat.name.as_deref().unwrap_or("(no hero yet)");
        lines.push(format!("  seat {} {name}: {}", seat.seat, seat.state.key()));
    }
    lines.join("\n")
}

/// A seat: its state and hero, the derived numbers and the marks once the build is legal, then the
/// problems and every slot still empty with its options.
pub(super) fn format_seat(seat: u64, view: &CreationView) -> String {
    let state = match view.state {
        BuildState::Drafting => "drafting",
        BuildState::Reviewing => "reviewing",
        BuildState::Ready => "ready",
    };
    let hero = view.hero.as_ref().map_or_else(
        || "not begun".to_owned(),
        |hero| format!("{} ({} {})", hero.name, hero.species, hero.class),
    );
    let mut lines = vec![format!("seat {seat} {state}: {hero}")];
    if let Some(sheet) = &view.sheet {
        let speed = sheet
            .speed_feet
            .map_or_else(|| "not in play yet".to_owned(), |feet| feet.to_string());
        lines.push(format!(
            "  hp {} | ac {} | initiative {:+} | speed {speed} | passive perception {}",
            sheet.hit_points, sheet.armor_class, sheet.initiative, sheet.passive_perception
        ));
        let abilities: Vec<String> = sheet
            .abilities
            .iter()
            .map(|ability| format!("{} {}", ability.key, ability.score))
            .collect();
        lines.push(format!("  abilities {}", abilities.join(" ")));
        let skills: Vec<String> = sheet
            .skills
            .iter()
            .filter(|skill| skill.source.is_some())
            .map(|skill| format!("{} {:+}", skill.name, skill.bonus))
            .collect();
        lines.push(format!("  skills {}", skills.join(", ")));
        for mark in &sheet.marks {
            lines.push(format!("  not in play yet: {}", mark.note));
        }
    }
    let picks: Vec<String> = view
        .picks
        .iter()
        .map(|(slot, option)| format!("{slot}={option}"))
        .collect();
    if !picks.is_empty() {
        lines.push(format!("  picks {}", picks.join(" ")));
    }
    for issue in &view.issues {
        lines.push(format!("  - {issue}"));
    }
    for group in &view.groups {
        let empty: Vec<&str> = group
            .slots
            .iter()
            .filter(|slot| !view.picks.contains_key(*slot))
            .map(String::as_str)
            .collect();
        if empty.is_empty() {
            continue;
        }
        let options: Vec<&str> = group
            .options
            .iter()
            .map(|option| option.id.as_str())
            .collect();
        lines.push(format!(
            "  {} [{}]: {}",
            group.label,
            empty.join(", "),
            options.join(" ")
        ));
    }
    match view.state {
        BuildState::Reviewing => lines.push(format!("  ready to submit: creation submit {seat}")),
        BuildState::Drafting if view.hero.is_none() => lines.push(format!(
            "  start a hero in the look studio, then: creation begin {seat}"
        )),
        BuildState::Drafting | BuildState::Ready => {}
    }
    lines.join("\n")
}

pub(super) fn format_start(view: &StartView) -> String {
    let mut lines = vec![format!("play starts in {}", view.next_machine)];
    for hero in &view.heroes {
        lines.push(format!(
            "  seat {} hero {} {}: hp {} ac {} ({} not in play yet)",
            hero.seat,
            hero.hero.hero,
            hero.hero.name,
            hero.sheet["hit_points"]["maximum"],
            hero.sheet["armor_class"]["value"],
            hero.marks.len()
        ));
    }
    lines.join("\n")
}

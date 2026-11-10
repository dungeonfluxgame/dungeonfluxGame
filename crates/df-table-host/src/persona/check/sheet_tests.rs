//! The warrior's sheet and the gauntlet's stations, read from the data files the command line
//! reads.

use df_rules::roll::{Ability, AttackKind, Skill};

use super::request::CheckKind;
use super::sheet::{read_sheet, read_station};

const SHEET: &str =
    include_str!("../../../../../development/market-threads/gauntlet_warrior.sheet.json");
const STATIONS: &str =
    include_str!("../../../../../development/market-threads/gauntlet.stations.json");

#[test]
fn the_warriors_sheet_gives_a_roll_its_modifiers_proficiencies_and_critical_range() {
    let sheet = read_sheet(SHEET).expect("the sheet reads");
    assert_eq!(sheet.id, "gauntlet_warrior");
    assert_eq!(sheet.proficiency_bonus, 2);
    assert_eq!(sheet.modifier(Ability::Strength), Ok(3));
    assert_eq!(sheet.modifier(Ability::Charisma), Ok(-1));
    assert_eq!(
        sheet.saving_throw_proficiencies,
        [Ability::Strength, Ability::Constitution]
    );
    for skill in [
        Skill::Athletics,
        Skill::Intimidation,
        Skill::Perception,
        Skill::Survival,
    ] {
        assert!(sheet.skill_proficiencies.contains(&skill), "{skill:?}");
    }
    assert_eq!(sheet.skill_proficiencies.len(), 4);
    assert_eq!(sheet.critical_from, 19);
    assert!(
        sheet.initiative_proficient,
        "Alert adds the Proficiency Bonus to initiative"
    );
}

#[test]
fn the_sheets_weapons_and_healing_are_the_choices_a_roll_may_use() {
    let sheet = read_sheet(SHEET).expect("the sheet reads");
    let longsword = sheet.attack("longsword").expect("a longsword");
    assert_eq!(longsword.kind, AttackKind::Melee);
    assert_eq!(longsword.ability, Ability::Strength);
    assert_eq!(longsword.damage.to_string(), "1d8+3");
    let longbow = sheet.attack("longbow").expect("a longbow");
    assert_eq!(longbow.kind, AttackKind::Ranged);
    assert_eq!(longbow.ability, Ability::Dexterity);
    assert_eq!(
        sheet
            .healing_of("second_wind")
            .map(|healing| healing.dice.to_string()),
        Some("1d10+3".to_owned())
    );
    assert!(sheet.equipment.iter().any(|item| item == "chain mail"));
    assert!(
        sheet
            .features
            .iter()
            .any(|feature| feature.name == "champion_remarkable_athlete"),
        "the features the judge reads for a grounded advantage"
    );
}

#[test]
fn a_sheet_that_does_not_read_says_what_is_wrong() {
    assert!(read_sheet("{}").is_err());
    let unknown_ability = SHEET.replacen("\"str\": 3", "\"might\": 3", 1);
    let message = read_sheet(&unknown_ability).expect_err("an unknown ability");
    assert!(message.contains("might"), "{message}");
    let unknown_skill = SHEET.replacen("\"perception\": \"Fighter", "\"sneezing\": \"Fighter", 1);
    let message = read_sheet(&unknown_skill).expect_err("an unknown skill");
    assert!(message.contains("sneezing"), "{message}");
}

#[test]
fn every_station_of_the_gauntlet_reads() {
    let listed: serde_json::Value = serde_json::from_str(STATIONS).expect("the file is JSON");
    let ids: Vec<&str> = listed["stations"]
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|station| station["id"].as_str())
        .collect();
    assert_eq!(ids.len(), 39);
    for id in ids {
        read_station(STATIONS, id).unwrap_or_else(|message| panic!("{id}: {message}"));
    }
}

#[test]
fn a_station_gives_its_setting_situation_and_designed_test() {
    let athletics = read_station(STATIONS, "athletics").expect("the station reads");
    assert_eq!(athletics.kind, CheckKind::AbilityCheck);
    assert_eq!(athletics.ability, Some(Ability::Strength));
    assert_eq!(athletics.skill, Some(Skill::Athletics));
    let death = read_station(STATIONS, "death_save").expect("the station reads");
    assert_eq!(
        (death.kind, death.ability, death.skill),
        (CheckKind::DeathSave, None, None)
    );
    assert!(read_station(STATIONS, "no_such_station").is_err());
}

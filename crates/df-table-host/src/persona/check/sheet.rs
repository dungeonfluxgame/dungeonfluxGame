//! Reading a character sheet and a gauntlet station from their data files into the wire values the
//! check routes take.
//!
//! The sheet file is an authored document with notes and numbers the check does not read, so only
//! the fields a roll needs are taken; a field the file gives that does not read is a message
//! naming it, which the command line shows as an unreadable file. Abilities are the sheet's short
//! keys (`str`, `dex`), skills the roll vocabulary's keys. Nothing here decides a roll: the
//! Proficiency Bonus, the modifiers and the weapons are the file's.

use std::collections::BTreeMap;
use std::str::FromStr;

use df_rules::roll::{Ability, AttackKind, DiceExpression, ModifierLabel, Skill};
use serde::Deserialize;
use serde_json::Value;

use super::request::{
    CheckKind, CheckSheet, CheckStation, SheetAttack, SheetFeature, SheetHealing,
};

/// The critical range start of a weapon attack with no Improved Critical (SRD 5.2.1 page 7).
const PLAIN_CRITICAL_FROM: u8 = 20;

#[derive(Deserialize)]
struct InitiativeFile {
    value: i16,
}

#[derive(Deserialize)]
struct AttackFile {
    id: String,
    name: String,
    kind: String,
    ability: String,
    damage: String,
}

#[derive(Deserialize)]
struct HealingFile {
    id: String,
    dice: String,
    #[serde(default)]
    bonus_label: Option<String>,
}

#[derive(Deserialize)]
struct SheetFile {
    id: String,
    proficiency_bonus: u8,
    ability_modifiers: BTreeMap<String, i8>,
    #[serde(default)]
    saving_throw_proficiencies: Vec<String>,
    #[serde(default)]
    skill_proficiencies: BTreeMap<String, Value>,
    #[serde(default)]
    expertise: Vec<String>,
    #[serde(default)]
    initiative_modifier: Option<InitiativeFile>,
    #[serde(default)]
    critical_range: Vec<u8>,
    #[serde(default)]
    attacks: Vec<AttackFile>,
    #[serde(default)]
    healing: Vec<HealingFile>,
    #[serde(default)]
    equipment: Vec<String>,
    #[serde(default)]
    class_features: BTreeMap<String, Value>,
    #[serde(default)]
    species_traits: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct StationFile {
    id: String,
    title: String,
    kind: String,
    #[serde(default)]
    ability: Option<String>,
    #[serde(default)]
    skill: Option<String>,
    setting: String,
    situation: String,
}

#[derive(Deserialize)]
struct StationsFile {
    stations: Vec<StationFile>,
}

fn ability_of(short: &str) -> Result<Ability, String> {
    match short {
        "str" => Ok(Ability::Strength),
        "dex" => Ok(Ability::Dexterity),
        "con" => Ok(Ability::Constitution),
        "int" => Ok(Ability::Intelligence),
        "wis" => Ok(Ability::Wisdom),
        "cha" => Ok(Ability::Charisma),
        _ => Err(format!(
            "'{short}' is not an ability (str, dex, con, int, wis or cha)"
        )),
    }
}

fn skill_of(key: &str) -> Result<Skill, String> {
    Skill::from_key(key).ok_or_else(|| format!("'{key}' is not a skill"))
}

fn attack_kind_of(kind: &str) -> Result<AttackKind, String> {
    match kind {
        "melee" | "melee_or_thrown" => Ok(AttackKind::Melee),
        "ranged" => Ok(AttackKind::Ranged),
        _ => Err(format!("'{kind}' is not an attack kind")),
    }
}

fn dice_of(text: &str) -> Result<DiceExpression, String> {
    DiceExpression::from_str(text).map_err(|error| format!("'{text}': {error}"))
}

/// The source a healing's flat bonus is named by; no label is a flat number.
fn bonus_label_of(key: Option<&str>) -> Result<ModifierLabel, String> {
    match key {
        None => Ok(ModifierLabel::Flat),
        Some(key) => ModifierLabel::from_key(key)
            .ok_or_else(|| format!("'{key}' is not a modifier label for a healing bonus")),
    }
}

/// The features of the sheet that are written as a sentence, in the sheet's words.
fn features_of(groups: [&BTreeMap<String, Value>; 2]) -> impl Iterator<Item = SheetFeature> + '_ {
    groups.into_iter().flat_map(|group| {
        group.iter().filter_map(|(name, value)| {
            value.as_str().map(|text| SheetFeature {
                name: name.clone(),
                text: text.to_owned(),
            })
        })
    })
}

/// The sheet the file `text` holds, or why it does not read.
pub(super) fn read_sheet(text: &str) -> Result<CheckSheet, String> {
    let file: SheetFile = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let ability_modifiers = file
        .ability_modifiers
        .iter()
        .map(|(short, modifier)| ability_of(short).map(|ability| (ability, *modifier)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let dexterity = ability_modifiers
        .get(&Ability::Dexterity)
        .copied()
        .unwrap_or_default();
    let initiative_proficient = file.initiative_modifier.as_ref().is_some_and(|initiative| {
        initiative.value - i16::from(dexterity) == i16::from(file.proficiency_bonus)
    });
    let attacks = file
        .attacks
        .iter()
        .map(|attack| {
            Ok(SheetAttack {
                id: attack.id.clone(),
                name: attack.name.clone(),
                kind: attack_kind_of(&attack.kind)?,
                ability: ability_of(&attack.ability)?,
                damage: dice_of(&attack.damage)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let healing = file
        .healing
        .iter()
        .map(|healing| {
            Ok(SheetHealing {
                id: healing.id.clone(),
                dice: dice_of(&healing.dice)?,
                bonus_label: bonus_label_of(healing.bonus_label.as_deref())?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(CheckSheet {
        id: file.id,
        proficiency_bonus: file.proficiency_bonus,
        ability_modifiers,
        saving_throw_proficiencies: file
            .saving_throw_proficiencies
            .iter()
            .map(|short| ability_of(short))
            .collect::<Result<_, _>>()?,
        skill_proficiencies: file
            .skill_proficiencies
            .keys()
            .map(|key| skill_of(key))
            .collect::<Result<_, _>>()?,
        skill_expertise: file
            .expertise
            .iter()
            .map(|key| skill_of(key))
            .collect::<Result<_, _>>()?,
        initiative_proficient,
        critical_from: file
            .critical_range
            .iter()
            .copied()
            .min()
            .unwrap_or(PLAIN_CRITICAL_FROM),
        attacks,
        healing,
        equipment: file.equipment.clone(),
        features: features_of([&file.class_features, &file.species_traits]).collect(),
    })
}

/// The station `id` of the stations file `text`, or why it is not there.
pub(super) fn read_station(text: &str, id: &str) -> Result<CheckStation, String> {
    let file: StationsFile = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let station = file
        .stations
        .into_iter()
        .find(|station| station.id == id)
        .ok_or_else(|| format!("no station '{id}'"))?;
    Ok(CheckStation {
        id: station.id,
        title: station.title,
        kind: CheckKind::from_key(&station.kind)
            .ok_or_else(|| format!("'{}' is not a kind of check", station.kind))?,
        ability: station.ability.as_deref().map(ability_of).transpose()?,
        skill: station.skill.as_deref().map(skill_of).transpose()?,
        setting: station.setting,
        situation: station.situation,
    })
}

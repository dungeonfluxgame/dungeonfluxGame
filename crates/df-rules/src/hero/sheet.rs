//! The derived sheet of a hero: every number the rules give it at level 1, in the shape of the
//! gauntlet's character sheet file (`development/market-threads/gauntlet_warrior.sheet.json`), so
//! the check judge reads it as it reads that file, with the design-only marks beside it.
//!
//! Owns the sheet's types and their serialized field names. Never does: compute a number
//! (`derive` does) or hold a value the rules did not give (a part with no source is `None` and
//! marked, never a stand-in).
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::choices::DesignOnlyReason;
use crate::roll::{Ability, AttackStyle, DieSize, Skill};

/// A value for each ability, written with the sheet's short keys.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct AbilityTable<T> {
    #[serde(rename = "str")]
    pub strength: T,
    #[serde(rename = "dex")]
    pub dexterity: T,
    #[serde(rename = "con")]
    pub constitution: T,
    #[serde(rename = "int")]
    pub intelligence: T,
    #[serde(rename = "wis")]
    pub wisdom: T,
    #[serde(rename = "cha")]
    pub charisma: T,
}

impl<T: Copy> AbilityTable<T> {
    /// The table of `values`, in the order of `Ability::ALL`.
    pub fn from_array(values: [T; 6]) -> Self {
        let [
            strength,
            dexterity,
            constitution,
            intelligence,
            wisdom,
            charisma,
        ] = values;
        Self {
            strength,
            dexterity,
            constitution,
            intelligence,
            wisdom,
            charisma,
        }
    }

    pub fn get(&self, ability: Ability) -> T {
        match ability {
            Ability::Strength => self.strength,
            Ability::Dexterity => self.dexterity,
            Ability::Constitution => self.constitution,
            Ability::Intelligence => self.intelligence,
            Ability::Wisdom => self.wisdom,
            Ability::Charisma => self.charisma,
        }
    }
}

/// The sheet's short key of an ability (`str`, `dex`).
pub fn short_key(ability: Ability) -> &'static str {
    match ability {
        Ability::Strength => "str",
        Ability::Dexterity => "dex",
        Ability::Constitution => "con",
        Ability::Intelligence => "int",
        Ability::Wisdom => "wis",
        Ability::Charisma => "cha",
    }
}

/// A number and how it was made.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Explained<T> {
    pub value: T,
    pub note: String,
}

/// The Hit Point maximum and how it was made.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HitPoints {
    pub maximum: u32,
    pub note: String,
}

/// One weapon attack, as the check judge reads it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SheetAttack {
    pub id: String,
    pub name: String,
    /// `melee`, `melee_or_thrown` or `ranged`.
    pub kind: &'static str,
    /// The sheet's short key of the ability the attack uses.
    pub ability: &'static str,
    pub attack_bonus: i16,
    /// The damage dice and modifier, as `1d8+3`.
    pub damage: String,
    pub damage_type: &'static str,
    /// The Fighting Style that touches this attack's rolls, for `RollStyle::of`: Archery on a
    /// Ranged weapon, Great Weapon Fighting on a two-handed Melee weapon.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<AttackStyle>,
}

/// One healing roll, as the check judge reads it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SheetHealing {
    pub id: &'static str,
    pub dice: String,
    pub bonus_label: &'static str,
}

/// How a class feature in play is used.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureStatus {
    /// It works now: in checks, rests, casting or the sheet's numbers.
    InPlay,
    /// It is shown on the sheet and applies when combat arrives (Sneak Attack, Weapon Mastery).
    AppliesInCombat,
}

impl FeatureStatus {
    pub fn key(self) -> &'static str {
        match self {
            Self::InPlay => "in_play",
            Self::AppliesInCombat => "applies_in_combat",
        }
    }
}

/// One weapon whose mastery property the hero can use.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SheetMastery {
    /// The weapon's id, as its attack names it (`dagger`).
    pub id: &'static str,
    pub name: &'static str,
    /// The mastery property's title (`Nick`).
    pub mastery: &'static str,
}

/// One spell the hero knows: a cantrip, or a level 1+ spell of its spellbook.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SheetSpell {
    /// The catalog id (`crate::spells`), which a cast names.
    pub id: &'static str,
    pub name: &'static str,
    /// 0 for a cantrip.
    pub level: u8,
    pub school: &'static str,
    pub ritual: bool,
    pub concentration: bool,
    /// Castable with a slot now: every cantrip, and the level 1+ spells prepared.
    pub prepared: bool,
    pub casting_time: &'static str,
    pub range: &'static str,
    pub components: &'static str,
    pub duration: &'static str,
    pub summary: &'static str,
    pub page: u16,
}

/// The spell slots of one level a hero has at most.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct SlotCount {
    pub level: u8,
    pub count: u8,
}

/// A spellcaster's numbers and spells (Spellcasting, SRD p. 106; Wizard, pp. 77-78).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Spellcasting {
    /// The sheet's short key of the spellcasting ability (`int`).
    pub ability: &'static str,
    /// 8 + the spellcasting ability modifier + the Proficiency Bonus.
    pub save_dc: i16,
    /// The spellcasting ability modifier + the Proficiency Bonus.
    pub attack_bonus: i16,
    /// How many level 1+ spells may be prepared at once.
    pub prepared_max: u8,
    pub slots: Vec<SlotCount>,
    /// The cantrips, then the spellbook's spells, in the order picked.
    pub spells: Vec<SheetSpell>,
}

impl Spellcasting {
    /// The spell `id` the hero knows, if it knows it.
    pub fn known(&self, id: &str) -> Option<&SheetSpell> {
        self.spells.iter().find(|spell| spell.id == id)
    }
}

/// One part of the build that is designed but not played: "Not in play yet".
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DesignMark {
    /// What is not in play, in the source's words (`Rage`, `Aasimar`).
    pub subject: String,
    pub reason: DesignOnlyReason,
    /// One plain sentence for the player.
    pub note: String,
}

/// A hero's derived sheet. Field names are the gauntlet sheet file's; `abilities` and
/// `proficient_skills` are the typed values the host's rules adapter reads and are not written.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HeroSheet {
    pub schema: u32,
    pub id: String,
    pub name: String,
    pub rules_source: &'static str,
    pub species: String,
    pub class: String,
    pub level: u8,
    pub background: String,
    pub alignment: String,
    pub ability_scores: AbilityTable<u8>,
    pub ability_score_note: String,
    pub ability_modifiers: AbilityTable<i8>,
    pub proficiency_bonus: u8,
    pub saving_throw_proficiencies: Vec<&'static str>,
    pub saving_throws: AbilityTable<i8>,
    /// Each proficient skill (the roll vocabulary's key) and where the proficiency comes from.
    pub skill_proficiencies: BTreeMap<&'static str, String>,
    pub expertise: Vec<&'static str>,
    /// The check modifier of every skill.
    pub skills: BTreeMap<&'static str, i8>,
    pub tool_proficiencies: Vec<String>,
    pub languages: Vec<String>,
    pub origin_feats: Vec<String>,
    /// Species traits in play, as sentences.
    pub species_traits: BTreeMap<String, String>,
    /// Class features in play, as sentences.
    pub class_features: BTreeMap<String, String>,
    /// How each feature of `class_features` is used, by the same key.
    pub feature_status: BTreeMap<String, FeatureStatus>,
    /// The weapons whose mastery property the hero uses (Weapon Mastery).
    pub weapon_masteries: Vec<SheetMastery>,
    /// The hero's spellcasting; none for a hero who casts no spells in play.
    pub spellcasting: Option<Spellcasting>,
    pub hit_die: String,
    pub hit_points: HitPoints,
    pub armor_class: Explained<i16>,
    /// The armour the hero wears that gives Disadvantage on Dexterity (Stealth) checks, by title.
    pub stealth_disadvantage: Option<String>,
    /// `None` when the species gives no Speed in play (a species with no admitted source).
    pub speed_feet: Option<u16>,
    pub size: Option<&'static str>,
    pub darkvision_feet: Option<u16>,
    pub initiative_modifier: Explained<i16>,
    pub passive_perception: i16,
    pub attacks: Vec<SheetAttack>,
    pub healing: Vec<SheetHealing>,
    pub equipment: Vec<String>,
    pub coins_gp: u32,
    pub critical_range: Vec<u8>,
    /// Everything designed that the rules cannot play yet.
    pub not_in_play: Vec<DesignMark>,
    #[serde(skip)]
    pub abilities: [u8; 6],
    /// The Hit Point Die, typed (`hit_die` is its key).
    #[serde(skip)]
    pub hit_die_size: DieSize,
    #[serde(skip)]
    pub proficient_skills: BTreeSet<Skill>,
}

/// One item of a hero's starting equipment, for the hero's inventory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StartingItem {
    /// The inventory id (`chain-mail`, `dungeoneers-pack`).
    pub id: String,
    pub name: String,
    pub count: u16,
    /// The SRD price of one, in copper, when the price table lists it.
    pub price_cp: Option<u64>,
    /// Using it does not use it up (armour, a weapon, a tool, a pack); ammunition and paper are
    /// used up.
    pub kept: bool,
}

/// A hero's starting equipment: its items, its gold, and the gear its ready pose shows.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StartingKit {
    pub items: Vec<StartingItem>,
    pub gold_gp: u32,
    pub armor: Option<String>,
    pub shield: bool,
    pub main_weapon: Option<String>,
}

impl StartingKit {
    /// The gear in words for a picture: "wearing Chain Mail, with a Shield, wielding a Greatsword".
    pub fn gear_words(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(armor) = &self.armor {
            parts.push(format!("wearing {armor}"));
        }
        if self.shield {
            parts.push("with a Shield".to_owned());
        }
        if let Some(weapon) = &self.main_weapon {
            parts.push(format!("wielding a {weapon}"));
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }
}

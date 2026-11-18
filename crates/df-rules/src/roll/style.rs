//! What a hero's Fighting Style feat does to a roll (SRD 5.2.1 pp. 87-88): the one rules path for
//! Archery's bonus and Great Weapon Fighting's floor, for the tactical rules, the hero sheet and
//! any roll the host makes (`RollRequest::resolve_styled`).
//!
//! Archery: +2 to attack rolls made with Ranged weapons. Great Weapon Fighting: on the damage dice
//! of an attack with a Melee weapon held in two hands (a Two-Handed or Versatile weapon), any 1 or
//! 2 counts as a 3. Defense changes Armor Class, not a roll (`hero` derives it); Two-Weapon
//! Fighting adds the ability modifier the damage roll already carries. Which weapon an attack uses
//! is the caller's: a style is asked for only where it applies, and refused on a roll it cannot
//! touch.
use serde::{Deserialize, Serialize};

use crate::listed::listed_enum;

/// Archery's bonus to an attack roll with a Ranged weapon (SRD p. 87).
pub const ARCHERY_ATTACK_BONUS: i16 = 2;
/// The least a damage die counts for under Great Weapon Fighting (SRD p. 88).
pub const GREAT_WEAPON_FIGHTING_FLOOR: u16 = 3;

listed_enum! {
    keyed
    /// A Fighting Style that touches an attack's rolls.
    #[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum AttackStyle {
        /// +2 to the attack roll of a Ranged weapon.
        Archery,
        /// A 1 or 2 on a damage die of a two-handed Melee weapon counts as a 3.
        GreatWeaponFighting,
    }
}

impl AttackStyle {
    pub fn key(self) -> &'static str {
        match self {
            Self::Archery => "archery",
            Self::GreatWeaponFighting => "great_weapon_fighting",
        }
    }
}

/// What styles apply to one roll: the attack roll's bonus (Archery) and the damage dice's floor
/// (Great Weapon Fighting). The default applies none.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RollStyle {
    /// Archery's +2: only on the attack roll of a ranged attack.
    #[serde(default)]
    pub archery: bool,
    /// Great Weapon Fighting's floor: only on damage dice.
    #[serde(default)]
    pub great_weapon_fighting: bool,
}

impl RollStyle {
    /// The style a sheet's attack names, for its attack roll and its damage roll alike (each roll
    /// takes the part that touches it).
    pub fn of(style: Option<AttackStyle>) -> Self {
        Self {
            archery: style == Some(AttackStyle::Archery),
            great_weapon_fighting: style == Some(AttackStyle::GreatWeaponFighting),
        }
    }
}

/// Archery's bonus to an attack with a weapon, when the attacker has the style and the weapon is
/// a Ranged weapon; 0 otherwise.
pub fn archery_bonus(archery: bool, ranged_weapon: bool) -> i16 {
    if archery && ranged_weapon {
        ARCHERY_ATTACK_BONUS
    } else {
        0
    }
}

/// A damage die's face as Great Weapon Fighting counts it.
pub fn great_weapon_face(face: u16) -> u16 {
    face.max(GREAT_WEAPON_FIGHTING_FLOOR)
}

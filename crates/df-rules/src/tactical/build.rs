use super::{Cell, CombatantSpec, CreatureKind, RuleError, Side, Size, Weapon};
use crate::local_journey::{self, LegalCharacter};

/// Creation-group/array data for the follow-up loader; current pack.rs cannot yet accept it.
pub const ARCHER_CREATION_JSON: &str =
    include_str!("../../../../content/hero-builds/dwarf-archer.json");
/// Selected level-one Dwarf Fighter/Soldier, Fighter B + Soldier B; source pp19–22,47–48,83–84.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArcherBuild {
    character: LegalCharacter,
}
impl ArcherBuild {
    pub fn character(&self) -> &LegalCharacter {
        &self.character
    }
    /// Derived combat profile, never client-supplied bonuses or AC.
    pub fn combatant(&self, id: u32, cell: Cell) -> CombatantSpec {
        CombatantSpec {
            id,
            side: Side::Heroes,
            cell,
            size: Size::Medium,
            kind: CreatureKind::Hero,
            abilities: self.character.abilities,
            armor_class: 15,
            hit_points: 13,
            speed_ft: 30,
            weapons: vec![Weapon::Longbow, Weapon::Scimitar, Weapon::Shortsword],
            masteries: vec![Weapon::Longbow, Weapon::Scimitar, Weapon::Shortsword],
            arrows: 20,
            bolts: 0,
            archery: true,
            savage_attacker: true,
            second_wind_uses: 2,
            shield: false,
        }
    }
}
/// Same submitted choices/pick-list contract as the existing journey. The new fixed
/// equipment/style/array IDs are admitted together; mixing packages is rejected.
pub fn validate_archer(name: &str, choices: &[(&str, &str)]) -> Result<ArcherBuild, RuleError> {
    if choices.len() != local_journey::BUILD_GROUPS.len() {
        return Err(RuleError::InvalidBuild);
    }
    let mut mapped = Vec::with_capacity(choices.len());
    for &(group, value) in choices {
        let replacement = match group {
            "array" if value == "dexterous" => "stalwart",
            "equipment" if value == "fighter-b-soldier-b" => "fighter-a-soldier-a",
            "style-masteries" if value == "archery-longbow-scimitar-shortsword" => {
                "defense-greatsword-flail-javelin"
            }
            "array" | "equipment" | "style-masteries" => return Err(RuleError::InvalidBuild),
            _ => value,
        };
        mapped.push((group, replacement));
    }
    let mut character =
        local_journey::validate_character(&local_journey::CreationRules::srd_521(), name, &mapped)
            .map_err(|_| RuleError::InvalidBuild)?;
    character.abilities = [13, 17, 15, 10, 12, 8];
    character.armor_class = 15;
    character.choices = choices
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    Ok(ArcherBuild { character })
}

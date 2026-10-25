use super::{BattleMap, Cell, RuleError, Weapon};
use crate::local_journey::{ChallengeRating, HeroSpecies, SpeciesCombatTraits, SpeciesSize};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Heroes,
    Enemies,
}
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
pub enum Size {
    Small,
    Medium,
    Large,
    Huge,
}
/// Source stat blocks; departures in the old adjacent journey are not imported here.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreatureKind {
    Hero,
    Bandit,
    Cultist,
    Guard,
    Goblin,
    Kobold,
    Skeleton,
    Zombie,
    Wolf,
    GiantRat,
    GiantWeasel,
    Mastiff,
    Scout,
    Commoner,
    Ogre,
    Owlbear,
    Wight,
    HillGiant,
    Troll,
}

/// Caller-owned combatant data. Construction validates the bounded supported subset.
#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CombatantSpec {
    pub id: u32,
    pub side: Side,
    pub cell: Cell,
    pub size: Size,
    pub kind: CreatureKind,
    #[serde(default)]
    pub species: Option<HeroSpecies>,
    #[serde(default)]
    pub species_options: crate::species::SpeciesOptions,
    pub abilities: [u8; 6],
    pub armor_class: u16,
    pub hit_points: u16,
    pub speed_ft: u16,
    pub weapons: Vec<Weapon>,
    pub masteries: Vec<Weapon>,
    pub arrows: u16,
    pub bolts: u16,
    pub archery: bool,
    pub savage_attacker: bool,
    pub second_wind_uses: u8,
    #[serde(default)]
    pub healers_kit_uses: u8,
    #[serde(default)]
    pub relentless_endurance_available: bool,
    /// An equipped shield bars a hero's two-handed attacks; creature attacks follow
    /// their stat blocks.
    pub shield: bool,
}
impl CreatureKind {
    /// The compiled SRD 5.2.1 creature id, none for a hero.
    pub fn srd_id(self) -> Option<&'static str> {
        Some(match self {
            Self::Hero => return None,
            Self::Bandit => "bandit",
            Self::Cultist => "cultist",
            Self::Guard => "guard",
            Self::Goblin => "goblin-warrior",
            Self::Kobold => "kobold-warrior",
            Self::Skeleton => "skeleton",
            Self::Zombie => "zombie",
            Self::Wolf => "wolf",
            Self::GiantRat => "giant-rat",
            Self::GiantWeasel => "giant-weasel",
            Self::Mastiff => "mastiff",
            Self::Scout => "scout",
            Self::Commoner => "commoner",
            Self::Ogre => "ogre",
            Self::Owlbear => "owlbear",
            Self::Wight => "wight",
            Self::HillGiant => "hill-giant",
            Self::Troll => "troll",
        })
    }

    /// SRD 5.2.1 pp. 313, 353, 355: the Owlbear climbs 40 ft.; Rat/Weasel 30 ft.
    /// The current battle grid is flat, so callers must use this only when a
    /// vertical movement model is admitted.
    pub fn climb_speed_ft(self) -> Option<u16> {
        match self {
            Self::GiantRat | Self::GiantWeasel => Some(30),
            Self::Owlbear => Some(40),
            _ => None,
        }
    }

    /// SRD 5.2.1 pp. 290, 302, 325, 343, 353, 355, 357, 364: these creature
    /// stat blocks have Darkvision 60 ft.; hero sight is selected by species.
    pub fn darkvision_ft(self) -> Option<u16> {
        match self {
            Self::Hero => None,
            Self::Goblin
            | Self::Kobold
            | Self::Skeleton
            | Self::Zombie
            | Self::Wolf
            | Self::GiantRat
            | Self::GiantWeasel
            | Self::Mastiff
            | Self::Ogre
            | Self::Owlbear
            | Self::Wight
            | Self::Troll => Some(60),
            _ => None,
        }
    }
}
impl CombatantSpec {
    /// The cell is the north-west corner of the source creature's space (SRD p. 14).
    pub fn space_cells(&self, origin: Cell) -> Vec<Cell> {
        let width = self.space_width_cells();
        (0..width)
            .flat_map(|x| {
                (0..width)
                    .map(move |y| Cell(origin.0.saturating_add(x), origin.1.saturating_add(y)))
            })
            .collect()
    }
    fn space_width_cells(&self) -> u16 {
        match self.size {
            Size::Small | Size::Medium => 1,
            Size::Large => 2,
            Size::Huge => 3,
        }
    }
    pub fn distance_from(&self, origin: Cell, other: &CombatantSpec, target: Cell) -> u32 {
        let width = self.space_width_cells();
        let other_width = other.space_width_cells();
        let col = origin
            .0
            .saturating_sub(target.0.saturating_add(other_width - 1))
            .max(target.0.saturating_sub(origin.0.saturating_add(width - 1)));
        let row = origin
            .1
            .saturating_sub(target.1.saturating_add(other_width - 1))
            .max(target.1.saturating_sub(origin.1.saturating_add(width - 1)));
        u32::from(col.max(row)) * 5
    }
    pub fn initiative_modifier(&self) -> i32 {
        match self.kind {
            CreatureKind::Wight => 4,
            CreatureKind::HillGiant => 2,
            _ => crate::local_journey::ability_modifier(self.abilities[1]),
        }
    }
    pub fn multiattack_count(&self) -> u8 {
        match self.kind {
            CreatureKind::Scout
            | CreatureKind::Owlbear
            | CreatureKind::Wight
            | CreatureKind::HillGiant => 2,
            CreatureKind::Troll => 3,
            _ => 1,
        }
    }
    pub fn melee_reach_ft(&self, weapon: Weapon) -> u32 {
        if matches!(
            (self.kind, weapon),
            (CreatureKind::Troll, Weapon::Rend) | (CreatureKind::HillGiant, Weapon::TreeClub)
        ) {
            10
        } else {
            5
        }
    }

    /// The supported fixed-HP Fighter profiles identify their level without changing legacy
    /// caller construction or snapshot schemas. Source validation checks every derived field.
    pub fn fighter_level(&self) -> Option<crate::local_journey::FighterProgression> {
        if self.kind != CreatureKind::Hero {
            return None;
        }
        let species = self.species?;
        (1..=5)
            .filter_map(|level| crate::local_journey::FighterProgression::new(level).ok())
            .find(|progression| {
                progression.hit_points(self.abilities[2], species)
                    + if self.species_options.tough {
                        2 * u16::from(progression.level())
                    } else {
                        0
                    }
                    == self.hit_points
            })
    }
    /// Upgrade an admitted level-one profile, retaining equipment and species.
    pub fn at_fighter_level(mut self, level: u8) -> Result<Self, RuleError> {
        validate_source_profile(&self)?;
        if self
            .fighter_level()
            .is_none_or(|progression| progression.level() != 1)
        {
            return Err(RuleError::InvalidBuild);
        }
        let progression = crate::local_journey::FighterProgression::new(level)
            .map_err(|_| RuleError::InvalidBuild)?;
        if level >= 4 {
            self.abilities[if self.archery { 1 } else { 0 }] += 2;
            self.masteries.push(if self.archery {
                Weapon::Javelin
            } else {
                Weapon::Spear
            });
            if self.archery {
                self.armor_class += 1;
            }
        }
        self.hit_points = progression.hit_points(
            self.abilities[2],
            self.species.ok_or(RuleError::InvalidBuild)?,
        );
        self.second_wind_uses = progression.second_wind_maximum();
        if self.species_options.tough {
            self.hit_points += 2 * u16::from(level);
        }
        validate_source_profile(&self)?;
        Ok(self)
    }
    pub fn species_traits(&self) -> Option<SpeciesCombatTraits> {
        self.species
            .map(|species| self.species_options.combat_traits(species))
    }

    pub fn darkvision_ft(&self) -> Option<u16> {
        if self.kind == CreatureKind::Hero {
            self.species_traits()?.darkvision_ft
        } else {
            self.kind.darkvision_ft()
        }
    }

    /// Source-derived hero attack and damage modifiers for a supported carried weapon.
    pub fn hero_attack_modifiers(
        &self,
        weapon: Weapon,
        ranged: bool,
        finesse_dexterity: bool,
    ) -> Result<(i16, i16), RuleError> {
        if self.kind != CreatureKind::Hero
            || (weapon != Weapon::UnarmedStrike && !self.weapons.contains(&weapon))
            || (finesse_dexterity && !weapon.finesse())
        {
            return Err(RuleError::InvalidWeapon);
        }
        let ability = if (ranged && weapon.ranged_weapon()) || finesse_dexterity {
            1
        } else {
            0
        };
        let damage = crate::local_journey::ability_modifier(self.abilities[ability]) as i16;
        Ok((
            damage
                + i16::from(
                    self.fighter_level()
                        .ok_or(RuleError::InvalidBuild)?
                        .proficiency_bonus(),
                )
                + if ranged && weapon.ranged_weapon() && self.archery {
                    2
                } else {
                    0
                },
            damage,
        ))
    }
    /// Revalidate the existing journey creation receipt and expose all carried weapons.
    pub fn melee_hero(
        character: &crate::local_journey::LegalCharacter,
        id: u32,
        cell: Cell,
    ) -> Result<Self, RuleError> {
        let choices: Vec<_> = character
            .choices
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        let checked = crate::local_journey::validate_character(
            &crate::local_journey::CreationRules::srd_521(),
            &character.name,
            &choices,
        )
        .map_err(|_| RuleError::InvalidBuild)?;
        if &checked != character {
            return Err(RuleError::InvalidBuild);
        }
        let species = checked
            .choices
            .iter()
            .find(|(key, _)| key == "species")
            .and_then(|(_, value)| HeroSpecies::from_choice(value))
            .ok_or(RuleError::InvalidBuild)?;
        let species_options = crate::species::SpeciesOptions::from_choices(species, &choices)
            .map_err(|_| RuleError::InvalidBuild)?;
        let traits = species_options.combat_traits(species);
        Ok(Self {
            id,
            side: Side::Heroes,
            cell,
            size: match traits.size {
                SpeciesSize::Small => Size::Small,
                SpeciesSize::Medium => Size::Medium,
            },
            kind: CreatureKind::Hero,
            species: Some(species),
            species_options,
            abilities: checked.abilities,
            armor_class: 17,
            hit_points: u16::try_from(checked.hit_points).map_err(|_| RuleError::InvalidBuild)?,
            speed_ft: traits.speed_ft,
            weapons: vec![
                Weapon::Greatsword,
                Weapon::Flail,
                Weapon::Javelin,
                Weapon::Spear,
                Weapon::Shortbow,
            ],
            masteries: vec![Weapon::Greatsword, Weapon::Flail, Weapon::Javelin],
            arrows: 20,
            bolts: 0,
            archery: false,
            savage_attacker: true,
            second_wind_uses: 2,
            healers_kit_uses: 10,
            relentless_endurance_available: traits.relentless_endurance,
            shield: false,
        })
    }
    /// Source-pinned SRD 5.2.1 stat block; excluded features are catalogued explicitly.
    /// Night/normal sight only; sunlight/hidden/alternate movement modes are outside this slice.
    pub fn creature(id: u32, srd_id: &str, cell: Cell, ammunition: u16) -> Result<Self, RuleError> {
        // Commoner is an authored scene-NPC profile, intentionally kept out of
        // the generated antagonist table. It is legal only when a caller has
        // already admitted a typed scene combat binding.
        let source = if srd_id == "commoner" {
            &crate::SCENE_COMMONER
        } else {
            crate::SRD_521_CREATURES
                .iter()
                .find(|creature| creature.id == srd_id)
                .ok_or(RuleError::UnsupportedCreature)?
        };
        let (kind, size, speed, weapons, shield) = match srd_id {
            "bandit" => (
                CreatureKind::Bandit,
                Size::Medium,
                30,
                vec![Weapon::Scimitar, Weapon::LightCrossbow],
                false,
            ),
            "cultist" => (
                CreatureKind::Cultist,
                Size::Medium,
                30,
                vec![Weapon::Sickle],
                false,
            ),
            "guard" => (
                CreatureKind::Guard,
                Size::Medium,
                30,
                vec![Weapon::Spear],
                true,
            ),
            "goblin-warrior" => (
                CreatureKind::Goblin,
                Size::Small,
                30,
                vec![Weapon::Scimitar, Weapon::Shortbow],
                true,
            ),
            "kobold-warrior" => (
                CreatureKind::Kobold,
                Size::Small,
                30,
                vec![Weapon::Dagger],
                false,
            ),
            "skeleton" => (
                CreatureKind::Skeleton,
                Size::Medium,
                30,
                vec![Weapon::Shortsword, Weapon::Shortbow],
                false,
            ),
            "zombie" => (
                CreatureKind::Zombie,
                Size::Medium,
                20,
                vec![Weapon::Slam],
                false,
            ),
            "wolf" => (
                CreatureKind::Wolf,
                Size::Medium,
                40,
                vec![Weapon::Bite],
                false,
            ),
            "giant-rat" => (
                CreatureKind::GiantRat,
                Size::Small,
                30,
                vec![Weapon::Bite],
                false,
            ),
            "giant-weasel" => (
                CreatureKind::GiantWeasel,
                Size::Medium,
                40,
                vec![Weapon::Bite],
                false,
            ),
            "mastiff" => (
                CreatureKind::Mastiff,
                Size::Medium,
                40,
                vec![Weapon::Bite],
                false,
            ),
            "scout" => (
                CreatureKind::Scout,
                Size::Medium,
                30,
                vec![Weapon::Shortsword, Weapon::Longbow],
                false,
            ),
            "commoner" => (
                CreatureKind::Commoner,
                Size::Medium,
                30,
                vec![Weapon::Club],
                false,
            ),
            "ogre" => (
                CreatureKind::Ogre,
                Size::Large,
                40,
                vec![Weapon::Greatclub, Weapon::Javelin],
                false,
            ),
            "owlbear" => (
                CreatureKind::Owlbear,
                Size::Large,
                40,
                vec![Weapon::Rend],
                false,
            ),
            "wight" => (
                CreatureKind::Wight,
                Size::Medium,
                30,
                vec![Weapon::NecroticSword, Weapon::NecroticBow],
                false,
            ),
            "hill-giant" => (
                CreatureKind::HillGiant,
                Size::Huge,
                40,
                vec![Weapon::TreeClub],
                false,
            ),
            "troll" => (
                CreatureKind::Troll,
                Size::Large,
                30,
                vec![Weapon::Rend],
                false,
            ),
            _ => return Err(RuleError::UnsupportedCreature),
        };
        Ok(Self {
            id,
            side: Side::Enemies,
            cell,
            size,
            kind,
            species: None,
            species_options: Default::default(),
            abilities: source.rules.abilities,
            armor_class: u16::from(source.rules.armor_class),
            hit_points: u16::from(source.rules.hit_points),
            speed_ft: speed,
            weapons,
            masteries: vec![],
            arrows: ammunition,
            bolts: ammunition,
            archery: false,
            savage_attacker: false,
            second_wind_uses: 0,
            healers_kit_uses: 0,
            relentless_endurance_available: false,
            shield,
        })
    }
}
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    pub action_used: bool,
    pub bonus_action_used: bool,
    pub reaction_used: bool,
    pub object_interaction_used: bool,
    pub movement_spent_ft: u16,
    pub dash: bool,
    pub disengaged: bool,
    pub dodging: bool,
}
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Health {
    pub hit_points: u16,
    pub maximum: u16,
    pub prone: bool,
    pub unconscious: bool,
    pub knocked_out: bool,
    pub dead: bool,
    pub stable: bool,
    pub death_successes: u8,
    pub death_failures: u8,
}
impl Health {
    pub fn incapacitated(self) -> bool {
        self.unconscious || self.dead
    }
}
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Effect {
    pub origin: u32,
    pub expires_turn: u32,
}
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct HelpEffect {
    pub helper: u32,
    pub helper_turn_count: u32,
    pub ally: u32,
    pub target: u32,
}
#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Combatant {
    pub(crate) spec: CombatantSpec,
    pub(crate) health: Health,
    pub(crate) budget: Budget,
    pub(crate) turn_count: u32,
    pub(crate) slow: Vec<Effect>,
    #[serde(default)]
    pub(crate) frost_chill: Vec<Effect>,
    #[serde(default)]
    pub(crate) species_uses_spent: u8,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) regeneration_inhibited: bool,
    pub(crate) sap: Vec<Effect>,
    pub(crate) vex: Vec<(u32, u32)>,
    pub(crate) light_first: Option<Weapon>,
    pub(crate) light_used: bool,
    pub(crate) savage_used: bool,
    pub(crate) extra_attacks: u8,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) action_surge_spent: bool,
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub(crate) additional_actions: u8,
    #[serde(default, skip_serializing_if = "is_zero_u16")]
    pub(crate) feature_movement_ft: u16,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) additional_dash: bool,
    pub(crate) ammunition_used: u16,
    pub(crate) recovered: bool,
    pub(crate) bolts_used: u16,
    pub(crate) weapon_counts: BTreeMap<Weapon, u16>,
    pub(crate) dropped: Vec<(Weapon, Cell)>,
    pub(crate) held: Vec<Weapon>,
    pub(crate) shield_equipped: bool,
}
fn is_zero_u8(value: &u8) -> bool {
    *value == 0
}
fn is_zero_u16(value: &u16) -> bool {
    *value == 0
}
impl Combatant {
    /// Dragonborn Breath Weapon / Goliath ancestry: PB uses per Long Rest (SRD pp.84-86).
    pub fn species_uses_remaining(&self) -> u8 {
        if matches!(
            self.spec.species,
            Some(HeroSpecies::Dragonborn | HeroSpecies::Goliath)
        ) {
            self.spec
                .fighter_level()
                .map_or(0, |level| level.proficiency_bonus())
                .saturating_sub(self.species_uses_spent)
        } else {
            0
        }
    }
    pub fn action_surge_available(&self) -> bool {
        self.spec
            .fighter_level()
            .is_some_and(|progression| progression.level() >= 2)
            && !self.action_surge_spent
    }
    pub fn additional_actions(&self) -> u8 {
        self.additional_actions
    }
    pub fn feature_movement_ft(&self) -> u16 {
        self.feature_movement_ft
    }
    pub(crate) fn spend_action(&mut self) -> Result<(), RuleError> {
        if !self.budget.action_used {
            self.budget.action_used = true;
        } else if self.additional_actions > 0 {
            self.additional_actions -= 1;
        } else {
            return Err(RuleError::ActionSpent);
        }
        Ok(())
    }
    pub fn id(&self) -> u32 {
        self.spec.id
    }
    pub fn side(&self) -> Side {
        self.spec.side
    }
    pub fn distance_to(&self, other: &Self) -> u32 {
        self.spec
            .distance_from(self.cell(), &other.spec, other.cell())
    }
    pub fn cell(&self) -> Cell {
        self.spec.cell
    }
    pub fn spec(&self) -> &CombatantSpec {
        &self.spec
    }
    /// Public conditions, without equipment or private combat statistics.
    pub fn conditions(&self) -> Vec<&'static str> {
        let mut result = Vec::new();
        for (present, name) in [
            (self.health.prone, "prone"),
            (self.health.unconscious, "unconscious"),
            (self.health.dead, "dead"),
            (self.health.stable, "stable"),
            (self.budget.dodging, "dodging"),
            (self.budget.disengaged, "disengaged"),
            (!self.slow.is_empty(), "slowed"),
            (!self.sap.is_empty(), "sapped"),
        ] {
            if present {
                result.push(name);
            }
        }
        result
    }
    pub fn health(&self) -> Health {
        self.health
    }
    pub fn budget(&self) -> Budget {
        self.budget
    }
    pub fn arrows(&self) -> u16 {
        self.spec.arrows
    }
    pub fn ammunition_used(&self) -> u16 {
        self.ammunition_used
    }
    pub fn speed_ft(&self) -> u16 {
        if self.health.incapacitated() {
            return 0;
        }
        self.spec
            .speed_ft
            .saturating_sub(if self.slow.is_empty() { 0 } else { 10 })
            .saturating_sub(if self.frost_chill.is_empty() { 0 } else { 10 })
    }
    pub fn movement_remaining_ft(&self) -> u16 {
        (self.speed_ft() * (1 + u16::from(self.budget.dash) + u16::from(self.additional_dash)))
            .saturating_sub(self.budget.movement_spent_ft)
            + self.feature_movement_ft
    }
    pub fn extra_attacks_available(&self) -> bool {
        self.extra_attacks > 0
    }
    pub fn bolts(&self) -> u16 {
        self.spec.bolts
    }
    pub fn dropped_weapons(&self) -> &[(Weapon, Cell)] {
        &self.dropped
    }
    pub fn weapon_count(&self, weapon: Weapon) -> u16 {
        self.weapon_counts.get(&weapon).copied().unwrap_or(0)
    }
    pub fn held_weapons(&self) -> &[Weapon] {
        &self.held
    }
    pub fn shield_equipped(&self) -> bool {
        self.shield_equipped
    }
}
#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingMove {
    pub actor: u32,
    pub path: Vec<Cell>,
    pub next: usize,
    pub eligible: Vec<u32>,
}
/// A detached pure battle; only resolve returns a successor. No durable authority is implied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Battle {
    pub(crate) map: BattleMap,
    pub(crate) actors: Vec<Combatant>,
    pub(crate) order: Vec<u32>,
    pub(crate) initiatives: BTreeMap<u32, i32>,
    pub(crate) active: usize,
    pub(crate) round: u32,
    pub(crate) pending: Option<PendingMove>,
    pub(crate) species_damage: Vec<super::species::SpeciesDamage>,
    pub(crate) helped_attacks: Vec<HelpEffect>,
    pub(crate) needs_death_save: bool,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreatureExperience {
    pub combatant: u32,
    pub challenge_rating: ChallengeRating,
    pub experience_points: u32,
}

#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
pub struct EncounterExperience {
    pub creatures: Vec<CreatureExperience>,
    pub total_xp: u32,
}
impl Battle {
    /// Carry admitted canonical resources into a fresh encounter without restoring them.
    /// Source profiles retain maximum HP and starting equipment; this only spends resources.
    pub fn with_hero_resources(
        mut self,
        id: u32,
        hit_points: u16,
        arrows: u16,
        second_wind: u8,
    ) -> Result<Self, RuleError> {
        let actor = self.actor_mut(id)?;
        if actor.spec.kind != CreatureKind::Hero
            || hit_points == 0
            || hit_points > actor.health.maximum
            || arrows > actor.spec.arrows
            || second_wind > actor.spec.second_wind_uses
        {
            return Err(RuleError::InvalidCombatant);
        }
        actor.health.hit_points = hit_points;
        actor.spec.arrows = arrows;
        actor.spec.second_wind_uses = second_wind;
        Ok(self)
    }
    /// Carry a Fighter's once-per-rest Action Surge across encounters.
    pub fn with_hero_species_uses(mut self, id: u32, remaining: u8) -> Result<Self, RuleError> {
        let actor = self.actor_mut(id)?;
        if !matches!(
            actor.spec.species,
            Some(HeroSpecies::Dragonborn | HeroSpecies::Goliath)
        ) {
            return Err(RuleError::InvalidBuild);
        }
        let maximum = actor
            .spec
            .fighter_level()
            .ok_or(RuleError::InvalidBuild)?
            .proficiency_bonus();
        actor.species_uses_spent = maximum
            .checked_sub(remaining)
            .ok_or(RuleError::InvalidCombatant)?;
        Ok(self)
    }
    pub fn with_hero_action_surge(mut self, id: u32, available: bool) -> Result<Self, RuleError> {
        let actor = self.actor_mut(id)?;
        if actor
            .spec
            .fighter_level()
            .is_none_or(|progression| progression.level() < 2)
        {
            return Err(RuleError::InvalidBuild);
        }
        actor.action_surge_spent = !available;
        Ok(self)
    }
    /// Carry remaining Healer's Kit uses and Orc Relentless Endurance into a new encounter.
    pub fn with_hero_feature_resources(
        mut self,
        id: u32,
        healers_kit_uses: u8,
        relentless_endurance_available: bool,
    ) -> Result<Self, RuleError> {
        let actor = self.actor_mut(id)?;
        let can_use_relentless_endurance = actor
            .spec
            .species
            .is_some_and(|species| species.combat_traits().relentless_endurance);
        if actor.spec.kind != CreatureKind::Hero
            || healers_kit_uses > 10
            || (relentless_endurance_available && !can_use_relentless_endurance)
        {
            return Err(RuleError::InvalidCombatant);
        }
        actor.spec.healers_kit_uses = healers_kit_uses;
        actor.spec.relentless_endurance_available = relentless_endurance_available;
        Ok(self)
    }
    /// Carry the admitted canonical inventory into a fresh hero encounter.
    pub fn with_hero_weapons(
        mut self,
        id: u32,
        quantities: &[(Weapon, u16)],
    ) -> Result<Self, RuleError> {
        let actor = self.actor_mut(id)?;
        if actor.spec.kind != CreatureKind::Hero
            || quantities.len() != actor.weapon_counts.len()
            || quantities
                .iter()
                .map(|(weapon, _)| *weapon)
                .collect::<BTreeSet<_>>()
                .len()
                != quantities.len()
            || quantities.iter().any(|(weapon, count)| {
                actor
                    .weapon_counts
                    .get(weapon)
                    .is_none_or(|maximum| count > maximum)
            })
        {
            return Err(RuleError::InvalidCombatant);
        }
        actor.weapon_counts = quantities.iter().copied().collect();
        actor.held.retain(|weapon| {
            actor
                .weapon_counts
                .get(weapon)
                .is_some_and(|count| *count > 0)
        });
        Ok(self)
    }
    pub fn map(&self) -> &BattleMap {
        &self.map
    }
    pub fn combatants(&self) -> &[Combatant] {
        &self.actors
    }
    pub fn actor(&self, id: u32) -> Result<&Combatant, RuleError> {
        self.actors
            .iter()
            .find(|a| a.id() == id)
            .ok_or(RuleError::UnknownActor)
    }
    pub fn active_actor(&self) -> Option<u32> {
        self.order.get(self.active).copied()
    }
    /// SRD 5.2.1 p. 13: the GM rolls once for an entire group of identical creatures.
    pub fn initiative_policy(&self) -> &'static str {
        "identical-monster-groups"
    }
    pub(crate) fn overlaps(&self, actor: &Combatant, origin: Cell, other: &Combatant) -> bool {
        let space = actor.spec.space_cells(origin);
        other
            .spec
            .space_cells(other.cell())
            .iter()
            .any(|cell| space.contains(cell))
    }
    pub fn initiative_order(&self) -> &[u32] {
        &self.order
    }
    pub fn round(&self) -> u32 {
        self.round
    }
    pub fn reaction_offers(&self) -> &[u32] {
        self.pending.as_ref().map_or(&[], |p| p.eligible.as_slice())
    }
    pub fn needs_death_save(&self) -> bool {
        self.needs_death_save
    }
    pub fn winner(&self) -> Option<Side> {
        let heroes = self
            .actors
            .iter()
            .any(|a| a.side() == Side::Heroes && !a.health.incapacitated());
        let enemies = self.actors.iter().any(|a| {
            a.side() == Side::Enemies
                && (!a.health.incapacitated()
                    || (a.spec.kind == CreatureKind::Troll
                        && !a.health.dead
                        && !a.health.knocked_out))
        });
        match (heroes, enemies) {
            (true, false) => Some(Side::Heroes),
            (false, true) => Some(Side::Enemies),
            _ => None,
        }
    }
    /// Award source-derived monster XP only after the heroes have won the encounter.
    /// SRD 5.2.1 p. 256: XP is awarded "for defeating the monster in combat or otherwise neutralizing it."
    pub fn experience_award(&self) -> Result<Option<EncounterExperience>, RuleError> {
        if self.winner() != Some(Side::Heroes) {
            return Ok(None);
        }
        let mut creatures = Vec::new();
        let mut total_xp = 0_u32;
        for actor in self
            .actors
            .iter()
            .filter(|actor| actor.side() == Side::Enemies && actor.health.incapacitated())
        {
            let id = actor
                .spec
                .kind
                .srd_id()
                .ok_or(RuleError::UnsupportedCreature)?;
            let creature = crate::SRD_521_CREATURES
                .iter()
                .find(|creature| creature.id == id)
                .ok_or(RuleError::UnsupportedCreature)?;
            let experience_points = creature
                .experience_points()
                .ok_or(RuleError::UnpricedExperience)?;
            total_xp = total_xp
                .checked_add(experience_points)
                .ok_or(RuleError::Capacity)?;
            creatures.push(CreatureExperience {
                combatant: actor.id(),
                challenge_rating: creature.challenge_rating,
                experience_points,
            });
        }
        Ok(Some(EncounterExperience {
            creatures,
            total_xp,
        }))
    }
    pub fn finished(&self) -> bool {
        ![Side::Heroes, Side::Enemies].iter().all(|side| {
            self.actors.iter().any(|a| {
                a.side() == *side
                    && (!a.health.incapacitated()
                        || (a.spec.kind == CreatureKind::Troll
                            && !a.health.dead
                            && !a.health.knocked_out))
            })
        })
    }
    pub(crate) fn actor_mut(&mut self, id: u32) -> Result<&mut Combatant, RuleError> {
        self.actors
            .iter_mut()
            .find(|a| a.id() == id)
            .ok_or(RuleError::UnknownActor)
    }
    pub(crate) fn can_see(&self, from: Cell, to: Cell) -> bool {
        self.map
            .cover_between(from, to, &[])
            .is_ok_and(|cover| cover != super::Cover::Total)
    }
    pub(crate) fn can_see_actor(&self, observer: &Combatant, target: &Combatant) -> bool {
        observer
            .spec
            .space_cells(observer.cell())
            .iter()
            .any(|from| {
                target
                    .spec
                    .space_cells(target.cell())
                    .iter()
                    .any(|to| self.can_see(*from, *to))
            })
            && (self.map.lighting() != super::Lighting::Darkness
                || observer
                    .spec
                    .darkvision_ft()
                    .is_some_and(|range| target.distance_to(observer) <= u32::from(range)))
    }
    pub(crate) fn step_cost(
        &self,
        actor: &Combatant,
        from: Cell,
        to: Cell,
    ) -> Result<u16, RuleError> {
        let from_space = actor.spec.space_cells(from);
        let to_space = actor.spec.space_cells(to);
        if to_space.iter().any(|cell| !self.map.contains(*cell)) {
            return Err(RuleError::OutOfBounds);
        }
        if from_space
            .iter()
            .zip(&to_space)
            .any(|(from, to)| !self.map.step_clear(*from, *to))
        {
            return Err(RuleError::Blocked);
        }
        let mut difficult = to_space.iter().any(|cell| self.map.is_difficult(*cell));
        for other in &self.actors {
            if other.id() != actor.id() && self.overlaps(actor, to, other) && !other.health.dead {
                let size_rank = |size| -> u8 {
                    match size {
                        Size::Small => 0,
                        Size::Medium => 1,
                        Size::Large => 2,
                        Size::Huge => 3,
                    }
                };
                let size_difference =
                    size_rank(actor.spec.size).abs_diff(size_rank(other.spec.size));
                if other.side() != actor.side()
                    && !other.health.incapacitated()
                    && size_difference < 2
                    // SRD p.86 Halfling Nimbleness: pass through any larger creature,
                    // while validate_path still forbids ending in its space.
                    && !(actor.spec.species == Some(HeroSpecies::Halfling)
                        && size_rank(other.spec.size) > size_rank(actor.spec.size))
                {
                    return Err(RuleError::Occupied);
                }
                difficult |= other.side() != actor.side();
            }
        }
        // SRD 5.2.1 p. 14: a dead body no longer occupies creature space; whether
        // its remains are Difficult Terrain needs a DM ruling. This flat-grid
        // policy leaves the cell passable (tactical.dead-space-passable).
        // Crawling and difficult terrain each add one extra foot, rather than multiply.
        Ok(5 * (1 + u16::from(difficult) + u16::from(actor.health.prone)))
    }
    /// Geometry/occupancy cost independent of current movement budget; useful for approach policy.
    pub fn path_cost(&self, id: u32, path: &[Cell]) -> Result<u32, RuleError> {
        let actor = self.actor(id)?;
        if path.len() > 16384 {
            return Err(RuleError::Capacity);
        }
        let mut from = actor.cell();
        let mut movement_cost_ft = 0_u32;
        for &cell in path {
            movement_cost_ft = movement_cost_ft
                .checked_add(u32::from(self.step_cost(actor, from, cell)?))
                .ok_or(RuleError::Capacity)?;
            from = cell;
        }
        Ok(movement_cost_ft)
    }
    pub fn validate_path(&self, id: u32, path: &[Cell]) -> Result<u16, RuleError> {
        let actor = self.actor(id)?;
        if actor.health.incapacitated() {
            return Err(RuleError::Incapacitated);
        }
        if path.is_empty() || path.len() > 1024 {
            return Err(RuleError::InvalidPath);
        }
        let mut from = actor.cell();
        let mut movement_cost_ft = 0_u16;
        for &to in path {
            movement_cost_ft = movement_cost_ft
                .checked_add(self.step_cost(actor, from, to)?)
                .ok_or(RuleError::MovementSpent)?;
            from = to;
        }
        if self.actors.iter().any(|other| {
            other.id() != id && self.overlaps(actor, from, other) && !other.health.dead
        }) {
            return Err(RuleError::Occupied);
        }
        if movement_cost_ft > actor.movement_remaining_ft() {
            return Err(RuleError::MovementSpent);
        }
        Ok(movement_cost_ft)
    }
    /// Dijkstra with deterministic cell ordering; includes no destination already occupied.
    /// If max_cost is None, searches the whole bounded map; otherwise caps total path cost.
    pub fn shortest_path(
        &self,
        id: u32,
        goal: Cell,
        max_cost: Option<u16>,
    ) -> Result<Vec<Cell>, RuleError> {
        let actor = self.actor(id)?;
        if !self.map.contains(goal) {
            return Err(RuleError::OutOfBounds);
        }
        if actor.health.incapacitated() {
            return Err(RuleError::Incapacitated);
        }
        if self.actors.iter().any(|other| {
            other.id() != id && self.overlaps(actor, goal, other) && !other.health.dead
        }) {
            return Err(RuleError::Occupied);
        }
        let mut frontier = BTreeSet::from([(0_u32, actor.cell())]);
        let mut movement_costs_ft = BTreeMap::from([(actor.cell(), 0_u32)]);
        let mut parents = BTreeMap::new();
        while let Some((movement_cost_ft, cell)) = frontier.pop_first() {
            if cell == goal {
                let mut path = Vec::new();
                let mut current = goal;
                while current != actor.cell() {
                    path.push(current);
                    current = *parents.get(&current).ok_or(RuleError::NoPath)?;
                }
                path.reverse();
                return Ok(path);
            }
            for next in self.map.neighbors(cell) {
                let Ok(step_cost_ft) = self.step_cost(actor, cell, next) else {
                    continue;
                };
                let Some(next_cost_ft) = movement_cost_ft.checked_add(u32::from(step_cost_ft))
                else {
                    continue;
                };
                if max_cost.is_some_and(|maximum_cost_ft| next_cost_ft > u32::from(maximum_cost_ft))
                {
                    continue;
                }
                if movement_costs_ft
                    .get(&next)
                    .is_none_or(|old_cost_ft| next_cost_ft < *old_cost_ft)
                {
                    if let Some(old_cost_ft) = movement_costs_ft.insert(next, next_cost_ft) {
                        frontier.remove(&(old_cost_ft, next));
                    }
                    parents.insert(next, cell);
                    frontier.insert((next_cost_ft, next));
                }
            }
        }
        Err(RuleError::NoPath)
    }
    pub fn reachable_cells(&self, id: u32) -> Result<Vec<Cell>, RuleError> {
        let actor = self.actor(id)?;
        if actor.health.incapacitated() {
            return Err(RuleError::Incapacitated);
        }
        let mut frontier = BTreeSet::from([(0_u16, actor.cell())]);
        let mut movement_costs_ft = BTreeMap::from([(actor.cell(), 0_u16)]);
        while let Some((movement_cost_ft, cell)) = frontier.pop_first() {
            for next in self.map.neighbors(cell) {
                let Ok(step_cost_ft) = self.step_cost(actor, cell, next) else {
                    continue;
                };
                let Some(next_cost_ft) = movement_cost_ft.checked_add(step_cost_ft) else {
                    continue;
                };
                if next_cost_ft > actor.movement_remaining_ft() {
                    continue;
                }
                if movement_costs_ft
                    .get(&next)
                    .is_none_or(|old_cost_ft| next_cost_ft < *old_cost_ft)
                {
                    if let Some(old_cost_ft) = movement_costs_ft.insert(next, next_cost_ft) {
                        frontier.remove(&(old_cost_ft, next));
                    }
                    frontier.insert((next_cost_ft, next));
                }
            }
        }
        Ok(movement_costs_ft
            .into_keys()
            .filter(|cell| {
                !self.actors.iter().any(|other| {
                    other.id() != id && self.overlaps(actor, *cell, other) && !other.health.dead
                })
            })
            .collect())
    }
}
/// Begin from validated actor data and supplied Initiative faces. Identical monster groups
/// share one supplied face; ties retain caller order (party agreement / explicit GM choice).
pub(super) fn begin_faces(
    map: BattleMap,
    specs: Vec<CombatantSpec>,
    initiative: &[u32],
) -> Result<Battle, RuleError> {
    if specs.len() < 2 || specs.len() > 64 || specs.len() != initiative.len() {
        return Err(RuleError::Capacity);
    }
    let mut ids = BTreeSet::new();
    let mut cells = BTreeSet::new();
    for spec in &specs {
        if spec.id == 0 || !ids.insert(spec.id) {
            return Err(RuleError::DuplicateId);
        }
        if spec.kind == CreatureKind::Hero && matches!(spec.size, Size::Large | Size::Huge) {
            return Err(RuleError::UnsupportedSize);
        }
        if spec.hit_points == 0
            || spec.hit_points > 1000
            || !(1..=100).contains(&spec.armor_class)
            || spec.speed_ft > 120
            || !spec.speed_ft.is_multiple_of(5)
            || spec.abilities.iter().any(|value| !(1..=30).contains(value))
            || spec.weapons.is_empty()
            || spec.weapons.len() > 12
            || spec.masteries.len() > 4
            || (spec.shield && spec.armor_class < 3)
            || spec.second_wind_uses > 3
            || spec.healers_kit_uses > 10
            || spec.arrows > 1000
            || spec.bolts > 1000
            || spec
                .weapons
                .iter()
                .enumerate()
                .any(|(i, weapon)| spec.weapons[..i].contains(weapon))
            || spec.masteries.iter().enumerate().any(|(i, weapon)| {
                (spec.kind != CreatureKind::Hero && !spec.weapons.contains(weapon))
                    || weapon.mastery().is_none()
                    || spec.masteries[..i].contains(weapon)
            })
        {
            return Err(RuleError::InvalidCombatant);
        }
        validate_source_profile(spec)?;
        if spec
            .space_cells(spec.cell)
            .into_iter()
            .any(|cell| map.is_blocked(cell) || !cells.insert(cell))
        {
            return Err(RuleError::Spawn);
        }
    }
    if ![Side::Heroes, Side::Enemies]
        .iter()
        .all(|side| specs.iter().any(|spec| spec.side == *side))
    {
        return Err(RuleError::InvalidCombatant);
    }
    for (index, &die) in initiative.iter().enumerate() {
        if !(1..=20).contains(&die) {
            return Err(RuleError::InvalidDraw);
        }
        for (earlier, spec) in specs.iter().take(index).enumerate() {
            if spec.kind != CreatureKind::Hero
                && spec.kind == specs[index].kind
                && die != initiative[earlier]
            {
                return Err(RuleError::InvalidDraw);
            }
        }
    }
    // SRD 5.2.1 p. 13 leaves Initiative ties to the DM. The deterministic
    // input order breaks ties here (tactical.initiative-ties-input-order).
    let mut order: Vec<usize> = (0..specs.len()).collect();
    order.sort_by_key(|&i| {
        (
            std::cmp::Reverse(initiative[i] as i32 + specs[i].initiative_modifier()),
            i,
        )
    });
    let order = order.into_iter().map(|index| specs[index].id).collect();
    let initiatives = specs
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            (
                spec.id,
                initiative[index] as i32 + spec.initiative_modifier(),
            )
        })
        .collect();
    let actors = specs
        .into_iter()
        .map(|spec| Combatant {
            health: Health {
                hit_points: spec.hit_points,
                maximum: spec.hit_points,
                prone: false,
                unconscious: false,
                knocked_out: false,
                dead: false,
                stable: false,
                death_successes: 0,
                death_failures: 0,
            },
            budget: Budget {
                action_used: false,
                bonus_action_used: false,
                reaction_used: false,
                object_interaction_used: false,
                movement_spent_ft: 0,
                dash: false,
                disengaged: false,
                dodging: false,
            },
            held: if spec.weapons[0].natural() {
                vec![]
            } else {
                vec![spec.weapons[0]]
            },
            shield_equipped: spec.shield,
            weapon_counts: spec
                .weapons
                .iter()
                .map(|weapon| {
                    (
                        *weapon,
                        if spec.kind == CreatureKind::Ogre && *weapon == Weapon::Javelin {
                            3
                        } else if *weapon == Weapon::Javelin {
                            8
                        } else if spec.kind == CreatureKind::Kobold && *weapon == Weapon::Dagger {
                            3
                        } else {
                            1
                        },
                    )
                })
                .collect(),
            spec,
            turn_count: 0,
            slow: vec![],
            frost_chill: vec![],
            species_uses_spent: 0,
            regeneration_inhibited: false,
            sap: vec![],
            vex: vec![],
            light_first: None,
            light_used: false,
            savage_used: false,
            extra_attacks: 0,
            action_surge_spent: false,
            additional_actions: 0,
            feature_movement_ft: 0,
            additional_dash: false,
            ammunition_used: 0,
            bolts_used: 0,
            recovered: false,
            dropped: vec![],
        })
        .collect();
    let mut battle = Battle {
        map,
        actors,
        order,
        initiatives,
        active: 0,
        round: 1,
        pending: None,
        species_damage: vec![],
        helped_attacks: Vec::new(),
        needs_death_save: false,
    };
    if let Some(id) = battle.active_actor() {
        battle.actor_mut(id)?.turn_count = 1;
    }
    Ok(battle)
}

/// Begin with existing ActualDraw records. Every identical monster kind shares one d20
/// record; hero draws remain individual. Draws follow first occurrence of each group in
/// specs. Tie order is an explicit player agreement/GM choice supplied by specs order.
pub fn begin(
    map: BattleMap,
    specs: Vec<CombatantSpec>,
    binding: super::DrawBinding<'_>,
    records: &[df_model::checkpoint::ActualDraw],
) -> Result<super::Resolution, RuleError> {
    if specs.len() < 2 || specs.len() > 64 {
        return Err(RuleError::Capacity);
    }
    let mut draws = super::resolve::Draws::new(binding, records)?;
    let mut initiatives = Vec::with_capacity(specs.len());
    let mut rerolls = Vec::new();
    for (index, actor) in specs.iter().enumerate() {
        if actor.kind != CreatureKind::Hero
            && let Some(earlier) = specs
                .iter()
                .take(index)
                .position(|other| other.kind == actor.kind)
        {
            initiatives.push(initiatives[earlier]);
            continue;
        }
        let mut face = super::resolve::take_d20(
            actor,
            actor.id,
            super::D20Test::Initiative,
            &mut draws,
            &mut rerolls,
        )?;
        // SRD 5.2.1 p302: a Kobold in sunlight has Disadvantage on ability checks, and
        // Initiative is a Dexterity check. The group makes one roll, with Disadvantage.
        if matches!(actor.kind, CreatureKind::Kobold | CreatureKind::Wight)
            && map.lighting() == super::Lighting::Sunlight
        {
            face = face.min(super::resolve::take_d20(
                actor,
                actor.id,
                super::D20Test::Initiative,
                &mut draws,
                &mut rerolls,
            )?);
        }
        if actor
            .fighter_level()
            .is_some_and(|progression| progression.champion())
        {
            face = face.max(super::resolve::take_d20(
                actor,
                actor.id,
                super::D20Test::Initiative,
                &mut draws,
                &mut rerolls,
            )?);
        }
        initiatives.push(face);
    }
    let battle = begin_faces(map, specs, &initiatives)?;
    Ok(super::Resolution {
        battle,
        draws: draws.finish()?,
        attacks: vec![],
        moves: vec![],
        death_saves: vec![],
        recoveries: vec![],
        helps: vec![],
        stabilizations: vec![],
        rerolls,
        saving_throws: vec![],
    })
}

pub(super) fn validate_source_profile(spec: &CombatantSpec) -> Result<(), RuleError> {
    if spec.kind == CreatureKind::Hero {
        let species = spec.species.ok_or(RuleError::InvalidBuild)?;
        let species_traits = spec.species_options.combat_traits(species);
        let expected_size = match species_traits.size {
            SpeciesSize::Small => Size::Small,
            SpeciesSize::Medium => Size::Medium,
        };
        let progression = spec.fighter_level().ok_or(RuleError::InvalidBuild)?;
        let mut base_abilities = spec.abilities;
        let mut base_masteries = spec.masteries.clone();
        if progression.level() >= 4 {
            let ability = if spec.archery { 1 } else { 0 };
            base_abilities[ability] = base_abilities[ability]
                .checked_sub(2)
                .ok_or(RuleError::InvalidBuild)?;
            if base_masteries.pop()
                != Some(if spec.archery {
                    Weapon::Javelin
                } else {
                    Weapon::Spear
                })
            {
                return Err(RuleError::InvalidBuild);
            }
        }
        let expected_hit_points = i32::from(progression.hit_points(spec.abilities[2], species))
            + if spec.species_options.tough {
                2 * i32::from(progression.level())
            } else {
                0
            };
        let standard_melee = matches!(
            base_abilities,
            [17, 13, 15, 10, 12, 8] | [17, 14, 14, 8, 10, 12]
        ) && spec.armor_class == 17
            && !spec.archery
            && spec.weapons
                == [
                    Weapon::Greatsword,
                    Weapon::Flail,
                    Weapon::Javelin,
                    Weapon::Spear,
                    Weapon::Shortbow,
                ]
            && base_masteries == [Weapon::Greatsword, Weapon::Flail, Weapon::Javelin];
        let standard_archer = base_abilities == [13, 17, 15, 10, 12, 8]
            && spec.armor_class == if progression.level() >= 4 { 16 } else { 15 }
            && spec.archery
            && spec.weapons == [Weapon::Longbow, Weapon::Scimitar, Weapon::Shortsword]
            && base_masteries == spec.weapons;
        if spec.side != Side::Heroes
            || !spec.species_options.valid_for(species)
            || spec.size != expected_size
            || (!standard_melee && !standard_archer)
            || i32::from(spec.hit_points) != expected_hit_points
            || spec.speed_ft != species_traits.speed_ft
            || !spec.savage_attacker
            || spec.second_wind_uses != progression.second_wind_maximum()
            || spec.healers_kit_uses > 10
            || (spec.relentless_endurance_available && !species_traits.relentless_endurance)
            || spec.shield
            || spec.bolts != 0
            || spec.arrows != 20
        {
            return Err(RuleError::InvalidBuild);
        }
    } else {
        if spec.species_options != Default::default()
            || spec.species.is_some()
            || spec.healers_kit_uses != 0
            || spec.relentless_endurance_available
        {
            return Err(RuleError::InvalidBuild);
        }
        let source_id = match spec.kind {
            CreatureKind::Bandit => "bandit",
            CreatureKind::Cultist => "cultist",
            CreatureKind::Guard => "guard",
            CreatureKind::Goblin => "goblin-warrior",
            CreatureKind::Kobold => "kobold-warrior",
            CreatureKind::Skeleton => "skeleton",
            CreatureKind::Zombie => "zombie",
            CreatureKind::Wolf => "wolf",
            CreatureKind::GiantRat => "giant-rat",
            CreatureKind::GiantWeasel => "giant-weasel",
            CreatureKind::Mastiff => "mastiff",
            CreatureKind::Scout => "scout",
            CreatureKind::Commoner => "commoner",
            CreatureKind::Ogre => "ogre",
            CreatureKind::Owlbear => "owlbear",
            CreatureKind::Wight => "wight",
            CreatureKind::HillGiant => "hill-giant",
            CreatureKind::Troll => "troll",
            CreatureKind::Hero => return Err(RuleError::InvalidBuild),
        };
        let mut expected = CombatantSpec::creature(spec.id, source_id, spec.cell, spec.arrows)?;
        expected.bolts = spec.bolts;
        if *spec != expected {
            return Err(RuleError::InvalidCombatant);
        }
    }
    Ok(())
}

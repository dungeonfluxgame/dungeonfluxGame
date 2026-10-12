use super::{BattleMap, Cell, RuleError, Weapon};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Heroes,
    Enemies,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Size {
    Small,
    Medium,
    Large,
}
/// Source stat blocks; departures in the old adjacent journey are not imported here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
}

/// Caller-owned combatant data. Construction validates the bounded supported subset.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CombatantSpec {
    pub id: u32,
    pub side: Side,
    pub cell: Cell,
    pub size: Size,
    pub kind: CreatureKind,
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
    /// An occupied shield hand makes a two-handed attack illegal; caller tracks equipment.
    pub shield: bool,
}
impl CombatantSpec {
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
        Ok(Self {
            id,
            side: Side::Heroes,
            cell,
            size: Size::Medium,
            kind: CreatureKind::Hero,
            abilities: checked.abilities,
            armor_class: 17,
            hit_points: 13,
            speed_ft: 30,
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
            shield: false,
        })
    }
    /// Exact supported SRD 5.2.1 stat block, including speed/actions/traits in resolution.
    /// Night/normal sight only; sunlight/hidden/alternate movement modes are outside this slice.
    pub fn creature(id: u32, srd_id: &str, cell: Cell, ammunition: u16) -> Result<Self, RuleError> {
        let source = crate::SRD_521_CREATURES
            .iter()
            .find(|creature| creature.id == srd_id)
            .ok_or(RuleError::UnsupportedCreature)?;
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
            _ => return Err(RuleError::UnsupportedCreature),
        };
        Ok(Self {
            id,
            side: Side::Enemies,
            cell,
            size,
            kind,
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
            shield,
        })
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Effect {
    pub origin: u32,
    pub expires_turn: u32,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Combatant {
    pub(crate) spec: CombatantSpec,
    pub(crate) health: Health,
    pub(crate) budget: Budget,
    pub(crate) turn_count: u32,
    pub(crate) slow: Vec<Effect>,
    pub(crate) sap: Vec<Effect>,
    pub(crate) vex: Vec<(u32, u32)>,
    pub(crate) light_first: Option<Weapon>,
    pub(crate) light_used: bool,
    pub(crate) savage_used: bool,
    pub(crate) extra_attacks: u8,
    pub(crate) ammunition_used: u16,
    pub(crate) recovered: bool,
    pub(crate) bolts_used: u16,
    pub(crate) weapon_counts: BTreeMap<Weapon, u16>,
    pub(crate) dropped: Vec<(Weapon, Cell)>,
    pub(crate) held: Vec<Weapon>,
    pub(crate) shield_equipped: bool,
}
impl Combatant {
    pub fn id(&self) -> u32 {
        self.spec.id
    }
    pub fn side(&self) -> Side {
        self.spec.side
    }
    pub fn cell(&self) -> Cell {
        self.spec.cell
    }
    pub fn spec(&self) -> &CombatantSpec {
        &self.spec
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
    }
    pub fn movement_remaining_ft(&self) -> u16 {
        (self.speed_ft() * if self.budget.dash { 2 } else { 1 })
            .saturating_sub(self.budget.movement_spent_ft)
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
    pub fn shield_equipped(&self) -> bool {
        self.shield_equipped
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
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
    pub(crate) active: usize,
    pub(crate) round: u32,
    pub(crate) pending: Option<PendingMove>,
    pub(crate) needs_death_save: bool,
}
impl Battle {
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
        let enemies = self
            .actors
            .iter()
            .any(|a| a.side() == Side::Enemies && !a.health.incapacitated());
        match (heroes, enemies) {
            (true, false) => Some(Side::Heroes),
            (false, true) => Some(Side::Enemies),
            _ => None,
        }
    }
    pub fn finished(&self) -> bool {
        ![Side::Heroes, Side::Enemies].iter().all(|side| {
            self.actors
                .iter()
                .any(|a| a.side() == *side && !a.health.incapacitated())
        })
    }
    pub(crate) fn actor_mut(&mut self, id: u32) -> Result<&mut Combatant, RuleError> {
        self.actors
            .iter_mut()
            .find(|a| a.id() == id)
            .ok_or(RuleError::UnknownActor)
    }
    pub(crate) fn cells(&self) -> Vec<Cell> {
        self.actors
            .iter()
            .filter(|a| !a.health.dead)
            .map(Combatant::cell)
            .collect()
    }
    pub(crate) fn can_see(&self, from: Cell, to: Cell) -> bool {
        self.map
            .cover_between(from, to, &[])
            .is_ok_and(|cover| cover != super::Cover::Total)
    }
    pub(crate) fn step_cost(
        &self,
        actor: &Combatant,
        from: Cell,
        to: Cell,
    ) -> Result<u16, RuleError> {
        if !self.map.contains(to) {
            return Err(RuleError::OutOfBounds);
        }
        if !self.map.step_clear(from, to) {
            return Err(RuleError::Blocked);
        }
        let mut difficult = self.map.is_difficult(to);
        for other in &self.actors {
            if other.id() != actor.id() && other.cell() == to && !other.health.dead {
                if other.side() != actor.side() && !other.health.incapacitated() {
                    return Err(RuleError::Occupied);
                }
                difficult |= other.side() != actor.side();
            }
        }
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
        let mut cost = 0_u32;
        for &cell in path {
            cost = cost
                .checked_add(u32::from(self.step_cost(actor, from, cell)?))
                .ok_or(RuleError::Capacity)?;
            from = cell;
        }
        Ok(cost)
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
        let mut cost = 0_u16;
        for &to in path {
            cost = cost
                .checked_add(self.step_cost(actor, from, to)?)
                .ok_or(RuleError::MovementSpent)?;
            from = to;
        }
        if self
            .actors
            .iter()
            .any(|other| other.id() != id && other.cell() == from && !other.health.dead)
        {
            return Err(RuleError::Occupied);
        }
        if cost > actor.movement_remaining_ft() {
            return Err(RuleError::MovementSpent);
        }
        Ok(cost)
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
        if self
            .actors
            .iter()
            .any(|other| other.id() != id && other.cell() == goal && !other.health.dead)
        {
            return Err(RuleError::Occupied);
        }
        let mut frontier = BTreeSet::from([(0_u32, actor.cell())]);
        let mut costs = BTreeMap::from([(actor.cell(), 0_u32)]);
        let mut parents = BTreeMap::new();
        while let Some((cost, cell)) = frontier.pop_first() {
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
                let Ok(step) = self.step_cost(actor, cell, next) else {
                    continue;
                };
                let Some(next_cost) = cost.checked_add(u32::from(step)) else {
                    continue;
                };
                if max_cost.is_some_and(|max| next_cost > u32::from(max)) {
                    continue;
                }
                if costs.get(&next).is_none_or(|old| next_cost < *old) {
                    if let Some(old) = costs.insert(next, next_cost) {
                        frontier.remove(&(old, next));
                    }
                    parents.insert(next, cell);
                    frontier.insert((next_cost, next));
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
        let mut costs = BTreeMap::from([(actor.cell(), 0_u16)]);
        while let Some((cost, cell)) = frontier.pop_first() {
            for next in self.map.neighbors(cell) {
                let Ok(step) = self.step_cost(actor, cell, next) else {
                    continue;
                };
                let Some(next_cost) = cost.checked_add(step) else {
                    continue;
                };
                if next_cost > actor.movement_remaining_ft() {
                    continue;
                }
                if costs.get(&next).is_none_or(|old| next_cost < *old) {
                    if let Some(old) = costs.insert(next, next_cost) {
                        frontier.remove(&(old, next));
                    }
                    frontier.insert((next_cost, next));
                }
            }
        }
        Ok(costs
            .into_keys()
            .filter(|cell| {
                !self
                    .actors
                    .iter()
                    .any(|other| other.id() != id && other.cell() == *cell && !other.health.dead)
            })
            .collect())
    }
}
/// Begin from validated actor data and supplied Initiative faces. Identical monster groups
/// share one supplied face; ties retain caller order (party agreement / explicit GM choice).
fn begin_faces(
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
        if spec.size == Size::Large {
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
            || spec.masteries.len() > 3
            || (spec.shield && spec.armor_class < 3)
            || spec.second_wind_uses > 2
            || spec.arrows > 1000
            || spec.bolts > 1000
            || spec
                .weapons
                .iter()
                .enumerate()
                .any(|(i, weapon)| spec.weapons[..i].contains(weapon))
            || spec.masteries.iter().enumerate().any(|(i, weapon)| {
                !spec.weapons.contains(weapon)
                    || weapon.mastery().is_none()
                    || spec.masteries[..i].contains(weapon)
            })
        {
            return Err(RuleError::InvalidCombatant);
        }
        validate_source_profile(spec)?;
        if map.is_blocked(spec.cell) || !cells.insert(spec.cell) {
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
    let mut order: Vec<usize> = (0..specs.len()).collect();
    order.sort_by_key(|&i| {
        (
            std::cmp::Reverse(
                initiative[i] as i32
                    + crate::local_journey::ability_modifier(specs[i].abilities[1]),
            ),
            i,
        )
    });
    let order = order.into_iter().map(|index| specs[index].id).collect();
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
            held: if matches!(spec.weapons[0], Weapon::Bite | Weapon::Slam) {
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
                        if *weapon == Weapon::Javelin {
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
            sap: vec![],
            vex: vec![],
            light_first: None,
            light_used: false,
            savage_used: false,
            extra_attacks: 0,
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
        active: 0,
        round: 1,
        pending: None,
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
        initiatives.push(draws.take(20)?);
    }
    let battle = begin_faces(map, specs, &initiatives)?;
    Ok(super::Resolution {
        battle,
        draws: draws.finish()?,
        attacks: vec![],
    })
}

fn validate_source_profile(spec: &CombatantSpec) -> Result<(), RuleError> {
    if spec.kind == CreatureKind::Hero {
        let standard_melee = matches!(
            spec.abilities,
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
            && spec.masteries == [Weapon::Greatsword, Weapon::Flail, Weapon::Javelin];
        let standard_archer = spec.abilities == [13, 17, 15, 10, 12, 8]
            && spec.armor_class == 15
            && spec.archery
            && spec.weapons == [Weapon::Longbow, Weapon::Scimitar, Weapon::Shortsword]
            && spec.masteries == spec.weapons;
        if spec.side != Side::Heroes
            || spec.size != Size::Medium
            || (!standard_melee && !standard_archer)
            || spec.hit_points != 13
            || spec.speed_ft != 30
            || !spec.savage_attacker
            || spec.second_wind_uses != 2
            || spec.shield
            || spec.bolts != 0
            || spec.arrows != 20
        {
            return Err(RuleError::InvalidBuild);
        }
    } else {
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

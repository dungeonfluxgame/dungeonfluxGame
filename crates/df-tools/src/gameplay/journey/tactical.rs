//! Native tactical orchestration. Rules alone own legality, positions and consequences.
use super::*;
use df_rules::tactical as tactical_rules;
use sha2::{Digest, Sha256};
use tactical_rules::{
    Action, AttackSpend, AttackUse, Battle, BattleMap, CombatantSpec, DrawBinding, RuleError, Side,
    Weapon,
};

const ENEMY_ENTITIES: [[u8; 16]; 4] = [[0x65; 16], [0x71; 16], [0x72; 16], [0x73; 16]];
const SCENES: &[(&str, &[u8])] = &[
    (
        "salt-quay",
        include_bytes!("../../../../../content/battle-scenes/salt-quay.json"),
    ),
    (
        "pinewood-trail",
        include_bytes!("../../../../../content/battle-scenes/pinewood-trail.json"),
    ),
    (
        "windward-pass",
        include_bytes!("../../../../../content/battle-scenes/windward-pass.json"),
    ),
    (
        "lamplit-market",
        include_bytes!("../../../../../content/battle-scenes/lamplit-market.json"),
    ),
    (
        "rookhill-gate",
        include_bytes!("../../../../../content/battle-scenes/rookhill-gate.json"),
    ),
    (
        "reedmoor-boardwalk",
        include_bytes!("../../../../../content/battle-scenes/reedmoor-boardwalk.json"),
    ),
    (
        "thistledown-farm",
        include_bytes!("../../../../../content/battle-scenes/thistledown-farm.json"),
    ),
    (
        "orrery-library",
        include_bytes!("../../../../../content/battle-scenes/orrery-library.json"),
    ),
    (
        "harbor-journey",
        include_bytes!("../../../../../content/battle-scenes/harbor-journey.json"),
    ),
    // The Gaussian-splat scan scenes. A story location never names one: they are reached only
    // when `DF_GAMEPLAY_BATTLE_SCENE` forces the scene (see `force_scene`), and the three that carry
    // no licence are private test scenes.
    (
        "scan-riverbed",
        include_bytes!("../../../../../content/battle-scenes/scan-riverbed.json"),
    ),
    (
        "scan-village",
        include_bytes!("../../../../../content/battle-scenes/scan-village.json"),
    ),
    (
        "scan-valley-bridges",
        include_bytes!("../../../../../content/battle-scenes/scan-valley-bridges.json"),
    ),
    (
        "scan-summit",
        include_bytes!("../../../../../content/battle-scenes/scan-summit.json"),
    ),
];

/// The typed lighting fact of every scene the server holds (SRD 5.2.1 p302 Sunlight Sensitivity).
/// The painted scenes are night or interior (checked against their backdrop paintings); the
/// photographic Gaussian-splat scans are daylight. Scene files cannot carry it themselves: the
/// renderer rejects unknown fields.
const LIGHTING: &[(&str, tactical_rules::Lighting)] = &[
    ("salt-quay", tactical_rules::Lighting::NightOrInterior),
    ("pinewood-trail", tactical_rules::Lighting::NightOrInterior),
    ("windward-pass", tactical_rules::Lighting::NightOrInterior),
    ("lamplit-market", tactical_rules::Lighting::NightOrInterior),
    ("rookhill-gate", tactical_rules::Lighting::NightOrInterior),
    (
        "reedmoor-boardwalk",
        tactical_rules::Lighting::NightOrInterior,
    ),
    (
        "thistledown-farm",
        tactical_rules::Lighting::NightOrInterior,
    ),
    ("orrery-library", tactical_rules::Lighting::NightOrInterior),
    ("harbor-journey", tactical_rules::Lighting::NightOrInterior),
    ("scan-riverbed", tactical_rules::Lighting::Sunlight),
    ("scan-village", tactical_rules::Lighting::Sunlight),
    ("scan-valley-bridges", tactical_rules::Lighting::Sunlight),
    ("scan-summit", tactical_rules::Lighting::Sunlight),
];
/// The rules map for authored scene bytes the server holds, with its lighting fact.
fn scene_map(bytes: &[u8], layout: &str) -> Result<BattleMap, RepositoryError> {
    let id = SCENES
        .iter()
        .find(|(_, held)| *held == bytes)
        .map(|(id, _)| *id)
        .ok_or(RepositoryError::InvalidCandidate)?;
    let lighting = LIGHTING
        .iter()
        .find(|(known, _)| *known == id)
        .map(|(_, lighting)| *lighting)
        .ok_or(RepositoryError::InvalidCandidate)?;
    Ok(BattleMap::from_json(bytes, layout)
        .map_err(bad)?
        .with_lighting(lighting))
}

/// The scene every fight is staged on, when `DF_GAMEPLAY_BATTLE_SCENE` forces one. The server is
/// the authority for the grid, so it chooses the layout from the scene that will be shown, not
/// from the story location's own scene.
static FORCED_SCENE: std::sync::OnceLock<Option<&'static str>> = std::sync::OnceLock::new();

/// Records the forced scene at startup. The id must be one of the scenes the server holds.
pub(in crate::gameplay) fn force_scene(id: Option<&str>) -> Result<(), String> {
    let forced = match id {
        None => None,
        Some(id) => Some(
            SCENES
                .iter()
                .find(|(known, _)| *known == id)
                .map(|(known, _)| *known)
                .ok_or_else(|| format!("DF_GAMEPLAY_BATTLE_SCENE {id:?} has no authored grid"))?,
        ),
    };
    FORCED_SCENE
        .set(forced)
        .map_err(|_| "the forced battle scene was already set".to_owned())
}

/// The scene a fight at the story's `place` is staged on.
fn staged_location(place: String) -> String {
    #[cfg(test)]
    if let Some(own) = TEST_SCENE_OVERRIDE.with(std::cell::Cell::get) {
        return own.to_owned();
    }
    FORCED_SCENE
        .get()
        .copied()
        .flatten()
        .map_or(place, str::to_owned)
}

/// The authored scene bytes the server holds for a scene id.
#[cfg(test)]
pub(super) fn scene_bytes(id: &str) -> Option<&'static [u8]> {
    SCENES
        .iter()
        .find(|(known, _)| *known == id)
        .map(|(_, bytes)| *bytes)
}

/// Every scene id the server holds.
#[cfg(test)]
pub(super) fn scene_ids() -> impl Iterator<Item = &'static str> {
    SCENES.iter().map(|(id, _)| *id)
}

pub(in crate::gameplay) fn enabled() -> bool {
    pack::active().is_some_and(|loaded| loaded.tuning.tactical.is_some())
}
pub(in crate::gameplay) fn present(state: &GameState) -> bool {
    state
        .encounters
        .iter()
        .any(|encounter| encounter.tactical.is_some())
}
/// The development setting that starts every fight against a chosen number of enemies
/// (`DF_GAMEPLAY_TEST_FIGHT`), optionally of a chosen creature (`DF_GAMEPLAY_TEST_FOE`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct TestFight {
    pub(in crate::gameplay) enemies: usize,
    pub(in crate::gameplay) foe: Option<String>,
}

/// The most enemies a fight holds: one entity per enemy, and four spawn cells on every layout
/// of every battle scene. Six heroes against six enemies needs both raised (see
/// `development/README.md`).
pub(in crate::gameplay) const MAXIMUM_ENEMIES: usize = ENEMY_ENTITIES.len();
/// Arrows and bolts a test fight's enemy carries when the creature is not the adventure's own.
const TEST_FOE_AMMUNITION: u16 = 20;

static TEST_FIGHT: std::sync::OnceLock<Option<TestFight>> = std::sync::OnceLock::new();

/// Records the test fight at startup; unset leaves every fight as the adventure made it.
pub(in crate::gameplay) fn force_test_fight(setting: Option<TestFight>) -> Result<(), String> {
    TEST_FIGHT
        .set(setting)
        .map_err(|_| "the test fight was already set".to_owned())
}

/// Whether this server runs the test fight setting.
pub(in crate::gameplay) fn test_fight_enabled() -> bool {
    test_fight().is_some()
}

#[cfg(test)]
thread_local! {
    /// A test's own setting and battle scene, so tests of one process do not share the global.
    static TEST_FIGHT_OVERRIDE: std::cell::RefCell<Option<TestFight>> = const { std::cell::RefCell::new(None) };
    static TEST_SCENE_OVERRIDE: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

fn test_fight() -> Option<TestFight> {
    #[cfg(test)]
    if let Some(own) = TEST_FIGHT_OVERRIDE.with(|own| own.borrow().clone()) {
        return Some(own);
    }
    TEST_FIGHT.get().cloned().flatten()
}

/// The enemies of a fight, in the order they take the entities and spawn cells: the adventure's
/// own groups, or `count` of the creature a test fight names (the adventure's first creature when
/// it names none).
fn roster(
    groups: &[pack::TacticalEnemy],
    test: Option<&TestFight>,
) -> Result<Vec<(String, u16)>, RepositoryError> {
    let Some(test) = test else {
        return Ok(groups
            .iter()
            .flat_map(|group| {
                std::iter::repeat_n((group.srd_id.clone(), group.ammunition), group.count.into())
            })
            .collect());
    };
    let own = groups.first().ok_or(RepositoryError::InvalidCandidate)?;
    let (srd_id, ammunition) = match &test.foe {
        Some(foe) => (foe.clone(), TEST_FOE_AMMUNITION),
        None => (own.srd_id.clone(), own.ammunition),
    };
    Ok(vec![(srd_id, ammunition); test.enemies])
}

pub(super) fn enemies() -> Result<Vec<(EntityId, String, u16)>, RepositoryError> {
    let loaded = pack::active().ok_or(RepositoryError::InvalidCandidate)?;
    let tuning = loaded
        .tuning
        .tactical
        .as_ref()
        .ok_or(RepositoryError::InvalidCandidate)?;
    roster(&tuning.enemies, test_fight().as_ref())?
        .into_iter()
        .enumerate()
        .map(|(index, (srd_id, ammunition))| {
            let bytes = ENEMY_ENTITIES.get(index).ok_or(RepositoryError::Capacity)?;
            Ok((entity(*bytes)?, srd_id, ammunition))
        })
        .collect()
}
pub(super) fn snapshot(state: &GameState) -> Result<&TacticalSnapshot, RepositoryError> {
    let [encounter] = state.encounters.as_slice() else {
        return Err(RepositoryError::InvalidCandidate);
    };
    encounter
        .tactical
        .as_ref()
        .ok_or(RepositoryError::InvalidCandidate)
}
pub(super) fn read(state: &GameState) -> Result<Battle, RepositoryError> {
    let saved = snapshot(state)?;
    if saved.schema != 1
        || saved.source_revision.as_str() != tactical_rules::SOURCE_REVISION
        || saved.scene_digest.0 != <[u8; 32]>::from(Sha256::digest(&saved.scene))
        || !SCENES
            .iter()
            .any(|(_, bytes)| *bytes == saved.scene.as_slice())
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let map = scene_map(&saved.scene, saved.layout.as_str())?;
    let battle = Battle::from_snapshot(map, &saved.state).map_err(bad)?;
    if battle.combatants().len() != saved.actors.len()
        || battle
            .combatants()
            .iter()
            .any(|actor| !saved.actors.iter().any(|(id, _)| *id == actor.id()))
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    Ok(battle)
}
fn canonical(state: &GameState, id: u32) -> Result<EntityId, RepositoryError> {
    snapshot(state)?
        .actors
        .iter()
        .find(|(candidate, _)| *candidate == id)
        .map(|(_, entity)| *entity)
        .ok_or(RepositoryError::InvalidCandidate)
}
fn actor_id(state: &GameState, entity: EntityId) -> Result<u32, RepositoryError> {
    snapshot(state)?
        .actors
        .iter()
        .find(|(_, candidate)| *candidate == entity)
        .map(|(id, _)| *id)
        .ok_or(RepositoryError::InvalidCandidate)
}
fn binding<'a>(
    operation: OperationId,
    source: &'a RuleReference,
    ordinal: u32,
) -> Result<DrawBinding<'a>, RepositoryError> {
    Ok(DrawBinding {
        operation,
        resolution: ResolutionId::from_bytes(operation.as_bytes()).map_err(bad)?,
        window: WindowId::from_bytes(operation.as_bytes()).map_err(bad)?,
        source,
        next_ordinal: ordinal,
    })
}
/// Records `step` as the refusing step when `result` is an error, then passes it through.
/// The first note of a command wins, so the innermost named step is what the session
/// owner's `REFUSAL` line prints.
fn noted<T>(
    step: &'static str,
    result: Result<T, RepositoryError>,
) -> Result<T, RepositoryError> {
    if result.is_err() {
        crate::gameplay::refusal_detail::note(step);
    }
    result
}
fn resolve_with_draws(
    operation: OperationId,
    draws: &mut Vec<ActualDraw>,
    supplier: DiceSource<'_>,
    mut resolve: impl FnMut(
        DrawBinding<'_>,
        &[ActualDraw],
    ) -> Result<tactical_rules::Resolution, RuleError>,
) -> Result<tactical_rules::Resolution, RepositoryError> {
    let source = rule()?;
    let start = draws.len();
    let ordinal = u32::try_from(start).map_err(bad)?;
    loop {
        match resolve(binding(operation, &source, ordinal)?, &draws[start..]) {
            Ok(result) => return Ok(result),
            Err(RuleError::MissingDraw { sides }) => {
                draw(operation, sides, draws, supplier)?;
            }
            Err(_) => {
                crate::gameplay::refusal_detail::note("tactical-resolve");
                return Err(RepositoryError::InvalidCandidate);
            }
        }
    }
}
fn store(state: &mut GameState, battle: &Battle) -> Result<(), RepositoryError> {
    let saved = snapshot(state)?.clone();
    for actor in battle.combatants() {
        let who = saved
            .actors
            .iter()
            .find(|(id, _)| *id == actor.id())
            .map(|(_, who)| *who)
            .ok_or(RepositoryError::InvalidCandidate)?;
        let health = actor.health();
        let budget = actor.budget();
        for (key, value) in [
            ("hit-points", u32::from(health.hit_points)),
            ("armor-class", u32::from(actor.spec().armor_class)),
            ("unconscious", u32::from(health.unconscious)),
            ("prone", u32::from(health.prone)),
            ("held-weapon", u32::from(!actor.held_weapons().is_empty())),
            ("second-wind", u32::from(actor.spec().second_wind_uses)),
            ("action-used", u32::from(budget.action_used)),
            ("bonus-used", u32::from(budget.bonus_action_used)),
        ] {
            set(state, who, key, value)?;
        }
        if actor.side() == Side::Heroes {
            for (entry, quantity) in [
                ("arrow", u64::from(actor.arrows())),
                ("crossbow-bolt", u64::from(actor.bolts())),
            ]
            .into_iter()
            .chain(
                actor
                    .spec()
                    .weapons
                    .iter()
                    .map(|weapon| (weapon.id(), u64::from(actor.weapon_count(*weapon)))),
            ) {
                if let Some(item) = state
                    .inventory
                    .iter_mut()
                    .find(|item| item.owner == who && item.origin.entry.as_str() == entry)
                {
                    item.quantity = quantity;
                } else if quantity != 0 {
                    return Err(RepositoryError::InvalidCandidate);
                }
            }
        }
        if actor.side() == Side::Heroes {
            state
                .inventory
                .retain(|item| item.owner != who || item.quantity != 0);
        }
        let world = state
            .entities
            .iter_mut()
            .find(|entity| entity.id == who)
            .ok_or(RepositoryError::InvalidCandidate)?;
        world.position = Some(Position {
            x: i64::from(actor.cell().0) * 5,
            y: i64::from(actor.cell().1) * 5,
            z: 0,
        });
    }
    let order = battle
        .initiative_order()
        .iter()
        .map(|id| canonical(state, *id))
        .collect::<Result<Vec<_>, _>>()?;
    let terminal = state.encounters.first().is_some_and(|encounter| {
        encounter
            .objectives
            .iter()
            .any(|objective| matches!(objective.entry.as_str(), "combat-victory" | "combat-defeat"))
    });
    let active = if battle.finished() || terminal {
        None
    } else {
        battle
            .active_actor()
            .map(|id| canonical(state, id))
            .transpose()?
    };
    let encounter = state
        .encounters
        .first_mut()
        .ok_or(RepositoryError::InvalidCandidate)?;
    encounter.turn_order = order;
    encounter.active_turn = active;
    encounter
        .tactical
        .as_mut()
        .ok_or(RepositoryError::InvalidCandidate)?
        .state = battle.snapshot().map_err(bad)?;
    Ok(())
}
fn grid_cell(cell: tactical_rules::Cell) -> rpc::GridCell {
    rpc::GridCell {
        col: u32::from(cell.0),
        row: u32::from(cell.1),
    }
}
fn cover_label(cover: tactical_rules::Cover) -> &'static str {
    match cover {
        tactical_rules::Cover::None => "none",
        tactical_rules::Cover::Half => "half (+2 AC)",
        tactical_rules::Cover::ThreeQuarters => "three quarters (+5 AC)",
        tactical_rules::Cover::Total => "total",
    }
}
/// The fight's attacks so far, in server order: earlier ops from the decision history,
/// then this op's already-recorded attacks. Only public outcome data feeds line picks.
fn prior_attacks(
    state: &GameState,
    heroes: &std::collections::HashSet<String>,
    speakers: &std::collections::HashMap<String, bool>,
    done: &[super::super::fight_lines::AttackFact],
) -> Result<Vec<super::super::fight_lines::AttackFact>, RepositoryError> {
    use super::super::fight_lines::AttackFact;
    let mut prior = Vec::new();
    for decision in &state.decisions {
        if decision.source_policy.as_str() != THREAD_POLICY {
            continue;
        }
        for outcome in accepted(decision)?.combat {
            if !matches!(
                rpc::OutcomeKind::try_from(outcome.kind),
                Ok(rpc::OutcomeKind::Attack | rpc::OutcomeKind::OpportunityAttack)
            ) {
                continue;
            }
            let attacker_can_speak = speakers.get(&outcome.actor_name).copied().unwrap_or(false);
            let target_can_speak = speakers.get(&outcome.target_name).copied().unwrap_or(false);
            prior.push(AttackFact {
                attacker: outcome.actor_name.clone(),
                attacker_hero: heroes.contains(&outcome.actor_name),
                attacker_can_speak,
                target: outcome.target_name.clone(),
                target_hero: heroes.contains(&outcome.target_name),
                target_can_speak,
                hit: outcome.hit,
                critical: outcome.critical,
                damage: outcome.damage,
                target_down: outcome.knocked_out,
                target_hp: outcome.target_hit_points,
            });
        }
    }
    prior.extend(done.iter().cloned());
    Ok(prior)
}

fn creature_can_speak(kind: tactical_rules::CreatureKind) -> bool {
    let id = match kind {
        tactical_rules::CreatureKind::Bandit => "bandit",
        tactical_rules::CreatureKind::Cultist => "cultist",
        tactical_rules::CreatureKind::Guard => "guard",
        tactical_rules::CreatureKind::Goblin => "goblin-warrior",
        tactical_rules::CreatureKind::Kobold => "kobold-warrior",
        tactical_rules::CreatureKind::Skeleton => "skeleton",
        tactical_rules::CreatureKind::Zombie => "zombie",
        tactical_rules::CreatureKind::Wolf => "wolf",
        tactical_rules::CreatureKind::GiantRat => "giant-rat",
        tactical_rules::CreatureKind::GiantWeasel => "giant-weasel",
        tactical_rules::CreatureKind::Mastiff => "mastiff",
        tactical_rules::CreatureKind::Scout => "scout",
        tactical_rules::CreatureKind::Hero => return true,
    };
    df_rules::SRD_521_CREATURES
        .iter()
        .find(|creature| creature.id == id)
        .is_some_and(|creature| creature.can_speak())
}

/// The cry pool family of one combatant: heroes cry from the hero pools by their chosen
/// portrait, enemies from the enemy pools by creature kind. Mirrors the voice families
/// the participant views carry for effort, pain and fall sounds.
fn cry_pool(state: &GameState, battle: &Battle, actor: u32, hero: bool) -> &'static str {
    if hero {
        let female = state
            .characters
            .iter()
            .find(|character| canonical(state, actor).is_ok_and(|who| character.entity == who))
            .and_then(|character| {
                character
                    .choices
                    .iter()
                    .find(|choice| choice.offer.as_str() == "portrait")
            })
            .is_some_and(|choice| {
                matches!(
                    choice.selected.as_str(),
                    "portrait-hero-2" | "portrait-hero-3"
                )
            });
        return if female { "hero-female" } else { "hero-male" };
    }
    let kind = battle
        .actor(actor)
        .map(|combatant| combatant.spec().kind)
        .unwrap_or(tactical_rules::CreatureKind::Bandit);
    match kind {
        tactical_rules::CreatureKind::Goblin | tactical_rules::CreatureKind::Kobold => {
            "small-creature"
        }
        tactical_rules::CreatureKind::Wolf
        | tactical_rules::CreatureKind::GiantRat
        | tactical_rules::CreatureKind::GiantWeasel
        | tactical_rules::CreatureKind::Mastiff => "beast",
        tactical_rules::CreatureKind::Skeleton | tactical_rules::CreatureKind::Zombie => "undead",
        _ => "humanoid",
    }
}

/// The public record of one accepted resolution, in the order it happened: squares entered
/// with their cost, attacks with the distance and mode that made them legal, Death Saving
/// Throws with the running counts. A reaction attack happens before the move it interrupts.
fn record_outcomes(
    state: &GameState,
    result: &tactical_rules::Resolution,
    opportunity: bool,
    outcomes: &mut Vec<rpc::CombatOutcome>,
) -> Result<(), RepositoryError> {
    use super::super::fight_lines::{AttackFact, CurrentAttack, FightSoFar};
    if !opportunity {
        record_moves(state, result, outcomes)?;
    }
    // Heroes in combatant order, matching party order, so hero cries carry their stage slot.
    let mut heroes: Vec<String> = Vec::new();
    let mut speakers = std::collections::HashMap::new();
    for combatant in result.battle.combatants() {
        let combatant_name = name(state, canonical(state, combatant.id())?)?;
        speakers.insert(
            combatant_name.clone(),
            combatant.side() == Side::Heroes || creature_can_speak(combatant.spec().kind),
        );
        if combatant.side() != Side::Heroes {
            continue;
        }
        heroes.push(combatant_name);
    }
    let hero_set: std::collections::HashSet<String> = heroes.iter().cloned().collect();
    let mut enemies: Vec<String> = Vec::new();
    for combatant in result.battle.combatants() {
        if combatant.side() == Side::Heroes {
            continue;
        }
        enemies.push(name(state, canonical(state, combatant.id())?)?);
    }
    let mut done: Vec<AttackFact> = Vec::new();
    for attack in &result.attacks {
        let actor = canonical(state, attack.actor)?;
        let target = canonical(state, attack.target)?;
        let actor_name = name(state, actor)?;
        let target_name = name(state, target)?;
        let actor_hero = hero_set.contains(&actor_name);
        let attacker_can_speak = speakers.get(&actor_name).copied().unwrap_or(false);
        let target_can_speak = speakers.get(&target_name).copied().unwrap_or(false);
        let health = result.battle.actor(attack.target).map_err(bad)?.health();
        let target_hp = u32::from(health.hit_points);
        let target_max_hp = u32::from(health.maximum);
        let fact = AttackFact {
            attacker: actor_name.clone(),
            attacker_hero: actor_hero,
            attacker_can_speak,
            target: target_name.clone(),
            target_hero: hero_set.contains(&target_name),
            target_can_speak,
            hit: attack.hit,
            critical: attack.critical,
            damage: u32::from(attack.damage),
            target_down: attack.knocked_out,
            target_hp,
        };
        let so_far = FightSoFar {
            prior: prior_attacks(state, &hero_set, &speakers, &done)?,
            enemy_names: enemies.clone(),
        };
        let mut lines = super::super::fight_lines::select(
            &CurrentAttack {
                fact: fact.clone(),
                attacker_pool: cry_pool(state, &result.battle, attack.actor, actor_hero),
                die: attack.selected,
                total: attack.selected as i32 + i32::from(attack.modifier),
                target_max_hp,
            },
            &so_far,
        );
        for line in &mut lines {
            if line.role == rpc::TableRole::Hero as i32
                && let Some(index) = heroes.iter().position(|hero| *hero == line.speaker)
            {
                line.hero_index = u32::try_from(index).unwrap_or(0);
            }
        }
        done.push(fact);
        outcomes.push(rpc::CombatOutcome {
            actor_name,
            target_name,
            attack_die: attack.selected,
            attack_modifier: i32::from(attack.modifier),
            attack_total: attack.selected as i32 + i32::from(attack.modifier),
            // Enemy defense is private mechanical truth, omitted from public outcomes.
            target_armor_class: 0,
            hit: attack.hit,
            critical: attack.critical,
            damage_dice: attack.damage_faces.clone(),
            damage: u32::from(attack.damage),
            target_hit_points: u32::from(
                result
                    .battle
                    .actor(attack.target)
                    .map_err(bad)?
                    .health()
                    .hit_points,
            ),
            knocked_out: attack.knocked_out,
            grazed: attack.graze,
            source_revision: model::source_revision(),
            actor_id: actor.as_bytes().to_vec(),
            target_id: target.as_bytes().to_vec(),
            weapon: attack.weapon.id().to_owned(),
            attack_style: if attack.ranged { 2 } else { 1 },
            damage_type: attack.damage_type.id().to_owned(),
            distance_ft: attack.distance_ft,
            kind: if opportunity {
                rpc::OutcomeKind::OpportunityAttack
            } else {
                rpc::OutcomeKind::Attack
            } as i32,
            roll_mode: i32::from(attack.roll_mode),
            faces: attack.faces.clone(),
            reasons: attack
                .advantage
                .iter()
                .map(|reason| format!("advantage: {}", reason_label(*reason)))
                .chain(
                    attack
                        .disadvantage
                        .iter()
                        .map(|reason| format!("disadvantage: {}", reason_label(*reason))),
                )
                .collect(),
            cover: cover_label(attack.cover).to_owned(),
            lines,
            ..Default::default()
        });
    }
    if opportunity {
        record_moves(state, result, outcomes)?;
    }
    for recovery in &result.recoveries {
        let actor = canonical(state, recovery.actor)?;
        outcomes.push(rpc::CombatOutcome {
            actor_name: name(state, actor)?,
            target_hit_points: 1,
            source_revision: model::source_revision(),
            actor_id: actor.as_bytes().to_vec(),
            kind: rpc::OutcomeKind::Recovery as i32,
            recovery_hours: recovery.hours,
            ..Default::default()
        });
    }
    for save in &result.death_saves {
        let actor = canonical(state, save.actor)?;
        use tactical_rules::DeathSaveResult as R;
        outcomes.push(rpc::CombatOutcome {
            actor_name: name(state, actor)?,
            attack_die: save.die,
            target_hit_points: u32::from(
                result
                    .battle
                    .actor(save.actor)
                    .map_err(bad)?
                    .health()
                    .hit_points,
            ),
            source_revision: model::source_revision(),
            actor_id: actor.as_bytes().to_vec(),
            kind: rpc::OutcomeKind::DeathSave as i32,
            death_save_result: match save.result {
                R::Success => rpc::DeathSaveResult::Success,
                R::Failure => rpc::DeathSaveResult::Failure,
                R::CriticalFailure => rpc::DeathSaveResult::CriticalFailure,
                R::Stabilised => rpc::DeathSaveResult::Stabilised,
                R::Died => rpc::DeathSaveResult::Died,
                R::Revived => rpc::DeathSaveResult::Revived,
            } as i32,
            death_successes: u32::from(save.successes),
            death_failures: u32::from(save.failures),
            ..Default::default()
        });
    }
    Ok(())
}
fn record_moves(
    state: &GameState,
    result: &tactical_rules::Resolution,
    outcomes: &mut Vec<rpc::CombatOutcome>,
) -> Result<(), RepositoryError> {
    for step in &result.moves {
        let actor = canonical(state, step.actor)?;
        outcomes.push(rpc::CombatOutcome {
            actor_name: name(state, actor)?,
            source_revision: model::source_revision(),
            actor_id: actor.as_bytes().to_vec(),
            kind: rpc::OutcomeKind::Move as i32,
            path: step.path.iter().copied().map(grid_cell).collect(),
            cost_ft: u32::from(step.cost_ft),
            remaining_ft: u32::from(step.remaining_ft),
            ..Default::default()
        });
    }
    Ok(())
}
fn apply(
    state: &mut GameState,
    battle: &Battle,
    actor: u32,
    action: &Action,
    operation: OperationId,
    recording: (&mut Vec<ActualDraw>, &mut Vec<rpc::CombatOutcome>),
    supplier: DiceSource<'_>,
) -> Result<Battle, RepositoryError> {
    let (draws, outcomes) = recording;
    let round = battle.round();
    let result = resolve_with_draws(operation, draws, supplier, |binding, records| {
        tactical_rules::resolve(battle, actor, action, binding, records)
    })?;
    if result.battle.round() != round {
        state.logical_time.ticks = state
            .logical_time
            .ticks
            .checked_add(6)
            .ok_or(RepositoryError::Capacity)?;
    }
    record_outcomes(
        state,
        &result,
        matches!(action, Action::Opportunity { .. }),
        outcomes,
    )?;
    store(state, &result.battle)?;
    Ok(result.battle)
}
fn finish_if_done(state: &mut GameState, battle: &Battle) -> Result<(), RepositoryError> {
    if !battle.finished() {
        return Ok(());
    }
    beat(state, "complete")?;
    Ok(())
}
fn policy(
    current: &Checkpoint,
    state: &GameState,
    battle: &Battle,
    actor: u32,
    operation: OperationId,
    ordinal: u32,
) -> Result<Action, RepositoryError> {
    use df_combat::{
        candidates::CandidateContext,
        ranking::{KnowledgeObserver, RankingObservation, TacticalKnowledge, UtilityPolicy},
        tactical::TacticContext,
    };
    let basis = current.basis();
    let pins = current.pins();
    let observer = KnowledgeObserver::Entity(canonical(state, actor)?);
    let definition = model::content("tactical-srd521-cover-policy")?;
    let observation = RankingObservation {
        basis: &basis,
        pins,
        observer,
        logical_time: state.logical_time,
        cause: operation,
        policy: &definition,
    };
    let knowledge = TacticalKnowledge {
        basis: &basis,
        pins,
        observer,
        grants: &[],
        beliefs: &[],
        npcs: &[],
    };
    let utility = UtilityPolicy {
        basis: &basis,
        pins,
        definition: &definition,
        criteria: &[],
    };
    let source = rule()?;
    noted(
        "enemy-policy-none",
        noted(
            "enemy-policy",
            df_combat::tactical::choose(
                battle,
                actor,
                TacticContext {
                    candidates: CandidateContext {
                        basis: &basis,
                        pins,
                    },
                    observation: &observation,
                    knowledge: &knowledge,
                    policy: &utility,
                    draws: binding(operation, &source, ordinal)?,
                },
            )
            .map_err(bad),
        )?
        .ok_or(RepositoryError::InvalidCandidate),
    )
}
fn enemy_turns(
    current: &Checkpoint,
    state: &mut GameState,
    mut battle: Battle,
    operation: OperationId,
    draws: &mut Vec<ActualDraw>,
    outcomes: &mut Vec<rpc::CombatOutcome>,
    supplier: DiceSource<'_>,
) -> Result<(), RepositoryError> {
    for _ in 0..128 {
        if battle.finished() {
            return finish_if_done(state, &battle);
        }
        let actor = if let Some(&reactor) = battle.reaction_offers().first() {
            reactor
        } else {
            battle
                .active_actor()
                .ok_or(RepositoryError::InvalidCandidate)?
        };
        if battle.needs_death_save() {
            battle = apply(
                state,
                &battle,
                actor,
                &Action::DeathSave,
                operation,
                (draws, outcomes),
                supplier,
            )?;
            continue;
        }
        if battle.actor(actor).map_err(bad)?.health().incapacitated() {
            battle = apply(
                state,
                &battle,
                actor,
                &Action::EndTurn,
                operation,
                (draws, outcomes),
                supplier,
            )?;
            continue;
        }
        if battle.actor(actor).map_err(bad)?.side() == Side::Heroes {
            return Ok(());
        }
        let action = policy(
            current,
            state,
            &battle,
            actor,
            operation,
            u32::try_from(draws.len()).map_err(bad)?,
        )?;
        battle = apply(
            state,
            &battle,
            actor,
            &action,
            operation,
            (draws, outcomes),
            supplier,
        )?;
    }
    Err(RepositoryError::Capacity)
}
pub(super) fn start(
    current: &Checkpoint,
    command: &CommandInput,
    state: &mut GameState,
    draws: &mut Vec<ActualDraw>,
    outcomes: &mut Vec<rpc::CombatOutcome>,
    supplier: DiceSource<'_>,
    scene: &pack::Scene,
) -> Result<(), RepositoryError> {
    let location = match &scene.setting {
        Some(setting) => setting.place_id.clone(),
        None if pack::active().is_some_and(|loaded| loaded.pack.id == "harbor-journey") => {
            "harbor-journey".to_owned()
        }
        None => {
            return noted(
                "fight-scene-location",
                Err(RepositoryError::InvalidCandidate),
            );
        }
    };
    let location = staged_location(location);
    let bytes = noted(
        "fight-scene-bytes",
        SCENES
            .iter()
            .find(|(id, _)| *id == location)
            .map(|(_, bytes)| *bytes)
            .ok_or(RepositoryError::InvalidCandidate),
    )?;
    let scene: tactical_rules::BattleSceneData =
        noted("fight-scene-parse", serde_json::from_slice(bytes).map_err(bad))?;
    if scene.layouts.is_empty() {
        return noted(
            "fight-scene-layouts",
            Err(RepositoryError::InvalidCandidate),
        );
    }
    let index = command
        .operation
        .as_bytes()
        .iter()
        .fold(0_usize, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(usize::from(*byte))
        })
        % scene.layouts.len();
    let layout = noted(
        "fight-scene-layout",
        scene
            .layouts
            .get(index)
            .ok_or(RepositoryError::InvalidCandidate),
    )?;
    let map = noted("fight-scene-map", scene_map(bytes, &layout.id))?;
    let mut specs = Vec::new();
    let mut actors = Vec::new();
    for (index, entity) in noted(
        "fight-hero-membership",
        participants(current)
            .into_iter()
            .map(|participant| {
                participant
                    .character
                    .ok_or(RepositoryError::InvalidCandidate)
            })
            .collect::<Result<Vec<_>, _>>(),
    )?
    .into_iter()
    .enumerate()
    {
        let character = noted(
            "fight-hero-character",
            state
                .characters
                .iter()
                .find(|character| character.entity == entity)
                .ok_or(RepositoryError::InvalidCandidate),
        )?;
        let cell = *noted(
            "fight-hero-spawn",
            map.hero_spawns()
                .get(index)
                .ok_or(RepositoryError::InvalidCandidate),
        )?;
        let choices = character
            .choices
            .iter()
            .filter(|choice| choice.offer.as_str() != "name")
            .map(|choice| (choice.offer.as_str(), choice.selected.as_str()))
            .collect::<Vec<_>>();
        let legal = noted(
            "fight-hero-build",
            pack::validate_character(&name(state, character.entity)?, &choices).map_err(bad),
        )?;
        let id = u32::try_from(index + 1).map_err(bad)?;
        let spec = if legal
            .choices
            .iter()
            .any(|(key, value)| key == "array" && value == "dexterous")
        {
            let borrowed = legal
                .choices
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str()))
                .collect::<Vec<_>>();
            noted(
                "fight-hero-spec",
                tactical_rules::validate_archer(&legal.name, &borrowed).map_err(bad),
            )?
            .combatant(id, cell)
        } else {
            noted(
                "fight-hero-spec",
                CombatantSpec::melee_hero(&legal, id, cell).map_err(bad),
            )?
        };
        specs.push(spec);
        actors.push((id, entity));
    }
    for (index, (who, source_id, ammunition)) in
        noted("fight-enemy-roster", enemies())?.into_iter().enumerate()
    {
        let id = u32::try_from(index + 3).map_err(bad)?;
        let cell = *noted(
            "fight-enemy-spawn",
            map.enemy_spawns()
                .get(index)
                .ok_or(RepositoryError::InvalidCandidate),
        )?;
        let spec = noted(
            "fight-enemy-spec",
            CombatantSpec::creature(id, &source_id, cell, ammunition).map_err(bad),
        )?;
        if state.entities.iter().any(|entity| entity.id == who) {
            return noted(
                "fight-enemy-duplicate",
                Err(RepositoryError::InvalidCandidate),
            );
        }
        state.entities.push(WorldEntity {
            id: who,
            definition: model::content("bandit")?,
            location: Some(room_entity()?),
            position: Some(Position {
                x: i64::from(cell.0) * 5,
                y: i64::from(cell.1) * 5,
                z: 0,
            }),
            identity_revision: model::label("tactical-srd521-1")?,
        });
        for (key, value) in [
            ("hit-points", i64::from(spec.hit_points)),
            ("armor-class", i64::from(spec.armor_class)),
            ("strength", i64::from(spec.abilities[0])),
            ("dexterity", i64::from(spec.abilities[1])),
            ("constitution", i64::from(spec.abilities[2])),
            ("intelligence", i64::from(spec.abilities[3])),
            ("wisdom", i64::from(spec.abilities[4])),
            ("charisma", i64::from(spec.abilities[5])),
            ("second-wind", 0),
            ("unconscious", 0),
            ("prone", 0),
            ("held-weapon", 1),
            ("action-used", 0),
            ("bonus-used", 0),
        ] {
            add_resource(state, who, key, value)?;
        }
        specs.push(spec);
        actors.push((id, who));
    }
    // Participant/challenge admission is performed before the first initiative draw.
    noted(
        "fight-admission",
        encounter_admission::admission(
            current,
            command,
            state,
            &actors.iter().map(|(_, who)| *who).collect::<Vec<_>>(),
        )
        .map_err(bad),
    )?;
    let mut result = noted(
        "fight-begin",
        resolve_with_draws(command.operation, draws, supplier, |binding, records| {
            tactical_rules::begin(map.clone(), specs.clone(), binding, records)
        }),
    )?;
    for (id, who) in &actors {
        if !state
            .characters
            .iter()
            .any(|character| character.entity == *who)
        {
            continue;
        }
        let arrows = state
            .inventory
            .iter()
            .find(|item| item.owner == *who && item.origin.entry.as_str() == "arrow")
            .map_or(0, |item| item.quantity);
        result.battle = noted(
            "fight-hero-resources",
            result
                .battle
                .clone()
                .with_hero_resources(
                    *id,
                    u16::try_from(value(state, *who, "hit-points")?).map_err(bad)?,
                    u16::try_from(arrows).map_err(bad)?,
                    u8::try_from(value(state, *who, "second-wind")?).map_err(bad)?,
                )
                .map_err(bad),
        )?;
        let quantities = result
            .battle
            .actor(*id)
            .map_err(bad)?
            .spec()
            .weapons
            .iter()
            .map(|weapon| {
                let item = state
                    .inventory
                    .iter()
                    .find(|item| item.owner == *who && item.origin.entry.as_str() == weapon.id());
                Ok((
                    *weapon,
                    u16::try_from(item.map_or(0, |item| item.quantity)).map_err(bad)?,
                ))
            })
            .collect::<Result<Vec<_>, RepositoryError>>()?;
        result.battle = noted(
            "fight-hero-weapons",
            result
                .battle
                .clone()
                .with_hero_weapons(*id, &quantities)
                .map_err(bad),
        )?;
    }
    state.encounters.push(EncounterState {
        id: RecordId::from_bytes(&ENCOUNTER).map_err(bad)?,
        definition: model::content("combat")?,
        participants: actors.iter().map(|(_, who)| *who).collect(),
        turn_order: Vec::new(),
        active_turn: None,
        objectives: vec![model::content("defend-courier")?],
        combat_policy: model::content("tactical-srd521-cover-policy")?,
        tactical: Some(TacticalSnapshot {
            schema: 1,
            source_revision: model::label(tactical_rules::SOURCE_REVISION)?,
            scene_digest: ContentDigest(Sha256::digest(bytes).into()),
            scene: bytes.to_vec(),
            layout: model::label(&layout.id)?,
            actors,
            state: result.battle.snapshot().map_err(bad)?,
        }),
    });
    noted("fight-store", store(state, &result.battle))?;
    noted("fight-beat", beat(state, "combat"))?;
    noted(
        "fight-enemy-turns",
        enemy_turns(
            current,
            state,
            result.battle,
            command.operation,
            draws,
            outcomes,
            supplier,
        ),
    )
}
fn weapon(actor: &tactical_rules::Combatant, weapon: Weapon) -> AttackUse {
    AttackUse {
        weapon,
        ranged: weapon.ranged_weapon(),
        hands: if weapon == Weapon::UnarmedStrike {
            0
        } else if weapon.two_handed() {
            2
        } else {
            1
        },
        finesse_dexterity: weapon.finesse(),
        knockout: false,
        savage_attacker: false,
        use_mastery: actor.spec().masteries.contains(&weapon),
    }
}
pub(in crate::gameplay) fn combat_action(kind: rpc::GameplayActionKind) -> bool {
    matches!(
        kind,
        rpc::GameplayActionKind::GreatswordAttack
            | rpc::GameplayActionKind::Move
            | rpc::GameplayActionKind::Dash
            | rpc::GameplayActionKind::Dodge
            | rpc::GameplayActionKind::Disengage
            | rpc::GameplayActionKind::SecondWind
            | rpc::GameplayActionKind::EndTurn
            | rpc::GameplayActionKind::OpportunityAttack
            | rpc::GameplayActionKind::DeclineReaction
            | rpc::GameplayActionKind::Stand
            | rpc::GameplayActionKind::PickUpWeapon
    )
}
fn options(
    state: &GameState,
    actor: u32,
    kind: rpc::GameplayActionKind,
) -> Result<Vec<(String, Action, String)>, RepositoryError> {
    use rpc::GameplayActionKind as K;
    let battle = read(state)?;
    let combatant = battle.actor(actor).map_err(bad)?;
    let source = rule()?;
    let context = binding(OperationId::from_bytes(&[1; 16]).map_err(bad)?, &source, 0)?;
    let mut candidates = Vec::new();
    if let Some(&reactor) = battle.reaction_offers().first() {
        if reactor != actor {
            return Ok(Vec::new());
        }
        if kind == K::DeclineReaction {
            candidates.push((
                String::new(),
                Action::Opportunity { attack: None },
                "Decline opportunity attack".to_owned(),
            ));
        }
        if kind == K::OpportunityAttack {
            for held in combatant
                .held_weapons()
                .iter()
                .copied()
                .filter(|held| !held.ranged_weapon())
                .chain([Weapon::UnarmedStrike])
            {
                let usage = weapon(combatant, held);
                candidates.push((
                    held.id().to_owned(),
                    Action::Opportunity {
                        attack: Some(usage),
                    },
                    format!("Opportunity attack · {}", weapon_display_name(held.id())),
                ));
            }
        }
    } else if !battle.finished() && battle.active_actor() == Some(actor) {
        match kind {
            K::Move => {
                for cell in battle.reachable_cells(actor).map_err(bad)? {
                    if cell == combatant.cell() {
                        continue;
                    }
                    let path = battle
                        .shortest_path(actor, cell, Some(combatant.movement_remaining_ft()))
                        .map_err(bad)?;
                    candidates.push((
                        format!("{}-{}", cell.0, cell.1),
                        Action::Move(path),
                        format!("Column {}, row {}", cell.0 + 1, cell.1 + 1),
                    ));
                }
            }
            K::GreatswordAttack => {
                for held in combatant
                    .spec()
                    .weapons
                    .iter()
                    .copied()
                    .chain([Weapon::UnarmedStrike])
                {
                    let modes: &[bool] = if held.ranged_weapon() {
                        &[true]
                    } else if held.range().is_some() {
                        &[false, true]
                    } else {
                        &[false]
                    };
                    for &ranged in modes {
                        for savage in [false, true] {
                            for mastery in [false, true] {
                                let mut usage = weapon(combatant, held);
                                usage.ranged = ranged;
                                usage.savage_attacker = savage;
                                usage.use_mastery = mastery;
                                for target in battle.legal_targets(actor, usage).map_err(bad)? {
                                    let label = format!(
                                        "{}{} → {}{}{}",
                                        weapon_display_name(held.id()),
                                        if ranged && !held.ranged_weapon() {
                                            " (throw)"
                                        } else {
                                            ""
                                        },
                                        name(state, canonical(state, target)?)?,
                                        if mastery { " · mastery" } else { "" },
                                        if savage { " · Savage Attacker" } else { "" }
                                    );
                                    for (spend, suffix, spend_label) in [
                                        (AttackSpend::Action, "action", ""),
                                        (AttackSpend::LightBonus, "bonus", " · Light bonus attack"),
                                        (AttackSpend::Nick, "nick", " · Nick extra attack"),
                                    ] {
                                        candidates.push((
                                            format!(
                                                "{}-{}-{}-{}-{}-{suffix}",
                                                held.id(),
                                                u8::from(ranged),
                                                target,
                                                u8::from(savage),
                                                u8::from(mastery)
                                            ),
                                            Action::Attack {
                                                target,
                                                weapon: usage,
                                                spend,
                                            },
                                            format!("{label}{spend_label}"),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            K::Stand => candidates.push((String::new(), Action::Stand, "Stand up".to_owned())),
            K::PickUpWeapon => {
                for held in &combatant.spec().weapons {
                    candidates.push((
                        held.id().to_owned(),
                        Action::PickUpWeapon(*held),
                        format!("Pick up {}", weapon_display_name(held.id())),
                    ));
                }
            }
            K::Dash => candidates.push((String::new(), Action::Dash, "Dash".to_owned())),
            K::Dodge => candidates.push((String::new(), Action::Dodge, "Dodge".to_owned())),
            K::Disengage => {
                candidates.push((String::new(), Action::Disengage, "Disengage".to_owned()))
            }
            K::SecondWind => {
                candidates.push((String::new(), Action::SecondWind, "Second Wind".to_owned()))
            }
            K::EndTurn => candidates.push((String::new(), Action::EndTurn, "End turn".to_owned())),
            _ => {}
        }
    }
    candidates.retain(|(_, action, _)| {
        matches!(
            tactical_rules::resolve(&battle, actor, action, context, &[]),
            Ok(_) | Err(RuleError::MissingDraw { .. })
        )
    });
    if kind == K::GreatswordAttack {
        let weapons = &combatant.spec().weapons;
        let mut ranked = candidates
            .into_iter()
            .map(|candidate| {
                let rank = if let Action::Attack {
                    target,
                    weapon,
                    spend,
                } = &candidate.1
                {
                    let (advantage, disadvantage) = battle
                        .attack_reasons(actor, *target, *weapon)
                        .map_err(bad)?;
                    let cover = battle.attack_cover(actor, *target).map_err(bad)?;
                    let cover_rank = match cover {
                        tactical_rules::Cover::None => 0,
                        tactical_rules::Cover::Half => 1,
                        tactical_rules::Cover::ThreeQuarters => 2,
                        tactical_rules::Cover::Total => 3,
                    };
                    let weapon_rank = weapons
                        .iter()
                        .position(|held| held.id() == weapon.weapon.id())
                        .unwrap_or(weapons.len());
                    let variant_rank = weapon.use_mastery || weapon.savage_attacker;
                    let spend_rank = match spend {
                        AttackSpend::Action => 0,
                        AttackSpend::LightBonus => 1,
                        AttackSpend::Nick => 2,
                        AttackSpend::Multiattack => 3,
                    };
                    (
                        !disadvantage.is_empty() && advantage.is_empty(),
                        cover_rank,
                        weapon_rank,
                        variant_rank,
                        spend_rank,
                    )
                } else {
                    (false, 0, 0, false, 0)
                };
                Ok((rank, candidate))
            })
            .collect::<Result<Vec<_>, RepositoryError>>()?;
        ranked.sort_by_key(|(rank, _)| *rank);
        candidates = ranked.into_iter().map(|(_, candidate)| candidate).collect();
    }
    if candidates.len() > 256 {
        return Err(RepositoryError::Capacity);
    }
    Ok(candidates)
}
pub(in crate::gameplay) fn groups(
    current: &Checkpoint,
    member: MemberId,
    kind: rpc::GameplayActionKind,
) -> Result<Vec<rpc::JourneyGroup>, RepositoryError> {
    if !matches!(
        kind,
        rpc::GameplayActionKind::Move
            | rpc::GameplayActionKind::GreatswordAttack
            | rpc::GameplayActionKind::OpportunityAttack
            | rpc::GameplayActionKind::PickUpWeapon
    ) {
        return Ok(Vec::new());
    }
    let actor = actor_id(current.state(), player_entity(member, current)?)?;
    let options = options(current.state(), actor, kind)?
        .into_iter()
        .map(|(option_id, _, label)| rpc::JourneyOption {
            option_id,
            label,
            description: String::new(),
            ..Default::default()
        })
        .collect();
    Ok(vec![rpc::JourneyGroup {
        group_id: "tactical-choice".to_owned(),
        label: if kind == rpc::GameplayActionKind::Move {
            "Reachable cell"
        } else {
            "Weapon and target"
        }
        .to_owned(),
        options,
    }])
}
pub(super) fn offered(
    current: &Checkpoint,
    member: MemberId,
) -> Result<Vec<(rpc::GameplayActionKind, String)>, RepositoryError> {
    use rpc::GameplayActionKind as K;
    // The fight is a sub-machine inside the machine's Fight state: only
    // kinds the machine accepts there may be built.
    let state = super::session_machine::SessionState::of(current)?;
    let actor = actor_id(current.state(), player_entity(member, current)?)?;
    let mut offers = Vec::new();
    for (kind, label) in [
        (K::Move, "Move"),
        (K::GreatswordAttack, "Attack"),
        (K::Dash, "Dash"),
        (K::Dodge, "Dodge"),
        (K::Disengage, "Disengage"),
        (K::SecondWind, "Second Wind"),
        (K::EndTurn, "End turn"),
        (K::Stand, "Stand up"),
        (K::PickUpWeapon, "Pick up weapon"),
        (K::OpportunityAttack, "Opportunity attack"),
        (K::DeclineReaction, "Decline opportunity attack"),
    ] {
        if super::session_machine::kind_accepted(state, kind)
            && !options(current.state(), actor, kind)?.is_empty()
        {
            offers.push((kind, label.to_owned()));
        }
    }
    Ok(offers)
}
pub(super) fn stage(
    current: &Checkpoint,
    command: &CommandInput,
    kind: rpc::GameplayActionKind,
    state: &mut GameState,
    draws: &mut Vec<ActualDraw>,
    outcomes: &mut Vec<rpc::CombatOutcome>,
    supplier: DiceSource<'_>,
) -> Result<(), RepositoryError> {
    let GameCommand::ProposeAction {
        actor: who,
        choices,
        targets,
        ..
    } = &command.command
    else {
        return Err(RepositoryError::InvalidCandidate);
    };
    let battle = read(state)?;
    let actor = actor_id(state, *who)?;
    if kind == rpc::GameplayActionKind::EndTurn && !choices.is_empty() {
        return Err(RepositoryError::InvalidCandidate);
    }
    if phase(current)? == rpc::JourneyPhase::Complete {
        if kind != rpc::GameplayActionKind::EndTurn || !choices.is_empty() || !dying(state)? {
            return Err(RepositoryError::InvalidCandidate);
        }
        return aftermath(state, command.operation, draws, outcomes, supplier);
    }
    let candidates = options(state, actor, kind)?;
    let selected = match choices.as_slice() {
        [] => candidates.iter().find(|(id, _, _)| id.is_empty()),
        [(group, id)] if group.as_str() == "tactical-choice" => candidates
            .iter()
            .find(|(offered, _, _)| offered == id.as_str()),
        _ => None,
    }
    .ok_or(RepositoryError::InvalidCandidate)?;
    if !targets.is_empty() {
        let expected = match &selected.1 {
            Action::Attack { target, .. } => canonical(state, *target)?,
            _ => return Err(RepositoryError::InvalidCandidate),
        };
        if targets.as_slice() != [expected] {
            return Err(RepositoryError::InvalidCandidate);
        }
    }
    let next = apply(
        state,
        &battle,
        actor,
        &selected.1,
        command.operation,
        (draws, outcomes),
        supplier,
    )?;
    enemy_turns(
        current,
        state,
        next,
        command.operation,
        draws,
        outcomes,
        supplier,
    )
}
pub(super) fn validate_input(
    current: &Checkpoint,
    member: MemberId,
    action: &ContentReference,
    targets: &[EntityId],
) -> Result<(), RepositoryError> {
    let kind = wire::action_kind(action.entry.as_str()).ok_or(RepositoryError::InvalidCandidate)?;
    if !offered(current, member)?
        .iter()
        .any(|(candidate, _)| *candidate == kind)
        || targets.len() > 1
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    if let Some(target) = targets.first() {
        let actor = actor_id(current.state(), player_entity(member, current)?)?;
        if !options(current.state(),actor,kind)?.iter().any(|(_,action,_)| matches!(action,Action::Attack {target:id,..} if canonical(current.state(),*id).is_ok_and(|who| who == *target))) { return Err(RepositoryError::InvalidCandidate); }
    }
    Ok(())
}
pub(super) fn victory(state: &GameState) -> Result<bool, RepositoryError> {
    if let Some(encounter) = state.encounters.first() {
        if encounter
            .objectives
            .iter()
            .any(|objective| objective.entry.as_str() == "combat-victory")
        {
            return Ok(true);
        }
        if encounter
            .objectives
            .iter()
            .any(|objective| objective.entry.as_str() == "combat-defeat")
        {
            return Ok(false);
        }
    }
    match read(state)?.winner() {
        Some(Side::Heroes) => Ok(true),
        Some(Side::Enemies) => Ok(false),
        _ => Err(RepositoryError::InvalidCandidate),
    }
}

pub(super) fn validate_encounter(current: &Checkpoint) -> Result<(), RepositoryError> {
    let state = current.state();
    let battle = read(state)?;
    let saved = snapshot(state)?;
    let encounter = state
        .encounters
        .first()
        .ok_or(RepositoryError::InvalidCandidate)?;
    let terminal = encounter
        .objectives
        .iter()
        .any(|objective| matches!(objective.entry.as_str(), "combat-victory" | "combat-defeat"));
    let participants = saved.actors.iter().map(|(_, who)| *who).collect::<Vec<_>>();
    let order = battle
        .initiative_order()
        .iter()
        .map(|id| canonical(state, *id))
        .collect::<Result<Vec<_>, _>>()?;
    if encounter.id != RecordId::from_bytes(&ENCOUNTER).map_err(bad)?
        || encounter.definition != model::content("combat")?
        || encounter.combat_policy != model::content("tactical-srd521-cover-policy")?
        || encounter.participants != participants
        || encounter.turn_order != order
        || (!battle.finished()
            && !terminal
            && encounter.active_turn
                != battle
                    .active_actor()
                    .map(|id| canonical(state, id))
                    .transpose()?)
        || ((battle.finished() || terminal) && encounter.active_turn.is_some())
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    for actor in battle.combatants() {
        let who = canonical(state, actor.id())?;
        let world = state
            .entities
            .iter()
            .find(|entity| entity.id == who)
            .ok_or(RepositoryError::InvalidCandidate)?;
        if world.location != Some(room_entity()?)
            || world.position
                != Some(Position {
                    x: i64::from(actor.cell().0) * 5,
                    y: i64::from(actor.cell().1) * 5,
                    z: 0,
                })
            || value(state, who, "hit-points")? != u32::from(actor.health().hit_points)
            || value(state, who, "armor-class")? != u32::from(actor.spec().armor_class)
            || value(state, who, "unconscious")? != u32::from(actor.health().unconscious)
            || value(state, who, "prone")? != u32::from(actor.health().prone)
            || value(state, who, "second-wind")? != u32::from(actor.spec().second_wind_uses)
            || value(state, who, "action-used")? != u32::from(actor.budget().action_used)
            || value(state, who, "bonus-used")? != u32::from(actor.budget().bonus_action_used)
            || value(state, who, "held-weapon")? != u32::from(!actor.held_weapons().is_empty())
            || [
                "strength",
                "dexterity",
                "constitution",
                "intelligence",
                "wisdom",
                "charisma",
            ]
            .iter()
            .enumerate()
            .any(|(index, key)| {
                !value(state, who, key)
                    .is_ok_and(|value| value == u32::from(actor.spec().abilities[index]))
            })
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        if actor.side() == Side::Heroes {
            for (entry, quantity) in [
                ("arrow", u64::from(actor.arrows())),
                ("crossbow-bolt", u64::from(actor.bolts())),
            ]
            .into_iter()
            .chain(
                actor
                    .spec()
                    .weapons
                    .iter()
                    .map(|weapon| (weapon.id(), u64::from(actor.weapon_count(*weapon)))),
            ) {
                let canonical = state
                    .inventory
                    .iter()
                    .find(|item| item.owner == who && item.origin.entry.as_str() == entry)
                    .map_or(0, |item| item.quantity);
                if canonical != quantity {
                    return Err(RepositoryError::InvalidCandidate);
                }
            }
        }
    }
    Ok(())
}
pub(super) fn validate_phase(
    current: &Checkpoint,
    phase: rpc::JourneyPhase,
    prefix: &[ContentReference],
    start: &GameFact,
    dialogue: &GameFact,
) -> Result<(), RepositoryError> {
    validate_encounter(current)?;
    let state = current.state();
    if start.audience != AudienceScope::Shared
        || start.revision <= dialogue.revision
        || !matches!(&start.value, FactValue::ContentEvent { subjects, .. } if subjects.is_empty())
        || !objective_outcome::defend_history(state)?
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let defend = model::content("defend-courier")?;
    if phase == rpc::JourneyPhase::Combat {
        if read(state)?.finished()
            || state.narrative.completed_beats != prefix
            || state
                .encounters
                .first()
                .is_none_or(|encounter| encounter.objectives != [defend.clone()])
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        return Ok(());
    }
    let victory = victory(state)?;
    let end = completion_source(state, start, victory, true)?;
    let mut completed = prefix.to_vec();
    completed.push(model::content("combat")?);
    if state.narrative.completed_beats != completed
        || state.narrative.active_beats != [model::content("complete")?]
        || end.revision <= start.revision
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    Ok(())
}
pub(super) fn completion_source<'a>(
    state: &'a GameState,
    start: &GameFact,
    victory: bool,
    require_narrative: bool,
) -> Result<&'a GameFact, RepositoryError> {
    let source_rule = rule()?;
    for decision in &state.decisions {
        if decision.source_policy.as_str() != THREAD_POLICY
            || decision.revision <= start.revision
            || accepted(decision)?.phase != rpc::JourneyPhase::Complete as i32
        {
            continue;
        }
        let Some(event) = state
            .facts
            .iter()
            .find(|fact| decision.facts.last() == Some(&fact.id))
        else {
            continue;
        };
        if event.audience != AudienceScope::Shared
            || (require_narrative && !state.narrative.accepted_facts.contains(&event.id))
            || !matches!(&event.value, FactValue::ContentEvent { definition, subjects }
                if wire::action_kind(definition.entry.as_str()).is_some_and(combat_action) && subjects.is_empty())
        {
            continue;
        }
        super::narrative_phase::validate_terminal_cause(state, event, decision)?;
        let battle = read(state)?;
        let defeated = battle
            .combatants()
            .iter()
            .filter(|actor| actor.side() == if victory { Side::Enemies } else { Side::Heroes })
            .all(|actor| {
                let Ok(who) = canonical(state, actor.id()) else {
                    return false;
                };
                state
                    .facts
                    .iter()
                    .filter(|fact| {
                        fact.revision < event.revision
                            || (fact.revision == event.revision && fact.ordinal < event.ordinal)
                    })
                    .filter_map(|fact| match &fact.value {
                        FactValue::ResourceChanged {
                            entity,
                            resource,
                            after,
                            source,
                            ..
                        } if *entity == who
                            && resource.as_str() == "hit-points"
                            && *source == source_rule =>
                        {
                            Some(*after)
                        }
                        _ => None,
                    })
                    .next_back()
                    == Some(0)
            });
        let causal = state.facts.iter().any(|fact| fact.operation == event.operation && fact.revision == event.revision
            && decision.facts.contains(&fact.id) && fact.ordinal < event.ordinal
            && matches!(&fact.value, FactValue::ResourceChanged { resource, after: 0, source, .. }
                if resource.as_str() == "hit-points" && *source == source_rule));
        if defeated && causal {
            return Ok(event);
        }
    }
    Err(RepositoryError::InvalidCandidate)
}

/// A hero is at 0 hit points and not dead: dying (Death Saving Throws each round) or Stable
/// (regains 1 hit point after 1d4 hours). Until each is resolved the party cannot rest or move on,
/// and the only offer is the one that lets time pass.
pub(in crate::gameplay) fn dying(state: &GameState) -> Result<bool, RepositoryError> {
    Ok(read(state)?
        .combatants()
        .iter()
        .any(|actor| actor.side() == Side::Heroes && at_zero(actor)))
}
fn at_zero(actor: &tactical_rules::Combatant) -> bool {
    actor.health().hit_points == 0 && !actor.health().dead
}
/// True when a hero still needs Death Saving Throws, as opposed to waiting out stable recovery.
pub(in crate::gameplay) fn needs_saves(state: &GameState) -> Result<bool, RepositoryError> {
    Ok(read(state)?
        .combatants()
        .iter()
        .any(|actor| actor.side() == Side::Heroes && at_zero(actor) && !actor.health().stable))
}
/// How a fight ended, as the closing scenes tell it. Victory is the heroes standing at the end;
/// it is costly when any hero is down or dead. Defeat leaves survivors unless every hero is dead.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum FightOutcome {
    Won,
    WonCostly,
    Lost,
    LostAll,
}
impl FightOutcome {
    pub(in crate::gameplay) const ALL: [Self; 4] =
        [Self::Won, Self::WonCostly, Self::Lost, Self::LostAll];
    pub(in crate::gameplay) fn classify(won: bool, any_down: bool, any_alive: bool) -> Self {
        match (won, any_down, any_alive) {
            (true, false, _) => Self::Won,
            (true, true, _) => Self::WonCostly,
            (false, _, true) => Self::Lost,
            (false, _, false) => Self::LostAll,
        }
    }
    /// The suffix of this outcome's closing scene variants.
    pub(in crate::gameplay) fn slug(self) -> &'static str {
        match self {
            Self::Won => "won",
            Self::WonCostly => "won-costly",
            Self::Lost => "lost",
            Self::LostAll => "lost-all",
        }
    }
}
/// The per-hero dead and down flags of the tactical snapshot decide the class.
pub(in crate::gameplay) fn outcome(state: &GameState) -> Result<FightOutcome, RepositoryError> {
    let won = victory(state)?;
    let battle = read(state)?;
    let mut down = false;
    let mut survivors = false;
    for actor in battle
        .combatants()
        .iter()
        .filter(|actor| actor.side() == Side::Heroes)
    {
        down |= actor.health().dead || actor.health().hit_points == 0;
        survivors |= !actor.health().dead;
    }
    Ok(FightOutcome::classify(won, down, survivors))
}
pub(super) fn aftermath(
    state: &mut GameState,
    operation: OperationId,
    draws: &mut Vec<ActualDraw>,
    outcomes: &mut Vec<rpc::CombatOutcome>,
    supplier: DiceSource<'_>,
) -> Result<(), RepositoryError> {
    let mut battle = read(state)?;
    if !needs_saves(state)? {
        // Only Stable heroes remain: 1d4 hours pass for each (SRD 5.2.1 p18).
        let stable = battle
            .combatants()
            .iter()
            .filter(|actor| actor.side() == Side::Heroes && at_zero(actor))
            .map(|actor| actor.id())
            .collect::<Vec<_>>();
        for actor in stable {
            let before = outcomes.len();
            battle = apply(
                state,
                &battle,
                actor,
                &Action::StableRecovery,
                operation,
                (draws, outcomes),
                supplier,
            )?;
            let hours = outcomes[before..]
                .iter()
                .map(|outcome| u64::from(outcome.recovery_hours))
                .sum::<u64>();
            state.logical_time.ticks = state
                .logical_time
                .ticks
                .checked_add(hours * 3600)
                .ok_or(RepositoryError::Capacity)?;
        }
        return Ok(());
    }
    let count = battle.combatants().len();
    for _ in 0..count {
        let actor = battle
            .active_actor()
            .ok_or(RepositoryError::InvalidCandidate)?;
        if battle.needs_death_save() {
            battle = apply(
                state,
                &battle,
                actor,
                &Action::DeathSave,
                operation,
                (draws, outcomes),
                supplier,
            )?;
        }
        battle = apply(
            state,
            &battle,
            actor,
            &Action::EndTurn,
            operation,
            (draws, outcomes),
            supplier,
        )?;
    }
    Ok(())
}
pub(super) fn rest(state: &mut GameState, operation: OperationId) -> Result<(), RepositoryError> {
    if dying(state)? {
        return Err(RepositoryError::InvalidCandidate);
    }
    let mut battle = read(state)?;
    let source = rule()?;
    let ids = battle
        .combatants()
        .iter()
        .filter(|actor| {
            actor.side() == Side::Heroes && actor.health().hit_points > 0 && !actor.health().dead
        })
        .map(|actor| actor.id())
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Err(RepositoryError::InvalidCandidate);
    }
    for actor in ids {
        battle = tactical_rules::resolve(
            &battle,
            actor,
            &Action::ShortRest {
                elapsed_seconds: pack::rest().short_rest_seconds,
            },
            binding(operation, &source, 0)?,
            &[],
        )
        .map_err(bad)?
        .battle;
    }
    store(state, &battle)
}

pub(super) fn initiative(
    current: &Checkpoint,
) -> Result<Vec<rpc::InitiativeRoll>, RepositoryError> {
    let state = current.state();
    let battle = read(state)?;
    let start = state.facts.iter().find(|fact| matches!(&fact.value, FactValue::ContentEvent {definition, ..} if definition.entry.as_str() == "defend-courier")).ok_or(RepositoryError::InvalidCandidate)?;
    let decision = state
        .decisions
        .iter()
        .find(|decision| {
            decision.operation == start.operation
                && decision.revision == start.revision
                && decision.facts.last() == Some(&start.id)
        })
        .ok_or(RepositoryError::InvalidCandidate)?;
    let mut groups = Vec::new();
    let mut ordinal = 0;
    let mut rolls = Vec::new();
    let source = rule()?;
    for actor in battle.combatants() {
        let draw_ordinal = if actor.side() == Side::Enemies {
            if let Some((_, saved)) = groups.iter().find(|(kind, _)| *kind == actor.spec().kind) {
                *saved
            } else {
                groups.push((actor.spec().kind, ordinal));
                ordinal += 1;
                ordinal - 1
            }
        } else {
            ordinal += 1;
            ordinal - 1
        };
        let draw = state
            .draws
            .iter()
            .find(|draw| {
                draw.operation == start.operation
                    && draw.ordinal == draw_ordinal
                    && draw.sides == 20
                    && draw.source == source
                    && decision.draws.contains(&draw_ordinal)
            })
            .ok_or(RepositoryError::InvalidCandidate)?;
        let who = canonical(state, actor.id())?;
        let modifier = rules::ability_modifier(actor.spec().abilities[1]);
        let order = battle
            .initiative_order()
            .iter()
            .position(|id| *id == actor.id())
            .ok_or(RepositoryError::InvalidCandidate)?;
        rolls.push(rpc::InitiativeRoll {
            participant_id: who.as_bytes().to_vec(),
            name: name(state, who)?,
            die: draw.value,
            modifier,
            total: draw.value as i32 + modifier,
            order: u32::try_from(order + 1).map_err(bad)?,
        });
    }
    let mut expected = rolls.clone();
    expected.sort_by_key(|roll| std::cmp::Reverse(roll.total));
    if expected
        .iter()
        .enumerate()
        .any(|(index, roll)| roll.order != index as u32 + 1)
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    rolls.sort_by_key(|roll| roll.order);
    Ok(rolls)
}

pub(in crate::gameplay) fn project_member(
    state: &GameState,
    who: EntityId,
    view: &mut rpc::PartyMemberView,
) -> Result<(), RepositoryError> {
    let battle = read(state)?;
    let actor = battle.actor(actor_id(state, who)?).map_err(bad)?;
    view.cell = Some(rpc::GridCell {
        col: u32::from(actor.cell().0),
        row: u32::from(actor.cell().1),
    });
    view.side = if actor.side() == Side::Heroes { 1 } else { 2 };
    view.maximum_hit_points = u32::from(actor.health().maximum);
    view.conditions = actor.conditions().into_iter().map(str::to_owned).collect();
    let portrait = if actor.side() == Side::Heroes {
        state
            .characters
            .iter()
            .find(|character| character.entity == who)
            .and_then(|character| {
                character
                    .choices
                    .iter()
                    .find(|choice| choice.offer.as_str() == "portrait")
            })
            .map(|choice| choice.selected.as_str())
            .unwrap_or("portrait-hero-1")
            .to_owned()
    } else {
        use tactical_rules::CreatureKind as C;
        let source = match actor.spec().kind {
            C::Bandit => "bandit",
            C::Cultist => "cultist",
            C::Guard => "guard",
            C::Goblin => "goblin-warrior",
            C::Kobold => "kobold-warrior",
            C::Skeleton => "skeleton",
            C::Zombie => "zombie",
            C::Wolf => "wolf",
            C::GiantRat => "giant-rat",
            C::GiantWeasel => "giant-weasel",
            C::Mastiff => "mastiff",
            C::Scout => "scout",
            C::Hero => return Err(RepositoryError::InvalidCandidate),
        };
        format!("portrait-ant-{source}")
    };
    view.voice_set = if actor.side() == Side::Heroes {
        if matches!(portrait.as_str(), "portrait-hero-2" | "portrait-hero-3") {
            "hero-female"
        } else {
            "hero-male"
        }
    } else {
        use tactical_rules::CreatureKind as C;
        match actor.spec().kind {
            C::Goblin | C::Kobold => "small-creature",
            C::Wolf | C::GiantRat | C::GiantWeasel | C::Mastiff => "beast",
            C::Skeleton | C::Zombie => "undead",
            _ => "humanoid",
        }
    }
    .to_owned();
    view.portrait = portrait;
    view.attack_style = if actor
        .held_weapons()
        .first()
        .is_some_and(|weapon| weapon.ranged_weapon())
    {
        2
    } else {
        1
    };
    for decision in state
        .decisions
        .iter()
        .rev()
        .filter(|decision| decision.source_policy.as_str() == THREAD_POLICY)
    {
        if let Some(outcome) = accepted(decision)?
            .combat
            .iter()
            .rev()
            .find(|outcome| outcome.actor_id == who.as_bytes())
        {
            view.attack_style = outcome.attack_style;
            break;
        }
    }
    Ok(())
}
/// Splits the recorded outcomes by kind and keeps the last `window` of each, so move and
/// Death Saving Throw records never push attacks out of the log the displays already read.
pub(in crate::gameplay) fn split_outcomes(
    all: Vec<rpc::CombatOutcome>,
    window: usize,
) -> (
    Vec<rpc::CombatOutcome>,
    Vec<rpc::CombatOutcome>,
    Vec<rpc::CombatOutcome>,
) {
    let mut attacks = Vec::new();
    let mut moves = Vec::new();
    let mut saves = Vec::new();
    for outcome in all {
        match rpc::OutcomeKind::try_from(outcome.kind) {
            Ok(rpc::OutcomeKind::Move) => moves.push(outcome),
            Ok(rpc::OutcomeKind::DeathSave | rpc::OutcomeKind::Recovery) => saves.push(outcome),
            _ => attacks.push(outcome),
        }
    }
    for list in [&mut attacks, &mut moves, &mut saves] {
        let excess = list.len().saturating_sub(window);
        list.drain(..excess);
    }
    (attacks, moves, saves)
}
/// The stat-block features of the creatures in this fight that the rules do not model, in the
/// words the disclosed rules note carries. Empty when every feature is implemented.
pub(in crate::gameplay) fn unmodelled_features_note(
    state: &GameState,
) -> Result<String, RepositoryError> {
    let battle = read(state)?;
    let mut ids: Vec<&'static str> = battle
        .combatants()
        .iter()
        .filter_map(|actor| actor.spec().kind.srd_id())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let mut note = String::new();
    for id in ids {
        for row in df_rules::missing_features(id) {
            note.push_str(&format!(
                " Disclosed gap {id}: {} is not modelled ({}).",
                row.name, row.note
            ));
        }
    }
    Ok(note)
}
pub(in crate::gameplay) fn project_combat(
    current: &Checkpoint,
    own: Option<EntityId>,
    view: &mut rpc::CombatView,
) -> Result<(), RepositoryError> {
    let state = current.state();
    let battle = read(state)?;
    let saved = snapshot(state)?;
    view.layout_id = saved.layout.as_str().to_owned();
    view.location_id = SCENES
        .iter()
        .find(|(_, bytes)| *bytes == saved.scene.as_slice())
        .map(|(id, _)| (*id).to_owned())
        .ok_or(RepositoryError::InvalidCandidate)?;
    for actor in battle.combatants() {
        let health = actor.health();
        if actor.side() == Side::Heroes && health.hit_points == 0 {
            view.death_saves.push(rpc::DeathSaveState {
                participant_id: canonical(state, actor.id())?.as_bytes().to_vec(),
                name: name(state, canonical(state, actor.id())?)?,
                successes: u32::from(health.death_successes),
                failures: u32::from(health.death_failures),
                stable: health.stable,
                dead: health.dead,
            });
        }
    }
    if let Some(&reactor) = battle.reaction_offers().first() {
        view.reaction_participant_id = canonical(state, reactor)?.as_bytes().to_vec();
    }
    if let Some(actor) = battle.active_actor().filter(|_| !view.finished)
        && battle.actor(actor).map_err(bad)?.side() == Side::Heroes
    {
        view.reachable_cells = battle
            .reachable_cells(actor)
            .map_err(bad)?
            .into_iter()
            .map(|cell| rpc::GridCell {
                col: u32::from(cell.0),
                row: u32::from(cell.1),
            })
            .collect();
        for (option_id, action, _) in
            options(state, actor, rpc::GameplayActionKind::GreatswordAttack)?
        {
            let Action::Attack { target, weapon, .. } = action else {
                continue;
            };
            let source = battle.actor(actor).map_err(bad)?;
            let destination = battle.actor(target).map_err(bad)?;
            let cells = battle
                .combatants()
                .iter()
                .filter(|candidate| {
                    candidate.id() != actor && candidate.id() != target && !candidate.health().dead
                })
                .map(|candidate| candidate.cell())
                .collect::<Vec<_>>();
            let cover = battle
                .map()
                .cover_between(source.cell(), destination.cell(), &cells)
                .map_err(bad)?;
            let (advantage, disadvantage) =
                battle.attack_reasons(actor, target, weapon).map_err(bad)?;
            let distance = source.cell().distance(destination.cell());
            let range = if weapon.ranged {
                if weapon
                    .weapon
                    .range()
                    .is_some_and(|(normal, _)| distance > u32::from(normal))
                {
                    "long range"
                } else {
                    "normal range"
                }
            } else {
                "melee reach"
            };
            if own != Some(canonical(state, actor)?) {
                let participant_id = canonical(state, target)?.as_bytes().to_vec();
                if !view
                    .legal_targets
                    .iter()
                    .any(|target| target.participant_id == participant_id)
                {
                    view.legal_targets.push(rpc::LegalCombatTarget {
                        participant_id,
                        ..Default::default()
                    });
                }
                continue;
            }
            view.legal_targets.push(rpc::LegalCombatTarget {
                participant_id: canonical(state, target)?.as_bytes().to_vec(),
                weapon: weapon.weapon.id().to_owned(),
                cover: cover_label(cover).to_owned(),
                range_reason: format!("{range}: {distance} ft"),
                disadvantage: !disadvantage.is_empty() && advantage.is_empty(),
                reasons: advantage
                    .iter()
                    .map(|reason| format!("advantage: {}", reason_label(*reason)))
                    .chain(
                        disadvantage
                            .iter()
                            .map(|reason| format!("disadvantage: {}", reason_label(*reason))),
                    )
                    .collect(),
                option_id,
            });
        }
    }
    if let Some(who) = own {
        let actor = battle.actor(actor_id(state, who)?).map_err(bad)?;
        let budget = actor.budget();
        view.own_budget = Some(rpc::TurnBudget {
            movement_ft: u32::from(actor.movement_remaining_ft()),
            action: !budget.action_used,
            bonus_action: !budget.bonus_action_used,
            reaction: !budget.reaction_used,
        });
        view.own_ammunition = Some(rpc::CombatAmmunition {
            arrows: u32::from(actor.arrows()),
            bolts: u32::from(actor.bolts()),
        });
    }
    Ok(())
}

pub(in crate::gameplay) fn has_turn(
    current: &Checkpoint,
    member: MemberId,
) -> Result<bool, RepositoryError> {
    let actor = actor_id(current.state(), player_entity(member, current)?)?;
    let battle = read(current.state())?;
    Ok(battle
        .reaction_offers()
        .first()
        .copied()
        .or(battle.active_actor())
        == Some(actor))
}
pub(in crate::gameplay) fn valid_selection(
    current: &Checkpoint,
    member: MemberId,
    kind: rpc::GameplayActionKind,
    choices: &[rpc::JourneyChoice],
) -> Result<bool, RepositoryError> {
    if !combat_action(kind) {
        return Ok(choices.is_empty());
    }
    if phase(current)? == rpc::JourneyPhase::Complete {
        return Ok(kind == rpc::GameplayActionKind::EndTurn
            && choices.is_empty()
            && dying(current.state())?);
    }
    let actor = actor_id(current.state(), player_entity(member, current)?)?;
    let candidates = options(current.state(), actor, kind)?;
    Ok(match choices {
        [] => candidates.iter().any(|(id, _, _)| id.is_empty()),
        [choice] if choice.group_id == "tactical-choice" => {
            candidates.iter().any(|(id, _, _)| *id == choice.option_id)
        }
        _ => false,
    })
}

fn reason_label(reason: tactical_rules::AttackReason) -> &'static str {
    use tactical_rules::AttackReason as R;
    match reason {
        R::LongRange => "long range",
        R::HostileNearby => "hostile within 5 ft",
        R::ProneAttacker => "attacker prone",
        R::ProneTarget => "target prone",
        R::UnconsciousTarget => "target unconscious",
        R::Dodge => "target dodging",
        R::Sap => "attacker affected by Sap",
        R::Heavy => "Heavy weapon ability requirement",
        R::PackTactics => "Pack Tactics",
        R::SunlightSensitivity => "Sunlight Sensitivity",
        R::Vex => "Vex mastery",
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::offered;
    use super::super::tests::{build, input, opening_story};
    use super::*;
    /// The active hero spends a Second Wind use, so a later aftermath rest
    /// has something to regain.
    fn spend_second_wind(current: &Checkpoint, operation: u8) -> Checkpoint {
        let member = [MEMBERS[0], MEMBERS[1]]
            .into_iter()
            .map(|bytes| MemberId::from_bytes(&bytes).unwrap())
            .find(|member| {
                offered(current, *member).is_ok_and(|offers| {
                    offers
                        .iter()
                        .any(|(kind, _)| *kind == rpc::GameplayActionKind::SecondWind)
                })
            })
            .expect("a hero can spend Second Wind");
        stage_with_supplier(
            current,
            &input(current, member, operation, "second-wind", vec![]),
            &mut |sides| Ok(sides.min(3)),
        )
        .unwrap()
    }
    pub(in crate::gameplay::journey) fn current_victory(approach: &str) -> Checkpoint {
        play_fight(current_combat(approach), 8, None, None).0
    }
    /// A victory whose first hero spent a Second Wind use on the opening
    /// turn, so the aftermath rest has something to regain. The ordinary
    /// played victory leaves both heroes untouched.
    pub(in crate::gameplay::journey) fn current_victory_with_spent_second_wind(
        approach: &str,
    ) -> Checkpoint {
        let spent = spend_second_wind(&current_combat(approach), 8);
        play_fight(spent, 9, None, None).0
    }
    pub(in crate::gameplay::journey) fn current_combat(approach: &str) -> Checkpoint {
        let opening = opening_story();
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let dialogue = stage_with_supplier(
            &opening,
            &input(&opening, first, 6, approach, vec![]),
            &mut |_| panic!("dialogue dice"),
        )
        .unwrap();
        let mut initiative = [20, 17, 1, 1, 1, 1].into_iter();
        stage_with_supplier(
            &dialogue,
            &input(&dialogue, first, 7, "defend-courier", vec![]),
            &mut |_| Ok(initiative.next().unwrap()),
        )
        .unwrap()
    }
    pub(in crate::gameplay::journey) fn current_defeat(approach: &str) -> Checkpoint {
        use rpc::GameplayActionKind as K;
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        let mut current = current_combat(approach);
        for operation in 8..100 {
            if phase(&current).unwrap() == rpc::JourneyPhase::Complete {
                return current;
            }
            let member = [first, second]
                .into_iter()
                .find(|member| wire::current_combat_turn(&current, *member).unwrap())
                .unwrap();
            let kind = if read(current.state()).unwrap().reaction_offers().is_empty() {
                K::EndTurn
            } else {
                K::DeclineReaction
            };
            current = submit(
                &current,
                member,
                &request(&current, operation, kind, None),
                &mut |sides| Ok(if sides == 20 { 20 } else { sides.min(3) }),
            );
        }
        panic!("bounded submitted defeat fixture did not finish")
    }
    fn mixed_opening() -> Checkpoint {
        let mut current = initial().unwrap();
        for operation in [1, 2] {
            current = stage_join(
                &current,
                &input(
                    &current,
                    bootstrap_member().unwrap(),
                    operation,
                    "join-room",
                    vec![],
                ),
            )
            .unwrap();
        }
        for (operation, bytes, name, archer) in [
            (3, MEMBERS[0], "Brynn", false),
            (4, MEMBERS[1], "Sylvie", true),
        ] {
            let mut choices = build(name);
            choices.retain(|(key, _)| {
                !matches!(key.as_str(), "array" | "equipment" | "style-masteries")
            });
            choices.push((
                model::label("build-package").unwrap(),
                model::label(if archer { "archer" } else { "melee-stalwart" }).unwrap(),
            ));
            choices.push((
                model::label("portrait").unwrap(),
                model::label(if archer {
                    "portrait-hero-3"
                } else {
                    "portrait-hero-1"
                })
                .unwrap(),
            ));
            let member = MemberId::from_bytes(&bytes).unwrap();
            current = stage_with_supplier(
                &current,
                &input(&current, member, operation, "create-character", choices),
                &mut |_| panic!("creation draws"),
            )
            .unwrap();
        }
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        stage_with_supplier(
            &current,
            &input(&current, member, 5, "begin-story", vec![]),
            &mut |_| panic!("opening draws"),
        )
        .unwrap()
    }
    /// Stages a fight with a chosen defend operation (which picks the layout) and dice
    /// faces (initiative first, in spec order, then the fight-start enemy turns).
    fn staged_fight(
        mixed: bool,
        operation: [u8; 16],
        faces: &[u32],
    ) -> Result<Checkpoint, RepositoryError> {
        staged_fight_with_character_storage_order(mixed, operation, faces, false)
    }
    fn staged_fight_with_character_storage_order(
        mixed: bool,
        operation: [u8; 16],
        faces: &[u32],
        reverse_character_storage: bool,
    ) -> Result<Checkpoint, RepositoryError> {
        let dialogue = dialogue_checkpoint(mixed, reverse_character_storage)?;
        start_fight_from(&dialogue, operation, faces)
    }
    fn dialogue_checkpoint(
        mixed: bool,
        reverse_character_storage: bool,
    ) -> Result<Checkpoint, RepositoryError> {
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let opening = if mixed {
            mixed_opening()
        } else {
            opening_story()
        };
        let mut dialogue = stage_with_supplier(
            &opening,
            &input(&opening, member, 6, "escort-courier", vec![]),
            &mut |_| panic!("dialogue draws"),
        )?;
        if reverse_character_storage {
            let mut state = dialogue.state().clone();
            state.characters.reverse();
            dialogue = model::checkpoint(dialogue.basis(), state)?;
        }
        Ok(dialogue)
    }
    fn start_fight_from(
        dialogue: &Checkpoint,
        operation: [u8; 16],
        faces: &[u32],
    ) -> Result<Checkpoint, RepositoryError> {
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let mut supplied = faces.iter().copied();
        let mut defend = input(&dialogue, member, 7, "defend-courier", vec![]);
        let GameInput::Game(command) = &mut defend else {
            panic!("game input");
        };
        command.operation = OperationId::from_bytes(&operation).expect("operation");
        stage_with_supplier(&dialogue, &defend, &mut |sides| {
            Ok(supplied.next().unwrap_or(10).clamp(1, sides))
        })
    }
    fn adventure_table_creatures() -> Vec<String> {
        let tables: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../../content/adventure-tables/antagonists.json"
        ))
        .unwrap();
        let mut creatures = std::collections::BTreeSet::new();
        for entry in tables["tables"]["antagonists"]["entries"]
            .as_array()
            .unwrap()
        {
            for enemy in entry["tactical"]["enemies"].as_array().unwrap() {
                creatures.insert(enemy["srd_id"].as_str().unwrap().to_owned());
            }
        }
        creatures.into_iter().collect()
    }
    fn finish_enemy_first_round(mut current: Checkpoint) -> Result<Checkpoint, String> {
        for operation in 8_u8..40 {
            let battle = read(current.state()).map_err(|error| format!("read: {error:?}"))?;
            if battle.finished() || battle.reaction_offers().is_empty() {
                return Ok(current);
            }
            let member = [MEMBERS[0], MEMBERS[1]]
                .into_iter()
                .map(|bytes| MemberId::from_bytes(&bytes).unwrap())
                .find(|member| offered(&current, *member).is_ok_and(|actions| !actions.is_empty()))
                .ok_or_else(|| "a reaction is offered but no hero may answer it".to_owned())?;
            let entry = wire::action_entry(rpc::GameplayActionKind::DeclineReaction)
                .ok_or_else(|| "decline-reaction has no action entry".to_owned())?;
            let _ = crate::gameplay::refusal_detail::take();
            current = stage_with_supplier(
                &current,
                &input(&current, member, operation, entry, vec![]),
                &mut |sides| Ok(10_u32.clamp(1, sides)),
            )
            .map_err(|error| {
                format!(
                    "decline reaction: step={:?} error={error:?}",
                    crate::gameplay::refusal_detail::take()
                )
            })?;
        }
        Err("hero reactions did not settle within 32 declines".to_owned())
    }
    /// An operation id whose layout hash selects `wanted` among `layouts` layouts,
    /// mirroring the fold in `start`. A repeated single byte always hashes even, so the
    /// first byte varies against a fixed tail.
    fn operation_for_layout(wanted: usize, layouts: usize) -> [u8; 16] {
        (0_u8..=255)
            .map(|first| {
                let mut operation = [7_u8; 16];
                operation[0] = first;
                operation
            })
            .find(|operation| {
                operation
                    .iter()
                    .fold(0_usize, |hash, value| {
                        hash.wrapping_mul(31).wrapping_add(usize::from(*value))
                    })
                    % layouts
                    == wanted
            })
            .expect("an operation id selects each layout")
    }
    /// HNT-26: three scouts opening on the salt quay produced a decision whose
    /// recorded receipt was longer than the checkpoint's per-text limit, so
    /// `finish` refused the fight at `finish-checkpoint`.
    #[test]
    fn three_scouts_opening_on_the_salt_quay_start_their_fight() {
        with_live(|| {
            let dialogue = dialogue_checkpoint(true, false).unwrap();
            let bytes = scene_bytes("salt-quay").unwrap();
            let data: tactical_rules::BattleSceneData = serde_json::from_slice(bytes).unwrap();
            let layout_index = data
                .layouts
                .iter()
                .position(|layout| layout.id == "open-quay")
                .expect("salt-quay has the open-quay layout");
            let operation = operation_for_layout(layout_index, data.layouts.len());
            TEST_SCENE_OVERRIDE.with(|own| own.set(Some("salt-quay")));
            TEST_FIGHT_OVERRIDE.with(|own| {
                *own.borrow_mut() = Some(TestFight {
                    enemies: 3,
                    foe: Some("scout".to_owned()),
                });
            });
            let _ = crate::gameplay::refusal_detail::take();
            let started = start_fight_from(&dialogue, operation, &[1, 2, 20]).map_err(|error| {
                format!(
                    "step={:?} error={error:?}",
                    crate::gameplay::refusal_detail::take()
                )
            });
            TEST_SCENE_OVERRIDE.with(|own| own.set(None));
            TEST_FIGHT_OVERRIDE.with(|own| *own.borrow_mut() = None);
            let started = started.expect("three scouts start their fight");
            let semantic = started
                .state()
                .decisions
                .last()
                .and_then(|decision| decision.semantic_output.as_deref())
                .expect("the fight-start decision records its receipt");
            assert!(
                semantic.len() > 4096,
                "this case must exercise the old 4096 limit, got {}",
                semantic.len()
            );
            assert!(
                semantic.len() <= model::limits().maximum_text_bytes,
                "recorded receipt {} exceeds the checkpoint text limit",
                semantic.len()
            );
        });
    }
    /// The budgets behind `model::limits().maximum_text_bytes`: one outcome
    /// filled to every producer cap at once (48-byte names, 4 fight lines, a
    /// 24-cell dash path, the longest source revision) encodes within 2,048
    /// bytes, and a decision full of them (6 combatants x 12 outcomes)
    /// hex-encodes within the checkpoint text limit.
    #[test]
    fn a_worst_case_receipt_fits_the_checkpoint_text_limit() {
        let line = rpc::FightLine {
            role: rpc::TableRole::Hero as i32,
            hero_index: 1,
            speaker: "s".repeat(48),
            text: "t".repeat(160),
        };
        let outcome = rpc::CombatOutcome {
            actor_name: "a".repeat(48),
            target_name: "b".repeat(48),
            attack_die: u32::MAX,
            attack_modifier: i32::MIN,
            attack_total: i32::MAX,
            target_armor_class: u32::MAX,
            hit: true,
            critical: true,
            damage_dice: vec![u32::MAX; 4],
            damage: u32::MAX,
            target_hit_points: u32::MAX,
            knocked_out: true,
            grazed: true,
            source_revision: "r".repeat(128),
            actor_id: vec![7; 16],
            target_id: vec![9; 16],
            weapon: "w".repeat(32),
            attack_style: 2,
            damage_type: "d".repeat(16),
            distance_ft: u32::MAX,
            kind: rpc::OutcomeKind::Attack as i32,
            path: (0..24)
                .map(|cell| rpc::GridCell {
                    col: cell,
                    row: cell,
                })
                .collect(),
            cost_ft: u32::MAX,
            remaining_ft: u32::MAX,
            roll_mode: -1,
            faces: vec![u32::MAX; 4],
            reasons: vec!["disadvantage: ".repeat(2); 6],
            cover: "c".repeat(16),
            death_save_result: rpc::DeathSaveResult::CriticalFailure as i32,
            death_successes: 3,
            death_failures: 3,
            recovery_hours: 4,
            lines: vec![line; 4],
            sequence: u64::MAX,
        };
        let encoded = outcome.encode_to_vec().len();
        assert!(encoded <= 2048, "one worst-case outcome is {encoded} bytes");
        let receipt = rpc::AcceptedAction {
            combat: vec![outcome; 6 * 12],
            phase: rpc::JourneyPhase::Combat as i32,
            ..Default::default()
        };
        let hex_len = receipt.encode_to_vec().len() * 2;
        assert!(
            hex_len <= model::limits().maximum_text_bytes,
            "a worst-case receipt hex-encodes to {hex_len} bytes"
        );
    }
    /// Exhaustively checks every stored adventure creature, scene layout and enemy count.
    #[test]
    #[ignore]
    fn every_adventure_creature_plays_its_first_round_on_every_layout() {
        with_live(|| {
            let creatures = adventure_table_creatures();
            assert!(!creatures.is_empty(), "adventure tables field creatures");
            let dialogues = [
                dialogue_checkpoint(true, false).unwrap(),
                dialogue_checkpoint(true, true).unwrap(),
            ];
            let mut failures = Vec::new();
            let mut checked = 0_usize;
            for (order_index, dialogue) in dialogues.iter().enumerate() {
                let membership_order = participants(dialogue)
                    .into_iter()
                    .map(|participant| participant.character.unwrap())
                    .collect::<Vec<_>>();
                for id in scene_ids() {
                    let bytes = scene_bytes(id).unwrap();
                    let data: tactical_rules::BattleSceneData =
                        serde_json::from_slice(bytes).unwrap();
                    TEST_SCENE_OVERRIDE.with(|own| own.set(scene_ids().find(|known| *known == id)));
                    for layout in &data.layouts {
                        let layout_index = data
                            .layouts
                            .iter()
                            .position(|known| known.id == layout.id)
                            .unwrap();
                        let operation = operation_for_layout(layout_index, data.layouts.len());
                        for creature in &creatures {
                            for count in 1..=MAXIMUM_ENEMIES {
                                checked += 1;
                                let tag = format!(
                                    "{id}/{} {creature}x{count} hero-order-{order_index}",
                                    layout.id
                                );
                                TEST_FIGHT_OVERRIDE.with(|own| {
                                    *own.borrow_mut() = Some(TestFight {
                                        enemies: count,
                                        foe: Some(creature.clone()),
                                    });
                                });
                                let _ = crate::gameplay::refusal_detail::take();
                                let started = start_fight_from(dialogue, operation, &[1, 2, 20])
                                    .map_err(|error| {
                                        format!(
                                            "start refused: step={:?} error={error:?}",
                                            crate::gameplay::refusal_detail::take()
                                        )
                                    })
                                    .and_then(finish_enemy_first_round);
                                let current = match started {
                                    Ok(current) => current,
                                    Err(problem) => {
                                        failures.push(format!("{tag}: {problem}"));
                                        continue;
                                    }
                                };
                                let battle = match read(current.state()) {
                                    Ok(battle) => battle,
                                    Err(error) => {
                                        failures.push(format!("{tag}: read: {error:?}"));
                                        continue;
                                    }
                                };
                                if snapshot(current.state()).map(|saved| saved.layout.as_str())
                                    != Ok(layout.id.as_str())
                                {
                                    failures.push(format!("{tag}: wrong layout staged"));
                                }
                                if current.state().encounters[0].participants[..2]
                                    != membership_order
                                {
                                    failures
                                        .push(format!("{tag}: heroes not staged in join order"));
                                }
                                let enemies: Vec<_> = battle
                                    .combatants()
                                    .iter()
                                    .filter(|actor| actor.side() == Side::Enemies)
                                    .collect();
                                if enemies.len() != count
                                    || enemies
                                        .iter()
                                        .any(|actor| actor.spec().kind.srd_id() != Some(creature))
                                {
                                    failures.push(format!(
                                        "{tag}: staged {:?} enemies",
                                        enemies
                                            .iter()
                                            .map(|actor| actor.spec().kind.srd_id())
                                            .collect::<Vec<_>>()
                                    ));
                                }
                                let enemies_first =
                                    battle.initiative_order()[..count].iter().all(|actor| {
                                        battle.actor(*actor).is_ok_and(|combatant| {
                                            combatant.side() == Side::Enemies
                                        })
                                    });
                                if !enemies_first {
                                    failures.push(format!(
                                        "{tag}: enemies not first in initiative {:?}",
                                        battle.initiative_order()
                                    ));
                                }
                                let hero_to_act = battle.finished()
                                    || battle.active_actor().is_some_and(|actor| {
                                        battle
                                            .actor(actor)
                                            .is_ok_and(|combatant| combatant.side() == Side::Heroes)
                                    });
                                if !hero_to_act {
                                    failures.push(format!(
                                        "{tag}: first round stopped on actor {:?} round {}",
                                        battle.active_actor(),
                                        battle.round()
                                    ));
                                }
                                if battle.round() != 1 {
                                    failures.push(format!(
                                        "{tag}: first round ended in round {}",
                                        battle.round()
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            TEST_SCENE_OVERRIDE.with(|own| own.set(None));
            TEST_FIGHT_OVERRIDE.with(|own| *own.borrow_mut() = None);
            println!(
                "{checked} combinations checked, {} refusals",
                failures.len()
            );
            assert!(
                failures.is_empty(),
                "{} failures:\n{}",
                failures.len(),
                failures.join("\n")
            );
        });
    }
    #[test]
    fn ses02_valley_bandits_start_a_fight_on_both_layouts_with_enemies_first() {
        with_live(|| {
            TEST_SCENE_OVERRIDE.with(|own| own.set(Some("scan-valley-bridges")));
            TEST_FIGHT_OVERRIDE.with(|own| {
                *own.borrow_mut() = Some(TestFight {
                    enemies: 3,
                    foe: Some("bandit".to_owned()),
                });
            });
            for layout_index in 0..2 {
                let operation = operation_for_layout(layout_index, 2);
                let _ = crate::gameplay::refusal_detail::take();
                match staged_fight(true, operation, &[1, 2, 20]) {
                    Ok(current) => {
                        let battle = read(current.state()).unwrap();
                        assert_eq!(
                            battle
                                .combatants()
                                .iter()
                                .filter(|actor| actor.side() == Side::Enemies)
                                .count(),
                            3,
                            "layout {layout_index}"
                        );
                    }
                    Err(error) => panic!(
                        "layout {layout_index} refused: step={:?} error={error:?}",
                        crate::gameplay::refusal_detail::take()
                    ),
                }
            }
            TEST_SCENE_OVERRIDE.with(|own| own.set(None));
            TEST_FIGHT_OVERRIDE.with(|own| *own.borrow_mut() = None);
        });
    }
    #[test]
    fn fight_start_uses_join_order_for_heroes_independent_of_character_storage_order() {
        with_live(|| {
            TEST_SCENE_OVERRIDE.with(|own| own.set(Some("scan-valley-bridges")));
            TEST_FIGHT_OVERRIDE.with(|own| {
                *own.borrow_mut() = Some(TestFight {
                    enemies: 3,
                    foe: Some("bandit".to_owned()),
                });
            });
            let operation = operation_for_layout(0, 2);
            for reverse_character_storage in [false, true] {
                let current = staged_fight_with_character_storage_order(
                    true,
                    operation,
                    &[1, 2, 20],
                    reverse_character_storage,
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "reverse_character_storage={reverse_character_storage}: step={:?} error={error:?}",
                        crate::gameplay::refusal_detail::take()
                    )
                });
                let membership_order = participants(&current)
                    .into_iter()
                    .map(|participant| participant.character.unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(
                    current.state().encounters[0].participants[..2],
                    membership_order
                );
            }
            TEST_SCENE_OVERRIDE.with(|own| own.set(None));
            TEST_FIGHT_OVERRIDE.with(|own| *own.borrow_mut() = None);
        });
    }
    fn fight(mixed: bool) -> Checkpoint {
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let opening = if mixed {
            mixed_opening()
        } else {
            opening_story()
        };
        let dialogue = stage_with_supplier(
            &opening,
            &input(&opening, member, 6, "escort-courier", vec![]),
            &mut |_| panic!("dialogue draws"),
        )
        .unwrap();
        let mut initiative = [20, 17, 1].into_iter();
        stage_with_supplier(
            &dialogue,
            &input(&dialogue, member, 7, "defend-courier", vec![]),
            &mut |_| Ok(initiative.next().unwrap_or(1)),
        )
        .unwrap()
    }
    /// A test fight against one to the most enemies starts on every battle scene the server
    /// holds (the two painted-story scenes, the others and the four scans): each enemy of the
    /// roster stands on its own enemy spawn cell of the chosen layout, and the heroes on theirs.
    #[test]
    fn a_test_fight_of_one_to_the_most_enemies_places_everyone_on_every_battle_scene() {
        with_live(|| {
            for id in scene_ids() {
                let bytes = scene_bytes(id).unwrap();
                let data: tactical_rules::BattleSceneData = serde_json::from_slice(bytes).unwrap();
                for layout in &data.layouts {
                    let map = BattleMap::from_json(bytes, &layout.id).unwrap();
                    assert!(
                        map.enemy_spawns().len() >= MAXIMUM_ENEMIES && map.hero_spawns().len() >= 2,
                        "{id}/{}: room for {MAXIMUM_ENEMIES} enemies and two heroes",
                        layout.id
                    );
                }
                TEST_SCENE_OVERRIDE.with(|own| own.set(scene_ids().find(|known| *known == id)));
                for count in [1, 2, 3, MAXIMUM_ENEMIES] {
                    TEST_FIGHT_OVERRIDE.with(|own| {
                        *own.borrow_mut() = Some(TestFight {
                            enemies: count,
                            foe: Some("kobold-warrior".to_owned()),
                        });
                    });
                    let current = fight(false);
                    let battle = read(current.state()).unwrap();
                    let saved = snapshot(current.state()).unwrap();
                    let map = BattleMap::from_json(&saved.scene, saved.layout.as_str()).unwrap();
                    let enemy_cells: Vec<_> = battle
                        .combatants()
                        .iter()
                        .filter(|actor| actor.side() == Side::Enemies)
                        .map(|actor| actor.cell())
                        .collect();
                    assert_eq!(enemy_cells.len(), count, "{id}: {count} enemies");
                    assert_eq!(
                        enemy_cells,
                        map.enemy_spawns()[..count].to_vec(),
                        "{id}: enemies stand on their spawn cells"
                    );
                    let hero_cells: Vec<_> = battle
                        .combatants()
                        .iter()
                        .filter(|actor| actor.side() == Side::Heroes)
                        .map(|actor| actor.cell())
                        .collect();
                    assert_eq!(hero_cells, map.hero_spawns()[..2].to_vec(), "{id}: heroes");
                }
            }
            TEST_SCENE_OVERRIDE.with(|own| own.set(None));
            TEST_FIGHT_OVERRIDE.with(|own| *own.borrow_mut() = None);
        });
    }

    fn play_fight(
        mut current: Checkpoint,
        first_operation: u8,
        replay: Option<&[ActualDraw]>,
        mut seed: Option<u64>,
    ) -> (Checkpoint, Vec<Vec<u8>>, Vec<Checkpoint>) {
        use prost::Message;
        let expect_victory = seed.is_none();
        let mut evidence = Vec::new();
        let mut checkpoints = Vec::new();
        for operation in first_operation..200 {
            if phase(&current).unwrap() == rpc::JourneyPhase::Complete {
                break;
            }
            let battle = read(current.state()).unwrap();
            let member = [MEMBERS[0], MEMBERS[1]]
                .into_iter()
                .map(|bytes| MemberId::from_bytes(&bytes).unwrap())
                .find(|member| !offered(&current, *member).unwrap().is_empty())
                .unwrap();
            let actor =
                actor_id(current.state(), player_entity(member, &current).unwrap()).unwrap();
            let attacks = options(
                current.state(),
                actor,
                rpc::GameplayActionKind::GreatswordAttack,
            )
            .unwrap();
            let (kind, selection) = if !battle.reaction_offers().is_empty() {
                (rpc::GameplayActionKind::DeclineReaction, None)
            } else if battle.actor(actor).unwrap().health().hit_points <= 6
                && !options(current.state(), actor, rpc::GameplayActionKind::SecondWind)
                    .unwrap()
                    .is_empty()
            {
                (rpc::GameplayActionKind::SecondWind, None)
            } else if let Some((id, _, _)) = attacks.first() {
                (rpc::GameplayActionKind::GreatswordAttack, Some(id.clone()))
            } else if !battle.actor(actor).unwrap().budget().action_used {
                let movements =
                    options(current.state(), actor, rpc::GameplayActionKind::Move).unwrap();
                let best = movements.iter().min_by_key(|(_, action, _)| {
                    let Action::Move(path) = action else {
                        panic!("move");
                    };
                    let destination = path.last().unwrap();
                    battle
                        .combatants()
                        .iter()
                        .filter(|enemy| {
                            enemy.side() == Side::Enemies && !enemy.health().incapacitated()
                        })
                        .map(|enemy| destination.distance(enemy.cell()))
                        .min()
                        .unwrap()
                });
                best.map(|(id, _, _)| (rpc::GameplayActionKind::Move, Some(id.clone())))
                    .unwrap_or((rpc::GameplayActionKind::EndTurn, None))
            } else {
                (rpc::GameplayActionKind::EndTurn, None)
            };
            let choices = selection
                .map(|id| {
                    vec![(
                        model::label("tactical-choice").unwrap(),
                        model::label(&id).unwrap(),
                    )]
                })
                .unwrap_or_default();
            let action = wire::action_entry(kind).unwrap();
            let records = replay.map(|records| {
                records
                    .iter()
                    .filter(|draw| draw.operation.as_bytes() == &[operation; 16])
                    .collect::<Vec<_>>()
            });
            let mut recorded = records.as_deref().unwrap_or(&[]).iter();
            current = stage_with_supplier(
                &current,
                &input(&current, member, operation, action, choices),
                &mut |sides| {
                    if replay.is_some() {
                        let draw = recorded.next().expect("replay must use a retained draw");
                        assert_eq!(draw.sides, sides);
                        Ok(draw.value)
                    } else if let Some(state) = &mut seed {
                        *state = state
                            .wrapping_mul(6364136223846793005)
                            .wrapping_add(1442695040888963407);
                        Ok(((*state >> 32) as u32) % sides + 1)
                    } else {
                        Ok(if sides == 20 {
                            if kind == rpc::GameplayActionKind::GreatswordAttack {
                                15
                            } else {
                                1
                            }
                        } else {
                            sides.min(3)
                        })
                    }
                },
            ).unwrap_or_else(|error|panic!("play operation={operation},kind={kind:?},seed={seed:?},battle={battle:?}: {error:?}"));
            assert!(recorded.next().is_none());
            let bytes =
                df_persistence::local_demo_scope::encode_owned_demo_checkpoint(&current, codec())
                    .unwrap();
            let view = wire::journey_view(
                &current,
                df_persistence::local_demo_scope::LocalDemoRole::Player,
                member,
            ).unwrap_or_else(|error|panic!("view operation={operation},phase={:?},initiative={:?},native_resume={:?}: {error:?}",phase(&current),initiative(&current),super::super::super::beat_script::validate_resume(&current)));
            evidence.push([bytes, view.encode_to_vec()].concat());
            checkpoints.push(current.clone());
        }
        assert_eq!(phase(&current).unwrap(), rpc::JourneyPhase::Complete);
        if expect_victory {
            assert!(victory(current.state()).unwrap());
        }
        (current, evidence, checkpoints)
    }
    fn codec() -> df_persistence::NativeCodecLimits {
        df_persistence::NativeCodecLimits {
            maximum_document_bytes: 2 * 1024 * 1024,
            maximum_allocated_bytes: 2 * 1024 * 1024,
            maximum_collection_items: 4096,
            maximum_text_bytes: 4096,
        }
    }
    #[test]
    fn mixed_build_live_actions_replay_bytes_and_midfight_checkpoint_resume_to_victory_rest() {
        let mut tuning: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../../content/harbor-journey/tuning.json"
        ))
        .unwrap();
        tuning["tactical"] =
            serde_json::json!({"enemies":[{"srd_id":"cultist","count":2,"ammunition":0}]});
        let loaded = pack::load_from_bytes(
            include_bytes!("../../../../../content/harbor-journey/pack.json"),
            &serde_json::to_vec(&tuning).unwrap(),
        )
        .unwrap();
        pack::with_test_pack(loaded, || {
            let started = fight(true);
            assert!(
                read(started.state())
                    .unwrap()
                    .actor(2)
                    .unwrap()
                    .spec()
                    .archery
            );
            // Spend a Second Wind use first, so the closing rest has a
            // use to regain; both plays spend it the same way.
            let started = spend_second_wind(&started, 8);
            let (final_state, first, checkpoints) = play_fight(started.clone(), 9, None, None);
            let (_, second, _) = play_fight(
                spend_second_wind(&fight(true), 8),
                9,
                Some(&final_state.state().draws),
                None,
            );
            assert_eq!(first, second);
            let mid = checkpoints.first().unwrap();
            assert!(phase(mid).unwrap() == rpc::JourneyPhase::Combat);
            let bytes =
                df_persistence::local_demo_scope::encode_owned_demo_checkpoint(mid, codec())
                    .unwrap();
            let restored = df_persistence::local_demo_scope::decode_owned_demo_checkpoint(
                &bytes,
                mid.basis(),
                &model::pins().unwrap(),
                &model::recovery().unwrap(),
                codec(),
            )
            .unwrap();
            for bytes in MEMBERS {
                let member = MemberId::from_bytes(&bytes).unwrap();
                assert_eq!(
                    offered(mid, member).unwrap(),
                    offered(&restored, member).unwrap()
                );
            }
            let (resumed, resumed_bytes, _) =
                play_fight(restored, 10, Some(&final_state.state().draws), None);
            assert_eq!(final_state, resumed);
            assert_eq!(&first[1..], resumed_bytes.as_slice());
            let battle = read(final_state.state()).unwrap();
            assert!(battle.actor(2).unwrap().arrows() < 20);
            assert_eq!(
                final_state
                    .state()
                    .inventory
                    .iter()
                    .find(|item| item.owner == entity(ENTITIES[1]).unwrap()
                        && item.origin.entry.as_str() == "arrow")
                    .unwrap()
                    .quantity,
                u64::from(battle.actor(2).unwrap().arrows())
            );
            let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
            let rested = stage_with_supplier(
                &final_state,
                &input(&final_state, member, 201, "short-rest", vec![]),
                &mut |_| panic!("rest draws"),
            )
            .unwrap();
            assert!(
                rested
                    .state()
                    .narrative
                    .completed_beats
                    .contains(&model::content("short-rest-complete").unwrap())
            );
        });
    }

    fn with_live(run: impl FnOnce()) {
        let mut tuning: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../../content/harbor-journey/tuning.json"
        ))
        .unwrap();
        tuning["tactical"] =
            serde_json::json!({"enemies":[{"srd_id":"bandit","count":2,"ammunition":20}]});
        let loaded = pack::load_from_bytes(
            include_bytes!("../../../../../content/harbor-journey/pack.json"),
            &serde_json::to_vec(&tuning).unwrap(),
        )
        .unwrap();
        pack::with_test_pack(loaded, run);
    }
    #[test]
    fn tactical_illegal_cell_target_wrong_turn_and_stale_revision_refuse_without_draw_or_state_change()
     {
        with_live(|| {
            let current = fight(true);
            let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
            let other = MemberId::from_bytes(&MEMBERS[1]).unwrap();
            let before =
                df_persistence::local_demo_scope::encode_owned_demo_checkpoint(&current, codec())
                    .unwrap();
            let choices = |id: &str| {
                vec![(
                    model::label("tactical-choice").unwrap(),
                    model::label(id).unwrap(),
                )]
            };
            let illegal_cell = input(&current, member, 8, "move", choices("999-999"));
            let illegal_target = input(
                &current,
                member,
                8,
                "greatsword-attack",
                choices("greatsword-0-999-0-0-action"),
            );
            let wrong_turn = input(&current, other, 8, "end-turn", vec![]);
            let mut stale = input(&current, member, 8, "end-turn", vec![]);
            let GameInput::Game(command) = &mut stale else {
                panic!("command");
            };
            command.observed_revision = initial().unwrap().basis().revision;
            for command in [illegal_cell, illegal_target, wrong_turn, stale] {
                assert_eq!(
                    stage_with_supplier(&current, &command, &mut |_| panic!(
                        "refused actions cannot draw"
                    )),
                    Err(RepositoryError::InvalidCandidate)
                );
                assert_eq!(
                    df_persistence::local_demo_scope::encode_owned_demo_checkpoint(
                        &current,
                        codec()
                    )
                    .unwrap(),
                    before
                );
            }
        });
    }
    #[test]
    fn tactical_views_scope_own_budgets_ammunition_sheet_and_ignore_hidden_enemy_information() {
        use df_persistence::local_demo_scope::{DISPLAY, LocalDemoRole as R};
        use prost::Message;
        with_live(|| {
            let current = fight(true);
            let display = MemberId::from_bytes(&DISPLAY).unwrap();
            let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
            let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
            for (role, member) in [
                (R::Display, display),
                (R::Player, first),
                (R::Player, second),
            ] {
                let view = wire::journey_view(&current, role, member).unwrap();
                let (narration, scene) = match view.audience.unwrap() {
                    rpc::view_message::Audience::Display(view) => {
                        (view.narration, view.scene.unwrap())
                    }
                    rpc::view_message::Audience::Player(view) => {
                        (view.narration, view.scene.unwrap())
                    }
                };
                assert!(scene.rules_note.contains("tactical.cover-centre-line"));
                assert!(scene.rules_note.contains("segment between cell centres"));
                assert!(!narration.contains("tactical.cover-centre-line"));
            }
            let inspect = |view: rpc::ViewMessage| match view.audience.unwrap() {
                rpc::view_message::Audience::Display(view) => view.journey.unwrap(),
                rpc::view_message::Audience::Player(view) => view.journey.unwrap(),
            };
            let public = inspect(wire::journey_view(&current, R::Display, display).unwrap());
            assert!(public.own_character.is_none());
            assert!(public.creation.is_none());
            let combat = public.combat.unwrap();
            assert!(combat.own_budget.is_none());
            assert!(combat.own_ammunition.is_none());
            assert!(!combat.reachable_cells.is_empty());
            let public_target = |target: &rpc::LegalCombatTarget| {
                !target.participant_id.is_empty()
                    && target.weapon.is_empty()
                    && target.cover.is_empty()
                    && target.range_reason.is_empty()
                    && !target.disadvantage
                    && target.reasons.is_empty()
                    && target.option_id.is_empty()
            };
            assert!(!combat.legal_targets.is_empty());
            assert!(combat.legal_targets.iter().all(public_target));
            let first_view = inspect(wire::journey_view(&current, R::Player, first).unwrap());
            let second_view = inspect(wire::journey_view(&current, R::Player, second).unwrap());
            assert_eq!(first_view.own_character.unwrap().name, "Brynn");
            assert_eq!(second_view.own_character.unwrap().name, "Sylvie");
            assert!(
                second_view
                    .combat
                    .as_ref()
                    .unwrap()
                    .legal_targets
                    .iter()
                    .all(public_target)
            );
            assert!(
                first_view
                    .combat
                    .as_ref()
                    .unwrap()
                    .legal_targets
                    .iter()
                    .any(|target| !target.weapon.is_empty() && !target.option_id.is_empty())
            );
            assert!(first_view.combat.unwrap().own_ammunition.is_some());
            let mut changed = current.state().clone();
            let private = changed
                .characters
                .iter_mut()
                .find(|character| character.entity == entity(ENTITIES[1]).unwrap())
                .unwrap()
                .choices
                .iter_mut()
                .find(|choice| choice.offer.as_str() == "alignment")
                .unwrap();
            private.selected = model::label("neutral-good").unwrap();
            let saved = changed.encounters[0].tactical.as_mut().unwrap();
            let mut private_battle: serde_json::Value =
                serde_json::from_slice(&saved.state).unwrap();
            private_battle["actors"][2]["spec"]["bolts"] = 19.into();
            saved.state = serde_json::to_vec(&private_battle).unwrap();
            let changed = model::checkpoint(current.basis(), changed).unwrap();
            assert_eq!(
                wire::journey_view(&current, R::Display, display)
                    .unwrap()
                    .encode_to_vec(),
                wire::journey_view(&changed, R::Display, display)
                    .unwrap()
                    .encode_to_vec()
            );
            assert_eq!(
                wire::journey_view(&current, R::Player, first)
                    .unwrap()
                    .encode_to_vec(),
                wire::journey_view(&changed, R::Player, first)
                    .unwrap()
                    .encode_to_vec()
            );
            assert_ne!(
                wire::journey_view(&current, R::Player, second)
                    .unwrap()
                    .encode_to_vec(),
                wire::journey_view(&changed, R::Player, second)
                    .unwrap()
                    .encode_to_vec()
            );
        });
    }
    #[test]
    fn canonical_harbor_default_three_bandits_last_three_to_five_rounds_and_all_action_labels_are_safe()
     {
        let loaded = pack::load_from_bytes(
            include_bytes!("../../../../../content/harbor-journey/pack.json"),
            include_bytes!("../../../../../content/harbor-journey/tuning.json"),
        )
        .unwrap();
        pack::with_test_pack(loaded, || {
            let started = fight(true);
            let battle = read(started.state()).unwrap();
            assert_eq!(
                battle
                    .combatants()
                    .iter()
                    .filter(|actor| actor.side() == Side::Enemies)
                    .count(),
                3
            );
            let creation = wire::creation_offer().unwrap();
            assert_eq!(
                creation
                    .groups
                    .iter()
                    .find(|group| group.group_id == "build-package")
                    .unwrap()
                    .options
                    .len(),
                3
            );
            assert_eq!(
                creation
                    .groups
                    .iter()
                    .find(|group| group.group_id == "portrait")
                    .unwrap()
                    .options
                    .len(),
                3
            );
            assert!(creation.groups.iter().all(|group| !matches!(
                group.group_id.as_str(),
                "array" | "equipment" | "style-masteries"
            )));
            let sheet = character_sheet(started.state(), entity(ENTITIES[1]).unwrap()).unwrap();
            assert_eq!(sheet.armor_class, 15);
            assert_eq!(sheet.hit_points, 13);
            assert!(sheet.facts.iter().any(|fact| fact.label == "Longbow attack"
                && fact.value.contains("+7 to hit")
                && fact.value.contains("1d8+3")));
            use rpc::GameplayActionKind as K;
            for kind in [
                K::Move,
                K::GreatswordAttack,
                K::Dash,
                K::Dodge,
                K::Disengage,
                K::SecondWind,
                K::EndTurn,
                K::OpportunityAttack,
                K::DeclineReaction,
                K::Stand,
                K::PickUpWeapon,
            ] {
                assert!(
                    !super::super::super::beat_script_runtime::action_label(
                        &offer_id(&started, kind),
                        kind
                    )
                    .unwrap()
                    .is_empty()
                );
            }
            let (finished, _, _) = play_fight(started, 8, None, None);
            let rounds = read(finished.state()).unwrap().round();
            eprintln!(
                "canonical harbor bounded fixture:3 bandits;greatsword+archer;layout={};rounds={rounds}",
                snapshot(finished.state()).unwrap().layout.as_str()
            );
            assert!((3..=5).contains(&rounds));
            assert!(finished.state().decisions.iter().any(|decision| {
                accepted(decision).is_ok_and(|receipt| {
                    receipt
                        .combat
                        .iter()
                        .any(|attack| ENEMY_ENTITIES.iter().any(|id| attack.actor_id == *id))
                })
            }));
        });
    }
    /// A recorded fight speaks the same cries and closing line when it is played again:
    /// line picks are deterministic in the recorded fight data.
    #[test]
    fn a_recorded_fight_says_the_same_lines_twice() {
        use df_persistence::local_demo_scope::{DISPLAY, LocalDemoRole as R};
        fn spoken(finished: &Checkpoint) -> (Vec<(i32, u32, String, String)>, Vec<String>) {
            let mut lines = Vec::new();
            for decision in &finished.state().decisions {
                let Ok(receipt) = accepted(decision) else {
                    continue;
                };
                for outcome in receipt.combat {
                    for line in outcome.lines {
                        lines.push((
                            line.role,
                            line.hero_index,
                            line.speaker.clone(),
                            line.text.clone(),
                        ));
                    }
                }
            }
            let display = MemberId::from_bytes(&DISPLAY).unwrap();
            let view = wire::journey_view(finished, R::Display, display).unwrap();
            let closing = match view.audience {
                Some(rpc::view_message::Audience::Display(view)) => view
                    .journey
                    .and_then(|journey| journey.combat)
                    .map(|combat| {
                        combat
                            .closing_lines
                            .iter()
                            .map(|line| line.text.clone())
                            .collect()
                    })
                    .unwrap_or_default(),
                _ => panic!("a display view carries the finished fight"),
            };
            (lines, closing)
        }
        with_live(|| {
            let first = spoken(&play_fight(fight(true), 8, None, None).0);
            let second = spoken(&play_fight(fight(true), 8, None, None).0);
            assert_eq!(first, second, "the same fight says the same lines twice");
            assert!(!first.0.is_empty(), "the fight speaks its cries");
            assert!(
                !first.1.is_empty(),
                "the finished fight has a closing line on the display"
            );
            let mut chars = 0usize;
            for (role, hero_index, speaker, text) in first
                .0
                .iter()
                .map(|(role, index, speaker, text)| (*role, *index, speaker.clone(), text.clone()))
            {
                assert!(
                    !text.contains('{') && !text.contains('}'),
                    "no placeholders survive: {text}"
                );
                assert!(
                    text.chars().count() <= super::super::super::fight_lines::MAXIMUM_LINE_CHARS,
                    "a cry fits its beat: {text}"
                );
                if role == rpc::TableRole::Hero as i32 {
                    assert!(hero_index < 2, "a hero cry names its party slot");
                    assert!(!speaker.is_empty(), "a hero cry names its speaker");
                }
                chars += text.chars().count();
            }
            for text in &first.1 {
                chars += text.chars().count();
            }
            assert!(
                chars <= 4_000,
                "one fight's lines stay inside the narrator budget: {chars} chars"
            );
        });
    }

    #[test]
    fn three_enemy_fight_views_fit_the_rpc_message_bound_for_each_seat() {
        let mut tuning: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../../content/harbor-journey/tuning.json"
        ))
        .unwrap();
        tuning["tactical"] =
            serde_json::json!({"enemies":[{"srd_id":"bandit","count":3,"ammunition":20}]});
        let loaded = pack::load_from_bytes(
            include_bytes!("../../../../../content/harbor-journey/pack.json"),
            &serde_json::to_vec(&tuning).unwrap(),
        )
        .unwrap();
        pack::with_test_pack(loaded, || {
            let current = fight(true);
            // The phones decode a watch view with the bridge's RPC ceiling; an owner's combat view
            // with three enemies must stay under it or the stream is cut mid-fight.
            for bytes in [MEMBERS[0], MEMBERS[1]] {
                let member = MemberId::from_bytes(&bytes).unwrap();
                let view = wire::journey_view(
                    &current,
                    df_persistence::local_demo_scope::LocalDemoRole::Player,
                    member,
                )
                .unwrap();
                assert!(view.encode_to_vec().len() <= df_rpc_bridge::RPC_MESSAGE_BYTES);
            }
        });
    }
    #[test]
    fn tactical_creation_packages_validate_and_lawful_unimplemented_cross_package_is_refused_explicitly()
     {
        with_live(|| {
            let current = mixed_opening();
            assert_eq!(
                value(current.state(), entity(ENTITIES[0]).unwrap(), "armor-class"),
                Ok(17)
            );
            assert_eq!(
                value(current.state(), entity(ENTITIES[1]).unwrap(), "armor-class"),
                Ok(15)
            );
            let mut cross = build("Archer");
            cross.retain(|(key, _)| {
                !matches!(key.as_str(), "array" | "equipment" | "style-masteries")
            });
            cross.push((
                model::label("build-package").unwrap(),
                model::label("archer").unwrap(),
            ));
            cross.push((
                model::label("equipment").unwrap(),
                model::label("fighter-a-soldier-a").unwrap(),
            ));
            let borrowed = cross
                .iter()
                .filter(|(key, _)| key.as_str() != "name")
                .map(|(key, value)| (key.as_str(), value.as_str()))
                .collect::<Vec<_>>();
            // Archery with Fighter A is SRD-lawful; this bounded package API does not implement it.
            assert!(pack::validate_character("Archer", &borrowed).is_err());
        });
    }
    #[test]
    fn tactical_recovery_refuses_canonical_mirror_map_hash_source_and_turn_forgery() {
        with_live(|| {
            let current = fight(true);
            for field in [
                "position", "arrows", "held", "ability", "layout", "source", "hash", "order",
                "active",
            ] {
                let mut state = current.state().clone();
                let who = entity(ENTITIES[0]).unwrap();
                match field {
                    "position" => {
                        state
                            .entities
                            .iter_mut()
                            .find(|entity| entity.id == who)
                            .unwrap()
                            .position
                            .as_mut()
                            .unwrap()
                            .x += 5
                    }
                    "arrows" => {
                        state
                            .inventory
                            .iter_mut()
                            .find(|item| item.owner == who && item.origin.entry.as_str() == "arrow")
                            .unwrap()
                            .quantity -= 1
                    }
                    "held" => set(&mut state, who, "held-weapon", 0).unwrap(),
                    "ability" => set(&mut state, who, "strength", 18).unwrap(),
                    "layout" => {
                        state.encounters[0].tactical.as_mut().unwrap().layout =
                            model::label("unknown").unwrap()
                    }
                    "source" => {
                        state.encounters[0]
                            .tactical
                            .as_mut()
                            .unwrap()
                            .source_revision = model::label("unknown").unwrap()
                    }
                    "hash" => {
                        state.encounters[0]
                            .tactical
                            .as_mut()
                            .unwrap()
                            .scene_digest
                            .0[0] ^= 1
                    }
                    "order" => state.encounters[0].turn_order.swap(0, 1),
                    "active" => {
                        state.encounters[0].active_turn = Some(entity(ENTITIES[1]).unwrap())
                    }
                    _ => panic!("field"),
                }
                let forged = model::checkpoint(current.basis(), state).unwrap();
                assert_eq!(
                    validate_encounter(&forged),
                    Err(RepositoryError::InvalidCandidate),
                    "{field}"
                );
                assert!(phase(&forged).is_err(), "{field}");
            }
        });
    }
    #[test]
    fn live_tactical_beat_starts_multiple_source_enemies_and_recovers_checkpoint() {
        let mut tuning: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../../content/harbor-journey/tuning.json"
        ))
        .unwrap();
        tuning["tactical"] =
            serde_json::json!({"enemies":[{"srd_id":"cultist","count":2,"ammunition":0}]});
        let loaded = pack::load_from_bytes(
            include_bytes!("../../../../../content/harbor-journey/pack.json"),
            &serde_json::to_vec(&tuning).unwrap(),
        )
        .unwrap();
        pack::with_test_pack(loaded, || {
            let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
            let current = opening_story();
            let current = stage_with_supplier(
                &current,
                &input(&current, first, 6, "escort-courier", vec![]),
                &mut |_| panic!("no dice"),
            )
            .unwrap();
            let mut staged = current.state().clone();
            let command = input(&current, first, 7, "defend-courier", vec![]);
            let mut recorded = Vec::new();
            let mut outcomes = Vec::new();
            let mut faces = [20, 19, 1].into_iter();
            start(
                &current,
                super::super::command(&command).unwrap(),
                &mut staged,
                &mut recorded,
                &mut outcomes,
                &mut |_| Ok(faces.next().unwrap()),
                &pack::scene(pack::SceneKey::Combat),
            )
            .expect("direct tactical start");
            let mut initiative = [20, 19, 1].into_iter();
            let fight = stage_with_supplier(
                &current,
                &input(&current, first, 7, "defend-courier", vec![]),
                &mut |_| Ok(initiative.next().unwrap()),
            )
            .unwrap();
            assert_eq!(read(fight.state()).unwrap().combatants().len(), 4);
            assert_eq!(phase(&fight).unwrap(), rpc::JourneyPhase::Combat);
            let codec = df_persistence::NativeCodecLimits {
                maximum_document_bytes: 2 * 1024 * 1024,
                maximum_allocated_bytes: 2 * 1024 * 1024,
                maximum_collection_items: 4096,
                maximum_text_bytes: 4096,
            };
            let encoded =
                df_persistence::local_demo_scope::encode_owned_demo_checkpoint(&fight, codec)
                    .unwrap();
            let restored = df_persistence::local_demo_scope::decode_owned_demo_checkpoint(
                &encoded,
                fight.basis(),
                &model::pins().unwrap(),
                &model::recovery().unwrap(),
                codec,
            )
            .unwrap();
            assert_eq!(fight, restored);
            assert_eq!(
                offered(&fight, first).unwrap(),
                offered(&restored, first).unwrap()
            );
            // Spend a Second Wind use first, so the closing rest has a
            // use to regain.
            let mut current = spend_second_wind(&fight, 8);
            for operation in 9..40 {
                if phase(&current).unwrap() == rpc::JourneyPhase::Complete {
                    break;
                }
                let member = [MEMBERS[0], MEMBERS[1]]
                    .into_iter()
                    .map(|bytes| MemberId::from_bytes(&bytes).unwrap())
                    .find(|member| !offered(&current, *member).unwrap().is_empty())
                    .unwrap();
                let attack = offered(&current, member)
                    .unwrap()
                    .iter()
                    .any(|(kind, _)| *kind == rpc::GameplayActionKind::GreatswordAttack);
                let choices = if attack {
                    vec![(
                        model::label("tactical-choice").unwrap(),
                        model::label(
                            &groups(&current, member, rpc::GameplayActionKind::GreatswordAttack)
                                .unwrap()[0]
                                .options[0]
                                .option_id,
                        )
                        .unwrap(),
                    )]
                } else {
                    vec![]
                };
                current = stage_with_supplier(
                    &current,
                    &input(
                        &current,
                        member,
                        operation,
                        if attack {
                            "greatsword-attack"
                        } else {
                            "end-turn"
                        },
                        choices,
                    ),
                    &mut |sides| {
                        Ok(if sides == 20 {
                            if attack { 20 } else { 1 }
                        } else {
                            sides
                        })
                    },
                )
                .unwrap();
            }
            assert_eq!(phase(&current).unwrap(), rpc::JourneyPhase::Complete);
            assert!(victory(current.state()).unwrap());
            current = stage_with_supplier(
                &current,
                &input(&current, first, 40, "short-rest", vec![]),
                &mut |_| panic!("no dice"),
            )
            .unwrap();
            assert!(
                current
                    .state()
                    .narrative
                    .completed_beats
                    .contains(&model::content("short-rest-complete").unwrap())
            );
        });
    }

    #[test]
    fn archer_attack_options_prioritize_unobstructed_melee_and_stay_paired_with_legal_targets() {
        use rpc::GameplayActionKind as K;
        with_live(|| {
            let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
            let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
            let mut current = fight(true);
            current = submit(
                &current,
                first,
                &request(&current, 8, K::EndTurn, None),
                &mut |_| panic!("end turn has no draw"),
            );
            let move_id = options(current.state(), 2, K::Move)
                .unwrap()
                .into_iter()
                .find(|(_, action, _)| {
                    matches!(action, Action::Move(path) if path.last() == Some(&tactical_rules::Cell(4, 2)))
                })
                .unwrap()
                .0;
            current = submit(
                &current,
                second,
                &request(&current, 9, K::Move, Some(&move_id)),
                &mut |_| panic!("move has no draw"),
            );

            let attacks = options(current.state(), 2, K::GreatswordAttack).unwrap();
            let (first_id, first_action, _) = attacks.first().unwrap();
            let group = groups(&current, second, K::GreatswordAttack).unwrap();
            assert_eq!(group[0].options[0].option_id, *first_id);
            let Action::Attack {
                target,
                weapon,
                spend,
            } = first_action
            else {
                panic!("first option must be an attack");
            };
            assert!(!weapon.ranged);
            assert!(!weapon.use_mastery);
            assert!(!weapon.savage_attacker);
            assert_eq!(*spend, AttackSpend::Action);

            let mut view = rpc::CombatView::default();
            project_combat(&current, Some(entity(ENTITIES[1]).unwrap()), &mut view).unwrap();
            let first_target = view
                .legal_targets
                .iter()
                .find(|legal| legal.option_id == *first_id)
                .unwrap();
            assert_eq!(first_target.cover, "none");
            assert!(!first_target.disadvantage);
            assert_eq!(
                first_target.participant_id,
                canonical(current.state(), *target).unwrap().as_bytes()
            );
            assert_eq!(
                group[0].options.len(),
                view.legal_targets
                    .iter()
                    .filter(|legal| !legal.option_id.is_empty())
                    .count()
            );
            assert!(group[0].options.iter().all(|option| {
                view.legal_targets
                    .iter()
                    .any(|legal| legal.option_id == option.option_id)
            }));
        });
    }

    #[test]
    fn canonical_live_default_seed_sample_records_source_enemy_attacks_and_typical_three_to_five_rounds()
     {
        let loaded = pack::load_from_bytes(
            include_bytes!("../../../../../content/harbor-journey/pack.json"),
            include_bytes!("../../../../../content/harbor-journey/tuning.json"),
        )
        .unwrap();
        pack::with_test_pack(loaded, || {
            let mut rounds = Vec::new();
            let mut victories = 0;
            for seed in 16..24 {
                let (finished, _, _) = play_fight(fight(true), 8, None, Some(seed));
                let count = read(finished.state()).unwrap().round();
                rounds.push(count);
                victories += usize::from(victory(finished.state()).unwrap());
                assert!(finished.state().decisions.iter().any(|decision| {
                    accepted(decision).is_ok_and(|receipt| {
                        receipt
                            .combat
                            .iter()
                            .any(|attack| ENEMY_ENTITIES.iter().any(|id| attack.actor_id == *id))
                    })
                }));
                eprintln!(
                    "default live source fight fixture seed={seed}; rounds={count}; victory={}",
                    victory(finished.state()).unwrap()
                );
            }
            rounds.sort();
            eprintln!(
                "default live sample rounds={rounds:?}; median={}; victories={victories}/8",
                rounds[4]
            );
            assert!((3..=5).contains(&rounds[4]));
        });
    }
    #[test]
    fn tactical_fight_scene_variant_owns_location_layout_and_pinned_checkpoint_map() {
        use super::super::super::adventure::setting::{SceneSetting, SettingKind};
        use super::super::super::beat_script_variety_tests as fixture;
        let mut content: pack::PackFile = serde_json::from_slice(include_bytes!(
            "../../../../../content/harbor-journey/pack.json"
        ))
        .unwrap();
        for scene in &mut content.scenes {
            scene.setting = Some(SceneSetting {
                place_id: "salt-quay".into(),
                name: "the Salt Quay".into(),
                short: "the quay".into(),
                kind: SettingKind::Quay,
                exterior: true,
                art_key: "loc-salt-quay-arrival".into(),
            });
            scene.asset = "assets/ui/generated/scene-salt-quay-arrival".into();
        }
        let mut script: super::super::super::beat_script::BeatScript = serde_json::from_slice(
            include_bytes!("../fixtures/third-approach-beat-script.json"),
        )
        .unwrap();
        for scene in script.scenes.values_mut() {
            scene.setting = content
                .scenes
                .iter()
                .find(|base| base.key == scene.key)
                .unwrap()
                .setting
                .clone();
            scene.asset = "assets/ui/generated/scene-salt-quay-arrival".into();
        }
        let mut scene = content
            .scenes
            .iter()
            .find(|scene| scene.key == pack::SceneKey::Combat)
            .unwrap()
            .clone();
        scene.setting = Some(SceneSetting {
            place_id: "pinewood-trail".into(),
            name: "the Pinewood Trail".into(),
            short: "the trail".into(),
            kind: SettingKind::ForestTrail,
            exterior: true,
            art_key: "loc-pinewood-trail-arrival".into(),
        });
        scene.asset = "assets/ui/generated/scene-pinewood-trail-arrival".into();
        script.scenes.insert("forest-fight".into(), scene);
        script
            .beats
            .iter_mut()
            .find(|beat| beat.kind == super::super::super::beat_script::BeatKind::Fight)
            .unwrap()
            .scene_variant = Some("forest-fight".into());
        content.beat_script = Some(script);
        let loaded = pack::load_from_bytes(
            &serde_json::to_vec(&content).unwrap(),
            include_bytes!("../../../../../content/harbor-journey/tuning.json"),
        )
        .unwrap();
        pack::with_test_pack(loaded, || {
            let mut current = fixture::prepared();
            for (operation, action) in [
                (5, "begin-story"),
                (6, "escort-courier"),
                (7, "defend-courier"),
            ] {
                current = fixture::act(&current, fixture::first(), operation, action);
            }
            let saved = snapshot(current.state()).unwrap();
            assert_eq!(
                saved.scene,
                SCENES
                    .iter()
                    .find(|(id, _)| *id == "pinewood-trail")
                    .unwrap()
                    .1
            );
            assert_eq!(
                super::super::super::beat_script_runtime::scene(current.state())
                    .unwrap()
                    .setting
                    .unwrap()
                    .place_id,
                "pinewood-trail"
            );
            let mut view = rpc::CombatView::default();
            project_combat(&current, None, &mut view).unwrap();
            assert_eq!(view.location_id, "pinewood-trail");
            assert_eq!(view.layout_id, saved.layout.as_str());
            let restored = fixture::persisted(&current);
            assert_eq!(current, restored);
        });
    }

    #[test]
    fn tactical_terminal_provenance_refuses_forged_cause_outcome_resource_source_and_unaccepted_event()
     {
        with_live(|| {
            let (current, _, _) = play_fight(fight(true), 8, None, None);
            let terminal = current
                .state()
                .decisions
                .last()
                .unwrap()
                .facts
                .last()
                .copied()
                .unwrap();
            for mutation in 0..4 {
                let mut state = current.state().clone();
                match mutation {
                    0 => {
                        state
                            .facts
                            .iter_mut()
                            .find(|fact| fact.id == terminal)
                            .unwrap()
                            .cause = Some(state.facts[0].id)
                    }
                    1 => {
                        let fact=state.facts.iter_mut().find(|fact|matches!(&fact.value,FactValue::ContentEvent {definition,..} if definition.entry.as_str()=="combat-victory")).unwrap();
                        if let FactValue::ContentEvent { definition, .. } = &mut fact.value {
                            *definition = model::content("combat-defeat").unwrap();
                        }
                    }
                    2 => {
                        let fact=state.facts.iter_mut().rev().find(|fact|matches!(&fact.value,FactValue::ResourceChanged {resource,after:0,..} if resource.as_str()=="hit-points")).unwrap();
                        if let FactValue::ResourceChanged { source, .. } = &mut fact.value {
                            *source = model::rule().unwrap();
                        }
                    }
                    _ => state.narrative.accepted_facts.retain(|id| *id != terminal),
                }
                let candidate = model::checkpoint(current.basis(), state).unwrap();
                assert_eq!(
                    phase(&candidate),
                    Err(RepositoryError::InvalidCandidate),
                    "mutation={mutation}"
                );
            }
        });
    }

    #[test]
    fn current_tactical_profile_preserves_e01_fixture_scenes_and_noncanonical_fight_replay() {
        use super::super::super::beat_script_variety_tests as fixture;
        let fixtures: [(&[u8], &[&str]); 4] = [
            (
                include_bytes!("../fixtures/third-approach-beat-script.json"),
                &["begin-story", "escort-courier", "defend-courier"],
            ),
            (
                include_bytes!("../fixtures/no-fight-beat-script.json"),
                &[
                    "begin-story",
                    "escort-courier",
                    "continue-conversation",
                    "choose-harbor-scene",
                ],
            ),
            (
                include_bytes!("../fixtures/branch-beat-script.json"),
                &[
                    "begin-story",
                    "ask-courier",
                    "continue-conversation",
                    "choose-harbor-scene",
                ],
            ),
            (
                include_bytes!("../fixtures/investigate-beat-script.json"),
                &[
                    "begin-story",
                    "look-crates",
                    "leave-investigation",
                    "choose-harbor-scene",
                ],
            ),
        ];
        for (script, actions) in fixtures {
            let mut content: pack::PackFile = serde_json::from_slice(include_bytes!(
                "../../../../../content/harbor-journey/pack.json"
            ))
            .unwrap();
            content.beat_script = Some(serde_json::from_slice(script).unwrap());
            let loaded = pack::load_from_bytes(
                &serde_json::to_vec(&content).unwrap(),
                include_bytes!("../../../../../content/harbor-journey/tuning.json"),
            )
            .unwrap();
            pack::with_test_pack(loaded, || {
                let mut current = fixture::prepared();
                for (index, id) in actions.iter().enumerate() {
                    current = fixture::act(&current, fixture::first(), index as u8 + 5, id);
                }
                if present(current.state()) {
                    assert!(read(current.state()).unwrap().combatants().len() >= 4);
                    // Spend a Second Wind use first, so the closing rest
                    // has a use to regain.
                    let current = spend_second_wind(&current, 8);
                    let (finished, evidence, _) = play_fight(current.clone(), 9, None, None);
                    let (_, replay, _) =
                        play_fight(current, 9, Some(&finished.state().draws), None);
                    assert_eq!(evidence, replay);
                    assert!(
                        offered(&finished, fixture::first())
                            .unwrap()
                            .iter()
                            .any(|(kind, _)| *kind == rpc::GameplayActionKind::ShortRest)
                    );
                    let rested = fixture::act(&finished, fixture::first(), 201, "short-rest");
                    assert!(
                        rested
                            .state()
                            .narrative
                            .completed_beats
                            .contains(&model::content("short-rest-complete").unwrap())
                    );
                    assert!(
                        !super::super::super::beat_script_runtime::action_label(
                            "opaque-tactical",
                            rpc::GameplayActionKind::Move
                        )
                        .unwrap()
                        .is_empty()
                    );
                } else {
                    assert_eq!(phase(&current).unwrap(), rpc::JourneyPhase::Complete);
                    assert!(current.state().draws.is_empty());
                }
            });
        }
    }

    #[test]
    fn submitted_archer_shot_uses_cover_consumes_ammunition_and_empty_quiver_removes_longbow_offer()
    {
        use rpc::GameplayActionKind as K;
        with_live(|| {
            let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
            let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
            let mut current = fight(true);
            let move_id=options(current.state(),1,K::Move).unwrap().into_iter().find(|(_,action,_)|matches!(action,Action::Move(path) if path.last()==Some(&tactical_rules::Cell(4,3)))).unwrap().0;
            current = submit(
                &current,
                first,
                &request(&current, 8, K::Move, Some(&move_id)),
                &mut |_| panic!("move has no draw"),
            );
            current = submit(
                &current,
                first,
                &request(&current, 9, K::EndTurn, None),
                &mut |_| panic!("next hero has no draw"),
            );
            let shot=options(current.state(),2,K::GreatswordAttack).unwrap().into_iter().find(|(_,action,_)|matches!(action,Action::Attack {target:4,weapon,..} if weapon.weapon==Weapon::Longbow && !weapon.use_mastery && !weapon.savage_attacker)).unwrap().0;
            let mut view = rpc::CombatView::default();
            project_combat(&current, Some(entity(ENTITIES[1]).unwrap()), &mut view).unwrap();
            assert_eq!(
                view.legal_targets
                    .iter()
                    .find(|target| target.option_id == shot)
                    .unwrap()
                    .cover,
                "half (+2 AC)"
            );
            current = submit(
                &current,
                second,
                &request(&current, 10, K::GreatswordAttack, Some(&shot)),
                &mut |_| Ok(6),
            );
            let receipt = accepted(current.state().decisions.last().unwrap()).unwrap();
            assert_eq!(receipt.combat[0].attack_total, 13);
            assert!(!receipt.combat[0].hit); // Source AC12 plus Half Cover2.
            assert_eq!(
                read(current.state()).unwrap().actor(2).unwrap().arrows(),
                19
            );
            let mut shots = 1;
            for operation in 11..110 {
                if shots == 20 {
                    break;
                }
                let battle = read(current.state()).unwrap();
                let actor = battle
                    .reaction_offers()
                    .first()
                    .copied()
                    .or(battle.active_actor())
                    .unwrap();
                let member = if actor == 1 { first } else { second };
                let ranged=options(current.state(),2,K::GreatswordAttack).unwrap().into_iter().find(|(_,action,_)|matches!(action,Action::Attack {weapon,..} if weapon.weapon==Weapon::Longbow));
                let (kind, option) = if !battle.reaction_offers().is_empty() {
                    (K::DeclineReaction, None)
                } else if actor == 2 && ranged.is_some() {
                    shots += 1;
                    (K::GreatswordAttack, ranged.map(|(id, _, _)| id))
                } else {
                    (K::EndTurn, None)
                };
                current = submit(
                    &current,
                    member,
                    &request(&current, operation, kind, option.as_deref()),
                    &mut |_| Ok(1),
                );
            }
            for operation in 110..114 {
                let battle = read(current.state()).unwrap();
                if battle.active_actor() == Some(2)
                    && !battle.actor(2).unwrap().budget().action_used
                {
                    break;
                }
                let actor = battle
                    .reaction_offers()
                    .first()
                    .copied()
                    .or(battle.active_actor())
                    .unwrap();
                current = submit(
                    &current,
                    if actor == 1 { first } else { second },
                    &request(
                        &current,
                        operation,
                        if battle.reaction_offers().is_empty() {
                            K::EndTurn
                        } else {
                            K::DeclineReaction
                        },
                        None,
                    ),
                    &mut |_| Ok(1),
                );
            }
            assert!(
                !read(current.state())
                    .unwrap()
                    .actor(2)
                    .unwrap()
                    .budget()
                    .action_used
            );
            assert_eq!(shots, 20);
            assert_eq!(read(current.state()).unwrap().actor(2).unwrap().arrows(), 0);
            assert!(options(current.state(),2,K::GreatswordAttack).unwrap().iter().all(|(_,action,_)|!matches!(action,Action::Attack {weapon,..} if weapon.weapon==Weapon::Longbow)));
            assert_eq!(
                current
                    .state()
                    .inventory
                    .iter()
                    .find(|item| item.owner == entity(ENTITIES[1]).unwrap()
                        && item.origin.entry.as_str() == "arrow")
                    .map_or(0, |item| item.quantity),
                0
            );
        });
    }
    fn request(
        current: &Checkpoint,
        operation: u8,
        kind: rpc::GameplayActionKind,
        option: Option<&str>,
    ) -> rpc::SubmitActionRequest {
        rpc::SubmitActionRequest {
            session_id: Some(rpc::SessionId {
                value: Some(current.basis().session.as_bytes().to_vec()),
            }),
            run_id: Some(rpc::RunId {
                value: Some(current.basis().run.as_bytes().to_vec()),
            }),
            observed_revision: Some(wire::revision(current.basis().revision)),
            operation_id: Some(rpc::OperationId {
                value: Some(vec![operation; 16]),
            }),
            offer_id: offer_id(current, kind),
            action_kind: kind as i32,
            action_choices: option
                .map(|id| {
                    vec![rpc::JourneyChoice {
                        group_id: "tactical-choice".into(),
                        option_id: id.into(),
                    }]
                })
                .unwrap_or_default(),
            ..Default::default()
        }
    }
    fn submit(
        current: &Checkpoint,
        member: MemberId,
        request: &rpc::SubmitActionRequest,
        supplier: &mut dyn FnMut(u32) -> Result<u32, RepositoryError>,
    ) -> Checkpoint {
        let kind = rpc::GameplayActionKind::try_from(request.action_kind).unwrap();
        assert_eq!(
            wire::action_offer_rejection(
                current,
                member,
                kind,
                &request.offer_id,
                &request.action_choices
            )
            .unwrap(),
            None
        );
        let input = wire::journey_input(request, member, current).unwrap();
        stage_with_supplier(current, &input, supplier).unwrap_or_else(|error| {
            panic!(
                "submitted operation {:?}, kind {kind:?}, phase {:?}: {error:?}",
                request.operation_id,
                phase(current)
            )
        })
    }
    #[test]
    fn submitted_tactical_wire_commands_refuse_illegal_options_wrong_turn_and_stale_offers_with_typed_codes()
     {
        use rpc::{GameplayActionKind as K, RejectionCode as R};
        with_live(|| {
            let current = fight(true);
            let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
            let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
            let before =
                df_persistence::local_demo_scope::encode_owned_demo_checkpoint(&current, codec())
                    .unwrap();
            for (member, kind, option, expected) in [
                (first, K::Move, Some("999-999"), R::InvalidSelection),
                (
                    first,
                    K::GreatswordAttack,
                    Some("greatsword-999-action"),
                    R::InvalidSelection,
                ),
                (second, K::EndTurn, None, R::WrongTurn),
            ] {
                let request = request(&current, 8, kind, option);
                assert_eq!(
                    wire::action_offer_rejection(
                        &current,
                        member,
                        kind,
                        &request.offer_id,
                        &request.action_choices
                    )
                    .unwrap(),
                    Some(expected)
                );
                assert!(wire::journey_input(&request, member, &current).is_ok());
                assert_eq!(
                    df_persistence::local_demo_scope::encode_owned_demo_checkpoint(
                        &current,
                        codec()
                    )
                    .unwrap(),
                    before
                );
            }
            let stale = request(&current, 8, K::Move, Some("999-999"));
            let advanced = submit(
                &current,
                first,
                &request(&current, 8, K::Dash, None),
                &mut |_| panic!("dash has no draw"),
            );
            assert_eq!(
                wire::action_offer_rejection(
                    &advanced,
                    first,
                    K::Move,
                    &stale.offer_id,
                    &stale.action_choices
                )
                .unwrap(),
                Some(R::StaleOffer)
            );
        });
    }
    #[test]
    fn submitted_disengage_prevents_source_opportunity_window_while_enemy_policy_declines_optional_attack()
     {
        use rpc::GameplayActionKind as K;
        with_live(|| {
            let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
            let current = fight(true);
            let cell_option = |current: &Checkpoint, cell| {
                options(current.state(),1,K::Move).unwrap().into_iter().find(|(_,action,_)|matches!(action,Action::Move(path) if path.last()==Some(&cell))).unwrap().0
            };
            let adjacent = cell_option(&current, tactical_rules::Cell(3, 2));
            let current = submit(
                &current,
                first,
                &request(&current, 8, K::Move, Some(&adjacent)),
                &mut |_| panic!("approach draws"),
            );
            let exit = cell_option(&current, tactical_rules::Cell(3, 3));
            let source = rule().unwrap();
            let operation = OperationId::from_bytes(&[9; 16]).unwrap();
            let movement = options(current.state(), 1, K::Move)
                .unwrap()
                .into_iter()
                .find(|(id, _, _)| *id == exit)
                .unwrap()
                .1;
            let pending = tactical_rules::resolve(
                &read(current.state()).unwrap(),
                1,
                &movement,
                binding(operation, &source, 0).unwrap(),
                &[],
            )
            .unwrap()
            .battle;
            assert!(!pending.reaction_offers().is_empty());
            let exited = submit(
                &current,
                first,
                &request(&current, 9, K::Move, Some(&exit)),
                &mut |_| panic!("enemy policy declines optional attack"),
            );
            assert!(read(exited.state()).unwrap().reaction_offers().is_empty());
            let disengaged = submit(
                &current,
                first,
                &request(&current, 9, K::Disengage, None),
                &mut |_| panic!("Disengage has no draw"),
            );
            let exit = cell_option(&disengaged, tactical_rules::Cell(3, 3));
            let movement = options(disengaged.state(), 1, K::Move)
                .unwrap()
                .into_iter()
                .find(|(id, _, _)| *id == exit)
                .unwrap()
                .1;
            let prevented = tactical_rules::resolve(
                &read(disengaged.state()).unwrap(),
                1,
                &movement,
                binding(operation, &source, 0).unwrap(),
                &[],
            )
            .unwrap()
            .battle;
            assert!(prevented.reaction_offers().is_empty());
            let exited = submit(
                &disengaged,
                first,
                &request(&disengaged, 10, K::Move, Some(&exit)),
                &mut |_| panic!("Disengage prevents opportunity draw"),
            );
            // The exit is recorded as a move with its path and cost, and nothing attacked it.
            let recorded = accepted(exited.state().decisions.last().unwrap())
                .unwrap()
                .combat;
            assert!(
                recorded
                    .iter()
                    .all(|outcome| outcome.kind == rpc::OutcomeKind::Move as i32)
            );
            assert!(
                recorded
                    .iter()
                    .any(|outcome| outcome.cost_ft > 0 && !outcome.path.is_empty())
            );
        });
    }

    #[test]
    fn submitted_optional_hero_reaction_during_enemy_turn_survives_checkpoint_and_declines_without_spending()
     {
        use rpc::GameplayActionKind as K;
        with_live(|| {
            let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
            let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
            let current = fight(true);
            let adjacent=options(current.state(),1,K::Move).unwrap().into_iter().find(|(_,action,_)|matches!(action,Action::Move(path) if path.last()==Some(&tactical_rules::Cell(3,2)))).unwrap().0;
            let current = submit(
                &current,
                first,
                &request(&current, 8, K::Move, Some(&adjacent)),
                &mut |_| panic!("move draws"),
            );
            let current = submit(
                &current,
                first,
                &request(&current, 9, K::EndTurn, None),
                &mut |_| Ok(1),
            );
            let current = submit(
                &current,
                second,
                &request(&current, 10, K::EndTurn, None),
                &mut |_| Ok(1),
            );
            let battle = read(current.state()).unwrap();
            assert_eq!(battle.reaction_offers().first(), Some(&1));
            assert_ne!(battle.active_actor(), Some(1));
            assert!(wire::current_combat_turn(&current, first).unwrap());
            let bytes =
                df_persistence::local_demo_scope::encode_owned_demo_checkpoint(&current, codec())
                    .unwrap();
            let resumed = df_persistence::local_demo_scope::decode_owned_demo_checkpoint(
                &bytes,
                current.basis(),
                &model::pins().unwrap(),
                &model::recovery().unwrap(),
                codec(),
            )
            .unwrap();
            assert_eq!(
                groups(&current, first, K::OpportunityAttack).unwrap(),
                groups(&resumed, first, K::OpportunityAttack).unwrap()
            );
            let command = request(&current, 11, K::DeclineReaction, None);
            let declined = submit(&current, first, &command, &mut |_| Ok(1));
            let replayed = submit(&resumed, first, &command, &mut |_| Ok(1));
            assert_eq!(declined, replayed);
            assert!(
                !read(declined.state())
                    .unwrap()
                    .actor(1)
                    .unwrap()
                    .budget()
                    .reaction_used
            );
        });
    }
    #[test]
    fn submitted_enemy_lethality_reaches_defeat_and_timed_dying_saves_without_universal_rest_revival()
     {
        use rpc::GameplayActionKind as K;
        with_live(|| {
            let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
            let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
            let mut current = fight(true);
            let mut saw_dying = false;
            for operation in 8..70 {
                if phase(&current).unwrap() == rpc::JourneyPhase::Complete {
                    break;
                }
                let member = [first, second]
                    .into_iter()
                    .find(|member| wire::current_combat_turn(&current, *member).unwrap())
                    .unwrap();
                current = submit(
                    &current,
                    member,
                    &request(&current, operation, K::EndTurn, None),
                    &mut |sides| Ok(if sides == 20 { 20 } else { sides.min(3) }),
                );
                saw_dying |= dying(current.state()).unwrap();
            }
            assert!(saw_dying);
            assert_eq!(phase(&current).unwrap(), rpc::JourneyPhase::Complete);
            assert!(!victory(current.state()).unwrap());
            assert_eq!(
                super::super::super::beat_script_runtime::scene(current.state())
                    .unwrap()
                    .key,
                pack::SceneKey::Defeat
            );
            let defeat = current.clone();
            for operation in 70..78 {
                if !dying(current.state()).unwrap() {
                    break;
                }
                let ticks = current.state().logical_time.ticks;
                current = submit(
                    &current,
                    first,
                    &request(&current, operation, K::EndTurn, None),
                    &mut |_| Ok(1),
                );
                assert!(current.state().logical_time.ticks > ticks);
            }
            assert!(!dying(current.state()).unwrap());
            assert!(
                read(current.state())
                    .unwrap()
                    .combatants()
                    .iter()
                    .filter(|actor| actor.side() == Side::Heroes)
                    .all(|actor| actor.health().dead)
            );
            assert!(
                offered(&current, first)
                    .unwrap()
                    .iter()
                    .all(|(kind, _)| *kind != K::ShortRest)
            );
            assert_eq!(
                stage_with_supplier(
                    &current,
                    &input(&current, first, 79, "short-rest", vec![]),
                    &mut |_| panic!("refused rest draws")
                ),
                Err(RepositoryError::InvalidCandidate)
            );
            let revived = submit(
                &defeat,
                first,
                &request(&defeat, 80, K::EndTurn, None),
                &mut |_| Ok(20),
            );
            assert!(!victory(revived.state()).unwrap());
            assert_eq!(phase(&revived).unwrap(), rpc::JourneyPhase::Complete);
            assert!(!dying(revived.state()).unwrap());
            let view = match wire::journey_view(
                &revived,
                df_persistence::local_demo_scope::LocalDemoRole::Player,
                first,
            )
            .unwrap()
            .audience
            .unwrap()
            {
                rpc::view_message::Audience::Player(view) => view.journey.unwrap(),
                _ => panic!("player audience"),
            };
            // A settled defeat is a story scene: the view carries no fight to act in.
            assert!(view.combat.is_none());
            assert!(
                offered(&revived, first)
                    .unwrap()
                    .iter()
                    .any(|(kind, _)| *kind == K::ShortRest),
                "handler={:?} authored={:?} active={:?}",
                handler_offered(&revived, first),
                authored_offered(&revived, first),
                revived.state().narrative
            );
            let rested = submit(
                &revived,
                first,
                &request(&revived, 81, K::ShortRest, None),
                &mut |_| panic!("rest has no draw"),
            );
            assert!(!victory(rested.state()).unwrap());
            assert!(
                read(rested.state())
                    .unwrap()
                    .combatants()
                    .iter()
                    .filter(|actor| actor.side() == Side::Heroes)
                    .all(|actor| actor.health().hit_points == 1)
            );
        });
    }

    /// The generated adventure of the live fight that left a hero dying after the party won: the
    /// same seed, location and antagonist, three Kobold Warriors against the two heroes, with the
    /// authored beat script of a generated pack (not the canonical harbour one).
    fn live_generated_pack() -> pack::LoadedPack {
        use crate::gameplay::adventure::{
            ForcedEntries, GenerateOptions, RulesMode, Tables, generate,
        };
        let tables = Tables::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../content/adventure-tables")
                .as_path(),
        )
        .unwrap();
        let generated = generate(
            &tables,
            0x575f_4310_795c_c7a2,
            GenerateOptions {
                rules_mode: RulesMode::CustomAllowed,
                ..GenerateOptions::default()
            },
            &ForcedEntries::from([
                ("location".into(), "orrery-library".into()),
                ("antagonist".into(), "kobold-warrior".into()),
            ]),
        )
        .unwrap();
        pack::load_from_bytes(
            &serde_json::to_vec(&generated.pack).unwrap(),
            &serde_json::to_vec(&generated.tuning).unwrap(),
        )
        .unwrap()
    }
    /// Searches seeds until the fight ends in victory with a hero at 0 hit points who is neither
    /// stable nor dead. Run inside `with_test_pack`.
    pub(in crate::gameplay::journey) fn victory_with_a_dying_hero() -> Checkpoint {
        for seed in 1..600_u64 {
            let (current, _, _) = play_fight(current_combat("escort-courier"), 8, None, Some(seed));
            if victory(current.state()).unwrap() && dying(current.state()).unwrap() {
                return current;
            }
        }
        panic!("no seed ends in victory with a dying hero");
    }
    fn combat_view(current: &Checkpoint, member: MemberId) -> rpc::CombatView {
        match wire::journey_view(
            current,
            df_persistence::local_demo_scope::LocalDemoRole::Player,
            member,
        )
        .unwrap()
        .audience
        .unwrap()
        {
            rpc::view_message::Audience::Player(view) => view.journey.unwrap().combat.unwrap(),
            _ => panic!("player audience"),
        }
    }
    /// Names the step that refuses an aftermath press, when one does.
    fn diagnose_refused_aftermath(
        current: &Checkpoint,
        member: MemberId,
        input: &GameInput,
    ) -> String {
        use rpc::GameplayActionKind as K;
        let GameInput::Game(command) = input else {
            return "not a game input".into();
        };
        let GameCommand::ProposeAction {
            action, targets, ..
        } = &command.command
        else {
            return "not a proposal".into();
        };
        let selected = crate::gameplay::beat_script_runtime::select(current, K::EndTurn, &[]);
        let validated = super::super::affordance::validate(current, member, action, targets);
        let mut state = current.state().clone();
        let (mut draws, mut outcomes) = (Vec::new(), Vec::new());
        let staged = super::stage(
            current,
            command,
            K::EndTurn,
            &mut state,
            &mut draws,
            &mut outcomes,
            &mut |_| Ok(12),
        );
        let phase = selected
            .as_ref()
            .map(|offer| super::super::phase_from_state(current, &state, &offer.id));
        format!(
            "select {:?}; affordance {validated:?}; tactical stage {staged:?}; phase_from_state {phase:?}",
            selected.as_ref().map(|offer| offer.id.clone())
        )
    }
    /// A won fight with a hero at 0 hit points was a dead end on the live table: the only offer,
    /// "Advance 6 seconds", was refused. Time now advances by the rules (a Death Saving Throw per
    /// round, then 1d4 hours for a Stable hero) until the hero is dead, stable and recovered, or
    /// back on their feet, every step is accepted, every roll is in the view, and the table can
    /// then rest or move on.
    #[test]
    fn a_won_fight_with_a_dying_hero_resolves_on_the_table_without_a_dead_end() {
        use rpc::GameplayActionKind as K;
        pack::with_test_pack(live_generated_pack(), || {
            let won = victory_with_a_dying_hero();
            let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
            let offers = offered(&won, first).unwrap();
            assert_eq!(offers.len(), 1, "{offers:?}");
            assert_eq!(offers[0].0, K::EndTurn);
            assert!(offers[0].1.contains("death saves"));
            let downed = read(won.state())
                .unwrap()
                .combatants()
                .iter()
                .find(|actor| actor.side() == Side::Heroes && actor.health().hit_points == 0)
                .map(|actor| actor.id())
                .unwrap();
            for (face, dies, revives) in
                [(15_u32, false, false), (2, true, false), (20, false, true)]
            {
                let mut current = won.clone();
                let mut rolls = Vec::new();
                let mut recovery_hours = 0;
                for operation in 150..180_u8 {
                    if !dying(current.state()).unwrap() {
                        break;
                    }
                    let ticks = current.state().logical_time.ticks;
                    let pressed = request(&current, operation, K::EndTurn, None);
                    if let Ok(input) = wire::journey_input(&pressed, first, &current)
                        && stage_with_supplier(&current, &input, &mut |sides| {
                            Ok(if sides == 20 { face } else { 3 })
                        })
                        .is_err()
                    {
                        panic!(
                            "aftermath press refused: {}",
                            diagnose_refused_aftermath(&current, first, &input)
                        );
                    }
                    current = submit(&current, first, &pressed, &mut |sides| {
                        Ok(if sides == 20 { face } else { 3 })
                    });
                    assert!(current.state().logical_time.ticks > ticks);
                    // What this press recorded, and that the view carries it.
                    let recorded = accepted(current.state().decisions.last().unwrap())
                        .unwrap()
                        .combat;
                    let view = combat_view(&current, first);
                    for outcome in recorded.iter().filter(|outcome| {
                        outcome.kind == rpc::OutcomeKind::DeathSave as i32
                            || outcome.kind == rpc::OutcomeKind::Recovery as i32
                    }) {
                        let mut expected = outcome.clone();
                        expected.sequence = 0;
                        let view_outcome = view
                            .death_save_outcomes
                            .iter()
                            .find(|view_outcome| {
                                let mut view_outcome = (*view_outcome).clone();
                                view_outcome.sequence = 0;
                                view_outcome == expected
                            })
                            .expect("the view must carry every recorded roll");
                        assert!(view_outcome.sequence > 0);
                    }
                    rolls.extend(
                        recorded
                            .iter()
                            .filter(|outcome| outcome.kind == rpc::OutcomeKind::DeathSave as i32)
                            .map(|outcome| {
                                (
                                    outcome.attack_die,
                                    outcome.death_successes,
                                    outcome.death_failures,
                                )
                            }),
                    );
                    recovery_hours += recorded
                        .iter()
                        .filter(|outcome| outcome.kind == rpc::OutcomeKind::Recovery as i32)
                        .map(|outcome| outcome.recovery_hours)
                        .max()
                        .unwrap_or(0);
                    assert!(
                        view.death_save_outcomes
                            .windows(2)
                            .all(|pair| pair[0].sequence < pair[1].sequence)
                    );
                    assert!(view.outcomes.iter().all(|outcome| {
                        outcome.kind != rpc::OutcomeKind::Move as i32
                            && outcome.kind != rpc::OutcomeKind::DeathSave as i32
                    }));
                }
                assert!(!dying(current.state()).unwrap(), "face {face}: still dying");
                assert!(
                    !rolls.is_empty(),
                    "face {face}: no Death Saving Throw reached the view"
                );
                assert!(rolls.iter().all(|(die, _, _)| *die == face));
                let healed = read(current.state())
                    .unwrap()
                    .actor(downed)
                    .unwrap()
                    .health();
                assert_eq!(healed.dead, dies, "face {face}");
                if !dies {
                    assert_eq!(healed.hit_points, 1, "face {face}");
                }
                assert_eq!(recovery_hours > 0, !dies && !revives, "face {face}");
                // Not a dead end: the table is offered what the story allows next, and it works.
                let next = offered(&current, first).unwrap();
                assert!(
                    next.iter()
                        .any(|(kind, _)| matches!(kind, K::ShortRest | K::ChooseHarborScene)),
                    "face {face}: {next:?}"
                );
                if !dies {
                    let rested = submit(
                        &current,
                        first,
                        &request(&current, 190, K::ShortRest, None),
                        &mut |_| panic!("rest has no draw"),
                    );
                    assert_eq!(phase(&rested).unwrap(), rpc::JourneyPhase::Complete);
                }
            }
        });
    }
}

#[cfg(test)]
mod shared_scene_tests {
    use super::*;
    use tactical_rules::{BattleSceneData, Cover};

    fn cell(col: i32, row: i32) -> df_render::battle::Cell {
        df_render::battle::Cell::new(col, row)
    }

    /// Every scene the server can stage a fight on, with its layouts as the rules parse them and
    /// as the renderer parses the same file.
    #[test]
    fn every_scan_layout_the_server_can_choose_is_valid_for_the_rules_and_matches_the_renderer() {
        let mut scans = 0;
        for id in scene_ids() {
            let bytes = scene_bytes(id).unwrap();
            let data: BattleSceneData = serde_json::from_slice(bytes).unwrap();
            let render = df_render::battle::load_embedded(id)
                .unwrap_or_else(|| panic!("{id}: the renderer has no scene of that id"))
                .unwrap_or_else(|error| panic!("{id}: {error:?}"));
            assert_eq!(data.layouts.len(), render.layouts.len(), "{id}");
            assert!(!data.layouts.is_empty(), "{id}: a layout to choose from");
            scans += usize::from(id.starts_with("scan-"));
            for layout in &data.layouts {
                // The rules accept it: every spawn is walkable and each side reaches the other.
                let map = BattleMap::from_json(bytes, &layout.id)
                    .unwrap_or_else(|error| panic!("{id}/{}: {error:?}", layout.id));
                let drawn = render
                    .layouts
                    .iter()
                    .find(|candidate| candidate.id == layout.id)
                    .unwrap_or_else(|| panic!("{id}/{}: the renderer lacks the layout", layout.id));
                // The renderer places everyone on the same spawn cells...
                let as_rules = |cells: &[df_render::battle::Cell]| -> Vec<(u16, u16)> {
                    cells
                        .iter()
                        .map(|c| (u16::try_from(c.col).unwrap(), u16::try_from(c.row).unwrap()))
                        .collect()
                };
                let rules_cells = |cells: &[tactical_rules::Cell]| -> Vec<(u16, u16)> {
                    cells.iter().map(|c| (c.0, c.1)).collect()
                };
                assert_eq!(
                    as_rules(&drawn.heroes),
                    rules_cells(map.hero_spawns()),
                    "{id}/{}",
                    layout.id
                );
                assert_eq!(
                    as_rules(&drawn.enemies),
                    rules_cells(map.enemy_spawns()),
                    "{id}/{}",
                    layout.id
                );
                // ... on a grid that blocks exactly the cells the rules block.
                let grid = render.grid.with_layout(drawn);
                let (cols, rows) = map.dimensions();
                assert_eq!(
                    (u32::from(cols), u32::from(rows)),
                    (grid.cols(), grid.rows())
                );
                for row in 0..i32::from(rows) {
                    for col in 0..i32::from(cols) {
                        let blocked = map.is_blocked(tactical_rules::Cell(col as u16, row as u16));
                        assert_eq!(
                            blocked,
                            !grid.is_walkable(cell(col, row)),
                            "{id}/{}: {col},{row}",
                            layout.id
                        );
                    }
                }
                // Every spawn is on an open cell, both sides can reach each other.
                for spawn in map.hero_spawns().iter().chain(map.enemy_spawns()) {
                    assert!(!map.is_blocked(*spawn), "{id}/{}: {spawn:?}", layout.id);
                }
                for hero in &drawn.heroes {
                    let reach = grid.reachable_from(*hero);
                    assert!(
                        drawn.enemies.iter().all(|enemy| reach.contains(enemy)),
                        "{id}/{}",
                        layout.id
                    );
                }
                // Cover sits on blocked cells and works along the line between cell centres: two
                // open cells on either side of a cover cell see each other through that cover.
                for obstacle in &layout.obstacles {
                    if obstacle.cover == Cover::None {
                        continue;
                    }
                    let mut proved = false;
                    for &tactical_rules::Cell(col, row) in &obstacle.cells {
                        for (dc, dr) in [(1_i32, 0_i32), (0, 1), (1, 1), (1, -1)] {
                            let before = (i32::from(col) - dc, i32::from(row) - dr);
                            let after = (i32::from(col) + dc, i32::from(row) + dr);
                            let open = |(c, r): (i32, i32)| {
                                c >= 0
                                    && r >= 0
                                    && c < i32::from(cols)
                                    && r < i32::from(rows)
                                    && !map.is_blocked(tactical_rules::Cell(c as u16, r as u16))
                            };
                            if open(before) && open(after) {
                                let from = tactical_rules::Cell(before.0 as u16, before.1 as u16);
                                let to = tactical_rules::Cell(after.0 as u16, after.1 as u16);
                                let cover = map.cover_between(from, to, &[]).unwrap();
                                assert!(
                                    cover >= obstacle.cover,
                                    "{id}/{}: {} through {col},{row}",
                                    layout.id,
                                    obstacle.id
                                );
                                proved = true;
                            }
                        }
                    }
                    // A cover object with no open cells on both sides still blocks movement.
                    for &tactical_rules::Cell(col, row) in &obstacle.cells {
                        assert!(
                            map.is_blocked(tactical_rules::Cell(col, row)),
                            "{id}/{}: {} blocks",
                            layout.id,
                            obstacle.id
                        );
                    }
                    let _ = proved;
                }
            }
        }
        assert_eq!(scans, 4, "the four scan scenes are authored for the server");
    }

    fn kobolds(count: u8) -> Vec<pack::TacticalEnemy> {
        vec![pack::TacticalEnemy {
            srd_id: "kobold-warrior".to_owned(),
            count,
            ammunition: 0,
        }]
    }

    #[test]
    fn a_test_fight_replaces_the_adventures_enemies_and_without_one_they_are_unchanged() {
        let own = vec![
            pack::TacticalEnemy {
                srd_id: "scout".to_owned(),
                count: 1,
                ammunition: 20,
            },
            pack::TacticalEnemy {
                srd_id: "bandit".to_owned(),
                count: 2,
                ammunition: 7,
            },
        ];
        let ids =
            |roster: Vec<(String, u16)>| roster.into_iter().map(|(id, _)| id).collect::<Vec<_>>();

        assert_eq!(
            ids(roster(&own, None).unwrap()),
            ["scout", "bandit", "bandit"]
        );
        let of_own = TestFight {
            enemies: 4,
            foe: None,
        };
        assert_eq!(
            roster(&own, Some(&of_own)).unwrap(),
            vec![("scout".to_owned(), 20); 4]
        );
        let wolves = TestFight {
            enemies: 1,
            foe: Some("wolf".to_owned()),
        };
        assert_eq!(
            roster(&kobolds(3), Some(&wolves)).unwrap(),
            vec![("wolf".to_owned(), TEST_FOE_AMMUNITION)]
        );
    }

    #[test]
    fn every_scene_the_server_holds_has_a_typed_lighting_fact_and_only_the_scans_are_sunlit() {
        for id in scene_ids() {
            let (_, lighting) = LIGHTING
                .iter()
                .find(|(known, _)| *known == id)
                .unwrap_or_else(|| panic!("scene {id} has no lighting fact"));
            assert_eq!(
                *lighting == tactical_rules::Lighting::Sunlight,
                id.starts_with("scan-"),
                "{id}"
            );
        }
        assert_eq!(LIGHTING.len(), scene_ids().count());
        // A real battle map carries the fact the rules read.
        let village = scene_bytes("scan-village").unwrap();
        assert_eq!(
            scene_map(village, "north-south").unwrap().lighting(),
            tactical_rules::Lighting::Sunlight
        );
        let quay = scene_bytes("salt-quay").unwrap();
        assert_eq!(
            scene_map(quay, "open-quay").unwrap().lighting(),
            tactical_rules::Lighting::NightOrInterior
        );
    }

    #[test]
    fn a_forced_scene_must_be_one_the_server_holds() {
        assert!(force_scene(Some("nowhere")).is_err());
    }

    #[test]
    fn the_server_stages_a_fight_on_the_story_location_unless_a_scene_is_forced() {
        // Not forced in this process: the story place stands.
        assert_eq!(staged_location("salt-quay".to_owned()), "salt-quay");
    }
}

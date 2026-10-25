//! Canonical room, authored story and source-qualified combat composition.
use df_model::checkpoint::*;
use df_persistence::local_demo_scope::PLAYER;
use df_protocol::common as rpc;
use df_rules::local_journey as rules;
use df_session::submission::RepositoryError;
use df_types::{MemberId, OperationId};
use prost::Message;
use std::time::Duration;

use super::{model, pack, refusal_detail, wire};

#[cfg(test)]
#[path = "journey_tests.rs"]
pub(super) mod checkpoint_fight_tests;

pub(super) mod affordance;
#[cfg(test)]
mod affordance_tests;
mod combat_transition;
#[cfg(test)]
mod combat_transition_tests;
pub(super) mod courier_reaction;
#[cfg(test)]
#[path = "creation_tests.rs"]
mod creation_tests;
pub(super) mod encounter_admission;
mod encounter_outcome;
mod enemy_tactics;
#[cfg(test)]
mod fight_ending_tests;
pub(super) mod living_scene;
#[cfg(test)]
mod living_scene_tests;
pub(super) mod merchant_bargaining;
#[cfg(test)]
mod merchant_bargaining_tests;
mod narrative_phase;
#[cfg(test)]
mod narrative_runtime_tests;
mod narrative_transition;
mod objective_outcome;
#[cfg(test)]
mod objective_outcome_tests;
pub(super) mod rewards;
pub(super) mod scene_fight;
#[cfg(test)]
mod session_flow_tests;
pub(super) mod session_machine;
#[cfg(test)]
mod sheet_modifier_tests;
pub(super) mod tactical;
#[cfg(test)]
mod text_pack_tests;

pub(super) const ROOM_CODE: &str = "LANTERN";
pub(super) fn room_code() -> String {
    pack::with_content(|pack| pack.room_code.clone())
}
pub(super) const ROOM_ENTITY: [u8; 16] = [0x45; 16];
pub(super) use df_rules::local_journey::MAXIMUM_PARTY;
const MEMBERS: [[u8; 16]; MAXIMUM_PARTY] = [
    [0x61; 16], [0x62; 16], [0x81; 16], [0x82; 16], [0x83; 16], [0x84; 16],
];
const ENTITIES: [[u8; 16]; MAXIMUM_PARTY] = [
    [0x63; 16], [0x64; 16], [0x75; 16], [0x76; 16], [0x77; 16], [0x78; 16],
];
const BANDIT: [u8; 16] = [0x65; 16];
const ENCOUNTER: [u8; 16] = [0x66; 16];
pub(super) const THREAD_POLICY: &str = "local-journey-rpc-3-threads-1-rest-1-inn-1-npc-1";
const PACKET_THREAD: &str = "sealed-packet-delivery-thread";
const THREAT_THREAD: &str = "dockside-threat-thread";
// This fixed authored slice admits one causal event to one thread. Packet delivery is outside
// this journey; combat victory resolves only the dockside threat and never delivers the packet.
const THREAD_EVENTS: &[(&str, &str)] = &[
    ("begin-story", PACKET_THREAD),
    ("private-courier-note", PACKET_THREAD),
    ("escort-courier", PACKET_THREAD),
    ("harbor-inn", PACKET_THREAD),
    ("defend-courier", THREAT_THREAD),
    ("scene-npc-assault", THREAT_THREAD),
    ("greatsword-attack", THREAT_THREAD),
    ("second-wind", THREAT_THREAD),
    ("action-surge", THREAT_THREAD),
    ("move", THREAT_THREAD),
    ("dash", THREAT_THREAD),
    ("dodge", THREAT_THREAD),
    ("disengage", THREAT_THREAD),
    ("opportunity-attack", THREAT_THREAD),
    ("decline-reaction", THREAT_THREAD),
    ("stand", THREAT_THREAD),
    ("pick-up-weapon", THREAT_THREAD),
    ("end-turn", THREAT_THREAD),
];
// `scene-person` is the definition of every world entity a scene assault creates for a scene
// person (Marek, Corin, Jessa); their identity labels carry the person id (`scene_fight`).
pub(super) const CONTENT_ENTRIES: &[&str] = &[
    "scene-interaction",
    "scene-person",
    "scene-assault",
    "npc-scene-interaction",
    "merchant-trade-quote",
    "merchant-quote-closure",
    "stabilise-kit",
    "stabilise-medicine",
    "help-attack",
    rewards::PAID,
    "room",
    "join-room",
    "start-new-adventure",
    "joined-player",
    "dwarf-fighter-soldier",
    "create-character",
    "begin-story",
    "opening",
    "dialogue",
    "ask-courier",
    "escort-courier",
    "courier-answer-seal",
    "courier-answer-escort",
    "private-courier-note",
    "defend-courier",
    "scene-npc-assault",
    "combat",
    "complete",
    "choose-harbor-scene",
    "harbor-inn",
    "bandit",
    "greatsword-attack",
    "second-wind",
    "action-surge",
    "move",
    "dash",
    "dodge",
    "disengage",
    "opportunity-attack",
    "decline-reaction",
    "stand",
    "pick-up-weapon",
    "end-turn",
    "short-rest",
    "short-rest-start",
    "knockout-short-rest-start",
    "knockout-short-rest-due",
    "short-rest-complete",
    "short-rest-interrupted",
    "combat-victory",
    "combat-defeat",
    "scimitar",
    "club",
    "normal-nonlethal-melee",
    "tactical-srd521-cover-policy",
    "fighter-a-soldier-a",
    "chain-mail",
    "greatsword",
    "flail",
    "javelin",
    "dungeoneer-pack",
    "spear",
    "shortbow",
    "arrow",
    "gaming-set",
    "healers-kit",
    "quiver",
    "travelers-clothes",
    "gold-piece",
    "studded-leather-armor",
    "longbow",
    "shortsword",
    PACKET_THREAD,
    THREAT_THREAD,
];
fn bad<T: std::fmt::Debug>(error: T) -> RepositoryError {
    refusal_detail::cause(
        "journey-value",
        format!("{}::{error:?}", std::any::type_name::<T>()),
    );
    RepositoryError::InvalidCandidate
}
pub(super) fn rule() -> Result<RuleReference, RepositoryError> {
    if tactical::enabled() {
        return Ok(RuleReference {
            catalog: model::label("srd521-journey-subset-2")?,
            source: model::label("srd521-creation-tactical-combat-rest")?,
            entry: model::label("PLAY-T02")?,
            clause: model::label("srd521-grid-actions-dying-cover-ruling")?,
        });
    }
    Ok(RuleReference {
        catalog: model::label("srd521-journey-subset-2")?,
        source: model::label("srd521-creation-combat-rest-selected")?,
        entry: model::label("dwarf-fighter-soldier-bandit")?,
        clause: model::label("normal-adjacent-nonlethal-and-no-hit-dice-rest")?,
    })
}
pub(super) fn entity(bytes: [u8; 16]) -> Result<EntityId, RepositoryError> {
    EntityId::from_bytes(&bytes).map_err(bad)
}
pub(super) fn room_entity() -> Result<EntityId, RepositoryError> {
    entity(ROOM_ENTITY)
}
pub(super) fn bootstrap_member() -> Result<MemberId, RepositoryError> {
    MemberId::from_bytes(&PLAYER).map_err(bad)
}
pub(super) fn player_entity(
    member: MemberId,
    current: &Checkpoint,
) -> Result<EntityId, RepositoryError> {
    current
        .state()
        .members
        .iter()
        .find(|link| link.member == member && member.as_bytes() != &PLAYER)
        .and_then(|link| link.character)
        .ok_or(RepositoryError::Unauthorized)
}
pub(super) fn participants(current: &Checkpoint) -> Vec<&MembershipLink> {
    current
        .state()
        .members
        .iter()
        .filter(|link| link.member.as_bytes() != &PLAYER)
        .collect()
}
pub(super) fn initial() -> Result<Checkpoint, RepositoryError> {
    let mut state = model::initial()?.state().clone();
    state.members = vec![MembershipLink {
        member: bootstrap_member()?,
        character: Some(room_entity()?),
    }];
    state.entities = vec![WorldEntity {
        id: room_entity()?,
        definition: model::content("room")?,
        location: None,
        position: None,
        identity_revision: model::label("local-room-1")?,
    }];
    state.characters.clear();
    state.narrative.definition = model::content("room")?;
    state.narrative.active_beats = vec![model::content("room")?];
    state.narrative.remaining_budget = pack::narrative_budget();
    model::checkpoint(model::basis()?, state)
}
// Deriving the phase and a hero's creation sheet re-validates every hero from their receipts,
// and every view asks many times; with six heroes and their starting gear that kept the owner
// thread CPU-bound and every call past its deadline (2026-10-24, live free roam). Both are pure
// functions of the game state, so the owner thread remembers the last few results by exact state.
thread_local! {
    static PHASES: std::cell::RefCell<std::collections::VecDeque<(GameState, pack::CacheIdentity, rpc::JourneyPhase)>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    static RECREATED: std::cell::RefCell<std::collections::VecDeque<(GameState, pack::CacheIdentity, GameState)>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}
const REMEMBERED_PHASES: usize = 8;
const REMEMBERED_RECREATIONS: usize = 4;

pub(super) fn phase(current: &Checkpoint) -> Result<rpc::JourneyPhase, RepositoryError> {
    // Resume admission includes pins, which an exact state comparison alone cannot check.
    current
        .validate_resume(current.basis(), &model::pins()?)
        .map_err(|error| {
            refusal_detail::cause("beat-script-checkpoint-resume", format!("{error:?}"));
            RepositoryError::InvalidCandidate
        })?;
    let pack = pack::cache_identity();
    let remembered = PHASES.with(|cache| {
        cache
            .borrow()
            .iter()
            .find(|(state, source, _)| *source == pack && state == current.state())
            .map(|(_, _, phase)| *phase)
    });
    if let Some(phase) = remembered {
        return Ok(phase);
    }
    let phase = match tactical::development_phase(current)? {
        Some(phase) => phase,
        None => derived_phase(current)?,
    };
    PHASES.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= REMEMBERED_PHASES {
            cache.pop_front();
        }
        cache.push_back((current.state().clone(), pack, phase));
    });
    Ok(phase)
}

fn derived_phase(current: &Checkpoint) -> Result<rpc::JourneyPhase, RepositoryError> {
    if let Some(phase) = super::beat_script::with_script(|script| {
        if script.uses_encounter() {
            None
        } else {
            Some(super::beat_script_runtime::validate(current, script))
        }
    }) {
        return phase;
    }
    let phase = narrative_phase::validate(current)
        .inspect_err(|_| refusal_detail::note("phase-narrative"))?;
    encounter_outcome::validate(current, phase)
        .inspect_err(|_| refusal_detail::note("phase-encounter"))?;
    super::beat_script::with_script(|script| {
        script.current(current.state())?;
        super::beat_script_runtime::validate_encounter_choices(current, script)
    })
    .inspect_err(|_| refusal_detail::note("phase-script-choices"))?;
    Ok(phase)
}
pub(super) fn resources() -> Result<Vec<ResourceConstraint>, RepositoryError> {
    resources_of(None)
}
/// The pack's rows for every owner, or only the rows of `only`. Each owner's rows are independent.
fn resources_of(only: Option<EntityId>) -> Result<Vec<ResourceConstraint>, RepositoryError> {
    let hero_hp = pack::with_creation(|rules| {
        Ok::<_, RepositoryError>(
            i64::from(rules.base_hit_points)
                // SRD 5.2.1 p. 87: Human Versatile can grant Tough (+2 HP per level).
                + 2
                + rules
                    .ability_arrays
                    .iter()
                    .map(|array| {
                        i64::from(rules::ability_modifier(
                            array.scores[2] + rules.background_bonus.constitution,
                        ))
                    })
                    .max()
                    .ok_or(RepositoryError::InvalidCandidate)?,
        )
    })?;
    let hero_ac = pack::with_creation(|rules| i64::from(rules.armor_class));
    let bandit = pack::bandit();
    let mut result = Vec::new();
    let mut owners = ENTITIES
        .into_iter()
        .map(|bytes| Ok((bytes, None)))
        .collect::<Result<Vec<_>, RepositoryError>>()?;
    if pack::with_content(|content| content.is_freeplay()) {
        // A social slice has no authored encounter roster. Only the scene people a hero's scene
        // assault can bring into a fight have enemy resource bounds.
        if tactical::scene_assault_only() {
            for (person, creature) in scene_fight::combatants() {
                let spec = df_rules::tactical::CombatantSpec::creature(
                    3,
                    creature.id,
                    df_rules::tactical::Cell(0, 0),
                    0,
                )
                .map_err(bad)?;
                owners.push((*scene_fight::person_entity(&person)?.as_bytes(), Some(spec)));
            }
        }
    } else if tactical::enabled() {
        for (who, source_id, ammunition) in tactical::enemies()? {
            owners.push((
                *who.as_bytes(),
                Some(
                    df_rules::tactical::CombatantSpec::creature(
                        3,
                        &source_id,
                        df_rules::tactical::Cell(0, 0),
                        ammunition,
                    )
                    .map_err(bad)?,
                ),
            ));
        }
    } else {
        owners.push((BANDIT, None));
    }
    let source = rule()?;
    for (owner, tactical_enemy) in owners {
        if only.is_some_and(|who| who.as_bytes() != &owner) {
            continue;
        }
        let enemy_hp = tactical_enemy
            .as_ref()
            .map_or(i64::from(bandit.hit_points), |spec| {
                i64::from(spec.hit_points)
            });
        let enemy_ac = tactical_enemy
            .as_ref()
            .map_or(i64::from(bandit.armor_class), |spec| {
                i64::from(spec.armor_class)
            });
        let enemy = owner == BANDIT || tactical_enemy.is_some();
        for (name, minimum, maximum) in [
            (
                "hit-points",
                if tactical::enabled() { 0 } else { 1 },
                if enemy { enemy_hp } else { hero_hp },
            ),
            (
                "armor-class",
                if enemy {
                    if tactical::enabled() {
                        enemy_ac.saturating_sub(2)
                    } else {
                        enemy_ac
                    }
                } else {
                    if tactical::enabled() { 15 } else { hero_ac }
                },
                if enemy { enemy_ac } else { hero_ac },
            ),
            ("strength", 1, 20),
            ("dexterity", 1, 20),
            ("constitution", 1, 20),
            ("intelligence", 1, 20),
            ("wisdom", 1, 20),
            ("charisma", 1, 20),
            ("second-wind", 0, i64::from(pack::second_wind().uses_max)),
            ("hit-point-dice", 0, if enemy { 0 } else { 1 }),
            ("short-rest-choice", 0, if enemy { 0 } else { 2 }),
            ("short-rest-die-roll", 0, if enemy { 0 } else { 10 }),
            ("healer-kit-uses", 0, if enemy { 0 } else { 10 }),
            ("unconscious", 0, 1),
            ("prone", 0, 1),
            ("held-weapon", 0, 1),
            ("action-used", 0, 1),
            ("bonus-used", 0, 1),
            // Set only for a hero who died in a freeplay fight (`scene_fight::close`).
            ("dead", 0, 1),
        ] {
            result.push(ResourceConstraint {
                owner: entity(owner)?,
                resource: model::label(name)?,
                minimum,
                maximum,
                source: source.clone(),
            });
        }
    }
    Ok(result)
}
fn carries_healers_kit(state: &GameState, who: EntityId) -> bool {
    state.inventory.iter().any(|item| {
        item.owner == who && item.origin.entry.as_str() == "healers-kit" && item.quantity > 0
    })
}
/// The test level is pinned in accepted creation choices; recovery never reads startup settings.
fn fighter_progression(
    state: &GameState,
    who: EntityId,
) -> Result<rules::FighterProgression, RepositoryError> {
    let level = state
        .characters
        .iter()
        .find(|character| character.entity == who)
        .and_then(|character| {
            character
                .choices
                .iter()
                .find(|choice| choice.offer.as_str() == "test-fighter-level")
        })
        .map(|choice| match choice.selected.as_str() {
            "fighter-5" => Ok(5),
            _ => Err(RepositoryError::InvalidCandidate),
        })
        .transpose()?
        .unwrap_or(1);
    rules::FighterProgression::new(level).map_err(bad)
}
pub(super) fn resources_for(state: &GameState) -> Result<Vec<ResourceConstraint>, RepositoryError> {
    resources_for_owners(state, None)
}
/// `resources_for` restricted to one owner: the same rows, order and bounds for that owner.
fn resources_for_owners(
    state: &GameState,
    only: Option<EntityId>,
) -> Result<Vec<ResourceConstraint>, RepositoryError> {
    let keep = |owner: EntityId| only.is_none_or(|who| who == owner);
    let mut inventory = resources_of(only)?;
    inventory.extend(scene_fight::created_resources_of(state, only)?);
    inventory.extend(
        tactical::development_resources(state)?
            .into_iter()
            .filter(|constraint| keep(constraint.owner)),
    );
    for character in state
        .characters
        .iter()
        .filter(|character| keep(character.entity))
    {
        let progression = fighter_progression(state, character.entity)?;
        // A recorded ability array marks a complete creation receipt. Older checkpoints lack it
        // and keep their pinned party-wide hit-point bound.
        let level_one = progression.level() == 1;
        let has_recorded_build = character
            .choices
            .iter()
            .any(|choice| choice.offer.as_str() == "array");
        if level_one && !has_recorded_build {
            continue;
        }
        let build = recorded_sheet(state, character, progression)?;
        for constraint in inventory.iter_mut().filter(|constraint| {
            constraint.owner == character.entity && constraint.resource.as_str() == "hit-points"
        }) {
            constraint.maximum = i64::from(build.hit_points);
        }
        if level_one {
            continue;
        }
        inventory.push(ResourceConstraint {
            owner: character.entity,
            resource: model::label("action-surge")?,
            minimum: 0,
            maximum: 1,
            source: rule()?,
        });
        for constraint in inventory
            .iter_mut()
            .filter(|constraint| constraint.owner == character.entity)
        {
            match constraint.resource.as_str() {
                "second-wind" => constraint.maximum = i64::from(build.second_wind_uses),
                "hit-point-dice" => constraint.maximum = i64::from(progression.level()),
                _ => {}
            }
        }
    }
    // A party entity's rows exist only with its character: orphan rows are refused whatever their
    // value, including the party-wide figure.
    inventory.retain(|constraint| {
        let party_entity = ENTITIES
            .iter()
            .any(|bytes| bytes == constraint.owner.as_bytes());
        !party_entity
            || state
                .characters
                .iter()
                .any(|character| character.entity == constraint.owner)
    });
    Ok(inventory)
}

fn add_resource(
    state: &mut GameState,
    who: EntityId,
    name: &str,
    value: i64,
) -> Result<(), RepositoryError> {
    let label = model::label(name)?;
    let constraint = resources_for_owners(state, Some(who))?
        .into_iter()
        .find(|value| value.owner == who && value.resource == label)
        .ok_or(RepositoryError::InvalidCandidate)?;
    state.resources.push(ResourceState {
        owner: who,
        resource: label,
        value,
        minimum: constraint.minimum,
        maximum: constraint.maximum,
        source: constraint.source,
    });
    Ok(())
}
pub(super) fn value(state: &GameState, who: EntityId, name: &str) -> Result<u32, RepositoryError> {
    let label = model::label(name)?;
    match state
        .resources
        .iter()
        .find(|resource| resource.owner == who && resource.resource == label)
    {
        Some(resource) => u32::try_from(resource.value).map_err(bad),
        None if name == "hit-point-dice"
            && state
                .characters
                .iter()
                .any(|character| character.entity == who) =>
        {
            Ok(1)
        }
        None if name == "short-rest-choice"
            && state
                .characters
                .iter()
                .any(|character| character.entity == who) =>
        {
            Ok(0)
        }
        None if name == "short-rest-die-roll"
            && state
                .characters
                .iter()
                .any(|character| character.entity == who) =>
        {
            Ok(0)
        }
        None if name == "healer-kit-uses" => Ok(if carries_healers_kit(state, who) {
            10
        } else {
            0
        }),
        None => Err(RepositoryError::InvalidCandidate),
    }
}
fn set(
    state: &mut GameState,
    who: EntityId,
    name: &str,
    value: u32,
) -> Result<(), RepositoryError> {
    let label = model::label(name)?;
    if !state
        .resources
        .iter()
        .any(|resource| resource.owner == who && resource.resource == label)
    {
        let initial = match name {
            "hit-point-dice"
                if state
                    .characters
                    .iter()
                    .any(|character| character.entity == who) =>
            {
                1
            }
            "short-rest-choice"
                if state
                    .characters
                    .iter()
                    .any(|character| character.entity == who) =>
            {
                0
            }
            "short-rest-die-roll"
                if state
                    .characters
                    .iter()
                    .any(|character| character.entity == who) =>
            {
                0
            }
            "healer-kit-uses" if carries_healers_kit(state, who) => 10,
            _ => return Err(RepositoryError::InvalidCandidate),
        };
        add_resource(state, who, name, initial)?;
    }
    let resource = state
        .resources
        .iter_mut()
        .find(|resource| resource.owner == who && resource.resource == label)
        .ok_or(RepositoryError::InvalidCandidate)?;
    resource.value = i64::from(value);
    Ok(())
}
pub(super) fn name(state: &GameState, who: EntityId) -> Result<String, RepositoryError> {
    if let Some(name) = tactical::development_name(state, who) {
        return Ok(name);
    }
    if tactical::enabled()
        && let Some((index, (_, source_id, _))) = tactical::enemies()?
            .iter()
            .enumerate()
            .find(|(_, (id, _, _))| *id == who)
    {
        return Ok(format!("{} {}", weapon_display_name(source_id), index + 1));
    }
    if who == entity(BANDIT)? {
        return Ok(pack::with_content(|pack| pack.bandit.name.clone()));
    }
    if let Some(person) = scene_fight::person_name(state, who) {
        return Ok(person);
    }
    if state.entities.iter().any(|candidate| {
        candidate.id == who && candidate.definition.entry.as_str() == "lantern-wharf-courier"
    }) {
        return Ok("Courier".to_owned());
    }
    let character = state
        .characters
        .iter()
        .find(|character| character.entity == who)
        .ok_or(RepositoryError::InvalidCandidate)?;
    character
        .choices
        .iter()
        .find(|choice| choice.offer.as_str() == "name")
        .ok_or(RepositoryError::InvalidCandidate)
        .and_then(|choice| stored_name(choice.selected.as_str()))
}

/// Returns a stable, non-display identity for an entity participating in a scene.
/// Scene text and authored NPC IDs are presentation-local; persisted inventory cards
/// need an actor key that survives display-name changes and reconnects.
pub(crate) fn canonical_actor_ref(
    state: &GameState,
    who: EntityId,
) -> Result<String, RepositoryError> {
    if !state.entities.iter().any(|entity| entity.id == who)
        && !state
            .characters
            .iter()
            .any(|character| character.entity == who)
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    Ok(format!("entity:{}", hex(who.as_bytes())))
}

/// Returns the single non-party participant for the legacy journey encounter.
/// The fixed Bandit remains the fallback for pre-binding checkpoints; a scene
/// assault resolves the canonical Courier entity instead.
pub(super) fn combat_enemy(state: &GameState) -> Result<EntityId, RepositoryError> {
    if let Some(encounter) = state.encounters.first() {
        if let Some(enemy) = encounter.participants.iter().copied().find(|candidate| {
            !state
                .characters
                .iter()
                .any(|character| character.entity == *candidate)
        }) {
            return Ok(enemy);
        }
    }
    entity(BANDIT)
}

pub(super) fn combat_creature(state: &GameState, who: EntityId) -> df_rules::SrdCreature {
    if state.entities.iter().any(|candidate| {
        candidate.id == who && candidate.definition.entry.as_str() == "lantern-wharf-courier"
    }) {
        df_rules::SCENE_COMMONER
    } else {
        // The selected pack owns legacy encounter mechanics, including custom tuning.
        df_rules::SrdCreature {
            rules: pack::bandit(),
            ..df_rules::SRD_521_CREATURES[0]
        }
    }
}
pub(super) fn combat_weapon_item(state: &GameState, who: EntityId) -> String {
    if combat_creature(state, who).id == "commoner" {
        "club".to_owned()
    } else {
        pack::bandit_weapon_item()
    }
}
pub(super) fn maximum_hit_points(state: &GameState, who: EntityId) -> Result<u32, RepositoryError> {
    if tactical::present(state) {
        let saved = tactical::snapshot(state)?;
        let id = saved
            .actors
            .iter()
            .find(|(_, entity)| *entity == who)
            .map(|(id, _)| *id)
            .ok_or(RepositoryError::InvalidCandidate)?;
        return Ok(u32::from(
            tactical::read(state)?
                .actor(id)
                .map_err(bad)?
                .health()
                .maximum,
        ));
    }
    sheet_bound(state, who)
}

/// A hero's sheet rebuilt from the creation choices recorded on the character, never from a stored
/// resource row: creation rules (SRD 5.2.1 pp. 47, 84, 87) at level one, advanced by the Fighter
/// progression above it.
fn recorded_sheet(
    state: &GameState,
    character: &CharacterState,
    progression: rules::FighterProgression,
) -> Result<rules::LegalCharacter, RepositoryError> {
    let selected = character
        .choices
        .iter()
        .filter(|choice| {
            super::hero_identity::is_character_pack_validation_choice(choice.offer.as_str())
                && choice.offer.as_str() != "test-fighter-level"
        })
        .map(|choice| (choice.offer.as_str(), choice.selected.as_str()))
        .collect::<Vec<_>>();
    let mut build = pack::validate_character(&name(state, character.entity)?, &selected)
        .inspect_err(|_| refusal_detail::note("sheet-recorded-choices"))
        .map_err(bad)?;
    if progression.level() > 1 {
        progression.apply(&mut build).map_err(bad)?;
    }
    Ok(build)
}

/// The hit point maximum a hero's recorded creation choices state.
fn sheet_bound(state: &GameState, who: EntityId) -> Result<u32, RepositoryError> {
    let character = state
        .characters
        .iter()
        .find(|character| character.entity == who)
        .ok_or(RepositoryError::InvalidCandidate)?;
    Ok(recorded_sheet(state, character, fighter_progression(state, who)?)?.hit_points)
}
fn weapon_display_name(item: &str) -> String {
    item.split('-')
        .map(|word| {
            let mut letters = word.chars();
            match letters.next() {
                Some(first) => format!(
                    "{}{rest}",
                    first.to_ascii_uppercase(),
                    rest = letters.as_str()
                ),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
/// The sheet a creation receipt stores. Public appearance remains in the durable canonical
/// identity and the owner's view; its bounded text is not duplicated in the immutable receipt.
pub(super) fn receipt_character_sheet(
    state: &GameState,
    who: EntityId,
) -> Result<rpc::CharacterSheet, RepositoryError> {
    let mut creation = recreated_heroes(state)?;
    creation.encounters.clear();
    creation.decisions.clear();
    creation.facts.clear();
    let mut sheet = character_sheet(&creation, who)?;
    // Saved creation receipts predate live custody facts. Preserve their exact creation-only
    // wire shape while the current sheet reports what the hero actually carries now.
    sheet
        .facts
        .retain(|fact| fact.label != "Current carried equipment");
    for fact in &mut sheet.facts {
        match fact.label.as_str() {
            "Starting armor (provenance)" => fact.label = "Armor".to_owned(),
            "Starting weapon (provenance)" => fact.label = "Weapon".to_owned(),
            _ if fact.value == "Carried; readiness is shown in the scene" => {
                fact.value = "Held".to_owned()
            }
            _ => {}
        }
    }
    sheet.identity = None;
    Ok(sheet)
}
pub(super) fn character_sheet(
    state: &GameState,
    who: EntityId,
) -> Result<rpc::CharacterSheet, RepositoryError> {
    let character = state
        .characters
        .iter()
        .find(|character| character.entity == who)
        .ok_or(RepositoryError::InvalidCandidate)?;
    let selected = character
        .choices
        .iter()
        .filter(|choice| {
            super::hero_identity::is_character_pack_validation_choice(choice.offer.as_str())
                && choice.offer.as_str() != "test-fighter-level"
        })
        .map(|choice| (choice.offer.as_str(), choice.selected.as_str()))
        .collect::<Vec<_>>();
    let mut build = pack::validate_character(&name(state, who)?, &selected).map_err(bad)?;
    let progression = fighter_progression(state, who)?;
    if progression.level() > 1 {
        progression.apply(&mut build).map_err(bad)?;
    }
    let archer = build
        .choices
        .iter()
        .any(|(key, value)| key == "array" && value == "dexterous");
    let mut facts = Vec::new();
    if pack::active().is_some_and(|loaded| !loaded.departures.is_empty()) {
        facts.push(rpc::SheetFact {
            label: "Supported rules".to_owned(),
            value: model::source_revision(),
        });
    }
    let option_label = |group: &'static str| -> Result<String, RepositoryError> {
        pack::with_content(|pack| {
            pack.creation
                .groups
                .iter()
                .find(|candidate| candidate.id == group)
                .and_then(|offered| {
                    offered.options.iter().find(|option| {
                        build
                            .choices
                            .iter()
                            .any(|(key, value)| *key == group && *value == option.id)
                    })
                })
                .map(|option| option.label.clone())
                .ok_or_else(|| {
                    refusal_detail::cause(
                        "sheet-accepted-option",
                        format!("MissingAcceptedOption group={group}"),
                    );
                    RepositoryError::InvalidCandidate
                })
        })
    };
    let equipment = if archer {
        pack::archer_equipment().map_err(bad)?
    } else {
        pack::with_content(|pack| pack.equipment.clone())
    };
    let gold = {
        equipment
            .iter()
            .filter(|row| row.item == "gold-piece")
            .map(|row| u32::from(row.quantity))
            .sum::<u32>()
    };
    if state.inventory.iter().any(|item| {
        item.owner == who && item.origin.entry.as_str() == "healers-kit" && item.quantity > 0
    }) {
        facts.push(rpc::SheetFact {
            label: "Healer's Kit".to_owned(),
            value: format!("{} uses", value(state, who, "healer-kit-uses")?),
        });
    }
    for (label, value) in [
        ("Species", option_label("species")?),
        ("Class", option_label("class")?),
        ("Background", option_label("background")?),
        (
            "Starting armor (provenance)",
            if archer {
                "Studded Leather"
            } else {
                "Chain Mail · Defense"
            }
            .to_owned(),
        ),
        (
            "Starting weapon (provenance)",
            if archer {
                "Longbow · Slow · +2 Archery".to_owned()
            } else {
                format!("{} · Graze", weapon_display_name(&pack::hero_weapon_item()))
            },
        ),
        ("Origin feat", "Savage Attacker".to_owned()),
        (
            "Masteries",
            if archer {
                "Longbow, Scimitar, Shortsword"
            } else {
                "Greatsword, Flail, Javelin"
            }
            .to_owned(),
        ),
        (
            "Starting equipment",
            format!(
                "{} · {gold} GP",
                if archer {
                    "Fighter B + Soldier B".to_owned()
                } else {
                    option_label("equipment")?
                }
            ),
        ),
    ] {
        facts.push(rpc::SheetFact {
            label: label.to_owned(),
            value: value.to_owned(),
        });
    }
    facts.push(rpc::SheetFact {
        label: "Current carried equipment".to_owned(),
        value: state
            .inventory
            .iter()
            .filter(|item| item.owner == who && item.origin.entry.as_str() != "gold-piece")
            .map(|item| {
                format!(
                    "{} x {}",
                    item.quantity,
                    item.origin.entry.as_str().replace('-', " ")
                )
            })
            .collect::<Vec<_>>()
            .join(", "),
    });
    let character = state
        .characters
        .iter()
        .find(|character| character.entity == who)
        .ok_or(RepositoryError::InvalidCandidate)?;
    for (label, key) in [
        ("First language", "language-1"),
        ("Second language", "language-2"),
        ("First class skill", "skill-1"),
        ("Second class skill", "skill-2"),
        ("Alignment", "alignment"),
        ("Gaming set", "gaming-set"),
    ] {
        let selected = character
            .choices
            .iter()
            .find(|choice| choice.offer.as_str() == key)
            .ok_or(RepositoryError::InvalidCandidate)?;
        facts.push(rpc::SheetFact {
            label: label.to_owned(),
            value: selected.selected.as_str().replace('-', " "),
        });
    }
    facts.push(rpc::SheetFact {
        label: "Hit Point Dice".to_owned(),
        value: format!(
            "{}/{} (d{})",
            value(state, who, "hit-point-dice")?,
            progression.level(),
            rules::LEVEL_ONE_FIGHTER_HIT_POINT_DIE_SIDES
        ),
    });
    if progression.level() > 1 {
        for (label, value) in [
            (
                "Class",
                format!("Fighter · Champion · level {}", progression.level()),
            ),
            (
                "Ability Score Improvement",
                if archer {
                    "+2 Dexterity".to_owned()
                } else {
                    "+2 Strength".to_owned()
                },
            ),
            (
                "Masteries",
                if archer {
                    "Longbow, Scimitar, Shortsword, Javelin".to_owned()
                } else {
                    "Greatsword, Flail, Javelin, Spear".to_owned()
                },
            ),
        ] {
            if let Some(fact) = facts.iter_mut().find(|fact| fact.label == label) {
                fact.value = value;
            } else {
                facts.push(rpc::SheetFact {
                    label: label.to_owned(),
                    value,
                });
            }
        }
    }
    let hit_point_die_roll = value(state, who, "short-rest-die-roll")?;
    if hit_point_die_roll != 0 {
        let constitution = u8::try_from(value(state, who, "constitution")?).map_err(bad)?;
        let modifier = rules::ability_modifier(constitution);
        let total = (i32::try_from(hit_point_die_roll).map_err(bad)? + modifier).max(1);
        facts.push(rpc::SheetFact {
            label: "Hit Point Die roll".to_owned(),
            value: format!(
                "d{} {hit_point_die_roll} {} {} = {total}, now {} of {}",
                rules::LEVEL_ONE_FIGHTER_HIT_POINT_DIE_SIDES,
                if modifier < 0 { "-" } else { "+" },
                modifier.unsigned_abs(),
                value(state, who, "hit-points")?,
                maximum_hit_points(state, who)?,
            ),
        });
    }
    facts.push(rpc::SheetFact {
        label: "Base language".to_owned(),
        value: "Common".to_owned(),
    });
    for (label, key) in [
        ("STR", "strength"),
        ("DEX", "dexterity"),
        ("CON", "constitution"),
        ("INT", "intelligence"),
        ("WIS", "wisdom"),
        ("CHA", "charisma"),
        ("Second Wind uses", "second-wind"),
    ] {
        facts.push(rpc::SheetFact {
            label: label.to_owned(),
            value: value(state, who, key)?.to_string(),
        });
    }
    if archer {
        let choices = build
            .choices
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect::<Vec<_>>();
        let profile = df_rules::tactical::validate_archer(&build.name, &choices)
            .map_err(bad)?
            .combatant(1, df_rules::tactical::Cell(0, 0));
        let profile = if progression.level() > 1 {
            profile.at_fighter_level(progression.level()).map_err(bad)?
        } else {
            profile
        };
        for weapon in &profile.weapons {
            let (attack, damage) = profile
                .hero_attack_modifiers(*weapon, weapon.ranged_weapon(), weapon.finesse())
                .map_err(bad)?;
            let (count, sides) = weapon.dice(weapon.two_handed());
            let range = weapon
                .range()
                .map(|(normal, long)| format!(" · {normal}/{long} ft"))
                .unwrap_or_default();
            facts.push(rpc::SheetFact {
                label: format!("{} attack", weapon_display_name(weapon.id())),
                value: format!(
                    "{attack:+} to hit · {count}d{sides}{damage:+} {:?}{range} · {:?}",
                    weapon.damage_type(),
                    weapon.mastery().ok_or(RepositoryError::InvalidCandidate)?
                ),
            });
        }
    }
    facts.push(rpc::SheetFact {
        label: "Condition".to_owned(),
        value: if value(state, who, "unconscious")? != 0 {
            "Unconscious and Prone"
        } else if value(state, who, "prone")? != 0 {
            "Prone"
        } else {
            "None"
        }
        .to_owned(),
    });
    let current_source_weapon = if archer {
        "longbow".to_owned()
    } else {
        pack::hero_weapon_item()
    };
    facts.push(rpc::SheetFact {
        label: weapon_display_name(&current_source_weapon),
        value: if state.inventory.iter().any(|item| {
            item.owner == who
                && item.quantity > 0
                && item.origin.entry.as_str() == current_source_weapon
        }) {
            "Carried; readiness is shown in the scene"
        } else {
            "Not currently carried"
        }
        .to_owned(),
    });
    // SRD 5.2.1 pp. 47, 83-84: Fighter, Soldier and Dwarf traits belong to the accepted build.
    for (label, value) in [
        (
            "Soldier skill proficiencies",
            "Athletics, Intimidation".to_owned(),
        ),
        (
            "Saving throw proficiencies",
            "Strength, Constitution".to_owned(),
        ),
        (
            "Proficiency Bonus",
            format!(
                "+{}",
                if progression.level() > 1 {
                    progression.proficiency_bonus()
                } else {
                    pack::with_creation(|creation| creation.proficiency_bonus)
                }
            ),
        ),
        (
            "Initiative",
            format!(
                "{:+}",
                rules::ability_modifier(value(state, who, "dexterity")? as u8)
            ),
        ),
        ("Armor training", "Light, Medium, Heavy, Shields".to_owned()),
        ("Weapon proficiencies", "Simple, Martial".to_owned()),
    ] {
        facts.push(rpc::SheetFact {
            label: label.to_owned(),
            value,
        });
    }
    fn species_sheet_facts(
        choices: &[(&str, &str)],
    ) -> Result<Vec<df_protocol::common::SheetFact>, df_rules::local_journey::JourneyRuleError>
    {
        use df_rules::local_journey::{HeroSpecies, JourneyRuleError, SpeciesSize};
        let species = choices
            .iter()
            .find(|(key, _)| *key == "species")
            .and_then(|(_, value)| HeroSpecies::from_choice(value))
            .ok_or(JourneyRuleError::InvalidChoice)?;
        let options = df_rules::species::SpeciesOptions::from_choices(species, choices)?;
        let traits = options.combat_traits(species);
        let mut facts = vec![];
        let mut push = |label: &str, value: String| {
            facts.push(df_protocol::common::SheetFact {
                label: label.to_owned(),
                value,
            })
        };
        push("Species", species.choice_id().to_owned());
        push(
            "Size",
            match traits.size {
                SpeciesSize::Small => "Small",
                SpeciesSize::Medium => "Medium",
            }
            .to_owned(),
        );
        push("Speed", format!("{} ft", traits.speed_ft));
        push(
            "Darkvision",
            traits
                .darkvision_ft
                .map_or_else(|| "None".to_owned(), |range| format!("{range} ft")),
        );
        if let Some(resistance) = options.resistance(species) {
            push(
                if species == HeroSpecies::Dwarf {
                    "Dwarven Resilience"
                } else {
                    "Damage Resistance"
                },
                resistance.id().to_owned(),
            );
        }
        if species == HeroSpecies::Dwarf {
            push("Stonecunning", "Tremorsense is not in play yet".to_owned());
        }
        for group in df_rules::species::choice_groups(species) {
            if let Some((_, value)) = choices.iter().find(|(key, _)| *key == group.id) {
                push(group.label, value.replace('-', " "));
            }
        }
        for trait_name in match species {
            HeroSpecies::Dwarf => vec!["Dwarven Toughness: +1 maximum HP per level"],
            HeroSpecies::Halfling => vec![
                "Luck: reroll natural 1 on D20 Tests",
                "Halfling Nimbleness: pass through larger creatures; cannot stop in their space",
            ],
            HeroSpecies::Dragonborn => vec![
                "Breath Weapon: 15-foot Cone or 30-foot Line; Dexterity save; Proficiency Bonus uses per Long Rest",
            ],
            HeroSpecies::Goliath
                if options.ancestry
                    != Some(df_rules::species::Ancestry::Giant(
                        df_rules::species::GiantAncestry::Cloud,
                    )) =>
            {
                vec!["Giant Ancestry: Proficiency Bonus uses per Long Rest"]
            }
            HeroSpecies::Human => vec!["Tough: +2 maximum HP per level"],
            HeroSpecies::Orc => vec![
                "Relentless Endurance: drop to 1 HP instead of 0 if not killed outright; once per Long Rest",
            ],
            _ => vec![],
        } {
            push("Species trait", trait_name.to_owned());
        }
        for gap in df_rules::species::not_in_play(species, options) {
            push("Not in play yet", gap.to_owned());
        }
        for integration_gap in match species {
            HeroSpecies::Dragonborn => vec!["Breath Weapon action offers"],
            HeroSpecies::Goliath => vec!["Giant Ancestry action and reaction offers"],
            HeroSpecies::Orc => vec!["Relentless Endurance resource carried across encounters"],
            _ => vec![],
        } {
            push("Not in play yet", integration_gap.to_owned());
        }
        Ok(facts)
    }

    // SRD Species pp.84?86: project the accepted species, including explicit unsupported traits.
    facts.extend(
        species_sheet_facts(&selected)
            .map_err(bad)?
            .into_iter()
            .filter(|fact| fact.label != "Species"),
    );
    // SRD 5.2.1 pp. 6, 7, 47: accepted ability modifiers and Fighter save proficiencies.
    let proficiency = i32::from(if progression.level() > 1 {
        progression.proficiency_bonus()
    } else {
        pack::with_creation(|creation| creation.proficiency_bonus)
    });
    for (label, ability) in [
        ("Strength saving throw", "strength"),
        ("Dexterity saving throw", "dexterity"),
        ("Constitution saving throw", "constitution"),
        ("Intelligence saving throw", "intelligence"),
        ("Wisdom saving throw", "wisdom"),
        ("Charisma saving throw", "charisma"),
    ] {
        let score = u8::try_from(value(state, who, ability)?).map_err(|_| {
            refusal_detail::note("sheet-save-score");
            RepositoryError::InvalidCandidate
        })?;
        let modifier = rules::ability_modifier(score)
            + if matches!(ability, "strength" | "constitution") {
                proficiency
            } else {
                0
            };
        facts.push(rpc::SheetFact {
            label: label.to_owned(),
            value: format!("{modifier:+}"),
        });
    }
    // SRD 5.2.1 p. 9: the complete skill table. The same native modifiers drive actual checks.
    for (label, skill) in [
        ("Acrobatics", "acrobatics"),
        ("Animal Handling", "animal-handling"),
        ("Arcana", "arcana"),
        ("Athletics", "athletics"),
        ("Deception", "deception"),
        ("History", "history"),
        ("Insight", "insight"),
        ("Intimidation", "intimidation"),
        ("Investigation", "investigation"),
        ("Medicine", "medicine"),
        ("Nature", "nature"),
        ("Perception", "perception"),
        ("Performance", "performance"),
        ("Persuasion", "persuasion"),
        ("Religion", "religion"),
        ("Sleight of Hand", "sleight-of-hand"),
        ("Stealth", "stealth"),
        ("Survival", "survival"),
    ] {
        let (ability, bonus) =
            super::beat_script_runtime::skill_check_modifiers(state, who, skill)?;
        let modifier = i32::from(ability) + i32::from(bonus);
        facts.push(rpc::SheetFact {
            label: label.to_owned(),
            value: format!("{modifier:+}"),
        });
    }
    if living_scene::has_interactions(state) || rewards::paid(state) {
        let copper = living_scene::balance_cp(state, who)?;
        facts.push(rpc::SheetFact {
            label: "Coin".to_owned(),
            value: format!("{} GP, {} CP", copper / 100, copper % 100),
        });
    }
    if tactical::present(state) {
        let battle = tactical::read(state)?;
        let actor = battle.actor(tactical::actor_id(state, who)?).map_err(bad)?;
        if actor.health().dead
            && let Some(condition) = facts.iter_mut().find(|fact| fact.label == "Condition")
        {
            condition.value = "Dead".to_owned();
        }
        // SRD 5.2.1 p. 177: an Attack's equip/unequip changes the authoritative in-hand weapon.
        for weapon in &actor.spec().weapons {
            let label = weapon_display_name(weapon.id());
            let value = if actor.held_weapons().contains(weapon) {
                "Held"
            } else if actor
                .dropped_weapons()
                .iter()
                .any(|(dropped, _)| dropped == weapon)
            {
                "Dropped; ownership retained"
            } else {
                "Stowed"
            };
            if let Some(fact) = facts.iter_mut().find(|fact| fact.label == label) {
                fact.value = value.to_owned();
            } else {
                facts.push(rpc::SheetFact {
                    label,
                    value: value.to_owned(),
                });
            }
        }
    }
    Ok(rpc::CharacterSheet {
        name: name(state, who)?,
        identity: if character
            .choices
            .iter()
            .any(|choice| choice.offer.as_str().starts_with("identity-appearance:"))
        {
            Some(
                super::hero_identity::ValidatedIdentity::from_canonical_appearance_choices(
                    &character
                        .choices
                        .iter()
                        .map(|choice| {
                            (
                                choice.offer.as_str().to_owned(),
                                choice.selected.as_str().to_owned(),
                            )
                        })
                        .collect::<Vec<_>>(),
                )
                .map_err(bad)?
                .to_rpc(),
            )
        } else {
            None
        },
        portrait_uri: super::hero_identity::approved_portrait_uri(state, who).unwrap_or_default(),
        portrait_draft_revision: character
            .choices
            .iter()
            .find(|choice| choice.offer.as_str() == "identity-portrait:revision")
            .and_then(|choice| choice.selected.as_str().parse().ok())
            .unwrap_or(0),
        facts,
        hit_points: value(state, who, "hit-points")?,
        maximum_hit_points: maximum_hit_points(state, who)?,
        armor_class: value(state, who, "armor-class")?,
        source_revision: model::source_revision(),
        ..Default::default()
    })
}
fn beat(state: &mut GameState, next: &str) -> Result<(), RepositoryError> {
    state
        .narrative
        .completed_beats
        .append(&mut state.narrative.active_beats);
    state.narrative.active_beats = vec![model::content(next)?];
    Ok(())
}
pub(super) fn offer_id(current: &Checkpoint, kind: rpc::GameplayActionKind) -> String {
    if kind == rpc::GameplayActionKind::ResolveErrand
        && let Some(id) = super::beat_script_runtime::resolve_id(current)
    {
        return id;
    }
    if tactical::present(current.state()) {
        return format!(
            "tactical-{}-{}-{}",
            kind as i32,
            current.basis().revision.epoch().get(),
            current.basis().revision.sequence()
        );
    }
    if let Some(id) = super::beat_script_runtime::scripted_id(current, kind) {
        return id;
    }
    if kind == rpc::GameplayActionKind::SceneAssault {
        return scene_fight::offer_id(current);
    }
    if kind == rpc::GameplayActionKind::ChooseHarborScene {
        return format!(
            "journey-inn-{}-{}-{}",
            current.basis().revision.epoch().get(),
            current.basis().revision.sequence(),
            current.state().logical_time.ticks
        );
    }
    if kind == rpc::GameplayActionKind::ShortRest {
        return format!(
            "journey-rest-{}-{}-{}",
            current.basis().revision.epoch().get(),
            current.basis().revision.sequence(),
            current.state().logical_time.ticks
        );
    }
    if matches!(
        kind,
        rpc::GameplayActionKind::GreatswordAttack
            | rpc::GameplayActionKind::SecondWind
            | rpc::GameplayActionKind::EndTurn
    ) && let Some(encounter) = current.state().encounters.first()
        && let Some(active) = encounter.active_turn
    {
        return format!(
            "journey-{}-{}-{}",
            kind as i32,
            current.state().logical_time.ticks,
            hex(active.as_bytes())
        );
    }
    format!("journey-{}-1", kind as i32)
}
pub(super) fn offered(
    current: &Checkpoint,
    member: MemberId,
) -> Result<Vec<(rpc::GameplayActionKind, String)>, RepositoryError> {
    if tactical::development_mode(current.state()) && tactical::present(current.state()) {
        return tactical::offered(current, member);
    }
    // The machine's state decides which kinds of offer may be built; each
    // branch below only builds labels and inputs for kinds it accepts.
    let state = session_machine::SessionState::of(current)?;
    if matches!(
        state,
        session_machine::SessionState::Ended | session_machine::SessionState::Preparing
    ) {
        player_entity(member, current)?;
        return Ok(if state == session_machine::SessionState::Ended {
            vec![(
                rpc::GameplayActionKind::StartNewAdventure,
                "Start a new adventure".to_owned(),
            )]
        } else {
            Vec::new()
        });
    }
    if tactical::present(current.state())
        && tactical::hero_is_dead(current.state(), player_entity(member, current)?)?
    {
        return Ok(Vec::new());
    }
    // A hero who died in a freeplay fight has nothing more to do: no fight is open to read.
    if player_entity(member, current).is_ok_and(|who| scene_fight::hero_dead(current.state(), who))
    {
        return Ok(Vec::new());
    }
    if state == session_machine::SessionState::Aftermath
        && short_rest_started(current.state())
        && !short_rest_completed(current.state())
    {
        let who = player_entity(member, current)?;
        return Ok(if hit_point_die_offered(current.state(), who)? {
            vec![(
                rpc::GameplayActionKind::ShortRest,
                short_rest_offer_label(current.state(), who)?,
            )]
        } else {
            Vec::new()
        });
    }
    if tactical::present(current.state()) {
        if state == session_machine::SessionState::Fight {
            return tactical::offered(current, member);
        }
        if state == session_machine::SessionState::Aftermath && tactical::dying(current.state())? {
            return Ok(vec![(
                rpc::GameplayActionKind::EndTurn,
                if tactical::needs_saves(current.state())? {
                    "Hold on while the fallen fight for breath (death saves)"
                } else {
                    "Wait with the wounded until they stir (a stable hero wakes with 1 hit point)"
                }
                .to_owned(),
            )]);
        }
    }
    affordance::offered(current, member).map_err(bad)
}

fn authored_offered(
    current: &Checkpoint,
    member: MemberId,
) -> Result<Vec<(rpc::GameplayActionKind, String)>, RepositoryError> {
    super::beat_script::admit_offers(current, handler_offered(current, member)?)
}

pub(super) fn new_adventure_requested(current: &Checkpoint) -> bool {
    current.state().facts.iter().any(|fact| {
        matches!(&fact.value, FactValue::ContentEvent { definition, .. }
            if definition.entry.as_str() == "start-new-adventure")
            && current.state().decisions.iter().any(|decision| {
                decision.operation == fact.operation && decision.facts.contains(&fact.id)
            })
    })
}

fn handler_offered(
    current: &Checkpoint,
    member: MemberId,
) -> Result<Vec<(rpc::GameplayActionKind, String)>, RepositoryError> {
    // Offer kinds are a function of the machine's state: each arm below
    // builds only kinds `session_machine::kind_accepted` allows there, and
    // keeps its own conditions for when among them.
    use session_machine::SessionState as State;
    let state = session_machine::SessionState::of(current)?;
    let who = player_entity(member, current)?;
    let ready = current
        .state()
        .characters
        .iter()
        .any(|character| character.entity == who);
    if !ready {
        return Ok(
            if session_machine::kind_accepted(state, rpc::GameplayActionKind::CreateCharacter) {
                vec![(
                    rpc::GameplayActionKind::CreateCharacter,
                    pack::offer_label(rpc::GameplayActionKind::CreateCharacter),
                )]
            } else {
                vec![]
            },
        );
    }
    let rest_accepted = session_machine::rest_accepted(current)?;
    if let Some(offers) = super::beat_script::with_script(|script| {
        if !script.uses_encounter() && state != State::Lobby {
            Some(script.current(current.state()).and_then(|beat| {
                beat.offers
                    .iter()
                    .filter(|offer| {
                        session_machine::next(state, session_machine::Event::of(offer.effect))
                            .is_ok()
                            && (offer.effect != super::beat_script::BeatEffect::ShortRest
                                || rest_accepted)
                    })
                    .map(|offer| {
                        let label = if offer.effect == super::beat_script::BeatEffect::ShortRest {
                            short_rest_offer_label(current.state(), who)
                        } else {
                            script.label(offer)
                        }?;
                        Ok((offer.effect.action(), label))
                    })
                    .collect()
            }))
        } else {
            None
        }
    }) {
        return offers;
    }
    let mut result = match state {
        State::Lobby
            if party_ready(current) && narrative_transition::story_permitted(current, member)? =>
        {
            vec![(
                rpc::GameplayActionKind::BeginStory,
                pack::offer_label(rpc::GameplayActionKind::BeginStory),
            )]
        }
        State::Lobby => vec![],
        State::Story => vec![
            (
                rpc::GameplayActionKind::AskCourier,
                pack::offer_label(rpc::GameplayActionKind::AskCourier),
            ),
            (
                rpc::GameplayActionKind::EscortCourier,
                pack::offer_label(rpc::GameplayActionKind::EscortCourier),
            ),
        ],
        State::Conversation if narrative_phase::courier_encounter_permitted(current)? => {
            vec![(
                rpc::GameplayActionKind::DefendCourier,
                pack::offer_label(rpc::GameplayActionKind::DefendCourier),
            )]
        }
        State::Conversation => vec![],
        State::Fight => {
            let encounter = current
                .state()
                .encounters
                .first()
                .ok_or(RepositoryError::InvalidCandidate)?;
            if encounter.active_turn != Some(who)
                || value(current.state(), who, "unconscious")? != 0
            {
                vec![]
            } else {
                let mut offered = Vec::new();
                if combat_transition::available(current, who)? {
                    offered.push((
                        rpc::GameplayActionKind::GreatswordAttack,
                        pack::offer_label(rpc::GameplayActionKind::GreatswordAttack),
                    ));
                }
                if value(current.state(), who, "bonus-used")? == 0
                    && value(current.state(), who, "second-wind")? > 0
                {
                    offered.push((
                        rpc::GameplayActionKind::SecondWind,
                        pack::offer_label(rpc::GameplayActionKind::SecondWind),
                    ));
                }
                offered.push((
                    rpc::GameplayActionKind::EndTurn,
                    pack::offer_label(rpc::GameplayActionKind::EndTurn),
                ));
                offered
            }
        }
        State::Aftermath => {
            if tactical::present(current.state())
                && !tactical::read(current.state())?
                    .combatants()
                    .iter()
                    .any(|actor| {
                        actor.side() == df_rules::tactical::Side::Heroes
                            && actor.health().hit_points > 0
                            && !actor.health().dead
                    })
            {
                return Ok(Vec::new());
            }
            // Validates the rest cursor and due schedules even when no
            // rest is offered; forged cursors refuse here.
            prepare_short_rest(current)?;
            // The machine's rest rule decides whether the rest is offered:
            // once per aftermath, while a hero has at least 1 Hit Point.
            let mut result = Vec::new();
            if session_machine::rest_accepted(current)? {
                result.push((
                    rpc::GameplayActionKind::ShortRest,
                    short_rest_offer_label(current.state(), who)?,
                ));
            }
            if inn_source(current)?.is_some() && value(current.state(), who, "unconscious")? == 0 {
                result.push((
                    rpc::GameplayActionKind::ChooseHarborScene,
                    pack::offer_label(rpc::GameplayActionKind::ChooseHarborScene),
                ));
            }
            result
        }
        // The end card accepts nothing, so it offers nothing.
        State::Ended | State::Preparing => vec![],
    };
    if matches!(state, State::Lobby | State::Story | State::Conversation)
        && session_machine::kind_accepted(state, rpc::GameplayActionKind::ExamineHarborSeal)
        && super::beat_script::with_script(|script| {
            script.current(current.state()).map(|beat| {
                beat.offers
                    .iter()
                    .any(|offer| offer.effect == super::beat_script::BeatEffect::Observe)
            })
        })?
    {
        let scene = super::beat_script_runtime::scene(current.state())?;
        if !scene.findings.is_empty() || !scene.cast.is_empty() {
            result.push((
                rpc::GameplayActionKind::ExamineHarborSeal,
                "Look around".to_owned(),
            ));
        }
    }
    Ok(result)
}
// The authored invitation follows the accepted victory, including after an admitted rest.
// It changes only the scene beat; no travel duration, geometry or delivery is inferred.
fn inn_source(current: &Checkpoint) -> Result<Option<FactId>, RepositoryError> {
    let state = current.state();
    if state.narrative.active_beats.as_slice() != [model::content("complete")?]
        || !combat_victory(state)?
        || !state
            .narrative
            .open_threads
            .contains(&model::content(PACKET_THREAD)?)
        || !party_ready(current)
    {
        return Ok(None);
    }
    for link in participants(current) {
        let who = link.character.ok_or(RepositoryError::InvalidCandidate)?;
        if !state
            .characters
            .iter()
            .any(|character| character.entity == who)
        {
            return Ok(None);
        }
    }
    if tactical::present(state) {
        let start = narrative_phase::source(state, "defend-courier", rpc::JourneyPhase::Combat)?;
        return tactical::completion_source(state, start, true, true).map(|event| Some(event.id));
    }
    let bandit = entity(BANDIT)?;
    let unconscious = model::label("unconscious")?;
    let source_rule = rule()?;
    let attack = model::content("greatsword-attack")?;
    for decision in &state.decisions {
        if decision.source_policy.as_str() != THREAD_POLICY {
            continue;
        }
        let accepted = accepted(decision)?;
        if accepted.phase != rpc::JourneyPhase::Complete as i32
            || !accepted.combat.iter().any(|outcome| {
                outcome.knocked_out && outcome.target_id.as_slice() == bandit.as_bytes()
            })
        {
            continue;
        }
        let Some(knockout) = state.facts.iter().find(|fact| {
            decision.facts.contains(&fact.id)
                && fact.operation == decision.operation
                && fact.revision == decision.revision
                && matches!(&fact.value, FactValue::ResourceChanged {
                    entity, resource, before: 0, after: 1, source
                } if *entity == bandit && *resource == unconscious && *source == source_rule)
        }) else {
            continue;
        };
        let Some(event) = state.facts.iter().find(|fact| {
            decision.facts.contains(&fact.id)
                && fact.operation == decision.operation
                && fact.revision == decision.revision
                && fact.ordinal > knockout.ordinal
                && matches!(&fact.value, FactValue::ContentEvent { definition, .. }
                    if definition == &attack)
        }) else {
            continue;
        };
        return Ok(Some(event.id));
    }
    Ok(None)
}

/// Whether a stored receipt's scene is the inn. Receipts written before scenes carried their source
/// key hold the same scene with that key empty; only a checkpoint under the legacy pins (see
/// `model::is_legacy_profile`) may carry them.
pub(super) fn is_inn_scene(recorded: &rpc::GameplayScene) -> bool {
    let current = inn_scene();
    *recorded == current
        || (model::active_profile_is_legacy()
            && *recorded
                == rpc::GameplayScene {
                    source_scene_key: String::new(),
                    ..current
                })
}

pub(super) fn inn_scene() -> rpc::GameplayScene {
    rpc::GameplayScene {
        destination: rpc::HarborDestination::HarborInn as i32,
        source_scene_key: pack::SceneKey::Inn.source_label().to_owned(),
        title: pack::scene(pack::SceneKey::Inn).title,
        scene_asset: pack::scene(pack::SceneKey::Inn).asset,
        speaker_portrait: pack::scene(pack::SceneKey::Inn).speaker_portrait,
        cast: Vec::new(),
        // The view fills in the rules note and the place, which the pack alone knows.
        rules_note: String::new(),
        place: String::new(),
        location_id: String::new(),
        live_backdrop: String::new(),
        live_plate: String::new(),
        plate_layout: None,
        plate_state: rpc::PlateState::Unspecified as i32,
        ..Default::default()
    }
}
pub(super) fn accepted(
    decision: &AcceptedDecision,
) -> Result<rpc::AcceptedAction, RepositoryError> {
    if let Some(accepted) = tactical::development_receipt(decision)? {
        return Ok(accepted);
    }
    if let Some(accepted) = scene_fight::opening_receipt(decision)? {
        return Ok(accepted);
    }
    if let Some(accepted) = living_scene::accepted(decision)? {
        return Ok(accepted);
    }
    if decision.source_policy.as_str() != THREAD_POLICY {
        return Err(RepositoryError::InvalidReceipt);
    }
    let text = decision
        .semantic_output
        .as_deref()
        .ok_or(RepositoryError::InvalidReceipt)?;
    let bytes = unhex(text)?;
    rpc::AcceptedAction::decode(bytes.as_slice()).map_err(bad)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CombatRoundError {
    Binding,
    Encounter,
    Start,
    Source,
    Units,
    Duration,
    Chronology,
    Capacity,
}

/// The initiative rolls the encounter started with, in turn order (first to act first), read
/// from the recorded d20 draws and ability scores exactly as `combat_round` validates them:
/// nothing is re-rolled or invented. A tie keeps the recorded turn order, so `order` is the
/// authoritative tie-break result.
pub(super) fn initiative_rolls(
    current: &Checkpoint,
) -> Result<Vec<rpc::InitiativeRoll>, CombatRoundError> {
    use CombatRoundError as Error;
    if tactical::present(current.state()) {
        return tactical::initiative(current).map_err(|_| Error::Start);
    }
    let state = current.state();
    let [encounter] = state.encounters.as_slice() else {
        return Err(Error::Encounter);
    };
    let start = state
        .facts
        .iter()
        .find(|fact| {
            matches!(&fact.value, FactValue::ContentEvent { definition, .. }
            if definition.entry.as_str() == "defend-courier")
        })
        .ok_or(Error::Start)?;
    let mut rolls = Vec::with_capacity(encounter.participants.len());
    for (ordinal, who) in encounter.participants.iter().enumerate() {
        let draw = state
            .draws
            .iter()
            .find(|draw| draw.operation == start.operation && draw.ordinal == ordinal as u32)
            .ok_or(Error::Start)?;
        let dexterity =
            u8::try_from(value(state, *who, "dexterity").map_err(|_| Error::Encounter)?)
                .map_err(|_| Error::Encounter)?;
        let modifier = rules::ability_modifier(dexterity);
        let order = encounter
            .turn_order
            .iter()
            .position(|turn| turn == who)
            .ok_or(Error::Encounter)?;
        rolls.push(rpc::InitiativeRoll {
            participant_id: who.as_bytes().to_vec(),
            name: name(state, *who).map_err(|_| Error::Source)?,
            die: draw.value,
            modifier,
            total: draw.value as i32 + modifier,
            order: order as u32 + 1,
        });
    }
    rolls.sort_by_key(|roll| roll.order);
    Ok(rolls)
}

/// Count only the current authored encounter's accepted initiative wraps.
pub(super) fn combat_round(current: &Checkpoint) -> Result<u32, CombatRoundError> {
    use CombatRoundError as Error;
    let pins = model::pins().map_err(|_| Error::Binding)?;
    current
        .validate_resume(current.basis(), &pins)
        .map_err(|_| Error::Binding)?;
    let state = current.state();
    if tactical::present(state) {
        tactical::validate_encounter(current).map_err(|_| Error::Encounter)?;
        return Ok(tactical::read(state).map_err(|_| Error::Encounter)?.round());
    }
    if state.facts.len() > 512 || state.decisions.len() > 512 {
        return Err(Error::Capacity);
    }
    let content = |entry| model::content(entry).map_err(|_| Error::Source);
    let monster = entity(BANDIT).map_err(|_| Error::Encounter)?;
    let party = participants(current)
        .into_iter()
        .map(|link| link.character.ok_or(Error::Encounter))
        .collect::<Result<Vec<_>, _>>()?;
    let expected = party.iter().copied().chain([monster]).collect::<Vec<_>>();
    let [encounter] = state.encounters.as_slice() else {
        return Err(Error::Encounter);
    };
    if !(1..=MAXIMUM_PARTY).contains(&party.len())
        || encounter.id != RecordId::from_bytes(&ENCOUNTER).map_err(|_| Error::Encounter)?
        || encounter.definition != content("combat")?
        || encounter.combat_policy != content("normal-nonlethal-melee")?
        || encounter.participants != expected
        || !objective_outcome::defend_history(state).map_err(|_| Error::Encounter)?
    {
        return Err(Error::Encounter);
    }
    let starts = state
        .facts
        .iter()
        .filter(|fact| {
            matches!(&fact.value, FactValue::ContentEvent { definition, .. }
            if definition.entry.as_str() == "defend-courier")
        })
        .collect::<Vec<_>>();
    let [start] = starts.as_slice() else {
        return Err(Error::Start);
    };
    let owner = |fact: &GameFact| {
        state
            .decisions
            .iter()
            .find(|decision| {
                decision.operation == fact.operation
                    && decision.revision == fact.revision
                    && decision.facts.contains(&fact.id)
            })
            .ok_or(Error::Source)
    };
    let start_decision = owner(start).map_err(|_| Error::Start)?;
    if start_decision.facts.last() != Some(&start.id)
        || start.audience != AudienceScope::Shared
        || !matches!(&start.value, FactValue::ContentEvent { definition, .. }
            if *definition == content("defend-courier")?)
        || accepted(start_decision).map_err(|_| Error::Start)?.phase
            != rpc::JourneyPhase::Combat as i32
    {
        return Err(Error::Start);
    }
    let source = rule().map_err(|_| Error::Start)?;
    for ordinal in 0..expected.len() as u32 {
        if !start_decision.draws.contains(&ordinal)
            || !state.draws.iter().any(|draw| {
                draw.operation == start.operation
                    && draw.ordinal == ordinal
                    && draw.sides == 20
                    && draw.source == source
            })
        {
            return Err(Error::Start);
        }
    }
    let mut initiative = Vec::new();
    for (priority, who) in expected.iter().enumerate() {
        let draw = state
            .draws
            .iter()
            .find(|draw| draw.operation == start.operation && draw.ordinal == priority as u32)
            .ok_or(Error::Start)?;
        let dexterity =
            u8::try_from(value(state, *who, "dexterity").map_err(|_| Error::Encounter)?)
                .map_err(|_| Error::Encounter)?;
        initiative.push((
            *who,
            draw.value as i32 + rules::ability_modifier(dexterity),
            priority,
        ));
    }
    initiative.sort_by_key(|(_, score, priority)| (std::cmp::Reverse(*score), *priority));
    if encounter.turn_order
        != initiative
            .into_iter()
            .map(|(who, _, _)| who)
            .collect::<Vec<_>>()
    {
        return Err(Error::Encounter);
    }
    let mut time = LogicalTime {
        ticks: 0,
        ticks_per_second: 1,
    };
    let mut round = 1u32;
    let mut advances: Vec<(OperationId, u64, bool)> = Vec::new();
    for fact in &state.facts {
        let FactValue::TimeAdvanced { before, after } = fact.value else {
            continue;
        };
        if before.ticks_per_second != 1
            || after.ticks_per_second != 1
            || state.logical_time.ticks_per_second != 1
        {
            return Err(Error::Units);
        }
        if before != time || fact.revision <= start.revision {
            return Err(Error::Chronology);
        }
        let decision = owner(fact)?;
        let terminal = state
            .facts
            .iter()
            .find(|event| Some(&event.id) == decision.facts.last())
            .ok_or(Error::Source)?;
        if fact.audience != AudienceScope::Shared
            || terminal.operation != fact.operation
            || terminal.revision != fact.revision
            || terminal.ordinal <= fact.ordinal
            || decision.facts.get(fact.ordinal as usize) != Some(&fact.id)
        {
            return Err(Error::Source);
        }
        match fact.cause {
            Some(id) => {
                let cause = state
                    .facts
                    .iter()
                    .find(|event| event.id == id)
                    .ok_or(Error::Source)?;
                let within_decision = cause.operation == fact.operation
                    && cause.revision == fact.revision
                    && cause.ordinal < fact.ordinal
                    && decision.facts.contains(&cause.id);
                let previous_decision = fact.ordinal == 0
                    && decision.facts.first() == Some(&fact.id)
                    && cause.revision < fact.revision
                    && state
                        .facts
                        .iter()
                        .take_while(|earlier| earlier.id != fact.id)
                        .last()
                        == Some(cause);
                if !within_decision && !previous_decision {
                    return Err(Error::Source);
                }
            }
            None if fact.ordinal == 0 && decision.facts.first() == Some(&fact.id) => {}
            None => return Err(Error::Source),
        }
        let accepted = accepted(decision).map_err(|_| Error::Source)?;
        let FactValue::ContentEvent { definition, .. } = &terminal.value else {
            return Err(Error::Source);
        };
        let duration = after
            .ticks
            .checked_sub(before.ticks)
            .ok_or(Error::Duration)?;
        let combat = if *definition == content("end-turn")?
            && [
                rpc::JourneyPhase::Combat as i32,
                rpc::JourneyPhase::Complete as i32,
            ]
            .contains(&accepted.phase)
        {
            true
        } else if *definition == content("short-rest")?
            && accepted.phase == rpc::JourneyPhase::Complete as i32
        {
            false
        } else {
            return Err(Error::Source);
        };
        if duration == 0 {
            return Err(Error::Duration);
        }
        if let Some((_, total, existing_combat)) = advances
            .iter_mut()
            .find(|(operation, _, _)| *operation == fact.operation)
        {
            if *existing_combat != combat {
                return Err(Error::Source);
            }
            *total = total.checked_add(duration).ok_or(Error::Capacity)?;
        } else {
            advances.push((fact.operation, duration, combat));
        }
        time = after;
    }
    for (_, duration, combat) in advances {
        if duration
            != if combat {
                u64::from(pack::combat().round_ticks)
            } else {
                u64::from(pack::rest().short_rest_seconds)
            }
        {
            return Err(Error::Duration);
        }
        if combat {
            round = round.checked_add(1).ok_or(Error::Capacity)?;
        }
    }
    if time != state.logical_time {
        return Err(Error::Chronology);
    }
    Ok(round)
}

/// A hero name is kept in a revision label. A name the label alphabet can hold is stored as typed,
/// as every save before REF-061 did; any other name ("Vix Ashlantern", "D'Artagnan") is stored as
/// `hx:` and its UTF-8 bytes in hex. Typed names never contain `:`, so the two forms cannot collide.
pub(super) fn stored_name_label(name: &str) -> Result<df_types::RevisionLabel, RepositoryError> {
    match df_types::RevisionLabel::new(Some(name)) {
        Ok(label) if !name.contains(':') => Ok(label),
        _ => model::label(&format!("hx:{}", hex(name.as_bytes()))),
    }
}
pub(super) fn stored_name(label: &str) -> Result<String, RepositoryError> {
    match label.strip_prefix("hx:") {
        Some(encoded) => String::from_utf8(unhex(encoded)?).map_err(bad),
        None => Ok(label.to_owned()),
    }
}
pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
pub(super) fn unhex(text: &str) -> Result<Vec<u8>, RepositoryError> {
    if text.len() > model::limits().maximum_text_bytes || !text.len().is_multiple_of(2) {
        return Err(RepositoryError::InvalidReceipt);
    }
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let digit = |byte| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                _ => None,
            };
            Ok(
                (digit(pair[0]).ok_or(RepositoryError::InvalidReceipt)? << 4)
                    | digit(pair[1]).ok_or(RepositoryError::InvalidReceipt)?,
            )
        })
        .collect()
}
pub(super) fn decode_join(
    decision: &AcceptedDecision,
) -> Result<([u8; 16], [u8; 16]), RepositoryError> {
    if decision.source_policy.as_str() != "local-room-join-1" {
        return Err(RepositoryError::InvalidReceipt);
    }
    let text = decision
        .semantic_output
        .as_deref()
        .ok_or(RepositoryError::InvalidReceipt)?;
    let values = text.split(':').collect::<Vec<_>>();
    let ["join", member, entity] = values.as_slice() else {
        return Err(RepositoryError::InvalidReceipt);
    };
    Ok((
        unhex(member)?.try_into().map_err(bad)?,
        unhex(entity)?.try_into().map_err(bad)?,
    ))
}
/// Every joined seat has its own accepted hero; empty tables cannot begin.
pub(super) fn party_ready(current: &Checkpoint) -> bool {
    let party = participants(current);
    (1..=MAXIMUM_PARTY).contains(&party.len())
        && current.state().characters.len() == party.len()
        && party.iter().all(|link| {
            current.state().characters.iter().any(|character| {
                Some(character.entity) == link.character && character.owner == link.member
            })
        })
}
pub(super) fn join_selection(
    current: &Checkpoint,
) -> Result<([u8; 16], [u8; 16]), RepositoryError> {
    let index = participants(current).len();
    Ok((
        *MEMBERS.get(index).ok_or(RepositoryError::Capacity)?,
        *ENTITIES.get(index).ok_or(RepositoryError::Capacity)?,
    ))
}
pub(super) fn stage_join(
    current: &Checkpoint,
    input: &GameInput,
) -> Result<Checkpoint, RepositoryError> {
    current
        .validate_resume(current.basis(), &model::pins()?)
        .map_err(bad)?;
    df_model::commands::validate_client_command(
        input,
        current,
        ReferenceInventory {
            rules: &[model::rule()?, rule()?],
            content: &model::contents()?,
            resources: &resources_for(current.state())?,
            assets: &model::creator_portrait_assets(current.state())?,
        },
        df_model::commands::CommandLimits {
            maximum_records: if df_persistence::local_demo_scope::input_retained_bytes_max(input)
                > 8192
            {
                256
            } else {
                32
            },
            maximum_text_bytes: 128,
            maximum_retained_bytes: df_persistence::local_demo_scope::input_retained_bytes_max(
                input,
            ),
        },
    )
    .map_err(bad)?;
    let GameInput::Game(command) = input else {
        return Err(RepositoryError::InvalidCandidate);
    };
    let GameCommand::ProposeAction {
        actor,
        action,
        targets,
        choices,
    } = &command.command
    else {
        return Err(RepositoryError::InvalidCandidate);
    };
    let (member_id, entity_id) = join_selection(current)?;
    if *actor != room_entity()?
        || command.member != bootstrap_member()?
        || *action != model::content("join-room")?
        || !targets.is_empty()
        || !choices.is_empty()
        || phase(current)? != rpc::JourneyPhase::Room
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let mut basis = current.basis();
    basis.revision = basis.revision.next_sequence().map_err(bad)?;
    let mut state = current.state().clone();
    state.members.push(MembershipLink {
        member: MemberId::from_bytes(&member_id).map_err(bad)?,
        character: Some(entity(entity_id)?),
    });
    state.entities.push(WorldEntity {
        id: entity(entity_id)?,
        definition: model::content("joined-player")?,
        location: Some(room_entity()?),
        position: Some(if entity_id == ENTITIES[0] {
            Position { x: 0, y: 5, z: 0 }
        } else {
            Position {
                x: 5,
                y: ((participants(current).len() - 1) * 5) as i64,
                z: 0,
            }
        }),
        identity_revision: model::label("joined-player-1")?,
    });
    finish(
        current,
        basis,
        state,
        command.operation,
        Vec::new(),
        DecisionContent {
            entry: "join-room",
            semantic: Some(format!("join:{}:{}", hex(&member_id), hex(&entity_id))),
            policy: "local-room-join-1",
            audience: AudienceScope::Shared,
            reaction: None,
            escort_target: None,
            objective_command: None,
            script_choice: None,
        },
    )
}
struct DecisionContent<'a> {
    entry: &'a str,
    semantic: Option<String>,
    policy: &'a str,
    audience: AudienceScope,
    reaction: Option<Box<df_interaction::reactions::ReactionProposal>>,
    escort_target: Option<EntityId>,
    objective_command: Option<&'a CommandInput>,
    script_choice: Option<(ContentReference, EntityId)>,
}
fn finish(
    current: &Checkpoint,
    basis: Basis,
    mut state: GameState,
    operation: OperationId,
    draws: Vec<ActualDraw>,
    content: DecisionContent<'_>,
) -> Result<Checkpoint, RepositoryError> {
    let DecisionContent {
        entry,
        semantic,
        policy,
        audience,
        reaction,
        escort_target,
        objective_command,
        script_choice,
    } = content;
    let objective = objective_command
        .map(|command| objective_outcome::pending(current, &state, command))
        .transpose()
        .inspect_err(|_| refusal_detail::note("finish-objective-pending"))?
        .flatten();
    if (entry == "escort-courier") != escort_target.is_some()
        || (reaction.is_some() && entry != "defend-courier")
    {
        refusal_detail::note("finish-content-contract");
        return Err(RepositoryError::InvalidCandidate);
    }
    let mut facts = Vec::new();
    let make_id = |ordinal: u32| {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(
            [
                operation.as_bytes().as_slice(),
                ordinal.to_le_bytes().as_slice(),
            ]
            .concat(),
        );
        FactId::from_bytes(&digest[..16]).map_err(bad)
    };
    for draw in &draws {
        let fact_id = make_id(draw.ordinal)?;
        state.facts.push(GameFact {
            id: fact_id,
            revision: basis.revision,
            operation,
            ordinal: draw.ordinal,
            cause: current.state().facts.last().map(|fact| fact.id),
            audience: AudienceScope::Shared,
            value: FactValue::DrawAccepted {
                operation,
                ordinal: draw.ordinal,
            },
        });
        facts.push(fact_id);
    }
    if entry == "short-rest" {
        if !short_rest_started(current.state()) {
            let id = make_id(facts.len() as u32)?;
            state.facts.push(GameFact {
                id,
                revision: basis.revision,
                operation,
                ordinal: facts.len() as u32,
                cause: current.state().facts.last().map(|fact| fact.id),
                audience: AudienceScope::Shared,
                value: FactValue::ContentEvent {
                    definition: model::content("short-rest-start")?,
                    subjects: state
                        .characters
                        .iter()
                        .map(|character| character.entity)
                        .collect(),
                },
            });
            facts.push(id);
        }
        if state.logical_time != current.state().logical_time {
            let id = make_id(facts.len() as u32)?;
            state.facts.push(GameFact {
                id,
                revision: basis.revision,
                operation,
                ordinal: facts.len() as u32,
                cause: facts
                    .last()
                    .copied()
                    .or_else(|| current.state().facts.last().map(|fact| fact.id)),
                audience: AudienceScope::Shared,
                value: FactValue::TimeAdvanced {
                    before: current.state().logical_time,
                    after: state.logical_time,
                },
            });
            facts.push(id);
        }
    }
    for resource in &state.resources {
        if let Some(before) = current
            .state()
            .resources
            .iter()
            .find(|value| value.owner == resource.owner && value.resource == resource.resource)
            && before.value != resource.value
        {
            let ordinal = facts.len() as u32;
            let id = make_id(ordinal)?;
            state.facts.push(GameFact {
                id,
                revision: basis.revision,
                operation,
                ordinal,
                cause: facts.last().copied(),
                audience: AudienceScope::Shared,
                value: FactValue::ResourceChanged {
                    entity: resource.owner,
                    resource: resource.resource.clone(),
                    before: before.value,
                    after: resource.value,
                    source: resource.source.clone(),
                },
            });
            facts.push(id);
        }
    }
    for moved in &state.entities {
        if let Some(before) = current
            .state()
            .entities
            .iter()
            .find(|before| before.id == moved.id)
            && before.location != moved.location
        {
            let destination = moved.location.ok_or_else(|| {
                refusal_detail::note("finish-move-destination");
                RepositoryError::InvalidCandidate
            })?;
            let position = moved.position.ok_or_else(|| {
                refusal_detail::note("finish-move-position");
                RepositoryError::InvalidCandidate
            })?;
            let id = make_id(facts.len() as u32)?;
            state.facts.push(GameFact {
                id,
                revision: basis.revision,
                operation,
                ordinal: facts.len() as u32,
                cause: facts.last().copied(),
                audience: AudienceScope::Shared,
                value: FactValue::EntityMoved {
                    entity: moved.id,
                    destination,
                    position,
                },
            });
            facts.push(id);
        }
    }
    let mut evidenced_time = current.state().logical_time;
    for scheduled in &state.schedules {
        if !current
            .state()
            .schedules
            .iter()
            .any(|before| before.id == scheduled.id)
        {
            if scheduled.definition != model::content("knockout-short-rest-due")? {
                refusal_detail::note("finish-schedule-definition");
                return Err(RepositoryError::InvalidCandidate);
            }
            let cause=state.facts.iter().find(|fact|fact.operation==operation && matches!(&fact.value,FactValue::ResourceChanged {entity,resource,before:0,after:1,..} if *entity==scheduled.entity && resource.as_str()=="unconscious")).map(|fact|fact.id).ok_or_else(|| {
                refusal_detail::note("finish-schedule-cause");
                RepositoryError::InvalidCandidate
            })?;
            let started = LogicalTime {
                ticks: scheduled
                    .due
                    .ticks
                    .checked_sub(
                        u64::from(scheduled.due.ticks_per_second)
                            * u64::from(pack::rest().short_rest_seconds),
                    )
                    .ok_or_else(|| {
                        refusal_detail::note("finish-schedule-start");
                        RepositoryError::InvalidCandidate
                    })?,
                ticks_per_second: scheduled.due.ticks_per_second,
            };
            if started.ticks_per_second != evidenced_time.ticks_per_second
                || started.ticks < evidenced_time.ticks
                || started.ticks > state.logical_time.ticks
            {
                refusal_detail::note("finish-schedule-time");
                return Err(RepositoryError::InvalidCandidate);
            }
            if started != evidenced_time {
                let id = make_id(facts.len() as u32)?;
                state.facts.push(GameFact {
                    id,
                    revision: basis.revision,
                    operation,
                    ordinal: facts.len() as u32,
                    cause: Some(cause),
                    audience: AudienceScope::Shared,
                    value: FactValue::TimeAdvanced {
                        before: evidenced_time,
                        after: started,
                    },
                });
                facts.push(id);
                evidenced_time = started;
            }
            let id = make_id(facts.len() as u32)?;
            state.facts.push(GameFact {
                id,
                revision: basis.revision,
                operation,
                ordinal: facts.len() as u32,
                cause: Some(cause),
                audience: AudienceScope::Shared,
                value: FactValue::ContentEvent {
                    definition: model::content("knockout-short-rest-start")?,
                    subjects: vec![scheduled.entity],
                },
            });
            facts.push(id);
        }
    }
    if entry != "short-rest" && state.logical_time != evidenced_time {
        let id = make_id(facts.len() as u32)?;
        state.facts.push(GameFact {
            id,
            revision: basis.revision,
            operation,
            ordinal: facts.len() as u32,
            cause: facts.last().copied(),
            audience: AudienceScope::Shared,
            value: FactValue::TimeAdvanced {
                before: evidenced_time,
                after: state.logical_time,
            },
        });
        facts.push(id);
    }
    if let Some(proposal) = reaction.as_deref() {
        let ordinal = facts.len() as u32;
        let id = make_id(ordinal)?;
        courier_reaction::stage_proposal(
            current,
            &mut state,
            proposal,
            operation,
            basis.revision,
            ordinal,
            id,
        )?;
        facts.push(id);
    }
    if let Some(heroes) = rewards::pay(&mut state)? {
        let ordinal = u32::try_from(facts.len()).map_err(bad)?;
        let id = make_id(ordinal)?;
        state.facts.push(GameFact {
            id,
            revision: basis.revision,
            operation,
            ordinal,
            cause: facts
                .last()
                .copied()
                .or_else(|| current.state().facts.last().map(|fact| fact.id)),
            audience: AudienceScope::Shared,
            value: FactValue::ContentEvent {
                definition: model::content(rewards::PAID)?,
                subjects: heroes,
            },
        });
        facts.push(id);
    }
    if let Some((definition, actor)) = script_choice {
        let ordinal = u32::try_from(facts.len()).map_err(bad)?;
        let id = make_id(ordinal)?;
        state.facts.push(GameFact {
            id,
            revision: basis.revision,
            operation,
            ordinal,
            cause: facts
                .last()
                .copied()
                .or_else(|| current.state().facts.last().map(|fact| fact.id)),
            audience: AudienceScope::Shared,
            value: FactValue::ContentEvent {
                definition,
                subjects: vec![actor],
            },
        });
        facts.push(id);
    }
    let escort_contact = if let Some(target) = escort_target {
        let ordinal = facts.len() as u32;
        let id = make_id(ordinal)?;
        state.facts.push(GameFact {
            id,
            revision: basis.revision,
            operation,
            ordinal,
            cause: facts
                .last()
                .copied()
                .or_else(|| current.state().facts.last().map(|fact| fact.id)),
            audience: AudienceScope::Shared,
            value: FactValue::ContentEvent {
                definition: model::content("courier-escort-contact")?,
                subjects: vec![target],
            },
        });
        facts.push(id);
        Some((id, target))
    } else {
        None
    };
    if let Some((definition, actor)) = &objective {
        let ordinal = u32::try_from(facts.len()).map_err(bad)?;
        let id = objective_outcome::cause_id(
            objective_command.ok_or(RepositoryError::InvalidCandidate)?,
            ordinal,
            definition,
        )?;
        state.facts.push(GameFact {
            id,
            revision: basis.revision,
            operation,
            ordinal,
            cause: facts
                .last()
                .copied()
                .or_else(|| current.state().facts.last().map(|fact| fact.id)),
            audience: AudienceScope::Shared,
            value: FactValue::ContentEvent {
                definition: definition.clone(),
                subjects: vec![*actor],
            },
        });
        facts.push(id);
    }
    let fact_id = make_id(facts.len() as u32)?;
    state.facts.push(GameFact {
        id: fact_id,
        revision: basis.revision,
        operation,
        ordinal: facts.len() as u32,
        cause: if entry == "harbor-inn" {
            Some(inn_source(current)?.ok_or_else(|| {
                refusal_detail::note("finish-inn-source");
                RepositoryError::InvalidCandidate
            })?)
        } else {
            facts
                .last()
                .copied()
                .or_else(|| current.state().facts.last().map(|fact| fact.id))
        },
        audience,
        value: FactValue::ContentEvent {
            definition: model::content(entry)?,
            subjects: if entry == "harbor-inn" {
                participants(current)
                    .into_iter()
                    .map(|link| link.character.ok_or(RepositoryError::InvalidCandidate))
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                Vec::new()
            },
        },
    });
    facts.push(fact_id);
    if let Some((contact_id, target)) = escort_contact {
        courier_reaction::perceive_escort(
            &mut state,
            contact_id,
            operation,
            basis.revision,
            target,
        )?;
    }
    state.decisions.push(AcceptedDecision {
        operation,
        revision: basis.revision,
        facts,
        draws: draws.iter().map(|draw| draw.ordinal).collect(),
        effects: Vec::new(),
        source_policy: model::label(policy)?,
        semantic_output: semantic,
    });
    state.draws.extend(draws);
    let candidate = model::checkpoint(basis, state)
        .inspect_err(|_| refusal_detail::note("finish-checkpoint"))?;
    let candidate = if objective.is_some() {
        objective_outcome::stage(
            current,
            candidate,
            objective_command.ok_or_else(|| {
                refusal_detail::note("finish-objective-command");
                RepositoryError::InvalidCandidate
            })?,
        )
        .inspect_err(|_| refusal_detail::note("finish-objective-stage"))?
    } else {
        candidate
    };
    let candidate = stage_authored_threads(candidate, fact_id)
        .inspect_err(|_| refusal_detail::note("finish-authored-threads"))?;
    phase(&candidate).inspect_err(|_| refusal_detail::note("finish-phase"))?;
    Ok(candidate)
}

/// An EndTurn recorded after the fight was won: the hours and rounds that pass while a hero at 0
/// hit points is dying or Stable. It is the table's time, not a consequence for any thread.
fn after_victory_time(state: &GameState, fact: &GameFact) -> bool {
    let named = |fact: &GameFact, entry: &str| matches!(&fact.value, FactValue::ContentEvent { definition, .. } if definition.entry.as_str() == entry);
    named(fact, "end-turn")
        && state
            .facts
            .iter()
            .take_while(|earlier| earlier.id != fact.id)
            .any(|earlier| named(earlier, "combat-victory"))
}

fn stage_authored_threads(
    candidate: Checkpoint,
    source_id: FactId,
) -> Result<Checkpoint, RepositoryError> {
    if super::beat_script::with_script(|script| {
        script.mode == super::beat_script::ScriptMode::Freeplay
    }) {
        let source = candidate
            .state()
            .facts
            .iter()
            .find(|fact| fact.id == source_id)
            .ok_or(RepositoryError::InvalidCandidate)?;
        if !matches!(&source.value, FactValue::ContentEvent { definition, subjects }
            if definition.entry.as_str() == "begin-story" && subjects.is_empty())
        {
            return Ok(candidate);
        }
        // Entering freeplay still consumes its exact accepted entry source. The legacy
        // thread director performed this bookkeeping while progressing a quest; freeplay
        // preserves the causal fact without creating that unrelated story thread.
        if source.audience != AudienceScope::Shared
            || candidate
                .state()
                .narrative
                .accepted_facts
                .contains(&source_id)
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        let mut state = candidate.state().clone();
        state.narrative.accepted_facts.push(source_id);
        return model::checkpoint(candidate.basis(), state);
    }
    use df_engine::director_staging::{
        DirectorLimits, DirectorStaging, ThreadDirectorLimits, compose_director_thread_progress,
    };
    use df_narrative::{
        ProgressLimits, ThreadCheckpointRequest, ThreadConsequenceSelection, ThreadDisposition,
    };
    let source = candidate
        .state()
        .facts
        .iter()
        .find(|fact| fact.id == source_id)
        .ok_or(RepositoryError::InvalidCandidate)?;
    let FactValue::ContentEvent { definition, .. } = &source.value else {
        return Err(RepositoryError::InvalidCandidate);
    };
    // Time passing after a won fight (a dying hero's timed Death Saving Throws, a Stable hero's
    // recovery) adds nothing to the threat thread: the winning blow already resolved it.
    if after_victory_time(candidate.state(), source) {
        return Ok(candidate);
    }
    let mut admitted_thread = None;
    for &(event, thread) in THREAD_EVENTS {
        if *definition == model::content(event)? {
            admitted_thread = Some(model::content(thread)?);
            break;
        }
    }
    let Some(thread) = admitted_thread else {
        return Ok(candidate);
    };
    if !candidate.state().decisions.iter().any(|decision| {
        decision.operation == source.operation
            && decision.revision == source.revision
            && decision.source_policy.as_str() == THREAD_POLICY
            && decision.facts.contains(&source_id)
    }) {
        return Err(RepositoryError::InvalidCandidate);
    }
    let bandit = entity(BANDIT)?;
    let unconscious = model::label("unconscious")?;
    let source_rule = rule()?;
    let defeated = if tactical::present(candidate.state()) {
        thread == model::content(THREAT_THREAD)?
            && tactical::read(candidate.state())?.winner() == Some(df_rules::tactical::Side::Heroes)
    } else {
        thread == model::content(THREAT_THREAD)? && candidate.state().facts.iter().any(|fact| {
        fact.operation == source.operation && fact.revision == source.revision && fact.ordinal < source.ordinal
            && matches!(&fact.value, FactValue::ResourceChanged { entity, resource, before: 0, after: 1, source }
                if *entity == bandit && *resource == unconscious && *source == source_rule)
    })
    };
    let selections = [ThreadConsequenceSelection {
        thread: &thread,
        event_definition: definition,
        source: source_id,
        disposition: if defeated {
            ThreadDisposition::Resolve
        } else {
            ThreadDisposition::Continue
        },
    }];
    let policy = model::label(THREAD_POLICY)?;
    let pins = model::pins()?;
    let staged = compose_director_thread_progress(
        &candidate,
        &pins,
        df_world::DueSelectionRequest {
            expected_basis: candidate.basis(),
            target_time: candidate.state().logical_time,
            paused: false,
            deadline_remaining: Duration::from_secs(1),
            // A scene continuation must honor the admitted schedule cursor's policy.
            // Replacing it with a narrative definition makes a valid post-Rest cursor stale.
            policy: candidate
                .state()
                .continuity
                .catch_up
                .as_ref()
                .map(|cursor| &cursor.policy)
                .unwrap_or(&candidate.state().narrative.definition),
        },
        None,
        ThreadCheckpointRequest {
            expected_basis: candidate.basis(),
            admitted_pins: &pins,
            policy: &policy,
            expected_policy: &policy,
            inventory: ReferenceInventory {
                rules: &[model::rule()?, source_rule],
                content: &model::contents()?,
                resources: &resources_for(candidate.state())?,
                assets: &model::creator_portrait_assets(candidate.state())?,
            },
            checkpoint_limits: model::limits(),
            selections: &selections,
        },
        ThreadDirectorLimits {
            directors: DirectorLimits {
                maximum_checkpoint_bytes: model::limits().maximum_retained_bytes,
                // Preserve the eight-checkpoint staging budget when six-seat snapshots grow.
                maximum_pass_bytes: 8 * model::limits().maximum_retained_bytes,
                maximum_relationships: 512,
                world: df_world::DueSelectionLimits {
                    queue_events: 512,
                    selected_events: 8,
                    output_bytes: 128 * 1024,
                },
            },
            narrative: ProgressLimits {
                records: 512,
                consequences: 1,
                work: 1024 * 1024,
            },
        },
    )
    .map_err(bad)?;
    match staged {
        DirectorStaging::Staged(staged) => Ok((*staged).into_candidate()),
        // This source-owned event may suspend at its own exact window. Preserve that staged
        // candidate for the session's transition classifier; an unrelated prior window cannot
        // turn this operation into pending. The director did not run downstream narrative work.
        DirectorStaging::RulesPending(pending)
            if pending
                .iter()
                .filter(|resolution| resolution.window.causal_fact == source_id)
                .count()
                == 1 =>
        {
            Ok(candidate)
        }
        DirectorStaging::RulesPending(_)
        | DirectorStaging::WorldPending(_)
        | DirectorStaging::MissingEnvironmentalProducer => Err(RepositoryError::InvalidCandidate),
    }
}

fn command(input: &GameInput) -> Result<&CommandInput, RepositoryError> {
    match input {
        GameInput::Game(command) => Ok(command),
        _ => Err(RepositoryError::InvalidCandidate),
    }
}
fn choice<'a>(
    choices: &'a [(df_types::RevisionLabel, df_types::RevisionLabel)],
    key: &str,
) -> Result<&'a str, RepositoryError> {
    choices
        .iter()
        .find(|(label, _)| label.as_str() == key)
        .map(|(_, value)| value.as_str())
        .ok_or(RepositoryError::InvalidCandidate)
}
type DiceSource<'a> = &'a mut dyn FnMut(u32) -> Result<u32, RepositoryError>;
fn sample_face(sides: u32) -> Result<u32, RepositoryError> {
    if sides != 20
        && !((tactical::enabled() || tactical::scene_assault_only())
            && [4, 6, 8, 10, 12].contains(&sides))
        && ![
            u32::from(pack::hero_weapon().damage_sides),
            u32::from(pack::bandit().weapon.damage_sides),
            u32::from(pack::second_wind().die_sides),
        ]
        .contains(&sides)
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    super::dice::draw(sides)
}
fn draw(
    operation: OperationId,
    sides: u32,
    draws: &mut Vec<ActualDraw>,
    supplier: DiceSource<'_>,
) -> Result<u32, RepositoryError> {
    if draws.len() >= 128 {
        return Err(RepositoryError::Capacity);
    }
    let value = supplier(sides)?;
    if !(1..=sides).contains(&value) {
        return Err(RepositoryError::InvalidCandidate);
    }
    draws.push(ActualDraw {
        operation,
        ordinal: draws.len() as u32,
        resolution: ResolutionId::from_bytes(operation.as_bytes()).map_err(bad)?,
        window: WindowId::from_bytes(operation.as_bytes()).map_err(bad)?,
        sides,
        value,
        source: rule()?,
    });
    Ok(value)
}
#[derive(Clone, Copy)]
struct AttackOptions {
    savage: bool,
    graze: bool,
}
fn held_weapon_id(who: EntityId) -> Result<EntityId, RepositoryError> {
    let mut id = *who.as_bytes();
    id[0] ^= 0x80;
    id[15] ^= if who == entity(BANDIT)? {
        1
    } else {
        u8::try_from(
            pack::COMPILED_ITEMS
                .iter()
                .position(|item| *item == pack::hero_weapon_item())
                .ok_or(RepositoryError::InvalidCandidate)?
                + 1,
        )
        .map_err(bad)?
    };
    entity(id)
}
fn knockout_rest_id(
    operation: OperationId,
    who: EntityId,
    started: u64,
) -> Result<RecordId, RepositoryError> {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(
        [
            b"source-knockout-rest-1".as_slice(),
            operation.as_bytes(),
            who.as_bytes(),
            &started.to_be_bytes(),
        ]
        .concat(),
    );
    RecordId::from_bytes(&digest[..16]).map_err(bad)
}
fn start_knockout_rest(
    state: &mut GameState,
    operation: OperationId,
    who: EntityId,
) -> Result<(), RepositoryError> {
    if value(state, who, "hit-points")? != 1 || value(state, who, "held-weapon")? != 1 {
        return Err(RepositoryError::InvalidCandidate);
    }
    set(state, who, "unconscious", 1)?;
    set(state, who, "prone", 1)?;
    set(state, who, "held-weapon", 0)?;
    let position = state
        .entities
        .iter()
        .find(|entity| entity.id == who)
        .and_then(|entity| entity.position)
        .ok_or(RepositoryError::InvalidCandidate)?;
    let weapon = held_weapon_id(who)?;
    let item = state
        .entities
        .iter_mut()
        .find(|entity| entity.id == weapon && entity.location == Some(who))
        .ok_or(RepositoryError::InvalidCandidate)?;
    item.location = Some(room_entity()?);
    item.position = Some(position);
    let due = LogicalTime {
        ticks: rules::short_rest_due(
            state.logical_time.ticks,
            state.logical_time.ticks_per_second,
            pack::rest().short_rest_seconds,
        )
        .map_err(bad)?,
        ticks_per_second: state.logical_time.ticks_per_second,
    };
    state.schedules.push(ScheduledEvent {
        id: knockout_rest_id(operation, who, state.logical_time.ticks)?,
        entity: who,
        due,
        definition: model::content("knockout-short-rest-due")?,
    });
    Ok(())
}

pub(super) fn combat_victory(state: &GameState) -> Result<bool, RepositoryError> {
    if tactical::present(state) {
        return tactical::victory(state);
    }
    let [encounter] = state.encounters.as_slice() else {
        return Err(RepositoryError::InvalidCandidate);
    };
    if encounter.active_turn.is_some() {
        return Err(RepositoryError::InvalidCandidate);
    }
    objective_outcome::outcome(state)
}
fn knockout_rest_started(
    current: &Checkpoint,
    event: &ScheduledEvent,
) -> Result<u64, RepositoryError> {
    if event.definition != model::content("knockout-short-rest-due")?
        || event.due.ticks_per_second != current.state().logical_time.ticks_per_second
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let duration = u64::from(event.due.ticks_per_second)
        .checked_mul(u64::from(pack::rest().short_rest_seconds))
        .ok_or(RepositoryError::Capacity)?;
    let started = event
        .due
        .ticks
        .checked_sub(duration)
        .ok_or(RepositoryError::InvalidCandidate)?;
    if started > current.state().logical_time.ticks
        || value(current.state(), event.entity, "unconscious")? != 1
        || value(current.state(), event.entity, "prone")? != 1
        || value(current.state(), event.entity, "held-weapon")? != 0
        || value(current.state(), event.entity, "hit-points")? != 1
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let start_definition = model::content("knockout-short-rest-start")?;
    let start=current.state().facts.iter().find(|fact| matches!(&fact.value,FactValue::ContentEvent { definition,subjects } if definition==&start_definition && subjects.as_slice()==[event.entity]) && knockout_rest_id(fact.operation,event.entity,started)==Ok(event.id)).ok_or(RepositoryError::InvalidCandidate)?;
    let historical_time = current
        .state()
        .facts
        .iter()
        .filter(|fact| (fact.revision, fact.ordinal) < (start.revision, start.ordinal))
        .filter_map(|fact| match fact.value {
            FactValue::TimeAdvanced { after, .. } => Some(((fact.revision, fact.ordinal), after)),
            _ => None,
        })
        .max_by_key(|(order, _)| *order)
        .map(|(_, time)| time)
        .unwrap_or(LogicalTime {
            ticks: 0,
            ticks_per_second: current.state().logical_time.ticks_per_second,
        });
    if historical_time.ticks != started
        || historical_time.ticks_per_second != event.due.ticks_per_second
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let cause = current
        .state()
        .facts
        .iter()
        .find(|fact| Some(fact.id) == start.cause && fact.operation == start.operation)
        .ok_or(RepositoryError::InvalidCandidate)?;
    if !matches!(&cause.value,FactValue::ResourceChanged { entity,resource,before:0,after:1,source } if *entity==event.entity && resource.as_str()=="unconscious" && *source==rule()?)
        || !current.state().decisions.iter().any(|decision| {
            decision.operation == start.operation
                && decision.source_policy.as_str() == THREAD_POLICY
                && decision.facts.contains(&start.id)
                && decision.facts.contains(&cause.id)
        })
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    for fact in current
        .state()
        .facts
        .iter()
        .filter(|fact| (fact.revision, fact.ordinal) > (start.revision, start.ordinal))
    {
        let interrupted = match &fact.value {
            FactValue::ResourceChanged {
                entity,
                resource,
                before,
                after,
                ..
            } => *entity == event.entity && resource.as_str() == "hit-points" && after < before,
            FactValue::ContentEvent {
                definition,
                subjects,
            } => {
                (definition.entry.as_str() == "defend-courier" && fact.operation != start.operation)
                    || (definition.entry.as_str() == "short-rest-interrupted"
                        && subjects.contains(&event.entity))
            }
            _ => false,
        };
        if interrupted {
            return Err(RepositoryError::InvalidCandidate);
        }
    }
    Ok(started)
}
fn prepare_short_rest(current: &Checkpoint) -> Result<df_world::DueSelection<'_>, RepositoryError> {
    if phase(current)? != rpc::JourneyPhase::Complete
        || !current.state().threats.is_empty()
        || !current.state().pending.is_empty()
        || !current.state().timers.is_empty()
        || !current.state().active_effects.is_empty()
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    combat_victory(current.state())?;
    let target = LogicalTime {
        ticks: rules::short_rest_due(
            current.state().logical_time.ticks,
            current.state().logical_time.ticks_per_second,
            pack::rest().short_rest_seconds,
        )
        .map_err(bad)?,
        ticks_per_second: current.state().logical_time.ticks_per_second,
    };
    let policy = model::content("short-rest")?;
    let selection = df_world::select_due_events(
        current,
        current.pins(),
        df_world::DueSelectionRequest {
            expected_basis: current.basis(),
            target_time: target,
            paused: false,
            deadline_remaining: Duration::from_secs(1),
            policy: &policy,
        },
        df_world::DueSelectionLimits {
            queue_events: 8,
            selected_events: 8,
            output_bytes: 65536,
        },
    )
    .map_err(bad)?;
    if selection.remaining_due() != 0 {
        return Err(RepositoryError::InvalidCandidate);
    }
    for (index, event) in selection.events.iter().enumerate() {
        knockout_rest_started(current, event)?;
        if selection.events[..index]
            .iter()
            .any(|previous| previous.entity == event.entity)
        {
            return Err(RepositoryError::InvalidCandidate);
        }
    }
    if tactical::present(current.state()) {
        if tactical::dying(current.state())? {
            return Err(RepositoryError::InvalidCandidate);
        }
        return Ok(selection);
    }
    for who in current
        .state()
        .characters
        .iter()
        .map(|character| character.entity)
        .chain([entity(BANDIT)?])
    {
        if value(current.state(), who, "unconscious")? != 0
            && !selection.events.iter().any(|event| event.entity == who)
        {
            return Err(RepositoryError::InvalidCandidate);
        }
    }
    Ok(selection)
}
fn complete_short_rest(
    current: &Checkpoint,
    state: &mut GameState,
    operation: OperationId,
) -> Result<(), RepositoryError> {
    let selection = prepare_short_rest(current)?;
    if tactical::present(state) {
        tactical::rest(state, operation)?;
        state.logical_time = selection.proposed_time;
        state.continuity.catch_up = Some(selection.cursor);
        let completed = model::content("short-rest-complete")?;
        if !state.narrative.completed_beats.contains(&completed) {
            state.narrative.completed_beats.push(completed);
        }
        return Ok(());
    }
    let fighters = state
        .characters
        .iter()
        .map(|character| character.entity)
        .collect::<Vec<_>>();
    for who in fighters.iter().copied().chain([entity(BANDIT)?]) {
        let start = selection
            .events
            .iter()
            .find(|event| event.entity == who)
            .map(|event| knockout_rest_started(current, event))
            .transpose()?
            .unwrap_or(current.state().logical_time.ticks);
        let fighter = fighters.contains(&who);
        let benefits = rules::complete_no_hit_dice_short_rest(
            &pack::rest(),
            rules::NoHitDiceShortRest {
                started: start,
                finished: selection.proposed_time.ticks,
                ticks_per_second: selection.proposed_time.ticks_per_second,
                interrupted: false,
                hit_points: value(state, who, "hit-points")?,
                fighter_second_wind_uses: if fighter {
                    Some(value(state, who, "second-wind")?)
                } else {
                    None
                },
                knockout_unconscious: value(state, who, "unconscious")? != 0,
            },
            pack::second_wind().uses_max,
            pack::second_wind().recharge_per_short_rest,
        )
        .map_err(bad)?;
        if let Some(uses) = benefits.fighter_second_wind_uses {
            set(state, who, "second-wind", uses)?;
        }
        if benefits.ends_knockout_unconscious {
            set(state, who, "unconscious", 0)?;
        }
    }
    state.logical_time = selection.proposed_time;
    state.continuity.catch_up = Some(selection.cursor);
    let completed = model::content("short-rest-complete")?;
    if !state.narrative.completed_beats.contains(&completed) {
        state.narrative.completed_beats.push(completed);
    }
    Ok(())
}

pub(super) fn short_rest_started(state: &GameState) -> bool {
    let fight_end = state.facts.iter().rposition(|fact| {
        matches!(&fact.value, FactValue::ContentEvent { definition, .. }
            if matches!(definition.entry.as_str(), "combat-victory" | "combat-defeat"))
    });
    state.facts.iter().enumerate().any(|(index, fact)| {
        fight_end.is_none_or(|end| index > end)
            && matches!(&fact.value, FactValue::ContentEvent { definition, .. }
                if definition.entry.as_str() == "short-rest-start")
    })
}

pub(super) fn short_rest_completed(state: &GameState) -> bool {
    let start = state.facts.iter().rposition(|fact| {
        matches!(&fact.value, FactValue::ContentEvent { definition, .. }
            if definition.entry.as_str() == "short-rest-start")
    });
    start.is_some_and(|start| {
        state.facts.iter().enumerate().any(|(index, fact)| {
            index > start
                && matches!(&fact.value, FactValue::TimeAdvanced { before, after }
                    if after.ticks > before.ticks)
        })
    })
}

fn hit_point_die_eligible(state: &GameState, who: EntityId) -> Result<bool, RepositoryError> {
    let hit_points = value(state, who, "hit-points")?;
    Ok(hit_points > 0
        && hit_points < maximum_hit_points(state, who)?
        && value(state, who, "hit-point-dice")? > 0)
}

fn short_rest_offer_label(state: &GameState, who: EntityId) -> Result<String, RepositoryError> {
    let dice = value(state, who, "hit-point-dice")?;
    let dice_label = match dice {
        0 => "no Hit Point Dice to spend".to_owned(),
        1 => "1 Hit Point Die to spend".to_owned(),
        count => format!("{count} Hit Point Dice to spend"),
    };
    Ok(format!("Short Rest · 1 hour · {dice_label}"))
}

pub(super) fn pending_hit_point_die_choices(state: &GameState) -> Result<bool, RepositoryError> {
    for character in &state.characters {
        if hit_point_die_offered(state, character.entity)? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn hit_point_die_offered(
    state: &GameState,
    who: EntityId,
) -> Result<bool, RepositoryError> {
    Ok(hit_point_die_eligible(state, who)?
        && (!short_rest_started(state) || value(state, who, "short-rest-choice")? != 1))
}

pub(in crate::gameplay) fn short_rest_choice_groups(
    current: &Checkpoint,
    member: MemberId,
) -> Result<Vec<rpc::JourneyGroup>, RepositoryError> {
    if session_machine::SessionState::of(current)? != session_machine::SessionState::Aftermath
        || !short_rest_started(current.state())
        || short_rest_completed(current.state())
    {
        return Ok(Vec::new());
    }
    let who = player_entity(member, current)?;
    if !hit_point_die_offered(current.state(), who)? {
        return Ok(Vec::new());
    }
    let keep_label = if value(current.state(), who, "hit-point-dice")? == 1 {
        "Keep the remaining Hit Point Die"
    } else {
        "Keep the remaining Hit Point Dice"
    };
    Ok(vec![rpc::JourneyGroup {
        group_id: "hit-point-die".to_owned(),
        label: "Hit Point Die".to_owned(),
        options: [("spend", "Spend 1 Hit Point Die"), ("keep", keep_label)]
            .into_iter()
            .map(|(option_id, label)| rpc::JourneyOption {
                option_id: option_id.to_owned(),
                label: label.to_owned(),
                ..Default::default()
            })
            .collect(),
    }])
}

fn start_short_rest(
    current: &Checkpoint,
    state: &mut GameState,
    operation: OperationId,
) -> Result<(), RepositoryError> {
    prepare_short_rest(current)?;
    let heroes = state
        .characters
        .iter()
        .map(|character| character.entity)
        .collect::<Vec<_>>();
    for who in heroes {
        set(state, who, "short-rest-choice", 0)?;
        set(state, who, "short-rest-die-roll", 0)?;
    }
    if !pending_hit_point_die_choices(state)? {
        complete_short_rest(current, state, operation)?;
    }
    Ok(())
}

fn choose_short_rest_hit_point_die(
    current: &Checkpoint,
    state: &mut GameState,
    who: EntityId,
    selected: &str,
    operation: OperationId,
    draws: &mut Vec<ActualDraw>,
    supplier: DiceSource<'_>,
) -> Result<(), RepositoryError> {
    if !short_rest_started(current.state()) {
        refusal_detail::note("short-rest-not-started");
        return Err(RepositoryError::InvalidCandidate);
    }
    if !hit_point_die_offered(current.state(), who)? {
        refusal_detail::note("short-rest-die-ineligible");
        return Err(RepositoryError::InvalidCandidate);
    }
    match selected {
        "keep" => set(state, who, "short-rest-choice", 1)?,
        "spend" => {
            let die = draw(
                operation,
                rules::LEVEL_ONE_FIGHTER_HIT_POINT_DIE_SIDES,
                draws,
                supplier,
            )?;
            let constitution = u8::try_from(value(state, who, "constitution")?).map_err(bad)?;
            let constitution_modifier = rules::ability_modifier(constitution);
            let (hit_points, _) = rules::heal_hit_point_die(
                die,
                constitution_modifier,
                value(state, who, "hit-points")?,
                maximum_hit_points(state, who)?,
            )
            .map_err(|_| {
                refusal_detail::note("short-rest-hit-die-heal");
                RepositoryError::InvalidCandidate
            })?;
            let remaining = value(state, who, "hit-point-dice")?
                .checked_sub(1)
                .ok_or(RepositoryError::InvalidCandidate)?;
            set(state, who, "hit-point-dice", remaining)?;
            set(state, who, "hit-points", hit_points)?;
            if tactical::present(state) {
                tactical::sync_hero_hit_points(state, who, hit_points)?;
            }
            set(state, who, "short-rest-choice", 2)?;
            set(state, who, "short-rest-die-roll", die)?;
        }
        _ => {
            refusal_detail::note("short-rest-hit-die-option");
            return Err(RepositoryError::InvalidCandidate);
        }
    }
    if !pending_hit_point_die_choices(state)? {
        complete_short_rest(current, state, operation)?;
    }
    Ok(())
}

fn attack(
    state: &mut GameState,
    operation: OperationId,
    who: EntityId,
    target: EntityId,
    options: AttackOptions,
    draws: &mut Vec<ActualDraw>,
    supplier: DiceSource<'_>,
) -> Result<rpc::CombatOutcome, RepositoryError> {
    let AttackOptions { savage, graze } = options;
    if value(state, who, "unconscious")? != 0 || value(state, target, "unconscious")? != 0 {
        return Err(RepositoryError::InvalidCandidate);
    }
    let monster = state
        .encounters
        .first()
        .is_some_and(|encounter| encounter.participants.contains(&who))
        && !state
            .characters
            .iter()
            .any(|character| character.entity == who);
    let creature = combat_creature(state, who);
    let weapon = if monster {
        creature.rules.weapon
    } else {
        pack::hero_weapon()
    };
    let ability = if monster {
        creature.rules.damage_modifier
    } else {
        rules::ability_modifier(value(state, who, "strength")? as u8)
    };
    let modifier = if monster {
        creature.rules.attack_bonus
    } else {
        ability + pack::with_creation(|rules| i32::from(rules.proficiency_bonus))
    };
    let die = draw(operation, 20, draws, supplier)?;
    let ac = value(state, target, "armor-class")?;
    let hit = die != 1
        && (die >= u32::from(pack::combat().critical_threshold)
            || die as i32 + modifier >= ac as i32);
    let mut dice = Vec::new();
    let count = u32::from(weapon.damage_dice)
        * (if die >= u32::from(pack::combat().critical_threshold) {
            2
        } else {
            1
        })
        * (if savage { 2 } else { 1 });
    if hit {
        for _ in 0..count {
            dice.push(draw(
                operation,
                u32::from(weapon.damage_sides),
                draws,
                supplier,
            )?);
        }
    }
    let outcome = rules::resolve_melee(rules::NormalMeleeAttack {
        critical_threshold: pack::combat().critical_threshold,
        damage_sides: weapon.damage_sides,
        die,
        modifier,
        armor_class: ac,
        damage_dice: &dice,
        weapon_dice: usize::from(weapon.damage_dice),
        damage_modifier: ability,
        savage_attacker_take_higher: savage,
        graze,
        target_hit_points: value(state, target, "hit-points")?,
    })
    .map_err(bad)?;
    set(state, target, "hit-points", outcome.target_hit_points)?;
    if outcome.knocked_out {
        start_knockout_rest(state, operation, target)?;
    }
    Ok(rpc::CombatOutcome {
        actor_name: name(state, who)?,
        target_name: name(state, target)?,
        attack_die: die,
        attack_modifier: modifier,
        attack_total: outcome.total,
        target_armor_class: ac,
        hit: outcome.hit,
        critical: outcome.critical,
        damage_dice: outcome.damage_dice,
        damage: outcome.damage,
        target_hit_points: outcome.target_hit_points,
        knocked_out: outcome.knocked_out,
        grazed: outcome.grazed,
        source_revision: model::source_revision(),
        actor_id: who.as_bytes().to_vec(),
        target_id: target.as_bytes().to_vec(),

        ..Default::default()
    })
}
fn finish_combat(state: &mut GameState) -> Result<bool, RepositoryError> {
    let [encounter] = state.encounters.as_slice() else {
        return Err(RepositoryError::InvalidCandidate);
    };
    if !(1..=MAXIMUM_PARTY).contains(&state.characters.len())
        || !matches!(
            encounter.objectives.as_slice(),
            [objective]
                if *objective == model::content("defend-courier")?
                    || *objective == model::content("scene-npc-assault")?
        )
        || encounter.active_turn.is_none()
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let enemy = combat_enemy(state)?;
    let all_players_down = state
        .characters
        .iter()
        .map(|character| value(state, character.entity, "unconscious"))
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .all(|value| *value != 0);
    let enemy_down = value(state, enemy, "unconscious")? != 0;
    if enemy_down && all_players_down {
        return Err(RepositoryError::InvalidCandidate);
    }
    if enemy_down || all_players_down {
        let encounter = state
            .encounters
            .first_mut()
            .ok_or(RepositoryError::InvalidCandidate)?;
        encounter.active_turn = None;
        beat(state, "complete")?;
        return Ok(true);
    }
    Ok(false)
}
fn advance(state: &mut GameState) -> Result<(), RepositoryError> {
    let encounter = state
        .encounters
        .first()
        .ok_or(RepositoryError::InvalidCandidate)?;
    let who = encounter
        .active_turn
        .ok_or(RepositoryError::InvalidCandidate)?;
    let order = encounter.turn_order.clone();
    let index = order
        .iter()
        .position(|value| *value == who)
        .ok_or(RepositoryError::InvalidCandidate)?;
    for offset in 1..=order.len() {
        let next = (index + offset) % order.len();
        if value(state, order[next], "unconscious")? == 0 {
            if next <= index {
                state.logical_time.ticks = state
                    .logical_time
                    .ticks
                    .checked_add(u64::from(pack::combat().round_ticks))
                    .ok_or(RepositoryError::Capacity)?;
            }
            state
                .encounters
                .first_mut()
                .ok_or(RepositoryError::InvalidCandidate)?
                .active_turn = Some(order[next]);
            set(state, order[next], "action-used", 0)?;
            set(state, order[next], "bonus-used", 0)?;
            return Ok(());
        }
    }
    Err(RepositoryError::InvalidCandidate)
}
fn enemy_turn(
    current: &Checkpoint,
    state: &mut GameState,
    operation: OperationId,
    draws: &mut Vec<ActualDraw>,
    outcomes: &mut Vec<rpc::CombatOutcome>,
    supplier: DiceSource<'_>,
) -> Result<(), RepositoryError> {
    if finish_combat(state)? {
        return Ok(());
    }
    let monster = combat_enemy(state)?;
    if state
        .encounters
        .first()
        .ok_or(RepositoryError::InvalidCandidate)?
        .active_turn
        != Some(monster)
    {
        return Ok(());
    }
    if let Some(target) = enemy_tactics::select(current, state, operation)? {
        let economy = rules::ActionEconomy {
            action_used: value(state, monster, "action-used")? != 0,
            bonus_action_used: value(state, monster, "bonus-used")? != 0,
        }
        .attack()
        .map_err(bad)?;
        outcomes.push(attack(
            state,
            operation,
            monster,
            target,
            AttackOptions {
                savage: false,
                graze: false,
            },
            draws,
            supplier,
        )?);
        set(
            state,
            monster,
            "action-used",
            u32::from(economy.action_used),
        )?;
    }
    if !finish_combat(state)? {
        advance(state)?;
    }
    Ok(())
}
fn start_combat(
    current: &Checkpoint,
    command: &CommandInput,
    state: &mut GameState,
    draws: &mut Vec<ActualDraw>,
    outcomes: &mut Vec<rpc::CombatOutcome>,
    supplier: DiceSource<'_>,
) -> Result<(), RepositoryError> {
    let operation = command.operation;
    let scene_binding = encounter_admission::scene_attack_target(current, command)?;
    if scene_binding.is_some() && !tactical::enabled() {
        return Err(RepositoryError::InvalidCandidate);
    }
    let monster = scene_binding.map_or(entity(BANDIT)?, |binding| binding.entity);
    let enemy_rules = scene_binding.map_or_else(pack::bandit, |binding| binding.creature.rules);
    if scene_binding.is_some() {
        let courier = state
            .entities
            .iter_mut()
            .find(|candidate| candidate.id == monster)
            .ok_or(RepositoryError::InvalidCandidate)?;
        courier.location = Some(room_entity()?);
        courier.position = Some(Position { x: 0, y: 0, z: 0 });
    } else {
        state.entities.push(WorldEntity {
            id: monster,
            definition: model::content("bandit")?,
            location: Some(room_entity()?),
            position: Some(Position { x: 0, y: 0, z: 0 }),
            identity_revision: model::label("srd521-bandit-1")?,
        });
    }
    for (key, value) in [
        ("hit-points", i64::from(enemy_rules.hit_points)),
        ("armor-class", i64::from(enemy_rules.armor_class)),
        ("strength", i64::from(enemy_rules.abilities[0])),
        ("dexterity", i64::from(enemy_rules.abilities[1])),
        ("constitution", i64::from(enemy_rules.abilities[2])),
        ("intelligence", i64::from(enemy_rules.abilities[3])),
        ("wisdom", i64::from(enemy_rules.abilities[4])),
        ("charisma", i64::from(enemy_rules.abilities[5])),
        ("second-wind", 0),
        ("unconscious", 0),
        ("prone", 0),
        ("held-weapon", 1),
        ("action-used", 0),
        ("bonus-used", 0),
    ] {
        add_resource(state, monster, key, value)?;
    }
    let weapon = held_weapon_id(monster)?;
    let weapon_item = if scene_binding.is_some() {
        "club".to_owned()
    } else {
        pack::bandit_weapon_item()
    };
    state.entities.push(WorldEntity {
        id: weapon,
        definition: model::content(&weapon_item)?,
        location: Some(monster),
        position: None,
        identity_revision: model::label("source-held-scimitar-1")?,
    });
    state.inventory.push(InventoryItem {
        item: weapon,
        owner: monster,
        quantity: 1,
        source: rule()?,
        origin: model::content(&weapon_item)?,
        attunement_owner: None,
    });
    let participants = state
        .members
        .iter()
        .filter(|link| link.member.as_bytes() != &PLAYER)
        .map(|link| {
            link.character.ok_or_else(|| {
                refusal_detail::note("combat-participant-missing-character");
                RepositoryError::InvalidCandidate
            })
        })
        .chain([Ok(monster)])
        .collect::<Result<Vec<_>, _>>()?;
    let admitted = encounter_admission::admission(current, command, state, &participants).map_err(
        |error| {
            refusal_detail::cause("combat-admission", format!("{error:?}"));
            bad(error)
        },
    )?;
    for _ in admitted.actors() {
        draw(operation, 20, draws, supplier)?;
    }
    let pins = model::pins()?;
    let source = rule()?;
    let input = GameInput::Game(command.clone());
    let order = df_engine::encounter_start::local_journey_turn_order(
        df_rules::RulesCommandInput {
            command: &input,
            supplied_draws: draws,
        },
        current,
        df_engine::command_entry::CommandEntryContext {
            current_basis: current.basis(),
            admitted_pins: &pins,
            inventory: ReferenceInventory {
                rules: &[model::rule()?, source.clone()],
                content: &model::contents()?,
                resources: &resources_for(current.state())?,
                assets: &model::creator_portrait_assets(current.state())?,
            },
            limits: df_engine::command_entry::CommandEntryLimits {
                command: df_model::commands::CommandLimits {
                    maximum_records: 32,
                    maximum_text_bytes: 128,
                    maximum_retained_bytes: 8192,
                },
                maximum_staged_bytes: model::limits().maximum_retained_bytes,
            },
        },
        &admitted,
        state,
        &source,
    )
    .map_err(|error| {
        refusal_detail::note("combat-turn-order");
        bad(error)
    })?
    .to_vec();
    let active = order.first().copied().ok_or_else(|| {
        refusal_detail::note("combat-empty-turn-order");
        RepositoryError::InvalidCandidate
    })?;
    state.encounters.push(EncounterState {
        id: RecordId::from_bytes(&ENCOUNTER).map_err(bad)?,
        definition: model::content("combat")?,
        participants,
        turn_order: order,
        active_turn: Some(active),
        objectives: vec![model::content(
            scene_binding.map_or("defend-courier", |binding| binding.objective),
        )?],
        combat_policy: model::content(if scene_binding.is_some() {
            "tactical-srd521-cover-policy"
        } else {
            "normal-nonlethal-melee"
        })?,
        tactical: None,
    });
    beat(state, "combat")?;
    if scene_binding.is_some() {
        Ok(())
    } else {
        enemy_turn(current, state, operation, draws, outcomes, supplier)
    }
}
fn accept_character(
    state: &mut GameState,
    who: EntityId,
    member: MemberId,
    choices: &[(df_types::RevisionLabel, df_types::RevisionLabel)],
) -> Result<(), RepositoryError> {
    accept_character_at_level(state, who, member, choices, tactical::test_fighter_level()?)
}
fn accept_character_at_level(
    state: &mut GameState,
    who: EntityId,
    member: MemberId,
    choices: &[(df_types::RevisionLabel, df_types::RevisionLabel)],
    test_level: u8,
) -> Result<(), RepositoryError> {
    let encoded = choice(choices, "name")
        .inspect_err(|_| refusal_detail::cause("character-accept", "NameMissing".to_owned()))?;
    let encoded_name = if encoded == "empty-name" {
        Vec::new()
    } else {
        unhex(encoded)
            .inspect_err(|_| refusal_detail::cause("character-accept", "NameEncoding".to_owned()))?
    };
    let name = std::str::from_utf8(&encoded_name).map_err(|error| {
        refusal_detail::cause("character-accept", "NameEncoding".to_owned());
        bad(error)
    })?;
    let options = choices
        .iter()
        .filter(|(group, _)| {
            super::hero_identity::is_character_pack_validation_choice(group.as_str())
        })
        .map(|(group, option)| (group.as_str(), option.as_str()))
        .collect::<Vec<_>>();
    let mut build = pack::validate_character(name, &options).map_err(|error| {
        refusal_detail::cause("character-accept", "CatalogValidation".to_owned());
        bad(error)
    })?;
    if test_level > 1 {
        rules::FighterProgression::new(test_level)
            .and_then(|progression| progression.apply(&mut build))
            .map_err(|error| {
                refusal_detail::cause("character-accept", "Progression".to_owned());
                bad(error)
            })?;
    }
    if state
        .characters
        .iter()
        .any(|character| character.entity == who)
    {
        refusal_detail::cause("character-accept", "AlreadyCreated".to_owned());
        return Err(RepositoryError::InvalidCandidate);
    }
    let source = rule()?;
    let name_label = stored_name_label(name)?;
    let mut selected = choices
        .iter()
        .map(|(offer, selected)| AcceptedChoice {
            participant: member,
            offer: offer.clone(),
            selected: if offer.as_str() == "name" {
                name_label.clone()
            } else {
                selected.clone()
            },
            source: source.clone(),
        })
        .collect::<Vec<_>>();
    if test_level > 1 {
        selected.push(AcceptedChoice {
            participant: member,
            offer: model::label("test-fighter-level")?,
            selected: model::label("fighter-5")?,
            source: source.clone(),
        });
    }
    state.characters.push(CharacterState {
        entity: who,
        build: model::content("dwarf-fighter-soldier")?,
        owner: member,
        choices: selected.clone(),
    });
    if choices
        .iter()
        .any(|(group, _)| group.as_str().starts_with("identity-appearance:"))
    {
        let identity = super::hero_identity::ValidatedIdentity::from_canonical_appearance_choices(
            &choices
                .iter()
                .map(|(group, value)| (group.as_str().to_owned(), value.as_str().to_owned()))
                .collect::<Vec<_>>(),
        )
        .map_err(bad)?;
        let species = build
            .choices
            .iter()
            .find(|(group, _)| group == "species")
            .map(|(_, value)| value.as_str())
            .ok_or(RepositoryError::InvalidCandidate)?;
        let mut pack = identity.canonical_visual_pack(who, species).map_err(bad)?;
        if let Some((_, key)) = choices
            .iter()
            .find(|(field, _)| field.as_str() == "identity-portrait:key")
        {
            let field = |name: &str| {
                choices
                    .iter()
                    .find(|(field, _)| field.as_str() == name)
                    .map(|(_, value)| value.as_str())
                    .ok_or(RepositoryError::InvalidCandidate)
            };
            let digest: [u8; 32] = unhex(field("identity-portrait:digest")?)?
                .try_into()
                .map_err(bad)?;
            let bytes = field("identity-portrait:bytes")?
                .parse::<u64>()
                .map_err(bad)?;
            pack.identities[0]
                .appearances
                .push(df_model::checkpoint::AssetReference {
                    key: key.clone(),
                    digest: df_model::checkpoint::ContentDigest(digest),
                    byte_length: bytes,
                    kind: df_model::checkpoint::AssetKind::Image,
                });
            use sha2::{Digest, Sha256};
            let mut hash = Sha256::new();
            hash.update(pack.digest.0);
            hash.update(digest);
            hash.update(key.as_str().as_bytes());
            let bound: [u8; 32] = hash.finalize().into();
            let revision = model::label(&format!("approved-look-{}", hex(&bound[..12])))?;
            pack.digest = df_model::checkpoint::ContentDigest(bound);
            pack.revision = revision.clone();
            pack.bible.revision = revision.clone();
            pack.identities[0].revision = revision.clone();
            if let Some(appearance) = &mut pack.identities[0].character_appearance {
                appearance.outfit_revision = revision;
            }
        }
        state.continuity.canonical_packs.push(pack);
    }
    state.continuity.creation.push(CharacterDraft {
        entity: who,
        member,
        ancestry: Some(model::content("dwarf-fighter-soldier")?),
        background: Some(model::content("dwarf-fighter-soldier")?),
        classes: vec![model::content("dwarf-fighter-soldier")?],
        choices: selected,
        phase: CreationPhase::Accepted,
    });
    for (key, value) in [
        ("hit-points", build.hit_points),
        ("armor-class", build.armor_class),
        ("strength", u32::from(build.abilities[0])),
        ("dexterity", u32::from(build.abilities[1])),
        ("constitution", u32::from(build.abilities[2])),
        ("intelligence", u32::from(build.abilities[3])),
        ("wisdom", u32::from(build.abilities[4])),
        ("charisma", u32::from(build.abilities[5])),
        ("second-wind", build.second_wind_uses),
        ("hit-point-dice", u32::from(test_level)),
        ("short-rest-choice", 0),
        ("short-rest-die-roll", 0),
        ("unconscious", 0),
        ("prone", 0),
        ("held-weapon", 1),
        ("action-used", 0),
        ("bonus-used", 0),
    ] {
        add_resource(state, who, key, i64::from(value))?;
    }
    if test_level > 1 {
        add_resource(state, who, "action-surge", 1)?;
    }
    // The source-selected bundles are inventory, not cosmetic rewards or generated loot.
    let equipment = if build
        .choices
        .iter()
        .any(|(key, value)| key == "array" && value == "dexterous")
    {
        pack::archer_equipment().map_err(bad)?
    } else {
        pack::with_content(|pack| pack.equipment.clone())
    };
    let healer_kit_uses = if equipment
        .iter()
        .any(|row| row.item == "healers-kit" && row.quantity > 0)
    {
        10
    } else {
        0
    };
    let equipment_revision = model::label("source-starting-equipment-1")?;
    let equipment_source = rule()?;
    for row in equipment {
        let entry = row.item.as_str();
        let quantity = u64::from(row.quantity);
        let index = pack::COMPILED_ITEMS
            .iter()
            .position(|item| *item == entry)
            .ok_or(RepositoryError::InvalidCandidate)?;
        let mut id = *who.as_bytes();
        id[0] ^= 0x80;
        id[15] ^= (index as u8) + 1;
        state.entities.push(WorldEntity {
            id: entity(id)?,
            definition: model::content(entry)?,
            location: Some(who),
            position: None,
            identity_revision: equipment_revision.clone(),
        });
        state.inventory.push(InventoryItem {
            item: entity(id)?,
            owner: who,
            quantity,
            source: equipment_source.clone(),
            origin: model::content(entry)?,
            attunement_owner: None,
        });
    }
    add_resource(state, who, "healer-kit-uses", healer_kit_uses)?;
    Ok(())
}
/// The one entry into a fight, called by `stage_using` only after the
/// machine accepted `StartFight`. Both fight engines are entered here and
/// nowhere else in production: the tactical fight through
/// `tactical::start`, the source-qualified fight through `start_combat`.
/// The fight is left only inside its own resolution (`finish_if_done` /
/// `finish_combat`, the `FightMayEnd` executors), which the machine's
/// outcome check in `stage_using` verifies lands in `Aftermath`.
fn enter_fight(
    current: &Checkpoint,
    command: &CommandInput,
    selected: &super::beat_script::BeatOffer,
    state: &mut GameState,
    draws: &mut Vec<ActualDraw>,
    outcomes: &mut Vec<rpc::CombatOutcome>,
    supplier: DiceSource<'_>,
) -> Result<(), RepositoryError> {
    state
        .narrative
        .open_threads
        .push(model::content(THREAT_THREAD)?);
    if tactical::enabled() {
        let scene = super::beat_script_runtime::fight_scene(current, selected)?;
        tactical::start(current, command, state, draws, outcomes, supplier, &scene)
    } else {
        start_combat(current, command, state, draws, outcomes, supplier)
    }
}

/// The one entry into a conversation, called by `stage_using` only after
/// the machine accepted `StoryChoice` from `Story` (or within a
/// conversation). Returns whether the exchange is private to the asker.
/// A conversation is left only through the machine's own edges: a script
/// step along the loaded script's beats, or `StartFight` via
/// `enter_fight`.
fn enter_conversation(
    current: &Checkpoint,
    input: &GameInput,
    state: &mut GameState,
    kind: rpc::GameplayActionKind,
) -> Result<bool, RepositoryError> {
    state.narrative = narrative_transition::select(current, input)?
        .state()
        .narrative
        .clone();
    Ok(kind == rpc::GameplayActionKind::AskCourier)
}

fn observe_check(
    current: &Checkpoint,
    actor: EntityId,
    member: MemberId,
    choices: &[(df_types::RevisionLabel, df_types::RevisionLabel)],
    offered_target: &str,
    operation: OperationId,
    draws: &mut Vec<ActualDraw>,
    supplier: DiceSource<'_>,
) -> Result<rpc::AbilityCheckResult, RepositoryError> {
    if choices.iter().any(|(group, target)| {
        group.as_str() == "attempt-thing"
            && (target.as_str() == "room"
                || [
                    "thing-", "person-", "way-", "offer-", "pack-", "absent-", "finding-",
                    "surface-",
                ]
                .iter()
                .any(|prefix| target.as_str().starts_with(prefix)))
    }) {
        let plan = super::investigation::plan(current, member, choices, operation)?;
        let die = draw(operation, 20, draws, supplier)?;
        let resolved = super::beat_script_runtime::resolve_skill_check(
            current.state(),
            actor,
            &plan.skill,
            u8::try_from(die).map_err(|_| {
                refusal_detail::note("investigation-die-range");
                RepositoryError::InvalidCandidate
            })?,
            plan.difficulty_class,
        )?;
        let scope = super::beat_script_runtime::scene_scope(current.state())?;
        return Ok(rpc::AbilityCheckResult {
            die: u32::from(resolved.die),
            ability_modifier: i32::from(resolved.ability_modifier),
            proficiency_bonus: u32::from(resolved.proficiency_bonus),
            total: i32::from(resolved.total),
            difficulty_class: u32::from(resolved.difficulty_class),
            succeeded: resolved.succeeded,
            source_revision: format!("srd521-p6:{scope}:{}:{}", plan.skill, plan.detail()),
            faces: vec![die],
            roll_mode: 0,
        });
    }
    let selected_finding = choices
        .iter()
        .find(|(group, _)| group.as_str() == "finding")
        .map(|(_, option)| option.as_str());
    let skill_choice = choices
        .iter()
        .find(|(group, _)| group.as_str() == "attempt-skill")
        .map(|(_, option)| option.as_str());
    let thing_choice = choices
        .iter()
        .find(|(group, _)| group.as_str() == "attempt-thing")
        .map(|(_, option)| option.as_str());
    if choices
        .iter()
        .filter(|(group, _)| group.as_str() == "finding")
        .count()
        > 1
        || choices
            .iter()
            .filter(|(group, _)| group.as_str() == "attempt-skill")
            .count()
            > 1
        || choices
            .iter()
            .filter(|(group, _)| group.as_str() == "attempt-thing")
            .count()
            > 1
        || choices
            .iter()
            .filter(|(group, _)| group.as_str() == super::beat_script_runtime::CHOICE_GROUP)
            .count()
            > 1
        || choices.iter().any(|(group, _)| {
            !matches!(
                group.as_str(),
                "finding"
                    | "attempt-skill"
                    | "attempt-thing"
                    | super::beat_script_runtime::CHOICE_GROUP
            )
        })
    {
        refusal_detail::note("search-choice-shape");
        return Err(RepositoryError::InvalidCandidate);
    }
    let scene = super::beat_script_runtime::scene(current.state())?;
    let watch_person = selected_finding.and_then(|id| id.strip_prefix("watch-"));
    if watch_person.is_some_and(|id| !scene.cast.iter().any(|actor| actor.id == id)) {
        refusal_detail::note("search-watch-target");
        return Err(RepositoryError::InvalidCandidate);
    }
    let finding = selected_finding
        .filter(|id| watch_person.is_none() && *id != "look-around")
        .and_then(|id| scene.findings.iter().find(|finding| finding.id == id));
    if selected_finding.is_some()
        && watch_person.is_none()
        && selected_finding != Some("look-around")
        && finding.is_none()
    {
        refusal_detail::note("search-finding");
        return Err(RepositoryError::InvalidCandidate);
    }
    let finding_skill = |finding: &pack::Finding| match finding.skill {
        pack::FindingSkill::Perception => "perception",
        pack::FindingSkill::Investigation => "investigation",
        pack::FindingSkill::Insight => "insight",
        pack::FindingSkill::Survival => "survival",
        pack::FindingSkill::Medicine => "medicine",
        pack::FindingSkill::Arcana => "arcana",
        pack::FindingSkill::History => "history",
        pack::FindingSkill::Nature => "nature",
        pack::FindingSkill::Religion => "religion",
    };
    let matching_finding = finding.or_else(|| {
        if watch_person.is_some() {
            return None;
        }
        let requested = match skill_choice.unwrap_or("perception") {
            "perception" => pack::FindingSkill::Perception,
            "investigation" => pack::FindingSkill::Investigation,
            "insight" => pack::FindingSkill::Insight,
            "survival" => pack::FindingSkill::Survival,
            "medicine" => pack::FindingSkill::Medicine,
            "arcana" => pack::FindingSkill::Arcana,
            "history" => pack::FindingSkill::History,
            "nature" => pack::FindingSkill::Nature,
            "religion" => pack::FindingSkill::Religion,
            _ => return None,
        };
        scene
            .findings
            .iter()
            .find(|finding| !finding.id.starts_with("observation-") && finding.skill == requested)
    });
    let skill = match (skill_choice, matching_finding, watch_person) {
        (None, None, Some(_)) => "insight",
        (Some("insight"), None, Some(_)) => "insight",
        (Some(_), None, Some(_)) => {
            refusal_detail::note("search-watch-skill");
            return Err(RepositoryError::InvalidCandidate);
        }
        (Some(skill), Some(finding), None) if skill == finding_skill(finding) => skill,
        (_, Some(_), Some(_)) => {
            refusal_detail::note("search-watch-finding");
            return Err(RepositoryError::InvalidCandidate);
        }
        (Some(_), Some(_), None) => {
            refusal_detail::note("search-skill-finding");
            return Err(RepositoryError::InvalidCandidate);
        }
        (Some(skill), None, None) => skill,
        (None, Some(finding), None) => finding_skill(finding),
        (None, None, None) => "perception",
    };
    let scene_name = super::beat_script_runtime::scene_scope(current.state())?;
    let detail = if let Some(person) = watch_person {
        format!("watch-{person}")
    } else if let Some(finding) = finding {
        format!("finding-{}", finding.id)
    } else if let Some(thing) = thing_choice {
        thing.to_owned()
    } else {
        offered_target.to_owned()
    };
    for decision in &current.state().decisions {
        let belongs_to_member = super::beat_script_runtime::choice(current.state(), decision)?
            .is_some_and(|(_, _, found_member)| found_member == member);
        let same_target_check = belongs_to_member
            && accepted(decision)?.check.is_some_and(|check| {
                let parts = check.source_revision.splitn(4, ':').collect::<Vec<_>>();
                parts.len() == 4
                    && parts[0] == "srd521-p6"
                    && parts[1] == scene_name
                    && parts[2] == skill
                    && parts[3] == detail
            });
        if same_target_check {
            refusal_detail::note("search-already-tried");
            return Err(RepositoryError::InvalidCandidate);
        }
    }
    // SRD 5.2.1 p. 183: "you must succeed on a DC 15 Dexterity (Stealth) check" to Hide.
    // SRD 5.2.1 p. 6: the GM sets check DCs; the table lists "Easy 10" and "Medium 15."
    let difficulty_class = if skill == "stealth" {
        15
    } else {
        matching_finding.map_or(10, |finding| finding.dc)
    };
    let die = draw(operation, 20, draws, supplier)?;
    // SRD 5.2.1 p. 6: "Add these modifiers to the number rolled on the d20" and
    // "equals or exceeds the target number, the D20 Test succeeds."
    let resolved = super::beat_script_runtime::resolve_skill_check(
        current.state(),
        actor,
        skill,
        u8::try_from(die).map_err(|_| {
            refusal_detail::note("search-die-range");
            RepositoryError::InvalidCandidate
        })?,
        difficulty_class,
    )?;
    let source_revision = format!("srd521-p6:{scene_name}:{skill}:{detail}");
    Ok(rpc::AbilityCheckResult {
        die: u32::from(resolved.die),
        ability_modifier: i32::from(resolved.ability_modifier),
        proficiency_bonus: u32::from(resolved.proficiency_bonus),
        total: i32::from(resolved.total),
        difficulty_class: u32::from(resolved.difficulty_class),
        succeeded: resolved.succeeded,
        source_revision,
        faces: vec![u32::from(resolved.die)],
        roll_mode: 0,
    })
}

fn stage_using(
    current: &Checkpoint,
    input: &GameInput,
    supplier: DiceSource<'_>,
) -> Result<Checkpoint, RepositoryError> {
    if let Some(staged) = tactical::development_stage(current, input, supplier)? {
        return Ok(staged);
    }
    super::beat_script::validate_resume(current).map_err(|error| {
        if short_rest_started(current.state()) && !short_rest_completed(current.state()) {
            refusal_detail::note("short-rest-resume-validation");
        }
        bad(error)
    })?;
    df_model::commands::validate_client_command(
        input,
        current,
        ReferenceInventory {
            rules: &[model::rule()?, rule()?],
            content: &model::contents()?,
            resources: &resources_for(current.state())?,
            assets: &model::creator_portrait_assets(current.state())?,
        },
        df_model::commands::CommandLimits {
            maximum_records: if df_persistence::local_demo_scope::input_retained_bytes_max(input)
                > 8192
            {
                256
            } else {
                32
            },
            maximum_text_bytes: 128,
            maximum_retained_bytes: df_persistence::local_demo_scope::input_retained_bytes_max(
                input,
            ),
        },
    )
    .map_err(|error| {
        if short_rest_started(current.state()) && !short_rest_completed(current.state()) {
            refusal_detail::note("short-rest-command-validation");
        }
        bad(error)
    })?;
    let command = command(input).map_err(|error| {
        if short_rest_started(current.state()) && !short_rest_completed(current.state()) {
            refusal_detail::note("short-rest-command-decode");
        }
        error
    })?;
    if (tactical::present(current.state())
        || !super::beat_script::with_script(|script| script.is_canonical()))
        && (command.basis != current.basis()
            || command.observed_revision != current.basis().revision)
    {
        if short_rest_started(current.state()) && !short_rest_completed(current.state()) {
            refusal_detail::note("short-rest-stale-basis");
        }
        return Err(RepositoryError::InvalidCandidate);
    }
    let GameCommand::ProposeAction {
        actor: who,
        action,
        targets,
        choices: submitted_choices,
    } = &command.command
    else {
        if short_rest_started(current.state()) && !short_rest_completed(current.state()) {
            refusal_detail::note("short-rest-command-shape");
        }
        return Err(RepositoryError::InvalidCandidate);
    };
    if *who != player_entity(command.member, current)? {
        return Err(RepositoryError::Unauthorized);
    }
    let kind = wire::action_kind(action.entry.as_str()).ok_or_else(|| {
        if short_rest_started(current.state()) && !short_rest_completed(current.state()) {
            refusal_detail::note("short-rest-action-kind");
        }
        RepositoryError::InvalidCandidate
    })?;
    // Closing a chapter and starting another are seat acknowledgements, including after death.
    if !matches!(
        kind,
        rpc::GameplayActionKind::StartNewAdventure | rpc::GameplayActionKind::ResolveErrand
    ) && tactical::present(current.state())
        && tactical::hero_is_dead(current.state(), *who)?
    {
        refusal_detail::note("dead-hero-action");
        return Err(RepositoryError::InvalidCandidate);
    }
    if tactical::combat_action(kind)
        && (command.basis != current.basis()
            || command.observed_revision != current.basis().revision)
    {
        refusal_detail::note("combat-stale-basis");
        return Err(RepositoryError::InvalidCandidate);
    }
    if kind == rpc::GameplayActionKind::StartNewAdventure {
        let state = session_machine::SessionState::of(current)
            .inspect_err(|_| refusal_detail::note("new-adventure-machine-state"))?;
        if session_machine::next(state, session_machine::Event::NewAdventure).is_err()
            || !targets.is_empty()
            || !submitted_choices.is_empty()
        {
            refusal_detail::note("new-adventure-machine");
            return Err(RepositoryError::InvalidCandidate);
        }
        let mut basis = current.basis();
        basis.revision = basis.revision.next_sequence().map_err(|_| {
            refusal_detail::note("new-adventure-revision");
            RepositoryError::InvalidCandidate
        })?;
        let accepted = rpc::AcceptedAction {
            phase: rpc::JourneyPhase::Complete as i32,
            ..Default::default()
        };
        return finish(
            current,
            basis,
            current.state().clone(),
            command.operation,
            Vec::new(),
            DecisionContent {
                entry: "start-new-adventure",
                semantic: Some(hex(&accepted.encode_to_vec())),
                policy: THREAD_POLICY,
                audience: AudienceScope::Shared,
                reaction: None,
                escort_target: None,
                objective_command: None,
                script_choice: None,
            },
        );
    }
    // A scene assault starts a fight inside the freeplay scene; it is no beat offer, so the
    // machine and the offer list are consulted by `scene_fight` itself.
    if kind == rpc::GameplayActionKind::SceneAssault {
        return scene_fight::stage_assault(current, command, supplier);
    }
    if kind == rpc::GameplayActionKind::ExamineHarborSeal
        && submitted_choices
            .iter()
            .any(|(group, _)| group.as_str() == "scene-option")
    {
        let mut scene_draws = Vec::new();
        return living_scene::stage_with_supplier(current, command, &mut scene_draws, supplier);
    }
    let selected =
        super::beat_script_runtime::select(current, kind, submitted_choices).map_err(|error| {
            if kind == rpc::GameplayActionKind::ShortRest && short_rest_started(current.state()) {
                refusal_detail::note("short-rest-script-select");
            }
            error
        })?;
    // The forward state machine gates every command before any handler runs:
    // a command that is illegal in the session's current state is refused
    // here, whatever its handler might have allowed.
    let machine_state = session_machine::SessionState::of(current).map_err(|error| {
        if kind == rpc::GameplayActionKind::ShortRest && short_rest_started(current.state()) {
            refusal_detail::note("short-rest-machine-state");
        }
        error
    })?;
    let machine_event = session_machine::Event::of(selected.effect);
    let machine_transition = match session_machine::next(machine_state, machine_event) {
        Ok(transition) => transition,
        Err(_) => {
            refusal_detail::note("session-machine");
            return Err(RepositoryError::InvalidCandidate);
        }
    };
    // The machine also decides the Aftermath rest rule: once per
    // aftermath, and only while a living hero has something to regain.
    if machine_event == session_machine::Event::Rest
        && !session_machine::rest_accepted(current).map_err(|error| {
            if kind == rpc::GameplayActionKind::ShortRest && short_rest_started(current.state()) {
                refusal_detail::note("short-rest-machine-rule");
            }
            error
        })?
    {
        refusal_detail::note("session-machine-rest");
        return Err(RepositoryError::InvalidCandidate);
    }
    let original_kind = kind;
    let kind = if selected.effect == super::beat_script::BeatEffect::ThirdApproach {
        rpc::GameplayActionKind::EscortCourier
    } else {
        kind
    };
    // An authored resolution is the script's own pure step; no handler affordance backs it.
    if selected.effect != super::beat_script::BeatEffect::Resolve {
        affordance::validate(current, command.member, action, targets).map_err(|error| {
            if kind == rpc::GameplayActionKind::ShortRest && short_rest_started(current.state()) {
                refusal_detail::note("short-rest-offer-validation");
            }
            bad(error)
        })?;
    }
    // Target identity is validated against the current source-admitted binding above.
    // The registered handlers consume their existing authored, implicit target form;
    // keep that source contract intact after exact typed target confirmation.
    let mut source_input = input.clone();
    let GameInput::Game(source_command) = &mut source_input else {
        return Err(RepositoryError::InvalidCandidate);
    };
    let GameCommand::ProposeAction {
        targets,
        choices,
        action: source_action,
        ..
    } = &mut source_command.command
    else {
        return Err(RepositoryError::InvalidCandidate);
    };
    if !tactical::present(current.state()) {
        targets.clear();
    }
    choices.retain(|(group, _)| group.as_str() != super::beat_script_runtime::CHOICE_GROUP);
    if selected.effect == super::beat_script::BeatEffect::ThirdApproach {
        *source_action = model::content("escort-courier")?;
    }
    let input = &source_input;
    let command = self::command(input)?;
    let GameCommand::ProposeAction {
        actor: who,
        choices,
        ..
    } = &command.command
    else {
        return Err(RepositoryError::InvalidCandidate);
    };
    let mut basis = current.basis();
    basis.revision = basis.revision.next_sequence().map_err(bad)?;
    let mut state = current.state().clone();
    let mut draws = Vec::new();
    let mut outcomes = Vec::new();
    let mut private = false;
    let mut check = None;
    let pure_target = super::beat_script_runtime::pure_target(current, &selected)?;
    // Time passing for a hero at 0 hit points after the fight belongs to the story beat that
    // follows the fight, not to a choice of that beat's own offers: it records no script choice,
    // so the script replay finds the beat it was already on.
    let aftermath = tactical::present(current.state())
        && kind == rpc::GameplayActionKind::EndTurn
        && super::beat_script::with_script(|script| {
            script
                .current(current.state())
                .is_ok_and(|beat| beat.kind != super::beat_script::BeatKind::Fight)
        });
    // A step of a freeplay scene fight: no beat offer backs it and no script choice records it.
    let scene_fight_step = scene_fight::open(current.state());
    match kind {
        kind if tactical::combat_action(kind) && tactical::present(&state) => {
            tactical::stage(
                current,
                command,
                kind,
                &mut state,
                &mut draws,
                &mut outcomes,
                supplier,
            )?;
        }
        _ if pure_target.is_some() => {
            let target = pure_target
                .as_ref()
                .ok_or(RepositoryError::InvalidCandidate)?;
            // A fight does not end while a hero is still making death saves.
            if tactical::present(&state) && tactical::dying(&state)? {
                return Err(RepositoryError::InvalidCandidate);
            }
            if selected.effect == super::beat_script::BeatEffect::Observe {
                check = Some(observe_check(
                    current,
                    *who,
                    command.member,
                    choices,
                    &selected.id,
                    command.operation,
                    &mut draws,
                    supplier,
                )?);
            } else if selected.effect == super::beat_script::BeatEffect::ChooseHarborScene {
                if choices.as_slice()
                    != [(
                        model::label("harbor-destination")?,
                        model::label("harbor-inn")?,
                    )]
                {
                    return Err(RepositoryError::InvalidCandidate);
                }
            } else if !choices.is_empty() {
                return Err(RepositoryError::InvalidCandidate);
            }
            if state.narrative.active_beats.as_slice() != [model::content(&target.state.durable)?] {
                super::beat_script_runtime::advance_state(&mut state, &target.state.durable)?;
            }
        }
        rpc::GameplayActionKind::CreateCharacter => {
            accept_character(&mut state, *who, command.member, choices)?
        }
        rpc::GameplayActionKind::BeginStory => {
            if !choices.is_empty() {
                return Err(RepositoryError::InvalidCandidate);
            }
            state.narrative = narrative_transition::select(current, input)?
                .state()
                .narrative
                .clone();
            if super::beat_script::with_script(|script| {
                script.mode != super::beat_script::ScriptMode::Freeplay
            }) {
                courier_reaction::enter_opening(&mut state).inspect_err(|error| {
                    refusal_detail::cause("courier-enter-opening", format!("{error:?}"));
                })?;
            }
        }
        rpc::GameplayActionKind::AskCourier | rpc::GameplayActionKind::EscortCourier => {
            if !choices.is_empty() {
                return Err(RepositoryError::InvalidCandidate);
            }
            private = enter_conversation(current, input, &mut state, kind)?;
        }
        rpc::GameplayActionKind::DefendCourier => {
            let scene_attack = encounter_admission::scene_attack_target(current, command)?;
            if scene_attack.is_none() && !choices.is_empty() {
                refusal_detail::note("combat-choice-shape");
                return Err(RepositoryError::InvalidCandidate);
            }
            enter_fight(
                current,
                command,
                &selected,
                &mut state,
                &mut draws,
                &mut outcomes,
                supplier,
            )
            .inspect_err(|_| refusal_detail::note("combat-start"))?;
        }
        rpc::GameplayActionKind::GreatswordAttack => {
            combat_transition::stage(
                current,
                command,
                &mut state,
                &mut draws,
                &mut outcomes,
                supplier,
            )?;
        }
        rpc::GameplayActionKind::SecondWind => {
            if !choices.is_empty() {
                return Err(RepositoryError::InvalidCandidate);
            }
            let economy = rules::ActionEconomy {
                action_used: value(&state, *who, "action-used")? != 0,
                bonus_action_used: value(&state, *who, "bonus-used")? != 0,
            }
            .second_wind(value(&state, *who, "second-wind")?)
            .map_err(bad)?;
            let die = draw(
                command.operation,
                u32::from(pack::second_wind().die_sides),
                &mut draws,
                supplier,
            )?;
            let (hp, uses) = rules::heal_second_wind(
                &pack::second_wind(),
                die,
                value(&state, *who, "hit-points")?,
                maximum_hit_points(&state, *who)?,
                value(&state, *who, "second-wind")?,
            )
            .map_err(bad)?;
            set(&mut state, *who, "hit-points", hp)?;
            set(&mut state, *who, "second-wind", uses)?;
            set(
                &mut state,
                *who,
                "bonus-used",
                u32::from(economy.bonus_action_used),
            )?;
        }
        rpc::GameplayActionKind::ChooseHarborScene => {
            if choices.as_slice()
                != [(
                    model::label("harbor-destination")?,
                    model::label("harbor-inn")?,
                )]
                || inn_source(current)?.is_none()
                || value(&state, *who, "unconscious")? != 0
            {
                return Err(RepositoryError::InvalidCandidate);
            }
            beat(&mut state, "harbor-inn")?;
        }
        rpc::GameplayActionKind::ShortRest => match choices.as_slice() {
            [] if !short_rest_started(current.state()) => {
                start_short_rest(current, &mut state, command.operation)?;
            }
            [(group, option)] if group.as_str() == "hit-point-die" => {
                choose_short_rest_hit_point_die(
                    current,
                    &mut state,
                    *who,
                    option.as_str(),
                    command.operation,
                    &mut draws,
                    supplier,
                )
                .inspect_err(|_| refusal_detail::note("short-rest-choice-apply"))?;
            }
            _ => {
                refusal_detail::note("short-rest-choice-shape");
                return Err(RepositoryError::InvalidCandidate);
            }
        },
        rpc::GameplayActionKind::EndTurn => {
            if !choices.is_empty() {
                return Err(RepositoryError::InvalidCandidate);
            }
            advance(&mut state)?;
            enemy_turn(
                current,
                &mut state,
                command.operation,
                &mut draws,
                &mut outcomes,
                supplier,
            )?;
        }
        _ => return Err(RepositoryError::InvalidCandidate),
    }
    let result = rpc::AcceptedAction {
        check,
        scene: if kind == rpc::GameplayActionKind::ChooseHarborScene {
            if let Some(target) = &pure_target {
                Some(super::beat_script_runtime::destination_scene(target)?)
            } else {
                Some(inn_scene())
            }
        } else {
            None
        },
        character: if kind == rpc::GameplayActionKind::CreateCharacter {
            Some(receipt_character_sheet(&state, *who)?)
        } else {
            None
        },
        combat: outcomes,
        phase: phase_from_state(current, &state, &selected.id)
            .inspect_err(|_| refusal_detail::note("short-rest-phase"))? as i32,
    };
    let encoded = hex(&result.encode_to_vec());
    // Admission counts retained capacity, not text length. Hex collection can retain an
    // 8 KiB buffer for a 4 KiB Archer sheet; creation must not keep that allocation slack.
    let semantic = Some(if kind == rpc::GameplayActionKind::CreateCharacter {
        encoded.into_boxed_str().into_string()
    } else {
        encoded
    });
    // The recorded receipt is the largest text a checkpoint holds; refuse here,
    // by name, rather than as an opaque checkpoint failure in `finish`.
    if semantic
        .as_ref()
        .is_some_and(|text| text.len() > model::limits().maximum_text_bytes)
    {
        refusal_detail::note("receipt-too-large");
        return Err(RepositoryError::Capacity);
    }
    let scene_attack = encounter_admission::scene_attack_target(current, &command)?.is_some();
    let entry = if pure_target.is_some() {
        super::beat_script_runtime::STEP
    } else if selected.effect == super::beat_script::BeatEffect::ThirdApproach {
        "escort-courier"
    } else if private {
        "private-courier-note"
    } else if scene_attack {
        "scene-npc-assault"
    } else if kind == rpc::GameplayActionKind::ChooseHarborScene {
        "harbor-inn"
    } else {
        action.entry.as_str()
    };
    let reaction = if kind == rpc::GameplayActionKind::DefendCourier && !scene_attack {
        courier_reaction::propose_on_defend(current, *who)?
    } else {
        None
    };
    let candidate = finish(
        current,
        basis,
        state,
        command.operation,
        draws,
        DecisionContent {
            entry,
            semantic,
            policy: THREAD_POLICY,
            audience: if private {
                AudienceScope::Members(vec![command.member])
            } else {
                AudienceScope::Shared
            },
            reaction,
            escort_target: (kind == rpc::GameplayActionKind::EscortCourier).then_some(*who),
            objective_command: Some(command),
            script_choice: if (super::beat_script::with_script(|script| script.is_canonical())
                && selected.effect != super::beat_script::BeatEffect::Observe)
                || original_kind == rpc::GameplayActionKind::CreateCharacter
                || aftermath
                || scene_fight_step
            {
                None
            } else {
                Some((super::beat_script_runtime::marker(&selected.id)?, *who))
            },
        },
    )
    .inspect_err(|_| {
        if kind == rpc::GameplayActionKind::ShortRest {
            refusal_detail::note("short-rest-candidate-finish");
        }
    })?;
    // The candidate must land where the machine's transition said: no
    // executor may move the session anywhere else, or backwards.
    let landed = session_machine::SessionState::of(&candidate)?;
    if !(if scene_fight_step {
        session_machine::scene_fight_outcome_legal(machine_state, landed)
    } else {
        session_machine::outcome_legal(machine_state, machine_event, machine_transition, landed)
    }) {
        refusal_detail::note("session-machine-outcome");
        return Err(RepositoryError::InvalidCandidate);
    }
    // A freeplay fight step stays on the freeplay beat, which no beat transition describes.
    if !scene_fight_step {
        super::beat_script::validate_step(current, &candidate, original_kind, &selected.id)
            .inspect_err(|error| {
                refusal_detail::cause("script-validate-step", format!("{error:?}"))
            })?;
    }
    if private {
        super::courier_ai::append_request(candidate, command.operation)
    } else {
        Ok(candidate)
    }
}
/// Qualify the earned native prefix independently of subsequent nonmechanical script
/// scenes. This detached validation projection never changes the durable checkpoint.
pub(super) fn validate_script_native_prefix(
    current: &Checkpoint,
    durable: &str,
) -> Result<(), RepositoryError> {
    let mut state = current.state().clone();
    state.narrative.active_beats = vec![model::content(durable)?];
    state.narrative.completed_beats = match durable {
        "room" => Vec::new(),
        "opening" => vec![model::content("room")?],
        "courier-answer-seal" | "courier-answer-escort" => {
            vec![model::content("room")?, model::content("opening")?]
        }
        _ => return Err(RepositoryError::InvalidCandidate),
    };
    let prefix = model::checkpoint(current.basis(), state)?;
    narrative_phase::validate(&prefix).map(|_| ())
}
/// Reuse character admission to verify a no-encounter script has preserved all source
/// choices, abilities, resources, equipment identities and creation provenance.
pub(super) fn validate_script_characters(state: &GameState) -> Result<(), RepositoryError> {
    // A freeplay scene that has seen a fight also holds the fought people's resources, and its
    // heroes' combat resources and weapon counts are no longer their creation values.
    let fought = scene_fight::fought(state);
    if state.characters.len() > MAXIMUM_PARTY
        || state.resources.iter().any(|r| {
            !state.characters.iter().any(|c| c.entity == r.owner)
                && !(fought && scene_fight::is_person(state, r.owner))
        })
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let mut expected = recreated_heroes(state)?;
    living_scene::replay_starting_custody(state, &mut expected)?;
    let actual_items = state
        .entities
        .iter()
        .filter(|entity| pack::COMPILED_ITEMS.contains(&entity.definition.entry.as_str()))
        .collect::<Vec<_>>();
    let expected_items = expected
        .entities
        .iter()
        .filter(|entity| pack::COMPILED_ITEMS.contains(&entity.definition.entry.as_str()))
        .collect::<Vec<_>>();
    let (resources_agree, inventory_agrees) = if fought {
        let heroes = state
            .resources
            .iter()
            .filter(|r| state.characters.iter().any(|c| c.entity == r.owner))
            .cloned()
            .collect::<Vec<_>>();
        (
            scene_fight::resources_match(&expected.resources, &heroes),
            scene_fight::inventory_matches(&expected.inventory, &state.inventory),
        )
    } else {
        (
            expected.resources == state.resources,
            expected.inventory == state.inventory,
        )
    };
    if expected.characters != state.characters
        || !resources_agree
        || !inventory_agrees
        || expected.continuity.creation != state.continuity.creation
        || actual_items != expected_items
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    Ok(())
}

/// The state as character creation left it: `state` with every accepted character created again
/// from its recorded choices, so each hero has the creation resources, kit and sheet. Refuses a
/// character whose record does not belong to its member or its build. The replay of a creation
/// receipt reads the sheet from this, because a freeplay fight changes a hero's combat resources.
pub(super) fn recreated_heroes(state: &GameState) -> Result<GameState, RepositoryError> {
    let pack = pack::cache_identity();
    let remembered = RECREATED.with(|cache| {
        cache
            .borrow()
            .iter()
            .find(|(input, source, _)| *source == pack && input == state)
            .map(|(_, _, recreated)| recreated.clone())
    });
    if let Some(recreated) = remembered {
        return Ok(recreated);
    }
    let recreated = derived_heroes(state)?;
    RECREATED.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= REMEMBERED_RECREATIONS {
            cache.pop_front();
        }
        cache.push_back((state.clone(), pack, recreated.clone()));
    });
    Ok(recreated)
}

fn derived_heroes(state: &GameState) -> Result<GameState, RepositoryError> {
    let source = rule()?;
    let bootstrap = bootstrap_member()?;
    let mut expected = state.clone();
    expected.characters.clear();
    expected.resources.clear();
    expected.inventory.clear();
    expected.continuity.creation.clear();
    expected
        .entities
        .retain(|entity| !pack::COMPILED_ITEMS.contains(&entity.definition.entry.as_str()));
    for character in &state.characters {
        if character.build != model::content("dwarf-fighter-soldier")?
            || character.owner == bootstrap
            || !state.members.iter().any(|link| {
                link.member == character.owner && link.character == Some(character.entity)
            })
            || character
                .choices
                .iter()
                .any(|choice| choice.participant != character.owner || choice.source != source)
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        let choices = character
            .choices
            .iter()
            .filter(|choice| choice.offer.as_str() != "test-fighter-level")
            .map(|choice| {
                let value = if choice.offer.as_str() == "name" {
                    model::label(&hex(stored_name(choice.selected.as_str())?.as_bytes()))?
                } else {
                    choice.selected.clone()
                };
                Ok((choice.offer.clone(), value))
            })
            .collect::<Result<Vec<_>, RepositoryError>>()?;
        accept_character_at_level(
            &mut expected,
            character.entity,
            character.owner,
            &choices,
            fighter_progression(state, character.entity)?.level(),
        )?;
    }
    Ok(expected)
}

fn phase_from_state(
    before: &Checkpoint,
    state: &GameState,
    selected_id: &str,
) -> Result<rpc::JourneyPhase, RepositoryError> {
    // The selected transition is validated at load and admission. Its receipt is
    // encoded before the new choice fact exists; finish validates the owned fact.
    super::beat_script::with_script(|script| {
        if script.uses_encounter() {
            narrative_phase::classify(state)
        } else {
            let beat = script.current(before.state())?;
            // A step of a freeplay scene fight never leaves the freeplay beat: the receipt says
            // Combat while the encounter is open and the beat's own phase once it has closed.
            if scene_fight::open(before.state()) {
                return Ok(scene_fight::phase_after(
                    state,
                    super::beat_script_runtime::phase_of(beat),
                ));
            }
            // After a won fight a dying hero's timed Death Saving Throws advance the clock on
            // the beat that follows the fight. The offer that does it belongs to the fight
            // beat, so the following beat has no transition for it and the story stays put.
            if tactical::present(before.state())
                && beat.kind != super::beat_script::BeatKind::Fight
                && tactical::dying(before.state())?
                && script.beats.iter().any(|fight| {
                    fight.kind == super::beat_script::BeatKind::Fight
                        && fight.offers.iter().any(|offer| {
                            offer.id == selected_id
                                && offer.effect.action() == rpc::GameplayActionKind::EndTurn
                        })
                })
            {
                return Ok(super::beat_script_runtime::phase_of(beat));
            }
            let trigger = super::beat_script::BeatTrigger::Offer {
                offer: selected_id.into(),
            };
            beat.transitions
                .iter()
                .find(|transition| transition.trigger == trigger)
                .and_then(|transition| script.find(&transition.target))
                .map(super::beat_script_runtime::phase_of)
                .ok_or(RepositoryError::InvalidCandidate)
        }
    })
}

struct JourneyHandler {
    pins: CheckpointPins,
}
impl df_rules::RulesCommandHandler for JourneyHandler {
    type Rejection = RepositoryError;
    fn pins(&self) -> &CheckpointPins {
        &self.pins
    }
    fn stage(
        &self,
        input: df_rules::RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<Checkpoint, RepositoryError> {
        let operation = command(input.command)?.operation;
        let mut supplied = input.supplied_draws.iter();
        let mut ordinal = 0;
        let mut consume = |sides| {
            let draw = supplied.next().ok_or_else(|| {
                refusal_detail::note("handler-missing-draw");
                RepositoryError::InvalidCandidate
            })?;
            if draw.operation != operation
                || draw.ordinal != ordinal
                || draw.sides != sides
                || draw.source != rule()?
                || !(1..=sides).contains(&draw.value)
            {
                refusal_detail::note("handler-draw-mismatch");
                return Err(RepositoryError::InvalidCandidate);
            }
            ordinal += 1;
            Ok(draw.value)
        };
        let staged = stage_using(current, input.command, &mut consume)
            .inspect_err(|_| refusal_detail::note("handler-stage-replay"))?;
        if supplied.next().is_some() {
            refusal_detail::note("handler-unused-draw");
            return Err(RepositoryError::InvalidCandidate);
        }
        Ok(staged)
    }
}

struct PreparedJourneyHandler<'a> {
    replay: &'a JourneyHandler,
    current: &'a Checkpoint,
    command: &'a GameInput,
    draws: &'a [ActualDraw],
    prepared: &'a Checkpoint,
}
impl df_rules::RulesCommandHandler for PreparedJourneyHandler<'_> {
    type Rejection = RepositoryError;
    fn pins(&self) -> &CheckpointPins {
        &self.replay.pins
    }
    fn stage(
        &self,
        input: df_rules::RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<Checkpoint, RepositoryError> {
        // This private handler exists for exactly one discovery result. Reuse it only for
        // those exact inputs; registry admission and candidate/draw/history checks still run.
        if current != self.current
            || input.command != self.command
            || input.supplied_draws != self.draws
        {
            refusal_detail::note("handler-discovery-mismatch");
            return Err(RepositoryError::InvalidCandidate);
        }
        Ok(self.prepared.clone())
    }
}
/// Native sampling is separate from the registered pure transition. Every actual
/// source-qualified draw is supplied explicitly; retries stop at the PG operation lookup.
pub(super) fn stage(
    current: &Checkpoint,
    input: &GameInput,
) -> Result<Checkpoint, RepositoryError> {
    stage_with_supplier(current, input, &mut sample_face)
}

pub(super) fn stage_with_supplier(
    current: &Checkpoint,
    input: &GameInput,
    supplier: DiceSource<'_>,
) -> Result<Checkpoint, RepositoryError> {
    let started = std::time::Instant::now();
    let prepared = stage_using(current, input, supplier).inspect_err(|_| {
        refusal_detail::note("stage-using");
    })?;
    let prepared_us = started.elapsed().as_micros();
    let operation = command(input)?.operation;
    let draws = prepared
        .state()
        .draws
        .iter()
        .filter(|draw| draw.operation == operation)
        .cloned()
        .collect::<Vec<_>>();
    let pins = model::pins()?;
    let source = rule()?;
    let selector = model::label("local-journey-handler-1")?;
    let manifest = model::source_manifest();
    let entries = [df_content::catalog::CatalogEntry::new(&source, &manifest)];
    let catalog = df_content::catalog::CatalogSnapshot::from_published(
        &pins.rules.catalog,
        &pins,
        &manifest,
        &entries,
        df_content::catalog::CatalogLimits {
            max_complete_bytes: 4096,
            max_entries: 1,
            max_item_bytes: 4096,
            max_total_item_bytes: 4096,
        },
    )
    .map_err(bad)?;
    let base_handler = JourneyHandler { pins: pins.clone() };
    let prepared_handler = PreparedJourneyHandler {
        replay: &base_handler,
        current,
        command: input,
        draws: &draws,
        prepared: &prepared,
    };
    let handler = courier_reaction::CourierJourneyHandler {
        command_handler: &prepared_handler,
    };
    let registrations = [df_rules::HandlerRegistration::new(
        &selector, &source, &handler,
    )];
    let registry =
        df_rules::DispatchRegistry::from_catalog(catalog, &registrations, 1).map_err(bad)?;
    let dispatched = std::time::Instant::now();
    let result = df_engine::command_entry::decide_registered_command(
        df_rules::RulesCommandInput {
            command: input,
            supplied_draws: &draws,
        },
        current,
        df_engine::command_entry::CommandEntryContext {
            current_basis: current.basis(),
            admitted_pins: &pins,
            inventory: ReferenceInventory {
                rules: &[model::rule()?, source.clone()],
                content: &model::contents()?,
                resources: &resources_for(prepared.state())?,
                assets: &model::creator_portrait_assets(prepared.state())?,
            },
            limits: df_engine::command_entry::CommandEntryLimits {
                command: df_model::commands::CommandLimits {
                    maximum_records: if df_persistence::local_demo_scope::input_retained_bytes_max(
                        input,
                    ) > 8192
                    {
                        256
                    } else {
                        32
                    },
                    maximum_text_bytes: 128,
                    maximum_retained_bytes:
                        df_persistence::local_demo_scope::input_retained_bytes_max(input),
                },
                maximum_staged_bytes: model::limits().maximum_retained_bytes,
            },
        },
        &registry,
        &selector,
        &source,
    )
    .map_err(|error| {
        if tactical::development_mode(current.state()) {
            super::actor::record_audit_line(
                "gameplay.dev.action_refused",
                format!("DEV-BATTLE registered-action refused error={error:?}"),
            );
        }
        refusal_detail::note("decide-command");
        bad(error)
    });
    super::actor::record_audit_line(
        "gameplay.stage_timing",
        format!(
            "STAGE operation={} prepare_us={prepared_us} dispatch_us={} total_us={} outcome={}",
            hex(operation.as_bytes()),
            dispatched.elapsed().as_micros(),
            started.elapsed().as_micros(),
            if result.is_ok() { "staged" } else { "refused" }
        ),
    );
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn input(
        current: &Checkpoint,
        member: MemberId,
        operation: u8,
        entry: &str,
        choices: Vec<(df_types::RevisionLabel, df_types::RevisionLabel)>,
    ) -> GameInput {
        GameInput::Game(CommandInput {
            basis: current.basis(),
            operation: OperationId::from_bytes(&[operation; 16]).expect("operation"),
            member,
            observed_revision: current.basis().revision,
            command: GameCommand::ProposeAction {
                actor: if member == bootstrap_member().expect("bootstrap") {
                    room_entity().expect("room")
                } else {
                    player_entity(member, current).expect("joined")
                },
                action: model::content(entry).expect("admitted content"),
                targets: vec![],
                choices,
            },
        })
    }
    pub(super) fn build(name: &str) -> Vec<(df_types::RevisionLabel, df_types::RevisionLabel)> {
        [
            ("name", hex(name.as_bytes())),
            ("species", "dwarf".to_owned()),
            ("class", "fighter-1".to_owned()),
            ("background", "soldier".to_owned()),
            ("array", "stalwart".to_owned()),
            ("alignment", "lawful-good".to_owned()),
            ("language-1", "dwarvish".to_owned()),
            ("language-2", "elvish".to_owned()),
            ("skill-1", "perception".to_owned()),
            ("skill-2", "survival".to_owned()),
            ("gaming-set", "dice".to_owned()),
            ("equipment", "fighter-a-soldier-a".to_owned()),
            (
                "style-masteries",
                "defense-greatsword-flail-javelin".to_owned(),
            ),
            ("allied-ties", "join-order".to_owned()),
        ]
        .into_iter()
        .map(|(key, value)| {
            (
                model::label(key).expect("key"),
                model::label(&value).expect("choice"),
            )
        })
        .collect()
    }
    fn finish_rest_keeping_dice(mut current: Checkpoint) -> Checkpoint {
        // b44a9198 made an eligible hero's spend-or-keep decision part of completing a rest.
        for (index, bytes) in MEMBERS.into_iter().take(2).enumerate() {
            let member = MemberId::from_bytes(&bytes).unwrap();
            if hit_point_die_offered(current.state(), player_entity(member, &current).unwrap())
                .unwrap()
            {
                current = stage_with_supplier(
                    &current,
                    &input(
                        &current,
                        member,
                        210 + index as u8,
                        "short-rest",
                        vec![(
                            model::label("hit-point-die").unwrap(),
                            model::label("keep").unwrap(),
                        )],
                    ),
                    &mut |_| panic!("keeping a Hit Point Die cannot draw"),
                )
                .unwrap();
            }
        }
        assert!(short_rest_completed(current.state()));
        current
    }
    pub(super) fn tuning_journey() -> Vec<Checkpoint> {
        if tactical::enabled() {
            let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
            // The ordinary played fight leaves both heroes untouched, and
            // the machine refuses a rest nobody gains from, so this journey
            // rests after a victory where a hero spent Second Wind.
            let victory = tactical::tests::current_victory_with_spent_second_wind("escort-courier");
            let rested = stage_with_supplier(
                &victory,
                &input(&victory, first, 201, "short-rest", vec![]),
                &mut |_| panic!("rest dice"),
            )
            .unwrap();
            let rested = finish_rest_keeping_dice(rested);
            let arrived =
                stage_with_supplier(&rested, &inn_command(&rested, first, 202), &mut |_| {
                    panic!("inn dice")
                })
                .unwrap();
            return vec![victory, rested, arrived];
        }
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        let mut checkpoints = vec![opening_story()];
        let mut apply =
            |member,
             operation,
             action: &str,
             choices,
             faces: &mut dyn FnMut(u32) -> Result<u32, RepositoryError>| {
                let current = checkpoints.last().unwrap();
                checkpoints.push(
                    stage_with_supplier(
                        current,
                        &input(current, member, operation, action, choices),
                        faces,
                    )
                    .unwrap_or_else(|error| {
                        panic!("{action}: {error:?}; step={:?}", refusal_detail::take())
                    }),
                );
            };
        apply(first, 6, "escort-courier", vec![], &mut |_| {
            panic!("dialogue dice")
        });
        let mut initiative = [20, 10, 1].into_iter();
        apply(first, 7, "defend-courier", vec![], &mut |_| {
            Ok(initiative.next().unwrap())
        });
        apply(first, 8, "end-turn", vec![], &mut |_| {
            panic!("first end dice")
        });
        apply(second, 9, "end-turn", vec![], &mut |sides| {
            Ok(if sides == 20 { 10 } else { sides })
        });
        apply(first, 10, "second-wind", vec![], &mut |sides| Ok(sides));
        apply(
            first,
            11,
            "greatsword-attack",
            vec![
                (
                    model::label("savage-attacker").unwrap(),
                    model::label("no").unwrap(),
                ),
                (model::label("graze").unwrap(), model::label("no").unwrap()),
            ],
            &mut |sides| {
                Ok(if sides == 20 {
                    u32::from(pack::combat().critical_threshold)
                } else {
                    sides
                })
            },
        );
        apply(first, 12, "short-rest", vec![], &mut |_| {
            panic!("rest dice")
        });
        let rest = checkpoints.pop().unwrap();
        checkpoints.push(finish_rest_keeping_dice(rest));
        let current = checkpoints.last().unwrap();
        checkpoints.push(
            stage_with_supplier(current, &inn_command(current, first, 13), &mut |_| {
                panic!("inn dice")
            })
            .unwrap(),
        );
        checkpoints
    }

    #[test]
    fn canonical_end_turn_from_prior_round_is_refused_without_advancing() {
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        let opening = opening_story();
        let dialogue = stage_with_supplier(
            &opening,
            &input(&opening, first, 6, "ask-courier", vec![]),
            &mut |_| panic!("dialogue dice"),
        )
        .unwrap();
        let mut initiative = [20, 10, 1].into_iter();
        let combat = stage_with_supplier(
            &dialogue,
            &input(&dialogue, first, 7, "defend-courier", vec![]),
            &mut |_| Ok(initiative.next().unwrap()),
        )
        .unwrap();
        let stale_end_turn = input(&combat, first, 8, "end-turn", vec![]);
        let after_first_turn = stage_with_supplier(
            &combat,
            &input(&combat, first, 9, "end-turn", vec![]),
            &mut |_| panic!("first hero turn cannot draw"),
        )
        .unwrap();
        let after_round = stage_with_supplier(
            &after_first_turn,
            &input(&after_first_turn, second, 10, "end-turn", vec![]),
            &mut |_| Ok(1),
        )
        .unwrap();
        let active_hero = after_round.state().encounters[0].active_turn;
        let draw_count = after_round.state().draws.len();

        assert_eq!(active_hero, Some(entity(ENTITIES[0]).unwrap()));
        assert_eq!(
            stage_with_supplier(&after_round, &stale_end_turn, &mut |_| {
                panic!("stale end-turn must not draw")
            }),
            Err(RepositoryError::InvalidCandidate)
        );
        assert_eq!(after_round.state().encounters[0].active_turn, active_hero);
        assert_eq!(after_round.state().draws.len(), draw_count);
    }

    #[test]
    fn every_generated_creature_plays_the_integrated_melee_rest_and_destination() {
        use crate::gameplay::adventure::{
            ForcedEntries, GenerateOptions, RulesMode, Tables, generate,
        };
        let tables = Tables::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../content/adventure-tables")
                .as_path(),
        )
        .unwrap();
        // Only creatures the antagonist table can roll: the CR 2-5 rows exist for level-5 test
        // fights (`dev fight start --foe`) and are not adventure antagonists.
        let table: serde_json::Value = serde_json::from_slice(
            &std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../content/adventure-tables/antagonists.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let rollable: Vec<&str> = table["tables"]["antagonists"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["id"].as_str())
            .collect();
        let creatures: Vec<_> = df_rules::SRD_521_CREATURES
            .iter()
            .filter(|creature| rollable.contains(&creature.id))
            .collect();
        assert!(creatures.len() >= 12, "antagonist table rolls {rollable:?}");
        for creature in creatures {
            let location = if ["skeleton", "wolf"].contains(&creature.id) {
                "pinewood-trail"
            } else {
                "salt-quay"
            };
            let mut generated = generate(
                &tables,
                42,
                GenerateOptions {
                    rules_mode: RulesMode::CustomAllowed,
                    ..GenerateOptions::default()
                },
                &ForcedEntries::from([
                    ("location".into(), location.into()),
                    ("antagonist".into(), creature.id.into()),
                ]),
            )
            .unwrap();
            // This test retains the archived single-melee profile and its knockout/rest contract.
            generated.tuning.tactical = None;
            let loaded = pack::load_from_bytes(
                &serde_json::to_vec(&generated.pack).unwrap(),
                &serde_json::to_vec(&generated.tuning).unwrap(),
            )
            .unwrap();
            pack::with_test_pack(loaded, || {
                let checkpoints = tuning_journey();
                let combat = &checkpoints[2];
                let enemy = entity(BANDIT).unwrap();
                assert_eq!(
                    value(combat.state(), enemy, "hit-points").unwrap(),
                    u32::from(creature.rules.hit_points)
                );
                assert_eq!(
                    value(combat.state(), enemy, "armor-class").unwrap(),
                    u32::from(creature.rules.armor_class)
                );
                assert_eq!(
                    name(combat.state(), enemy).unwrap(),
                    generated.pack.bandit.name
                );
                let final_state = checkpoints.last().unwrap();
                assert_eq!(
                    inn_scene().speaker_portrait,
                    generated
                        .pack
                        .scenes
                        .iter()
                        .find(|s| s.key == pack::SceneKey::Inn)
                        .unwrap()
                        .speaker_portrait
                );
                assert!(
                    final_state
                        .state()
                        .narrative
                        .active_beats
                        .contains(&model::content("harbor-inn").unwrap())
                );
                assert_eq!(pack::bandit_weapon_item(), "scimitar");
            });
        }
    }

    #[test]
    fn live_tempo_follows_real_journey_and_projects_identical_public_cues() {
        use crate::gameplay::actor::tempo::TempoOwner;
        use df_persistence::local_demo_scope::{DISPLAY, LocalDemoRole};
        use std::time::{Duration, Instant};
        let origin = Instant::now();
        let room = initial().unwrap();
        let mut tempo = TempoOwner::start(&room, pack::TempoTuning::default(), origin).unwrap();
        let checkpoints = tuning_journey();
        for (index, checkpoint) in checkpoints.iter().enumerate() {
            let before = checkpoint.clone();
            let accepted = accepted(checkpoint.state().decisions.last().unwrap()).unwrap();
            let kind = if index == 7 {
                rpc::GameplayActionKind::ShortRest
            } else if index == 8 {
                rpc::GameplayActionKind::ChooseHarborScene
            } else {
                rpc::GameplayActionKind::Unspecified
            };
            let now = origin + Duration::from_secs((index as u64 + 1) * 8);
            tempo
                .accepted(checkpoint, &accepted, kind as i32, now)
                .unwrap();
            let mut display = wire::journey_view(
                checkpoint,
                LocalDemoRole::Display,
                MemberId::from_bytes(&DISPLAY).unwrap(),
            )
            .unwrap();
            let mut player = wire::journey_view(
                checkpoint,
                LocalDemoRole::Player,
                MemberId::from_bytes(&MEMBERS[0]).unwrap(),
            )
            .unwrap();
            tempo.decorate(&mut display, checkpoint, now).unwrap();
            tempo.decorate(&mut player, checkpoint, now).unwrap();
            let rpc::view_message::Audience::Display(display) = display.audience.unwrap() else {
                panic!("display")
            };
            let rpc::view_message::Audience::Player(player) = player.audience.unwrap() else {
                panic!("player")
            };
            assert_eq!(
                display.journey.unwrap().tempo,
                player.journey.unwrap().tempo
            );
            assert_eq!(
                checkpoint, &before,
                "tempo leaves all mechanical state and pins unchanged"
            );
        }
        let rest = &checkpoints[7];
        let mut resumed_rest =
            TempoOwner::start(rest, pack::TempoTuning::default(), origin).unwrap();
        let mut rested_view = wire::journey_view(
            rest,
            LocalDemoRole::Display,
            MemberId::from_bytes(&DISPLAY).unwrap(),
        )
        .unwrap();
        resumed_rest
            .decorate(&mut rested_view, rest, origin)
            .unwrap();
        let rpc::view_message::Audience::Display(rested_view) = rested_view.audience.unwrap()
        else {
            panic!("display")
        };
        assert_eq!(
            rested_view
                .journey
                .unwrap()
                .tempo
                .unwrap()
                .intensity_permille,
            80
        );
        let mut authored: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../content/harbor-journey/pack.json"
        ))
        .unwrap();
        for scene in authored["scenes"].as_array_mut().unwrap() {
            if scene["key"] == "victory" {
                scene["narration"] = serde_json::json!("The fight is over.");
            }
            if scene["key"] == "defeat" {
                scene["narration"] = serde_json::json!("The fight is over. Your party falls.");
            }
            if scene["key"] == "victory" || scene["key"] == "defeat" {
                scene["narration_rested"] = serde_json::json!("The fight is over.");
            }
        }
        let loaded = pack::load_from_bytes(
            &serde_json::to_vec(&authored).unwrap(),
            include_bytes!("fixtures/harbor-journey-tuning-baseline.json"),
        )
        .unwrap();
        pack::with_test_pack(loaded, || {
            let edited = tuning_journey();
            let mut resumed =
                TempoOwner::start(&edited[7], pack::TempoTuning::default(), origin).unwrap();
            let mut view = wire::journey_view(
                &edited[7],
                LocalDemoRole::Display,
                MemberId::from_bytes(&DISPLAY).unwrap(),
            )
            .unwrap();
            resumed.decorate(&mut view, &edited[7], origin).unwrap();
            let rpc::view_message::Audience::Display(view) = view.audience.unwrap() else {
                panic!("display")
            };
            assert_eq!(
                view.journey.unwrap().tempo.unwrap().intensity_permille,
                80,
                "public typed rest survives identical and overlapping prose"
            );
        });
        let opening = &checkpoints[0];
        let victory = &checkpoints[6];
        let before_recovery = victory.clone();
        let mut recovered =
            TempoOwner::start(opening, pack::TempoTuning::default(), origin).unwrap();
        let mut recovered_view = wire::journey_view(
            victory,
            LocalDemoRole::Display,
            MemberId::from_bytes(&DISPLAY).unwrap(),
        )
        .unwrap();
        let recovery_now = origin + Duration::from_secs(10);
        recovered
            .decorate(&mut recovered_view, victory, recovery_now)
            .unwrap();
        let rpc::view_message::Audience::Display(recovered_public) =
            recovered_view.audience.as_ref().unwrap()
        else {
            panic!("display")
        };
        let recovered_cue = recovered_public.journey.as_ref().unwrap().tempo.unwrap();
        assert_eq!(recovered_cue.intensity_permille, 200);
        assert_eq!(recovered_cue.phase, rpc::TempoPhase::Release as i32);
        assert!(recovered_cue.sequence > 1);
        recovered
            .decorate(&mut recovered_view, victory, recovery_now)
            .unwrap();
        let rpc::view_message::Audience::Display(repeated_public) =
            recovered_view.audience.unwrap()
        else {
            panic!("display")
        };
        assert_eq!(
            repeated_public.journey.unwrap().tempo.unwrap(),
            recovered_cue
        );
        assert_eq!(victory, &before_recovery);
        let mut combat_recovery =
            TempoOwner::start(&checkpoints[2], pack::TempoTuning::default(), origin).unwrap();
        let mut rest_view = wire::journey_view(
            rest,
            LocalDemoRole::Display,
            MemberId::from_bytes(&DISPLAY).unwrap(),
        )
        .unwrap();
        combat_recovery
            .decorate(&mut rest_view, rest, recovery_now)
            .unwrap();
        let rpc::view_message::Audience::Display(rest_public) = rest_view.audience.unwrap() else {
            panic!("display")
        };
        let rest_cue = rest_public.journey.unwrap().tempo.unwrap();
        assert_eq!(rest_cue.intensity_permille, 80);
        assert_eq!(rest_cue.phase, rpc::TempoPhase::Calm as i32);
        assert!(rest_cue.sequence > 1);
        let mut resumed = TempoOwner::start(
            checkpoints.last().unwrap(),
            pack::TempoTuning::default(),
            origin,
        )
        .unwrap();
        let mut view = wire::journey_view(
            checkpoints.last().unwrap(),
            LocalDemoRole::Display,
            MemberId::from_bytes(&DISPLAY).unwrap(),
        )
        .unwrap();
        resumed
            .decorate(&mut view, checkpoints.last().unwrap(), origin)
            .unwrap();
        let rpc::view_message::Audience::Display(view) = view.audience.unwrap() else {
            panic!("display")
        };
        let cue = view.journey.unwrap().tempo.unwrap();
        assert_eq!(cue.intensity_permille, 60);
        assert_eq!(cue.phase, rpc::TempoPhase::Calm as i32);
        assert_eq!(cue.sequence, 1);
    }

    #[test]
    fn live_tempo_cannot_observe_private_note_or_unrevealed_antagonist_statistics() {
        use crate::gameplay::actor::tempo::TempoOwner;
        use df_persistence::local_demo_scope::{DISPLAY, LocalDemoRole};
        use std::time::{Duration, Instant};
        let origin = Instant::now();
        let now = origin + Duration::from_secs(10);
        let opening = opening_story();
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let dialogue = stage_with_supplier(
            &opening,
            &input(&opening, first, 6, "ask-courier", vec![]),
            &mut |_| panic!("dialogue dice"),
        )
        .unwrap();
        let mut changed = dialogue.state().clone();
        let hidden = changed.facts.iter_mut().find(|fact| fact.audience == AudienceScope::Members(vec![first]) && matches!(&fact.value, FactValue::ContentEvent { definition, .. } if definition.entry.as_str() == "private-courier-note")).unwrap();
        hidden.value = FactValue::ContentEvent {
            definition: model::content("harbor").unwrap(),
            subjects: vec![],
        };
        let changed = model::checkpoint(dialogue.basis(), changed).unwrap();
        let accepted = accepted(dialogue.state().decisions.last().unwrap()).unwrap();
        let observe = |opening: &Checkpoint, current: &Checkpoint| {
            let mut owner =
                TempoOwner::start(opening, pack::TempoTuning::default(), origin).unwrap();
            owner
                .accepted(
                    current,
                    &accepted,
                    rpc::GameplayActionKind::AskCourier as i32,
                    now,
                )
                .unwrap();
            let mut view = wire::journey_view(
                current,
                LocalDemoRole::Display,
                MemberId::from_bytes(&DISPLAY).unwrap(),
            )
            .unwrap();
            owner.decorate(&mut view, current, now).unwrap();
            let rpc::view_message::Audience::Display(view) = view.audience.unwrap() else {
                panic!("display")
            };
            view.journey.unwrap().tempo.unwrap()
        };
        let original = observe(&opening, &dialogue);
        assert_eq!(original, observe(&opening, &changed));
        let mut tuning: serde_json::Value = serde_json::from_slice(include_bytes!(
            "fixtures/harbor-journey-tuning-baseline.json"
        ))
        .unwrap();
        tuning["bandit"]["attack_bonus"] = serde_json::json!(19);
        tuning["bandit"]["abilities"]["intelligence"] = serde_json::json!(20);
        let loaded = pack::load_from_bytes(
            include_bytes!("../../../../content/harbor-journey/pack.json"),
            &serde_json::to_vec(&tuning).unwrap(),
        )
        .unwrap();
        pack::with_test_pack(loaded, || {
            // Rule departure text is publicly disclosed, but mechanical stats themselves never
            // enter the tempo input. Public phase/outcomes/HP alone select the exact same cue.
            let changed_opening = opening_story();
            let changed_dialogue = stage_with_supplier(
                &changed_opening,
                &input(&changed_opening, first, 6, "ask-courier", vec![]),
                &mut |_| panic!("dialogue dice"),
            )
            .unwrap();
            assert_eq!(original, observe(&changed_opening, &changed_dialogue));
        });
    }

    #[test]
    fn baseline_tuning_reproduces_full_compiled_baseline_journey() {
        let baseline = tuning_journey();
        let loaded = pack::load_from_bytes(
            // 613839eb changed the shipped inn asset; tuning parity uses one frozen pack.
            include_bytes!("fixtures/harbor-journey-pack-baseline.json"),
            include_bytes!("fixtures/harbor-journey-tuning-baseline.json"),
        )
        .unwrap();
        pack::with_test_pack(loaded, || {
            let actual = tuning_journey();
            assert_eq!(actual.len(), baseline.len());
            for (actual, expected) in actual.iter().zip(&baseline) {
                assert_eq!(actual.state(), expected.state());
                assert_eq!(actual.basis(), expected.basis());
            }
        });
    }

    #[test]
    fn tuned_journey_changes_creation_attack_healing_rest_and_completes() {
        let mut tuning: serde_json::Value = serde_json::from_slice(include_bytes!(
            "fixtures/harbor-journey-tuning-baseline.json"
        ))
        .unwrap();
        tuning["hero"]["armor_class"] = 15.into();
        tuning["hero"]["proficiency_bonus"] = 3.into();
        tuning["hero"]["base_hit_points"] = 18.into();
        tuning["hero"]["background_bonus"]["constitution"] = 2.into();
        tuning["hero"]["ability_arrays"][0]["scores"][2] = 16.into();
        tuning["hero"]["second_wind"]["die_sides"] = 12.into();
        tuning["hero"]["second_wind"]["flat_bonus"] = 3.into();
        tuning["hero"]["second_wind"]["uses_max"] = 3.into();
        tuning["hero"]["second_wind"]["recharge_per_short_rest"] = 2.into();
        tuning["hero"]["weapon"]["damage_sides"] = 8.into();
        tuning["bandit"]["hit_points"] = 30.into();
        tuning["bandit"]["armor_class"] = 15.into();
        tuning["bandit"]["abilities"]["dexterity"] = 16.into();
        tuning["bandit"]["attack_bonus"] = 7.into();
        tuning["bandit"]["damage_modifier"] = 2.into();
        tuning["bandit"]["weapon"]["damage_sides"] = 4.into();
        tuning["combat"]["round_ticks"] = 7.into();
        tuning["combat"]["critical_threshold"] = 18.into();
        tuning["combat"]["combat_log_entries"] = 1.into();
        tuning["rest"]["short_rest_seconds"] = 600.into();
        tuning["narrative"]["intervention_budget"] = 16.into();
        let loaded = pack::load_from_bytes(
            include_bytes!("../../../../content/harbor-journey/pack.json"),
            &serde_json::to_vec(&tuning).unwrap(),
        )
        .unwrap();
        pack::with_test_pack(loaded, || {
            assert_eq!(initial().unwrap().state().narrative.remaining_budget, 16);
            assert_eq!(
                rules::heal_second_wind(&pack::second_wind(), 12, 1, 22, 2).unwrap(),
                (16, 1)
            );
            let checkpoints = tuning_journey();
            let hero = entity(ENTITIES[0]).unwrap();
            let monster = entity(BANDIT).unwrap();
            let sheet = character_sheet(checkpoints[0].state(), hero).unwrap();
            assert_eq!(
                (
                    sheet.hit_points,
                    sheet.maximum_hit_points,
                    sheet.armor_class
                ),
                (22, 22, 15)
            );
            assert!(
                wire::creation_offer()
                    .unwrap()
                    .groups
                    .iter()
                    .find(|group| group.group_id == "array")
                    .unwrap()
                    .options[0]
                    .description
                    .contains("CON18")
            );
            assert_eq!(
                value(checkpoints[2].state(), monster, "hit-points").unwrap(),
                30
            );
            assert_eq!(
                value(checkpoints[2].state(), monster, "armor-class").unwrap(),
                15
            );
            let hurt = checkpoints[4].state();
            assert_eq!(hurt.logical_time.ticks, 7);
            assert_eq!(value(hurt, hero, "hit-points").unwrap(), 16);
            let enemy = accepted(hurt.decisions.last().unwrap())
                .unwrap()
                .combat
                .remove(0);
            assert_eq!((enemy.attack_modifier, enemy.damage), (7, 6));
            assert_eq!(
                value(checkpoints[5].state(), hero, "hit-points").unwrap(),
                22
            );
            assert_eq!(
                value(checkpoints[5].state(), hero, "second-wind").unwrap(),
                2
            );
            let outcome = accepted(checkpoints[6].state().decisions.last().unwrap())
                .unwrap()
                .combat
                .remove(0);
            assert_eq!(outcome.attack_modifier, 6);
            assert!(outcome.critical);
            assert_eq!(outcome.attack_die, 18);
            assert_eq!(outcome.damage_dice, [8, 8, 8, 8]);
            assert_eq!(outcome.target_armor_class, 15);
            assert_eq!(checkpoints[7].state().logical_time.ticks, 607);
            assert_eq!(
                value(checkpoints[7].state(), hero, "second-wind").unwrap(),
                3
            );
            let restored = rules::complete_no_hit_dice_short_rest(
                &pack::rest(),
                rules::NoHitDiceShortRest {
                    started: 0,
                    finished: 600,
                    ticks_per_second: 1,
                    interrupted: false,
                    hit_points: 1,
                    fighter_second_wind_uses: Some(1),
                    knockout_unconscious: false,
                },
                pack::second_wind().uses_max,
                pack::second_wind().recharge_per_short_rest,
            )
            .unwrap();
            assert_eq!(restored.fighter_second_wind_uses, Some(3));
            let view = wire::journey_view(
                &checkpoints[6],
                df_persistence::local_demo_scope::LocalDemoRole::Player,
                MemberId::from_bytes(&MEMBERS[0]).unwrap(),
            )
            .unwrap();
            let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
                panic!("player");
            };
            let outcomes = player.journey.unwrap().combat.unwrap().outcomes;
            assert_eq!(outcomes.len(), 1);
            let mut actual = outcomes[0].clone();
            actual.sequence = 0;
            assert_eq!(actual, outcome);
            assert!(outcomes[0].sequence > 0);
            assert!(
                outcomes
                    .windows(2)
                    .all(|pair| pair[0].sequence < pair[1].sequence)
            );
            assert_eq!(
                phase(checkpoints.last().unwrap()).unwrap(),
                rpc::JourneyPhase::Complete
            );
        });
    }

    #[test]
    fn a_forged_constitution_row_and_hit_points_above_the_recorded_build_are_refused() {
        let current = opening_story();
        let hero = entity(ENTITIES[0]).unwrap();
        let recorded = maximum_hit_points(current.state(), hero).unwrap();
        let mut forged = current.state().clone();
        for resource in forged
            .resources
            .iter_mut()
            .filter(|resource| resource.owner == hero)
        {
            match resource.resource.as_str() {
                "constitution" => resource.value = 20,
                "hit-points" => {
                    // The party-wide row would admit this figure; the hero's recorded build does not.
                    resource.maximum = i64::from(recorded) + 2;
                    resource.value = resource.maximum;
                }
                _ => {}
            }
        }
        assert_eq!(maximum_hit_points(&forged, hero).unwrap(), recorded);
        assert!(model::checkpoint(current.basis(), forged).is_err());
    }

    #[test]
    fn resource_rows_of_a_party_entity_without_a_character_are_refused_whatever_their_value() {
        let current = opening_story();
        let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        let orphan = current
            .state()
            .characters
            .iter()
            .find(|character| character.owner == second)
            .unwrap()
            .entity;
        let mut orphaned = current.state().clone();
        orphaned
            .characters
            .retain(|character| character.owner != second);
        assert!(model::checkpoint(current.basis(), orphaned.clone()).is_err());
        // The same orphan with its hit points exactly at the party-wide figure is refused too.
        let party_wide = resources()
            .unwrap()
            .into_iter()
            .find(|constraint| {
                constraint.owner == orphan && constraint.resource.as_str() == "hit-points"
            })
            .unwrap()
            .maximum;
        let hit_points = orphaned
            .resources
            .iter_mut()
            .find(|resource| resource.owner == orphan && resource.resource.as_str() == "hit-points")
            .unwrap();
        hit_points.maximum = party_wide;
        hit_points.value = party_wide;
        assert!(model::checkpoint(current.basis(), orphaned).is_err());
    }

    #[test]
    fn a_stored_inn_scene_may_lack_its_source_key_only_under_the_legacy_pins() {
        let check = |accepts_keyless: bool| {
            let current = inn_scene();
            let mut keyless = current.clone();
            keyless.source_scene_key.clear();
            assert!(is_inn_scene(&current));
            assert_eq!(is_inn_scene(&keyless), accepts_keyless);
            for change in 0..6 {
                let mut scene = keyless.clone();
                match change {
                    0 => scene.title.push('!'),
                    1 => scene.scene_asset.push('x'),
                    2 => scene.destination = rpc::HarborDestination::Unspecified as i32,
                    3 => scene.speaker_portrait.push('x'),
                    4 => scene.rules_note.push('x'),
                    _ => scene.source_scene_key.push('x'),
                }
                assert!(!is_inn_scene(&scene), "change {change}");
            }
        };
        let legacy = pack::load_from_bytes(
            include_bytes!("fixtures/legacy-beats/pack.json"),
            include_bytes!("fixtures/harbor-journey-tuning-baseline.json"),
        )
        .unwrap();
        pack::with_test_pack(legacy, || check(true));
        // The shipped tuning is tactical: the current profile.
        let current = pack::load_from_bytes(
            include_bytes!("../../../../content/harbor-journey/pack.json"),
            include_bytes!("../../../../content/harbor-journey/tuning.json"),
        )
        .unwrap();
        pack::with_test_pack(current, || check(false));
        check(false);
    }

    #[test]
    fn empty_character_name_crosses_codec_but_cannot_create_or_draw() {
        let initial = initial().expect("empty room");
        let current = stage_join(
            &initial,
            &input(
                &initial,
                bootstrap_member().expect("bootstrap"),
                1,
                "join-room",
                vec![],
            ),
        )
        .expect("real joined member");
        let member = MemberId::from_bytes(&MEMBERS[0]).expect("joined member");
        let basis = current.basis();
        let request = rpc::SubmitActionRequest {
            session_id: Some(rpc::SessionId {
                value: Some(basis.session.as_bytes().to_vec()),
            }),
            run_id: Some(rpc::RunId {
                value: Some(basis.run.as_bytes().to_vec()),
            }),
            operation_id: Some(rpc::OperationId {
                value: Some(vec![2; 16]),
            }),
            observed_revision: Some(super::super::wire::revision(basis.revision)),
            offer_id: offer_id(&current, rpc::GameplayActionKind::CreateCharacter),
            action_kind: rpc::GameplayActionKind::CreateCharacter as i32,
            character: Some(rpc::CharacterSelection {
                choices: vec![rpc::JourneyChoice {
                    group_id: "portrait".to_owned(),
                    option_id: "portrait-hero-1".to_owned(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let converted = super::super::wire::journey_input(&request, member, &current)
            .expect("bounded semantic-invalid request must reach server validation");
        df_model::commands::validate_client_command(
            &converted,
            &current,
            ReferenceInventory {
                rules: &[
                    model::rule().expect("existing rule"),
                    rule().expect("journey rule"),
                ],
                content: &model::contents().expect("content"),
                resources: &resources_for(current.state()).expect("resources"),
                assets: &[],
            },
            df_model::commands::CommandLimits {
                maximum_records: 32,
                maximum_text_bytes: 128,
                maximum_retained_bytes: 8192,
            },
        )
        .expect("canonical structural admission");
        assert_eq!(
            rules::validate_character(&rules::CreationRules::srd_521(), "", &[]),
            Err(rules::JourneyRuleError::InvalidName)
        );
        let before = current.clone();
        let mut calls = 0;
        assert!(
            stage_using(&current, &converted, &mut |_| {
                calls += 1;
                Ok(1)
            })
            .is_err()
        );
        assert_eq!(calls, 0);
        assert_eq!(current, before);
        assert!(current.state().characters.is_empty());
    }
    #[test]
    fn the_initiative_rolls_come_from_the_recorded_draws_in_turn_order() {
        let mut current = initial().expect("empty room");
        for op in [1, 2] {
            current = stage_join(
                &current,
                &input(
                    &current,
                    bootstrap_member().expect("bootstrap"),
                    op,
                    "join-room",
                    vec![],
                ),
            )
            .expect("actual join");
        }
        let first = MemberId::from_bytes(&MEMBERS[0]).expect("first");
        let second = MemberId::from_bytes(&MEMBERS[1]).expect("second");
        let mut none = |_: u32| Err(RepositoryError::InvalidCandidate);
        for (op, member, name) in [(3, second, "Vale"), (4, first, "Brynn")] {
            current = stage_using(
                &current,
                &input(&current, member, op, "create-character", build(name)),
                &mut none,
            )
            .expect("character");
        }
        for (op, kind) in [(5, "begin-story"), (6, "ask-courier")] {
            current = stage_using(
                &current,
                &input(&current, first, op, kind, vec![]),
                &mut none,
            )
            .expect("story step");
        }
        // Three recorded d20 faces: the first hero 4, the second 15, the bandit 15.
        let mut faces = [4, 15, 15].into_iter();
        let mut dice = |_: u32| Ok(faces.next().expect("three initiative draws"));
        current = stage_using(
            &current,
            &input(&current, first, 7, "defend-courier", vec![]),
            &mut dice,
        )
        .expect("initiative");
        let rolls = initiative_rolls(&current).expect("rolls from the record");
        let encounter = current.state().encounters.first().expect("encounter");
        assert_eq!(rolls.len(), 3);
        let ids: Vec<Vec<u8>> = rolls
            .iter()
            .map(|roll| roll.participant_id.clone())
            .collect();
        let turn: Vec<Vec<u8>> = encounter
            .turn_order
            .iter()
            .map(|who| who.as_bytes().to_vec())
            .collect();
        assert_eq!(ids, turn, "listed in the encounter's own turn order");
        assert_eq!(
            rolls.iter().map(|roll| roll.order).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        for roll in &rolls {
            assert_eq!(roll.total, roll.die as i32 + roll.modifier);
            assert!([4, 15].contains(&roll.die));
            assert!(!roll.name.is_empty());
        }
        let totals: Vec<i32> = rolls.iter().map(|roll| roll.total).collect();
        assert!(
            totals.windows(2).all(|pair| pair[0] >= pair[1]),
            "highest first: {totals:?}"
        );
        // The tie between the second hero and the bandit (both rolled 15) resolves by the
        // recorded order, not a re-roll: no new draw was made to produce the view.
        assert_eq!(current.state().draws.len(), 3);
    }
    #[test]
    fn reverse_character_finalization_preserves_join_order_initiative_and_private_dialogue() {
        let mut current = initial().expect("empty room");
        for op in [1, 2] {
            current = stage_join(
                &current,
                &input(
                    &current,
                    bootstrap_member().expect("bootstrap"),
                    op,
                    "join-room",
                    vec![],
                ),
            )
            .expect("actual join");
        }
        let first = MemberId::from_bytes(&MEMBERS[0]).expect("first");
        let second = MemberId::from_bytes(&MEMBERS[1]).expect("second");
        let no_draw = |_: u32| Err(RepositoryError::InvalidCandidate);
        let mut supply = no_draw;
        current = stage_using(
            &current,
            &input(&current, second, 3, "create-character", build("Vale")),
            &mut supply,
        )
        .expect("second creates first");
        current = stage_using(
            &current,
            &input(&current, first, 4, "create-character", build("Brynn")),
            &mut supply,
        )
        .expect("first creates second");
        current = stage_using(
            &current,
            &input(&current, first, 5, "begin-story", vec![]),
            &mut supply,
        )
        .expect("opening");
        current = stage_using(
            &current,
            &input(&current, first, 6, "ask-courier", vec![]),
            &mut supply,
        )
        .expect("dialogue saved");
        let private = current.state().facts.last().expect("fact");
        assert_eq!(private.audience, AudienceScope::Members(vec![first]));
        let mut equal = |_: u32| Ok(10);
        current = stage_using(
            &current,
            &input(&current, first, 7, "defend-courier", vec![]),
            &mut equal,
        )
        .expect("initiative");
        let encounter = current.state().encounters.first().expect("encounter");
        assert_eq!(
            encounter.turn_order,
            vec![
                entity(ENTITIES[0]).expect("first entity"),
                entity(ENTITIES[1]).expect("second entity"),
                entity(BANDIT).expect("bandit")
            ]
        );
        assert_eq!(
            encounter.active_turn,
            Some(entity(ENTITIES[0]).expect("first entity"))
        );
        assert_eq!(current.state().draws.len(), 3);
        let wrong = input(
            &current,
            second,
            8,
            "greatsword-attack",
            vec![
                (
                    model::label("savage-attacker").expect("key"),
                    model::label("no").expect("option"),
                ),
                (
                    model::label("graze").expect("key"),
                    model::label("no").expect("option"),
                ),
            ],
        );
        let mut forbid = |_: u32| panic!("wrong turn must never draw");
        assert!(stage_using(&current, &wrong, &mut forbid).is_err());
        assert_eq!(current.state().draws.len(), 3);
    }

    pub(super) fn prepared_story() -> Checkpoint {
        let mut current = initial().expect("initial room");
        for op in [1, 2] {
            current = stage_join(
                &current,
                &input(
                    &current,
                    bootstrap_member().unwrap(),
                    op,
                    "join-room",
                    vec![],
                ),
            )
            .expect("joined member");
        }
        for (op, bytes, name) in [(3, MEMBERS[0], "Brynn"), (4, MEMBERS[1], "Vale")] {
            current = stage_with_supplier(
                &current,
                &input(
                    &current,
                    MemberId::from_bytes(&bytes).unwrap(),
                    op,
                    "create-character",
                    build(name),
                ),
                &mut |_| panic!("creation cannot draw"),
            )
            .unwrap_or_else(|error| {
                panic!(
                    "registered character outcome: {error:?}, step {:?}",
                    refusal_detail::take()
                )
            });
        }
        current
    }

    pub(super) fn table_story(count: usize) -> Checkpoint {
        let mut current = initial().unwrap();
        assert!(!party_ready(&current));
        for index in 0..count {
            current = stage_join(
                &current,
                &input(
                    &current,
                    bootstrap_member().unwrap(),
                    index as u8 + 1,
                    "join-room",
                    vec![],
                ),
            )
            .unwrap();
        }
        for (index, name) in ["Brakka", "Sigrun", "Brynn", "Corin", "Elara", "Vix"]
            .into_iter()
            .take(count)
            .enumerate()
        {
            assert!(!party_ready(&current));
            let member = MemberId::from_bytes(&MEMBERS[index]).unwrap();
            assert!(
                !offered(&current, member)
                    .unwrap()
                    .iter()
                    .any(|(kind, _)| *kind == rpc::GameplayActionKind::BeginStory)
            );
            let mut choices = build(name);
            if count == MAXIMUM_PARTY {
                // Exercise the accepted per-hero text caps, not just six minimal sheets.
                let mut identity = super::super::hero_identity::Identity::from_menus(&[])
                    .unwrap()
                    .to_rpc();
                identity.appearance.as_mut().unwrap().public_description =
                    format!("{}\u{96ea}{}", "\u{1f409}".repeat(698), "a".repeat(1301));
                identity.origin = "\u{1f409}".repeat(160);
                identity.occupation = "\u{1f409}".repeat(160);
                identity.motivation = "\u{1f409}".repeat(160);
                identity.backstory = "\u{1f409}".repeat(480);
                identity.facts = (0..6)
                    .map(|fact| format!("{fact}{}", "\u{1f409}".repeat(159)))
                    .collect();
                let validated =
                    super::super::hero_identity::Identity::from_rpc(identity.clone()).unwrap();
                for (field, selected) in validated.canonical_appearance_choices() {
                    choices.push((
                        model::label(&format!("identity-appearance:{field}")).unwrap(),
                        model::label(selected).unwrap(),
                    ));
                }
                for (field, selected) in validated.canonical_public_choices() {
                    choices.push((
                        model::label(&field).unwrap(),
                        model::label(&selected).unwrap(),
                    ));
                }
                choices.extend(
                    super::super::hero_library::identity_choices(Some(&rpc::CharacterSelection {
                        name: name.to_owned(),
                        identity: Some(identity),
                        portrait_scenery: format!("{}{}", "\u{1f409}".repeat(136), "a".repeat(224)),
                        ..Default::default()
                    }))
                    .unwrap(),
                );
            }

            if !super::super::beat_script::with_script(|script| script.is_canonical()) {
                choices.push((
                    model::label(super::super::beat_script_runtime::CHOICE_GROUP).unwrap(),
                    model::label("create-character").unwrap(),
                ));
            }
            // Match the compact wire expansion: admission counts unused Vec capacity too.
            // Keep every maximal identity field while avoiding allocation growth in this fixture.
            choices.shrink_to_fit();
            let creation = input(
                &current,
                member,
                index as u8 + 10,
                "create-character",
                choices,
            );
            current =
                stage_with_supplier(&current, &creation, &mut |_| panic!("creation cannot draw"))
                    .unwrap_or_else(|error| {
                        panic!(
                            "seat {index} creation: {error:?}; bytes={:?}; detail={:?}; step={:?}",
                            creation.retained_bytes(),
                            df_session::submission::take_refusal_diagnostic(),
                            refusal_detail::take()
                        )
                    });
        }
        assert!(party_ready(&current));
        if count == MAXIMUM_PARTY {
            assert_eq!(join_selection(&current), Err(RepositoryError::Capacity));
            assert_eq!(
                stage_join(
                    &current,
                    &input(
                        &current,
                        bootstrap_member().unwrap(),
                        25,
                        "join-room",
                        vec![]
                    )
                ),
                Err(RepositoryError::Capacity)
            );
        }
        let choices = if super::super::beat_script::with_script(|script| script.is_canonical()) {
            vec![]
        } else {
            vec![(
                model::label(super::super::beat_script_runtime::CHOICE_GROUP).unwrap(),
                model::label("begin-story").unwrap(),
            )]
        };
        stage_with_supplier(
            &current,
            &input(
                &current,
                MemberId::from_bytes(&MEMBERS[0]).unwrap(),
                26,
                "begin-story",
                choices,
            ),
            &mut |_| panic!("opening cannot draw"),
        )
        .unwrap_or_else(|error| {
            panic!(
                "table opening: {error:?}; detail={:?}; step={:?}",
                df_session::submission::take_refusal_diagnostic(),
                refusal_detail::take()
            )
        })
    }

    #[test]
    fn hs54_memos_reject_changed_content_under_the_same_basis_and_foreign_pins() {
        let current = table_story(1);
        assert!(current.state().encounters.is_empty());
        assert_eq!(phase(&current).unwrap(), rpc::JourneyPhase::Opening);
        let role = df_persistence::local_demo_scope::LocalDemoRole::Display;
        let member = bootstrap_member().unwrap();
        let expected = wire::journey_view(&current, role, member).unwrap();
        let mut changed = current.state().clone();
        // A structural checkpoint can retain its basis while its journey receipt is invalid.
        changed.decisions.last_mut().unwrap().source_policy =
            model::label("hs54-foreign-policy").unwrap();
        let alias = model::checkpoint(current.basis(), changed).unwrap();
        assert_eq!(
            current.basis(),
            alias.basis(),
            "Basis alone does not uniquely name content"
        );
        assert!(wire::journey_view(&alias, role, member).is_err());
        assert_eq!(
            wire::journey_view(&current, role, member).unwrap(),
            expected
        );

        let mut pins = current.pins().clone();
        pins.build = df_types::BuildIdentity::new(
            Some("hs54"),
            Some("native"),
            Some("wasm"),
            Some("configuration"),
            Some("content"),
        )
        .unwrap();
        let foreign = Checkpoint::new(
            CHECKPOINT_SCHEMA,
            current.basis(),
            pins,
            current.state().clone(),
            ReferenceInventory {
                rules: &[model::rule().unwrap(), rule().unwrap()],
                content: &model::contents().unwrap(),
                resources: &resources_for(current.state()).unwrap(),
                assets: &[],
            },
            model::limits(),
        )
        .unwrap();
        assert_eq!(foreign.state(), current.state());
        assert!(
            phase(&foreign).is_err(),
            "a cached phase never bypasses resume admission"
        );
        assert!(wire::journey_view(&foreign, role, member).is_err());
    }

    #[test]
    fn hs54_pack_replacement_invalidates_views_and_phase_memos() {
        let loaded = pack::load_from_bytes(
            include_bytes!("../../../../content/merchant-shop/pack.json"),
            include_bytes!("../../../../content/merchant-shop/tuning.json"),
        )
        .unwrap();
        let mut replacement = loaded.clone();
        replacement.pack.room_code = "HS54NEW".into();
        pack::with_test_pack(loaded, || {
            let current = table_story(1);
            assert!(current.state().encounters.is_empty());
            assert_eq!(phase(&current).unwrap(), rpc::JourneyPhase::Opening);
            let member = bootstrap_member().unwrap();
            let role = df_persistence::local_demo_scope::LocalDemoRole::Display;
            let before = wire::journey_view(&current, role, member).unwrap();
            pack::with_test_pack(replacement, || {
                let after = wire::journey_view(&current, role, member).unwrap();
                assert_ne!(
                    before, after,
                    "installed pack identity is a memo input even with equal source pins"
                );
            });
            assert_eq!(wire::journey_view(&current, role, member).unwrap(), before);
        });
    }

    #[test]
    fn hs54_six_hero_free_roam_100_views_cpu_budget() {
        let loaded = pack::load_from_bytes(
            include_bytes!("../../../../content/merchant-shop/pack.json"),
            include_bytes!("../../../../content/merchant-shop/tuning.json"),
        )
        .unwrap();
        pack::with_test_pack(loaded, || {
            let current = table_story(6);
            for line in super::super::actor::audit_lines_containing("STAGE ") {
                eprintln!("{line}");
            }
            assert_eq!(current.state().characters.len(), 6);
            assert_eq!(phase(&current).unwrap(), rpc::JourneyPhase::Opening);
            let checkpoint_started = std::time::Instant::now();
            for _ in 0..100 {
                std::hint::black_box(
                    model::checkpoint(current.basis(), current.state().clone()).unwrap(),
                );
            }
            eprintln!(
                "HS54 checkpoint_rebuilds=100 elapsed_us={}",
                checkpoint_started.elapsed().as_micros()
            );
            let started = std::time::Instant::now();
            for index in 0..100 {
                let role = if index % 7 == 0 {
                    df_persistence::local_demo_scope::LocalDemoRole::Display
                } else {
                    df_persistence::local_demo_scope::LocalDemoRole::Player
                };
                let member = MemberId::from_bytes(&MEMBERS[index % 6]).unwrap();
                let view = wire::journey_view(&current, role, member).unwrap();
                std::hint::black_box(view);
            }
            let elapsed = started.elapsed();
            // Parallel CI workers need scheduler headroom; this still bounds each view to 20 ms.
            eprintln!(
                "HS54 six_heroes views=100 elapsed_us={} budget_us=2000000",
                elapsed.as_micros()
            );
            assert!(
                elapsed < Duration::from_secs(2),
                "100 pure projections exceeded the parallel-load wall-time budget: {elapsed:?}"
            );
        });
    }

    #[test]
    fn one_to_six_joined_seats_begin_only_after_every_hero_is_accepted_and_resume() {
        let codec = || df_persistence::NativeCodecLimits {
            maximum_document_bytes: 4 * 1024 * 1024,
            maximum_allocated_bytes: 4 * 1024 * 1024,
            maximum_collection_items: 8192,
            maximum_text_bytes: model::limits().maximum_text_bytes,
        };
        for count in 1..=MAXIMUM_PARTY {
            let opening = table_story(count);
            assert_eq!(phase(&opening).unwrap(), rpc::JourneyPhase::Opening);
            assert_eq!(opening.state().characters.len(), count);
            assert_eq!(participants(&opening).len(), count);
            let bytes =
                df_persistence::local_demo_scope::encode_owned_demo_checkpoint(&opening, codec())
                    .unwrap();
            let restored = df_persistence::local_demo_scope::decode_owned_demo_checkpoint(
                &bytes,
                opening.basis(),
                &model::pins().unwrap(),
                &model::recovery().unwrap(),
                codec(),
            )
            .unwrap();
            assert_eq!(restored, opening);
            assert_eq!(phase(&restored).unwrap(), rpc::JourneyPhase::Opening);
            for (index, link) in participants(&restored).into_iter().enumerate() {
                assert_eq!(link.member.as_bytes(), &MEMBERS[index]);
                assert_eq!(link.character.unwrap().as_bytes(), &ENTITIES[index]);
                let view = wire::journey_view(
                    &restored,
                    df_persistence::local_demo_scope::LocalDemoRole::Player,
                    link.member,
                )
                .unwrap();
                let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
                    panic!("player view");
                };
                assert_eq!(player.journey.unwrap().party.len(), count);
            }
        }
        assert_eq!(MEMBERS[0], [0x61; 16]);
        assert_eq!(MEMBERS[1], [0x62; 16]);
        assert_eq!(ENTITIES[0], [0x63; 16]);
        assert_eq!(ENTITIES[1], [0x64; 16]);
    }

    #[test]
    fn one_to_six_heroes_finish_the_courier_fight_and_receive_the_inn_invitation() {
        for count in 1..=MAXIMUM_PARTY {
            let opening = table_story(count);
            let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
            let dialogue = stage_with_supplier(
                &opening,
                &input(&opening, first, 27, "escort-courier", vec![]),
                &mut |_| panic!("dialogue cannot draw"),
            )
            .unwrap();
            let mut ordinal = 0;
            let mut current = stage_with_supplier(
                &dialogue,
                &input(&dialogue, first, 28, "defend-courier", vec![]),
                &mut |_| {
                    ordinal += 1;
                    Ok(if ordinal <= count { 20 } else { 1 })
                },
            )
            .unwrap_or_else(|error| {
                panic!(
                    "{count}-hero courier start: {error:?}; detail={:?}; step={:?}",
                    df_session::submission::take_refusal_diagnostic(),
                    refusal_detail::take()
                )
            });
            assert_eq!(current.state().encounters[0].turn_order.len(), count + 1);
            assert_eq!(current.state().draws.len(), count + 1);
            for operation in 30..100 {
                if phase(&current).unwrap() == rpc::JourneyPhase::Complete {
                    break;
                }
                let member = participants(&current)
                    .into_iter()
                    .map(|link| link.member)
                    .find(|member| {
                        offered(&current, *member)
                            .unwrap()
                            .iter()
                            .any(|(kind, _)| *kind == rpc::GameplayActionKind::GreatswordAttack)
                    })
                    .unwrap();
                current = stage_with_supplier(
                    &current,
                    &input(&current, member, operation, "greatsword-attack", vec![]),
                    &mut |sides| Ok(sides),
                )
                .unwrap();
            }
            assert_eq!(phase(&current).unwrap(), rpc::JourneyPhase::Complete);
            assert!(combat_victory(current.state()).unwrap());
            assert!(inn_source(&current).unwrap().is_some());
        }
    }

    pub(super) fn opening_story() -> Checkpoint {
        let current = prepared_story();
        stage_with_supplier(
            &current,
            &input(
                &current,
                MemberId::from_bytes(&MEMBERS[0]).unwrap(),
                5,
                "begin-story",
                vec![],
            ),
            &mut |_| panic!("opening cannot draw"),
        )
        .unwrap_or_else(|error| {
            let source = narrative_transition::select(
                &current,
                &input(
                    &current,
                    MemberId::from_bytes(&MEMBERS[0]).unwrap(),
                    5,
                    "begin-story",
                    vec![],
                ),
            )
            .map(|_| ());
            panic!(
                "registered opening: {error:?}; source={source:?}; detail={:?}; step={:?}",
                df_session::submission::take_refusal_diagnostic(),
                refusal_detail::take()
            );
        })
    }

    #[test]
    fn source_bound_pending_stops_thread_composition_and_reaches_journey_transition_consumer() {
        let current = prepared_story();
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let command = input(&current, member, 5, "begin-story", vec![]);
        let completed =
            stage_with_supplier(&current, &command, &mut |_| panic!("opening cannot draw"))
                .unwrap();
        let source = completed.state().facts.last().unwrap();
        let decision = completed.state().decisions.last().unwrap();
        assert_eq!(decision.operation, source.operation);
        assert!(decision.effects.is_empty());
        let mut staged = completed.state().clone();
        // Retain the registered source decision and its authored opening state, before the
        // director applies the dependent thread consequence.
        staged.narrative = narrative_transition::select(&current, &command)
            .unwrap()
            .state()
            .narrative
            .clone();
        assert!(!staged.narrative.accepted_facts.contains(&source.id));
        staged.pending.push(PendingResolution {
            id: ResolutionId::from_bytes(&[0xb3; 16]).unwrap(),
            basis: completed.basis(),
            continuation: model::label("opening-source-continuation").unwrap(),
            window: ResolutionWindow {
                id: WindowId::from_bytes(&[0xb4; 16]).unwrap(),
                phase: TriggerPhase::BeforeConsequence,
                causal_fact: source.id,
                source: rule().unwrap(),
                timer: None,
            },
            next: PendingInput::Choice {
                remaining: vec![OfferedResponse {
                    participant: member,
                    offer: model::label("opening-source-offer").unwrap(),
                    options: vec![model::label("opening-source-option").unwrap()],
                    source: rule().unwrap(),
                }],
            },
            choices: vec![],
            draw_ordinals: vec![],
            spent: vec![],
            rulings: vec![],
        });
        let staged = model::checkpoint(completed.basis(), staged).unwrap();
        let suspended = stage_authored_threads(staged.clone(), source.id).unwrap();
        assert_eq!(suspended, staged);
        assert!(suspended.state().intents.is_empty());
        assert!(
            !suspended
                .state()
                .narrative
                .accepted_facts
                .contains(&source.id)
        );
        let result = model::staged_transition(&current, suspended, source.operation);
        assert!(matches!(
            result,
            df_model::transition::TransitionResult::Pending {
                resolution,
                window,
                next: PendingInput::Choice { remaining },
                ..
            } if resolution == ResolutionId::from_bytes(&[0xb3; 16]).unwrap()
                && window == WindowId::from_bytes(&[0xb4; 16]).unwrap()
                && remaining.len() == 1
        ));
        assert_eq!(current, prepared_story());
    }

    #[test]
    fn registered_narrative_tracks_private_packet_and_ignores_unmapped_events() {
        let ready = prepared_story();
        assert!(ready.state().narrative.accepted_facts.is_empty());
        assert!(ready.state().narrative.open_threads.is_empty());
        let current = opening_story();
        assert_eq!(
            current.state().narrative.open_threads,
            vec![model::content(PACKET_THREAD).unwrap()]
        );
        assert_eq!(current.state().narrative.accepted_facts.len(), 1);
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let command = input(&current, member, 6, "ask-courier", vec![]);
        let private =
            stage_with_supplier(&current, &command, &mut |_| panic!("dialogue cannot draw"))
                .expect("private accepted source");
        let replay = stage_with_supplier(&current, &command, &mut |_| panic!("replay cannot draw"))
            .expect("same immutable source");
        assert_eq!(private, replay);
        assert_eq!(
            private.state().narrative.open_threads,
            current.state().narrative.open_threads
        );
        assert_eq!(private.state().narrative.accepted_facts.len(), 2);
        let source = private.state().facts.last().unwrap();
        assert_eq!(source.audience, AudienceScope::Members(vec![member]));
        assert!(private.state().knowledge.is_empty());
        assert!(private.state().draws.is_empty());
        assert_eq!(
            private
                .state()
                .decisions
                .last()
                .unwrap()
                .source_policy
                .as_str(),
            THREAD_POLICY
        );
        assert_eq!(
            accepted(private.state().decisions.last().unwrap())
                .unwrap()
                .phase,
            rpc::JourneyPhase::Dialogue as i32
        );
        assert_eq!(
            stage_authored_threads(private.clone(), source.id),
            Err(RepositoryError::InvalidCandidate)
        );
        let mut suspended = private.state().clone();
        suspended.pending.push(PendingResolution {
            id: ResolutionId::from_bytes(&[0xb1; 16]).unwrap(),
            basis: private.basis(),
            continuation: model::label("private-note-continuation").unwrap(),
            window: ResolutionWindow {
                id: WindowId::from_bytes(&[0xb2; 16]).unwrap(),
                phase: TriggerPhase::BeforeConsequence,
                causal_fact: source.id,
                source: rule().unwrap(),
                timer: None,
            },
            next: PendingInput::Choice {
                remaining: vec![OfferedResponse {
                    participant: member,
                    offer: model::label("private-note-offer").unwrap(),
                    options: vec![model::label("private-note-option").unwrap()],
                    source: rule().unwrap(),
                }],
            },
            choices: vec![],
            draw_ordinals: vec![],
            spent: vec![],
            rulings: vec![],
        });
        let suspended = model::checkpoint(private.basis(), suspended).unwrap();
        assert_eq!(
            stage_authored_threads(suspended.clone(), source.id),
            Ok(suspended.clone())
        );
        // This private note already applied source effects. Adding a window afterward cannot
        // turn completed mechanics into a pending continuation.
        assert_eq!(
            df_model::transition::TransitionResult::<RepositoryError>::classify(
                &current,
                suspended,
                source.operation,
            ),
            Err(df_model::transition::TransitionError::PendingEffects)
        );
        assert!(
            stage_with_supplier(&private, &command, &mut |_| panic!(
                "stale command cannot draw"
            ))
            .is_err()
        );
        let untouched = private.clone();
        let mut old_pins = private.pins().clone();
        old_pins.content.package_digest = ContentDigest([99; 32]);
        let stale = Checkpoint::new(
            private.schema(),
            private.basis(),
            old_pins,
            private.state().clone(),
            ReferenceInventory {
                rules: &[model::rule().unwrap(), rule().unwrap()],
                content: &model::contents().unwrap(),
                resources: &resources_for(private.state()).unwrap(),
                assets: &[],
            },
            model::limits(),
        )
        .unwrap();
        assert!(
            stage_with_supplier(
                &stale,
                &input(&stale, member, 7, "defend-courier", vec![]),
                &mut |_| Ok(10)
            )
            .is_err()
        );
        assert_eq!(private, untouched);
    }

    #[test]
    fn registered_hit_preserves_threat_until_actual_knockout_then_retains_packet_branch() {
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let mut current = opening_story();
        current = stage_with_supplier(
            &current,
            &input(&current, member, 6, "escort-courier", vec![]),
            &mut |_| panic!("escort cannot draw"),
        )
        .unwrap();
        current = stage_with_supplier(
            &current,
            &input(&current, member, 7, "defend-courier", vec![]),
            &mut |_| Ok(10),
        )
        .unwrap();
        let threads = vec![
            model::content(PACKET_THREAD).unwrap(),
            model::content(THREAT_THREAD).unwrap(),
        ];
        assert_eq!(current.state().narrative.open_threads, threads);
        assert_eq!(current.state().draws.len(), 3);
        let attack_choices = || {
            vec![
                (
                    model::label("savage-attacker").unwrap(),
                    model::label("no").unwrap(),
                ),
                (model::label("graze").unwrap(), model::label("no").unwrap()),
            ]
        };
        let command = input(&current, member, 8, "greatsword-attack", attack_choices());
        current = stage_with_supplier(&current, &command, &mut |sides| {
            Ok(if sides == 20 { 10 } else { 1 })
        })
        .unwrap();
        assert_eq!(
            value(current.state(), entity(BANDIT).unwrap(), "unconscious").unwrap(),
            0
        );
        assert_eq!(current.state().narrative.open_threads, threads);
        assert_eq!(current.state().draws.len(), 6);
        current = stage_with_supplier(
            &current,
            &input(&current, member, 9, "end-turn", vec![]),
            &mut |_| panic!("next player needs no enemy draw"),
        )
        .unwrap();
        let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        let before = current.clone();
        let command = input(&current, second, 10, "greatsword-attack", attack_choices());
        let defeated = stage_with_supplier(&current, &command, &mut |sides| {
            Ok(if sides == 20 { 10 } else { 6 })
        })
        .unwrap();
        assert_eq!(
            value(defeated.state(), entity(BANDIT).unwrap(), "unconscious").unwrap(),
            1
        );
        assert_eq!(
            value(defeated.state(), entity(BANDIT).unwrap(), "hit-points").unwrap(),
            1
        );
        assert_eq!(
            defeated.state().narrative.open_threads,
            vec![model::content(PACKET_THREAD).unwrap()]
        );
        assert_eq!(phase(&defeated).unwrap(), rpc::JourneyPhase::Complete);
        assert_eq!(defeated.state().draws.len(), before.state().draws.len() + 3);
        assert_eq!(current, before);
        let repeated = stage_with_supplier(&current, &command, &mut |sides| {
            Ok(if sides == 20 { 10 } else { 6 })
        })
        .unwrap();
        assert_eq!(defeated, repeated);
        let decision = defeated.state().decisions.last().unwrap();
        let receipt = accepted(decision).unwrap();
        assert!(receipt.combat[0].knocked_out);
        assert_eq!(decision.source_policy.as_str(), THREAD_POLICY);
        assert!(defeated.state().facts.iter().any(|fact| decision.facts.contains(&fact.id)
            && matches!(&fact.value, FactValue::ResourceChanged { entity: who, resource, before: 0, after: 1, .. }
                if *who == entity(BANDIT).unwrap() && resource.as_str() == "unconscious")));
        assert!(defeated.state().continuity.catch_up.is_none());
    }

    #[test]
    fn registered_party_defeat_keeps_both_threads_unresolved() {
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        let opening = opening_story();
        let dialogue = stage_with_supplier(
            &opening,
            &input(&opening, first, 6, "escort-courier", vec![]),
            &mut |_| panic!("escort cannot draw"),
        )
        .unwrap();
        let mut initiative_count = 0;
        let combat = stage_with_supplier(
            &dialogue,
            &input(&dialogue, first, 7, "defend-courier", vec![]),
            &mut |sides| {
                if sides == 20 {
                    initiative_count += 1;
                    Ok(if initiative_count <= 2 { 1 } else { 20 })
                } else {
                    Ok(6)
                }
            },
        )
        .unwrap();
        assert_eq!(
            value(combat.state(), entity(ENTITIES[0]).unwrap(), "unconscious").unwrap(),
            1
        );
        assert_eq!(
            value(combat.state(), entity(BANDIT).unwrap(), "unconscious").unwrap(),
            0
        );
        let defeated = stage_with_supplier(
            &combat,
            &input(&combat, second, 8, "end-turn", vec![]),
            &mut |sides| Ok(if sides == 20 { 20 } else { 6 }),
        )
        .unwrap();
        assert_eq!(phase(&defeated).unwrap(), rpc::JourneyPhase::Complete);
        assert_eq!(
            value(
                defeated.state(),
                entity(ENTITIES[1]).unwrap(),
                "unconscious"
            )
            .unwrap(),
            1
        );
        assert_eq!(
            value(defeated.state(), entity(BANDIT).unwrap(), "unconscious").unwrap(),
            0
        );
        assert_eq!(
            defeated.state().narrative.open_threads,
            vec![
                model::content(PACKET_THREAD).unwrap(),
                model::content(THREAT_THREAD).unwrap()
            ]
        );

        assert!(!combat_victory(defeated.state()).unwrap());
        let completion = super::super::adventure::campaign_completion::terminal_outcome;
        assert!(completion(&defeated).unwrap().is_none());
        let started = stage_with_supplier(
            &defeated,
            &input(&defeated, first, 9, "short-rest", vec![]),
            &mut |_| panic!("rest dice"),
        )
        .unwrap();
        assert!(completion(&started).unwrap().is_none());
        let rested = finish_rest_keeping_dice(started.clone());
        let outcome = completion(&rested).unwrap().unwrap();
        assert_eq!(
            outcome.result,
            super::super::adventure::campaign::ChapterResult::Defeat
        );
        assert_eq!(
            outcome.approach,
            super::super::adventure::campaign::Approach::Escorted
        );
        assert_eq!(outcome.member, hex(first.as_bytes()));
        assert!(outcome.rested);
        assert!(!outcome.destination_reached);
        assert!(!outcome.antagonist_spared);

        let original = defeated.clone();
        let command = input(&defeated, first, 9, "short-rest", vec![]);
        assert_eq!(
            stage_with_supplier(&defeated, &command, &mut |_| panic!("rest cannot draw")),
            Ok(started.clone())
        );
        assert_rest_preserves_battle(&defeated, &rested);
        for bytes in ENTITIES.into_iter().take(2) {
            let who = entity(bytes).unwrap();
            assert_eq!(value(rested.state(), who, "hit-points"), Ok(1));
            assert_eq!(value(rested.state(), who, "unconscious"), Ok(0));
            assert_eq!(value(rested.state(), who, "prone"), Ok(1));
            assert_eq!(value(rested.state(), who, "held-weapon"), Ok(0));
            assert_eq!(
                rested
                    .state()
                    .entities
                    .iter()
                    .find(|item| item.id == held_weapon_id(who).unwrap())
                    .unwrap()
                    .location,
                Some(room_entity().unwrap())
            );
        }
        assert_eq!(defeated, original);
        assert_eq!(
            stage_with_supplier(&defeated, &command, &mut |_| panic!("deterministic rest")),
            Ok(started)
        );
        assert!(stage_with_supplier(&rested, &command, &mut |_| panic!("stale rest")).is_err());
        // One rest per aftermath: a second rest in the same aftermath is
        // refused by the machine, however much the heroes still lack.
        assert!(
            stage_with_supplier(
                &rested,
                &input(&rested, first, 10, "short-rest", vec![]),
                &mut |_| panic!("second rest cannot draw"),
            )
            .is_err()
        );
        let mut shifted = defeated.state().clone();
        let event = shifted
            .schedules
            .iter_mut()
            .find(|event| event.due.ticks > 3600)
            .unwrap();
        let original_start = event.due.ticks - 3600;
        let operation=defeated.state().facts.iter().find(|fact|matches!(&fact.value,FactValue::ContentEvent {definition,subjects} if definition.entry.as_str()=="knockout-short-rest-start" && subjects.contains(&event.entity))).unwrap().operation;
        event.due.ticks -= 1;
        event.id = knockout_rest_id(operation, event.entity, original_start - 1).unwrap();
        let shifted = model::checkpoint(defeated.basis(), shifted).unwrap();
        assert!(
            prepare_short_rest(&shifted).is_err(),
            "coordinated due/id rewrite must not alter accepted source time"
        );
        let mut unsupported = defeated.state().clone();
        unsupported.schedules[0].definition = model::content("short-rest-interrupted").unwrap();
        let unsupported = model::checkpoint(defeated.basis(), unsupported).unwrap();
        assert!(prepare_short_rest(&unsupported).is_err());
        let mut interrupted = defeated.state().clone();
        let last = interrupted.facts.last().unwrap().clone();
        let interruption_id = FactId::from_bytes(&[0xee; 16]).unwrap();
        interrupted.facts.push(GameFact {
            id: interruption_id,
            ordinal: last.ordinal + 1,
            value: FactValue::ContentEvent {
                definition: model::content("short-rest-interrupted").unwrap(),
                subjects: vec![entity(ENTITIES[0]).unwrap()],
            },
            cause: Some(last.id),
            ..last
        });
        interrupted
            .decisions
            .last_mut()
            .unwrap()
            .facts
            .push(interruption_id);
        let interrupted = model::checkpoint(defeated.basis(), interrupted).unwrap();
        assert!(
            prepare_short_rest(&interrupted).is_err(),
            "saved interruption source evidence must refuse without benefits"
        );
        assert!(
            stage_with_supplier(
                &interrupted,
                &input(&interrupted, first, 11, "short-rest", vec![]),
                &mut |_| panic!("interrupted rest cannot draw")
            )
            .is_err()
        );
    }
    fn assert_rest_preserves_battle(before: &Checkpoint, after: &Checkpoint) {
        assert_eq!(
            after.state().logical_time.ticks,
            before.state().logical_time.ticks + 3600
        );
        assert_eq!(after.state().draws, before.state().draws);
        assert_eq!(after.state().inventory, before.state().inventory);
        assert_eq!(after.state().entities, before.state().entities);
        assert_eq!(after.state().knowledge, before.state().knowledge);
        assert_eq!(
            after.state().narrative.open_threads,
            before.state().narrative.open_threads
        );
        assert_eq!(
            combat_victory(after.state()),
            combat_victory(before.state())
        );
        for resource in &before.state().resources {
            if resource.resource.as_str() == "hit-points" {
                assert_eq!(
                    value(after.state(), resource.owner, "hit-points"),
                    Ok(u32::try_from(resource.value).unwrap())
                );
            }
        }
        assert_eq!(after.state().schedules, before.state().schedules);
        assert!(after.state().continuity.catch_up.is_some());
    }
    #[test]
    fn victorious_rest_recharges_one_use_without_healing_or_rewriting_threat_closure() {
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let opening = opening_story();
        assert!(
            stage_with_supplier(
                &opening,
                &input(&opening, member, 6, "short-rest", vec![]),
                &mut |_| panic!("premature rest")
            )
            .is_err()
        );
        let private = stage_with_supplier(
            &opening,
            &input(&opening, member, 6, "ask-courier", vec![]),
            &mut |_| panic!("dialogue"),
        )
        .unwrap();
        let mut draw_index = 0;
        let combat = stage_with_supplier(
            &private,
            &input(&private, member, 7, "defend-courier", vec![]),
            &mut |_| {
                draw_index += 1;
                Ok(if draw_index == 1 { 20 } else { 1 })
            },
        )
        .unwrap();
        let wind = stage_with_supplier(
            &combat,
            &input(&combat, member, 8, "second-wind", vec![]),
            &mut |_| Ok(1),
        )
        .unwrap();
        let victory = stage_with_supplier(
            &wind,
            &input(
                &wind,
                member,
                9,
                "greatsword-attack",
                vec![
                    (
                        model::label("savage-attacker").unwrap(),
                        model::label("no").unwrap(),
                    ),
                    (model::label("graze").unwrap(), model::label("no").unwrap()),
                ],
            ),
            &mut |sides| Ok(if sides == 20 { 20 } else { 6 }),
        )
        .unwrap();
        assert!(combat_victory(victory.state()).unwrap());
        assert_eq!(
            value(victory.state(), entity(ENTITIES[0]).unwrap(), "second-wind"),
            Ok(1)
        );
        let command = input(&victory, member, 10, "short-rest", vec![]);
        let rest =
            stage_with_supplier(&victory, &command, &mut |_| panic!("rest cannot draw")).unwrap();
        assert_rest_preserves_battle(&victory, &rest);
        assert_eq!(
            value(rest.state(), entity(ENTITIES[0]).unwrap(), "second-wind"),
            Ok(2)
        );
        assert_eq!(
            value(rest.state(), entity(BANDIT).unwrap(), "unconscious"),
            Ok(0)
        );
        assert_eq!(value(rest.state(), entity(BANDIT).unwrap(), "prone"), Ok(1));
        assert_eq!(
            rest.state().narrative.open_threads,
            vec![model::content(PACKET_THREAD).unwrap()]
        );
        let mut invalid = input(
            &victory,
            member,
            11,
            "short-rest",
            vec![(model::label("elapsed").unwrap(), model::label("1").unwrap())],
        );
        assert!(stage_with_supplier(&victory, &invalid, &mut |_| panic!("invalid rest")).is_err());
        // One rest per aftermath: the second is refused by the machine.
        invalid = input(&rest, member, 11, "short-rest", vec![]);
        assert!(
            stage_with_supplier(&rest, &invalid, &mut |_| panic!("second rest cannot draw"))
                .is_err()
        );
    }

    #[test]
    fn srd_p17_p47_p187_short_rest_waits_for_each_eligible_hero_choice() {
        let loaded = pack::load_from_bytes(
            include_bytes!("fixtures/harbor-journey-pack-integration-baseline.json"),
            include_bytes!("fixtures/harbor-journey-tuning-baseline.json"),
        )
        .unwrap();
        pack::with_test_pack(loaded, short_rest_hit_point_die_choice_flow);
    }

    fn short_rest_hit_point_die_choice_flow() {
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        let victory = gaining_victory();
        let mut state = victory.state().clone();
        let first_hero = player_entity(first, &victory).unwrap();
        let second_hero = player_entity(second, &victory).unwrap();
        set(&mut state, first_hero, "hit-points", 9).unwrap();
        set(&mut state, second_hero, "hit-points", 9).unwrap();
        assert_eq!(
            short_rest_offer_label(&state, first_hero).unwrap(),
            "Short Rest · 1 hour · 1 Hit Point Die to spend"
        );
        set(&mut state, first_hero, "hit-point-dice", 0).unwrap();
        assert_eq!(
            short_rest_offer_label(&state, first_hero).unwrap(),
            "Short Rest · 1 hour · no Hit Point Dice to spend"
        );
        set(&mut state, first_hero, "hit-point-dice", 2).unwrap();
        assert_eq!(
            short_rest_offer_label(&state, first_hero).unwrap(),
            "Short Rest · 1 hour · 2 Hit Point Dice to spend"
        );
        set(&mut state, first_hero, "hit-point-dice", 1).unwrap();
        let victory = model::checkpoint(victory.basis(), state).unwrap();

        let started = stage_with_supplier(
            &victory,
            &input(&victory, first, 20, "short-rest", vec![]),
            &mut |_| panic!("starting the rest does not roll"),
        )
        .unwrap();
        assert_eq!(started.state().logical_time, victory.state().logical_time);
        assert_eq!(value(started.state(), first_hero, "hit-point-dice"), Ok(1));
        assert_eq!(value(started.state(), second_hero, "hit-point-dice"), Ok(1));
        for member in [first, second] {
            let view = wire::journey_view(
                &started,
                df_persistence::local_demo_scope::LocalDemoRole::Player,
                member,
            )
            .unwrap();
            let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
                panic!("player view");
            };
            let offer = player
                .offers
                .iter()
                .find(|offer| offer.action_kind == rpc::GameplayActionKind::ShortRest as i32)
                .expect("eligible hero's Hit Point Die choice");
            assert_eq!(
                offer.label,
                "Short Rest · 1 hour · 1 Hit Point Die to spend"
            );
            assert_eq!(offer.input_groups.len(), 1, "{member:?}");
            assert_eq!(offer.input_groups[0].group_id, "hit-point-die");
            assert_eq!(
                offer.input_groups[0]
                    .options
                    .iter()
                    .map(|option| option.option_id.as_str())
                    .collect::<Vec<_>>(),
                ["spend", "keep"]
            );
        }

        let kept = stage_with_supplier(
            &started,
            &input(
                &started,
                first,
                21,
                "short-rest",
                vec![(
                    model::label("hit-point-die").unwrap(),
                    model::label("keep").unwrap(),
                )],
            ),
            &mut |_| panic!("keeping a die does not roll"),
        )
        .unwrap();
        assert_eq!(kept.state().logical_time, victory.state().logical_time);
        assert_eq!(value(kept.state(), first_hero, "hit-point-dice"), Ok(1));
        assert_eq!(value(kept.state(), first_hero, "short-rest-choice"), Ok(1));
        assert!(
            stage_with_supplier(
                &kept,
                &input(
                    &kept,
                    first,
                    23,
                    "short-rest",
                    vec![(
                        model::label("hit-point-die").unwrap(),
                        model::label("spend").unwrap(),
                    )],
                ),
                &mut |_| panic!("one hero cannot answer twice"),
            )
            .is_err()
        );

        assert!(short_rest_started(kept.state()));
        assert!(hit_point_die_eligible(kept.state(), second_hero).unwrap());
        let spent = stage_with_supplier(
            &kept,
            &input(
                &kept,
                second,
                24,
                "short-rest",
                vec![(
                    model::label("hit-point-die").unwrap(),
                    model::label("spend").unwrap(),
                )],
            ),
            &mut |sides| {
                assert_eq!(sides, rules::LEVEL_ONE_FIGHTER_HIT_POINT_DIE_SIDES);
                Ok(sides)
            },
        )
        .unwrap();
        assert_eq!(
            value(spent.state(), second_hero, "hit-points"),
            Ok(maximum_hit_points(spent.state(), second_hero).unwrap())
        );
        assert_eq!(value(spent.state(), second_hero, "hit-point-dice"), Ok(0));
        assert_eq!(value(spent.state(), first_hero, "hit-point-dice"), Ok(1));
        assert_eq!(
            value(spent.state(), second_hero, "short-rest-die-roll"),
            Ok(10)
        );
        let sheet = character_sheet(spent.state(), second_hero).unwrap();
        assert!(sheet.facts.iter().any(|fact| {
            fact.label == "Hit Point Die roll" && fact.value == "d10 10 + 2 = 12, now 13 of 13"
        }));
        assert!(!session_machine::rest_accepted(&spent).unwrap());
        assert_eq!(
            spent.state().logical_time.ticks,
            victory.state().logical_time.ticks + 3600
        );
        for member in [first, second] {
            let view = wire::journey_view(
                &spent,
                df_persistence::local_demo_scope::LocalDemoRole::Player,
                member,
            )
            .unwrap();
            let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
                panic!("player view");
            };
            assert!(
                player.offers.iter().any(|offer| {
                    offer.action_kind != rpc::GameplayActionKind::ShortRest as i32
                }),
                "the story's own choices must return for {member:?}"
            );
        }
        assert!(
            stage_with_supplier(
                &spent,
                &input(
                    &spent,
                    second,
                    25,
                    "short-rest",
                    vec![(
                        model::label("hit-point-die").unwrap(),
                        model::label("spend").unwrap()
                    )],
                ),
                &mut |_| panic!("a spent die cannot be spent twice"),
            )
            .is_err()
        );
    }

    /// A victory where the first hero has a Second Wind use to regain, so
    /// the aftermath rest is one the machine accepts. The ordinary played
    /// victories leave both heroes untouched.
    pub(super) fn gaining_victory() -> Checkpoint {
        if tactical::enabled() {
            return tactical::tests::current_victory_with_spent_second_wind("ask-courier");
        }
        let victory = inn_victory();
        let mut state = victory.state().clone();
        let hero = state.characters[0].entity;
        set(&mut state, hero, "second-wind", 1).unwrap();
        model::checkpoint(victory.basis(), state).unwrap()
    }

    pub(super) fn inn_victory() -> Checkpoint {
        if tactical::enabled() {
            return tactical::tests::current_victory("ask-courier");
        }
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let opening = opening_story();
        let private = stage_with_supplier(
            &opening,
            &input(&opening, member, 6, "ask-courier", vec![]),
            &mut |_| panic!("no dialogue dice"),
        )
        .unwrap();
        let mut draw_index = 0;
        let combat = stage_with_supplier(
            &private,
            &input(&private, member, 7, "defend-courier", vec![]),
            &mut |_| {
                draw_index += 1;
                Ok(if draw_index == 1 { 20 } else { 1 })
            },
        )
        .unwrap();
        stage_with_supplier(
            &combat,
            &input(
                &combat,
                member,
                8,
                "greatsword-attack",
                vec![
                    (
                        model::label("savage-attacker").unwrap(),
                        model::label("no").unwrap(),
                    ),
                    (model::label("graze").unwrap(), model::label("no").unwrap()),
                ],
            ),
            &mut |sides| Ok(if sides == 20 { 20 } else { 6 }),
        )
        .unwrap()
    }

    fn round_combat() -> Checkpoint {
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let opening = opening_story();
        let dialogue = stage_with_supplier(
            &opening,
            &input(&opening, first, 6, "ask-courier", vec![]),
            &mut |_| panic!("dialogue cannot draw"),
        )
        .unwrap();
        stage_with_supplier(
            &dialogue,
            &input(&dialogue, first, 7, "defend-courier", vec![]),
            &mut |_| Ok(10),
        )
        .unwrap()
    }
    fn wrapped_combat() -> Checkpoint {
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        let combat = round_combat();
        assert_eq!(combat_round(&combat), Ok(1));
        let partial = stage_with_supplier(
            &combat,
            &input(&combat, first, 8, "end-turn", vec![]),
            &mut |_| panic!("first turn has no enemy roll"),
        )
        .unwrap();
        assert_eq!(combat_round(&partial), Ok(1));
        assert_eq!(partial.state().logical_time, combat.state().logical_time);
        let wrapped = stage_with_supplier(
            &partial,
            &input(&partial, second, 9, "end-turn", vec![]),
            &mut |_| Ok(1),
        )
        .unwrap();
        assert_eq!(wrapped.state().logical_time.ticks, 6);
        assert_eq!(combat_round(&wrapped), Ok(2));
        wrapped
    }

    #[test]
    fn actual_combat_refuses_reordered_initiative_before_action_draws() {
        let combat = round_combat();
        assert_eq!(phase(&combat).unwrap(), rpc::JourneyPhase::Combat);
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let mut state = combat.state().clone();
        state.encounters[0].turn_order.swap(0, 1);
        let forged = model::checkpoint(combat.basis(), state).unwrap();
        assert_eq!(phase(&forged), Err(RepositoryError::InvalidCandidate));
        assert!(offered(&forged, first).is_err());
        let command = input(
            &forged,
            first,
            30,
            "greatsword-attack",
            vec![
                (
                    model::label("savage-attacker").unwrap(),
                    model::label("no").unwrap(),
                ),
                (model::label("graze").unwrap(), model::label("no").unwrap()),
            ],
        );
        let before = forged.clone();
        let mut draws = 0;
        assert_eq!(
            stage_with_supplier(&forged, &command, &mut |sides| {
                draws += 1;
                Ok(if sides == 20 { 10 } else { 6 })
            }),
            Err(RepositoryError::InvalidCandidate)
        );
        assert_eq!(draws, 0);
        assert_eq!(forged, before);
    }

    #[test]
    fn actual_victory_refuses_current_resource_contradicting_unrested_outcome() {
        let victory = inn_victory();
        assert_eq!(phase(&victory).unwrap(), rpc::JourneyPhase::Complete);
        let mut state = victory.state().clone();
        set(&mut state, entity(BANDIT).unwrap(), "unconscious", 0).unwrap();
        let forged = model::checkpoint(victory.basis(), state).unwrap();
        assert_eq!(phase(&forged), Err(RepositoryError::InvalidCandidate));
    }

    #[test]
    fn actual_combat_refuses_unsourced_knockout_before_action_draws() {
        let combat = round_combat();
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let mut state = combat.state().clone();
        let other = entity(ENTITIES[1]).unwrap();
        for (resource, value) in [
            ("hit-points", 1),
            ("unconscious", 1),
            ("prone", 1),
            ("held-weapon", 0),
        ] {
            set(&mut state, other, resource, value).unwrap();
        }
        let forged = model::checkpoint(combat.basis(), state).unwrap();
        assert_eq!(phase(&forged), Err(RepositoryError::InvalidCandidate));
        assert!(offered(&forged, first).is_err());
        let command = input(
            &forged,
            first,
            30,
            "greatsword-attack",
            vec![
                (
                    model::label("savage-attacker").unwrap(),
                    model::label("no").unwrap(),
                ),
                (model::label("graze").unwrap(), model::label("no").unwrap()),
            ],
        );
        let before = forged.clone();
        let mut draws = 0;
        assert_eq!(
            stage_with_supplier(&forged, &command, &mut |_| {
                draws += 1;
                Ok(10)
            }),
            Err(RepositoryError::InvalidCandidate)
        );
        assert_eq!(draws, 0);
        assert_eq!(forged, before);
    }
    fn round_view(current: &Checkpoint, member: MemberId, display: bool) -> rpc::ViewMessage {
        use df_persistence::local_demo_scope::LocalDemoRole;
        wire::journey_view(
            current,
            if display {
                LocalDemoRole::Display
            } else {
                LocalDemoRole::Player
            },
            member,
        )
        .unwrap()
    }
    fn assert_round_views(current: &Checkpoint, expected: u32) {
        let original = current.clone();
        for (member, display) in [
            (MemberId::from_bytes(&MEMBERS[0]).unwrap(), false),
            (MemberId::from_bytes(&MEMBERS[1]).unwrap(), false),
            (bootstrap_member().unwrap(), true),
        ] {
            let view = round_view(current, member, display);
            let decoded = rpc::ViewMessage::decode(view.encode_to_vec().as_slice()).unwrap();
            assert_eq!(decoded, view);
            let journey = match decoded.audience.unwrap() {
                rpc::view_message::Audience::Player(view) => view.journey.unwrap(),
                rpc::view_message::Audience::Display(view) => view.journey.unwrap(),
            };
            assert_eq!(journey.combat.unwrap().round, expected);
        }
        assert_eq!(
            current, &original,
            "projection cannot mutate any canonical record"
        );
    }
    #[test]
    fn encounter_round_one_survives_real_rest_inn_and_restored_checkpoint() {
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let victory = gaining_victory();
        assert_round_views(&victory, 1);
        let command = input(&victory, first, 9, "short-rest", vec![]);
        let rest = stage_with_supplier(&victory, &command, &mut |_| panic!("rest dice")).unwrap();
        let retry = stage_with_supplier(&victory, &command, &mut |_| panic!("retry dice")).unwrap();
        assert_eq!(rest, retry);
        assert_rest_preserves_battle(&victory, &rest);
        assert_eq!(rest.state().logical_time.ticks, 3600);
        assert_round_views(&rest, 1);
        let arrived = stage_with_supplier(&rest, &inn_command(&rest, first, 10), &mut |_| {
            panic!("Inn dice")
        })
        .unwrap();
        inn_preserves(&rest, &arrived);
        assert_round_views(&arrived, 1);
        let restored = model::checkpoint(arrived.basis(), arrived.state().clone()).unwrap();
        assert_eq!(restored, arrived);
        assert_round_views(&restored, 1);
    }
    #[test]
    fn encounter_round_two_counts_only_real_wrap_and_survives_rest_inn_retry() {
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let wrapped = wrapped_combat();
        assert_round_views(&wrapped, 2);
        let command = input(
            &wrapped,
            first,
            10,
            "greatsword-attack",
            vec![
                (
                    model::label("savage-attacker").unwrap(),
                    model::label("no").unwrap(),
                ),
                (model::label("graze").unwrap(), model::label("no").unwrap()),
            ],
        );
        let victory = stage_with_supplier(&wrapped, &command, &mut |sides| {
            Ok(if sides == 20 { 20 } else { 6 })
        })
        .unwrap();
        let retry = stage_with_supplier(&wrapped, &command, &mut |sides| {
            Ok(if sides == 20 { 20 } else { 6 })
        })
        .unwrap();
        assert_eq!(victory, retry);
        assert!(combat_victory(victory.state()).unwrap());
        // The wrapped fight leaves both heroes untouched, so spend a
        // Second Wind use on paper for the rest to regain.
        let mut spent = victory.state().clone();
        let hero = spent.characters[0].entity;
        set(&mut spent, hero, "second-wind", 1).unwrap();
        let victory = model::checkpoint(victory.basis(), spent).unwrap();
        let rest = stage_with_supplier(
            &victory,
            &input(&victory, first, 11, "short-rest", vec![]),
            &mut |_| panic!("rest dice"),
        )
        .unwrap();
        assert_rest_preserves_battle(&victory, &rest);
        assert_eq!(rest.state().logical_time.ticks, 3606);
        let arrived = stage_with_supplier(&rest, &inn_command(&rest, first, 12), &mut |_| {
            panic!("Inn dice")
        })
        .unwrap();
        inn_preserves(&rest, &arrived);
        for current in [&victory, &rest, &arrived] {
            assert_round_views(current, 2);
        }
        let restored = model::checkpoint(arrived.basis(), arrived.state().clone()).unwrap();
        assert_round_views(&restored, 2);
        assert_eq!(restored, arrived);
    }

    #[test]
    fn encounter_round_wrap_with_monster_between_heroes_needs_no_attack_or_cause_fact() {
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        let opening = opening_story();
        let dialogue = stage_with_supplier(
            &opening,
            &input(&opening, first, 6, "escort-courier", vec![]),
            &mut |_| panic!("dialogue"),
        )
        .unwrap();
        let mut dice = [20, 1, 10].into_iter();
        let combat = stage_with_supplier(
            &dialogue,
            &input(&dialogue, first, 7, "defend-courier", vec![]),
            &mut |_| Ok(dice.next().expect("exact initiative draws")),
        )
        .unwrap();
        assert_eq!(
            combat.state().encounters[0].turn_order,
            vec![
                entity(ENTITIES[0]).unwrap(),
                entity(BANDIT).unwrap(),
                entity(ENTITIES[1]).unwrap()
            ]
        );
        let partial = stage_with_supplier(
            &combat,
            &input(&combat, first, 8, "end-turn", vec![]),
            &mut |_| Ok(1),
        )
        .unwrap();
        assert_eq!(partial.state().logical_time.ticks, 0);
        assert_round_views(&partial, 1);
        let command = input(&partial, second, 9, "end-turn", vec![]);
        let wrapped =
            stage_with_supplier(&partial, &command, &mut |_| panic!("no monster turn here"))
                .unwrap();
        let retry =
            stage_with_supplier(&partial, &command, &mut |_| panic!("no retry draw")).unwrap();
        assert_eq!(wrapped, retry);
        assert_eq!(wrapped.state().logical_time.ticks, 6);
        let decision = wrapped.state().decisions.last().unwrap();
        assert!(decision.draws.is_empty());
        assert!(accepted(decision).unwrap().combat.is_empty());
        let time = wrapped
            .state()
            .facts
            .iter()
            .find(|fact| {
                fact.operation == decision.operation
                    && matches!(fact.value, FactValue::TimeAdvanced { .. })
            })
            .unwrap();
        assert_eq!(time.ordinal, 0);
        assert_eq!(time.cause, None);
        assert_round_views(&wrapped, 2);
        let restored = model::checkpoint(wrapped.basis(), wrapped.state().clone()).unwrap();
        assert_eq!(restored, wrapped);
        assert_round_views(&restored, 2);
    }

    #[test]
    fn encounter_round_counts_wrap_before_same_operation_knockout_start_once() {
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        let opening = opening_story();
        let dialogue = stage_with_supplier(
            &opening,
            &input(&opening, first, 6, "escort-courier", vec![]),
            &mut |_| panic!("dialogue"),
        )
        .unwrap();
        let mut dice = [1, 1, 20, 1].into_iter();
        let combat = stage_with_supplier(
            &dialogue,
            &input(&dialogue, first, 7, "defend-courier", vec![]),
            &mut |_| Ok(dice.next().expect("initiative and initial monster miss")),
        )
        .unwrap();
        assert_eq!(
            combat.state().encounters[0].turn_order,
            vec![
                entity(BANDIT).unwrap(),
                entity(ENTITIES[0]).unwrap(),
                entity(ENTITIES[1]).unwrap()
            ]
        );
        assert_eq!(
            combat.state().encounters[0].active_turn,
            Some(entity(ENTITIES[0]).unwrap())
        );
        let partial = stage_with_supplier(
            &combat,
            &input(&combat, first, 8, "end-turn", vec![]),
            &mut |_| panic!("next player, no monster roll"),
        )
        .unwrap();
        let wrapped = stage_with_supplier(
            &partial,
            &input(&partial, second, 9, "end-turn", vec![]),
            &mut |sides| Ok(if sides == 20 { 20 } else { 6 }),
        )
        .unwrap();
        assert_eq!(
            value(wrapped.state(), entity(ENTITIES[0]).unwrap(), "unconscious"),
            Ok(1)
        );
        assert_eq!(wrapped.state().logical_time.ticks, 6);
        let operation = wrapped.state().decisions.last().unwrap().operation;
        let facts = wrapped
            .state()
            .facts
            .iter()
            .filter(|fact| fact.operation == operation)
            .collect::<Vec<_>>();
        let times = facts
            .iter()
            .filter(|fact| matches!(fact.value, FactValue::TimeAdvanced { .. }))
            .collect::<Vec<_>>();
        assert_eq!(times.len(), 1);
        assert!(
            matches!(times[0].value, FactValue::TimeAdvanced { before, after }
            if before.ticks == 0 && after.ticks == 6)
        );
        let knockout = facts.iter().find(|fact| matches!(&fact.value,
            FactValue::ContentEvent { definition, .. } if definition.entry.as_str() == "knockout-short-rest-start")).unwrap();
        assert!(times[0].ordinal < knockout.ordinal);
        assert!(knockout.ordinal < facts.last().unwrap().ordinal);
        assert_round_views(&wrapped, 2);
        assert_eq!(
            wrapped.state().encounters[0].active_turn,
            Some(entity(ENTITIES[1]).unwrap())
        );
    }

    #[test]
    fn encounter_round_skips_down_actor_without_inventing_a_wrap() {
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        let opening = opening_story();
        let dialogue = stage_with_supplier(
            &opening,
            &input(&opening, first, 6, "escort-courier", vec![]),
            &mut |_| panic!("dialogue"),
        )
        .unwrap();
        let mut ordinal = 0;
        let combat = stage_with_supplier(
            &dialogue,
            &input(&dialogue, first, 7, "defend-courier", vec![]),
            &mut |sides| {
                ordinal += 1;
                Ok(if ordinal <= 2 {
                    1
                } else if sides == 20 {
                    20
                } else {
                    6
                })
            },
        )
        .unwrap();
        assert_eq!(
            value(combat.state(), entity(ENTITIES[0]).unwrap(), "unconscious"),
            Ok(1)
        );
        assert_eq!(
            combat.state().encounters[0].active_turn,
            Some(entity(ENTITIES[1]).unwrap())
        );
        assert_eq!(combat.state().logical_time.ticks, 0);
        assert_round_views(&combat, 1);
        let wrapped = stage_with_supplier(
            &combat,
            &input(&combat, second, 8, "end-turn", vec![]),
            &mut |_| Ok(1),
        )
        .unwrap();
        assert_eq!(wrapped.state().logical_time.ticks, 6);
        assert_eq!(
            wrapped.state().encounters[0].active_turn,
            Some(entity(ENTITIES[1]).unwrap())
        );
        assert_round_views(&wrapped, 2);
    }
    #[test]
    fn encounter_round_refuses_forged_time_source_units_duration_and_start_without_partial_state() {
        use CombatRoundError as Error;
        let current = wrapped_combat();
        let untouched = current.clone();
        for (case, expected) in [
            (0, Error::Source),
            (1, Error::Source),
            (2, Error::Units),
            (3, Error::Duration),
            (4, Error::Source),
            (5, Error::Start),
            (6, Error::Encounter),
            (7, Error::Chronology),
        ] {
            let mut state = current.state().clone();
            let time_index = state
                .facts
                .iter()
                .position(|fact| matches!(fact.value, FactValue::TimeAdvanced { .. }))
                .unwrap();
            let operation = state.facts[time_index].operation;
            match case {
                0 => {
                    state
                        .decisions
                        .iter_mut()
                        .find(|decision| decision.operation == operation)
                        .unwrap()
                        .source_policy = model::label("foreign-policy").unwrap()
                }
                1 => {
                    let terminal = state
                        .decisions
                        .iter()
                        .find(|decision| decision.operation == operation)
                        .unwrap()
                        .facts
                        .last()
                        .copied()
                        .unwrap();
                    let FactValue::ContentEvent { definition, .. } = &mut state
                        .facts
                        .iter_mut()
                        .find(|fact| fact.id == terminal)
                        .unwrap()
                        .value
                    else {
                        panic!("event")
                    };
                    *definition = model::content("escort-courier").unwrap();
                }
                2 | 3 => {
                    let FactValue::TimeAdvanced { before, after } =
                        &mut state.facts[time_index].value
                    else {
                        panic!("time")
                    };
                    if case == 2 {
                        before.ticks_per_second = 2;
                        after.ticks_per_second = 2;
                    } else {
                        after.ticks = 5;
                    }
                }
                4 => state.facts[time_index].cause = None,
                5 => {
                    let fact = state.facts.iter_mut().find(|fact| matches!(&fact.value,
                        FactValue::ContentEvent { definition, .. } if definition.entry.as_str() == "defend-courier")).unwrap();
                    let FactValue::ContentEvent { definition, .. } = &mut fact.value else {
                        panic!("start")
                    };
                    *definition = model::content("escort-courier").unwrap();
                }
                6 => state.encounters[0].turn_order.swap(0, 1),
                7 => state.logical_time.ticks = 600,
                _ => unreachable!(),
            }
            let candidate = model::checkpoint(current.basis(), state).unwrap();
            let before = candidate.clone();
            assert_eq!(combat_round(&candidate), Err(expected), "case {case}");
            assert_eq!(candidate, before);
        }
        assert_eq!(current, untouched);
        let mut stale_pins = current.pins().clone();
        stale_pins.content.package_digest = ContentDigest([99; 32]);
        let stale = Checkpoint::new(
            current.schema(),
            current.basis(),
            stale_pins,
            current.state().clone(),
            ReferenceInventory {
                rules: &[model::rule().unwrap(), rule().unwrap()],
                content: &model::contents().unwrap(),
                resources: &resources_for(current.state()).unwrap(),
                assets: &[],
            },
            model::limits(),
        )
        .unwrap();
        assert_eq!(combat_round(&stale), Err(Error::Binding));
        let mut unowned = current.state().clone();
        let time = unowned
            .facts
            .iter_mut()
            .find(|fact| matches!(fact.value, FactValue::TimeAdvanced { .. }))
            .unwrap();
        time.operation = OperationId::from_bytes(&[99; 16]).unwrap();
        assert!(
            model::checkpoint(current.basis(), unowned).is_err(),
            "canonical ownership rejects unowned time before projection"
        );
    }

    pub(super) fn inn_command(current: &Checkpoint, member: MemberId, operation: u8) -> GameInput {
        input(
            current,
            member,
            operation,
            "choose-harbor-scene",
            vec![(
                model::label("harbor-destination").unwrap(),
                model::label("harbor-inn").unwrap(),
            )],
        )
    }
    fn inn_preserves(before: &Checkpoint, after: &Checkpoint) {
        assert_eq!(
            after.basis().revision,
            before.basis().revision.next_sequence().unwrap()
        );
        assert_eq!(after.pins(), before.pins());
        let mut comparable = after.state().clone();
        comparable.facts = before.state().facts.clone();
        comparable.decisions = before.state().decisions.clone();
        comparable.narrative = before.state().narrative.clone();
        assert_eq!(
            &comparable,
            before.state(),
            "all mechanical, world, private and sibling records remain exact"
        );
        assert_eq!(after.state().facts.len(), before.state().facts.len() + 1);
        assert_eq!(
            after.state().decisions.len(),
            before.state().decisions.len() + 1
        );
        let event = after.state().facts.last().unwrap();
        assert_eq!(event.cause, inn_source(before).unwrap());
        assert_eq!(event.audience, AudienceScope::Shared);
        assert!(
            matches!(&event.value, FactValue::ContentEvent { definition, subjects }
            if *definition == model::content("harbor-inn").unwrap()
                && *subjects == participants(before).into_iter().map(|link| link.character.unwrap()).collect::<Vec<_>>())
        );
        assert_eq!(
            after.state().narrative.active_beats,
            vec![model::content("harbor-inn").unwrap()]
        );
        assert_eq!(
            after.state().narrative.open_threads,
            before.state().narrative.open_threads
        );
        let mut expected_narrative = before.state().narrative.clone();
        expected_narrative
            .completed_beats
            .extend(expected_narrative.active_beats.clone());
        expected_narrative.active_beats = vec![model::content("harbor-inn").unwrap()];
        expected_narrative.accepted_facts.push(event.id);
        assert_eq!(after.state().narrative, expected_narrative);
        let decision = after.state().decisions.last().unwrap();
        assert_eq!(decision.source_policy.as_str(), THREAD_POLICY);
        assert_eq!(decision.facts, vec![event.id]);
        assert!(decision.draws.is_empty() && decision.effects.is_empty());
        assert_eq!(accepted(decision).unwrap().scene, Some(inn_scene()));
    }
    #[test]
    fn actual_victory_inn_is_voluntary_for_either_member_before_or_after_rest_without_mechanical_effects()
     {
        let victory = gaining_victory();
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let rest = stage_with_supplier(
            &victory,
            &input(&victory, first, 9, "short-rest", vec![]),
            &mut |_| panic!("rest dice"),
        )
        .unwrap();
        for current in [&victory, &rest] {
            for bytes in MEMBERS.into_iter().take(2) {
                let member = MemberId::from_bytes(&bytes).unwrap();
                let offers = offered(current, member).unwrap();
                // The rest is offered once per aftermath: at the victory,
                // where a hero has a use to regain, and not again after it.
                assert_eq!(
                    offers
                        .iter()
                        .any(|(kind, _)| *kind == rpc::GameplayActionKind::ShortRest),
                    current == &victory
                );
                assert!(
                    offers
                        .iter()
                        .any(|(kind, _)| *kind == rpc::GameplayActionKind::ChooseHarborScene)
                );
                let before = current.clone();
                let command = inn_command(current, member, 10);
                let arrived =
                    stage_with_supplier(current, &command, &mut |_| panic!("scene cannot draw"))
                        .unwrap();
                inn_preserves(current, &arrived);
                assert_eq!(current, &before);
                assert!(
                    !offered(&arrived, member)
                        .unwrap()
                        .iter()
                        .any(|(kind, _)| *kind == rpc::GameplayActionKind::ChooseHarborScene)
                );
                assert!(
                    stage_with_supplier(
                        &arrived,
                        &inn_command(&arrived, member, 11),
                        &mut |_| panic!("duplicate scene")
                    )
                    .is_err()
                );
                assert_eq!(
                    stage_with_supplier(current, &command, &mut |_| panic!(
                        "same supplied transition"
                    ))
                    .unwrap(),
                    arrived
                );
            }
        }
        assert_ne!(
            offer_id(&victory, rpc::GameplayActionKind::ChooseHarborScene),
            offer_id(&rest, rpc::GameplayActionKind::ChooseHarborScene)
        );
    }
    #[test]
    fn inn_refuses_unfinished_defeated_incapacitated_and_missing_victory_source_without_candidate_escape()
     {
        let victory = inn_victory();
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let opening = opening_story();
        assert!(
            stage_with_supplier(
                &opening,
                &inn_command(&opening, first, 20),
                &mut |_| panic!("unfinished")
            )
            .is_err()
        );
        let mut defeated = victory.state().clone();
        defeated.encounters[0].objectives = vec![model::content("combat-defeat").unwrap()];
        let defeated = model::checkpoint(victory.basis(), defeated).unwrap();
        let mut no_source = victory.state().clone();
        let op = inn_source(&victory).unwrap().unwrap();
        let source_operation = no_source
            .facts
            .iter()
            .find(|fact| fact.id == op)
            .unwrap()
            .operation;
        no_source
            .decisions
            .iter_mut()
            .find(|decision| decision.operation == source_operation)
            .unwrap()
            .source_policy = model::label("foreign-invitation-policy").unwrap();
        let no_source = model::checkpoint(victory.basis(), no_source).unwrap();
        let mut incapacitated = victory.state().clone();
        set(
            &mut incapacitated,
            player_entity(first, &victory).unwrap(),
            "unconscious",
            1,
        )
        .unwrap();
        // This is a canonical negative input, not an admitted unconscious rules transition.
        let incapacitated = model::checkpoint(victory.basis(), incapacitated).unwrap();
        for current in [&defeated, &no_source, &incapacitated] {
            let before = current.clone();
            assert!(!offered(current, first).is_ok_and(|offers| {
                offers
                    .iter()
                    .any(|(kind, _)| *kind == rpc::GameplayActionKind::ChooseHarborScene)
            }));
            assert!(
                stage_with_supplier(current, &inn_command(current, first, 21), &mut |_| panic!(
                    "invalid scene"
                ))
                .is_err()
            );
            assert_eq!(current, &before);
        }
    }
    #[test]
    fn inn_generated_destination_rejects_forgery_extra_choices_wrong_actor_stale_basis_and_source_pins()
     {
        let victory = inn_victory();
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let basis = victory.basis();
        let mut request = rpc::SubmitActionRequest {
            session_id: Some(rpc::SessionId {
                value: Some(basis.session.as_bytes().to_vec()),
            }),
            run_id: Some(rpc::RunId {
                value: Some(basis.run.as_bytes().to_vec()),
            }),
            observed_revision: Some(wire::revision(basis.revision)),
            operation_id: Some(rpc::OperationId {
                value: Some(vec![30; 16]),
            }),
            offer_id: offer_id(&victory, rpc::GameplayActionKind::ChooseHarborScene),
            action_kind: rpc::GameplayActionKind::ChooseHarborScene as i32,
            destination: rpc::HarborDestination::HarborInn as i32,
            ..Default::default()
        };
        assert!(!wire::unrelated_payload(
            &request,
            rpc::GameplayActionKind::ChooseHarborScene
        ));
        let exact = wire::journey_input(&request, member, &victory).unwrap();
        let arrived = stage_with_supplier(&victory, &exact, &mut |_| panic!("scene dice")).unwrap();
        inn_preserves(&victory, &arrived);
        for destination in [0, 1, 2, 99] {
            request.destination = destination;
            assert!(wire::unrelated_payload(
                &request,
                rpc::GameplayActionKind::ChooseHarborScene
            ));
            let forged = wire::journey_input(&request, member, &victory).unwrap();
            assert!(
                stage_with_supplier(&victory, &forged, &mut |_| panic!("forged destination"))
                    .is_err()
            );
        }
        let mut extra = exact.clone();
        if let GameInput::Game(CommandInput {
            command: GameCommand::ProposeAction { choices, .. },
            ..
        }) = &mut extra
        {
            choices.push((model::label("elapsed").unwrap(), model::label("1").unwrap()));
        }
        let mut wrong_actor = exact.clone();
        if let GameInput::Game(CommandInput {
            command: GameCommand::ProposeAction { actor, .. },
            ..
        }) = &mut wrong_actor
        {
            *actor = entity(ENTITIES[1]).unwrap();
        }
        for invalid in [extra, wrong_actor] {
            assert!(
                stage_with_supplier(&victory, &invalid, &mut |_| panic!("invalid binding"))
                    .is_err()
            );
        }
        assert!(stage_with_supplier(&arrived, &exact, &mut |_| panic!("stale basis")).is_err());
        let mut pins = victory.pins().clone();
        pins.content.package_digest = ContentDigest([0x90; 32]);
        let stale_source = Checkpoint::new(
            CHECKPOINT_SCHEMA,
            victory.basis(),
            pins,
            victory.state().clone(),
            ReferenceInventory {
                rules: &[model::rule().unwrap(), rule().unwrap()],
                content: &model::contents().unwrap(),
                resources: &resources_for(victory.state()).unwrap(),
                assets: &[],
            },
            model::limits(),
        )
        .unwrap();
        assert!(
            stage_with_supplier(
                &stale_source,
                &inn_command(&stale_source, member, 31),
                &mut |_| panic!("stale source")
            )
            .is_err()
        );
        assert_eq!(
            victory.state().narrative.active_beats,
            vec![model::content("complete").unwrap()]
        );
    }
    #[test]
    fn inn_last_constructor_capacity_and_due_world_pending_leave_original_checkpoint_exact() {
        let victory = inn_victory();
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let mut state = victory.state().clone();
        let mut full = victory.clone();
        for _ in 0..512 {
            state
                .narrative
                .completed_beats
                .push(model::content("opening").unwrap());
            match model::checkpoint(victory.basis(), state.clone()) {
                Ok(candidate) => full = candidate,
                Err(_) => break,
            }
        }
        assert!(
            full.state().narrative.completed_beats.len()
                > victory.state().narrative.completed_beats.len()
        );
        let before = full.clone();
        assert!(
            stage_with_supplier(&full, &inn_command(&full, member, 40), &mut |_| panic!(
                "capacity"
            ))
            .is_err()
        );
        assert_eq!(full, before);
        let mut due_state = victory.state().clone();
        due_state.logical_time = due_state.schedules[0].due;
        let due = model::checkpoint(victory.basis(), due_state).unwrap();
        let before = due.clone();
        assert!(
            stage_with_supplier(&due, &inn_command(&due, member, 41), &mut |_| panic!(
                "World prerequisite"
            ))
            .is_err()
        );
        assert_eq!(due, before);
    }
    #[test]
    fn inn_safe_views_keep_actual_party_and_private_courier_records_with_only_server_destination_offers()
     {
        use df_persistence::local_demo_scope::LocalDemoRole;
        let victory = inn_victory();
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        let offered_view = wire::journey_view(&victory, LocalDemoRole::Player, first).unwrap();
        let Some(rpc::view_message::Audience::Player(view)) = offered_view.audience else {
            panic!("player");
        };
        let offer = view
            .offers
            .iter()
            .find(|offer| offer.action_kind == rpc::GameplayActionKind::ChooseHarborScene as i32)
            .unwrap();
        assert_eq!(
            offer.destinations,
            vec![rpc::HarborSceneOption {
                destination: rpc::HarborDestination::HarborInn as i32,
                label: "Harbor Inn".to_owned()
            }]
        );
        let arrived =
            stage_with_supplier(&victory, &inn_command(&victory, second, 50), &mut |_| {
                panic!("scene")
            })
            .unwrap();
        inn_preserves(&victory, &arrived);
        // 613839eb added the declared stage floor at the wire projection boundary.
        let mut expected_scene = inn_scene();
        expected_scene.floor = Some(rpc::StageFloor {
            feet_mid: 90,
            feet_far: 78,
            key_from_left: false,
            wet: true,
        });
        for (role, principal) in [
            (LocalDemoRole::Player, first),
            (LocalDemoRole::Player, second),
            (LocalDemoRole::Display, bootstrap_member().unwrap()),
        ] {
            let projected = wire::journey_view(&arrived, role, principal).unwrap();
            match projected.audience.unwrap() {
                rpc::view_message::Audience::Player(view) => {
                    assert_eq!(view.scene, Some(expected_scene.clone()));
                    assert_eq!(view.journey.unwrap().party.len(), 2);
                    assert!(!view.narration.contains("Vell"));
                    assert!(!view.offers.iter().any(|offer| offer.action_kind
                        == rpc::GameplayActionKind::ChooseHarborScene as i32));
                }
                rpc::view_message::Audience::Display(view) => {
                    assert_eq!(view.scene, Some(expected_scene.clone()));
                    assert!(!view.narration.contains("Vell"));
                }
            }
        }
        assert!(arrived.state().facts.iter().any(|fact| matches!(&fact.value, FactValue::ContentEvent { definition, .. } if definition.entry.as_str() == "private-courier-note") && fact.audience == AudienceScope::Members(vec![first])));
    }

    #[test]
    fn post_rest_inn_preserves_real_cursor_and_refuses_forged_policy_time_or_pending_queue() {
        let victory = gaining_victory();
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let rested = stage_with_supplier(
            &victory,
            &input(&victory, member, 60, "short-rest", vec![]),
            &mut |_| panic!("rest dice"),
        )
        .unwrap();
        let cursor = rested.state().continuity.catch_up.as_ref().unwrap();
        assert_eq!(cursor.policy, model::content("short-rest").unwrap());
        let arrived = stage_with_supplier(&rested, &inn_command(&rested, member, 61), &mut |_| {
            panic!("scene dice")
        })
        .unwrap();
        inn_preserves(&rested, &arrived);
        assert_eq!(arrived.state().continuity.catch_up.as_ref(), Some(cursor));
        for variant in 0..3 {
            let mut state = rested.state().clone();
            let cursor = state.continuity.catch_up.as_mut().unwrap();
            match variant {
                0 => cursor.policy = model::content("harbor-inn").unwrap(),
                1 => cursor.time.ticks += 1,
                _ => cursor.pending_events.push(state.schedules[0].id),
            }
            let forged = model::checkpoint(rested.basis(), state).unwrap();
            assert_eq!(
                df_world::select_due_events(
                    &forged,
                    forged.pins(),
                    df_world::DueSelectionRequest {
                        expected_basis: forged.basis(),
                        target_time: forged.state().logical_time,
                        paused: false,
                        deadline_remaining: Duration::from_secs(1),
                        policy: &model::content("short-rest").unwrap(),
                    },
                    df_world::DueSelectionLimits {
                        queue_events: 8,
                        selected_events: 8,
                        output_bytes: 65536
                    },
                ),
                Err(df_world::DueSelectionError::StaleCursor)
            );
            let before = forged.clone();
            assert!(
                stage_with_supplier(&forged, &inn_command(&forged, member, 62), &mut |_| panic!(
                    "forged cursor"
                ))
                .is_err(),
                "variant {variant} must refuse the forged cursor"
            );
            assert_eq!(forged, before);
        }
    }
    #[test]
    fn campaign_terminal_policy_waits_for_destination_and_retains_actual_secret_holder_and_rest() {
        let completion = super::super::adventure::campaign_completion::terminal_outcome;
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let victory = gaining_victory();
        assert!(completion(&victory).unwrap().is_none());
        let arrived = stage_with_supplier(&victory, &inn_command(&victory, first, 90), &mut |_| {
            panic!("inn dice")
        })
        .unwrap();
        let outcome = completion(&arrived).unwrap().unwrap();
        assert_eq!(outcome.member, hex(first.as_bytes()));
        assert_eq!(
            outcome.approach,
            super::super::adventure::campaign::Approach::Asked
        );
        assert!(!outcome.rested);
        assert!(outcome.destination_reached && outcome.antagonist_spared);
        let rested = stage_with_supplier(
            &victory,
            &input(&victory, first, 91, "short-rest", vec![]),
            &mut |_| panic!("rest dice"),
        )
        .unwrap();
        assert!(completion(&rested).unwrap().is_none());
        let arrived = stage_with_supplier(&rested, &inn_command(&rested, first, 92), &mut |_| {
            panic!("inn dice")
        })
        .unwrap();
        assert!(completion(&arrived).unwrap().unwrap().rested);
        let escorted = tuning_journey();
        let outcome = completion(escorted.last().unwrap()).unwrap().unwrap();
        assert_eq!(
            outcome.approach,
            super::super::adventure::campaign::Approach::Escorted
        );
        assert_eq!(outcome.member, hex(first.as_bytes()));
        assert!(outcome.rested && outcome.antagonist_spared);
    }
    #[test]
    fn campaign_completion_worker_reconciles_restart_replays_without_rewrite_and_storage_error_is_nonfatal()
     {
        use super::super::adventure::{
            GenerateOptions,
            campaign::{CampaignRecord, digest, generate_frame, next_chapter},
            campaign_completion::worker,
            campaign_render::generate_chapter,
            campaign_startup::CampaignRun,
            campaign_store::CampaignStore,
            campaign_tables::CampaignTables,
        };
        let project = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let root = project.join("artifacts/tmp/play-g4").join(format!(
            "worker-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let tables = CampaignTables::load(&project.join("content/adventure-tables")).unwrap();
        let mut record =
            CampaignRecord::new(generate_frame(&tables, 42, GenerateOptions::default()).unwrap())
                .unwrap();
        let mut plan = next_chapter(&tables, &record.frame, &record).unwrap();
        let mut generated = generate_chapter(&tables, &record.frame, &mut plan).unwrap();
        generated.pack.campaign = Some(pack::CampaignIdentity {
            id: "test".into(),
            chapter: 1,
            frame_sha256: record.frame_digest.clone(),
        });
        let documents = generated.documents(true).unwrap();
        let (loaded, _) = super::super::adventure::record::reload(&documents).unwrap();
        record
            .begin(
                plan,
                "df_gameplay_demo_test_ch01".into(),
                digest(&documents.record),
            )
            .unwrap();
        let store = CampaignStore::new(&root, "test").unwrap();
        store.save(&record, None).unwrap();
        let arrived = pack::with_test_pack(loaded.clone(), || {
            let victory = inn_victory();
            let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
            stage_with_supplier(&victory, &inn_command(&victory, first, 93), &mut |_| {
                panic!("inn dice")
            })
            .unwrap()
        });
        pack::with_test_pack(loaded, || {
            let run = CampaignRun {
                root: root.clone(),
                id: "test".into(),
                number: 1,
                frame_digest: record.frame_digest.clone(),
            };
            // Simulate failure after the authoritative game commit, then reconciliation
            // of that same checkpoint on restart. Holding the OS lock cannot block observe.
            let lock = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(root.join("development/campaigns/test/write.lock"))
                .unwrap();
            lock.try_lock().unwrap();
            let (observer, background) = worker(run.clone()).unwrap();
            observer.observe(&arrived);
            drop(background);
            assert_eq!(
                arrived.state().narrative.active_beats,
                [model::content("harbor-inn").unwrap()]
            );
            drop(lock);
            assert!(
                store.load().unwrap().unwrap().record.chapters[0]
                    .outcome
                    .is_none()
            );
            let (observer, background) = worker(run.clone()).unwrap();
            observer.observe(&arrived);
            drop(background);
            let stored = store.load().unwrap().unwrap();
            assert!(stored.record.chapters[0].outcome.is_some());
            let before =
                std::fs::read(root.join("development/campaigns/test/campaign.json")).unwrap();
            let (observer, background) = worker(run).unwrap();
            observer.observe(&arrived);
            drop(background);
            assert_eq!(
                std::fs::read(root.join("development/campaigns/test/campaign.json")).unwrap(),
                before
            );
            let mut record = stored.record;
            let mut conflicting = record.chapters[0].outcome.clone().unwrap();
            conflicting.rested = !conflicting.rested;
            assert!(record.complete(1, conflicting).is_err());
            std::fs::remove_dir_all(root).unwrap();
        });
    }
    #[test]
    fn campaign_finale_projects_bounded_branch_endings_and_preserves_defeat_mechanics() {
        use super::super::adventure::{
            GenerateOptions,
            campaign::{
                Approach, CampaignRecord, ChapterOutcome, ChapterResult, generate_frame,
                next_chapter,
            },
            campaign_render::generate_chapter,
            campaign_tables::CampaignTables,
        };
        let tables = CampaignTables::load(
            &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../content/adventure-tables"),
        )
        .unwrap();
        for desperate in [false, true] {
            let frame = generate_frame(&tables, 42, GenerateOptions::default()).unwrap();
            let mut record = CampaignRecord::new(frame.clone()).unwrap();
            let mut final_plan = loop {
                let mut plan = next_chapter(&tables, &frame, &record).unwrap();
                if plan.chapter_type == "finale" {
                    break plan;
                }
                generate_chapter(&tables, &frame, &mut plan).unwrap();
                let n = plan.number;
                record
                    .begin(plan, "synthetic".into(), "digest".into())
                    .unwrap();
                record
                    .complete(
                        n,
                        ChapterOutcome {
                            result: if desperate {
                                ChapterResult::Defeat
                            } else {
                                ChapterResult::Victory
                            },
                            approach: Approach::Escorted,
                            member: "hero".into(),
                            rested: true,
                            destination_reached: !desperate,
                            antagonist_spared: false,
                            campaign_time: u64::from(n) * 3600,
                        },
                    )
                    .unwrap();
            };
            assert_eq!(final_plan.desperate, desperate);
            let generated = generate_chapter(&tables, &frame, &mut final_plan).unwrap();
            let ending = generated.pack.campaign_epilogue.clone().unwrap();
            let documents = generated.documents(true).unwrap();
            let (loaded, _) = super::super::adventure::record::reload(&documents).unwrap();
            pack::with_test_pack(loaded, || {
                let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
                for (victory, expected) in [
                    (inn_victory(), ending.asked.as_str()),
                    (
                        tuning_journey().into_iter().rev().nth(1).unwrap(),
                        ending.escorted.as_str(),
                    ),
                ] {
                    let arrived = stage_with_supplier(
                        &victory,
                        &inn_command(&victory, first, 94),
                        &mut |_| panic!("destination dice"),
                    )
                    .unwrap();
                    let projected = wire::journey_view(
                        &arrived,
                        df_persistence::local_demo_scope::LocalDemoRole::Player,
                        first,
                    )
                    .unwrap();
                    let rpc::view_message::Audience::Player(view) = projected.audience.unwrap()
                    else {
                        panic!("player projection");
                    };
                    assert_eq!(view.narration, expected);
                    assert!(view.narration.chars().count() <= 220);
                    assert!(
                        view.narration.contains(
                            serde_json::to_value(&frame).unwrap()["artifact"]["name"]
                                .as_str()
                                .unwrap()
                        )
                    );
                    assert!(
                        !view.narration.contains(
                            serde_json::to_value(&frame).unwrap()["villain"]["name"]
                                .as_str()
                                .unwrap()
                        )
                    );
                }
                for approach in ["ask-courier", "escort-courier"] {
                    let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
                    let defeated = tactical::tests::current_defeat(approach);
                    assert!(!combat_victory(defeated.state()).unwrap());
                    let mut settled = defeated.clone();
                    for operation in 110..115 {
                        if !tactical::dying(settled.state()).unwrap() {
                            break;
                        }
                        settled = stage_with_supplier(
                            &settled,
                            &input(&settled, first, operation, "end-turn", vec![]),
                            &mut |_| Ok(20),
                        )
                        .unwrap();
                    }
                    let rested = stage_with_supplier(
                        &settled,
                        &input(&settled, first, 120, "short-rest", vec![]),
                        &mut |_| panic!("rest dice"),
                    )
                    .unwrap();
                    let rested = finish_rest_keeping_dice(rested);
                    for (checkpoint, key) in
                        [(&defeated, "defeat_mech"), (&rested, "defeat_rested")]
                    {
                        let projected = wire::journey_view(
                            checkpoint,
                            df_persistence::local_demo_scope::LocalDemoRole::Player,
                            first,
                        )
                        .unwrap();
                        let rpc::view_message::Audience::Player(view) = projected.audience.unwrap()
                        else {
                            panic!("player projection");
                        };
                        let prose: serde_json::Value = serde_json::from_slice(
                            &std::fs::read(
                                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                                    .join("../../content/adventure-tables/prose-patterns.json"),
                            )
                            .unwrap(),
                        )
                        .unwrap();
                        let mandatory = prose["tables"][key]["variants"][0]["text"]
                            .as_str()
                            .unwrap();
                        // 59ee1eba/c6d72482 separate the rested story from its mechanical note.
                        let note = view.scene.as_ref().unwrap().rules_note.as_str();
                        let mut expected = if key == "defeat_mech" {
                            let rules = mandatory.replace("{story}", "");
                            let rules = rules.split_whitespace().collect::<Vec<_>>().join(" ");
                            assert_eq!(note, rules);
                            ending.defeat.clone()
                        } else {
                            assert_eq!(
                                note,
                                "The party has been defeated. Heroes at 0 HP follow timed death saves; defeat does not restore hit points or held weapons. During the Short Rest each eligible hero chooses whether to spend Hit Point Dice to heal. Eligible heroes regain one use of Second Wind. A stable creature that isn't healed regains 1 Hit Point after 1d4 hours; a dead creature remains dead."
                            );
                            format!(
                                "{} The party was defeated. The party spends an hour catching its breath. That rest does not change the original defeat.",
                                ending.defeat
                            )
                        };
                        if approach == "escort-courier" {
                            expected.push(' ');
                            expected.push_str(&pack::with_content(|content| {
                                content.escort_support_suffix.replace("{name}", "Brynn")
                            }));
                        }
                        // The defeat text opens with what the party fought and who fell.
                        assert!(view.narration.ends_with(&expected), "{}", view.narration);
                        assert!(!view.narration.contains("RULES:"), "{}", view.narration);
                        assert!(view.narration.contains("overwhelmed the party"));
                        assert!(ending.defeat.chars().count() <= 220);
                        assert!(
                            !view.narration.contains(
                                serde_json::to_value(&frame).unwrap()["villain"]["name"]
                                    .as_str()
                                    .unwrap()
                            )
                        );
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod refusal_audit_tests {
    use super::*;
    #[test]
    fn character_stage_refusal_prints_its_own_step_before_outer_session_step() {
        refusal_detail::begin_action();
        let mut state = initial().unwrap().state().clone();
        let who = entity([5; 16]).unwrap();
        let member = df_types::MemberId::from_bytes(&[6; 16]).unwrap();
        assert_eq!(
            accept_character_at_level(&mut state, who, member, &[], 1),
            Err(RepositoryError::InvalidCandidate)
        );
        // The session records its outer stage after the candidate stage has rejected.
        df_session::submission::note_refusal_diagnostic(
            "session-engine-transition",
            "InvalidCandidate".to_owned(),
        );
        crate::gameplay::actor::record_refusal(
            rpc::GameplayActionKind::CreateCharacter as i32,
            "InvalidCandidate",
            "refused",
        );
        let lines =
            crate::gameplay::actor::audit_lines_containing("REFUSAL action=CreateCharacter");
        assert!(lines.iter().any(|line| {
            line.contains("step=character-accept outer=session-engine-transition")
                && line.contains("cause=NameMissing")
        }));
    }
}

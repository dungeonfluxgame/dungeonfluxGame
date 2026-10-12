//! Canonical room, authored story and source-qualified combat composition.
use df_model::checkpoint::*;
use df_persistence::local_demo_scope::PLAYER;
use df_protocol::common as rpc;
use df_rules::local_journey as rules;
use df_session::submission::RepositoryError;
use df_types::{MemberId, OperationId};
use prost::Message;
use std::time::Duration;

use super::{model, pack, wire};

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
mod encounter_admission;
mod encounter_outcome;
mod enemy_tactics;
mod narrative_phase;
#[cfg(test)]
mod narrative_runtime_tests;
mod narrative_transition;
mod objective_outcome;
#[cfg(test)]
mod objective_outcome_tests;
#[cfg(test)]
mod session_flow_tests;
#[cfg(test)]
mod text_pack_tests;

pub(super) const ROOM_CODE: &str = "LANTERN";
pub(super) fn room_code() -> String {
    pack::with_content(|pack| pack.room_code.clone())
}
pub(super) const ROOM_ENTITY: [u8; 16] = [0x45; 16];
const MEMBERS: [[u8; 16]; 2] = [[0x61; 16], [0x62; 16]];
const ENTITIES: [[u8; 16]; 2] = [[0x63; 16], [0x64; 16]];
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
    ("greatsword-attack", THREAT_THREAD),
    ("second-wind", THREAT_THREAD),
    ("end-turn", THREAT_THREAD),
];
pub(super) const CONTENT_ENTRIES: &[&str] = &[
    "room",
    "join-room",
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
    "combat",
    "complete",
    "choose-harbor-scene",
    "harbor-inn",
    "bandit",
    "greatsword-attack",
    "second-wind",
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
    "normal-nonlethal-melee",
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
    PACKET_THREAD,
    THREAT_THREAD,
];
fn bad<T>(_: T) -> RepositoryError {
    RepositoryError::InvalidCandidate
}
pub(super) fn rule() -> Result<RuleReference, RepositoryError> {
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
pub(super) fn phase(current: &Checkpoint) -> Result<rpc::JourneyPhase, RepositoryError> {
    let phase = narrative_phase::validate(current)?;
    encounter_outcome::validate(current, phase)?;
    Ok(phase)
}
pub(super) fn resources() -> Result<Vec<ResourceConstraint>, RepositoryError> {
    let hero_hp = pack::with_creation(|rules| {
        Ok::<_, RepositoryError>(
            i64::from(rules.base_hit_points)
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
    for owner in ENTITIES.into_iter().chain([BANDIT]) {
        for (name, minimum, maximum) in [
            (
                "hit-points",
                1,
                if owner == BANDIT {
                    i64::from(bandit.hit_points)
                } else {
                    hero_hp
                },
            ),
            (
                "armor-class",
                if owner == BANDIT {
                    i64::from(bandit.armor_class)
                } else {
                    hero_ac
                },
                if owner == BANDIT {
                    i64::from(bandit.armor_class)
                } else {
                    hero_ac
                },
            ),
            ("strength", 1, 20),
            ("dexterity", 1, 20),
            ("constitution", 1, 20),
            ("intelligence", 1, 20),
            ("wisdom", 1, 20),
            ("charisma", 1, 20),
            ("second-wind", 0, i64::from(pack::second_wind().uses_max)),
            ("unconscious", 0, 1),
            ("prone", 0, 1),
            ("held-weapon", 0, 1),
            ("action-used", 0, 1),
            ("bonus-used", 0, 1),
        ] {
            result.push(ResourceConstraint {
                owner: entity(owner)?,
                resource: model::label(name)?,
                minimum,
                maximum,
                source: rule()?,
            });
        }
    }
    Ok(result)
}
fn add_resource(
    state: &mut GameState,
    who: EntityId,
    name: &str,
    value: i64,
) -> Result<(), RepositoryError> {
    let label = model::label(name)?;
    let constraint = resources()?
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
    state
        .resources
        .iter()
        .find(|resource| resource.owner == who && resource.resource == label)
        .ok_or(RepositoryError::InvalidCandidate)
        .and_then(|resource| u32::try_from(resource.value).map_err(bad))
}
fn set(
    state: &mut GameState,
    who: EntityId,
    name: &str,
    value: u32,
) -> Result<(), RepositoryError> {
    let label = model::label(name)?;
    let resource = state
        .resources
        .iter_mut()
        .find(|resource| resource.owner == who && resource.resource == label)
        .ok_or(RepositoryError::InvalidCandidate)?;
    resource.value = i64::from(value);
    Ok(())
}
pub(super) fn name(state: &GameState, who: EntityId) -> Result<String, RepositoryError> {
    if who == entity(BANDIT)? {
        return Ok(pack::with_content(|pack| pack.bandit.name.clone()));
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
        .map(|choice| choice.selected.as_str().to_owned())
        .ok_or(RepositoryError::InvalidCandidate)
}
pub(super) fn maximum_hit_points(state: &GameState, who: EntityId) -> Result<u32, RepositoryError> {
    let constitution = value(state, who, "constitution")?;
    let base = pack::with_creation(|rules| rules.base_hit_points);
    u32::try_from(i32::from(base) + rules::ability_modifier(constitution as u8)).map_err(bad)
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
pub(super) fn character_sheet(
    state: &GameState,
    who: EntityId,
) -> Result<rpc::CharacterSheet, RepositoryError> {
    let mut facts = Vec::new();
    if pack::active().is_some_and(|loaded| !loaded.departures.is_empty()) {
        facts.push(rpc::SheetFact {
            label: "Supported rules".to_owned(),
            value: model::source_revision(),
        });
    }
    let option_label = |group: &str| -> Result<String, RepositoryError> {
        pack::with_content(|pack| {
            pack.creation
                .groups
                .iter()
                .find(|candidate| candidate.id == group)
                .and_then(|group| group.options.first())
                .map(|option| option.label.clone())
                .ok_or(RepositoryError::InvalidCandidate)
        })
    };
    let gold = pack::with_content(|pack| {
        pack.equipment
            .iter()
            .filter(|row| row.item == "gold-piece")
            .map(|row| u32::from(row.quantity))
            .sum::<u32>()
    });
    for (label, value) in [
        ("Species", option_label("species")?),
        ("Class", option_label("class")?),
        ("Background", option_label("background")?),
        ("Armor", "Chain Mail · Defense".to_owned()),
        (
            "Weapon",
            format!("{} · Graze", weapon_display_name(&pack::hero_weapon_item())),
        ),
        ("Origin feat", "Savage Attacker".to_owned()),
        ("Masteries", "Greatsword, Flail, Javelin".to_owned()),
        (
            "Starting equipment",
            format!("{} · {gold} GP", option_label("equipment")?),
        ),
    ] {
        facts.push(rpc::SheetFact {
            label: label.to_owned(),
            value: value.to_owned(),
        });
    }
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
    facts.push(rpc::SheetFact {
        label: weapon_display_name(&pack::hero_weapon_item()),
        value: if value(state, who, "held-weapon")? != 0 {
            "Held"
        } else {
            "Dropped; ownership retained"
        }
        .to_owned(),
    });
    Ok(rpc::CharacterSheet {
        name: name(state, who)?,
        facts,
        hit_points: value(state, who, "hit-points")?,
        maximum_hit_points: maximum_hit_points(state, who)?,
        armor_class: value(state, who, "armor-class")?,
        source_revision: model::source_revision(),
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
    affordance::offered(current, member).map_err(bad)
}

fn authored_offered(
    current: &Checkpoint,
    member: MemberId,
) -> Result<Vec<(rpc::GameplayActionKind, String)>, RepositoryError> {
    let phase = phase(current)?;
    let who = player_entity(member, current)?;
    let ready = current
        .state()
        .characters
        .iter()
        .any(|character| character.entity == who);
    if !ready {
        return Ok(vec![(
            rpc::GameplayActionKind::CreateCharacter,
            pack::offer_label(rpc::GameplayActionKind::CreateCharacter),
        )]);
    }
    let result = match phase {
        rpc::JourneyPhase::Room
            if current.state().characters.len() == 2
                && narrative_transition::story_permitted(current, member)? =>
        {
            vec![(
                rpc::GameplayActionKind::BeginStory,
                pack::offer_label(rpc::GameplayActionKind::BeginStory),
            )]
        }
        rpc::JourneyPhase::Room => vec![],
        rpc::JourneyPhase::Opening => vec![
            (
                rpc::GameplayActionKind::AskCourier,
                pack::offer_label(rpc::GameplayActionKind::AskCourier),
            ),
            (
                rpc::GameplayActionKind::EscortCourier,
                pack::offer_label(rpc::GameplayActionKind::EscortCourier),
            ),
        ],
        rpc::JourneyPhase::Dialogue if narrative_phase::courier_encounter_permitted(current)? => {
            vec![(
                rpc::GameplayActionKind::DefendCourier,
                pack::offer_label(rpc::GameplayActionKind::DefendCourier),
            )]
        }
        rpc::JourneyPhase::Dialogue => vec![],
        rpc::JourneyPhase::Combat => {
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
        rpc::JourneyPhase::Complete => {
            prepare_short_rest(current)?;
            let mut result = vec![(
                rpc::GameplayActionKind::ShortRest,
                pack::offer_label(rpc::GameplayActionKind::ShortRest),
            )];
            if inn_source(current)?.is_some() && value(current.state(), who, "unconscious")? == 0 {
                result.push((
                    rpc::GameplayActionKind::ChooseHarborScene,
                    pack::offer_label(rpc::GameplayActionKind::ChooseHarborScene),
                ));
            }
            result
        }
        _ => return Err(RepositoryError::InvalidCandidate),
    };
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
        || participants(current).len() != 2
        || state.characters.len() != 2
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

pub(super) fn inn_scene() -> rpc::GameplayScene {
    rpc::GameplayScene {
        destination: rpc::HarborDestination::HarborInn as i32,
        title: pack::scene(pack::SceneKey::Inn).title,
        scene_asset: pack::scene(pack::SceneKey::Inn).asset,
        speaker_portrait: pack::scene(pack::SceneKey::Inn).speaker_portrait,
        cast: Vec::new(),
        // The view fills in the rules note and the place, which the pack alone knows.
        rules_note: String::new(),
        place: String::new(),
        location_id: String::new(),
    }
}
pub(super) fn accepted(
    decision: &AcceptedDecision,
) -> Result<rpc::AcceptedAction, RepositoryError> {
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
    if party.len() != 2
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
    for ordinal in 0..3 {
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
                if cause.operation != fact.operation
                    || cause.revision != fact.revision
                    || cause.ordinal >= fact.ordinal
                    || !decision.facts.contains(&cause.id)
                {
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

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
pub(super) fn unhex(text: &str) -> Result<Vec<u8>, RepositoryError> {
    if text.len() > 4096 || !text.len().is_multiple_of(2) {
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
            resources: &resources()?,
            assets: &[],
        },
        df_model::commands::CommandLimits {
            maximum_records: 32,
            maximum_text_bytes: 128,
            maximum_retained_bytes: 8192,
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
            Position { x: 5, y: 0, z: 0 }
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
    } = content;
    let objective = objective_command
        .map(|command| objective_outcome::pending(current, &state, command))
        .transpose()?
        .flatten();
    if (entry == "escort-courier") != escort_target.is_some()
        || (reaction.is_some() && entry != "defend-courier")
    {
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
        let id = make_id(facts.len() as u32)?;
        state.facts.push(GameFact {
            id,
            revision: basis.revision,
            operation,
            ordinal: facts.len() as u32,
            cause: facts.last().copied(),
            audience: AudienceScope::Shared,
            value: FactValue::TimeAdvanced {
                before: current.state().logical_time,
                after: state.logical_time,
            },
        });
        facts.push(id);
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
            let destination = moved.location.ok_or(RepositoryError::InvalidCandidate)?;
            let position = moved.position.ok_or(RepositoryError::InvalidCandidate)?;
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
                return Err(RepositoryError::InvalidCandidate);
            }
            let cause=state.facts.iter().find(|fact|fact.operation==operation && matches!(&fact.value,FactValue::ResourceChanged {entity,resource,before:0,after:1,..} if *entity==scheduled.entity && resource.as_str()=="unconscious")).map(|fact|fact.id).ok_or(RepositoryError::InvalidCandidate)?;
            let started = LogicalTime {
                ticks: scheduled
                    .due
                    .ticks
                    .checked_sub(
                        u64::from(scheduled.due.ticks_per_second)
                            * u64::from(pack::rest().short_rest_seconds),
                    )
                    .ok_or(RepositoryError::InvalidCandidate)?,
                ticks_per_second: scheduled.due.ticks_per_second,
            };
            if started.ticks_per_second != evidenced_time.ticks_per_second
                || started.ticks < evidenced_time.ticks
                || started.ticks > state.logical_time.ticks
            {
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
            Some(inn_source(current)?.ok_or(RepositoryError::InvalidCandidate)?)
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
    let candidate = model::checkpoint(basis, state)?;
    let candidate = if objective.is_some() {
        objective_outcome::stage(
            current,
            candidate,
            objective_command.ok_or(RepositoryError::InvalidCandidate)?,
        )?
    } else {
        candidate
    };
    let candidate = stage_authored_threads(candidate, fact_id)?;
    phase(&candidate)?;
    Ok(candidate)
}

fn stage_authored_threads(
    candidate: Checkpoint,
    source_id: FactId,
) -> Result<Checkpoint, RepositoryError> {
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
    let defeated = thread == model::content(THREAT_THREAD)? && candidate.state().facts.iter().any(|fact| {
        fact.operation == source.operation && fact.revision == source.revision && fact.ordinal < source.ordinal
            && matches!(&fact.value, FactValue::ResourceChanged { entity, resource, before: 0, after: 1, source }
                if *entity == bandit && *resource == unconscious && *source == source_rule)
    });
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
                resources: &resources()?,
                assets: &[],
            },
            checkpoint_limits: model::limits(),
            selections: &selections,
        },
        ThreadDirectorLimits {
            directors: DirectorLimits {
                maximum_checkpoint_bytes: model::limits().maximum_retained_bytes,
                maximum_pass_bytes: 8 * 1024 * 1024,
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
        && ![
            u32::from(pack::hero_weapon().damage_sides),
            u32::from(pack::bandit().weapon.damage_sides),
            u32::from(pack::second_wind().die_sides),
        ]
        .contains(&sides)
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    for _ in 0..16 {
        let mut bytes = [0_u8; 1];
        use std::io::Read;
        std::fs::File::open("/dev/urandom")
            .and_then(|mut source| source.read_exact(&mut bytes))
            .map_err(|_| RepositoryError::Unavailable)?;
        let [byte] = bytes;
        let bucket = 256 - (256 % sides);
        if u32::from(byte) < bucket {
            return Ok(u32::from(byte) % sides + 1);
        }
    }
    Err(RepositoryError::Unavailable)
}
fn draw(
    operation: OperationId,
    sides: u32,
    draws: &mut Vec<ActualDraw>,
    supplier: DiceSource<'_>,
) -> Result<u32, RepositoryError> {
    if draws.len() >= 32 {
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
fn complete_short_rest(current: &Checkpoint, state: &mut GameState) -> Result<(), RepositoryError> {
    let selection = prepare_short_rest(current)?;
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
    let monster = who == entity(BANDIT)?;
    let bandit = pack::bandit();
    let weapon = if monster {
        bandit.weapon
    } else {
        pack::hero_weapon()
    };
    let ability = if monster {
        bandit.damage_modifier
    } else {
        rules::ability_modifier(value(state, who, "strength")? as u8)
    };
    let modifier = if monster {
        bandit.attack_bonus
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
    })
}
fn finish_combat(state: &mut GameState) -> Result<bool, RepositoryError> {
    let [encounter] = state.encounters.as_slice() else {
        return Err(RepositoryError::InvalidCandidate);
    };
    if state.characters.len() != 2
        || encounter.objectives != [model::content("defend-courier")?]
        || encounter.active_turn.is_none()
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let enemy = entity(BANDIT)?;
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
    let monster = entity(BANDIT)?;
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
    let monster = entity(BANDIT)?;
    state.entities.push(WorldEntity {
        id: monster,
        definition: model::content("bandit")?,
        location: Some(room_entity()?),
        position: Some(Position { x: 0, y: 0, z: 0 }),
        identity_revision: model::label("srd521-bandit-1")?,
    });
    for (key, value) in [
        ("hit-points", i64::from(pack::bandit().hit_points)),
        ("armor-class", i64::from(pack::bandit().armor_class)),
        ("strength", i64::from(pack::bandit().abilities[0])),
        ("dexterity", i64::from(pack::bandit().abilities[1])),
        ("constitution", i64::from(pack::bandit().abilities[2])),
        ("intelligence", i64::from(pack::bandit().abilities[3])),
        ("wisdom", i64::from(pack::bandit().abilities[4])),
        ("charisma", i64::from(pack::bandit().abilities[5])),
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
    state.entities.push(WorldEntity {
        id: weapon,
        definition: model::content(&pack::bandit_weapon_item())?,
        location: Some(monster),
        position: None,
        identity_revision: model::label("source-held-scimitar-1")?,
    });
    state.inventory.push(InventoryItem {
        item: weapon,
        owner: monster,
        quantity: 1,
        source: rule()?,
        origin: model::content(&pack::bandit_weapon_item())?,
        attunement_owner: None,
    });
    let participants = state
        .members
        .iter()
        .filter(|link| link.member.as_bytes() != &PLAYER)
        .map(|link| link.character.ok_or(RepositoryError::InvalidCandidate))
        .chain([Ok(monster)])
        .collect::<Result<Vec<_>, _>>()?;
    let admitted =
        encounter_admission::admission(current, command, state, &participants).map_err(bad)?;
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
                resources: &resources()?,
                assets: &[],
            },
            limits: df_engine::command_entry::CommandEntryLimits {
                command: df_model::commands::CommandLimits {
                    maximum_records: 32,
                    maximum_text_bytes: 128,
                    maximum_retained_bytes: 8192,
                },
                maximum_staged_bytes: 1024 * 1024,
            },
        },
        &admitted,
        state,
        &source,
    )
    .map_err(bad)?
    .to_vec();
    let active = order
        .first()
        .copied()
        .ok_or(RepositoryError::InvalidCandidate)?;
    state.encounters.push(EncounterState {
        id: RecordId::from_bytes(&ENCOUNTER).map_err(bad)?,
        definition: model::content("combat")?,
        participants,
        turn_order: order,
        active_turn: Some(active),
        objectives: vec![model::content("defend-courier")?],
        combat_policy: model::content("normal-nonlethal-melee")?,
    });
    beat(state, "combat")?;
    enemy_turn(current, state, operation, draws, outcomes, supplier)
}
fn accept_character(
    state: &mut GameState,
    who: EntityId,
    member: MemberId,
    choices: &[(df_types::RevisionLabel, df_types::RevisionLabel)],
) -> Result<(), RepositoryError> {
    let encoded = choice(choices, "name")?;
    let encoded_name = if encoded == "empty-name" {
        Vec::new()
    } else {
        unhex(encoded)?
    };
    let name = std::str::from_utf8(&encoded_name).map_err(bad)?;
    let options = choices
        .iter()
        .filter(|(group, _)| group.as_str() != "name")
        .map(|(group, option)| (group.as_str(), option.as_str()))
        .collect::<Vec<_>>();
    let build = pack::validate_character(name, &options).map_err(bad)?;
    if state
        .characters
        .iter()
        .any(|character| character.entity == who)
    {
        return Err(RepositoryError::InvalidCandidate);
    }
    let source = rule()?;
    let name_label = model::label(name)?;
    let selected = choices
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
    state.characters.push(CharacterState {
        entity: who,
        build: model::content("dwarf-fighter-soldier")?,
        owner: member,
        choices: selected.clone(),
    });
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
        ("unconscious", 0),
        ("prone", 0),
        ("held-weapon", 1),
        ("action-used", 0),
        ("bonus-used", 0),
    ] {
        add_resource(state, who, key, i64::from(value))?;
    }
    // The source-selected bundles are inventory, not cosmetic rewards or generated loot.
    let equipment = pack::with_content(|pack| pack.equipment.clone());
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
            identity_revision: model::label("source-starting-equipment-1")?,
        });
        state.inventory.push(InventoryItem {
            item: entity(id)?,
            owner: who,
            quantity,
            source: rule()?,
            origin: model::content(entry)?,
            attunement_owner: None,
        });
    }
    Ok(())
}
fn stage_using(
    current: &Checkpoint,
    input: &GameInput,
    supplier: DiceSource<'_>,
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
            resources: &resources()?,
            assets: &[],
        },
        df_model::commands::CommandLimits {
            maximum_records: 32,
            maximum_text_bytes: 128,
            maximum_retained_bytes: 8192,
        },
    )
    .map_err(bad)?;
    let command = command(input)?;
    let GameCommand::ProposeAction {
        actor: who,
        action,
        targets,
        ..
    } = &command.command
    else {
        return Err(RepositoryError::InvalidCandidate);
    };
    if *who != player_entity(command.member, current)? {
        return Err(RepositoryError::Unauthorized);
    }
    let kind = wire::action_kind(action.entry.as_str()).ok_or(RepositoryError::InvalidCandidate)?;
    affordance::validate(current, command.member, action, targets).map_err(bad)?;
    // Target identity is validated against the current source-admitted binding above.
    // The registered handlers consume their existing authored, implicit target form;
    // keep that source contract intact after exact typed target confirmation.
    let mut source_input = input.clone();
    let GameInput::Game(source_command) = &mut source_input else {
        return Err(RepositoryError::InvalidCandidate);
    };
    let GameCommand::ProposeAction { targets, .. } = &mut source_command.command else {
        return Err(RepositoryError::InvalidCandidate);
    };
    targets.clear();
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
    match kind {
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
            courier_reaction::enter_opening(&mut state)?;
        }
        rpc::GameplayActionKind::AskCourier | rpc::GameplayActionKind::EscortCourier => {
            if !choices.is_empty() {
                return Err(RepositoryError::InvalidCandidate);
            }
            state.narrative = narrative_transition::select(current, input)?
                .state()
                .narrative
                .clone();
            private = kind == rpc::GameplayActionKind::AskCourier;
        }
        rpc::GameplayActionKind::DefendCourier => {
            if !choices.is_empty() {
                return Err(RepositoryError::InvalidCandidate);
            }
            state
                .narrative
                .open_threads
                .push(model::content(THREAT_THREAD)?);
            start_combat(
                current,
                command,
                &mut state,
                &mut draws,
                &mut outcomes,
                supplier,
            )?;
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
        rpc::GameplayActionKind::ShortRest => {
            if !choices.is_empty() {
                return Err(RepositoryError::InvalidCandidate);
            }
            complete_short_rest(current, &mut state)?;
        }
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
        check: None,
        scene: if kind == rpc::GameplayActionKind::ChooseHarborScene {
            Some(inn_scene())
        } else {
            None
        },
        character: if kind == rpc::GameplayActionKind::CreateCharacter {
            Some(character_sheet(&state, *who)?)
        } else {
            None
        },
        combat: outcomes,
        phase: phase_from_state(&state)? as i32,
    };
    let semantic = Some(hex(&result.encode_to_vec()));
    let entry = if private {
        "private-courier-note"
    } else if kind == rpc::GameplayActionKind::ChooseHarborScene {
        "harbor-inn"
    } else {
        action.entry.as_str()
    };
    let reaction = if kind == rpc::GameplayActionKind::DefendCourier {
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
        },
    )?;
    if private {
        super::courier_ai::append_request(candidate, command.operation)
    } else {
        Ok(candidate)
    }
}
fn phase_from_state(state: &GameState) -> Result<rpc::JourneyPhase, RepositoryError> {
    // The receipt is encoded before its causal fact exists. Admission follows finish(),
    // after the accepted decision and authored thread consequences have been staged.
    narrative_phase::classify(state)
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
            let draw = supplied.next().ok_or(RepositoryError::InvalidCandidate)?;
            if draw.operation != operation
                || draw.ordinal != ordinal
                || draw.sides != sides
                || draw.source != rule()?
                || !(1..=sides).contains(&draw.value)
            {
                return Err(RepositoryError::InvalidCandidate);
            }
            ordinal += 1;
            Ok(draw.value)
        };
        let staged = stage_using(current, input.command, &mut consume)?;
        if supplied.next().is_some() {
            return Err(RepositoryError::InvalidCandidate);
        }
        Ok(staged)
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
    let prepared = stage_using(current, input, supplier)?;
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
    let handler = courier_reaction::CourierJourneyHandler {
        command_handler: &base_handler,
    };
    let registrations = [df_rules::HandlerRegistration::new(
        &selector, &source, &handler,
    )];
    let registry =
        df_rules::DispatchRegistry::from_catalog(catalog, &registrations, 1).map_err(bad)?;
    df_engine::command_entry::decide_registered_command(
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
                resources: &resources()?,
                assets: &[],
            },
            limits: df_engine::command_entry::CommandEntryLimits {
                command: df_model::commands::CommandLimits {
                    maximum_records: 32,
                    maximum_text_bytes: 128,
                    maximum_retained_bytes: 8192,
                },
                maximum_staged_bytes: 1024 * 1024,
            },
        },
        &registry,
        &selector,
        &source,
    )
    .map_err(bad)
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
    pub(super) fn tuning_journey() -> Vec<Checkpoint> {
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
                    .unwrap(),
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
        for creature in &df_rules::SRD_521_CREATURES {
            let location = if ["skeleton", "wolf"].contains(&creature.id) {
                "pinewood-trail"
            } else {
                "salt-quay"
            };
            let generated = generate(
                &tables,
                42,
                GenerateOptions {
                    rules_mode: RulesMode::CustomAllowed,
                },
                &ForcedEntries::from([
                    ("location".into(), location.into()),
                    ("antagonist".into(), creature.id.into()),
                ]),
            )
            .unwrap();
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
            include_bytes!("../../../../content/harbor-journey/tuning.json"),
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
            "../../../../content/harbor-journey/tuning.json"
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
    fn shipped_tuning_reproduces_full_compiled_baseline_journey() {
        let baseline = tuning_journey();
        let loaded = pack::load_from_bytes(
            include_bytes!("../../../../content/harbor-journey/pack.json"),
            include_bytes!("../../../../content/harbor-journey/tuning.json"),
        )
        .unwrap();
        pack::with_test_pack(loaded, || {
            for (actual, expected) in tuning_journey().iter().zip(&baseline) {
                assert_eq!(actual.state(), expected.state());
                assert_eq!(actual.basis(), expected.basis());
            }
        });
    }

    #[test]
    fn tuned_journey_changes_creation_attack_healing_rest_and_completes() {
        let mut tuning: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../content/harbor-journey/tuning.json"
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
            assert_eq!(outcomes, vec![outcome]);
            assert_eq!(
                phase(checkpoints.last().unwrap()).unwrap(),
                rpc::JourneyPhase::Complete
            );
        });
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
            character: Some(rpc::CharacterSelection::default()),
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
                resources: &resources().expect("resources"),
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
            .expect("registered character outcome");
        }
        current
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
        .expect("registered opening")
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
                resources: &resources().unwrap(),
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
        let rested = stage_with_supplier(
            &defeated,
            &input(&defeated, first, 9, "short-rest", vec![]),
            &mut |_| panic!("rest dice"),
        )
        .unwrap();
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
        let rested =
            stage_with_supplier(&defeated, &command, &mut |_| panic!("rest cannot draw")).unwrap();
        assert_rest_preserves_battle(&defeated, &rested);
        for bytes in ENTITIES {
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
            Ok(rested.clone())
        );
        assert!(stage_with_supplier(&rested, &command, &mut |_| panic!("stale rest")).is_err());
        let next = stage_with_supplier(
            &rested,
            &input(&rested, first, 10, "short-rest", vec![]),
            &mut |_| panic!("fresh rest cannot draw"),
        )
        .unwrap();
        assert_rest_preserves_battle(&rested, &next);
        for bytes in ENTITIES {
            assert_eq!(
                value(next.state(), entity(bytes).unwrap(), "second-wind"),
                Ok(2)
            );
        }
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
        invalid = input(&rest, member, 11, "short-rest", vec![]);
        let second = stage_with_supplier(&rest, &invalid, &mut |_| panic!("fresh rest")).unwrap();
        assert_rest_preserves_battle(&rest, &second);
    }

    pub(super) fn inn_victory() -> Checkpoint {
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
        let victory = inn_victory();
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
                resources: &resources().unwrap(),
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
        let victory = inn_victory();
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let rest = stage_with_supplier(
            &victory,
            &input(&victory, first, 9, "short-rest", vec![]),
            &mut |_| panic!("rest dice"),
        )
        .unwrap();
        for current in [&victory, &rest] {
            for bytes in MEMBERS {
                let member = MemberId::from_bytes(&bytes).unwrap();
                let offers = offered(current, member).unwrap();
                assert!(
                    offers
                        .iter()
                        .any(|(kind, _)| *kind == rpc::GameplayActionKind::ShortRest)
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
                resources: &resources().unwrap(),
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
        for (role, principal) in [
            (LocalDemoRole::Player, first),
            (LocalDemoRole::Player, second),
            (LocalDemoRole::Display, bootstrap_member().unwrap()),
        ] {
            let projected = wire::journey_view(&arrived, role, principal).unwrap();
            match projected.audience.unwrap() {
                rpc::view_message::Audience::Player(view) => {
                    assert_eq!(view.scene, Some(inn_scene()));
                    assert_eq!(view.journey.unwrap().party.len(), 2);
                    assert!(!view.narration.contains("Vell"));
                    assert!(!view.offers.iter().any(|offer| offer.action_kind
                        == rpc::GameplayActionKind::ChooseHarborScene as i32));
                }
                rpc::view_message::Audience::Display(view) => {
                    assert_eq!(view.scene, Some(inn_scene()));
                    assert!(!view.narration.contains("Vell"));
                }
            }
        }
        assert!(arrived.state().facts.iter().any(|fact| matches!(&fact.value, FactValue::ContentEvent { definition, .. } if definition.entry.as_str() == "private-courier-note") && fact.audience == AudienceScope::Members(vec![first])));
    }

    #[test]
    fn post_rest_inn_preserves_real_cursor_and_refuses_forged_policy_time_or_pending_queue() {
        let victory = inn_victory();
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
                .is_err()
            );
            assert_eq!(forged, before);
        }
    }
    #[test]
    fn campaign_terminal_policy_waits_for_destination_and_retains_actual_secret_holder_and_rest() {
        let completion = super::super::adventure::campaign_completion::terminal_outcome;
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let victory = inn_victory();
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
                    let opening = opening_story();
                    let dialogue = stage_with_supplier(
                        &opening,
                        &input(&opening, first, 6, approach, vec![]),
                        &mut |_| panic!("dialogue dice"),
                    )
                    .unwrap();
                    let mut initiative = 0;
                    let combat = stage_with_supplier(
                        &dialogue,
                        &input(&dialogue, first, 7, "defend-courier", vec![]),
                        &mut |sides| {
                            if sides == 20 {
                                initiative += 1;
                                Ok(if initiative <= 2 { 1 } else { 20 })
                            } else {
                                Ok(sides)
                            }
                        },
                    )
                    .unwrap();
                    let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
                    let defeated = stage_with_supplier(
                        &combat,
                        &input(&combat, second, 8, "end-turn", vec![]),
                        &mut |sides| Ok(if sides == 20 { 20 } else { sides }),
                    )
                    .unwrap();
                    assert!(!combat_victory(defeated.state()).unwrap());
                    let rested = stage_with_supplier(
                        &defeated,
                        &input(&defeated, first, 9, "short-rest", vec![]),
                        &mut |_| panic!("rest dice"),
                    )
                    .unwrap();
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
                        // Before the rest the rules wording is the scene's own rules note and is
                        // never joined into the narration; the rested text carries its wording
                        // inside the story itself.
                        let note = view.scene.as_ref().unwrap().rules_note.as_str();
                        let mut expected = if key == "defeat_mech" {
                            let rules = mandatory.replace("{story}", "");
                            let rules = rules.split_whitespace().collect::<Vec<_>>().join(" ");
                            assert_eq!(note, rules);
                            ending.defeat.clone()
                        } else {
                            assert_eq!(note, "");
                            format!("{} {mandatory}", ending.defeat)
                        };
                        if approach == "escort-courier" {
                            expected.push(' ');
                            expected.push_str(&pack::with_content(|content| {
                                content.escort_support_suffix.replace("{name}", "Brynn")
                            }));
                        }
                        assert_eq!(view.narration, expected);
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

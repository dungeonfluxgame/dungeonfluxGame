//! Native Bandit selection over the authored adjacent-melee candidate working copy.
use df_combat::candidates::{
    CandidateContext, CandidateError, CandidateLimits, LegalOfferOwner, enumerate_candidates,
};
use df_combat::ranking::{
    KnowledgeObserver, RankingObservation, TacticalKnowledge, UtilityAssignment, UtilityLimits,
    UtilityPolicy, rank_tactics,
};
use df_model::checkpoint::{
    Basis, Checkpoint, ContentReference, EntityId, GameState, LogicalTime, Position, RecordId,
    RuleReference,
};
use df_session::submission::RepositoryError;
use df_types::OperationId;

use super::{
    ENCOUNTER, ENTITIES, combat_creature, combat_enemy, combat_weapon_item, entity, held_weapon_id,
    model, room_entity, rule, rules,
};

struct NativeObservation {
    basis: Basis,
    actor: EntityId,
    cause: OperationId,
    logical_time: LogicalTime,
    policy: ContentReference,
}

#[derive(Eq, PartialEq)]
struct NativeAttack {
    actor: EntityId,
    target: EntityId,
    weapon: EntityId,
    action: ContentReference,
    source: RuleReference,
}

struct CurrentNativeTurn<'a> {
    current: &'a Checkpoint,
    staged: &'a GameState,
    operation: OperationId,
}

impl CurrentNativeTurn<'_> {
    fn resource(&self, actor: EntityId, key: &str) -> Result<u32, RepositoryError> {
        let resource = self
            .staged
            .resources
            .iter()
            .find(|resource| resource.owner == actor && resource.resource.as_str() == key)
            .ok_or(RepositoryError::InvalidCandidate)?;
        if resource.source != rule()? {
            return Err(RepositoryError::InvalidCandidate);
        }
        u32::try_from(resource.value).map_err(|_| RepositoryError::InvalidCandidate)
    }
}

impl LegalOfferOwner for CurrentNativeTurn<'_> {
    type Observation = NativeObservation;
    type Offer = NativeAttack;
    type Error = RepositoryError;

    fn validate_observation(
        &self,
        observation: &NativeObservation,
        current: CandidateContext<'_>,
    ) -> Result<(), RepositoryError> {
        if *current.basis != self.current.basis() || observation.basis != *current.basis {
            return Err(RepositoryError::RevisionConflict);
        }
        let pins = model::pins()?;
        if current.pins != self.current.pins() || current.pins != &pins {
            return Err(RepositoryError::InvalidCandidate);
        }
        self.current
            .validate_resume(*current.basis, current.pins)
            .map_err(|_| RepositoryError::InvalidCandidate)?;
        let monster = combat_enemy(self.staged)?;
        let [encounter] = self.staged.encounters.as_slice() else {
            return Err(RepositoryError::InvalidCandidate);
        };
        let scene_assault = encounter
            .objectives
            .contains(&model::content("scene-npc-assault")?);
        let party = super::participants(self.current)
            .into_iter()
            .map(|link| link.character.ok_or(RepositoryError::InvalidCandidate))
            .collect::<Result<Vec<_>, _>>()?;
        if observation.actor != monster
            || observation.cause != self.operation
            || observation.logical_time != self.staged.logical_time
            || observation.logical_time.ticks_per_second != 1
            || self.current.state().logical_time.ticks_per_second != 1
            || observation
                .logical_time
                .ticks
                .checked_sub(self.current.state().logical_time.ticks)
                .is_none_or(|elapsed| {
                    elapsed != 0 && elapsed != u64::from(super::pack::combat().round_ticks)
                })
            || observation.policy
                != model::content(if scene_assault {
                    "tactical-srd521-cover-policy"
                } else {
                    "normal-nonlethal-melee"
                })?
            || self.staged.members != self.current.state().members
            || self.staged.characters != self.current.state().characters
            || party.as_slice() != [entity(ENTITIES[0])?, entity(ENTITIES[1])?]
            || encounter.id
                != RecordId::from_bytes(&ENCOUNTER)
                    .map_err(|_| RepositoryError::InvalidCandidate)?
            || encounter.definition != model::content("combat")?
            || encounter.combat_policy != observation.policy
            || encounter.active_turn != Some(monster)
            || encounter.participants != [party[0], party[1], monster]
            || encounter.turn_order.len() != 3
            || encounter.participants.iter().any(|actor| {
                encounter
                    .turn_order
                    .iter()
                    .filter(|entry| *entry == actor)
                    .count()
                    != 1
            })
            || !(encounter
                .objectives
                .contains(&model::content("defend-courier")?)
                || encounter
                    .objectives
                    .contains(&model::content("scene-npc-assault")?))
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        let actor = self
            .staged
            .entities
            .iter()
            .find(|actor| actor.id == monster)
            .ok_or(RepositoryError::InvalidCandidate)?;
        if (!scene_assault && actor.definition != model::content("bandit")?)
            || (!scene_assault && actor.identity_revision != model::label("srd521-bandit-1")?)
            || (scene_assault
                && (actor.definition != model::content("lantern-wharf-courier")?
                    || actor.identity_revision != model::label("lantern-wharf-courier-1")?))
            || actor.location != Some(room_entity()?)
            || (if scene_assault {
                actor.position.is_none()
            } else {
                actor.position != Some(Position { x: 0, y: 0, z: 0 })
            })
            || self.resource(monster, "unconscious")? != 0
            || self.resource(monster, "hit-points")? == 0
            || self.resource(monster, "prone")? != 0
            || self.resource(monster, "dexterity")?
                != u32::from(combat_creature(self.staged, monster).rules.abilities[1])
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        Ok(())
    }

    fn is_current_legal(
        &self,
        observation: &NativeObservation,
        _current: CandidateContext<'_>,
        offer: &NativeAttack,
    ) -> Result<bool, RepositoryError> {
        if offer.actor != observation.actor
            || offer.weapon != held_weapon_id(observation.actor)?
            || offer.action != model::content(&combat_weapon_item(self.staged, observation.actor))?
            || offer.source != rule()?
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        let target = self
            .staged
            .entities
            .iter()
            .find(|entity| entity.id == offer.target)
            .ok_or(RepositoryError::InvalidCandidate)?;
        let expected_position = if offer.target == entity(ENTITIES[0])? {
            Position { x: 0, y: 5, z: 0 }
        } else if offer.target == entity(ENTITIES[1])? {
            Position { x: 5, y: 0, z: 0 }
        } else {
            return Err(RepositoryError::InvalidCandidate);
        };
        let build = model::content("dwarf-fighter-soldier")?;
        let Some(character) = self
            .staged
            .characters
            .iter()
            .find(|character| character.entity == target.id && character.build == build)
        else {
            return Err(RepositoryError::InvalidCandidate);
        };
        if !self
            .staged
            .members
            .iter()
            .any(|link| link.member == character.owner && link.character == Some(target.id))
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        let weapon = self
            .staged
            .entities
            .iter()
            .find(|weapon| weapon.id == offer.weapon)
            .ok_or(RepositoryError::InvalidCandidate)?;
        let item = self
            .staged
            .inventory
            .iter()
            .find(|item| item.item == offer.weapon)
            .ok_or(RepositoryError::InvalidCandidate)?;
        if weapon.definition != offer.action
            || weapon.identity_revision != model::label("source-held-scimitar-1")?
            || item.owner != offer.actor
            || item.quantity != 1
            || item.source != offer.source
            || item.origin != offer.action
            || item.attunement_owner.is_some()
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        let held = weapon.location == Some(offer.actor) && weapon.position.is_none();
        let economy = rules::ActionEconomy {
            action_used: self.resource(offer.actor, "action-used")? != 0,
            bonus_action_used: self.resource(offer.actor, "bonus-used")? != 0,
        };
        // These exact authored five-foot cells are the selected adjacent profile;
        // this adapter does not infer general reach, movement, cover or advantage.
        Ok(held
            && self.resource(offer.actor, "held-weapon")? == 1
            && economy.attack().is_ok()
            && target.location == Some(room_entity()?)
            && target.position == Some(expected_position)
            && target.definition == model::content("joined-player")?
            && target.identity_revision == model::label("joined-player-1")?
            && self.resource(target.id, "unconscious")? == 0
            && self.resource(target.id, "prone")? == 0
            && self.resource(target.id, "hit-points")? != 0)
    }

    fn same_offer(
        &self,
        left: &NativeAttack,
        right: &NativeAttack,
    ) -> Result<bool, RepositoryError> {
        Ok(left == right)
    }

    fn offer_bytes(&self, offer: &NativeAttack) -> Result<usize, RepositoryError> {
        Ok(48
            + offer.action.package.as_str().len()
            + offer.action.entry.as_str().len()
            + offer.source.catalog.as_str().len()
            + offer.source.source.as_str().len()
            + offer.source.entry.as_str().len()
            + offer.source.clause.as_str().len())
    }
}

fn admission_error(error: CandidateError<RepositoryError>) -> RepositoryError {
    match error {
        CandidateError::StaleBasis => RepositoryError::RevisionConflict,
        CandidateError::Owner(error) => error,
        CandidateError::OfferCapacity { .. }
        | CandidateError::ComparisonCapacity { .. }
        | CandidateError::ComparisonCountOverflow
        | CandidateError::CandidateCapacity { .. }
        | CandidateError::ByteCapacity { .. }
        | CandidateError::AllocationCapacity => RepositoryError::Capacity,
        CandidateError::StalePins | CandidateError::Duplicate { .. } => {
            RepositoryError::InvalidCandidate
        }
    }
}

pub(super) fn select(
    current: &Checkpoint,
    staged: &GameState,
    operation: OperationId,
) -> Result<Option<EntityId>, RepositoryError> {
    if staged.entities.len() > 512
        || staged.resources.len() > 512
        || staged.inventory.len() > 512
        || staged.members.len() > 3
        || staged.characters.len() > 2
    {
        return Err(RepositoryError::Capacity);
    }
    // The checkpoint basis/pins remain immutable during this one atomic decision.
    // The staged turn/resources are a working copy, never a committed observation.
    let basis = current.basis();
    let context = CandidateContext {
        basis: &basis,
        pins: current.pins(),
    };
    let scene_assault = staged.encounters.first().is_some_and(|encounter| {
        encounter
            .objectives
            .iter()
            .any(|objective| objective.entry.as_str() == "scene-npc-assault")
    });
    let policy = model::content(if scene_assault {
        "tactical-srd521-cover-policy"
    } else {
        "normal-nonlethal-melee"
    })?;
    let actor = combat_enemy(staged)?;
    let observation = NativeObservation {
        basis,
        actor,
        cause: operation,
        logical_time: staged.logical_time,
        policy: policy.clone(),
    };
    let owner = CurrentNativeTurn {
        current,
        staged,
        operation,
    };
    let offers = ENTITIES
        .into_iter()
        .map(|target| {
            Ok(NativeAttack {
                actor,
                target: entity(target)?,
                weapon: held_weapon_id(actor)?,
                action: model::content(&combat_weapon_item(staged, actor))?,
                source: rule()?,
            })
        })
        .collect::<Result<Vec<_>, RepositoryError>>()?;
    let admitted = enumerate_candidates(
        &owner,
        &observation,
        context,
        context,
        &offers,
        CandidateLimits {
            max_offers: 2,
            max_identity_comparisons: 1,
            max_candidates: 2,
            max_candidate_bytes: 4096,
        },
    )
    .map_err(admission_error)?;
    let ranking_observation = RankingObservation {
        basis: &basis,
        pins: current.pins(),
        observer: KnowledgeObserver::Entity(actor),
        logical_time: staged.logical_time,
        cause: operation,
        policy: &policy,
    };
    let assignments = admitted
        .offers()
        .iter()
        .map(|offer| UtilityAssignment {
            offer: *offer,
            contributions: &[],
        })
        .collect::<Vec<_>>();
    let knowledge = TacticalKnowledge {
        basis: &basis,
        pins: current.pins(),
        observer: KnowledgeObserver::Entity(actor),
        grants: &[],
        beliefs: &[],
        npcs: &[],
    };
    let policy = UtilityPolicy {
        basis: &basis,
        pins: current.pins(),
        definition: &policy,
        criteria: &[],
    };
    let ranked = rank_tactics(
        &ranking_observation,
        &admitted,
        &assignments,
        &knowledge,
        &policy,
        UtilityLimits {
            candidates: 2,
            contributions: 0,
            knowledge_records: 0,
            criteria: 0,
        },
    )
    .map_err(|_| RepositoryError::InvalidCandidate)?;
    let Some(chosen) = ranked.preferred() else {
        return Ok(None);
    };
    if !owner.is_current_legal(&observation, context, chosen.offer())? {
        return Err(RepositoryError::InvalidCandidate);
    }
    Ok(Some(chosen.offer().target))
}

#[cfg(test)]
mod tests {
    use super::super::{
        BANDIT, MEMBERS, accepted, enemy_turn, set, stage_with_supplier,
        tests::{input, opening_story},
        value,
    };
    use super::*;
    use df_types::MemberId;

    fn dialogue() -> Checkpoint {
        let current = opening_story();
        stage_with_supplier(
            &current,
            &input(
                &current,
                MemberId::from_bytes(&MEMBERS[0]).unwrap(),
                6,
                "escort-courier",
                vec![],
            ),
            &mut |_| panic!("dialogue cannot draw"),
        )
        .unwrap()
    }

    fn combat() -> Checkpoint {
        let current = dialogue();
        stage_with_supplier(
            &current,
            &input(
                &current,
                MemberId::from_bytes(&MEMBERS[0]).unwrap(),
                7,
                "defend-courier",
                vec![],
            ),
            &mut |_| Ok(10),
        )
        .unwrap()
    }

    fn native_state(current: &Checkpoint) -> GameState {
        let mut state = current.state().clone();
        state.encounters[0].active_turn = Some(entity(BANDIT).unwrap());
        state
    }

    fn operation() -> OperationId {
        OperationId::from_bytes(&[8; 16]).unwrap()
    }

    #[test]
    fn actual_bandit_first_start_uses_only_ordered_initiative_and_selected_attack_draws() {
        let current = dialogue();
        let before = current.clone();
        let member = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let command = input(&current, member, 7, "defend-courier", vec![]);
        let run = || {
            let mut faces = [(20, 1), (20, 1), (20, 20), (20, 20), (6, 6), (6, 6)].into_iter();
            let candidate = stage_with_supplier(&current, &command, &mut |sides| {
                let (expected, face) = faces.next().expect("no tactical random draws");
                assert_eq!(sides, expected);
                Ok(face)
            })
            .unwrap();
            assert_eq!(faces.next(), None);
            candidate
        };
        let result = run();
        assert_eq!(result, run(), "same source inputs replay identically");
        let outcome = accepted(result.state().decisions.last().unwrap()).unwrap();
        assert_eq!(outcome.combat.len(), 1);
        let attack = &outcome.combat[0];
        assert_eq!(attack.actor_id, BANDIT);
        assert_eq!(attack.target_id, ENTITIES[0]);
        assert_eq!(attack.damage_dice, [6, 6]);
        assert_eq!(attack.damage, 13);
        assert_eq!(attack.source_revision, rules::SOURCE_REVISION);
        assert!(attack.knocked_out);
        let monster = entity(BANDIT).unwrap();
        assert_eq!(value(result.state(), monster, "action-used"), Ok(1));
        assert_eq!(value(result.state(), monster, "bonus-used"), Ok(0));
        assert_eq!(
            result.state().encounters[0].active_turn,
            Some(entity(ENTITIES[1]).unwrap())
        );
        assert_eq!(
            result
                .state()
                .draws
                .iter()
                .map(|draw| draw.ordinal)
                .collect::<Vec<_>>(),
            [0, 1, 2, 3, 4, 5]
        );
        assert_eq!(result.state().logical_time, current.state().logical_time);
        assert_eq!(result.state().knowledge, current.state().knowledge);
        assert_eq!(current, before);
    }

    #[test]
    fn actual_end_turn_consumes_native_choice_and_preserves_other_resources() {
        let prepared = combat();
        // Canonical character storage order can differ from joined party order
        // when the second joined member finalizes their character first.
        let mut state = prepared.state().clone();
        state.characters.reverse();
        let current = model::checkpoint(prepared.basis(), state).unwrap();
        let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
        let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        let next = stage_with_supplier(
            &current,
            &input(&current, first, 8, "end-turn", vec![]),
            &mut |_| panic!("second player's turn cannot draw"),
        )
        .unwrap();
        let mut calls = 0;
        let result = stage_with_supplier(
            &next,
            &input(&next, second, 9, "end-turn", vec![]),
            &mut |sides| {
                calls += 1;
                assert_eq!(sides, 20);
                Ok(1)
            },
        )
        .unwrap();
        let decision = result.state().decisions.last().unwrap();
        let outcome = accepted(decision).unwrap();
        assert_eq!(calls, 1, "only a missed native attack, no selection roll");
        assert_eq!(outcome.combat[0].target_id, ENTITIES[0]);
        assert!(!outcome.combat[0].hit);
        assert_eq!(decision.draws, [0]);
        assert_eq!(result.state().draws.len(), 4);
        assert_eq!(
            result.state().encounters[0].turn_order,
            next.state().encounters[0].turn_order
        );
        assert_eq!(
            result.state().encounters[0].active_turn,
            Some(entity(ENTITIES[0]).unwrap())
        );
        assert_eq!(result.state().logical_time.ticks, 6);
        for resource in &next.state().resources {
            if !matches!(resource.resource.as_str(), "action-used" | "bonus-used") {
                assert!(result.state().resources.contains(resource));
            }
        }
        assert_eq!(result.state().knowledge, next.state().knowledge);
    }

    #[test]
    fn selector_omits_unconscious_or_nonadjacent_target_without_hidden_stat_weighting() {
        let current = combat();
        let first = entity(ENTITIES[0]).unwrap();
        let second = entity(ENTITIES[1]).unwrap();
        let mut staged = native_state(&current);
        // A weak second target does not change the authored stable ordering.
        set(&mut staged, second, "hit-points", 1).unwrap();
        assert_eq!(select(&current, &staged, operation()), Ok(Some(first)));
        set(&mut staged, first, "unconscious", 1).unwrap();
        assert_eq!(select(&current, &staged, operation()), Ok(Some(second)));
        set(&mut staged, first, "unconscious", 0).unwrap();
        staged
            .entities
            .iter_mut()
            .find(|entity| entity.id == first)
            .unwrap()
            .position = Some(Position { x: 0, y: 10, z: 0 });
        assert_eq!(select(&current, &staged, operation()), Ok(Some(second)));
        staged
            .entities
            .iter_mut()
            .find(|entity| entity.id == second)
            .unwrap()
            .location = None;
        assert_eq!(select(&current, &staged, operation()), Ok(None));
        assert_eq!(current.state().draws.len(), 3);
    }

    #[test]
    fn actual_native_no_action_does_not_attack_spend_or_draw_and_advances_existing_order() {
        let current = combat();
        for spent in [false, true] {
            let mut staged = native_state(&current);
            let monster = entity(BANDIT).unwrap();
            if spent {
                set(&mut staged, monster, "action-used", 1).unwrap();
            } else {
                set(&mut staged, monster, "held-weapon", 0).unwrap();
                let weapon = held_weapon_id(monster).unwrap();
                staged
                    .entities
                    .iter_mut()
                    .find(|entity| entity.id == weapon)
                    .unwrap()
                    .location = None;
            }
            let before_resources = staged.resources.clone();
            let mut draws = Vec::new();
            let mut outcomes = Vec::new();
            enemy_turn(
                &current,
                &mut staged,
                operation(),
                &mut draws,
                &mut outcomes,
                &mut |_| panic!("NoAction cannot roll"),
            )
            .unwrap();
            assert!(draws.is_empty());
            assert!(outcomes.is_empty());
            assert_eq!(staged.resources, before_resources);
            assert_eq!(
                staged.encounters[0].active_turn,
                Some(entity(ENTITIES[0]).unwrap())
            );
            assert_eq!(staged.logical_time.ticks, 6);
        }
    }

    #[test]
    fn actual_native_turn_rejects_actor_policy_and_source_corruption_before_draws() {
        let current = combat();
        let before = current.clone();
        for case in 0..6 {
            let mut staged = native_state(&current);
            let monster = entity(BANDIT).unwrap();
            match case {
                0 => {
                    staged
                        .entities
                        .iter_mut()
                        .find(|entity| entity.id == monster)
                        .unwrap()
                        .definition = model::content("joined-player").unwrap()
                }
                1 => staged.encounters[0].combat_policy = model::content("combat").unwrap(),
                2 => {
                    staged
                        .resources
                        .iter_mut()
                        .find(|resource| {
                            resource.owner == monster && resource.resource.as_str() == "action-used"
                        })
                        .unwrap()
                        .source = model::rule().unwrap()
                }
                3 => staged.resources.retain(|resource| {
                    !(resource.owner == monster && resource.resource.as_str() == "action-used")
                }),
                4 => staged.encounters[0].turn_order[1] = monster,
                _ => {
                    staged
                        .inventory
                        .iter_mut()
                        .find(|item| item.owner == monster)
                        .unwrap()
                        .source = model::rule().unwrap()
                }
            }
            let unchanged = staged.clone();
            let mut draws = Vec::new();
            let mut outcomes = Vec::new();
            assert_eq!(
                enemy_turn(
                    &current,
                    &mut staged,
                    operation(),
                    &mut draws,
                    &mut outcomes,
                    &mut |_| panic!("invalid native source cannot draw")
                ),
                Err(RepositoryError::InvalidCandidate)
            );
            assert_eq!(staged, unchanged);
            assert!(draws.is_empty());
            assert!(outcomes.is_empty());
        }
        assert_eq!(current, before);
    }

    #[test]
    fn native_observation_refuses_stale_basis_pins_operation_and_logical_time() {
        let current = combat();
        let staged = native_state(&current);
        let basis = current.basis();
        let context = CandidateContext {
            basis: &basis,
            pins: current.pins(),
        };
        let owner = CurrentNativeTurn {
            current: &current,
            staged: &staged,
            operation: operation(),
        };
        let mut observation = NativeObservation {
            basis,
            actor: entity(BANDIT).unwrap(),
            cause: operation(),
            logical_time: staged.logical_time,
            policy: model::content("normal-nonlethal-melee").unwrap(),
        };
        assert_eq!(owner.validate_observation(&observation, context), Ok(()));
        observation.basis.revision = basis.revision.next_sequence().unwrap();
        assert_eq!(
            owner.validate_observation(&observation, context),
            Err(RepositoryError::RevisionConflict)
        );
        observation.basis = basis;
        observation.cause = OperationId::from_bytes(&[9; 16]).unwrap();
        assert_eq!(
            owner.validate_observation(&observation, context),
            Err(RepositoryError::InvalidCandidate)
        );
        observation.cause = operation();
        observation.logical_time.ticks += 6;
        assert_eq!(
            owner.validate_observation(&observation, context),
            Err(RepositoryError::InvalidCandidate)
        );
        observation.logical_time = staged.logical_time;
        let mut pins = current.pins().clone();
        pins.content.content_digest.0[0] ^= 1;
        assert_eq!(
            owner.validate_observation(
                &observation,
                CandidateContext {
                    basis: &basis,
                    pins: &pins
                }
            ),
            Err(RepositoryError::InvalidCandidate)
        );
        let mut over_capacity = staged.clone();
        over_capacity.characters.push(staged.characters[0].clone());
        assert_eq!(
            select(&current, &over_capacity, operation()),
            Err(RepositoryError::Capacity)
        );
    }
}

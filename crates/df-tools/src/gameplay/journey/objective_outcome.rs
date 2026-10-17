//! The authored native combat objective uses the same accepted rules decision.
use df_encounter::objectives::{
    AuthoredObjectiveTransition, ObjectivePolicyError, ObjectivePolicyOwner,
    ObjectiveProposalLimits, ObjectiveProposalRequest, propose_objective_transitions,
};
use df_model::checkpoint::*;
use df_protocol::common as rpc;
use df_session::submission::RepositoryError;

use super::{
    ENCOUNTER, THREAD_POLICY, accepted, model, narrative_phase, offered, phase, player_entity,
    rule, value,
};

fn invalid<T>() -> Result<T, RepositoryError> {
    Err(RepositoryError::InvalidCandidate)
}

// This binds retained terminal identity to native command attribution. It is structural
// integrity, not authentication of independently rewritten accepted repository history.
fn identity(
    operation: df_types::OperationId,
    ordinal: u32,
    member: df_types::MemberId,
    actor: EntityId,
    outcome: &ContentReference,
    action: &ContentReference,
) -> Result<FactId, RepositoryError> {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(b"native-journey-objective");
    digest.update(operation.as_bytes());
    digest.update(ordinal.to_le_bytes());
    digest.update(member.as_bytes());
    digest.update(actor.as_bytes());
    for reference in [outcome, action] {
        for value in [&reference.package, &reference.entry] {
            digest.update(
                u64::try_from(value.as_str().len())
                    .map_err(super::bad)?
                    .to_le_bytes(),
            );
            digest.update(value.as_str().as_bytes());
        }
    }
    FactId::from_bytes(&digest.finalize()[..16]).map_err(super::bad)
}

pub(super) fn cause_id(
    command: &CommandInput,
    ordinal: u32,
    outcome: &ContentReference,
) -> Result<FactId, RepositoryError> {
    let GameCommand::ProposeAction { actor, action, .. } = &command.command else {
        return invalid();
    };
    identity(
        command.operation,
        ordinal,
        command.member,
        *actor,
        outcome,
        action,
    )
}

fn actors(state: &GameState) -> Result<Vec<EntityId>, RepositoryError> {
    let actors = state
        .members
        .iter()
        .filter(|link| link.member.as_bytes() != &super::PLAYER)
        .map(|link| link.character.ok_or(RepositoryError::InvalidCandidate))
        .collect::<Result<Vec<_>, _>>()?;
    if actors.len() != 2 || actors.first() == actors.get(1) {
        return invalid();
    }
    Ok(actors)
}

pub(super) fn pending(
    before: &Checkpoint,
    state: &GameState,
    command: &CommandInput,
) -> Result<Option<(ContentReference, EntityId)>, RepositoryError> {
    let [previous] = before.state().encounters.as_slice() else {
        return Ok(None);
    };
    // A scene assault in a freeplay scene ends by closing its encounter in that same scene
    // (`scene_fight::close`): there is no victory or defeat beat for an objective to move to.
    if super::scene_fight::open(before.state()) {
        return Ok(None);
    }
    let [encounter] = state.encounters.as_slice() else {
        return invalid();
    };
    // Rest, Inn and reads of already terminal checkpoints never request a proposal.
    if previous.active_turn.is_none() || encounter.active_turn.is_some() {
        return Ok(None);
    }
    let GameCommand::ProposeAction { actor, action, .. } = &command.command else {
        return invalid();
    };
    if !(encounter.objectives == [model::content("defend-courier")?]
        || encounter.objectives == [model::content("scene-npc-assault")?])
        || *actor != player_entity(command.member, before)?
        || !(if super::tactical::present(state) {
            super::wire::action_kind(action.entry.as_str())
                .is_some_and(super::tactical::combat_action)
        } else {
            ["greatsword-attack", "end-turn"].contains(&action.entry.as_str())
        })
    {
        return invalid();
    }
    if super::tactical::present(state) {
        return Ok(Some((
            model::content(if super::tactical::victory(state)? {
                "combat-victory"
            } else {
                "combat-defeat"
            })?,
            *actor,
        )));
    }
    let party = actors(state)?;
    let party_down = party
        .iter()
        .map(|actor| value(state, *actor, "unconscious"))
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .all(|value| *value == 1);
    let monster_down = value(state, super::combat_enemy(state)?, "unconscious")? == 1;
    if party_down == monster_down {
        return invalid();
    }
    Ok(Some((
        model::content(if monster_down {
            "combat-victory"
        } else {
            "combat-defeat"
        })?,
        *actor,
    )))
}

struct Terminal<'a> {
    cause: &'a GameFact,
    action: &'a GameFact,
    decision: &'a AcceptedDecision,
    actor: EntityId,
}

fn terminal(
    state: &GameState,
    victory: bool,
    require_narrative: bool,
) -> Result<Terminal<'_>, RepositoryError> {
    if state.facts.len() > 512 || state.decisions.len() > 512 {
        return Err(RepositoryError::Capacity);
    }
    let party = actors(state)?;
    let [encounter] = state.encounters.as_slice() else {
        return invalid();
    };
    if encounter.id != RecordId::from_bytes(&ENCOUNTER).map_err(super::bad)?
        || encounter.definition != model::content("combat")?
        || encounter.combat_policy
            != model::content(if super::tactical::present(state) {
                "tactical-srd521-cover-policy"
            } else {
                "normal-nonlethal-melee"
            })?
        || (!super::tactical::present(state)
            && encounter.participants
                != party
                    .iter()
                    .copied()
                    .chain([super::combat_enemy(state)?])
                    .collect::<Vec<_>>())
    {
        return invalid();
    }
    let start_entry = if encounter.objectives == [model::content("scene-npc-assault")?] {
        "scene-npc-assault"
    } else {
        "defend-courier"
    };
    let start = narrative_phase::source(state, start_entry, rpc::JourneyPhase::Combat)?;
    if start.audience != AudienceScope::Shared
        || !matches!(&start.value, FactValue::ContentEvent { subjects, .. } if subjects.is_empty())
    {
        return invalid();
    }
    // Reuse the existing accepted knockout/resource/source qualification. Before narrative
    // staging only the new last action is not yet on the narrative accepted-fact ledger.
    let action =
        narrative_phase::completion_source(state, start, &party, victory, require_narrative)?;
    let mut causes = state.facts.iter().filter(|fact| {
        matches!(&fact.value, FactValue::ContentEvent { definition, .. }
            if definition.entry.as_str() == "combat-victory" || definition.entry.as_str() == "combat-defeat")
    });
    let cause = causes.next().ok_or(RepositoryError::InvalidCandidate)?;
    if causes.next().is_some()
        || cause.operation != action.operation
        || cause.revision != action.revision
        || cause.revision <= start.revision
        || cause.ordinal.checked_add(1) != Some(action.ordinal)
        || action.cause != Some(cause.id)
        || cause.audience != AudienceScope::Shared
    {
        return invalid();
    }
    let FactValue::ContentEvent {
        definition,
        subjects,
    } = &cause.value
    else {
        return invalid();
    };
    let [actor] = subjects.as_slice() else {
        return invalid();
    };
    if !party.contains(actor)
        || *definition
            != model::content(if victory {
                "combat-victory"
            } else {
                "combat-defeat"
            })?
    {
        return invalid();
    }
    let owner = state
        .characters
        .iter()
        .find(|character| character.entity == *actor)
        .ok_or(RepositoryError::InvalidCandidate)?
        .owner;
    let FactValue::ContentEvent {
        definition: action_definition,
        ..
    } = &action.value
    else {
        return invalid();
    };
    if !state
        .members
        .iter()
        .any(|link| link.member == owner && link.character == Some(*actor))
        || cause.id
            != identity(
                cause.operation,
                cause.ordinal,
                owner,
                *actor,
                definition,
                action_definition,
            )?
    {
        return invalid();
    }
    let decision = state
        .decisions
        .iter()
        .find(|decision| {
            decision.operation == cause.operation && decision.revision == cause.revision
        })
        .ok_or(RepositoryError::InvalidCandidate)?;
    if decision.source_policy.as_str() != THREAD_POLICY
        || decision.facts.get(cause.ordinal as usize) != Some(&cause.id)
        || decision.facts.last() != Some(&action.id)
        || decision.facts.get(action.ordinal as usize) != Some(&action.id)
        || !accepted(decision)?
            .combat
            .iter()
            .all(|outcome| outcome.source_revision == model::source_revision())
    {
        return invalid();
    }
    let previous = cause
        .ordinal
        .checked_sub(1)
        .and_then(|ordinal| decision.facts.get(ordinal as usize))
        .and_then(|id| state.facts.iter().find(|fact| fact.id == *id))
        .ok_or(RepositoryError::InvalidCandidate)?;
    if previous.operation != cause.operation
        || previous.revision != cause.revision
        || previous.ordinal.checked_add(1) != Some(cause.ordinal)
        || cause.cause != Some(previous.id)
    {
        return invalid();
    }
    Ok(Terminal {
        cause,
        action,
        decision,
        actor: *actor,
    })
}

/// Legacy committed pairs retain their existing read semantics. Singleton results
/// require the original accepted Defend source and the qualified terminal receipt.
pub(super) fn outcome(state: &GameState) -> Result<bool, RepositoryError> {
    let [encounter] = state.encounters.as_slice() else {
        return invalid();
    };
    let defend = model::content("defend-courier")?;
    let assault = model::content("scene-npc-assault")?;
    for (entry, victory) in [("combat-victory", true), ("combat-defeat", false)] {
        let result = model::content(entry)?;
        if encounter.objectives == [defend.clone(), result.clone()]
            || encounter.objectives == [assault.clone(), result.clone()]
        {
            return Ok(victory);
        }
        if encounter.objectives == [result] {
            terminal(state, victory, true)?;
            return Ok(victory);
        }
    }
    invalid()
}

pub(super) fn defend_history(state: &GameState) -> Result<bool, RepositoryError> {
    let [encounter] = state.encounters.as_slice() else {
        return invalid();
    };
    if encounter.objectives == [model::content("defend-courier")?]
        || encounter.objectives == [model::content("scene-npc-assault")?]
    {
        return Ok(true);
    }
    outcome(state)?;
    Ok(true)
}

struct CurrentObjective<'a> {
    before: &'a Checkpoint,
    command: &'a CommandInput,
    victory: bool,
}

impl ObjectivePolicyOwner for CurrentObjective<'_> {
    fn admit_transition(
        &self,
        current: &Checkpoint,
        transition: &AuthoredObjectiveTransition<'_>,
    ) -> Result<(), ObjectivePolicyError> {
        let admitted = (|| -> Result<(), RepositoryError> {
            let pins = model::pins()?;
            self.before
                .validate_resume(self.command.basis, &pins)
                .map_err(super::bad)?;
            current
                .validate_resume(current.basis(), &pins)
                .map_err(super::bad)?;
            let mut expected_basis = self.before.basis();
            expected_basis.revision = expected_basis
                .revision
                .next_sequence()
                .map_err(super::bad)?;
            let GameCommand::ProposeAction {
                actor,
                action,
                targets,
                ..
            } = &self.command.command
            else {
                return invalid();
            };
            let assault_objective = model::content("scene-npc-assault")?;
            let cause = terminal(current.state(), self.victory, false)?;
            let kind = super::wire::action_kind(action.entry.as_str())
                .ok_or(RepositoryError::InvalidCandidate)?;
            if current.basis() != expected_basis
                || self.command.observed_revision != self.before.basis().revision
                || current.state().members != self.before.state().members
                || phase(self.before)? != rpc::JourneyPhase::Combat
                || !offered(self.before, self.command.member)?
                    .iter()
                    .any(|(offered, _)| *offered == kind)
                || (!super::tactical::present(current.state()) && !targets.is_empty())
                || *actor != player_entity(self.command.member, self.before)?
                || cause.actor != *actor
                || cause.cause.operation != self.command.operation
                || cause.cause.revision != expected_basis.revision
                || !matches!(&cause.action.value, FactValue::ContentEvent { definition, .. } if definition == action)
                || transition.encounter != RecordId::from_bytes(&ENCOUNTER).map_err(super::bad)?
                || transition.objective_index != 0
                || transition.expected
                    != &model::content(
                        if self
                            .before
                            .state()
                            .encounters
                            .first()
                            .is_some_and(|encounter| encounter.objectives == [assault_objective])
                        {
                            "scene-npc-assault"
                        } else {
                            "defend-courier"
                        },
                    )?
                || transition.replacement
                    != &model::content(if self.victory {
                        "combat-victory"
                    } else {
                        "combat-defeat"
                    })?
                || transition.policy != transition.expected
                || transition.source != &rule()?
                || transition.actor != *actor
                || transition.cause != cause.cause
                || current.state().decisions.last() != Some(cause.decision)
                || current.state().facts.get(..self.before.state().facts.len())
                    != Some(self.before.state().facts.as_slice())
                || current
                    .state()
                    .decisions
                    .get(..self.before.state().decisions.len())
                    != Some(self.before.state().decisions.as_slice())
                || current.state().decisions.len() != self.before.state().decisions.len() + 1
            {
                return invalid();
            }
            Ok(())
        })();
        admitted.map_err(|_| ObjectivePolicyError::PolicyMismatch)
    }
}

pub(super) fn stage(
    before: &Checkpoint,
    candidate: Checkpoint,
    command: &CommandInput,
) -> Result<Checkpoint, RepositoryError> {
    let last = candidate
        .state()
        .decisions
        .last()
        .ok_or(RepositoryError::InvalidCandidate)?;
    let enemy = super::combat_enemy(candidate.state())?;
    let victory = if super::tactical::present(candidate.state()) {
        super::tactical::victory(candidate.state())?
    } else {
        accepted(last)?
            .combat
            .iter()
            .any(|outcome| outcome.knocked_out && outcome.target_id.as_slice() == enemy.as_bytes())
    };
    let cause = terminal(candidate.state(), victory, false)?;
    let assault_objective = model::content("scene-npc-assault")?;
    let expected = if candidate
        .state()
        .encounters
        .first()
        .is_some_and(|encounter| encounter.objectives == [assault_objective])
    {
        model::content("scene-npc-assault")?
    } else {
        model::content("defend-courier")?
    };
    let replacement = model::content(if victory {
        "combat-victory"
    } else {
        "combat-defeat"
    })?;
    let source = rule()?;
    let transitions = [AuthoredObjectiveTransition {
        encounter: RecordId::from_bytes(&ENCOUNTER).map_err(super::bad)?,
        objective_index: 0,
        expected: &expected,
        replacement: &replacement,
        policy: &expected,
        source: &source,
        actor: cause.actor,
        cause: cause.cause,
    }];
    let rules = [model::rule()?, source.clone()];
    let pins = model::pins()?;
    let content = model::contents()?;
    let resources = super::resources()?;
    let proposal = propose_objective_transitions(
        &CurrentObjective {
            before,
            command,
            victory,
        },
        ObjectiveProposalRequest {
            current: &candidate,
            expected_basis: candidate.basis(),
            admitted_pins: &pins,
            inventory: ReferenceInventory {
                rules: &rules,
                content: &content,
                resources: &resources,
                assets: &[],
            },
            transitions: &transitions,
            checkpoint_limits: model::limits(),
            limits: ObjectiveProposalLimits {
                maximum_changes: 1,
                maximum_records: 512,
                maximum_comparisons: 4096,
                maximum_input_bytes: 2 * 1024 * 1024,
                maximum_output_bytes: 1024 * 1024,
            },
        },
    )
    .map_err(super::bad)?;
    Ok(proposal.checkpoint)
}

//! Current combat status must agree with the accepted authored encounter.
use df_model::checkpoint::{AudienceScope, Checkpoint, EntityId, FactValue, GameState};
use df_protocol::common as rpc;
use df_session::submission::RepositoryError;

use super::{
    BANDIT, THREAD_POLICY, accepted, combat_round, entity, model, objective_outcome, rule, value,
};

fn invalid<T>() -> Result<T, RepositoryError> {
    Err(RepositoryError::InvalidCandidate)
}

fn is_down(state: &GameState, actor: EntityId) -> Result<bool, RepositoryError> {
    let hit_points = value(state, actor, "hit-points")?;
    let unconscious = value(state, actor, "unconscious")?;
    let prone = value(state, actor, "prone")?;
    let held_weapon = value(state, actor, "held-weapon")?;
    if hit_points == 0 || unconscious > 1 || prone > 1 || held_weapon > 1 {
        return invalid();
    }
    if unconscious == 1 && (hit_points != 1 || prone != 1 || held_weapon != 0) {
        return invalid();
    }
    let mut sourced = 0i64;
    for fact in &state.facts {
        let FactValue::ResourceChanged {
            entity,
            resource,
            before,
            after,
            source,
        } = &fact.value
        else {
            continue;
        };
        if *entity != actor || resource.as_str() != "unconscious" {
            continue;
        }
        if fact.audience != AudienceScope::Shared
            || *source != rule()?
            || *before != sourced
            || !matches!((*before, *after), (0, 1) | (1, 0))
        {
            return invalid();
        }
        let decision = state
            .decisions
            .iter()
            .find(|decision| {
                decision.operation == fact.operation
                    && decision.revision == fact.revision
                    && decision.facts.get(fact.ordinal as usize) == Some(&fact.id)
                    && decision.source_policy.as_str() == THREAD_POLICY
            })
            .ok_or(RepositoryError::InvalidCandidate)?;
        let receipt = accepted(decision).map_err(|_| RepositoryError::InvalidCandidate)?;
        if *after == 1 {
            if !receipt.combat.iter().any(|outcome| {
                outcome.knocked_out && outcome.target_id.as_slice() == actor.as_bytes()
            }) {
                return invalid();
            }
        } else {
            let terminal = state
                .facts
                .iter()
                .find(|event| Some(&event.id) == decision.facts.last())
                .ok_or(RepositoryError::InvalidCandidate)?;
            if receipt.phase != rpc::JourneyPhase::Complete as i32
                || !matches!(&terminal.value, FactValue::ContentEvent { definition, .. }
                    if *definition == model::content("short-rest")?)
            {
                return invalid();
            }
        }
        sourced = *after;
    }
    if sourced != i64::from(unconscious) {
        return invalid();
    }
    Ok(unconscious == 1)
}

pub(super) fn validate(
    current: &Checkpoint,
    phase: rpc::JourneyPhase,
) -> Result<(), RepositoryError> {
    if !matches!(
        phase,
        rpc::JourneyPhase::Combat | rpc::JourneyPhase::Complete
    ) {
        return Ok(());
    }
    if super::tactical::present(current.state()) {
        return super::tactical::validate_encounter(current);
    }
    // The checkpoint supplies source-qualified resource bounds. The accepted initiative
    // draws and time facts supply the order and elapsed rounds.
    combat_round(current).map_err(|_| RepositoryError::InvalidCandidate)?;
    let state = current.state();
    let [encounter] = state.encounters.as_slice() else {
        return invalid();
    };
    let Some((monster, party)) = encounter.participants.split_last() else {
        return invalid();
    };
    let expected_party = super::participants(current)
        .into_iter()
        .map(|link| link.character.ok_or(RepositoryError::InvalidCandidate))
        .collect::<Result<Vec<_>, _>>()?;
    if !(1..=super::MAXIMUM_PARTY).contains(&party.len())
        || party != expected_party
        || *monster != entity(BANDIT)?
    {
        return invalid();
    }
    let mut party_down = true;
    for hero in party {
        party_down &= is_down(state, *hero)?;
    }
    let monster_down = is_down(state, *monster)?;
    let objective = model::content("defend-courier")?;
    if phase == rpc::JourneyPhase::Combat {
        let Some(active) = encounter.active_turn else {
            return invalid();
        };
        if encounter.objectives != [objective]
            || monster_down
            || party_down
            || !encounter.turn_order.contains(&active)
            || is_down(state, active)?
        {
            return invalid();
        }
        return Ok(());
    }
    let outcome = objective_outcome::outcome(state)?;
    if encounter.active_turn.is_some() {
        return invalid();
    }
    // An accepted short rest can end knockout after the recorded combat result. Before
    // that rest, the current actors must still express the terminal combat condition.
    if !state
        .narrative
        .completed_beats
        .contains(&model::content("short-rest-complete")?)
        && ((outcome && (!monster_down || party_down))
            || (!outcome && (monster_down || !party_down)))
    {
        return invalid();
    }
    Ok(())
}

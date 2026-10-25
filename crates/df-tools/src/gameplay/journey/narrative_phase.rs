//! Explicit prerequisites for the currently authored native journey, after causal staging.
use df_model::checkpoint::*;
use df_protocol::common as rpc;
use df_session::submission::RepositoryError;

use super::{
    BANDIT, ENCOUNTER, PACKET_THREAD, THREAD_POLICY, THREAT_THREAD, accepted, combat_victory,
    entity, model, objective_outcome, participants, refusal_detail, rule,
};

fn invalid<T>() -> Result<T, RepositoryError> {
    crate::gameplay::refusal_detail::note("narrative-phase");
    Err(RepositoryError::InvalidCandidate)
}

pub(super) fn classify(state: &GameState) -> Result<rpc::JourneyPhase, RepositoryError> {
    let [beat] = state.narrative.active_beats.as_slice() else {
        return invalid();
    };
    if *beat != model::content(beat.entry.as_str())? {
        return invalid();
    }
    match beat.entry.as_str() {
        "room" => Ok(rpc::JourneyPhase::Room),
        "opening" => Ok(rpc::JourneyPhase::Opening),
        "courier-answer-seal" | "courier-answer-escort" => Ok(rpc::JourneyPhase::Dialogue),
        "combat" => Ok(rpc::JourneyPhase::Combat),
        "complete" | "defeat" | "harbor-inn" | "epilogue" | "the-end" => {
            Ok(rpc::JourneyPhase::Complete)
        }
        _ => invalid(),
    }
}

// A named beat and an arbitrary content fact are insufficient. The source must be the
// terminal event of its accepted native decision and actually consumed by narrative.
pub(super) fn source<'a>(
    state: &'a GameState,
    entry: &str,
    phase: rpc::JourneyPhase,
) -> Result<&'a GameFact, RepositoryError> {
    let definition = model::content(entry)?;
    let courier = model::content(super::super::courier_ai::DEFINITION)?;
    let mut sources = state.facts.iter().filter(|fact| {
        matches!(&fact.value, FactValue::ContentEvent { definition: actual, .. }
            if *actual == definition)
            || (entry == "private-courier-note"
                && state.intents.iter().any(|intent| {
                    intent.operation == fact.operation
                        && intent.basis.revision == fact.revision
                        && intent.kind == EffectKind::RunAi
                        && intent.definition == courier
                        && state.decisions.iter().any(|decision| {
                            decision.operation == fact.operation
                                && decision.revision == fact.revision
                                && decision.effects == [intent.id]
                                && decision.facts.last() == Some(&fact.id)
                        })
                }))
    });
    let Some(fact) = sources.next() else {
        return invalid();
    };
    if sources.next().is_some() || !state.narrative.accepted_facts.contains(&fact.id) {
        return invalid();
    }
    let decision = state
        .decisions
        .iter()
        .find(|decision| {
            decision.operation == fact.operation
                && decision.revision == fact.revision
                && decision.source_policy.as_str() == THREAD_POLICY
                && decision.facts.last() == Some(&fact.id)
                && decision.facts.get(fact.ordinal as usize) == Some(&fact.id)
        })
        .ok_or(RepositoryError::InvalidCandidate)?;
    if accepted(decision)?.phase != phase as i32 {
        return invalid();
    }
    // The inn explicitly cites the historical victory; validate() checks that exception.
    if entry != "harbor-inn" {
        validate_terminal_cause(state, fact, decision)?;
    }
    Ok(fact)
}

pub(super) fn validate_terminal_cause(
    state: &GameState,
    fact: &GameFact,
    decision: &AcceptedDecision,
) -> Result<(), RepositoryError> {
    let index = state
        .facts
        .iter()
        .position(|record| record.id == fact.id)
        .ok_or(RepositoryError::InvalidCandidate)?;
    let preceding = index
        .checked_sub(1)
        .and_then(|index| state.facts.get(index));
    // finish() appends the source after its own facts; without those, it cites the
    // latest prior checkpoint fact, which may be a courier/prepared event.
    if fact.ordinal > 0 {
        let previous = preceding.ok_or(RepositoryError::InvalidCandidate)?;
        if previous.operation != fact.operation
            || previous.revision != fact.revision
            || previous.ordinal.checked_add(1) != Some(fact.ordinal)
            || decision.facts.get(fact.ordinal as usize - 1) != Some(&previous.id)
        {
            return invalid();
        }
    } else if preceding.is_some_and(|previous| previous.revision >= fact.revision) {
        return invalid();
    }
    if decision.facts.get(fact.ordinal as usize) != Some(&fact.id)
        || fact.cause != preceding.map(|previous| previous.id)
    {
        return invalid();
    }
    Ok(())
}

fn is_shared_empty(fact: &GameFact) -> bool {
    fact.audience == AudienceScope::Shared
        && matches!(&fact.value, FactValue::ContentEvent { subjects, .. } if subjects.is_empty())
}

// Historical narrative progress survives disclosure changes. A new encounter offer
// still requires the current source shape admitted by encounter_admission.
pub(super) fn courier_encounter_permitted(current: &Checkpoint) -> Result<bool, RepositoryError> {
    let state = current.state();
    if state.narrative.active_beats == [model::content("courier-answer-escort")?] {
        return Ok(is_shared_empty(source(
            state,
            "escort-courier",
            rpc::JourneyPhase::Dialogue,
        )?));
    }
    if state.narrative.active_beats != [model::content("courier-answer-seal")?] {
        return Ok(false);
    }
    let note = source(state, "private-courier-note", rpc::JourneyPhase::Dialogue)?;
    let definition = model::content("private-courier-note")?;
    Ok(
        matches!(&note.value, FactValue::ContentEvent { definition: actual, subjects }
        if *actual == definition && subjects.is_empty())
            && matches!(&note.audience, AudienceScope::Members(members)
            if members.len() == 1 && members.first().is_some_and(|member|
                participants(current).into_iter().any(|link|
                    link.member == *member && link.character.is_some()))),
    )
}

pub(super) fn validate(current: &Checkpoint) -> Result<rpc::JourneyPhase, RepositoryError> {
    let phase =
        validate_phase(current).inspect_err(|_| refusal_detail::note("narrative-phase-state"))?;
    validate_thread_progress(current, phase)
        .inspect_err(|_| refusal_detail::note("narrative-thread-progress"))?;
    Ok(phase)
}

// Phase markers alone do not establish ownership of the narrative ledger. Qualify the
// terminal native sources before offers or the registered rules handler can consume it.
fn validate_thread_progress(
    current: &Checkpoint,
    phase: rpc::JourneyPhase,
) -> Result<(), RepositoryError> {
    use df_narrative::{
        ProgressLimits, ThreadCheckpointRequest, ThreadConsequenceSelection, ThreadDisposition,
        stage_checkpoint_thread_progress,
    };

    let state = current.state();
    let packet = model::content(PACKET_THREAD)?;
    let threat = model::content(THREAT_THREAD)?;
    let mut threads = Vec::new();
    if phase != rpc::JourneyPhase::Room {
        threads.push(packet.clone());
    }
    if phase == rpc::JourneyPhase::Combat
        || (phase == rpc::JourneyPhase::Complete && !combat_victory(state)?)
    {
        threads.push(threat.clone());
    }
    if state.narrative.open_threads != threads {
        return invalid();
    }
    let private = if state
        .narrative
        .active_beats
        .iter()
        .chain(&state.narrative.completed_beats)
        .any(|beat| beat.entry.as_str() == "courier-answer-seal")
    {
        Some(source(state, "private-courier-note", rpc::JourneyPhase::Dialogue)?.id)
    } else {
        None
    };
    let events = super::THREAD_EVENTS
        .iter()
        .map(|(event, thread)| Ok((model::content(event)?, model::content(thread)?)))
        .collect::<Result<Vec<_>, RepositoryError>>()?;
    let mut sources = Vec::new();
    let mut last = None;
    for fact in &state.facts {
        let definition = match &fact.value {
            FactValue::ContentEvent { definition, .. } => Some(definition),
            _ => None,
        };
        let thread = if Some(fact.id) == private {
            Some(&packet)
        } else {
            events
                .iter()
                .find_map(|(event, thread)| (Some(event) == definition).then_some(thread))
        };
        let Some(thread) = thread else {
            continue;
        };
        if super::after_victory_time(state, fact) {
            continue;
        }
        let decision = state
            .decisions
            .iter()
            .find(|decision| {
                decision.operation == fact.operation
                    && decision.revision == fact.revision
                    && decision.facts.last() == Some(&fact.id)
            })
            .ok_or(RepositoryError::InvalidCandidate)?;
        let receipt = accepted(decision).map_err(|_| RepositoryError::InvalidCandidate)?;
        if definition.is_none_or(|event| event.entry.as_str() != "harbor-inn") {
            validate_terminal_cause(state, fact, decision)?;
        }
        let resolved = thread == &threat
            && (if super::tactical::present(state) {
                state.facts.iter().any(|source| source.operation == fact.operation && source.revision == fact.revision
                    && matches!(&source.value, FactValue::ContentEvent {definition, ..} if definition.entry.as_str() == "combat-victory"))
            } else {
                receipt.combat.iter().any(|outcome| {
                    outcome.knocked_out && outcome.target_id.as_slice() == BANDIT.as_slice()
                })
            });
        sources.push(fact.id);
        last = definition.map(|definition| ThreadConsequenceSelection {
            thread,
            event_definition: definition,
            source: fact.id,
            disposition: if resolved {
                ThreadDisposition::Resolve
            } else {
                ThreadDisposition::Continue
            },
        });
    }
    if state.narrative.accepted_facts != sources {
        return invalid();
    }

    // Re-stage only the last admitted consequence, not a replay of historical rules.
    // Current private payload replacement can remove its ContentEvent shape; in that case
    // the reducer validates the unchanged ledger without restoring revoked disclosure.
    let mut before = state.clone();
    if let Some(selection) = &last {
        if before.narrative.accepted_facts.pop() != Some(selection.source) {
            return invalid();
        }
        if selection.disposition == ThreadDisposition::Resolve {
            before.narrative.open_threads.push(selection.thread.clone());
        }
    }
    let before = model::checkpoint(current.basis(), before)?;
    let policy = model::label(THREAD_POLICY)?;
    let proposed = stage_checkpoint_thread_progress(
        &before,
        ThreadCheckpointRequest {
            expected_basis: current.basis(),
            admitted_pins: current.pins(),
            policy: &policy,
            expected_policy: &policy,
            inventory: ReferenceInventory {
                rules: &[model::rule()?, rule()?],
                content: &model::contents()?,
                resources: &super::resources_for(before.state())?,
                assets: &crate::gameplay::model::creator_portrait_assets(before.state())?,
            },
            checkpoint_limits: model::limits(),
            selections: last.as_slice(),
        },
        ProgressLimits {
            records: 512,
            consequences: 1,
            work: 1024 * 1024,
        },
    )
    .map_err(|_| RepositoryError::InvalidCandidate)?;
    if proposed.checkpoint != *current {
        return invalid();
    }
    Ok(())
}

fn validate_phase(current: &Checkpoint) -> Result<rpc::JourneyPhase, RepositoryError> {
    current
        .validate_resume(current.basis(), &model::pins()?)
        .map_err(|_| RepositoryError::InvalidCandidate)?;
    let state = current.state();
    let phase = classify(state)?;
    // The same scan bound as the other journey reads: an oversized state is a capacity limit.
    if state.facts.len() > 512 || state.decisions.len() > 512 {
        return Err(RepositoryError::Capacity);
    }
    if state.narrative.definition != model::content("room")? {
        return invalid();
    }
    if phase == rpc::JourneyPhase::Room {
        if !state.narrative.completed_beats.is_empty()
            || !state.narrative.open_threads.is_empty()
            || !state.narrative.accepted_facts.is_empty()
            || !state.encounters.is_empty()
        {
            return invalid();
        }
        return Ok(phase);
    }
    let party = participants(current);
    if !super::party_ready(current) {
        return invalid();
    }
    let build = model::content("dwarf-fighter-soldier")?;
    let actors = party
        .iter()
        .map(|link| {
            let who = link.character.ok_or(RepositoryError::InvalidCandidate)?;
            if !state.characters.iter().any(|character| {
                character.entity == who
                    && character.owner == link.member
                    && character.build == build
            }) {
                return invalid();
            }
            Ok(who)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if actors
        .iter()
        .enumerate()
        .any(|(index, actor)| actors[..index].contains(actor))
    {
        return invalid();
    }
    let opening = source(state, "begin-story", rpc::JourneyPhase::Opening)?;
    if !is_shared_empty(opening)
        || !state
            .narrative
            .open_threads
            .contains(&model::content(PACKET_THREAD)?)
    {
        return invalid();
    }
    let mut completed = vec![model::content("room")?];
    if phase == rpc::JourneyPhase::Opening {
        if state.narrative.completed_beats != completed || !state.encounters.is_empty() {
            return invalid();
        }
        return Ok(phase);
    }
    completed.push(model::content("opening")?);
    let answer = if state
        .narrative
        .active_beats
        .contains(&model::content("courier-answer-seal")?)
        || state
            .narrative
            .completed_beats
            .contains(&model::content("courier-answer-seal")?)
    {
        "courier-answer-seal"
    } else {
        "courier-answer-escort"
    };
    let dialogue = source(
        state,
        if answer == "courier-answer-seal" {
            "private-courier-note"
        } else {
            "escort-courier"
        },
        rpc::JourneyPhase::Dialogue,
    )?;
    // Current private disclosure selection may be revoked or replaced. Its accepted
    // story outcome remains; wire and encounter admission validate current disclosure.
    let permitted = answer == "courier-answer-seal" || is_shared_empty(dialogue);
    if !permitted || dialogue.revision <= opening.revision {
        return invalid();
    }
    if phase == rpc::JourneyPhase::Dialogue {
        if state.narrative.completed_beats != completed || !state.encounters.is_empty() {
            return invalid();
        }
        return Ok(phase);
    }
    completed.push(model::content(answer)?);
    let start = source(state, "defend-courier", rpc::JourneyPhase::Combat)?;
    let [encounter] = state.encounters.as_slice() else {
        return invalid();
    };
    if super::tactical::present(state) {
        super::tactical::validate_encounter(current)?;
        if phase == rpc::JourneyPhase::Combat {
            super::tactical::validate_phase(current, phase, &completed, start, dialogue)?;
            return Ok(phase);
        }
    }
    let monster = entity(BANDIT)?;
    let expected = actors.iter().copied().chain([monster]).collect::<Vec<_>>();
    if !super::tactical::present(state)
        && (!is_shared_empty(start)
            || start.revision <= dialogue.revision
            || encounter.id
                != RecordId::from_bytes(&ENCOUNTER)
                    .map_err(|_| RepositoryError::InvalidCandidate)?
            || encounter.definition != model::content("combat")?
            || encounter.combat_policy != model::content("normal-nonlethal-melee")?
            || encounter.participants != expected
            || !objective_outcome::defend_history(state)?)
    {
        return invalid();
    }
    if phase == rpc::JourneyPhase::Combat {
        if state.narrative.completed_beats != completed
            || encounter.active_turn.is_none()
            || encounter.objectives != [model::content("defend-courier")?]
            || !state
                .narrative
                .open_threads
                .contains(&model::content(THREAT_THREAD)?)
        {
            return invalid();
        }
        return Ok(phase);
    }
    completed.push(model::content("combat")?);
    let victory = combat_victory(state)?;
    if state
        .narrative
        .open_threads
        .contains(&model::content(THREAT_THREAD)?)
        == victory
    {
        return invalid();
    }
    let end = completion_source(state, start, &actors, victory, true)?;
    // A fight's ending steps follow the inn (after a victory) or the defeat scene.
    let active = state.narrative.active_beats[0].entry.as_str();
    let ending = matches!(active, "epilogue" | "the-end");
    if ending && !victory {
        completed.push(model::content("complete")?);
    }
    if active == "harbor-inn" || (ending && victory) {
        completed.push(model::content("complete")?);
        let inn = source(state, "harbor-inn", rpc::JourneyPhase::Complete)?;
        if !victory
            || inn.revision <= end.revision
            || inn.cause != Some(end.id)
            || inn.audience != AudienceScope::Shared
            || !matches!(&inn.value, FactValue::ContentEvent { subjects, .. } if *subjects == actors)
        {
            return invalid();
        }
        let decision = state
            .decisions
            .iter()
            .find(|decision| decision.operation == inn.operation)
            .ok_or(RepositoryError::InvalidCandidate)?;
        if accepted(decision)?.scene != Some(super::inn_scene()) {
            return invalid();
        }
    }
    if ending {
        if victory {
            completed.push(model::content("harbor-inn")?);
        }
        if active == "the-end" {
            completed.push(model::content("epilogue")?);
        }
    }
    ending_steps(
        state,
        end,
        if active == "the-end" {
            if super::tactical::all_heroes_dead(state)? {
                0
            } else {
                2
            }
        } else {
            usize::from(ending)
        },
    )?;
    let rest = model::content("short-rest-complete")?;
    let markers = state
        .narrative
        .completed_beats
        .iter()
        .filter(|beat| **beat == rest)
        .count();
    if markers > 1 || (markers == 1 && !accepted_rest(state, end, &actors)?) {
        return invalid();
    }
    let actual_completed = state
        .narrative
        .completed_beats
        .iter()
        .filter(|beat| **beat != rest)
        .cloned()
        .collect::<Vec<_>>();
    if actual_completed != completed {
        return invalid();
    }
    Ok(phase)
}

/// The ending steps taken since the fight ended: one resolution, then the epilogue's close. Each
/// is its own accepted decision whose terminal event is the step marker.
fn ending_steps(
    state: &GameState,
    completion: &GameFact,
    expected: usize,
) -> Result<(), RepositoryError> {
    let step = model::content(super::super::beat_script_runtime::STEP)?;
    let mut found = 0;
    for decision in &state.decisions {
        if decision.source_policy.as_str() != THREAD_POLICY
            || decision.revision <= completion.revision
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
        if !matches!(&event.value, FactValue::ContentEvent { definition, subjects }
            if *definition == step && subjects.is_empty())
        {
            continue;
        }
        let receipt = accepted(decision)?;
        if event.audience != AudienceScope::Shared
            || receipt.phase != rpc::JourneyPhase::Complete as i32
            || receipt.scene.is_some()
            || !receipt.combat.is_empty()
        {
            return invalid();
        }
        validate_terminal_cause(state, event, decision)?;
        found += 1;
    }
    if found == expected { Ok(()) } else { invalid() }
}

fn accepted_rest(
    state: &GameState,
    completion: &GameFact,
    actors: &[EntityId],
) -> Result<bool, RepositoryError> {
    let terminal = model::content("short-rest")?;
    let start = model::content("short-rest-start")?;
    for decision in &state.decisions {
        if decision.source_policy.as_str() != THREAD_POLICY
            || decision.revision <= completion.revision
        {
            continue;
        }
        let Some(event) = state.facts.iter().find(|fact| {
            decision.facts.last() == Some(&fact.id)
                && matches!(&fact.value, FactValue::ContentEvent { definition, .. } if *definition == terminal)
        }) else {
            continue;
        };
        let advancement = state.facts.iter().find(|fact| {
            fact.operation == event.operation && fact.revision == event.revision
                && fact.audience == AudienceScope::Shared
                && decision.facts.get(fact.ordinal as usize) == Some(&fact.id)
                && matches!(&fact.value, FactValue::TimeAdvanced { before, after }
                    if before.ticks_per_second != 0
                        && before.ticks_per_second == after.ticks_per_second
                        && after.ticks.checked_sub(before.ticks) == Some(u64::from(before.ticks_per_second) * u64::from(super::pack::rest().short_rest_seconds))
                        && after.ticks_per_second == state.logical_time.ticks_per_second
                        && after.ticks <= state.logical_time.ticks)
        });
        let Some(advancement) = advancement else {
            continue;
        };
        validate_terminal_cause(state, advancement, decision).map_err(|_| {
            crate::gameplay::refusal_detail::note("rest-time-advance");
            RepositoryError::InvalidCandidate
        })?;
        if !is_shared_empty(event) {
            return invalid();
        }
        let receipt = accepted(decision).map_err(|_| {
            crate::gameplay::refusal_detail::note("rest-completion-receipt");
            RepositoryError::InvalidCandidate
        })?;
        if receipt.phase != rpc::JourneyPhase::Complete as i32 {
            return invalid();
        }
        validate_terminal_cause(state, event, decision).map_err(|_| {
            crate::gameplay::refusal_detail::note("rest-completion-terminal");
            RepositoryError::InvalidCandidate
        })?;
        let completion_index = state
            .facts
            .iter()
            .position(|fact| fact.id == completion.id)
            .ok_or_else(|| {
                crate::gameplay::refusal_detail::note("rest-completion-source");
                RepositoryError::InvalidCandidate
            })?;
        let event_index = state
            .facts
            .iter()
            .position(|fact| fact.id == event.id)
            .ok_or_else(|| {
                crate::gameplay::refusal_detail::note("rest-completion-event");
                RepositoryError::InvalidCandidate
            })?;
        let began = state
            .facts
            .iter()
            .enumerate()
            .find(|(index, fact)| {
                *index > completion_index
                    && *index < event_index
                    && fact.audience == AudienceScope::Shared
                    && matches!(&fact.value, FactValue::ContentEvent { definition, subjects }
                    if *definition == start && subjects.len() == actors.len()
                        && actors.iter().all(|actor| subjects.contains(actor)))
            })
            .map(|(_, fact)| fact)
            .ok_or_else(|| {
                crate::gameplay::refusal_detail::note("rest-start-event");
                RepositoryError::InvalidCandidate
            })?;
        let started = state
            .decisions
            .iter()
            .find(|started| {
                started.operation == began.operation
                    && started.revision == began.revision
                    && started.facts.get(began.ordinal as usize) == Some(&began.id)
            })
            .ok_or_else(|| {
                crate::gameplay::refusal_detail::note("rest-start-decision");
                RepositoryError::InvalidCandidate
            })?;
        let start_event = state
            .facts
            .iter()
            .find(|fact| started.facts.last() == Some(&fact.id))
            .ok_or_else(|| {
                crate::gameplay::refusal_detail::note("rest-start-terminal");
                RepositoryError::InvalidCandidate
            })?;
        if started.source_policy.as_str() != THREAD_POLICY
            || !matches!(&start_event.value, FactValue::ContentEvent { definition, .. }
                if *definition == terminal)
        {
            return invalid();
        }
        let start_receipt = accepted(started).map_err(|_| {
            crate::gameplay::refusal_detail::note("rest-start-receipt");
            RepositoryError::InvalidCandidate
        })?;
        if start_receipt.phase != rpc::JourneyPhase::Complete as i32 {
            return invalid();
        }
        validate_terminal_cause(state, began, started).map_err(|_| {
            crate::gameplay::refusal_detail::note("rest-start-cause");
            RepositoryError::InvalidCandidate
        })?;
        validate_terminal_cause(state, start_event, started).map_err(|_| {
            crate::gameplay::refusal_detail::note("rest-start-terminal-cause");
            RepositoryError::InvalidCandidate
        })?;
        return Ok(true);
    }
    Ok(false)
}

pub(super) fn completion_source<'a>(
    state: &'a GameState,
    start: &GameFact,
    actors: &[EntityId],
    victory: bool,
    require_narrative: bool,
) -> Result<&'a GameFact, RepositoryError> {
    if super::tactical::present(state) {
        return super::tactical::completion_source(state, start, victory, require_narrative);
    }
    let monster = entity(BANDIT)?;
    let source_rule = rule()?;
    for decision in &state.decisions {
        if decision.source_policy.as_str() != THREAD_POLICY || decision.revision <= start.revision {
            continue;
        }
        let receipt = accepted(decision)?;
        if receipt.phase != rpc::JourneyPhase::Complete as i32 {
            continue;
        }
        let Some(event) = state
            .facts
            .iter()
            .find(|fact| decision.facts.last() == Some(&fact.id))
        else {
            continue;
        };
        if !is_shared_empty(event)
            || (require_narrative && !state.narrative.accepted_facts.contains(&event.id))
            || !matches!(&event.value, FactValue::ContentEvent { definition, .. }
                if *definition == model::content("greatsword-attack")? || *definition == model::content("end-turn")?)
        {
            continue;
        }
        validate_terminal_cause(state, event, decision)?;
        let defeated = |who: EntityId| {
            receipt.combat.iter().any(|outcome| outcome.knocked_out && outcome.target_id.as_slice() == who.as_bytes())
                && state.facts.iter().any(|fact| {
                    fact.operation == event.operation && fact.revision == event.revision
                        && fact.ordinal < event.ordinal && decision.facts.contains(&fact.id)
                        && fact.audience == AudienceScope::Shared
                        && matches!(&fact.value, FactValue::ResourceChanged { entity, resource, before: 0, after: 1, source }
                            if *entity == who && resource.as_str() == "unconscious" && *source == source_rule)
                })
        };
        let finished = if victory {
            defeated(monster)
        } else {
            actors.iter().copied().any(defeated)
                && actors.iter().all(|who| {
                    let last = state.facts.iter().filter(|fact| {
                        (fact.revision, fact.ordinal) < (event.revision, event.ordinal)
                            && matches!(&fact.value, FactValue::ResourceChanged { entity, resource, .. }
                                if entity == who && resource.as_str() == "unconscious")
                    }).max_by_key(|fact| (fact.revision, fact.ordinal));
                    last.is_some_and(|fact| {
                        matches!(&fact.value, FactValue::ResourceChanged { before: 0, after: 1, source, .. }
                            if *source == source_rule)
                            && state.decisions.iter().any(|owner| {
                                owner.operation == fact.operation && owner.revision == fact.revision
                                    && owner.facts.contains(&fact.id)
                                    && accepted(owner).is_ok_and(|receipt| receipt.combat.iter().any(|outcome|
                                        outcome.knocked_out && outcome.target_id.as_slice() == who.as_bytes()))
                            })
                    })
                })
        };
        if finished {
            return Ok(event);
        }
    }
    invalid()
}

#[cfg(test)]
mod tests {
    use super::super::{
        MEMBERS, bootstrap_member, initial, offered, phase, stage_join, stage_with_supplier,
        tests::{
            build, gaining_victory, inn_command, inn_victory, input, opening_story, prepared_story,
        },
    };
    use super::*;
    use df_types::MemberId;
    use prost::Message;

    fn first() -> MemberId {
        MemberId::from_bytes(&MEMBERS[0]).unwrap()
    }

    fn dialogue(entry: &str) -> Checkpoint {
        let opening = opening_story();
        stage_with_supplier(
            &opening,
            &input(&opening, first(), 6, entry, vec![]),
            &mut |_| panic!("dialogue does not roll"),
        )
        .unwrap()
    }

    fn combat(current: &Checkpoint) -> Checkpoint {
        stage_with_supplier(
            current,
            &input(current, first(), 7, "defend-courier", vec![]),
            &mut |_| Ok(10),
        )
        .unwrap()
    }

    fn refuses(current: &Checkpoint, command: GameInput) {
        let before = current.clone();
        let mut draws = 0;
        assert_eq!(
            stage_with_supplier(current, &command, &mut |_| {
                draws += 1;
                Ok(10)
            }),
            Err(RepositoryError::InvalidCandidate)
        );
        assert_eq!(draws, 0);
        assert_eq!(current, &before);
    }

    #[test]
    fn actual_stage_rejects_forged_phase_references_before_any_draw_or_publication() {
        let ready = prepared_story();
        let opening = opening_story();
        let private = dialogue("ask-courier");
        let combat = combat(&private);
        let victory = inn_victory();
        for (current, beat, action) in [
            (&ready, "opening", "ask-courier"),
            (&ready, "complete", "short-rest"),
            (&opening, "courier-answer-seal", "defend-courier"),
            (&opening, "dialogue", "defend-courier"),
            (&private, "combat", "end-turn"),
            (&combat, "complete", "short-rest"),
            (&victory, "harbor-inn", "short-rest"),
        ] {
            let mut state = current.state().clone();
            state.narrative.active_beats = vec![model::content(beat).unwrap()];
            let forged = model::checkpoint(current.basis(), state).unwrap();
            assert_eq!(
                phase(&forged),
                Err(RepositoryError::InvalidCandidate),
                "beat {beat}"
            );
            assert!(offered(&forged, first()).is_err());
            refuses(&forged, input(&forged, first(), 30, action, vec![]));
        }
    }

    #[test]
    fn missing_character_cannot_bypass_phase_admission_with_creation_offer() {
        let room = initial().unwrap();
        let joined = stage_join(
            &room,
            &input(&room, bootstrap_member().unwrap(), 1, "join-room", vec![]),
        )
        .unwrap();
        let mut state = joined.state().clone();
        state.narrative.active_beats = vec![model::content("complete").unwrap()];
        let forged = model::checkpoint(joined.basis(), state).unwrap();
        assert!(offered(&forged, first()).is_err());
        refuses(
            &forged,
            input(&forged, first(), 2, "create-character", build("Brynn")),
        );
    }

    #[test]
    fn actual_phase_requires_accepted_source_policy_receipt_scope_subjects_and_prior_beat() {
        for entry in ["ask-courier", "escort-courier"] {
            let current = dialogue(entry);
            for case in 0..7 {
                let mut state = current.state().clone();
                let id = state.facts.last().unwrap().id;
                match case {
                    0 => state.narrative.accepted_facts.retain(|fact| *fact != id),
                    1 => {
                        state.decisions.last_mut().unwrap().source_policy =
                            model::label("foreign-policy").unwrap()
                    }
                    2 => {
                        let decision = state.decisions.last_mut().unwrap();
                        let mut receipt = accepted(decision).unwrap();
                        receipt.phase = rpc::JourneyPhase::Complete as i32;
                        decision.semantic_output =
                            Some(super::super::hex(&receipt.encode_to_vec()));
                    }
                    3 => {
                        state.facts.last_mut().unwrap().audience =
                            AudienceScope::Members(vec![bootstrap_member().unwrap()])
                    }
                    4 => {
                        let FactValue::ContentEvent { subjects, .. } =
                            &mut state.facts.last_mut().unwrap().value
                        else {
                            panic!("event")
                        };
                        subjects.push(super::super::room_entity().unwrap());
                    }
                    5 => state.narrative.completed_beats.clear(),
                    _ => {
                        let FactValue::ContentEvent { definition, .. } =
                            &mut state.facts.last_mut().unwrap().value
                        else {
                            panic!("event")
                        };
                        *definition = model::content("ask-courier").unwrap();
                    }
                }
                let forged = model::checkpoint(current.basis(), state).unwrap();
                refuses(
                    &forged,
                    input(&forged, first(), 30, "defend-courier", vec![]),
                );
            }
        }
    }

    #[test]
    fn actual_phase_rejects_missing_or_unrelated_cause_on_accepted_native_sources() {
        let opening = opening_story();
        let private = dialogue("ask-courier");
        let escort = dialogue("escort-courier");
        let combat = combat(&private);
        let victory = inn_victory();
        for (current, entry, action) in [
            (&opening, "begin-story", "ask-courier"),
            (&private, "private-courier-note", "defend-courier"),
            (&escort, "escort-courier", "defend-courier"),
            (&combat, "defend-courier", "end-turn"),
            (&victory, "greatsword-attack", "short-rest"),
        ] {
            for remove in [false, true] {
                let mut state = current.state().clone();
                let unrelated = state.facts.first().unwrap().id;
                let source = state
                    .facts
                    .iter_mut()
                    .find(|fact| {
                        matches!(&fact.value, FactValue::ContentEvent { definition, .. }
                        if definition.entry.as_str() == entry)
                    })
                    .unwrap();
                assert_ne!(source.cause, Some(unrelated));
                source.cause = if remove { None } else { Some(unrelated) };
                let forged = model::checkpoint(current.basis(), state).unwrap();
                assert_eq!(
                    phase(&forged),
                    Err(RepositoryError::InvalidCandidate),
                    "accepted {entry} source must retain its actual cause"
                );
                refuses(&forged, input(&forged, first(), 30, action, vec![]));
            }
        }
    }

    #[test]
    fn actual_causal_sources_survive_prepared_completion_and_recovered_combat() {
        use super::super::super::courier_ai;

        let pending = dialogue("ask-courier");
        let effect = pending.state().intents.first().unwrap();
        let completion = courier_ai::execute(&pending, effect).unwrap();
        let prepared = courier_ai::stage_completion(
            &pending,
            &completion,
            courier_ai::completion_operation(effect).unwrap(),
        )
        .unwrap();
        assert_eq!(prepared.state().facts, pending.state().facts);
        assert_eq!(phase(&prepared).unwrap(), rpc::JourneyPhase::Dialogue);
        let combat = combat(&prepared);
        let start = combat.state().facts.last().unwrap();
        let decision = combat.state().decisions.last().unwrap();
        assert_eq!(
            start.cause,
            decision.facts.get(start.ordinal as usize - 1).copied()
        );
        let restored = model::checkpoint(combat.basis(), combat.state().clone()).unwrap();
        assert_eq!(phase(&restored).unwrap(), rpc::JourneyPhase::Combat);
        assert_eq!(restored.state().knowledge, pending.state().knowledge);
        assert_eq!(
            restored
                .state()
                .facts
                .iter()
                .find(|fact| fact.id == pending.state().facts.last().unwrap().id)
                .unwrap()
                .audience,
            AudienceScope::Members(vec![first()])
        );
    }

    #[test]
    fn current_private_disclosure_changes_preserve_historic_public_phase() {
        use super::super::super::{courier_ai, wire};
        use df_persistence::local_demo_scope::{DISPLAY, LocalDemoRole};

        let pending = dialogue("ask-courier");
        let effect = pending.state().intents.first().unwrap();
        let completion = courier_ai::execute(&pending, effect).unwrap();
        let current = courier_ai::stage_completion(
            &pending,
            &completion,
            courier_ai::completion_operation(effect).unwrap(),
        )
        .unwrap();
        let display = MemberId::from_bytes(&DISPLAY).unwrap();
        let shared = wire::journey_view(&current, LocalDemoRole::Display, display).unwrap();
        for case in 0..5 {
            let mut state = current.state().clone();
            let note = state.facts.last_mut().unwrap();
            match case {
                0 => note.audience = AudienceScope::Host,
                1 => note.audience = AudienceScope::Members(vec![bootstrap_member().unwrap()]),
                2 => note.audience = AudienceScope::Shared,
                3 => {
                    note.audience = AudienceScope::Members(vec![
                        first(),
                        MemberId::from_bytes(&MEMBERS[1]).unwrap(),
                    ])
                }
                _ => {
                    note.value = FactValue::ContentEvent {
                        definition: model::content("harbor").unwrap(),
                        subjects: vec![],
                    }
                }
            }
            let changed = model::checkpoint(current.basis(), state).unwrap();
            assert_eq!(phase(&changed).unwrap(), rpc::JourneyPhase::Dialogue);
            assert_eq!(
                wire::journey_view(&changed, LocalDemoRole::Display, display).unwrap(),
                shared
            );
        }
    }

    #[test]
    fn orthogonal_rest_marker_requires_accepted_native_rest_and_time_source() {
        let victory = gaining_victory();
        let mut forged = victory.state().clone();
        forged
            .narrative
            .completed_beats
            .push(model::content("short-rest-complete").unwrap());
        let forged = model::checkpoint(victory.basis(), forged).unwrap();
        refuses(&forged, input(&forged, first(), 30, "short-rest", vec![]));
        let rested = stage_with_supplier(
            &victory,
            &input(&victory, first(), 9, "short-rest", vec![]),
            &mut |_| panic!("rest does not roll"),
        )
        .unwrap();
        assert_eq!(phase(&rested).unwrap(), rpc::JourneyPhase::Complete);
        for case in 0..4 {
            let mut state = rested.state().clone();
            let decision = state.decisions.last_mut().unwrap();
            match case {
                0 => decision.source_policy = model::label("foreign-rest-policy").unwrap(),
                1 => {
                    let mut receipt = accepted(decision).unwrap();
                    receipt.phase = rpc::JourneyPhase::Combat as i32;
                    decision.semantic_output = Some(super::super::hex(&receipt.encode_to_vec()));
                }
                2 => state.facts.last_mut().unwrap().cause = None,
                _ => {
                    let time = state
                        .facts
                        .iter_mut()
                        .find(|fact| {
                            fact.operation == decision.operation
                                && matches!(fact.value, FactValue::TimeAdvanced { .. })
                        })
                        .unwrap();
                    let FactValue::TimeAdvanced { before, after } = &mut time.value else {
                        panic!("time")
                    };
                    *before = *after;
                }
            }
            let changed = model::checkpoint(rested.basis(), state).unwrap();
            refuses(&changed, input(&changed, first(), 30, "short-rest", vec![]));
        }
    }

    #[test]
    fn completion_requires_accepted_knockout_evidence_even_with_valid_completed_beats() {
        let current = inn_victory();
        for case in 0..3 {
            let mut state = current.state().clone();
            let decision = state.decisions.last_mut().unwrap();
            match case {
                0 => decision.source_policy = model::label("foreign-policy").unwrap(),
                1 => {
                    let mut receipt = accepted(decision).unwrap();
                    receipt.combat.clear();
                    decision.semantic_output = Some(super::super::hex(&receipt.encode_to_vec()));
                }
                _ => {
                    let fact = state.facts.iter_mut().find(|fact| fact.operation == decision.operation
                        && matches!(&fact.value, FactValue::ResourceChanged { entity, resource, .. }
                            if *entity == super::entity(BANDIT).unwrap() && resource.as_str() == "unconscious")).unwrap();
                    let FactValue::ResourceChanged { source, .. } = &mut fact.value else {
                        panic!("knockout")
                    };
                    *source = model::rule().unwrap();
                }
            }
            let forged = model::checkpoint(current.basis(), state).unwrap();
            refuses(&forged, input(&forged, first(), 30, "short-rest", vec![]));
        }
    }

    #[test]
    fn actual_native_stages_and_restored_inn_preserve_both_dialogue_paths_and_private_audience() {
        assert_eq!(phase(&initial().unwrap()).unwrap(), rpc::JourneyPhase::Room);
        assert_eq!(phase(&prepared_story()).unwrap(), rpc::JourneyPhase::Room);
        assert_eq!(phase(&opening_story()).unwrap(), rpc::JourneyPhase::Opening);
        for entry in ["ask-courier", "escort-courier"] {
            let dialogue = dialogue(entry);
            let expected = if entry == "ask-courier" {
                AudienceScope::Members(vec![first()])
            } else {
                AudienceScope::Shared
            };
            assert_eq!(dialogue.state().facts.last().unwrap().audience, expected);
            let knowledge = dialogue.state().knowledge.clone();
            assert_eq!(phase(&dialogue).unwrap(), rpc::JourneyPhase::Dialogue);
            let combat = combat(&dialogue);
            assert_eq!(phase(&combat).unwrap(), rpc::JourneyPhase::Combat);
            assert_eq!(combat.state().knowledge, knowledge);
            assert_eq!(
                combat
                    .state()
                    .facts
                    .iter()
                    .find(|fact| fact.id == dialogue.state().facts.last().unwrap().id)
                    .unwrap()
                    .audience,
                expected
            );
        }
        let victory = gaining_victory();
        assert_eq!(phase(&victory).unwrap(), rpc::JourneyPhase::Complete);
        let rest = stage_with_supplier(
            &victory,
            &input(&victory, first(), 9, "short-rest", vec![]),
            &mut |_| panic!("rest cannot roll"),
        )
        .unwrap();
        assert_eq!(
            super::super::value(rest.state(), entity(BANDIT).unwrap(), "unconscious").unwrap(),
            0
        );
        for (before, already_rested) in [(&victory, false), (&rest, true)] {
            let arrived =
                stage_with_supplier(before, &inn_command(before, first(), 10), &mut |_| {
                    panic!("inn cannot roll")
                })
                .unwrap();
            let restored = model::checkpoint(arrived.basis(), arrived.state().clone()).unwrap();
            assert_eq!(phase(&restored).unwrap(), rpc::JourneyPhase::Complete);
            assert_eq!(restored.state().knowledge, before.state().knowledge);
            assert_eq!(restored.state().logical_time, before.state().logical_time);
            assert_eq!(restored.state().resources, before.state().resources);
            let rested = stage_with_supplier(
                &restored,
                &input(&restored, first(), 11, "short-rest", vec![]),
                &mut |_| panic!("restored rest cannot roll"),
            );
            if already_rested {
                // One rest per aftermath, at the inn as at the fight.
                assert!(rested.is_err());
            } else {
                assert_eq!(
                    phase(&rested.unwrap()).unwrap(),
                    rpc::JourneyPhase::Complete
                );
            }
        }
    }

    #[test]
    fn actual_stage_and_join_reject_stale_identity_revision_and_pins() {
        let current = opening_story();
        for case in 0..4 {
            let mut command = input(&current, first(), 30, "ask-courier", vec![]);
            let GameInput::Game(ref mut proposed) = command else {
                panic!("command")
            };
            match case {
                0 => proposed.basis.run = df_types::RunId::from_bytes(&[99; 16]).unwrap(),
                1 => proposed.basis.session = df_types::SessionId::from_bytes(&[99; 16]).unwrap(),
                2 => proposed.basis.revision = proposed.basis.revision.next_sequence().unwrap(),
                _ => {
                    proposed.observed_revision = proposed.observed_revision.next_sequence().unwrap()
                }
            }
            refuses(&current, command);
        }
        let mut pins = current.pins().clone();
        pins.content.package_digest = ContentDigest([99; 32]);
        let stale = Checkpoint::new(
            current.schema(),
            current.basis(),
            pins,
            current.state().clone(),
            ReferenceInventory {
                rules: &[model::rule().unwrap(), rule().unwrap()],
                content: &model::contents().unwrap(),
                resources: &super::super::resources_for(current.state()).unwrap(),
                assets: &[],
            },
            model::limits(),
        )
        .unwrap();
        assert_eq!(phase(&stale), Err(RepositoryError::InvalidCandidate));
        refuses(&stale, input(&stale, first(), 30, "ask-courier", vec![]));
        let room = initial().unwrap();
        let mut stale_join = input(&room, bootstrap_member().unwrap(), 1, "join-room", vec![]);
        let GameInput::Game(ref mut proposed) = stale_join else {
            panic!("join")
        };
        proposed.basis.revision = proposed.basis.revision.next_sequence().unwrap();
        let before = room.clone();
        assert_eq!(
            stage_join(&room, &stale_join),
            Err(RepositoryError::InvalidCandidate)
        );
        assert_eq!(room, before);
    }

    #[test]
    fn defeat_requires_both_accepted_party_knockouts_and_survives_rest_recovery() {
        let dialogue = dialogue("escort-courier");
        let mut initiative = 0;
        let combat = stage_with_supplier(
            &dialogue,
            &input(&dialogue, first(), 7, "defend-courier", vec![]),
            &mut |sides| {
                if sides == 20 {
                    initiative += 1;
                    Ok(if initiative <= 2 { 1 } else { 20 })
                } else {
                    Ok(6)
                }
            },
        )
        .unwrap();
        let mut counterfeit = combat.state().clone();
        counterfeit
            .narrative
            .completed_beats
            .append(&mut counterfeit.narrative.active_beats);
        counterfeit.narrative.active_beats = vec![model::content("complete").unwrap()];
        counterfeit.encounters[0].active_turn = None;
        counterfeit.encounters[0]
            .objectives
            .push(model::content("combat-defeat").unwrap());
        let counterfeit = model::checkpoint(combat.basis(), counterfeit).unwrap();
        refuses(
            &counterfeit,
            input(&counterfeit, first(), 30, "short-rest", vec![]),
        );
        let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        let defeated = stage_with_supplier(
            &combat,
            &input(&combat, second, 8, "end-turn", vec![]),
            &mut |sides| Ok(if sides == 20 { 20 } else { 6 }),
        )
        .unwrap();
        assert_eq!(phase(&defeated).unwrap(), rpc::JourneyPhase::Complete);
        assert!(!combat_victory(defeated.state()).unwrap());
        let rest = stage_with_supplier(
            &defeated,
            &input(&defeated, second, 9, "short-rest", vec![]),
            &mut |_| panic!("rest does not roll"),
        )
        .unwrap();
        let restored = model::checkpoint(rest.basis(), rest.state().clone()).unwrap();
        assert_eq!(phase(&restored).unwrap(), rpc::JourneyPhase::Complete);
        assert!(!combat_victory(restored.state()).unwrap());
        assert_eq!(
            restored.state().narrative.open_threads,
            defeated.state().narrative.open_threads
        );
        assert!(
            !offered(&restored, second)
                .unwrap()
                .iter()
                .any(|(kind, _)| *kind == rpc::GameplayActionKind::ChooseHarborScene)
        );
    }
}

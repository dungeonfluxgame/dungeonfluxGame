//! Source owner for the existing native story and courier alternatives.
use df_model::checkpoint::*;
use df_narrative::{
    AdmittedBeatAlternative, BeatCause, BeatSelectionLimits, CheckpointBeatRequest,
    CheckpointConvergenceRequest, ConvergenceLimits, NarrativeBudgetLimits,
    stage_checkpoint_convergence,
};
use df_protocol::common as rpc;
use df_session::submission::RepositoryError;
use df_types::MemberId;

use super::{
    PACKET_THREAD, THREAD_POLICY, accepted, command, model, participants, player_entity, rule,
};

#[cfg(test)]
#[path = "narrative_transition_tests.rs"]
mod tests;

fn refusal(cause: &'static str) -> RepositoryError {
    crate::gameplay::refusal_detail::cause("narrative-transition", cause.to_owned());
    RepositoryError::InvalidCandidate
}
fn invalid<T>(cause: &'static str) -> Result<T, RepositoryError> {
    Err(refusal(cause))
}

fn native_sources<'a>(
    current: &'a Checkpoint,
    event: &ContentReference,
    phase: rpc::JourneyPhase,
) -> Result<Vec<(&'a GameFact, rpc::AcceptedAction)>, RepositoryError> {
    let state = current.state();
    let mut sources = Vec::new();
    for (index, source) in state.facts.iter().enumerate() {
        if !matches!(&source.value, FactValue::ContentEvent { definition, .. } if definition == event)
        {
            continue;
        }
        let decision = state
            .decisions
            .iter()
            .find(|decision| {
                decision.operation == source.operation
                    && decision.revision == source.revision
                    && decision.facts.last() == Some(&source.id)
                    && decision.facts.get(source.ordinal as usize) == Some(&source.id)
            })
            .ok_or_else(|| refusal("SourceDecisionMissing"))?;
        let receipt = accepted(decision)?;
        let prior = index
            .checked_sub(1)
            .and_then(|index| state.facts.get(index));
        if source.audience != AudienceScope::Shared
            || !matches!(&source.value, FactValue::ContentEvent { subjects, .. } if subjects.is_empty())
            || receipt.phase != phase as i32
            || source.cause != prior.map(|fact| fact.id)
            || (source.ordinal > 0
                && !prior.is_some_and(|fact| {
                    fact.operation == source.operation
                        && fact.revision == source.revision
                        && fact.ordinal.checked_add(1) == Some(source.ordinal)
                        && decision.facts.get(source.ordinal as usize - 1) == Some(&fact.id)
                }))
        {
            return invalid("SourceReceiptOrCausalHistoryMismatch");
        }
        sources.push((source, receipt));
    }
    Ok(sources)
}

/// Selects only the three current source-backed native alternatives. No command
/// can manufacture an authored alternative or replace a canonical prerequisite.
pub(super) fn select(
    current: &Checkpoint,
    input: &GameInput,
) -> Result<Checkpoint, RepositoryError> {
    let command = command(input)?;
    if command.basis != current.basis() || command.observed_revision != current.basis().revision {
        return invalid("ObservedBasisMismatch");
    }
    let GameCommand::ProposeAction {
        actor,
        action,
        targets,
        choices,
    } = &command.command
    else {
        return invalid("ExpectedProposeAction");
    };
    if *actor != player_entity(command.member, current)?
        || !targets.is_empty()
        || !choices.is_empty()
    {
        return invalid("ActorTargetsOrChoicesMismatch");
    }
    select_for(current, command.member, action)
}

// Room snapshots may retain ready character state while losing a creation cause.
// Do not advertise a story action that the same-state selector must refuse.
pub(super) fn story_permitted(
    current: &Checkpoint,
    member: MemberId,
) -> Result<bool, RepositoryError> {
    super::phase(current)?;
    // Offer probing is not a refused command: preserve only the caller's diagnostic.
    let diagnostic = df_session::submission::take_refusal_diagnostic();
    let step = crate::gameplay::refusal_detail::take();
    let result = match select_for(current, member, &model::content("begin-story")?) {
        Ok(_) => Ok(true),
        Err(RepositoryError::InvalidCandidate | RepositoryError::InvalidReceipt) => Ok(false),
        Err(error) => Err(error),
    };
    if matches!(result, Ok(false)) {
        let _ = df_session::submission::take_refusal_diagnostic();
        let _ = crate::gameplay::refusal_detail::take();
    }
    if let Some(detail) = diagnostic {
        df_session::submission::note_refusal_diagnostic(detail.step, detail.cause);
    }
    if let Some(step) = step {
        crate::gameplay::refusal_detail::note(step);
    }
    result
}

/// The active source profile authorizes earned routes only. Its configured balance is
/// capped explicitly; no strong-event tariff or free-play trigger was supplied.
/// Source-selected events/rates must be published before either list can be populated.
fn native_budget()
-> Result<df_content::narrative::NarrativeBudgetPolicy<ContentReference>, RepositoryError> {
    Ok(df_content::narrative::NarrativeBudgetPolicy {
        definition: model::content("narrative-intervention-policy")?,
        // The registered room journey uses the same initial and maximum tuned balance.
        // The older harbor profile initializes one; no tariff is authored here.
        maximum: super::pack::narrative_budget(),
        strong_events: vec![],
        free_play_events: vec![],
    })
}

fn select_for(
    current: &Checkpoint,
    member: MemberId,
    action: &ContentReference,
) -> Result<Checkpoint, RepositoryError> {
    let phase = super::phase(current)?;
    if !participants(current)
        .iter()
        .any(|link| link.member == member)
    {
        return invalid("MemberNotParticipant");
    }
    let room = model::content("room")?;
    let opening = model::content("opening")?;
    let seal = model::content("courier-answer-seal")?;
    let escort = model::content("courier-answer-escort")?;
    let begin = model::content("begin-story")?;
    let ask = model::content("ask-courier")?;
    let escort_action = model::content("escort-courier")?;
    let create = model::content("create-character")?;
    let packet = model::content(PACKET_THREAD)?;
    let mut causes = Vec::new();
    let required_threads;
    let alternatives = if phase == rpc::JourneyPhase::Room && *action == begin {
        let party = participants(current);
        if !super::party_ready(current) {
            return invalid("PartyCreationCountMismatch");
        }
        let sources = native_sources(current, &create, rpc::JourneyPhase::Room)?;
        if sources.len() != party.len() {
            return invalid("CreationSourceCountMismatch");
        }
        let mut remaining = sources
            .iter()
            .map(|(_, receipt)| receipt.character.as_ref())
            .collect::<Vec<_>>();
        for link in party {
            let who = link
                .character
                .ok_or_else(|| refusal("MemberCharacterMissing"))?;
            let sheet = super::receipt_character_sheet(current.state(), who)?;
            let index = remaining
                .iter()
                .position(|candidate| *candidate == Some(&sheet))
                .ok_or_else(|| refusal("CreationSheetReceiptMismatch"))?;
            remaining.remove(index);
        }
        for (source, _) in sources {
            causes.push(BeatCause {
                fact: source.id,
                event: &create,
                consumed_by_narrative: false,
            });
        }
        required_threads = Vec::new();
        vec![AdmittedBeatAlternative {
            selection: &begin,
            from: &room,
            to: &opening,
            causes: &causes,
            required_threads: &required_threads,
            opened_thread: if crate::gameplay::beat_script::with_script(|script| {
                script.mode == crate::gameplay::beat_script::ScriptMode::Freeplay
            }) {
                None
            } else {
                Some(&packet)
            },
        }]
    } else if phase == rpc::JourneyPhase::Opening && (*action == ask || *action == escort_action) {
        let sources = native_sources(current, &begin, rpc::JourneyPhase::Opening)?;
        let [(source, _)] = sources.as_slice() else {
            return invalid("OpeningSourceCountMismatch");
        };
        causes.push(BeatCause {
            fact: source.id,
            event: &begin,
            consumed_by_narrative: true,
        });
        required_threads = vec![packet];
        vec![
            AdmittedBeatAlternative {
                selection: &ask,
                from: &opening,
                to: &seal,
                causes: &causes,
                required_threads: &required_threads,
                opened_thread: None,
            },
            AdmittedBeatAlternative {
                selection: &escort_action,
                from: &opening,
                to: &escort,
                causes: &causes,
                required_threads: &required_threads,
                opened_thread: None,
            },
        ]
    } else {
        return invalid("ActionUnavailableInPhase");
    };
    let policy = model::label(THREAD_POLICY)?;
    if crate::gameplay::beat_script::with_script(|script| {
        script.mode == crate::gameplay::beat_script::ScriptMode::Freeplay
    }) {
        // Entry into a social place needs causal beat selection, not a story intervention
        // budget. Reuse the same authored-source validation without inventing story policy.
        return df_narrative::stage_checkpoint_beat_selection(
            current,
            CheckpointBeatRequest {
                expected_basis: current.basis(),
                admitted_pins: current.pins(),
                policy: &policy,
                expected_policy: &policy,
                recipient: member,
                selection: action,
                alternatives: &alternatives,
                inventory: ReferenceInventory {
                    rules: &[model::rule()?, rule()?],
                    content: &model::contents()?,
                    resources: &super::resources_for(current.state())?,
                    assets: &crate::gameplay::model::creator_portrait_assets(current.state())?,
                },
                checkpoint_limits: model::limits(),
            },
            BeatSelectionLimits {
                records: 512,
                alternatives: 2,
                work: 1024 * 1024,
            },
        )
        .map_err(|error| {
            crate::gameplay::refusal_detail::cause("narrative-transition", format!("{error:?}"));
            RepositoryError::InvalidCandidate
        });
    }
    let budget = native_budget()?;
    let proposal = stage_checkpoint_convergence(
        current,
        CheckpointConvergenceRequest {
            beat: CheckpointBeatRequest {
                expected_basis: current.basis(),
                admitted_pins: current.pins(),
                policy: &policy,
                expected_policy: &policy,
                recipient: member,
                selection: action,
                alternatives: &alternatives,
                inventory: ReferenceInventory {
                    rules: &[model::rule()?, rule()?],
                    content: &model::contents()?,
                    resources: &super::resources_for(current.state())?,
                    assets: &crate::gameplay::model::creator_portrait_assets(current.state())?,
                },
                checkpoint_limits: model::limits(),
            },
            budget: &budget,
            expected_budget: &budget,
            strong_receipt: None,
            delivery: None,
        },
        ConvergenceLimits {
            beats: BeatSelectionLimits {
                records: 512,
                alternatives: 2,
                work: 1024 * 1024,
            },
            budget: NarrativeBudgetLimits {
                records: 512,
                work: 1024 * 1024,
            },
            maximum_evidence_bytes: 8192,
        },
    )
    .map_err(|error| {
        crate::gameplay::refusal_detail::cause("narrative-transition", format!("{error:?}"));
        RepositoryError::InvalidCandidate
    })?;
    Ok(proposal.checkpoint)
}

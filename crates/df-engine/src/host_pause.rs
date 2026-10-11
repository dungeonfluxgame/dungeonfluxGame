//! Canonical host pause staging. Permission, durable acknowledgement and timer ownership
//! remain with the native session; a source mapping is not a host grant.
use df_model::checkpoint::{
    AcceptedDecision, AudienceScope, Basis, Checkpoint, CheckpointError, CheckpointLimits,
    CheckpointPins, ContentReference, FactId, FactValue, GameFact, GameInput, HostCommand,
    ReferenceInventory,
};
use df_model::commands::{CommandError, CommandLimits, validate_client_command};
use df_types::RevisionLabel;

/// Explicit admitted content mapping for one session lineage. No label is parsed and
/// no initial value is inferred. These borrowed selections confer no source or access rights.
#[derive(Clone, Copy)]
pub struct HostPausePolicy<'a> {
    pub command: &'a ContentReference,
    pub paused_event: &'a ContentReference,
    pub resumed_event: &'a ContentReference,
    pub source_policy: &'a RevisionLabel,
    pub initially_paused: bool,
}

/// The current engine owner supplies exact admitted pins, inventory and finite bounds.
pub struct HostPauseContext<'a> {
    pub current_basis: Basis,
    pub admitted_pins: &'a CheckpointPins,
    pub inventory: ReferenceInventory<'a>,
    pub checkpoint_limits: CheckpointLimits,
    pub command_limits: CommandLimits,
    pub policy: HostPausePolicy<'a>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostPauseError {
    Checkpoint(CheckpointError),
    Command(CommandError),
    InvalidPolicy,
    MalformedPauseRecord,
    UnsupportedCommand,
    StaleBasis,
    DuplicateOperation,
    Capacity,
    InvalidTime,
    PausedTimeAdvance,
    CandidateLineage,
}

fn validate_current(
    current: &Checkpoint,
    context: &HostPauseContext<'_>,
) -> Result<(), HostPauseError> {
    current
        .validate_admitted(
            context.current_basis,
            context.admitted_pins,
            context.inventory,
            context.checkpoint_limits,
        )
        .map_err(HostPauseError::Checkpoint)?;
    let policy = context.policy;
    if policy.command == policy.paused_event
        || policy.command == policy.resumed_event
        || policy.paused_event == policy.resumed_event
    {
        return Err(HostPauseError::InvalidPolicy);
    }
    for reference in [policy.command, policy.paused_event, policy.resumed_event] {
        if reference.package != context.admitted_pins.content.package
            || !context.inventory.content.contains(reference)
        {
            return Err(HostPauseError::InvalidPolicy);
        }
    }
    Ok(())
}

/// Read the current pause value from exactly bound accepted canonical facts.
/// Matching seed/unbound facts, malformed decisions or source-policy mismatch fail closed.
/// Unrelated facts cannot pause a session. The owner must reauthorize the source selection.
pub fn committed_pause(
    current: &Checkpoint,
    context: &HostPauseContext<'_>,
) -> Result<bool, HostPauseError> {
    validate_current(current, context)?;
    let policy = context.policy;
    let mut paused = policy.initially_paused;
    for fact in &current.state().facts {
        let FactValue::ContentEvent {
            definition,
            subjects,
        } = &fact.value
        else {
            continue;
        };
        let desired = if definition == policy.paused_event {
            true
        } else if definition == policy.resumed_event {
            false
        } else {
            continue;
        };
        let (_, decision) = current
            .committed_outcome(fact.id)
            .ok_or(HostPauseError::MalformedPauseRecord)?;
        if fact.audience != AudienceScope::Shared
            || !subjects.is_empty()
            || fact.ordinal != 0
            || fact.cause.is_some()
            || decision.source_policy != *policy.source_policy
            || decision.facts.as_slice() != [fact.id]
            || !decision.draws.is_empty()
            || !decision.effects.is_empty()
            || decision.semantic_output.is_some()
        {
            return Err(HostPauseError::MalformedPauseRecord);
        }
        paused = desired;
    }
    Ok(paused)
}

/// Stage one explicit desired pause value, never a toggle. Call only after current
/// host authentication and exact operation lookup; the repository must recheck permission
/// at commit. The supplied fact identity is allocated by that owner, not inferred here.
/// No clock read, timer cancellation, publication, native work or durable commit occurs.
///
/// Pending resolution basis is rebased as required by the canonical checkpoint invariant;
/// its windows, choices, continuation, draws, spends and rulings remain unchanged.
/// Timers, schedules and accepted intents retain their original identities and generations.
pub fn stage_host_pause(
    current: &Checkpoint,
    input: &GameInput,
    fact_id: FactId,
    context: &HostPauseContext<'_>,
) -> Result<Checkpoint, HostPauseError> {
    committed_pause(current, context)?;
    let GameInput::Host(host) = input else {
        return Err(HostPauseError::UnsupportedCommand);
    };
    if host.basis != context.current_basis {
        return Err(HostPauseError::StaleBasis);
    }
    let HostCommand::SetPause { paused, policy } = &host.command else {
        return Err(HostPauseError::UnsupportedCommand);
    };
    if policy != context.policy.command {
        return Err(HostPauseError::InvalidPolicy);
    }
    validate_client_command(input, current, context.inventory, context.command_limits)
        .map_err(HostPauseError::Command)?;
    if current
        .state()
        .decisions
        .iter()
        .any(|decision| decision.operation == host.operation)
    {
        return Err(HostPauseError::DuplicateOperation);
    }
    let mut next = current.basis();
    next.revision = next
        .revision
        .next_sequence()
        .map_err(|_| HostPauseError::Capacity)?;
    let mut state = current.state().clone();
    state
        .facts
        .try_reserve(1)
        .map_err(|_| HostPauseError::Capacity)?;
    state
        .decisions
        .try_reserve(1)
        .map_err(|_| HostPauseError::Capacity)?;
    for pending in &mut state.pending {
        pending.basis = next;
    }
    state.facts.push(GameFact {
        id: fact_id,
        revision: next.revision,
        operation: host.operation,
        ordinal: 0,
        cause: None,
        audience: AudienceScope::Shared,
        value: FactValue::ContentEvent {
            definition: if *paused {
                context.policy.paused_event.clone()
            } else {
                context.policy.resumed_event.clone()
            },
            subjects: vec![],
        },
    });
    state.decisions.push(AcceptedDecision {
        operation: host.operation,
        revision: next.revision,
        facts: vec![fact_id],
        draws: vec![],
        effects: vec![],
        source_policy: context.policy.source_policy.clone(),
        semantic_output: None,
    });
    Checkpoint::new(
        current.schema(),
        next,
        current.pins().clone(),
        state,
        context.inventory,
        context.checkpoint_limits,
    )
    .map_err(HostPauseError::Checkpoint)
}

/// Guard a staged native engine candidate before the session repository commit.
/// A paused canonical checkpoint cannot advance logical time, including when the same
/// candidate resumes. The subsequent accepted game action may advance source-valid time.
/// This gate does not authorize that action or convert elapsed process time into game ticks.
pub fn guard_pause_time_transition(
    current: &Checkpoint,
    candidate: &Checkpoint,
    context: &HostPauseContext<'_>,
) -> Result<(), HostPauseError> {
    let paused = committed_pause(current, context)?;
    let mut next = current.basis();
    next.revision = next
        .revision
        .next_sequence()
        .map_err(|_| HostPauseError::CandidateLineage)?;
    candidate
        .validate_admitted(
            next,
            context.admitted_pins,
            context.inventory,
            context.checkpoint_limits,
        )
        .map_err(HostPauseError::Checkpoint)?;
    let before = current.state().logical_time;
    let after = candidate.state().logical_time;
    if after.ticks_per_second != before.ticks_per_second || after.ticks < before.ticks {
        return Err(HostPauseError::InvalidTime);
    }
    if paused && after != before {
        return Err(HostPauseError::PausedTimeAdvance);
    }
    Ok(())
}

//! Pure Tempo staging over an already selected canonical Engine candidate.
use df_model::checkpoint::{
    Checkpoint, CheckpointError, CheckpointLimits, CheckpointPins, ReferenceInventory,
};
use df_tempo::elapsed::{TempoAdvanceRequest, TempoElapsedPolicy};
use df_tempo::impulse::{
    ImpulseAdvance, ImpulseError, ImpulseLimits, ImpulsePolicy, PermittedTempoInput,
    advance_impulse,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TempoStageError {
    Impulse(ImpulseError),
    InvalidCandidate(CheckpointError),
}

/// Stages existing bounded intensity/inertia policy, then validates the detached checkpoint.
///
/// `policies` supplies elapsed and impulse policy respectively; `limits` supplies impulse and
/// canonical checkpoint limits. These are current source-admitted inputs, not authentication.
/// The target and pause decision remain the native owner's responsibility. The returned existing
/// advance retains its basis, elapsed disposition and timestamped curve for owner revalidation.
///
/// Only TempoState changes. Logical time, rules windows/options, resources, facts, decisions,
/// effects and execution mode remain exactly as selected by preceding Engine stages. Refusal
/// returns neither a partial checkpoint nor an advance; the borrowed candidate stays unchanged.
/// No clock, draw, provider, timer, publication or commit is executed. Durable operation retry,
/// owner fencing and audience-safe presentation remain the Session/presentation owners' duties.
pub fn stage_tempo_candidate(
    current: &Checkpoint,
    admitted_pins: &CheckpointPins,
    request: TempoAdvanceRequest,
    policies: (Option<&TempoElapsedPolicy>, Option<&ImpulsePolicy>),
    input: PermittedTempoInput,
    limits: (ImpulseLimits, CheckpointLimits),
    inventory: ReferenceInventory<'_>,
) -> Result<(Checkpoint, ImpulseAdvance), TempoStageError> {
    let advance = advance_impulse(
        current,
        admitted_pins,
        request,
        policies.0,
        policies.1,
        input,
        limits.0,
    )
    .map_err(TempoStageError::Impulse)?;
    let mut state = current.state().clone();
    state.tempo = advance.state.clone();
    let candidate = Checkpoint::new(
        current.schema(),
        current.basis(),
        admitted_pins.clone(),
        state,
        inventory,
        limits.1,
    )
    .map_err(TempoStageError::InvalidCandidate)?;
    Ok((candidate, advance))
}

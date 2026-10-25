//! Pure presentation-anchor advancement. No game time, rules expiry or effect is advanced.
use std::time::Duration;

use df_model::checkpoint::{
    Basis, Checkpoint, CheckpointError, CheckpointPins, ContentReference, ExecutionMode,
    LogicalTime, TempoState,
};

/// Already decoded and source-admitted immutable policy supplied by the content owner.
/// This shape does not decode catalog bytes, establish rights or calibrate any intensity policy.
/// The exact timebase must remain bound to this definition and the current content pins.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TempoElapsedPolicy {
    pub definition: ContentReference,
    pub ticks_per_second: u32,
    pub maximum_elapsed_ticks: u64,
}

/// Supplied elapsed presentation time, independent of authoritative logical game time.
/// `paused` is the owning server's approved pause decision, never inferred from inactivity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TempoAdvanceRequest {
    pub expected_basis: Basis,
    pub observed_logical_time: LogicalTime,
    pub from_presentation_ticks: u64,
    pub elapsed: Duration,
    pub paused: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TempoElapsedLimits {
    pub maximum_checkpoint_bytes: usize,
    pub maximum_fatigue_entries: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TempoAdvanceError {
    InvalidLimits,
    Capacity,
    Binding(CheckpointError),
    LogicalTimeMismatch,
    StalePresentationAnchor,
    PolicyUnavailable,
    PolicyMismatch,
    InvalidPolicy,
    FractionalTicks,
    ElapsedLimit,
    TimeOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ElapsedDisposition {
    Paused,
    Replay,
    Unchanged,
    Advanced,
}

/// Detached canonical state for the engine's staged working copy. No publication or commit.
/// Only the presentation anchor changes; impulse/inertia and fatigue policy have separate owners.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ElapsedAdvance {
    pub state: TempoState,
    pub elapsed_ticks: u64,
    pub disposition: ElapsedDisposition,
}

/// Validates the current immutable basis and advances only explicitly supplied presentation time.
/// No clock, random source, provider, database or environment is accessed.
///
/// Conversion is exact: a duration not representable in whole policy ticks is refused, not
/// rounded or silently dropped. All arithmetic is checked; an overlarge catch-up is refused
/// rather than silently clipped. The native owner supplies bounded slices and rechecks basis
/// and anchor at atomic decision commit. Repeating a request after its state has been staged
/// fails the old anchor check. Identical requests against an unchanged immutable checkpoint
/// yield identical proposals; this pure function cannot itself claim durable deduplication.
///
/// Replay preserves recorded state instead of synthesizing new presentation work. Paused input
/// preserves every tempo field. PreparedOnly can stage data, but no outcome dispatches a cue.
pub fn advance_elapsed(
    current: &Checkpoint,
    admitted_pins: &CheckpointPins,
    request: TempoAdvanceRequest,
    policy: Option<&TempoElapsedPolicy>,
    limits: TempoElapsedLimits,
) -> Result<ElapsedAdvance, TempoAdvanceError> {
    if limits.maximum_checkpoint_bytes == 0 {
        return Err(TempoAdvanceError::InvalidLimits);
    }
    if current
        .retained_bytes()
        .ok_or(TempoAdvanceError::Capacity)?
        > limits.maximum_checkpoint_bytes
        || current.state().tempo.fatigue.len() > limits.maximum_fatigue_entries
    {
        return Err(TempoAdvanceError::Capacity);
    }
    current
        .validate_resume(request.expected_basis, admitted_pins)
        .map_err(TempoAdvanceError::Binding)?;
    if request.observed_logical_time != current.state().logical_time {
        return Err(TempoAdvanceError::LogicalTimeMismatch);
    }
    let state = &current.state().tempo;
    if request.from_presentation_ticks != state.presentation_ticks {
        return Err(TempoAdvanceError::StalePresentationAnchor);
    }
    let policy = policy.ok_or(TempoAdvanceError::PolicyUnavailable)?;
    if policy.definition != state.policy
        || policy.definition.package != admitted_pins.content.package
    {
        return Err(TempoAdvanceError::PolicyMismatch);
    }
    if policy.ticks_per_second == 0 || policy.maximum_elapsed_ticks == 0 {
        return Err(TempoAdvanceError::InvalidPolicy);
    }
    if current.state().mode == ExecutionMode::Replay {
        return Ok(ElapsedAdvance {
            state: state.clone(),
            elapsed_ticks: 0,
            disposition: ElapsedDisposition::Replay,
        });
    }
    if request.paused {
        return Ok(ElapsedAdvance {
            state: state.clone(),
            elapsed_ticks: 0,
            disposition: ElapsedDisposition::Paused,
        });
    }
    let elapsed_tick_nanos = request
        .elapsed
        .as_nanos()
        .checked_mul(u128::from(policy.ticks_per_second))
        .ok_or(TempoAdvanceError::TimeOverflow)?;
    if elapsed_tick_nanos % 1_000_000_000 != 0 {
        return Err(TempoAdvanceError::FractionalTicks);
    }
    let elapsed_ticks = u64::try_from(elapsed_tick_nanos / 1_000_000_000)
        .map_err(|_| TempoAdvanceError::TimeOverflow)?;
    if elapsed_ticks > policy.maximum_elapsed_ticks {
        return Err(TempoAdvanceError::ElapsedLimit);
    }
    let next_presentation_ticks = state
        .presentation_ticks
        .checked_add(elapsed_ticks)
        .ok_or(TempoAdvanceError::TimeOverflow)?;
    let mut state = state.clone();
    state.presentation_ticks = next_presentation_ticks;
    Ok(ElapsedAdvance {
        state,
        elapsed_ticks,
        disposition: if elapsed_ticks == 0 {
            ElapsedDisposition::Unchanged
        } else {
            ElapsedDisposition::Advanced
        },
    })
}

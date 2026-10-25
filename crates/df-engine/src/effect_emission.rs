//! Pure bounded declaration of canonical accepted effect records; no execution or readiness.
use df_model::checkpoint::{
    Basis, Checkpoint, CheckpointError, CheckpointPins, DurableIntent, DurableStatus, EffectId,
    EffectKind,
};
use df_types::OperationId;
use std::mem::size_of;

/// Explicit caller bounds. Comparisons include worst-case canonical lookup and timer checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectEmissionLimits {
    pub maximum_scan_records: usize,
    pub maximum_effects: usize,
    pub maximum_comparisons: usize,
    pub maximum_retained_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectEmissionError {
    Checkpoint(CheckpointError),
    MissingDecision,
    StaleDecision,
    MissingEffect,
    UndeclaredEffect,
    StaleEffect,
    NotPending,
    InvalidTimerBinding,
    RevisionOverflow,
    Capacity,
}

/// Engine-owned read-only registration inspection, supplied by the native composition owner.
///
/// Implementations inspect the exact canonical record against their closed registered inventory.
/// They must return the classified native refusal when a kind lacks an executor or terminal
/// route. Inspection starts no work and is not a gameplay readiness or commit capability.
/// Unknown ingress must be rejected before constructing the canonical EffectKind; there is no
/// string/numeric dispatch fallback in this port.
/// ```compile_fail
/// use df_model::checkpoint::EffectKind;
/// let unknown: EffectKind = 99;
/// ```
pub trait EffectRegistrationInspector {
    type Refusal;

    fn inspect(&self, intent: &DurableIntent) -> Result<(), Self::Refusal>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EffectInspectionError<R> {
    Declaration(EffectEmissionError),
    Registration {
        effect: EffectId,
        kind: EffectKind,
        refusal: R,
    },
}

/// Connects a handler's actual next-revision checkpoint to native registration inspection.
///
/// The handler stages the canonical AcceptedDecision and DurableIntent records in its owned
/// checkpoint. This function validates and detaches those records, then inspects them in declared
/// order. Refusal returns no batch, and inspection invokes neither an executor nor a terminal
/// consumer. A successful read-only inspection still grants no readiness/commit/dispatch rights;
/// session must atomically commit the staged checkpoint before any durable execution.
pub fn inspect_staged_effects<I: EffectRegistrationInspector>(
    staged: &Checkpoint,
    current_basis: Basis,
    admitted: &CheckpointPins,
    operation: OperationId,
    limits: EffectEmissionLimits,
    inspector: &I,
) -> Result<Vec<DurableIntent>, EffectInspectionError<I::Refusal>> {
    let expected = Basis {
        revision: current_basis.revision.next_sequence().map_err(|_| {
            EffectInspectionError::Declaration(EffectEmissionError::RevisionOverflow)
        })?,
        ..current_basis
    };
    let effects = declare_accepted_effects(staged, expected, admitted, operation, limits)
        .map_err(EffectInspectionError::Declaration)?;
    for intent in &effects {
        inspector
            .inspect(intent)
            .map_err(|refusal| EffectInspectionError::Registration {
                effect: intent.id,
                kind: intent.kind,
                refusal,
            })?;
    }
    Ok(effects)
}

/// Detaches exactly the named canonical accepted decision's pending intents in declared order.
///
/// The checkpoint must match the trusted current basis and source/build pins exactly. Every
/// effect must belong to that operation at that exact revision; unlisted operation intents are
/// refused rather than dropped. Existing status, IDs, generation, slots and definitions are
/// preserved. This is a declaration, not a dispatch claim: session owns durable admission,
/// deduplication, commit fencing and job/timer lifetime; server owns native registrations.
/// The canonical AcceptedDecision here can be staged or already committed; possession alone
/// proves neither. Production registration must gate gameplay admission, and session commit
/// precedes dispatch. No provider, timer, rules, SDK or terminal call occurs.
/// Recovery of older decisions and dispatch of claimed/unknown records are separate owner paths.
/// On any refusal the checkpoint is unchanged and no partially declared batch is returned.
pub fn declare_accepted_effects(
    checkpoint: &Checkpoint,
    expected: Basis,
    admitted: &CheckpointPins,
    operation: OperationId,
    limits: EffectEmissionLimits,
) -> Result<Vec<DurableIntent>, EffectEmissionError> {
    checkpoint
        .validate_resume(expected, admitted)
        .map_err(EffectEmissionError::Checkpoint)?;
    let state = checkpoint.state();
    let scan_records = state
        .decisions
        .len()
        .checked_add(state.intents.len())
        .and_then(|count| count.checked_add(state.timers.len()))
        .ok_or(EffectEmissionError::Capacity)?;
    if limits.maximum_scan_records == 0
        || limits.maximum_effects == 0
        || limits.maximum_comparisons == 0
        || limits.maximum_retained_bytes == 0
        || scan_records > limits.maximum_scan_records
        || state.decisions.len() > limits.maximum_comparisons
    {
        return Err(EffectEmissionError::Capacity);
    }
    let decision = state
        .decisions
        .iter()
        .find(|decision| decision.operation == operation)
        .ok_or(EffectEmissionError::MissingDecision)?;
    if decision.revision != expected.revision {
        return Err(EffectEmissionError::StaleDecision);
    }
    let comparisons = decision
        .effects
        .len()
        .checked_mul(state.intents.len())
        .and_then(|count| count.checked_mul(3))
        .and_then(|count| {
            decision
                .effects
                .len()
                .checked_mul(state.timers.len())
                .and_then(|timers| count.checked_add(timers))
        })
        .and_then(|count| count.checked_add(state.decisions.len()))
        .and_then(|count| count.checked_add(state.intents.len()))
        .ok_or(EffectEmissionError::Capacity)?;
    if decision.effects.len() > limits.maximum_effects || comparisons > limits.maximum_comparisons {
        return Err(EffectEmissionError::Capacity);
    }
    let mut required_bytes = size_of::<Vec<DurableIntent>>()
        .checked_add(
            decision
                .effects
                .len()
                .checked_mul(size_of::<DurableIntent>())
                .ok_or(EffectEmissionError::Capacity)?,
        )
        .ok_or(EffectEmissionError::Capacity)?;
    for id in &decision.effects {
        let intent = state
            .intents
            .iter()
            .find(|intent| intent.id == *id)
            .ok_or(EffectEmissionError::MissingEffect)?;
        if intent.operation != operation || intent.basis != expected {
            return Err(EffectEmissionError::StaleEffect);
        }
        if intent.status != DurableStatus::Pending {
            return Err(EffectEmissionError::NotPending);
        }
        match intent.kind {
            EffectKind::ArmTimer | EffectKind::CancelTimer => {
                let timer = state
                    .timers
                    .iter()
                    .find(|timer| Some(timer.id) == intent.timer)
                    .ok_or(EffectEmissionError::InvalidTimerBinding)?;
                if timer.generation != intent.generation
                    || matches!(
                        timer.status,
                        DurableStatus::Completed | DurableStatus::Failed | DurableStatus::Cancelled
                    )
                {
                    return Err(EffectEmissionError::InvalidTimerBinding);
                }
            }
            EffectKind::RunAi
            | EffectKind::RunMedia
            | EffectKind::LoadMemoryCandidates
            | EffectKind::CancelJob
            | EffectKind::PublishPresentation => {}
        }
        required_bytes = required_bytes
            .checked_add(intent.definition.package.retained_heap_bytes())
            .and_then(|count| count.checked_add(intent.definition.entry.retained_heap_bytes()))
            .ok_or(EffectEmissionError::Capacity)?;
        if required_bytes > limits.maximum_retained_bytes {
            return Err(EffectEmissionError::Capacity);
        }
    }
    for intent in &state.intents {
        if intent.operation == operation && !decision.effects.contains(&intent.id) {
            return Err(EffectEmissionError::UndeclaredEffect);
        }
    }
    let mut effects = Vec::new();
    effects
        .try_reserve_exact(decision.effects.len())
        .map_err(|_| EffectEmissionError::Capacity)?;
    for id in &decision.effects {
        let intent = state
            .intents
            .iter()
            .find(|intent| intent.id == *id)
            .ok_or(EffectEmissionError::MissingEffect)?;
        effects.push(intent.clone());
    }
    // Count actual returned allocation capacity; neither allocator rounding nor clone behavior
    // is assumed to preserve the borrowed preflight estimate.
    let retained_bytes = effects.iter().try_fold(
        size_of::<Vec<DurableIntent>>()
            .checked_add(
                effects
                    .capacity()
                    .checked_mul(size_of::<DurableIntent>())
                    .ok_or(EffectEmissionError::Capacity)?,
            )
            .ok_or(EffectEmissionError::Capacity)?,
        |retained_bytes, intent| {
            retained_bytes
                .checked_add(intent.definition.package.retained_heap_bytes())
                .and_then(|retained_bytes| {
                    retained_bytes.checked_add(intent.definition.entry.retained_heap_bytes())
                })
                .ok_or(EffectEmissionError::Capacity)
        },
    )?;
    if retained_bytes > limits.maximum_retained_bytes {
        return Err(EffectEmissionError::Capacity);
    }
    Ok(effects)
}

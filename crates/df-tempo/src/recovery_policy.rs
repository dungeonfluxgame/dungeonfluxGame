//! Private D03 contract example, mounted only by the recovery_policy boundary test.
//! Counter pressure is supplied source policy, never a rulebook or device-safety threshold.
//! Existing canonical fatigue owns recovery, modes and authorized motion ceilings. This example
//! selects one least-fatigued affordable effect and charges its existing counter; it emits no cue.

use df_model::checkpoint::{Checkpoint, CheckpointPins, ContentReference};
use df_tempo::elapsed::{
    ElapsedDisposition, TempoAdvanceRequest, TempoElapsedLimits, TempoElapsedPolicy,
};
use df_tempo::fatigue::{
    FatigueAdvance, FatigueAdvanceError, FatigueModePolicy, MotionPreference, advance_fatigue_modes,
};

pub(crate) struct RecoveryRequest<'a> {
    pub elapsed: TempoAdvanceRequest,
    pub elapsed_policy: Option<&'a TempoElapsedPolicy>,
    pub fatigue_policy: Option<&'a FatigueModePolicy>,
    pub preference: Option<MotionPreference>,
    /// None explicitly recovers without selecting. Positive units charge one admitted use.
    pub pressure_units: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RotationDisposition {
    RecoveryOnly,
    Suppressed,
    Exhausted,
    Selected(ContentReference),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoveryProposal {
    pub advance: FatigueAdvance,
    pub rotation: RotationDisposition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecoveryError {
    Fatigue(FatigueAdvanceError),
    ZeroPressure,
}

/// Canonical presentation elapsed/recovery runs once. Selection requires an advanced anchor;
/// paused, replay and zero-time requests retain exact counters and never charge a fresh impact.
/// Least counter wins, with exact package/entry lexical ties independent of producer order.
/// An unaffordable effect is declined rather than clipped or silently charged beyond its cap.
/// The caller must source-admit policy/preferences and atomically stage/recheck this proposal.
pub(crate) fn advance_recovery(
    current: &Checkpoint,
    pins: &CheckpointPins,
    request: RecoveryRequest<'_>,
    limits: TempoElapsedLimits,
) -> Result<RecoveryProposal, RecoveryError> {
    if request.pressure_units == Some(0) {
        return Err(RecoveryError::ZeroPressure);
    }
    let mut advance = advance_fatigue_modes(
        current,
        pins,
        request.elapsed,
        request.elapsed_policy,
        limits,
        request.fatigue_policy,
        request.preference,
    )
    .map_err(RecoveryError::Fatigue)?;
    let Some(pressure_units) = request.pressure_units else {
        return Ok(RecoveryProposal {
            advance,
            rotation: RotationDisposition::RecoveryOnly,
        });
    };
    if advance.disposition != ElapsedDisposition::Advanced {
        return Ok(RecoveryProposal {
            advance,
            rotation: RotationDisposition::Suppressed,
        });
    }
    let policy = request.fatigue_policy.ok_or(RecoveryError::Fatigue(
        FatigueAdvanceError::PolicyUnavailable,
    ))?;
    let selected = advance
        .state
        .fatigue
        .iter()
        .enumerate()
        .filter(|(_, (definition, counter))| {
            policy.effects.iter().any(|rule| {
                rule.definition == *definition
                    && counter
                        .checked_add(pressure_units)
                        .is_some_and(|next| next <= rule.maximum_counter_units)
            })
        })
        .min_by_key(|(_, (definition, counter))| {
            (
                *counter,
                definition.package.as_str(),
                definition.entry.as_str(),
            )
        })
        .map(|(index, _)| index);
    let rotation = if let Some(index) = selected {
        // The index comes from this same bounded, unmodified vector, not caller input.
        let (definition, counter) = &mut advance.state.fatigue[index];
        *counter = counter
            .checked_add(pressure_units)
            .ok_or(RecoveryError::Fatigue(FatigueAdvanceError::CounterLimit))?;
        RotationDisposition::Selected(definition.clone())
    } else {
        RotationDisposition::Exhausted
    };
    Ok(RecoveryProposal { advance, rotation })
}

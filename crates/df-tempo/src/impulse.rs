//! Bounded scalar impulse/inertia staging under the tempo-engine policy contract.
//! Inertia is a signed intensity target offset that recovers toward zero. Explicit source-admitted
//! slew/deadband/shock coefficients avoid direct target jumps; timestamped linear segments avoid
//! per-tick work. These design units are neither calibrated device limits nor mechanical rules.
//! Content admission, permitted recommendation production, durable deduplication and commit remain
//! caller-owned. This boundary emits no one-shot cue and performs no I/O or hidden-state inference.

use crate::elapsed::{
    ElapsedDisposition, TempoAdvanceError, TempoAdvanceRequest, TempoElapsedLimits,
    TempoElapsedPolicy, advance_elapsed,
};
use df_model::checkpoint::{
    AudienceScope, Basis, Checkpoint, CheckpointPins, ContentReference, FactId, FactValue,
    TempoState,
};

/// Already source-admitted named event policy. Rates use intensity units per presentation tick.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamedImpulseRule {
    pub event: ContentReference,
    pub inertia_units: i64,
    pub rise_units_per_tick: u64,
    pub fall_units_per_tick: u64,
}

/// Immutable source-qualified coefficients, bound to the checkpoint's exact tempo policy.
/// Checking these values does not authenticate their source or establish production calibration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImpulsePolicy {
    pub definition: ContentReference,
    pub minimum_intensity: i64,
    pub maximum_intensity: i64,
    pub maximum_absolute_inertia: i64,
    pub recovery_units_per_tick: u64,
    pub rise_units_per_tick: u64,
    pub fall_units_per_tick: u64,
    pub deadband_units: u64,
    pub named_impulses: Vec<NamedImpulseRule>,
}

/// A current canonical public event, supplied by the admitted server input producer.
/// The presentation anchor prevents an old impulse request being reused after staging.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImpulseEvent {
    pub fact: FactId,
    pub at_presentation_ticks: u64,
}

/// The producer must qualify the target from permitted facts/recommendations before this call.
/// Basis equality proves freshness, not authorization. Only Shared named events are supported.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PermittedTempoInput {
    pub expected_basis: Basis,
    pub target_intensity: i64,
    pub impulse: Option<ImpulseEvent>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImpulseLimits {
    pub elapsed: TempoElapsedLimits,
    pub maximum_named_impulses: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImpulseError {
    Elapsed(TempoAdvanceError),
    PolicyUnavailable,
    PolicyMismatch,
    InvalidPolicy,
    Capacity,
    DuplicateRule,
    InvalidState,
    StaleInput,
    InvalidTarget,
    StaleImpulseAnchor,
    UnknownFact,
    StaleFact,
    HiddenImpulse,
    UnapprovedImpulse,
    ImpulseRequiresElapsed,
    Arithmetic,
}

/// Convex linear interpolation between admitted bounded endpoints, with whole-tick sampling.
/// Private fields preserve monotonic timestamps and the nonzero span. No extrapolation occurs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImpulseCurve {
    from_ticks: u64,
    through_ticks: u64,
    ticks_per_second: u32,
    from_intensity: i64,
    through_intensity: i64,
}

impl ImpulseCurve {
    pub fn from_ticks(&self) -> u64 {
        self.from_ticks
    }

    pub fn through_ticks(&self) -> u64 {
        self.through_ticks
    }

    pub fn ticks_per_second(&self) -> u32 {
        self.ticks_per_second
    }

    /// Returns None outside the admitted interval. Rounding is toward the starting endpoint.
    pub fn intensity_at(&self, presentation_ticks: u64) -> Option<i64> {
        if presentation_ticks > self.through_ticks {
            return None;
        }
        let presentation_tick_offset = presentation_ticks.checked_sub(self.from_ticks)?;
        let presentation_tick_span = self.through_ticks.checked_sub(self.from_ticks)?;
        let intensity_delta = i128::from(self.through_intensity) - i128::from(self.from_intensity);
        // Each factor is at most u64::MAX, so their product fits u128 even at full i64 range.
        let intensity_step = intensity_delta.unsigned_abs() * u128::from(presentation_tick_offset);
        let intensity_step =
            u64::try_from(intensity_step.checked_div(u128::from(presentation_tick_span))?).ok()?;
        let value =
            i128::from(self.from_intensity) + intensity_delta.signum() * i128::from(intensity_step);
        i64::try_from(value).ok()
    }
}

/// Detached candidate for the engine working copy; no checkpoint, publication or commit mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImpulseAdvance {
    pub basis: Basis,
    pub state: TempoState,
    pub elapsed_ticks: u64,
    pub disposition: ElapsedDisposition,
    pub curve: Option<ImpulseCurve>,
}

/// Stages one bounded segment and inertia recovery using only explicit presentation time.
/// The starting offset (plus any approved shock) selects a capped target for this segment;
/// source-supplied slew limits approach it, then recovery sets the next offset. Successive
/// segments express shock/release without allocating one entry per tick. All endpoints and
/// samples stay within policy caps, even when repeated shocks saturate inertia.
///
/// Pause/replay preserve exact recorded tempo and emit no curve after current validation.
/// PreparedOnly can stage the same pure proposal as Live; neither dispatches anything.
/// A zero-duration shock refuses; staging a shock therefore advances the anchor. Identical
/// requests on one immutable checkpoint are deterministic, not a durable deduplication claim.
/// The native owner rechecks basis/anchor and operation identity before its atomic commit.
pub fn advance_impulse(
    checkpoint: &Checkpoint,
    admitted: &CheckpointPins,
    request: TempoAdvanceRequest,
    elapsed_policy: Option<&TempoElapsedPolicy>,
    policy: Option<&ImpulsePolicy>,
    input: PermittedTempoInput,
    limits: ImpulseLimits,
) -> Result<ImpulseAdvance, ImpulseError> {
    let mut advanced = advance_elapsed(
        checkpoint,
        admitted,
        request,
        elapsed_policy,
        limits.elapsed,
    )
    .map_err(ImpulseError::Elapsed)?;
    let policy = policy.ok_or(ImpulseError::PolicyUnavailable)?;
    validate_policy(checkpoint, admitted, policy, limits)?;
    if input.expected_basis != checkpoint.basis() {
        return Err(ImpulseError::StaleInput);
    }
    if !(policy.minimum_intensity..=policy.maximum_intensity).contains(&input.target_intensity) {
        return Err(ImpulseError::InvalidTarget);
    }
    let rule = input
        .impulse
        .map(|event| validate_event(checkpoint, event, policy))
        .transpose()?;
    if matches!(
        advanced.disposition,
        ElapsedDisposition::Paused | ElapsedDisposition::Replay
    ) {
        return Ok(ImpulseAdvance {
            basis: checkpoint.basis(),
            state: advanced.state,
            elapsed_ticks: 0,
            disposition: advanced.disposition,
            curve: None,
        });
    }
    if advanced.elapsed_ticks == 0 {
        if rule.is_some() {
            return Err(ImpulseError::ImpulseRequiresElapsed);
        }
        return Ok(ImpulseAdvance {
            basis: checkpoint.basis(),
            state: advanced.state,
            elapsed_ticks: 0,
            disposition: advanced.disposition,
            curve: None,
        });
    }
    let maximum_absolute_inertia_units = i128::from(policy.maximum_absolute_inertia);
    let inertia_offset_units = (i128::from(advanced.state.inertia)
        + i128::from(rule.map_or(0, |rule| rule.inertia_units)))
    .clamp(
        -maximum_absolute_inertia_units,
        maximum_absolute_inertia_units,
    );
    let target_intensity_units = (i128::from(input.target_intensity) + inertia_offset_units).clamp(
        i128::from(policy.minimum_intensity),
        i128::from(policy.maximum_intensity),
    );
    let starting_intensity_units = advanced.state.intensity;
    let intensity_delta = target_intensity_units - i128::from(starting_intensity_units);
    let slew_rate_units_per_tick = if intensity_delta >= 0 {
        rule.map_or(policy.rise_units_per_tick, |rule| rule.rise_units_per_tick)
    } else {
        rule.map_or(policy.fall_units_per_tick, |rule| rule.fall_units_per_tick)
    };
    let intensity_delta_units = intensity_delta.unsigned_abs();
    let intensity_step_units = if intensity_delta_units <= u128::from(policy.deadband_units) {
        0
    } else {
        bounded_product(
            slew_rate_units_per_tick,
            advanced.elapsed_ticks,
            intensity_delta_units,
        )?
    };
    advanced.state.intensity = i64::try_from(
        i128::from(starting_intensity_units)
            + intensity_delta.signum() * i128::from(intensity_step_units),
    )
    .map_err(|_| ImpulseError::Arithmetic)?;
    let inertia_recovery_units = bounded_product(
        policy.recovery_units_per_tick,
        advanced.elapsed_ticks,
        inertia_offset_units.unsigned_abs(),
    )?;
    advanced.state.inertia = i64::try_from(
        inertia_offset_units - inertia_offset_units.signum() * i128::from(inertia_recovery_units),
    )
    .map_err(|_| ImpulseError::Arithmetic)?;
    Ok(ImpulseAdvance {
        basis: checkpoint.basis(),
        curve: Some(ImpulseCurve {
            from_ticks: checkpoint.state().tempo.presentation_ticks,
            through_ticks: advanced.state.presentation_ticks,
            ticks_per_second: elapsed_policy
                .ok_or(ImpulseError::PolicyUnavailable)?
                .ticks_per_second,
            from_intensity: starting_intensity_units,
            through_intensity: advanced.state.intensity,
        }),
        state: advanced.state,
        elapsed_ticks: advanced.elapsed_ticks,
        disposition: advanced.disposition,
    })
}

fn bounded_product(
    rate_units_per_tick: u64,
    elapsed_ticks: u64,
    ceiling_units: u128,
) -> Result<u64, ImpulseError> {
    // Two u64 factors fit u128. The admitted intensity/offset difference fits u64.
    u64::try_from((u128::from(rate_units_per_tick) * u128::from(elapsed_ticks)).min(ceiling_units))
        .map_err(|_| ImpulseError::Arithmetic)
}

fn validate_policy(
    checkpoint: &Checkpoint,
    admitted: &CheckpointPins,
    policy: &ImpulsePolicy,
    limits: ImpulseLimits,
) -> Result<(), ImpulseError> {
    if policy.definition != checkpoint.state().tempo.policy {
        return Err(ImpulseError::PolicyMismatch);
    }
    if policy.named_impulses.len() > limits.maximum_named_impulses {
        return Err(ImpulseError::Capacity);
    }
    if policy.minimum_intensity > policy.maximum_intensity || policy.maximum_absolute_inertia < 0 {
        return Err(ImpulseError::InvalidPolicy);
    }
    let state = &checkpoint.state().tempo;
    if !(policy.minimum_intensity..=policy.maximum_intensity).contains(&state.intensity)
        || i128::from(state.inertia).abs() > i128::from(policy.maximum_absolute_inertia)
    {
        return Err(ImpulseError::InvalidState);
    }
    for (index, rule) in policy.named_impulses.iter().enumerate() {
        if rule.event.package != admitted.content.package {
            return Err(ImpulseError::PolicyMismatch);
        }
        if policy
            .named_impulses
            .iter()
            .take(index)
            .any(|prior| prior.event == rule.event)
        {
            return Err(ImpulseError::DuplicateRule);
        }
    }
    Ok(())
}

fn validate_event<'a>(
    checkpoint: &Checkpoint,
    event: ImpulseEvent,
    policy: &'a ImpulsePolicy,
) -> Result<&'a NamedImpulseRule, ImpulseError> {
    if event.at_presentation_ticks != checkpoint.state().tempo.presentation_ticks {
        return Err(ImpulseError::StaleImpulseAnchor);
    }
    let fact = checkpoint
        .state()
        .facts
        .iter()
        .find(|fact| fact.id == event.fact)
        .ok_or(ImpulseError::UnknownFact)?;
    if fact.revision != checkpoint.basis().revision {
        return Err(ImpulseError::StaleFact);
    }
    if fact.audience != AudienceScope::Shared {
        return Err(ImpulseError::HiddenImpulse);
    }
    let FactValue::ContentEvent { definition, .. } = &fact.value else {
        return Err(ImpulseError::UnapprovedImpulse);
    };
    policy
        .named_impulses
        .iter()
        .find(|rule| rule.event == *definition)
        .ok_or(ImpulseError::UnapprovedImpulse)
}

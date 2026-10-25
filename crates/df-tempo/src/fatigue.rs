//! Bounded pure fatigue staging. Content policy and authorized preferences are supplied by
//! the native composition owner; these values do not authenticate their own provenance.
use crate::elapsed::{
    ElapsedDisposition, TempoAdvanceError, TempoAdvanceRequest, TempoElapsedLimits,
    TempoElapsedPolicy, advance_elapsed,
};
use df_model::checkpoint::{
    Basis, Checkpoint, CheckpointPins, ContentReference, ExecutionMode, TempoState,
};

/// Counter units and decay are explicit authored policy, not inferred from inactivity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectFatigueRule {
    pub definition: ContentReference,
    pub maximum_counter_units: u64,
    pub recovery_units_per_presentation_tick: u64,
}

/// A permission ceiling on an already approved plan; it is not a camera command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MotionAllowance {
    pub screen_shake: bool,
    pub forced_camera_punch: bool,
    pub readable_emphasis: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FatigueModePolicy {
    pub definition: ContentReference,
    pub effects: Vec<EffectFatigueRule>,
    pub normal_motion: MotionAllowance,
}

/// This input must come from the current authorized server preference producer.
/// Basis equality provides freshness only; it does not prove membership or consent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MotionPreference {
    pub expected_basis: Basis,
    pub reduced_motion: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FatigueAdvance {
    pub state: TempoState,
    pub elapsed_ticks: u64,
    pub disposition: ElapsedDisposition,
    pub execution_mode: ExecutionMode,
    pub motion: MotionAllowance,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FatigueAdvanceError {
    Elapsed(TempoAdvanceError),
    PolicyUnavailable,
    PreferenceUnavailable,
    StalePreference,
    PolicyMismatch,
    Capacity,
    DuplicateFatigue,
    DuplicateRule,
    MissingEffectRule,
    UnexpectedEffectRule,
    InvalidMotionPolicy,
    CounterLimit,
    RecoveryOverflow,
}

/// Calls the sole elapsed owner, then stages recovery without mutating the checkpoint.
/// Paused/replay requests preserve fatigue exactly. Replay never emits fresh impacts.
/// No game-time, execution-mode, activity, decisions or mechanical fields are changed.
pub fn advance_fatigue_modes(
    checkpoint: &Checkpoint,
    admitted: &CheckpointPins,
    request: TempoAdvanceRequest,
    elapsed_policy: Option<&TempoElapsedPolicy>,
    limits: TempoElapsedLimits,
    mode_policy: Option<&FatigueModePolicy>,
    preference: Option<MotionPreference>,
) -> Result<FatigueAdvance, FatigueAdvanceError> {
    let policy = mode_policy.ok_or(FatigueAdvanceError::PolicyUnavailable)?;
    let preference = preference.ok_or(FatigueAdvanceError::PreferenceUnavailable)?;
    if preference.expected_basis != checkpoint.basis() {
        return Err(FatigueAdvanceError::StalePreference);
    }
    if policy.definition != checkpoint.state().tempo.policy {
        return Err(FatigueAdvanceError::PolicyMismatch);
    }
    if policy.effects.len() > limits.maximum_fatigue_entries {
        return Err(FatigueAdvanceError::Capacity);
    }
    if !policy.normal_motion.readable_emphasis {
        return Err(FatigueAdvanceError::InvalidMotionPolicy);
    }
    let mut advanced = advance_elapsed(checkpoint, admitted, request, elapsed_policy, limits)
        .map_err(FatigueAdvanceError::Elapsed)?;
    validate_effect_rules(&advanced.state, policy)?;
    if advanced.disposition == ElapsedDisposition::Advanced {
        for (effect_definition, fatigue_counter_units) in &mut advanced.state.fatigue {
            let rule = policy
                .effects
                .iter()
                .find(|rule| rule.definition == *effect_definition)
                .ok_or(FatigueAdvanceError::MissingEffectRule)?;
            let recovery = rule
                .recovery_units_per_presentation_tick
                .checked_mul(advanced.elapsed_ticks)
                .ok_or(FatigueAdvanceError::RecoveryOverflow)?;
            // Recovery consumes existing fatigue and never creates negative debt.
            *fatigue_counter_units = fatigue_counter_units.saturating_sub(recovery);
        }
    }
    let motion = if preference.reduced_motion || advanced.disposition == ElapsedDisposition::Replay
    {
        MotionAllowance {
            screen_shake: false,
            forced_camera_punch: false,
            readable_emphasis: true,
        }
    } else {
        policy.normal_motion
    };
    Ok(FatigueAdvance {
        state: advanced.state,
        elapsed_ticks: advanced.elapsed_ticks,
        disposition: advanced.disposition,
        execution_mode: checkpoint.state().mode,
        motion,
    })
}

fn validate_effect_rules(
    state: &TempoState,
    policy: &FatigueModePolicy,
) -> Result<(), FatigueAdvanceError> {
    for (entry_index, (effect_definition, fatigue_counter_units)) in
        state.fatigue.iter().enumerate()
    {
        if state
            .fatigue
            .iter()
            .take(entry_index)
            .any(|(prior_definition, _)| prior_definition == effect_definition)
        {
            return Err(FatigueAdvanceError::DuplicateFatigue);
        }
        let rule = policy
            .effects
            .iter()
            .find(|rule| rule.definition == *effect_definition)
            .ok_or(FatigueAdvanceError::MissingEffectRule)?;
        if *fatigue_counter_units > rule.maximum_counter_units {
            return Err(FatigueAdvanceError::CounterLimit);
        }
    }
    for (rule_index, rule) in policy.effects.iter().enumerate() {
        if policy
            .effects
            .iter()
            .take(rule_index)
            .any(|prior| prior.definition == rule.definition)
        {
            return Err(FatigueAdvanceError::DuplicateRule);
        }
        if !state
            .fatigue
            .iter()
            .any(|(effect_definition, _)| *effect_definition == rule.definition)
        {
            return Err(FatigueAdvanceError::UnexpectedEffectRule);
        }
    }
    Ok(())
}

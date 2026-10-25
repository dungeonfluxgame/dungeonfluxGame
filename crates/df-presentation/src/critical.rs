//! Source-admitted committed outcomes gate optional celebratory cues.
use df_model::checkpoint::{
    AcceptedDecision, AssetReference, AudienceScope, Basis, Checkpoint, CheckpointError,
    CheckpointLimits, CheckpointPins, ContentReference, CriticalCueEligibility, ExecutionMode,
    FactValue, GameFact, LogicalTime, ReferenceInventory, ResolutionId, RuleReference,
};

/// Exact current canonical cue. Identity equality does not grant source or disclosure rights.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CriticalContext<'a> {
    pub basis: Basis,
    pub pins: &'a CheckpointPins,
    pub cue: &'a CriticalCueEligibility,
}

/// A rules/source owner classifies the resolved result; a die value never supplies this admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolvedOutcome {
    Succeeded,
    Failed,
}

/// Borrowed source-owner admission, not a persisted result schema or a source-validation issuer.
/// The owner must recheck its current source authorization before constructing this value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedOutcomeAdmission<'a> {
    pub context: CriticalContext<'a>,
    pub fact: &'a GameFact,
    pub decision: &'a AcceptedDecision,
    pub resolution: ResolutionId,
    pub source: &'a RuleReference,
    pub outcome: ResolvedOutcome,
}

/// Current cadence/fatigue decision supplied by the owning policy; no thresholds are inferred here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CueCadence {
    Allowed,
    Suppressed,
}

/// Current audience/rights/content/complete-asset filtered records from trusted owners.
/// Asset references do not prove byte integrity: the existing assets owner admits complete bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PermittedCriticalCue<'a> {
    pub context: CriticalContext<'a>,
    pub fact: &'a GameFact,
    pub content: &'a [ContentReference],
    pub assets: &'a [AssetReference],
    pub cadence: CueCadence,
    pub expires: LogicalTime,
}

/// Bounds external inventories and candidate work; canonical state uses CheckpointLimits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CriticalCueLimits {
    pub maximum_items: usize,
    pub maximum_asset_bytes: u64,
    pub maximum_duration_ticks: u64,
}

/// Explicit presentation time and caller-admitted bounds; this does not advance logical game time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CriticalCueRequest<'a> {
    pub context: CriticalContext<'a>,
    pub now: LogicalTime,
    pub duration_ticks: u64,
    pub limits: CriticalCueLimits,
}

/// Payload-free classifications for the caller's shared instrumentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CriticalCueError {
    Checkpoint(CheckpointError),
    InvalidLimits,
    Capacity,
    InvalidCurrentCue,
    MissingOutcomeAdmission,
    InvalidOutcomeAdmission,
    OutcomeNotSuccessful,
    PendingResolution,
    MissingPermission,
    StalePermission,
    AudienceDenied,
    AssetsUnavailable,
    InvalidTime,
    Expired,
    Suppressed,
    ReplaySuppressed,
}

/// Gate celebration on an exact committed, source-admitted and currently permitted outcome.
///
/// No value of an ActualDraw, arbitrary event label or accepted input can infer success. The
/// trusted rules/source owner explicitly admits the resolved outcome; structural checkpoint
/// validation alone cannot qualify its semantics. Matching pending work blocks celebration.
/// The result borrows the existing canonical cue, without mutation, I/O, provider dispatch,
/// new timing/rule authority or playback promises. The session/presentation owner still owns
/// current authorization, one-shot deduplication, publication, interruption and playback.
pub fn select_committed_celebration<'a>(
    checkpoint: &Checkpoint,
    request: CriticalCueRequest<'a>,
    inventory: ReferenceInventory<'_>,
    checkpoint_limits: CheckpointLimits,
    outcome: Option<ResolvedOutcomeAdmission<'_>>,
    permitted: Option<PermittedCriticalCue<'_>>,
) -> Result<&'a CriticalCueEligibility, CriticalCueError> {
    let limits = request.limits;
    if limits.maximum_items == 0
        || limits.maximum_items > 4096
        || limits.maximum_asset_bytes == 0
        || limits.maximum_duration_ticks == 0
    {
        return Err(CriticalCueError::InvalidLimits);
    }
    bound_inputs(request, inventory, checkpoint_limits, outcome, permitted)?;
    let current = request.context;
    checkpoint
        .validate_admitted(current.basis, current.pins, inventory, checkpoint_limits)
        .map_err(CriticalCueError::Checkpoint)?;
    let state = checkpoint.state();
    if state.mode == ExecutionMode::Replay {
        return Err(CriticalCueError::ReplaySuppressed);
    }
    if !state.continuity.critical_cues.contains(current.cue) {
        return Err(CriticalCueError::InvalidCurrentCue);
    }
    let outcome = outcome.ok_or(CriticalCueError::MissingOutcomeAdmission)?;
    if outcome.context != current
        || outcome.resolution != current.cue.resolution
        || outcome.fact.id != current.cue.fact
        || !state.facts.contains(outcome.fact)
        || !state.decisions.contains(outcome.decision)
        || outcome.decision.operation != outcome.fact.operation
        || outcome.decision.revision != outcome.fact.revision
        || !outcome.decision.facts.contains(&outcome.fact.id)
        || outcome.source.catalog != current.pins.rules.catalog
        || !inventory.rules.contains(outcome.source)
        || !source_matches_record(outcome)
    {
        return Err(CriticalCueError::InvalidOutcomeAdmission);
    }
    if state
        .pending
        .iter()
        .any(|pending| pending.id == outcome.resolution)
    {
        return Err(CriticalCueError::PendingResolution);
    }
    if outcome.outcome != ResolvedOutcome::Succeeded {
        return Err(CriticalCueError::OutcomeNotSuccessful);
    }
    let permitted = permitted.ok_or(CriticalCueError::MissingPermission)?;
    if permitted.context != current || permitted.fact != outcome.fact {
        return Err(CriticalCueError::StalePermission);
    }
    if current.cue.audience != outcome.fact.audience {
        return Err(CriticalCueError::AudienceDenied);
    }
    if current.cue.policy.package != current.pins.content.package
        || !permitted.content.contains(&current.cue.policy)
    {
        return Err(CriticalCueError::AudienceDenied);
    }
    if permitted.cadence == CueCadence::Suppressed {
        return Err(CriticalCueError::Suppressed);
    }
    if request.now.ticks_per_second == 0
        || permitted.expires.ticks_per_second != request.now.ticks_per_second
        || request.duration_ticks == 0
        || request.duration_ticks > current.cue.maximum_duration_ticks
        || request.duration_ticks > limits.maximum_duration_ticks
    {
        return Err(CriticalCueError::InvalidTime);
    }
    if permitted.expires.ticks <= request.now.ticks
        || request.duration_ticks > permitted.expires.ticks - request.now.ticks
    {
        return Err(CriticalCueError::Expired);
    }
    if current.cue.ready_assets.is_empty() {
        return Err(CriticalCueError::AssetsUnavailable);
    }
    let mut asset_bytes = 0_u64;
    for (asset_index, asset) in current.cue.ready_assets.iter().enumerate() {
        if asset.byte_length == 0
            || !permitted.assets.contains(asset)
            || current
                .cue
                .ready_assets
                .iter()
                .take(asset_index)
                .any(|prior| prior.key == asset.key)
        {
            return Err(CriticalCueError::AssetsUnavailable);
        }
        asset_bytes = asset_bytes
            .checked_add(asset.byte_length)
            .ok_or(CriticalCueError::Capacity)?;
        if asset_bytes > limits.maximum_asset_bytes {
            return Err(CriticalCueError::Capacity);
        }
    }
    Ok(current.cue)
}

fn bound_inputs(
    request: CriticalCueRequest<'_>,
    inventory: ReferenceInventory<'_>,
    checkpoint_limits: CheckpointLimits,
    outcome: Option<ResolvedOutcomeAdmission<'_>>,
    permitted: Option<PermittedCriticalCue<'_>>,
) -> Result<(), CriticalCueError> {
    let mut item_count = 0_usize;
    let mut add_item_count = |length: usize| {
        item_count = item_count
            .checked_add(length)
            .ok_or(CriticalCueError::Capacity)?;
        if item_count > request.limits.maximum_items {
            return Err(CriticalCueError::Capacity);
        }
        Ok(())
    };
    for length in [
        request.context.cue.ready_assets.len(),
        audience_items(&request.context.cue.audience),
        inventory.rules.len(),
        inventory.content.len(),
        inventory.resources.len(),
        inventory.assets.len(),
    ] {
        add_item_count(length)?;
    }
    if let Some(outcome) = outcome {
        for length in [
            outcome.context.cue.ready_assets.len(),
            audience_items(&outcome.context.cue.audience),
            audience_items(&outcome.fact.audience),
            outcome.decision.facts.len(),
            outcome.decision.draws.len(),
            outcome.decision.effects.len(),
        ] {
            add_item_count(length)?;
        }
        add_item_count(fact_items(outcome.fact))?;
        if outcome
            .decision
            .semantic_output
            .as_ref()
            .is_some_and(|text| text.len() > checkpoint_limits.maximum_text_bytes)
        {
            return Err(CriticalCueError::Capacity);
        }
    }
    if let Some(permitted) = permitted {
        for length in [
            permitted.context.cue.ready_assets.len(),
            audience_items(&permitted.context.cue.audience),
            audience_items(&permitted.fact.audience),
            fact_items(permitted.fact),
            permitted.content.len(),
            permitted.assets.len(),
        ] {
            add_item_count(length)?;
        }
    }
    Ok(())
}

fn audience_items(audience: &AudienceScope) -> usize {
    match audience {
        AudienceScope::Members(members) => members.len(),
        AudienceScope::Shared | AudienceScope::Host => 0,
    }
}

fn fact_items(fact: &GameFact) -> usize {
    match &fact.value {
        FactValue::ContentEvent { subjects, .. } => subjects.len(),
        FactValue::RulingAccepted { ruling, .. } => audience_items(&ruling.audience),
        _ => 0,
    }
}

fn source_matches_record(outcome: ResolvedOutcomeAdmission<'_>) -> bool {
    match &outcome.fact.value {
        FactValue::DrawAccepted { .. } => false,
        FactValue::ChoiceAccepted {
            resolution, choice, ..
        } => *resolution == outcome.resolution && &choice.source == outcome.source,
        FactValue::RulingAccepted {
            resolution, ruling, ..
        } => *resolution == outcome.resolution && &ruling.source == outcome.source,
        FactValue::ResourceChanged { source, .. } => source == outcome.source,
        _ => true,
    }
}

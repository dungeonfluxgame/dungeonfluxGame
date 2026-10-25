use df_model::checkpoint::{
    AssetDemand, AssetReference, AudienceScope, Basis, CHECKPOINT_SCHEMA, Checkpoint,
    CheckpointError, CheckpointPins, ContentReference, EntityId, ExecutionMode, FactId,
    LogicalTime, NarrativeMoment, PresentationDemand, RecordId, ShotPlan,
};

/// Exact current committed records. Neither IDs nor equality grant disclosure rights.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MomentContext<'a> {
    pub basis: Basis,
    pub pins: &'a CheckpointPins,
    pub mode: ExecutionMode,
    pub moment: &'a NarrativeMoment,
    pub presentation: &'a PresentationDemand,
}

/// Source-approved alternatives in policy order, supplied by the content/knowledge owner.
/// These are borrowed canonical records, not a second persisted PresentationPlan schema.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MomentAlternative<'a> {
    pub context: MomentContext<'a>,
    pub shots: &'a [ShotPlan],
    pub demands: &'a [AssetDemand],
}

/// Current audience/source/rights/suppression-filtered inventory supplied by a trusted owner.
/// Caller rechecks access at disclosure; constructing this value is not authorization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PermittedMoment<'a> {
    pub context: MomentContext<'a>,
    pub facts: &'a [FactId],
    pub attributed_claims: &'a [RecordId],
    pub entities: &'a [EntityId],
    pub content: &'a [ContentReference],
    pub assets: &'a [AssetReference],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MomentLimits {
    pub maximum_alternatives: usize,
    pub maximum_items: usize,
    pub maximum_shots: usize,
    pub maximum_demands: usize,
    pub maximum_duration_ticks: u64,
    pub maximum_reference_bytes: u64,
    pub maximum_demand_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MomentError {
    Checkpoint(CheckpointError),
    InvalidLimits,
    Capacity,
    InvalidCurrentContext,
    StalePermittedContext,
    UnpermittedMoment,
    InvalidTimebase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MomentDisposition {
    Selected,
    Unavailable,
    Expired,
}

/// Payload-free observations for the caller's shared instrumentation path.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MomentRejections {
    pub stale: usize,
    pub unpermitted: usize,
    pub invalid: usize,
    pub expired: usize,
}

/// A source-preserving projection of one permitted alternative.
///
/// `selected` borrows the canonical moment, shot, and demand records supplied by the caller.
/// Shot definitions, references, voice, and emphasized fact IDs remain descriptors for their
/// owning content/media consumers; this result does not resolve text, authorize bytes, or
/// promise playback.
pub struct MomentSelection<'a> {
    pub disposition: MomentDisposition,
    pub selected: Option<&'a MomentAlternative<'a>>,
    pub rejections: MomentRejections,
}

/// Select only records present in the immutable checkpoint supplied by the session owner.
///
/// Structural checkpoint validity is not audience/source authorization: the current filtered
/// inventory and approved alternative order still come from the existing trusted producer.
/// The scan and all nested candidate work are bounded before any record comparison. Selection
/// does not commit, authorize bytes, or make a tentative/generated moment canonical.
pub fn select_committed_moment_plan<'a>(
    checkpoint: &Checkpoint,
    current: MomentContext<'_>,
    now: LogicalTime,
    permitted: Option<PermittedMoment<'_>>,
    alternatives: &'a [MomentAlternative<'a>],
    limits: MomentLimits,
) -> Result<MomentSelection<'a>, MomentError> {
    validate_limits(limits)?;
    bound_input(current, permitted, alternatives, limits)?;
    // Reuse the canonical recovery guard, including redaction and unavailable-source refusal.
    checkpoint
        .validate_resume(current.basis, current.pins)
        .map_err(MomentError::Checkpoint)?;
    let state = checkpoint.state();
    let mut record_count = 0;
    for record_count_to_add in [
        state.continuity.moments.len(),
        state.presentation.len(),
        state.continuity.shots.len(),
        state.continuity.demands.len(),
        state.facts.len(),
        state.decisions.len(),
    ] {
        add_items(&mut record_count, record_count_to_add, limits.maximum_items)?;
    }
    for decision in &state.decisions {
        add_items(
            &mut record_count,
            decision.facts.len(),
            limits.maximum_items,
        )?;
    }
    if state.mode != current.mode
        || !state.continuity.moments.contains(current.moment)
        || !state.presentation.contains(current.presentation)
    {
        return Err(MomentError::InvalidCurrentContext);
    }
    // Background moment facts can be seeded. Only causal outcomes claim decision lineage.
    if current
        .presentation
        .causal_facts
        .iter()
        .any(|id| checkpoint.committed_outcome(*id).is_none())
    {
        return Err(MomentError::InvalidCurrentContext);
    }
    for alternative in alternatives {
        // Ignore obsolete alternatives; the selector reports them as stale without disclosure.
        if alternative.context != current {
            continue;
        }
        if !alternative
            .shots
            .iter()
            .all(|shot| state.continuity.shots.contains(shot))
            || !alternative
                .demands
                .iter()
                .all(|demand| state.continuity.demands.contains(demand))
        {
            return Err(MomentError::InvalidCurrentContext);
        }
    }
    select_moment_plan(current, now, permitted, alternatives, limits)
}

/// Select the first complete, current, permitted alternative without I/O or mutation.
///
/// The caller supplies policy order, approved semantics and tick units. This function
/// cannot infer source policy, tempo, consent, caption correspondence, current rights,
/// or a playback timeline. Missing inventory returns Unavailable, even in Live mode.
/// It never waits for media, dispatches providers, creates facts, or advances game time.
/// All nested input work is bounded before equality/membership comparisons or selection.
pub fn select_moment_plan<'a>(
    current: MomentContext<'_>,
    now: LogicalTime,
    permitted: Option<PermittedMoment<'_>>,
    alternatives: &'a [MomentAlternative<'a>],
    limits: MomentLimits,
) -> Result<MomentSelection<'a>, MomentError> {
    validate_limits(limits)?;
    bound_input(current, permitted, alternatives, limits)?;
    if now.ticks_per_second == 0 {
        return Err(MomentError::InvalidTimebase);
    }
    if current.presentation.source_revision != current.basis.revision
        || current.presentation.audience != current.moment.audience
        || !current
            .presentation
            .causal_facts
            .iter()
            .all(|fact| current.moment.facts.contains(fact))
    {
        return Err(MomentError::InvalidCurrentContext);
    }
    let mut selection = MomentSelection {
        disposition: MomentDisposition::Unavailable,
        selected: None,
        rejections: MomentRejections::default(),
    };
    let Some(permitted) = permitted else {
        return Ok(selection);
    };
    if permitted.context != current {
        return Err(MomentError::StalePermittedContext);
    }
    if !permitted.entities.contains(&current.moment.location)
        || !current
            .moment
            .characters
            .iter()
            .all(|id| permitted.entities.contains(id))
        || !current
            .moment
            .facts
            .iter()
            .all(|id| permitted.facts.contains(id))
        || !current
            .moment
            .attributed_claims
            .iter()
            .all(|id| permitted.attributed_claims.contains(id))
        || !content_allowed(&current.moment.semantic_focus, permitted)
        || !content_allowed(&current.presentation.definition, permitted)
    {
        return Err(MomentError::UnpermittedMoment);
    }
    for alternative in alternatives {
        if alternative.context != current {
            selection.rejections.stale += 1;
            continue;
        }
        match validate_alternative(alternative, now, permitted, limits) {
            Ok(()) => {
                selection.disposition = MomentDisposition::Selected;
                selection.selected = Some(alternative);
                return Ok(selection);
            }
            Err(AlternativeRefusal::Unpermitted) => selection.rejections.unpermitted += 1,
            Err(AlternativeRefusal::Invalid) => selection.rejections.invalid += 1,
            Err(AlternativeRefusal::Expired) => selection.rejections.expired += 1,
        }
    }
    if !alternatives.is_empty() && selection.rejections.expired == alternatives.len() {
        selection.disposition = MomentDisposition::Expired;
    }
    Ok(selection)
}

fn validate_limits(limits: MomentLimits) -> Result<(), MomentError> {
    if limits.maximum_alternatives == 0
        || limits.maximum_alternatives > 256
        || limits.maximum_items == 0
        || limits.maximum_items > 4096
        || limits.maximum_shots == 0
        || limits.maximum_shots > 256
        || limits.maximum_demands > 256
        || limits.maximum_duration_ticks == 0
        || limits.maximum_reference_bytes == 0
        || limits.maximum_demand_bytes == 0
    {
        return Err(MomentError::InvalidLimits);
    }
    Ok(())
}

fn audience_items(audience: &AudienceScope) -> usize {
    match audience {
        AudienceScope::Members(members) => members.len(),
        AudienceScope::Shared | AudienceScope::Host => 0,
    }
}

fn add_items(
    total: &mut usize,
    item_count: usize,
    maximum_items: usize,
) -> Result<(), MomentError> {
    *total = total.checked_add(item_count).ok_or(MomentError::Capacity)?;
    if *total > maximum_items {
        return Err(MomentError::Capacity);
    }
    Ok(())
}

fn context_items(
    context: MomentContext<'_>,
    total: &mut usize,
    maximum_items: usize,
) -> Result<(), MomentError> {
    for item_count in [
        context.moment.characters.len(),
        context.moment.facts.len(),
        context.moment.attributed_claims.len(),
        context.presentation.causal_facts.len(),
        audience_items(&context.moment.audience),
        audience_items(&context.presentation.audience),
    ] {
        add_items(total, item_count, maximum_items)?;
    }
    Ok(())
}

fn bound_input(
    current: MomentContext<'_>,
    permitted: Option<PermittedMoment<'_>>,
    alternatives: &[MomentAlternative<'_>],
    limits: MomentLimits,
) -> Result<(), MomentError> {
    if alternatives.len() > limits.maximum_alternatives {
        return Err(MomentError::Capacity);
    }
    let mut item_count = alternatives.len();
    context_items(current, &mut item_count, limits.maximum_items)?;
    if let Some(permitted) = permitted {
        context_items(permitted.context, &mut item_count, limits.maximum_items)?;
        for item_count_to_add in [
            permitted.facts.len(),
            permitted.attributed_claims.len(),
            permitted.entities.len(),
            permitted.content.len(),
            permitted.assets.len(),
        ] {
            add_items(&mut item_count, item_count_to_add, limits.maximum_items)?;
        }
    }
    for alternative in alternatives {
        context_items(alternative.context, &mut item_count, limits.maximum_items)?;
        if alternative.shots.len() > limits.maximum_shots
            || alternative.demands.len() > limits.maximum_demands
        {
            return Err(MomentError::Capacity);
        }
        add_items(
            &mut item_count,
            alternative.shots.len(),
            limits.maximum_items,
        )?;
        add_items(
            &mut item_count,
            alternative.demands.len(),
            limits.maximum_items,
        )?;
        for shot in alternative.shots {
            for item_count_to_add in [
                shot.subjects.len(),
                shot.references.len(),
                shot.performance.emphasis_facts.len(),
                audience_items(&shot.audience),
                usize::from(shot.performance.voice.is_some()),
            ] {
                add_items(&mut item_count, item_count_to_add, limits.maximum_items)?;
            }
        }
        for demand in alternative.demands {
            add_items(
                &mut item_count,
                demand.key.references.len(),
                limits.maximum_items,
            )?;
            add_items(
                &mut item_count,
                audience_items(&demand.key.audience),
                limits.maximum_items,
            )?;
        }
    }
    Ok(())
}

enum AlternativeRefusal {
    Invalid,
    Unpermitted,
    Expired,
}

fn content_allowed(reference: &ContentReference, permitted: PermittedMoment<'_>) -> bool {
    reference.package == permitted.context.pins.content.package
        && permitted.content.contains(reference)
}

fn validate_alternative(
    alternative: &MomentAlternative<'_>,
    now: LogicalTime,
    permitted: PermittedMoment<'_>,
    limits: MomentLimits,
) -> Result<(), AlternativeRefusal> {
    if alternative.shots.is_empty() {
        return Err(AlternativeRefusal::Invalid);
    }
    let alternative_context = alternative.context;
    let mut duration_ticks = 0_u64;
    let mut reference_bytes = 0_u64;
    for (shot_index, shot) in alternative.shots.iter().enumerate() {
        if shot.moment != alternative_context.moment.id
            || shot.audience != alternative_context.moment.audience
            || shot.duration_ticks == 0
            || alternative
                .shots
                .iter()
                .take(shot_index)
                .any(|prior| prior.id == shot.id)
        {
            return Err(AlternativeRefusal::Invalid);
        }
        duration_ticks = duration_ticks
            .checked_add(shot.duration_ticks)
            .ok_or(AlternativeRefusal::Invalid)?;
        if duration_ticks > limits.maximum_duration_ticks {
            return Err(AlternativeRefusal::Invalid);
        }
        if !content_allowed(&shot.definition, permitted)
            || !content_allowed(&shot.performance.definition, permitted)
            || !shot
                .subjects
                .iter()
                .all(|id| alternative_context.moment.characters.contains(id))
            || !shot
                .performance
                .emphasis_facts
                .iter()
                .all(|id| alternative_context.moment.facts.contains(id))
        {
            return Err(AlternativeRefusal::Unpermitted);
        }
        for reference in shot.references.iter().chain(shot.performance.voice.iter()) {
            check_reference(reference, permitted, &mut reference_bytes, limits)?;
        }
    }
    let mut demand_bytes = 0_u64;
    for (demand_index, demand) in alternative.demands.iter().enumerate() {
        if demand.basis != alternative_context.basis
            || demand.mode != alternative_context.mode
            || demand.key.schema != CHECKPOINT_SCHEMA
            || demand.key.source != alternative_context.pins.content.content_digest
            || demand.key.moment != alternative_context.moment.id
            || demand.key.identity != alternative_context.moment.identity_revision
            || demand.key.audience != alternative_context.moment.audience
            || demand.maximum_bytes == 0
            || demand.expires.ticks_per_second != now.ticks_per_second
            || alternative
                .demands
                .iter()
                .take(demand_index)
                .any(|prior| prior.id == demand.id)
        {
            return Err(AlternativeRefusal::Invalid);
        }
        if !content_allowed(&demand.policy, permitted) {
            return Err(AlternativeRefusal::Unpermitted);
        }
        if demand.expires.ticks <= now.ticks {
            return Err(AlternativeRefusal::Expired);
        }
        demand_bytes = demand_bytes
            .checked_add(demand.maximum_bytes)
            .ok_or(AlternativeRefusal::Invalid)?;
        if demand_bytes > limits.maximum_demand_bytes {
            return Err(AlternativeRefusal::Invalid);
        }
        for reference in &demand.key.references {
            check_reference(reference, permitted, &mut reference_bytes, limits)?;
        }
    }
    Ok(())
}

fn check_reference(
    reference: &AssetReference,
    permitted: PermittedMoment<'_>,
    total: &mut u64,
    limits: MomentLimits,
) -> Result<(), AlternativeRefusal> {
    if reference.byte_length == 0 || !permitted.assets.contains(reference) {
        return Err(AlternativeRefusal::Unpermitted);
    }
    *total = total
        .checked_add(reference.byte_length)
        .ok_or(AlternativeRefusal::Invalid)?;
    if *total > limits.maximum_reference_bytes {
        return Err(AlternativeRefusal::Invalid);
    }
    Ok(())
}

//! Bounded server-side forecasts describe heuristic intent, never calibrated probability.
//!
//! Decision: borrow canonical demands and policy, retain caller order, and expose only an
//! ordinal heuristic. No score can authorize bytes, disclose a future, change execution
//! mode, delay a committed moment, release Unknown liability, or erase demand history.
//! The current permitted inventory is a trusted-owner precondition, rechecked at disclosure;
//! equality with current records is a stale-input fence, not proof of access or rights.
//!
//! Alternatives rejected: a second persisted ForecastCandidate schema, percentage scores,
//! and a new fallback selector. Callers use the existing moment selector independently of
//! forecast success. Calibration, weights, forecast horizons, observed use/waste, latency,
//! cost, and G08/G12 qualification remain unresolved; this module supplies none of them.

use df_model::checkpoint::{
    AssetDemand, AudienceScope, CHECKPOINT_SCHEMA, LogicalTime, PrefetchPolicy,
};

use crate::moment_selection::{MomentContext, PermittedMoment};

/// Caller-supplied ordinal only. Larger values have no probability or rights meaning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeuristicRank(pub u32);

/// Neither state promises observed use, hit rate, calibrated probability, or profitability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Calibration {
    Uncalibrated,
    Unavailable,
}

/// A borrowed server-side candidate; no new persistent ID or application contract.
/// Branch is a caller-assigned index within the selected policy's branch bound.
/// Duration is supplied work duration in the explicit logical clock's tick units,
/// not an inferred forecast horizon or a playback timeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ForecastCandidate<'a> {
    pub demand: &'a AssetDemand,
    pub branch: u32,
    pub duration_ticks: u64,
    pub rank: HeuristicRank,
}

/// Caller bounds restrict work in addition to the canonical policy's own caps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ForecastLimits {
    pub maximum_candidates: usize,
    pub maximum_items: usize,
    pub maximum_reference_bytes: u64,
    pub maximum_demand_bytes: u64,
    pub maximum_duration_ticks: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForecastError {
    InvalidLimits,
    Capacity,
    InvalidTimebase,
    StaleContext,
    InvalidDemand,
    Expired,
    DuplicateDemand,
    ForecastUnavailable,
    InventoryUnavailable,
    Unpermitted,
}

/// Entire input accepted in supplied order, or a typed refusal with no partial result.
/// Borrowing preserves identities and liability references. Expiry is not cancellation.
#[derive(Debug)]
pub struct ForecastContract<'a> {
    candidates: &'a [ForecastCandidate<'a>],
    calibration: Calibration,
}

impl<'a> ForecastContract<'a> {
    pub fn candidates(&self) -> &'a [ForecastCandidate<'a>] {
        self.candidates
    }

    pub fn calibration(&self) -> Calibration {
        self.calibration
    }
}

/// Qualify supplied forecast intent without ranking, dispatch, I/O, mutation or fallback.
/// All nested work and byte sums are bounded before equality or inventory membership.
/// Missing inventory refuses forecast intent; it must not block the moment selector.
pub fn qualify_forecast<'a>(
    current: MomentContext<'_>,
    now: LogicalTime,
    permitted: Option<PermittedMoment<'_>>,
    policy: &PrefetchPolicy,
    candidates: &'a [ForecastCandidate<'a>],
    calibration: Calibration,
    limits: ForecastLimits,
) -> Result<ForecastContract<'a>, ForecastError> {
    if limits.maximum_candidates == 0
        || limits.maximum_candidates > 256
        || limits.maximum_items == 0
        || limits.maximum_items > 4096
        || limits.maximum_reference_bytes == 0
        || limits.maximum_demand_bytes == 0
        || limits.maximum_duration_ticks == 0
        || policy.maximum_candidates == 0
        || policy.maximum_candidates > 256
        || policy.maximum_branches == 0
        || policy.maximum_branches > 256
        || policy.maximum_bytes == 0
        || policy.maximum_duration_ticks == 0
    {
        return Err(ForecastError::InvalidLimits);
    }
    if candidates.len() > limits.maximum_candidates
        || candidates.len() > policy.maximum_candidates as usize
    {
        return Err(ForecastError::Capacity);
    }
    let mut item_count = candidates.len();
    bound_context(current, &mut item_count, limits.maximum_items)?;
    if let Some(inventory) = permitted {
        bound_context(inventory.context, &mut item_count, limits.maximum_items)?;
        for item_count_to_add in [
            inventory.facts.len(),
            inventory.attributed_claims.len(),
            inventory.entities.len(),
            inventory.content.len(),
            inventory.assets.len(),
        ] {
            add_items(&mut item_count, item_count_to_add, limits.maximum_items)?;
        }
    }
    let mut reference_bytes = 0;
    let mut demand_bytes = 0;
    let mut duration_ticks = 0;
    for candidate in candidates {
        let demand = candidate.demand;
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
        for reference in &demand.key.references {
            add_bytes(
                &mut reference_bytes,
                reference.byte_length,
                limits.maximum_reference_bytes,
            )?;
        }
        add_bytes(
            &mut demand_bytes,
            demand.maximum_bytes,
            limits.maximum_demand_bytes.min(policy.maximum_bytes),
        )?;
        add_bytes(
            &mut duration_ticks,
            candidate.duration_ticks,
            limits
                .maximum_duration_ticks
                .min(policy.maximum_duration_ticks),
        )?;
    }
    if candidates.is_empty() {
        return Err(ForecastError::ForecastUnavailable);
    }
    if now.ticks_per_second == 0
        || candidates
            .iter()
            .any(|candidate| candidate.demand.expires.ticks_per_second != now.ticks_per_second)
    {
        return Err(ForecastError::InvalidTimebase);
    }
    if current.presentation.source_revision != current.basis.revision
        || current.presentation.audience != current.moment.audience
        || !current
            .presentation
            .causal_facts
            .iter()
            .all(|fact| current.moment.facts.contains(fact))
    {
        return Err(ForecastError::StaleContext);
    }
    let inventory = permitted.ok_or(ForecastError::InventoryUnavailable)?;
    if inventory.context != current {
        return Err(ForecastError::StaleContext);
    }
    if policy.definition.package != current.pins.content.package
        || !inventory.content.contains(&policy.definition)
    {
        return Err(ForecastError::Unpermitted);
    }
    for (index, candidate) in candidates.iter().enumerate() {
        let demand = candidate.demand;
        if candidates[..index]
            .iter()
            .any(|prior| prior.demand.id == demand.id)
        {
            return Err(ForecastError::DuplicateDemand);
        }
        if demand.basis != current.basis
            || demand.mode != current.mode
            || demand.key.source != current.pins.content.content_digest
            || demand.key.moment != current.moment.id
            || demand.key.identity != current.moment.identity_revision
            || demand.key.audience != current.moment.audience
            || demand.policy != policy.definition
        {
            return Err(ForecastError::StaleContext);
        }
        if demand.key.schema != CHECKPOINT_SCHEMA
            || demand.maximum_bytes == 0
            || candidate.duration_ticks == 0
            || candidate.branch >= policy.maximum_branches
            || demand
                .key
                .references
                .iter()
                .any(|reference| reference.byte_length == 0)
        {
            return Err(ForecastError::InvalidDemand);
        }
        if demand.expires.ticks <= now.ticks {
            return Err(ForecastError::Expired);
        }
        if !demand
            .key
            .references
            .iter()
            .all(|reference| inventory.assets.contains(reference))
        {
            return Err(ForecastError::Unpermitted);
        }
    }
    Ok(ForecastContract {
        candidates,
        calibration,
    })
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
) -> Result<(), ForecastError> {
    *total = total
        .checked_add(item_count)
        .ok_or(ForecastError::Capacity)?;
    if *total > maximum_items {
        return Err(ForecastError::Capacity);
    }
    Ok(())
}

fn add_bytes(total: &mut u64, byte_count: u64, maximum_bytes: u64) -> Result<(), ForecastError> {
    *total = total
        .checked_add(byte_count)
        .ok_or(ForecastError::Capacity)?;
    if *total > maximum_bytes {
        return Err(ForecastError::Capacity);
    }
    Ok(())
}

fn bound_context(
    context: MomentContext<'_>,
    total: &mut usize,
    maximum_items: usize,
) -> Result<(), ForecastError> {
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

//! Authorized read projection of durable usage and existing Book receipts, never a money authority.
//! Raw outputs, prompts and tariffs are neither copied nor interpreted here.

use std::collections::BTreeMap;

use crate::cost::{
    CostRoute, CostUnits, PinnedCostReceipt, UsageEvidence, UsageRevision, pinned_call_id,
};
use crate::execution::{DispatchId, DispatchMode, ExecutionScope, ExecutionStageId, StageKind};
use crate::{ImageJobId, ImageOwner};
use df_types::Money;

use super::RuntimeUsage;

/// Finite bounds selected by the authorized reader; excess is rejected without truncation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsageProjectionLimits {
    pub max_attempts: usize,
    pub max_observations: usize,
    pub max_unit_lines: usize,
    pub max_word_bytes: usize,
}

/// Which read-projection envelope was exceeded; no record content is exposed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsageProjectionBound {
    Attempts,
    Observations,
    Receipts,
    UnitLines,
    WordBytes,
}

/// Projection failures never turn missing or conflicting evidence into a monetary value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsageProjectionError {
    InvalidLimits,
    BoundExceeded(UsageProjectionBound),
    ScopeMismatch,
    AttemptConflict,
    UnknownAttempt,
    RouteConflict,
    RevisionConflict,
    InvalidUnits,
    AccountingConflict,
    ReceiptCallMismatch,
    ReceiptRevisionMismatch,
    ReceiptObservationMismatch,
    ReceiptConflict,
    ReceiptWithoutUsage,
    CurrencyMismatch,
}

/// One stored execution attempt. Retries retain their own dispatch entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsageAttempt {
    pub scope: ExecutionScope,
    pub kind: StageKind,
    pub mode: DispatchMode,
}

/// A coherent snapshot supplied by the existing Book adapter, without a second pricing path.
pub type RawBookCostReceipt = PinnedCostReceipt;

/// Matched by the authorized runtime reader, never constructible by a consumer request.
#[derive(Clone, Eq, PartialEq)]
pub struct ActualCostReceipt {
    scope: ExecutionScope,
    record: RawBookCostReceipt,
}

impl std::fmt::Debug for ActualCostReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActualCostReceipt")
            .field("revision", &self.record.revision)
            .field("durable", &self.record.durable)
            .finish_non_exhaustive()
    }
}

impl ActualCostReceipt {
    /// Validates a host Book snapshot against the same durable raw observation, without repricing.
    pub(super) fn verify(
        record: RawBookCostReceipt,
        usage: &RuntimeUsage,
        limits: UsageProjectionLimits,
    ) -> Result<Self, UsageProjectionError> {
        validate_limits(limits)?;
        validate_usage(usage, limits)?;
        validate_route(&record.route, limits)?;
        validate_units(&record.units, limits)?;
        if record.call_id != pinned_call_id(usage.dispatch.as_bytes()) {
            return Err(UsageProjectionError::ReceiptCallMismatch);
        }
        if record.revision != usage.revision {
            return Err(UsageProjectionError::ReceiptRevisionMismatch);
        }
        if !same_route(&record.route, &usage.route) {
            return Err(UsageProjectionError::RouteConflict);
        }
        if record.evidence != usage.evidence
            || record.usable != usage.usable
            || !same_units(&record.units, effective_units(usage))
        {
            return Err(UsageProjectionError::ReceiptObservationMismatch);
        }
        if record
            .amount
            .is_some_and(|amount| amount.currency().as_str() != "USD")
        {
            return Err(UsageProjectionError::CurrencyMismatch);
        }
        Ok(Self {
            scope: scope_of(usage),
            record,
        })
    }
}

/// A Book amount, with uncertainty kept separate from a verified legitimate zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageUsageAmount {
    /// Stored local transform with no supplier dispatch or finance identity; never Money zero.
    NotApplicable,
    Pending,
    Unknown,
    /// The fully priced stored Book amount; evidence still identifies estimated quantities.
    Known(Money),
}

/// Exact heterogeneous units for one latest revision; no categories or attempts are summed.
#[derive(Clone, Eq, PartialEq)]
pub struct RecordedStageUsage {
    pub revision: UsageRevision,
    pub provider: String,
    pub model: String,
    pub units: Vec<CostUnits>,
    pub evidence: UsageEvidence,
    pub usable: bool,
}

impl std::fmt::Debug for RecordedStageUsage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordedStageUsage")
            .field("revision", &self.revision)
            .field("unit_lines", &self.units.len())
            .field("evidence", &self.evidence)
            .field("usable", &self.usable)
            .finish_non_exhaustive()
    }
}

/// No observation is different from an observed Unknown supplier outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StageUsageObservation {
    Pending,
    Recorded(RecordedStageUsage),
}

/// One stage attempt, including billed unusable work and distinct paid retries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageStageUsage {
    pub stage: ExecutionStageId,
    pub dispatch: DispatchId,
    pub kind: StageKind,
    pub observation: StageUsageObservation,
    pub amount: ImageUsageAmount,
}

/// A scoped read view. It intentionally has no mixed-category or monetary grand total.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageUsageSummary {
    pub job: ImageJobId,
    pub stages: Vec<ImageStageUsage>,
}

/// Supplied only after the service's stored authorization; this value does not confer access.
pub struct UsageProjectionInput<'a> {
    pub owner: &'a ImageOwner,
    pub job: ImageJobId,
    pub attempts: &'a [UsageAttempt],
    pub observations: &'a [RuntimeUsage],
    pub receipts: &'a [ActualCostReceipt],
    pub limits: UsageProjectionLimits,
}

/// Trusted infrastructure input from the coherent Book read port, never a client request DTO.
pub struct UsageBookProjectionInput<'a> {
    pub owner: &'a ImageOwner,
    pub job: ImageJobId,
    pub attempts: &'a [UsageAttempt],
    pub observations: &'a [RuntimeUsage],
    pub book_receipts: &'a [PinnedCostReceipt],
    pub limits: UsageProjectionLimits,
}

impl UsageProjectionLimits {
    pub fn validate(self) -> Result<(), UsageProjectionError> {
        validate_limits(self)
    }
}

/// Matches host Book reads to the latest raw journal facts after authorization and DB read.
/// This bridge validates identity, not a new issuer; the host owns Book-reader provenance.
pub fn project_book_usage(
    input: UsageBookProjectionInput<'_>,
) -> Result<ImageUsageSummary, UsageProjectionError> {
    let base = UsageProjectionInput {
        owner: input.owner,
        job: input.job,
        attempts: input.attempts,
        observations: input.observations,
        receipts: &[],
        limits: input.limits,
    };
    validate_limits(input.limits)?;
    check_bound(
        input.attempts.len(),
        input.limits.max_attempts,
        UsageProjectionBound::Attempts,
    )?;
    check_bound(
        input.observations.len(),
        input.limits.max_observations,
        UsageProjectionBound::Observations,
    )?;
    check_bound(
        input.book_receipts.len(),
        input.limits.max_observations,
        UsageProjectionBound::Receipts,
    )?;
    let attempts = index_attempts(&base)?;
    let latest = latest_observations(&base, &attempts)?;
    let by_call: BTreeMap<_, _> = latest
        .values()
        .map(|usage| (pinned_call_id(usage.dispatch.as_bytes()), *usage))
        .collect();
    let mut verified = Vec::with_capacity(input.book_receipts.len());
    for receipt in input.book_receipts {
        let usage = by_call
            .get(&receipt.call_id)
            .ok_or(UsageProjectionError::ReceiptCallMismatch)?;
        validate_route(&receipt.route, input.limits)?;
        validate_units(&receipt.units, input.limits)?;
        if !same_route(&receipt.route, &usage.route) {
            return Err(UsageProjectionError::RouteConflict);
        }
        if receipt
            .amount
            .is_some_and(|amount| amount.currency().as_str() != "USD")
        {
            return Err(UsageProjectionError::CurrencyMismatch);
        }
        // Cross-authority read skew is pending; a different revision supplies no amount.
        if receipt.revision != usage.revision {
            continue;
        }
        verified.push(ActualCostReceipt::verify(
            receipt.clone(),
            usage,
            input.limits,
        )?);
    }
    project_usage(UsageProjectionInput {
        owner: input.owner,
        job: input.job,
        attempts: input.attempts,
        observations: input.observations,
        receipts: &verified,
        limits: input.limits,
    })
}

type AttemptKey = (ExecutionStageId, DispatchId);

/// Projects the latest revision per attempt. Conflicts and bounded excess never silently drop data.
pub fn project_usage(
    input: UsageProjectionInput<'_>,
) -> Result<ImageUsageSummary, UsageProjectionError> {
    validate_limits(input.limits)?;
    check_bound(
        input.attempts.len(),
        input.limits.max_attempts,
        UsageProjectionBound::Attempts,
    )?;
    check_bound(
        input.observations.len(),
        input.limits.max_observations,
        UsageProjectionBound::Observations,
    )?;
    check_bound(
        input.receipts.len(),
        input.limits.max_observations,
        UsageProjectionBound::Receipts,
    )?;
    let attempts = index_attempts(&input)?;
    let latest = latest_observations(&input, &attempts)?;
    let receipts = current_receipts(&input, &attempts, &latest)?;
    let stages = attempts
        .into_iter()
        .map(|(key, attempt)| {
            let usage = latest.get(&key).copied();
            let receipt = receipts.get(&key).copied();
            ImageStageUsage {
                stage: key.0,
                dispatch: key.1,
                kind: attempt.kind,
                observation: usage.map_or(StageUsageObservation::Pending, |usage| {
                    StageUsageObservation::Recorded(RecordedStageUsage {
                        revision: usage.revision,
                        provider: usage.route.provider.clone(),
                        model: usage.route.model.clone(),
                        units: effective_units(usage).to_vec(),
                        evidence: usage.evidence,
                        usable: usage.usable,
                    })
                }),
                amount: if attempt.kind == StageKind::LocalTransform {
                    ImageUsageAmount::NotApplicable
                } else {
                    amount_of(usage, receipt)
                },
            }
        })
        .collect();
    Ok(ImageUsageSummary {
        job: input.job,
        stages,
    })
}

fn index_attempts<'a>(
    input: &UsageProjectionInput<'a>,
) -> Result<BTreeMap<AttemptKey, &'a UsageAttempt>, UsageProjectionError> {
    let mut attempts = BTreeMap::new();
    let mut dispatches = BTreeMap::new();
    let mut kinds = BTreeMap::new();
    for attempt in input.attempts {
        check_scope(&attempt.scope, input)?;
        let key = (attempt.scope.stage, attempt.scope.dispatch);
        if attempt.kind == StageKind::LocalTransform && !attempt.mode.is_local() {
            return Err(UsageProjectionError::AccountingConflict);
        }
        if let DispatchMode::Hosted(tuple) = attempt.mode
            && (tuple.attempt == [0; 16] || tuple.operation == [0; 16] || tuple.effect == [0; 16])
        {
            return Err(UsageProjectionError::AccountingConflict);
        }
        if attempts
            .get(&key)
            .is_some_and(|stored: &&UsageAttempt| stored.mode != attempt.mode)
        {
            return Err(UsageProjectionError::AccountingConflict);
        }
        if dispatches
            .insert(key.1, key.0)
            .is_some_and(|stage| stage != key.0)
            || kinds
                .insert(key.0, attempt.kind)
                .is_some_and(|kind| kind != attempt.kind)
        {
            return Err(UsageProjectionError::AttemptConflict);
        }
        attempts.entry(key).or_insert(attempt);
    }
    Ok(attempts)
}

fn latest_observations<'a>(
    input: &UsageProjectionInput<'a>,
    attempts: &BTreeMap<AttemptKey, &UsageAttempt>,
) -> Result<BTreeMap<AttemptKey, &'a RuntimeUsage>, UsageProjectionError> {
    let mut latest: BTreeMap<AttemptKey, &RuntimeUsage> = BTreeMap::new();
    let mut revisions: BTreeMap<(AttemptKey, UsageRevision), &RuntimeUsage> = BTreeMap::new();
    for usage in input.observations {
        check_scope(&scope_of(usage), input)?;
        let key = (usage.stage, usage.dispatch);
        let attempt = attempts
            .get(&key)
            .ok_or(UsageProjectionError::UnknownAttempt)?;
        if attempt.kind == StageKind::LocalTransform {
            return Err(UsageProjectionError::AccountingConflict);
        }
        validate_usage(usage, input.limits)?;
        if let Some(stored) = revisions.insert((key, usage.revision), usage)
            && !same_observation(stored, usage)
        {
            return Err(UsageProjectionError::RevisionConflict);
        }
        if let Some(stored) = latest.get(&key) {
            if !same_route(&stored.route, &usage.route) {
                return Err(UsageProjectionError::RouteConflict);
            }
            if usage.revision == stored.revision && !same_observation(stored, usage) {
                return Err(UsageProjectionError::RevisionConflict);
            }
            if usage.revision <= stored.revision {
                continue;
            }
        }
        latest.insert(key, usage);
    }
    Ok(latest)
}

fn current_receipts<'a>(
    input: &UsageProjectionInput<'a>,
    attempts: &BTreeMap<AttemptKey, &UsageAttempt>,
    latest: &BTreeMap<AttemptKey, &RuntimeUsage>,
) -> Result<BTreeMap<AttemptKey, &'a ActualCostReceipt>, UsageProjectionError> {
    let mut receipts: BTreeMap<AttemptKey, &ActualCostReceipt> = BTreeMap::new();
    for receipt in input.receipts {
        check_scope(&receipt.scope, input)?;
        validate_route(&receipt.record.route, input.limits)?;
        validate_units(&receipt.record.units, input.limits)?;
        let key = (receipt.scope.stage, receipt.scope.dispatch);
        if !attempts.contains_key(&key) {
            return Err(UsageProjectionError::UnknownAttempt);
        }
        let usage = latest
            .get(&key)
            .ok_or(UsageProjectionError::ReceiptWithoutUsage)?;
        if !same_route(&receipt.record.route, &usage.route) {
            return Err(UsageProjectionError::RouteConflict);
        }
        // A read-skewed old/new Book revision cannot supply this observation's amount.
        if receipt.record.revision != usage.revision {
            continue;
        }
        if receipt.record.evidence != usage.evidence
            || receipt.record.usable != usage.usable
            || !same_units(&receipt.record.units, effective_units(usage))
        {
            return Err(UsageProjectionError::ReceiptObservationMismatch);
        }
        if let Some(stored) = receipts.get(&key) {
            if stored.record.amount != receipt.record.amount {
                return Err(UsageProjectionError::ReceiptConflict);
            }
            if stored.record.durable {
                continue;
            }
        }
        receipts.insert(key, receipt);
    }
    Ok(receipts)
}

fn amount_of(
    usage: Option<&RuntimeUsage>,
    receipt: Option<&ActualCostReceipt>,
) -> ImageUsageAmount {
    let Some(usage) = usage else {
        return ImageUsageAmount::Pending;
    };
    if usage.evidence == UsageEvidence::Unknown || effective_units(usage).is_empty() {
        return ImageUsageAmount::Unknown;
    }
    let Some(receipt) = receipt.filter(|receipt| receipt.record.durable) else {
        return ImageUsageAmount::Pending;
    };
    receipt
        .record
        .amount
        .map_or(ImageUsageAmount::Unknown, ImageUsageAmount::Known)
}

fn validate_limits(limits: UsageProjectionLimits) -> Result<(), UsageProjectionError> {
    if limits.max_attempts == 0
        || limits.max_observations == 0
        || limits.max_unit_lines == 0
        || limits.max_word_bytes == 0
    {
        return Err(UsageProjectionError::InvalidLimits);
    }
    Ok(())
}

fn check_bound(
    actual: usize,
    maximum: usize,
    bound: UsageProjectionBound,
) -> Result<(), UsageProjectionError> {
    if actual > maximum {
        return Err(UsageProjectionError::BoundExceeded(bound));
    }
    Ok(())
}

fn check_scope(
    scope: &ExecutionScope,
    input: &UsageProjectionInput<'_>,
) -> Result<(), UsageProjectionError> {
    if &scope.owner != input.owner || scope.job != input.job {
        return Err(UsageProjectionError::ScopeMismatch);
    }
    Ok(())
}

fn scope_of(usage: &RuntimeUsage) -> ExecutionScope {
    ExecutionScope {
        owner: usage.owner.clone(),
        job: usage.job,
        stage: usage.stage,
        dispatch: usage.dispatch,
    }
}

fn validate_usage(
    usage: &RuntimeUsage,
    limits: UsageProjectionLimits,
) -> Result<(), UsageProjectionError> {
    validate_route(&usage.route, limits)?;
    validate_units(&usage.units, limits)
}

fn validate_route(
    route: &CostRoute,
    limits: UsageProjectionLimits,
) -> Result<(), UsageProjectionError> {
    for word in [&route.provider, &route.model, &route.cause, &route.detail] {
        check_bound(
            word.len(),
            limits.max_word_bytes,
            UsageProjectionBound::WordBytes,
        )?;
    }
    validate_units(&route.maxima, limits)
}

fn validate_units(
    units: &[CostUnits],
    limits: UsageProjectionLimits,
) -> Result<(), UsageProjectionError> {
    check_bound(
        units.len(),
        limits.max_unit_lines,
        UsageProjectionBound::UnitLines,
    )?;
    for unit in units {
        if unit.category.is_empty() {
            return Err(UsageProjectionError::InvalidUnits);
        }
        check_bound(
            unit.category.len(),
            limits.max_word_bytes,
            UsageProjectionBound::WordBytes,
        )?;
    }
    let ordered = ordered_units(units);
    if ordered
        .windows(2)
        .any(|pair| matches!(pair, [(a, _), (b, _)] if a == b))
    {
        return Err(UsageProjectionError::InvalidUnits);
    }
    Ok(())
}

fn ordered_units(units: &[CostUnits]) -> Vec<(&str, u64)> {
    let mut ordered: Vec<_> = units
        .iter()
        .map(|unit| (unit.category.as_str(), unit.quantity))
        .collect();
    ordered.sort_unstable();
    ordered
}

fn same_units(first: &[CostUnits], second: &[CostUnits]) -> bool {
    ordered_units(first) == ordered_units(second)
}

fn same_route(first: &CostRoute, second: &CostRoute) -> bool {
    first.provider == second.provider
        && first.model == second.model
        && first.cause == second.cause
        && first.detail == second.detail
        && same_units(&first.maxima, &second.maxima)
}

fn effective_units(usage: &RuntimeUsage) -> &[CostUnits] {
    if usage.evidence == UsageEvidence::Unknown || usage.units.is_empty() {
        &usage.route.maxima
    } else {
        &usage.units
    }
}

fn same_observation(first: &RuntimeUsage, second: &RuntimeUsage) -> bool {
    first.evidence == second.evidence
        && first.usable == second.usable
        && same_units(effective_units(first), effective_units(second))
}

#[cfg(test)]
#[path = "usage_projection_tests.rs"]
mod tests;

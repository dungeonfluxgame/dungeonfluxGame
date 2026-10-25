use df_model::checkpoint::{
    AcceptedDecision, ActualDraw, Basis, CheckpointPins, JobCompletion, JobId, JobOutcome,
    ResolutionId, RuleReference, WindowId,
};
use df_types::{OperationId, RevisionLabel, SessionRevision};

/// Exact caller-admitted fixture provenance. Matching it grants no game authority.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ReplayContext<'a> {
    pub basis: Basis,
    pub pins: &'a CheckpointPins,
    pub policy: &'a RevisionLabel,
}

/// Explicit positive traversal and aggregate semantic text bounds, without defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplayLimits {
    pub maximum_records: usize,
    pub maximum_semantic_bytes: usize,
}

/// Fixture refusals retain neither private semantic text nor provider payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayError {
    Capacity,
    Stale,
    Missing,
    MissingSemantic,
    Duplicate,
    InvalidDraw,
    InvalidOrder,
    InvalidRecord,
    CounterOverflow,
    Unused { remaining: usize },
}

/// A supplied draw request binds the entire source and timing context, not just die size.
pub struct DrawRequest<'a> {
    pub context: ReplayContext<'a>,
    pub operation: OperationId,
    pub ordinal: u32,
    pub resolution: ResolutionId,
    pub window: WindowId,
    pub sides: u32,
    pub source: &'a RuleReference,
}

/// Test-only ordered recorded dice for one operation, including a resumed counter.
///
/// Construction validates all supplied records. Consumption returns the original
/// canonical record; no RNG, seed, I/O or provider callable exists. Failed requests
/// change neither the next ordinal nor consumed count. A terminal u32 ordinal that
/// cannot advance the next counter is refused before consumption. These records
/// do not establish source qualification or legal mechanics; those remain rules-owned.
pub struct SuppliedDice<'a> {
    context: ReplayContext<'a>,
    operation: OperationId,
    records: &'a [ActualDraw],
    consumed: usize,
    next_ordinal: u32,
}

impl<'a> SuppliedDice<'a> {
    pub fn new(
        context: ReplayContext<'a>,
        operation: OperationId,
        next_ordinal: u32,
        records: &'a [ActualDraw],
        limits: ReplayLimits,
    ) -> Result<Self, ReplayError> {
        validate_limits(limits)?;
        if records.len() > limits.maximum_records {
            return Err(ReplayError::Capacity);
        }
        for (index, record) in records.iter().enumerate() {
            if records
                .iter()
                .take(index)
                .any(|previous| previous.ordinal == record.ordinal)
            {
                return Err(ReplayError::Duplicate);
            }
            if record.operation != operation || record.source.catalog != context.pins.rules.catalog
            {
                return Err(ReplayError::Stale);
            }
            if record.sides == 0 || record.value == 0 || record.value > record.sides {
                return Err(ReplayError::InvalidDraw);
            }
            let offset = u32::try_from(index).map_err(|_| ReplayError::CounterOverflow)?;
            let expected = next_ordinal
                .checked_add(offset)
                .ok_or(ReplayError::CounterOverflow)?;
            if record.ordinal != expected {
                return Err(ReplayError::InvalidOrder);
            }
        }
        Ok(Self {
            context,
            operation,
            records,
            consumed: 0,
            next_ordinal,
        })
    }

    pub fn next_ordinal(&self) -> u32 {
        self.next_ordinal
    }

    pub fn consumed(&self) -> usize {
        self.consumed
    }

    pub fn remaining(&self) -> usize {
        self.records.len() - self.consumed
    }

    pub fn take(&mut self, request: DrawRequest<'_>) -> Result<&'a ActualDraw, ReplayError> {
        if request.context != self.context || request.operation != self.operation {
            return Err(ReplayError::Stale);
        }
        if request.ordinal < self.next_ordinal {
            return Err(ReplayError::Duplicate);
        }
        if request.ordinal != self.next_ordinal {
            return Err(ReplayError::InvalidOrder);
        }
        let record = self
            .records
            .get(self.consumed)
            .ok_or(ReplayError::Missing)?;
        if record.resolution != request.resolution
            || record.window != request.window
            || record.source != *request.source
        {
            return Err(ReplayError::Stale);
        }
        if request.sides != record.sides {
            return Err(ReplayError::InvalidDraw);
        }
        let next_ordinal = self
            .next_ordinal
            .checked_add(1)
            .ok_or(ReplayError::CounterOverflow)?;
        let consumed = self
            .consumed
            .checked_add(1)
            .ok_or(ReplayError::CounterOverflow)?;
        self.next_ordinal = next_ordinal;
        self.consumed = consumed;
        Ok(record)
    }

    /// Refuses success when a fixture silently leaves supplied outcomes unused.
    pub fn finish(&self) -> Result<(), ReplayError> {
        match self.remaining() {
            0 => Ok(()),
            remaining => Err(ReplayError::Unused { remaining }),
        }
    }
}

/// Identity of one recorded native AI completion, including its original generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedJobKey {
    pub operation: OperationId,
    pub job: JobId,
    pub generation: u64,
}

/// Borrowed recorded semantic history; repeated reads preserve exact accepted records.
///
/// Missing/stale history is a typed refusal with no generation fallback. This fixture
/// does not apply state, migrate schemas, reconstruct reducers, or claim state hashes.
/// The caller admits source/access context; canonical decisions and native AI jobs
/// retain their original revisions, draw ordinals, output bytes, policy and model.
pub struct SemanticReplay<'a> {
    context: ReplayContext<'a>,
    decisions: &'a [AcceptedDecision],
    jobs: &'a [JobCompletion],
}

impl<'a> SemanticReplay<'a> {
    pub fn new(
        context: ReplayContext<'a>,
        decisions: &'a [AcceptedDecision],
        jobs: &'a [JobCompletion],
        limits: ReplayLimits,
    ) -> Result<Self, ReplayError> {
        validate_limits(limits)?;
        let mut record_count = decisions
            .len()
            .checked_add(jobs.len())
            .ok_or(ReplayError::Capacity)?;
        if record_count > limits.maximum_records {
            return Err(ReplayError::Capacity);
        }
        let mut bytes = 0usize;
        for (index, decision) in decisions.iter().enumerate() {
            if decisions
                .iter()
                .take(index)
                .any(|previous| previous.operation == decision.operation)
            {
                return Err(ReplayError::Duplicate);
            }
            if decision.revision > context.basis.revision
                || decision.source_policy != *context.policy
            {
                return Err(ReplayError::Stale);
            }
            for related_record_count in [
                decision.facts.len(),
                decision.draws.len(),
                decision.effects.len(),
            ] {
                record_count = record_count
                    .checked_add(related_record_count)
                    .ok_or(ReplayError::Capacity)?;
                if record_count > limits.maximum_records {
                    return Err(ReplayError::Capacity);
                }
            }
            if repeated(&decision.facts) || repeated(&decision.draws) || repeated(&decision.effects)
            {
                return Err(ReplayError::Duplicate);
            }
            if let Some(output) = &decision.semantic_output {
                count_text(&mut bytes, output, limits)?;
            }
        }
        for (index, job) in jobs.iter().enumerate() {
            if jobs
                .iter()
                .take(index)
                .any(|previous| previous.job == job.job)
            {
                return Err(ReplayError::Duplicate);
            }
            if job.basis != context.basis {
                return Err(ReplayError::Stale);
            }
            if job.generation == 0 {
                return Err(ReplayError::InvalidRecord);
            }
            match &job.outcome {
                JobOutcome::Ai {
                    semantic_output,
                    policy,
                    ..
                } => {
                    if policy.package != context.pins.content.package {
                        return Err(ReplayError::Stale);
                    }
                    count_text(&mut bytes, semantic_output, limits)?;
                }
                _ => return Err(ReplayError::InvalidRecord),
            }
        }
        Ok(Self {
            context,
            decisions,
            jobs,
        })
    }

    /// Returns the exact accepted record, including its committed draw counters.
    pub fn decision(
        &self,
        context: ReplayContext<'_>,
        operation: OperationId,
        revision: SessionRevision,
    ) -> Result<&'a AcceptedDecision, ReplayError> {
        if context != self.context {
            return Err(ReplayError::Stale);
        }
        let record = self
            .decisions
            .iter()
            .find(|record| record.operation == operation)
            .ok_or(ReplayError::Missing)?;
        if record.revision != revision {
            return Err(ReplayError::Stale);
        }
        if record.semantic_output.is_none() {
            return Err(ReplayError::MissingSemantic);
        }
        Ok(record)
    }

    /// Returns the exact native AI result; no job or provider work is dispatched.
    pub fn prepared_job(
        &self,
        context: ReplayContext<'_>,
        key: PreparedJobKey,
    ) -> Result<&'a JobCompletion, ReplayError> {
        if context != self.context {
            return Err(ReplayError::Stale);
        }
        let record = self
            .jobs
            .iter()
            .find(|record| record.job == key.job)
            .ok_or(ReplayError::Missing)?;
        if record.operation != key.operation || record.generation != key.generation {
            return Err(ReplayError::Stale);
        }
        Ok(record)
    }
}

fn validate_limits(limits: ReplayLimits) -> Result<(), ReplayError> {
    if limits.maximum_records == 0 || limits.maximum_semantic_bytes == 0 {
        return Err(ReplayError::Capacity);
    }
    Ok(())
}

fn repeated<T: PartialEq>(records: &[T]) -> bool {
    records
        .iter()
        .enumerate()
        .any(|(index, record)| records.iter().take(index).any(|prior| prior == record))
}

fn count_text(
    semantic_bytes: &mut usize,
    output: &str,
    limits: ReplayLimits,
) -> Result<(), ReplayError> {
    *semantic_bytes = semantic_bytes
        .checked_add(output.len())
        .ok_or(ReplayError::Capacity)?;
    if *semantic_bytes > limits.maximum_semantic_bytes {
        return Err(ReplayError::Capacity);
    }
    Ok(())
}

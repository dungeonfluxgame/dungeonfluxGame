//! Test-only output evidence records. These records do not authenticate agents,
//! inspect files/processes, approve tasks, or change the development queue.
//! The normal lifecycle owner supplies the actual assignment, capabilities,
//! current retained output custody and complete owned-handle observations.

use df_types::{BuildIdentity, RevisionLabel};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EvidenceScope<'a> {
    pub task: &'a RevisionLabel,
    pub attempt: &'a RevisionLabel,
    pub build: &'a BuildIdentity,
    pub assets: &'a RevisionLabel,
}

/// Current assignment and actual capabilities, supplied separately from claims.
#[derive(Clone, Copy)]
pub struct ReviewContext<'a> {
    pub scope: EvidenceScope<'a>,
    pub worker: &'a RevisionLabel,
    pub evaluator: &'a RevisionLabel,
    pub model: &'a RevisionLabel,
    pub computer_use: bool,
    pub vision: bool,
    pub audio: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationKind {
    NativeBoundary,
    BrowserInteractionAndVision,
    AudioPlayback,
}

#[derive(Clone, Copy)]
pub struct RequiredCriterion<'a> {
    pub id: &'a RevisionLabel,
    pub observation: ObservationKind,
}

/// Identifies bytes whose revision and retention were checked by the caller.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct OutputReference<'a> {
    pub id: &'a RevisionLabel,
    pub revision: &'a RevisionLabel,
    pub bytes: usize,
}

/// Actual observation/custody supplied by the trusted output collector.
#[derive(Clone, Copy)]
pub struct RetainedOutput<'a> {
    pub scope: EvidenceScope<'a>,
    pub observer: &'a RevisionLabel,
    pub observation: ObservationKind,
    pub output: OutputReference<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservedResult {
    Pass,
    Fail,
}

/// Supporting claims and gaps cannot be promoted to observed passes.
#[derive(Clone, Copy)]
pub enum CriterionState<'a> {
    Observed {
        result: ObservedResult,
        observed: &'a str,
        output: OutputReference<'a>,
    },
    Pending {
        reason: &'a str,
    },
    Unsupported {
        reason: &'a str,
    },
    Inconclusive {
        reason: &'a str,
    },
    Unperformed {
        reason: &'a str,
    },
    WorkerSupportingClaim {
        reference: &'a RevisionLabel,
    },
}

#[derive(Clone, Copy)]
pub struct CriterionEvidence<'a> {
    pub criterion: &'a RevisionLabel,
    pub procedure: &'a str,
    pub input: &'a str,
    pub expected: &'a str,
    pub state: CriterionState<'a>,
}

pub struct EvidenceReport<'a> {
    pub scope: EvidenceScope<'a>,
    pub observer: &'a RevisionLabel,
    pub model: &'a RevisionLabel,
    pub criteria: &'a [CriterionEvidence<'a>],
}

/// Scoped observations from the complete owned-process collector, not the report.
#[derive(Clone, Copy)]
pub struct OwnedHandle<'a> {
    pub scope: EvidenceScope<'a>,
    pub id: &'a RevisionLabel,
    pub owner: &'a RevisionLabel,
    pub terminal: bool,
}

/// Positive explicit limits; canonical labels also retain their 128-byte bound.
#[derive(Clone, Copy)]
pub struct EvidenceLimits {
    pub maximum_records: usize,
    pub maximum_text_bytes: usize,
}

/// Safe structural errors retain no supplied labels, paths or payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceError {
    Capacity,
    SelfReview,
    IdentityMismatch,
    InvalidCriterionMap,
    MissingDescription,
    MissingOutput,
    StaleOutput,
    MissingCapability,
    ForeignHandle,
    UnterminatedHandle,
    DuplicateHandle,
}

/// Counts only: even all-pass observations are not a task approval verdict.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EvidenceSummary {
    pub passed: usize,
    pub failed: usize,
    pub unobserved: usize,
}

impl EvidenceSummary {
    pub fn has_unobserved_criteria(self) -> bool {
        self.unobserved != 0
    }
}

/// Checks the named criterion map against separately supplied current observations.
/// This pure format does not establish that the collector/assignment is truthful;
/// the trusted lifecycle owner must obtain and retain those actual observations.
pub fn inspect_evidence(
    context: ReviewContext<'_>,
    required: &[RequiredCriterion<'_>],
    report: &EvidenceReport<'_>,
    outputs: &[RetainedOutput<'_>],
    handles: &[OwnedHandle<'_>],
    limits: EvidenceLimits,
) -> Result<EvidenceSummary, EvidenceError> {
    let total_record_count = required
        .len()
        .checked_add(report.criteria.len())
        .and_then(|count| count.checked_add(outputs.len()))
        .and_then(|count| count.checked_add(handles.len()))
        .ok_or(EvidenceError::Capacity)?;
    if limits.maximum_records == 0
        || limits.maximum_text_bytes == 0
        || required.is_empty()
        || total_record_count > limits.maximum_records
    {
        return Err(EvidenceError::Capacity);
    }
    if context.worker == context.evaluator {
        return Err(EvidenceError::SelfReview);
    }
    if report.scope != context.scope
        || report.observer != context.evaluator
        || report.model != context.model
    {
        return Err(EvidenceError::IdentityMismatch);
    }
    if required.len() != report.criteria.len() {
        return Err(EvidenceError::InvalidCriterionMap);
    }
    for (index, criterion) in required.iter().enumerate() {
        if required
            .iter()
            .take(index)
            .any(|prior| prior.id == criterion.id)
        {
            return Err(EvidenceError::InvalidCriterionMap);
        }
    }
    for (index, handle) in handles.iter().enumerate() {
        if handles
            .iter()
            .take(index)
            .any(|prior| prior.id == handle.id)
        {
            return Err(EvidenceError::DuplicateHandle);
        }
        if handle.owner != context.evaluator || handle.scope != context.scope {
            return Err(EvidenceError::ForeignHandle);
        }
        if !handle.terminal {
            return Err(EvidenceError::UnterminatedHandle);
        }
    }
    let mut total_text_bytes = 0usize;
    let mut summary = EvidenceSummary::default();
    for (index, evidence) in report.criteria.iter().enumerate() {
        if report
            .criteria
            .iter()
            .take(index)
            .any(|prior| prior.criterion == evidence.criterion)
        {
            return Err(EvidenceError::InvalidCriterionMap);
        }
        let criterion = required
            .iter()
            .find(|criterion| criterion.id == evidence.criterion)
            .ok_or(EvidenceError::InvalidCriterionMap)?;
        for text in [evidence.procedure, evidence.input, evidence.expected] {
            count_text(&mut total_text_bytes, text, limits.maximum_text_bytes)?;
        }
        match evidence.state {
            CriterionState::Observed {
                result,
                observed,
                output,
            } => {
                count_text(&mut total_text_bytes, observed, limits.maximum_text_bytes)?;
                let has_required_capability = match criterion.observation {
                    ObservationKind::NativeBoundary => true,
                    ObservationKind::BrowserInteractionAndVision => {
                        context.computer_use && context.vision
                    }
                    ObservationKind::AudioPlayback => context.audio,
                };
                if !has_required_capability {
                    return Err(EvidenceError::MissingCapability);
                }
                let mut matches = outputs
                    .iter()
                    .filter(|record| record.output.id == output.id);
                let retained = matches.next().ok_or(EvidenceError::MissingOutput)?;
                if matches.next().is_some()
                    || retained.output != output
                    || output.bytes == 0
                    || retained.scope != context.scope
                    || retained.observer != context.evaluator
                    || retained.observation != criterion.observation
                {
                    return Err(EvidenceError::StaleOutput);
                }
                match result {
                    ObservedResult::Pass => summary.passed += 1,
                    ObservedResult::Fail => summary.failed += 1,
                }
            }
            CriterionState::Pending { reason }
            | CriterionState::Unsupported { reason }
            | CriterionState::Inconclusive { reason }
            | CriterionState::Unperformed { reason } => {
                count_text(&mut total_text_bytes, reason, limits.maximum_text_bytes)?;
                summary.unobserved += 1;
            }
            CriterionState::WorkerSupportingClaim { reference } => {
                count_text(
                    &mut total_text_bytes,
                    reference.as_str(),
                    limits.maximum_text_bytes,
                )?;
                summary.unobserved += 1;
            }
        }
    }
    Ok(summary)
}

fn count_text(
    total_text_bytes: &mut usize,
    text: &str,
    maximum: usize,
) -> Result<(), EvidenceError> {
    *total_text_bytes = total_text_bytes
        .checked_add(text.len())
        .ok_or(EvidenceError::Capacity)?;
    if *total_text_bytes > maximum {
        return Err(EvidenceError::Capacity);
    }
    if text.trim().is_empty() {
        return Err(EvidenceError::MissingDescription);
    }
    Ok(())
}

use df_model::checkpoint::{
    Basis, Checkpoint, CheckpointError, CheckpointLimits, CheckpointPins, ContentPins,
    ContentReference, FactId, FactValue, GameFact, NarrativeState, ReferenceInventory,
};
use df_types::{OperationId, RevisionLabel, SessionRevision};

#[cfg(test)]
#[path = "thread_progress_tests.rs"]
mod tests;

/// Records and admitted references come from one validated owner snapshot.
pub(crate) struct ThreadProgressBasis<'a> {
    pub current: Basis,
    pub expected: Basis,
    pub content: &'a ContentPins,
    pub expected_content: &'a ContentPins,
    pub policy: &'a RevisionLabel,
    pub expected_policy: &'a RevisionLabel,
    pub narrative: &'a NarrativeState,
    pub admitted_content: &'a [ContentReference],
    pub facts: &'a [GameFact],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreadDisposition {
    Continue,
    Resolve,
}

/// The engine/content producer has already admitted this consequence and its
/// thread disposition. A ContentEvent or a fact ID alone never admits closure.
/// These private views are not authored predicates or persistence schemas.
pub(crate) struct AcceptedThreadConsequence<'a> {
    pub thread: &'a ContentReference,
    pub source: &'a GameFact,
    pub disposition: ThreadDisposition,
    pub ending_policy: Option<ContentReference>,
}

/// Explicit record, consequence and comparison budgets; no calibrated defaults.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgressLimits {
    pub records: usize,
    pub consequences: usize,
    pub work: usize,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ProgressError {
    WrongSession,
    WrongRun,
    StaleBasis,
    StaleContent,
    StalePolicy,
    Capacity,
    UnadmittedContent,
    DuplicateThread,
    DuplicateFact,
    MissingFact,
    FutureFact,
    InvalidChronology,
    SourceFactMismatch,
    ClosedThread,
    RepeatedConsequence,
    ConflictingConsequences,
}

/// Internal causal evidence; projection requires current audience authorization.
#[derive(Debug, Eq, PartialEq)]
pub struct ThreadProgressEvidence {
    pub thread: ContentReference,
    pub source: FactId,
    pub revision: SessionRevision,
    pub operation: OperationId,
    pub ordinal: u32,
    pub cause: Option<FactId>,
    pub disposition: ThreadDisposition,
    /// Explicit authored eligibility only; no campaign termination or publication authority.
    pub ending_policy: Option<ContentReference>,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ThreadProgressProposal {
    pub basis: Basis,
    pub content: ContentPins,
    pub policy: RevisionLabel,
    pub narrative: NarrativeState,
    pub evidence: Vec<ThreadProgressEvidence>,
}

/// An already admitted consuming-owner selection, not an authored closure rule.
/// Source selection requires this exact committed event definition and fact ID.
pub struct ThreadConsequenceSelection<'a> {
    pub thread: &'a ContentReference,
    pub event_definition: &'a ContentReference,
    pub source: FactId,
    pub disposition: ThreadDisposition,
}

/// Immutable source/policy admission for one bounded checkpoint proposal.
pub struct ThreadCheckpointRequest<'a> {
    pub expected_basis: Basis,
    pub admitted_pins: &'a CheckpointPins,
    pub policy: &'a RevisionLabel,
    pub expected_policy: &'a RevisionLabel,
    pub inventory: ReferenceInventory<'a>,
    pub checkpoint_limits: CheckpointLimits,
    pub selections: &'a [ThreadConsequenceSelection<'a>],
}

#[derive(Debug, Eq, PartialEq)]
pub enum CheckpointProgressError {
    Binding(CheckpointError),
    Progress(ProgressError),
    SourceDefinition,
    SourceAdmission,
    UnsupportedEndingPolicy,
    InvalidEndingDisposition,
    InvalidCandidate(CheckpointError),
}

#[derive(Debug, Eq, PartialEq)]
/// Detached server-side candidate and causal metadata; never an audience projection.
pub struct CheckpointThreadProgressProposal {
    pub checkpoint: Checkpoint,
    pub policy: RevisionLabel,
    pub evidence: Vec<ThreadProgressEvidence>,
}

/// Selects existing source-qualified committed events and stages their already
/// admitted dispositions through the canonical thread reducer. The detached
/// checkpoint preserves the exact basis/pins and every sibling state field.
/// This is a concrete engine-consumable proposal, never a durable apply operation.
pub fn stage_checkpoint_thread_progress(
    current: &Checkpoint,
    request: ThreadCheckpointRequest<'_>,
    limits: ProgressLimits,
) -> Result<CheckpointThreadProgressProposal, CheckpointProgressError> {
    stage_admitted_thread_progress(current, request, limits, None)
}

type ThreadSourceAdmission<'a> = dyn Fn(
        &Checkpoint,
        &RevisionLabel,
        &ThreadConsequenceSelection<'_>,
        &GameFact,
    ) -> Result<Option<ContentReference>, CheckpointProgressError>
    + 'a;

impl ThreadCheckpointRequest<'_> {
    /// Revalidates every canonical consequence with the immutable content owner.
    /// The owner performs a bounded pure lookup in its current content snapshot,
    /// admitting the exact thread/event/disposition and any ending policy.
    /// Missing or withdrawn required policy must refuse, never become ordinary closure.
    /// Eligibility remains detached evidence; session owns durable ending/publication.
    pub fn stage_source_admitted(
        self,
        current: &Checkpoint,
        limits: ProgressLimits,
        admit: impl Fn(
            &Checkpoint,
            &RevisionLabel,
            &ThreadConsequenceSelection<'_>,
            &GameFact,
        ) -> Result<Option<ContentReference>, CheckpointProgressError>,
    ) -> Result<CheckpointThreadProgressProposal, CheckpointProgressError> {
        stage_admitted_thread_progress(current, self, limits, Some(&admit))
    }
}

fn stage_admitted_thread_progress(
    current: &Checkpoint,
    request: ThreadCheckpointRequest<'_>,
    limits: ProgressLimits,
    admit: Option<&ThreadSourceAdmission<'_>>,
) -> Result<CheckpointThreadProgressProposal, CheckpointProgressError> {
    current
        .validate_resume(request.expected_basis, request.admitted_pins)
        .map_err(CheckpointProgressError::Binding)?;
    if request.policy != request.expected_policy {
        return Err(CheckpointProgressError::Progress(
            ProgressError::StalePolicy,
        ));
    }
    let bytes = current
        .retained_bytes()
        .ok_or(CheckpointProgressError::Progress(ProgressError::Capacity))?;
    if bytes > request.checkpoint_limits.maximum_retained_bytes
        || request.selections.len() > limits.consequences
        || current.state().facts.len() > limits.records
        || current.state().decisions.len() > limits.records
        || request.inventory.content.len() > limits.records
    {
        return Err(CheckpointProgressError::Progress(ProgressError::Capacity));
    }
    let basis = ThreadProgressBasis {
        current: current.basis(),
        expected: request.expected_basis,
        content: &current.pins().content,
        expected_content: &request.admitted_pins.content,
        policy: request.policy,
        expected_policy: request.expected_policy,
        narrative: &current.state().narrative,
        admitted_content: request.inventory.content,
        facts: &current.state().facts,
    };
    let mut work = Work {
        remaining: limits.work,
    };
    let mut consequences = Vec::with_capacity(request.selections.len());
    for selection in request.selections {
        content(&basis, selection.thread, &mut work).map_err(CheckpointProgressError::Progress)?;
        content(&basis, selection.event_definition, &mut work)
            .map_err(CheckpointProgressError::Progress)?;
        let source = fact(basis.facts, selection.source, &mut work)
            .map_err(CheckpointProgressError::Progress)?;
        let FactValue::ContentEvent { definition, .. } = &source.value else {
            return Err(CheckpointProgressError::SourceDefinition);
        };
        work.charge().map_err(CheckpointProgressError::Progress)?;
        if definition != selection.event_definition {
            return Err(CheckpointProgressError::SourceDefinition);
        }
        // Checkpoint structure validates decision references, but permits orphan facts.
        // Narrative may consume only an exact event retained by its accepted owner.
        let mut accepted = false;
        for decision in &current.state().decisions {
            work.charge().map_err(CheckpointProgressError::Progress)?;
            if decision.operation == source.operation && decision.revision == source.revision {
                work.charge().map_err(CheckpointProgressError::Progress)?;
                accepted = decision.facts.get(source.ordinal as usize) == Some(&source.id);
                break;
            }
        }
        if !accepted {
            return Err(CheckpointProgressError::Progress(
                ProgressError::SourceFactMismatch,
            ));
        }
        let ending_policy = if let Some(admit) = admit {
            work.charge().map_err(CheckpointProgressError::Progress)?;
            let policy = admit(current, request.policy, selection, source)?;
            if let Some(policy) = &policy {
                if selection.disposition != ThreadDisposition::Resolve {
                    return Err(CheckpointProgressError::InvalidEndingDisposition);
                }
                content(&basis, policy, &mut work).map_err(CheckpointProgressError::Progress)?;
            }
            policy
        } else {
            None
        };
        consequences.push(AcceptedThreadConsequence {
            thread: selection.thread,
            source,
            disposition: selection.disposition,
            ending_policy,
        });
    }
    let proposed = stage_thread_progress(
        basis,
        &consequences,
        ProgressLimits {
            work: work.remaining,
            ..limits
        },
    )
    .map_err(CheckpointProgressError::Progress)?;
    let mut state = current.state().clone();
    state.narrative = proposed.narrative;
    let mut pins = current.pins().clone();
    pins.content = proposed.content;
    let checkpoint = Checkpoint::new(
        current.schema(),
        proposed.basis,
        pins,
        state,
        request.inventory,
        request.checkpoint_limits,
    )
    .map_err(CheckpointProgressError::InvalidCandidate)?;
    Ok(CheckpointThreadProgressProposal {
        checkpoint,
        policy: proposed.policy,
        evidence: proposed.evidence,
    })
}

struct Work {
    remaining: usize,
}

impl Work {
    fn charge(&mut self) -> Result<(), ProgressError> {
        self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or(ProgressError::Capacity)?;
        Ok(())
    }
}

fn contains<T: PartialEq>(
    values: &[T],
    expected: &T,
    work: &mut Work,
) -> Result<bool, ProgressError> {
    for value in values {
        work.charge()?;
        if value == expected {
            return Ok(true);
        }
    }
    Ok(false)
}

fn fact<'a>(
    records: &'a [GameFact],
    expected: FactId,
    work: &mut Work,
) -> Result<&'a GameFact, ProgressError> {
    for record in records {
        work.charge()?;
        if record.id == expected {
            return Ok(record);
        }
    }
    Err(ProgressError::MissingFact)
}

fn content(
    basis: &ThreadProgressBasis<'_>,
    reference: &ContentReference,
    work: &mut Work,
) -> Result<(), ProgressError> {
    if reference.package != basis.content.package
        || !contains(basis.admitted_content, reference, work)?
    {
        return Err(ProgressError::UnadmittedContent);
    }
    Ok(())
}

/// Stages canonical thread retention using already accepted rules/world facts.
///
/// Progress records the source once without closing a thread. Only an explicitly
/// admitted Resolve removes that exact reference; all other open references and
/// all beat, budget and definition fields survive unchanged. No new thread,
/// branch, fact, knowledge grant or authored success is inferred.
///
/// The snapshot owner bounds/validates each complete GameFact payload before this
/// pass. Local bounds cover record counts and comparisons; inputs are not decoded
/// or source-qualified here. The engine must supply admitted dispositions and
/// merge compatible proposals; session must revalidate basis/content/policy and
/// atomically commit the selected state plus evidence/operation receipt. Replay
/// consumes retained records, and cancellation/rejection commits nothing.
pub(crate) fn stage_thread_progress(
    basis: ThreadProgressBasis<'_>,
    consequences: &[AcceptedThreadConsequence<'_>],
    limits: ProgressLimits,
) -> Result<ThreadProgressProposal, ProgressError> {
    if basis.current.session != basis.expected.session {
        return Err(ProgressError::WrongSession);
    }
    if basis.current.run != basis.expected.run {
        return Err(ProgressError::WrongRun);
    }
    if basis.current.revision != basis.expected.revision {
        return Err(ProgressError::StaleBasis);
    }
    if basis.content != basis.expected_content {
        return Err(ProgressError::StaleContent);
    }
    if basis.policy != basis.expected_policy {
        return Err(ProgressError::StalePolicy);
    }
    for count in [
        basis.facts.len(),
        basis.admitted_content.len(),
        basis.narrative.active_beats.len(),
        basis.narrative.completed_beats.len(),
        basis.narrative.open_threads.len(),
        basis.narrative.accepted_facts.len(),
    ] {
        if count > limits.records {
            return Err(ProgressError::Capacity);
        }
    }
    if consequences.len() > limits.consequences
        || basis
            .narrative
            .accepted_facts
            .len()
            .checked_add(consequences.len())
            .is_none_or(|count| count > limits.records)
    {
        return Err(ProgressError::Capacity);
    }
    let mut work = Work {
        remaining: limits.work,
    };
    content(&basis, &basis.narrative.definition, &mut work)?;
    for reference in basis
        .narrative
        .active_beats
        .iter()
        .chain(&basis.narrative.completed_beats)
    {
        content(&basis, reference, &mut work)?;
    }
    for (position, thread) in basis.narrative.open_threads.iter().enumerate() {
        content(&basis, thread, &mut work)?;
        if contains(&basis.narrative.open_threads[..position], thread, &mut work)? {
            return Err(ProgressError::DuplicateThread);
        }
    }
    // Match the model's canonical causal ordering: causes precede their effects,
    // revisions never decrease, and each operation has contiguous zero-based ordinals.
    for (position, record) in basis.facts.iter().enumerate() {
        work.charge()?;
        if record.revision > basis.current.revision {
            return Err(ProgressError::FutureFact);
        }
        let previous = &basis.facts[..position];
        if previous
            .last()
            .is_some_and(|last| last.revision > record.revision)
        {
            return Err(ProgressError::InvalidChronology);
        }
        let mut cause_seen = record.cause.is_none();
        let mut previous_ordinal = None;
        for earlier in previous {
            work.charge()?;
            if earlier.id == record.id {
                return Err(ProgressError::DuplicateFact);
            }
            if earlier.operation == record.operation {
                previous_ordinal = Some(earlier.ordinal);
            }
            cause_seen |= Some(earlier.id) == record.cause;
        }
        if !cause_seen {
            return Err(ProgressError::InvalidChronology);
        }
        let expected_ordinal = match previous_ordinal {
            Some(ordinal) => ordinal
                .checked_add(1)
                .ok_or(ProgressError::InvalidChronology)?,
            None => 0,
        };
        if record.ordinal != expected_ordinal {
            return Err(ProgressError::InvalidChronology);
        }
        if let FactValue::ContentEvent { definition, .. } = &record.value {
            content(&basis, definition, &mut work)?;
        }
    }
    for (position, id) in basis.narrative.accepted_facts.iter().enumerate() {
        fact(basis.facts, *id, &mut work)?;
        if contains(&basis.narrative.accepted_facts[..position], id, &mut work)? {
            return Err(ProgressError::DuplicateFact);
        }
    }
    // Validate the whole batch before cloning any candidate. A bad tail cannot
    // leave the first consequence partially applied.
    for (position, consequence) in consequences.iter().enumerate() {
        content(&basis, consequence.thread, &mut work)?;
        if !contains(&basis.narrative.open_threads, consequence.thread, &mut work)? {
            return Err(ProgressError::ClosedThread);
        }
        let canonical = fact(basis.facts, consequence.source.id, &mut work)?;
        work.charge()?;
        if canonical != consequence.source {
            return Err(ProgressError::SourceFactMismatch);
        }
        if contains(&basis.narrative.accepted_facts, &canonical.id, &mut work)? {
            return Err(ProgressError::RepeatedConsequence);
        }
        for earlier in consequences.iter().take(position) {
            work.charge()?;
            if earlier.source.id == canonical.id {
                return Err(ProgressError::RepeatedConsequence);
            }
            if earlier.thread == consequence.thread {
                return Err(ProgressError::ConflictingConsequences);
            }
        }
    }
    let mut narrative = basis.narrative.clone();
    let mut evidence = Vec::with_capacity(consequences.len());
    // Retain committed fact order rather than the producer's iteration order.
    for canonical in basis.facts {
        for consequence in consequences {
            work.charge()?;
            if canonical.id != consequence.source.id {
                continue;
            }
            narrative.accepted_facts.push(canonical.id);
            if consequence.disposition == ThreadDisposition::Resolve {
                let mut retained = Vec::with_capacity(narrative.open_threads.len());
                for thread in &narrative.open_threads {
                    work.charge()?;
                    if thread != consequence.thread {
                        retained.push(thread.clone());
                    }
                }
                narrative.open_threads = retained;
            }
            evidence.push(ThreadProgressEvidence {
                thread: consequence.thread.clone(),
                source: canonical.id,
                revision: canonical.revision,
                operation: canonical.operation,
                ordinal: canonical.ordinal,
                cause: canonical.cause,
                disposition: consequence.disposition,
                ending_policy: consequence.ending_policy.clone(),
            });
        }
    }
    Ok(ThreadProgressProposal {
        basis: basis.current,
        content: basis.content.clone(),
        policy: basis.policy.clone(),
        narrative,
        evidence,
    })
}

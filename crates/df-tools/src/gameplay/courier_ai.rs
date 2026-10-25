//! The installed run's private clue, admitted as a prepared-only recording.
//! Its exact bytes are digest-bound to the requesting operation; no provider fallback.
use df_ai::admission::{
    CompleteRecord, RecordEvent, RecordIdentity, RecordLimits, RecordingPublisher,
    admit_complete_record,
};
use df_ai::lookup::{PreparedRead, ReadResult, lookup_authorized_prepared};
use df_engine::effect_emission::{
    EffectEmissionLimits, EffectInspectionError, EffectRegistrationInspector,
    inspect_staged_effects,
};
use df_model::checkpoint::*;
use df_session::submission::RepositoryError;
use df_types::{LocaleTag, MemberId, OperationId, RevisionLabel, SessionRevision};
use sha2::{Digest, Sha256};

use super::{journey, model};

pub(super) const DEFINITION: &str = "courier-private-answer-prepared-1";
pub(super) const POLICY: &str = "courier-private-answer-policy-1";
pub(super) const MODEL: &str = "authored-courier-recording-1";
// Exact authored byte locale; this is not listener negotiation or translation.
const SOURCE_LOCALE: &str = "en";
pub(super) const RESPONSE: &str = "The courier quietly tells you: the sealed packet is addressed to Vell at the Harbor Inn. Keep its destination between you.";

fn response() -> String {
    super::pack::with_content(|pack| pack.courier_clue.clone())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CourierError {
    Unavailable,
    Source,
    Stale,
    Record,
    Capacity,
}

struct CourierRegistration<'a> {
    current: &'a Checkpoint,
    staged: &'a Checkpoint,
    source: &'a CommandInput,
}

impl EffectRegistrationInspector for CourierRegistration<'_> {
    type Refusal = CourierError;

    fn inspect(&self, intent: &DurableIntent) -> Result<(), Self::Refusal> {
        // The compiled execute/stage_completion pair shares this closed source binding.
        // Inspection neither reads the recording nor obtains a completion grant.
        let member = recipient(self.staged, intent)?;
        if member != self.source.member || journey::player_entity(member, self.current).is_err() {
            return Err(CourierError::Source);
        }
        Ok(())
    }
}

pub(super) fn inspect_candidate(
    current: &Checkpoint,
    staged: &Checkpoint,
    source: &CommandInput,
) -> Result<(), EffectInspectionError<CourierError>> {
    inspect_staged_effects(
        staged,
        current.basis(),
        current.pins(),
        source.operation,
        EffectEmissionLimits {
            maximum_scan_records: model::limits().maximum_records,
            maximum_effects: 1,
            // One effect costs at most four comparisons per canonical record.
            maximum_comparisons: 2048,
            maximum_retained_bytes: 8192,
        },
        &CourierRegistration {
            current,
            staged,
            source,
        },
    )?;
    Ok(())
}

fn id(domain: &[u8], basis: Basis, operation: OperationId) -> [u8; 16] {
    let mut hash = Sha256::new();
    hash.update(domain);
    hash.update(basis.session.as_bytes());
    hash.update(basis.run.as_bytes());
    hash.update(basis.revision.epoch().get().to_be_bytes());
    hash.update(basis.revision.sequence().to_be_bytes());
    hash.update(operation.as_bytes());
    let digest = hash.finalize();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes
}

pub(super) fn completion_operation(intent: &DurableIntent) -> Result<OperationId, CourierError> {
    OperationId::from_bytes(&id(b"courier-completion-1", intent.basis, intent.operation))
        .map_err(|_| CourierError::Source)
}

/// Exact expected native input for receipt lookup. Constructing it does not execute
/// the recording; the native engine executes only after the session's lookup gate.
pub(super) fn expected_completion(intent: &DurableIntent) -> Result<JobCompletion, CourierError> {
    Ok(JobCompletion {
        basis: intent.basis,
        operation: intent.operation,
        job: intent.job.ok_or(CourierError::Source)?,
        generation: intent.generation,
        outcome: JobOutcome::Ai {
            semantic_output: response(),
            policy: model::content(POLICY).map_err(|_| CourierError::Source)?,
            model: model::label(MODEL).map_err(|_| CourierError::Source)?,
        },
    })
}

pub(super) fn completion_fingerprint(completion: &JobCompletion) -> Result<[u8; 32], CourierError> {
    let JobOutcome::Ai {
        semantic_output,
        policy,
        model,
    } = &completion.outcome
    else {
        return Err(CourierError::Unavailable);
    };
    if semantic_output.len() > 512 {
        return Err(CourierError::Capacity);
    }
    let mut hash = Sha256::new();
    hash.update(b"local-courier-completion-v1");
    hash.update(completion.basis.session.as_bytes());
    hash.update(completion.basis.run.as_bytes());
    hash.update(completion.basis.revision.epoch().get().to_be_bytes());
    hash.update(completion.basis.revision.sequence().to_be_bytes());
    hash.update(completion.operation.as_bytes());
    hash.update(completion.job.as_bytes());
    hash.update(completion.generation.to_be_bytes());
    for field_value in [
        semantic_output.as_str(),
        policy.package.as_str(),
        policy.entry.as_str(),
        model.as_str(),
    ] {
        hash.update((field_value.len() as u64).to_be_bytes());
        hash.update(field_value.as_bytes());
    }
    Ok(hash.finalize().into())
}

/// Called inside the existing source transition, before its candidate is committed.
pub(super) fn append_request(
    candidate: Checkpoint,
    operation: OperationId,
) -> Result<Checkpoint, RepositoryError> {
    let mut state = candidate.state().clone();
    let intent = DurableIntent {
        id: EffectId::from_bytes(&id(b"courier-effect-1", candidate.basis(), operation))
            .map_err(|_| RepositoryError::InvalidCandidate)?,
        basis: candidate.basis(),
        operation,
        slot: 0,
        kind: EffectKind::RunAi,
        job: Some(
            JobId::from_bytes(&id(b"courier-job-1", candidate.basis(), operation))
                .map_err(|_| RepositoryError::InvalidCandidate)?,
        ),
        timer: None,
        generation: 1,
        status: DurableStatus::Pending,
        definition: model::content(DEFINITION)?,
    };
    let decision = state
        .decisions
        .iter_mut()
        .find(|decision| {
            decision.operation == operation && decision.revision == candidate.basis().revision
        })
        .ok_or(RepositoryError::InvalidCandidate)?;
    if !decision.effects.is_empty() {
        return Err(RepositoryError::InvalidCandidate);
    }
    decision.effects.push(intent.id);
    state.intents.push(intent);
    model::checkpoint(candidate.basis(), state)
}

/// Source and audience come from the committed reveal, never from recording text.
pub(super) fn recipient(
    current: &Checkpoint,
    intent: &DurableIntent,
) -> Result<MemberId, CourierError> {
    if current.state().mode != ExecutionMode::PreparedOnly {
        return Err(CourierError::Unavailable);
    }
    source_recipient(current, intent)
}

/// Source ownership qualification shared by the distinct bounded recording example;
/// this does not select an execution mode or execute/replay any content.
fn source_recipient(
    current: &Checkpoint,
    intent: &DurableIntent,
) -> Result<MemberId, CourierError> {
    current
        .validate_resume(
            current.basis(),
            &model::pins().map_err(|_| CourierError::Source)?,
        )
        .map_err(|_| CourierError::Stale)?;
    if intent.kind != EffectKind::RunAi
        || intent.definition != model::content(DEFINITION).map_err(|_| CourierError::Source)?
    {
        return Err(CourierError::Unavailable);
    }
    if intent.basis.session != current.basis().session
        || intent.basis.run != current.basis().run
        || intent.basis.revision.epoch() != current.basis().revision.epoch()
        || intent.basis.revision > current.basis().revision
        || intent.generation != 1
        || intent.slot != 0
        || intent.job.is_none()
        || intent.timer.is_some()
        || !current.state().intents.contains(intent)
    {
        return Err(CourierError::Stale);
    }
    let decision = current
        .state()
        .decisions
        .iter()
        .find(|decision| {
            decision.operation == intent.operation
                && decision.revision == intent.basis.revision
                && decision.source_policy.as_str() == journey::THREAD_POLICY
                && decision.effects == [intent.id]
        })
        .ok_or(CourierError::Source)?;
    let definition = model::content("private-courier-note").map_err(|_| CourierError::Source)?;
    let mut sources = current.state().facts.iter().filter(|fact| fact.operation == intent.operation && fact.revision == intent.basis.revision && decision.facts.contains(&fact.id) && matches!(&fact.value, FactValue::ContentEvent { definition: source, subjects } if *source == definition && subjects.is_empty()));
    let source = sources.next().ok_or(CourierError::Source)?;
    if sources.next().is_some() {
        return Err(CourierError::Source);
    }
    let AudienceScope::Members(members) = &source.audience else {
        return Err(CourierError::Source);
    };
    let [member] = members.as_slice() else {
        return Err(CourierError::Source);
    };
    if !current
        .state()
        .members
        .iter()
        .any(|link| link.member == *member && link.character.is_some())
    {
        return Err(CourierError::Source);
    }
    Ok(*member)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecordingBasis {
    pins: CheckpointPins,
    policy: ContentReference,
    model: RevisionLabel,
    source_locale: LocaleTag,
    current_basis: Basis,
    completion: JobCompletion,
    cause: AcceptedDecision,
    source: (
        FactId,
        SessionRevision,
        OperationId,
        AudienceScope,
        FactValue,
    ),
    listener: MemberId,
}
struct AuthoredRecording<'a> {
    current: &'a Checkpoint,
    intent: &'a DurableIntent,
    key: ContentReference,
    basis: RecordingBasis,
    response: String,
}
impl PreparedRead for AuthoredRecording<'_> {
    type Key = ContentReference;
    type Basis = RecordingBasis;
    type Artifact = String;
    type Failure = CourierError;
    fn read_prepared(
        &self,
        _: &Self::Key,
    ) -> ReadResult<'_, Self::Key, Self::Basis, Self::Artifact, Self::Failure> {
        Ok(Some((&self.key, &self.basis, &self.response)))
    }
    fn read_replay(
        &self,
        _: &Self::Key,
    ) -> ReadResult<'_, Self::Key, Self::Basis, Self::Artifact, Self::Failure> {
        Ok(None)
    }
}

fn recording_basis(
    current: &Checkpoint,
    intent: &DurableIntent,
) -> Result<RecordingBasis, CourierError> {
    let listener = source_recipient(current, intent)?;
    let cause = current
        .state()
        .decisions
        .iter()
        .find(|d| d.operation == intent.operation && d.revision == intent.basis.revision)
        .ok_or(CourierError::Source)?
        .clone();
    let source_definition =
        model::content("private-courier-note").map_err(|_| CourierError::Source)?;
    let source = current.state().facts.iter().find(|f| cause.facts.contains(&f.id) && f.operation == intent.operation && matches!(&f.value, FactValue::ContentEvent { definition, subjects } if definition == &source_definition && subjects.is_empty())).ok_or(CourierError::Source)?;
    Ok(RecordingBasis {
        pins: current.pins().clone(),
        policy: model::content(POLICY).map_err(|_| CourierError::Source)?,
        model: model::label(MODEL).map_err(|_| CourierError::Source)?,
        source_locale: LocaleTag::parse(SOURCE_LOCALE).map_err(|_| CourierError::Source)?,
        current_basis: current.basis(),
        completion: expected_completion(intent)?,
        cause,
        source: (
            source.id,
            source.revision,
            source.operation,
            source.audience.clone(),
            source.value.clone(),
        ),
        listener,
    })
}

impl df_ai::lookup::ReadAuthority for AuthoredRecording<'_> {
    type AuthorizationFailure = CourierError;
    fn authorize_prepared(
        &self,
        key: &ContentReference,
        basis: &RecordingBasis,
    ) -> Result<(), CourierError> {
        recipient(self.current, self.intent)?;
        if self.intent.status != DurableStatus::Pending {
            return Err(CourierError::Stale);
        }
        if key != &self.intent.definition || basis != &recording_basis(self.current, self.intent)? {
            return Err(CourierError::Stale);
        }
        Ok(())
    }
    fn authorize_replay(
        &self,
        _: &ContentReference,
        _: &RecordingBasis,
    ) -> Result<(), CourierError> {
        // This registered authored consumer has no recording-storage Replay owner.
        Err(CourierError::Unavailable)
    }
}

struct ResponseAdmission<'a> {
    current: &'a Checkpoint,
    intent: &'a DurableIntent,
}
impl RecordingPublisher for ResponseAdmission<'_> {
    type Key = JobId;
    type Basis = Basis;
    type Artifact = JobCompletion;
    type Error = CourierError;
    fn publish_complete(
        &mut self,
        record: CompleteRecord<JobId, Basis>,
    ) -> Result<JobCompletion, CourierError> {
        recipient(self.current, self.intent)?;
        if self.intent.status != DurableStatus::Pending || record.bytes() != response().as_bytes() {
            return Err(CourierError::Record);
        }
        // Transfer a complete candidate to the session owner; this is not reusable storage
        // or publication. The owner revalidates and commits before any view reads it.
        Ok(JobCompletion {
            basis: record.identity().basis,
            operation: record.identity().operation,
            job: record.identity().key,
            generation: self.intent.generation,
            outcome: JobOutcome::Ai {
                semantic_output: response(),
                policy: model::content(POLICY).map_err(|_| CourierError::Source)?,
                model: model::label(MODEL).map_err(|_| CourierError::Source)?,
            },
        })
    }
}

pub(super) fn execute(
    current: &Checkpoint,
    intent: &DurableIntent,
) -> Result<JobCompletion, CourierError> {
    recipient(current, intent)?;
    if intent.status != DurableStatus::Pending {
        return Err(CourierError::Stale);
    }
    let basis = recording_basis(current, intent)?;
    let recording = AuthoredRecording {
        current,
        intent,
        key: model::content(DEFINITION).map_err(|_| CourierError::Source)?,
        basis: basis.clone(),
        response: response(),
    };
    let text = lookup_authorized_prepared(&recording, &intent.definition, &basis)
        .map_err(|_| CourierError::Unavailable)?;
    let job = intent.job.ok_or(CourierError::Source)?;
    admit_record(
        current,
        intent,
        [
            Ok(RecordEvent::Chunk(text.as_bytes().to_vec())),
            Ok(RecordEvent::Complete {
                identity: RecordIdentity {
                    key: job,
                    basis: intent.basis,
                    operation: intent.operation,
                },
                byte_length: text.len() as u64,
                sha256: Sha256::digest(text.as_bytes()).into(),
            }),
        ],
    )
}

pub(super) fn admit_record(
    current: &Checkpoint,
    intent: &DurableIntent,
    events: impl IntoIterator<
        Item = Result<RecordEvent<JobId, Basis>, df_ai::admission::RecordInputError>,
    >,
) -> Result<JobCompletion, CourierError> {
    let expected = RecordIdentity {
        key: intent.job.ok_or(CourierError::Source)?,
        basis: intent.basis,
        operation: intent.operation,
    };
    admit_complete_record(
        &mut ResponseAdmission { current, intent },
        expected,
        RecordLimits::new(512, 512, 2).map_err(|_| CourierError::Capacity)?,
        events,
    )
    .map_err(|_| CourierError::Record)
}

/// A detached next-revision candidate. Nothing is applied when any final check fails.
pub(super) fn stage_completion(
    current: &Checkpoint,
    completion: &JobCompletion,
    operation: OperationId,
) -> Result<Checkpoint, CourierError> {
    let intent = current
        .state()
        .intents
        .iter()
        .find(|intent| intent.job == Some(completion.job))
        .ok_or(CourierError::Source)?;
    recipient(current, intent)?;
    if intent.status != DurableStatus::Pending
        || completion.basis != intent.basis
        || completion.operation != intent.operation
        || completion.generation != intent.generation
        || operation != completion_operation(intent)?
        || operation == intent.operation
    {
        return Err(CourierError::Stale);
    }
    let JobOutcome::Ai {
        semantic_output,
        policy,
        model: recording_model,
    } = &completion.outcome
    else {
        return Err(CourierError::Unavailable);
    };
    if semantic_output.as_str() != response()
        || *policy != model::content(POLICY).map_err(|_| CourierError::Source)?
        || *recording_model != model::label(MODEL).map_err(|_| CourierError::Source)?
    {
        return Err(CourierError::Source);
    }
    let mut basis = current.basis();
    basis.revision = basis
        .revision
        .next_sequence()
        .map_err(|_| CourierError::Capacity)?;
    let mut state = current.state().clone();
    let staged_intent = state
        .intents
        .iter_mut()
        .find(|record| record.id == intent.id)
        .ok_or(CourierError::Source)?;
    staged_intent.status = DurableStatus::Completed;
    state.decisions.push(AcceptedDecision {
        operation,
        revision: basis.revision,
        facts: vec![],
        draws: vec![],
        effects: vec![],
        source_policy: model::label(POLICY).map_err(|_| CourierError::Source)?,
        semantic_output: Some(semantic_output.clone()),
    });
    model::checkpoint(basis, state).map_err(|_| CourierError::Capacity)
}

pub(super) fn saved_response(
    current: &Checkpoint,
    member: MemberId,
) -> Result<Option<&str>, RepositoryError> {
    for intent in &current.state().intents {
        if intent.definition.entry.as_str() != DEFINITION
            || intent.status != DurableStatus::Completed
        {
            continue;
        }
        if recipient(current, intent).map_err(|_| RepositoryError::InvalidCandidate)? != member {
            continue;
        }
        let operation =
            completion_operation(intent).map_err(|_| RepositoryError::InvalidCandidate)?;
        let decision = current
            .state()
            .decisions
            .iter()
            .find(|decision| {
                decision.operation == operation && decision.source_policy.as_str() == POLICY
            })
            .ok_or(RepositoryError::InvalidCandidate)?;
        // Restored receipts must retain the causal terminal shape written by stage_completion.
        if decision.revision <= intent.basis.revision
            || decision.revision > current.basis().revision
            || !decision.facts.is_empty()
            || !decision.draws.is_empty()
            || !decision.effects.is_empty()
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        let text = decision
            .semantic_output
            .as_deref()
            .ok_or(RepositoryError::InvalidCandidate)?;
        if text != response() {
            return Err(RepositoryError::InvalidCandidate);
        }
        return Ok(Some(text));
    }
    Ok(None)
}

#[cfg(test)]
#[path = "courier_ai_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "courier_registration_tests.rs"]
mod registration_tests;

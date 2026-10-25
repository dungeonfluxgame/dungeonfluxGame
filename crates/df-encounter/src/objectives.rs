//! Pure authored transitions over existing canonical encounter objective references.

use df_model::checkpoint::{
    AcceptedDecision, Basis, Checkpoint, CheckpointError, CheckpointLimits, CheckpointPins,
    ContentReference, EncounterState, EntityId, FactValue, GameFact, RecordId, ReferenceInventory,
    RuleReference,
};

/// One already-authored mapping, not a success/failure enum or executable predicate.
///
/// The source owner must admit this exact mapping and its current actor/rights against
/// the checkpoint's complete content/rules/policy pins. The cause is a committed
/// canonical ContentEvent, never narration, intent or an elapsed-time assertion.
pub struct AuthoredObjectiveTransition<'a> {
    pub encounter: RecordId,
    pub objective_index: usize,
    pub expected: &'a ContentReference,
    pub replacement: &'a ContentReference,
    pub policy: &'a ContentReference,
    pub source: &'a RuleReference,
    pub actor: EntityId,
    pub cause: &'a GameFact,
}

/// Source qualification and current permissions remain separate from record presence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectivePolicyError {
    UnqualifiedSource,
    PolicyMismatch,
    RightsDenied,
    ActorDenied,
}

/// Trusted pure source admission, evaluated afresh for every requested transition.
///
/// Admission must check the authored expected-to-replacement mapping, exact cause
/// operation/revision/definition/subjects/audience, and current actor and source-use
/// rights against the full checkpoint pins. Inventory membership alone is insufficient.
/// Checks must be deterministic, bounded and perform no I/O or mutation. This port
/// defines no source catalog, persistent objective status or new game mechanic.
pub trait ObjectivePolicyOwner {
    fn admit_transition(
        &self,
        current: &Checkpoint,
        transition: &AuthoredObjectiveTransition<'_>,
    ) -> Result<(), ObjectivePolicyError>;

    /// Admit source-owned termination after every transition in the complete batch validates.
    ///
    /// The encounter is the unchanged canonical record. The source must review the complete
    /// replacement batch, accepted causes, current rights and rules timing before returning
    /// true. This clears only its turn order and active turn; objective references retain the
    /// authored success, failure or escape meaning. No reward or mechanical outcome is made.
    /// False preserves turns. Existing owners default to continuation; labels alone never end
    /// an encounter. The same deterministic, bounded, pure admission requirements apply.
    fn admit_end_state(
        &self,
        _current: &Checkpoint,
        _encounter: &EncounterState,
        _transitions: &[AuthoredObjectiveTransition<'_>],
    ) -> Result<bool, ObjectivePolicyError> {
        Ok(false)
    }
}

/// Explicit decision bounds; none has an unlimited default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectiveProposalLimits {
    pub maximum_changes: usize,
    pub maximum_records: usize,
    pub maximum_comparisons: usize,
    pub maximum_input_bytes: usize,
    pub maximum_output_bytes: usize,
}

pub struct ObjectiveProposalRequest<'a> {
    pub current: &'a Checkpoint,
    pub expected_basis: Basis,
    pub admitted_pins: &'a CheckpointPins,
    pub inventory: ReferenceInventory<'a>,
    pub transitions: &'a [AuthoredObjectiveTransition<'a>],
    pub checkpoint_limits: CheckpointLimits,
    pub limits: ObjectiveProposalLimits,
}

/// Safe refusals contain no source text, private identities or cause payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectiveProposalError {
    Snapshot(CheckpointError),
    Policy(ObjectivePolicyError),
    Capacity,
    UnadmittedContent,
    UnadmittedSource,
    UnavailableSource,
    UnknownEncounter,
    UnknownObjective,
    StaleObjective,
    UnchangedObjective,
    ConflictingChanges,
    ForeignCause,
    UnsupportedCause,
    UncommittedCause,
    ActorMismatch,
    InvalidCandidate(CheckpointError),
}

/// Borrowed exact source evidence. Audience and causal metadata stay on the GameFact.
pub struct ObjectiveChangeEvidence<'a> {
    pub encounter: RecordId,
    pub objective_index: usize,
    pub policy: &'a ContentReference,
    pub source: &'a RuleReference,
    pub cause: &'a GameFact,
    pub decision: &'a AcceptedDecision,
}

/// Detached candidate, never a durable commit or permission to disclose private evidence.
///
/// Its basis and pins are unchanged. Engine/session must compose the next accepted
/// decision, revalidate current source/actor/rights and atomically commit under its
/// ownership fence. Replay and operation deduplication remain that owner's work.
pub struct ObjectiveProposal<'a> {
    pub checkpoint: Checkpoint,
    pub evidence: Vec<ObjectiveChangeEvidence<'a>>,
}

struct Work {
    remaining: usize,
}

fn add_input_bytes(
    total: &mut usize,
    bytes: usize,
    maximum: usize,
) -> Result<(), ObjectiveProposalError> {
    *total = total
        .checked_add(bytes)
        .filter(|total| *total <= maximum)
        .ok_or(ObjectiveProposalError::Capacity)?;
    Ok(())
}

fn label_input_bytes(
    label: &df_types::RevisionLabel,
    total: &mut usize,
    request: &ObjectiveProposalRequest<'_>,
) -> Result<(), ObjectiveProposalError> {
    if label.as_str().len() > request.checkpoint_limits.maximum_text_bytes {
        return Err(ObjectiveProposalError::Capacity);
    }
    add_input_bytes(
        total,
        label.retained_heap_bytes(),
        request.limits.maximum_input_bytes,
    )
}

fn content_input_bytes(
    reference: &ContentReference,
    total: &mut usize,
    request: &ObjectiveProposalRequest<'_>,
) -> Result<(), ObjectiveProposalError> {
    add_input_bytes(
        total,
        size_of::<ContentReference>(),
        request.limits.maximum_input_bytes,
    )?;
    for label in [&reference.package, &reference.entry] {
        label_input_bytes(label, total, request)?;
    }
    Ok(())
}

fn rule_input_bytes(
    reference: &RuleReference,
    total: &mut usize,
    request: &ObjectiveProposalRequest<'_>,
) -> Result<(), ObjectiveProposalError> {
    add_input_bytes(
        total,
        size_of::<RuleReference>(),
        request.limits.maximum_input_bytes,
    )?;
    for label in [
        &reference.catalog,
        &reference.source,
        &reference.entry,
        &reference.clause,
    ] {
        label_input_bytes(label, total, request)?;
    }
    Ok(())
}

// Referenced records are conservatively counted for every declared view, including aliases.
// Admission covers complete caller-owned views before any trusted policy callback runs.
fn admit_request_bytes(
    request: &ObjectiveProposalRequest<'_>,
    mut total: usize,
) -> Result<(), ObjectiveProposalError> {
    for reference in request.inventory.content {
        content_input_bytes(reference, &mut total, request)?;
    }
    for source in request.inventory.rules {
        rule_input_bytes(source, &mut total, request)?;
    }
    for constraint in request.inventory.resources {
        add_input_bytes(
            &mut total,
            size_of::<df_model::checkpoint::ResourceConstraint>(),
            request.limits.maximum_input_bytes,
        )?;
        label_input_bytes(&constraint.resource, &mut total, request)?;
        rule_input_bytes(&constraint.source, &mut total, request)?;
    }
    for asset in request.inventory.assets {
        add_input_bytes(
            &mut total,
            size_of::<df_model::checkpoint::AssetReference>(),
            request.limits.maximum_input_bytes,
        )?;
        label_input_bytes(&asset.key, &mut total, request)?;
    }
    for transition in request.transitions {
        add_input_bytes(
            &mut total,
            size_of::<AuthoredObjectiveTransition<'_>>(),
            request.limits.maximum_input_bytes,
        )?;
        for reference in [
            transition.expected,
            transition.replacement,
            transition.policy,
        ] {
            content_input_bytes(reference, &mut total, request)?;
        }
        rule_input_bytes(transition.source, &mut total, request)?;
        let FactValue::ContentEvent {
            definition,
            subjects,
        } = &transition.cause.value
        else {
            return Err(ObjectiveProposalError::UnsupportedCause);
        };
        if subjects.len() > request.limits.maximum_records {
            return Err(ObjectiveProposalError::Capacity);
        }
        add_input_bytes(
            &mut total,
            size_of::<GameFact>(),
            request.limits.maximum_input_bytes,
        )?;
        let subject_bytes = subjects
            .capacity()
            .checked_mul(size_of::<EntityId>())
            .ok_or(ObjectiveProposalError::Capacity)?;
        add_input_bytes(
            &mut total,
            subject_bytes,
            request.limits.maximum_input_bytes,
        )?;
        content_input_bytes(definition, &mut total, request)?;
        if let df_model::checkpoint::AudienceScope::Members(members) = &transition.cause.audience {
            if members.len() > request.limits.maximum_records {
                return Err(ObjectiveProposalError::Capacity);
            }
            let audience_bytes = members
                .capacity()
                .checked_mul(size_of::<df_types::MemberId>())
                .ok_or(ObjectiveProposalError::Capacity)?;
            add_input_bytes(
                &mut total,
                audience_bytes,
                request.limits.maximum_input_bytes,
            )?;
        }
    }
    Ok(())
}

impl Work {
    fn charge(&mut self) -> Result<(), ObjectiveProposalError> {
        self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or(ObjectiveProposalError::Capacity)?;
        Ok(())
    }
}

fn find<'a, T>(
    records: &'a [T],
    work: &mut Work,
    predicate: impl Fn(&T) -> bool,
) -> Result<Option<&'a T>, ObjectiveProposalError> {
    for record in records {
        work.charge()?;
        if predicate(record) {
            return Ok(Some(record));
        }
    }
    Ok(None)
}

fn content(
    reference: &ContentReference,
    request: &ObjectiveProposalRequest<'_>,
    work: &mut Work,
) -> Result<(), ObjectiveProposalError> {
    if reference.package != request.admitted_pins.content.package
        || find(request.inventory.content, work, |admitted| {
            admitted == reference
        })?
        .is_none()
    {
        return Err(ObjectiveProposalError::UnadmittedContent);
    }
    Ok(())
}

/// Stage only explicit source-owner-admitted changes to existing objective references.
///
/// Every transition validates before the checkpoint is cloned. Exact committed facts
/// and their accepted decisions constrain causes; a valid inventory never substitutes
/// for authored policy/current rights admission. A failed tail returns no partial
/// result. Canonical encounter/objective ordering, all sibling state and every existing
/// audience are preserved. Source-admitted terminal encounters also clear their existing turns.
/// No new actor, reward, fact, draw or mechanical outcome is made.
pub fn propose_objective_transitions<'a>(
    owner: &impl ObjectivePolicyOwner,
    request: ObjectiveProposalRequest<'a>,
) -> Result<ObjectiveProposal<'a>, ObjectiveProposalError> {
    request
        .current
        .validate_resume(request.expected_basis, request.admitted_pins)
        .map_err(ObjectiveProposalError::Snapshot)?;
    let state = request.current.state();
    let input_bytes = request
        .current
        .retained_bytes()
        .ok_or(ObjectiveProposalError::Capacity)?;
    let counts = [
        state.encounters.len(),
        state.facts.len(),
        state.decisions.len(),
        state.continuity.recovery.unavailable_sources.len(),
        request.inventory.content.len(),
        request.inventory.rules.len(),
        request.inventory.resources.len(),
        request.inventory.assets.len(),
    ];
    if input_bytes > request.limits.maximum_input_bytes
        || request.transitions.len() > request.limits.maximum_changes
        || counts
            .into_iter()
            .any(|count| count > request.limits.maximum_records)
    {
        return Err(ObjectiveProposalError::Capacity);
    }
    admit_request_bytes(&request, input_bytes)?;
    let mut output_bytes = input_bytes
        .checked_add(size_of::<Vec<ObjectiveChangeEvidence<'a>>>())
        .and_then(|bytes| bytes.checked_add(size_of::<Vec<RecordId>>()))
        .and_then(|bytes| {
            request
                .transitions
                .len()
                .checked_mul(size_of::<ObjectiveChangeEvidence<'a>>() + size_of::<RecordId>())
                .and_then(|evidence| bytes.checked_add(evidence))
        })
        .ok_or(ObjectiveProposalError::Capacity)?;
    for transition in request.transitions {
        output_bytes = output_bytes
            .checked_add(transition.replacement.package.retained_heap_bytes())
            .and_then(|bytes| bytes.checked_add(transition.replacement.entry.retained_heap_bytes()))
            .ok_or(ObjectiveProposalError::Capacity)?;
    }
    if output_bytes > request.limits.maximum_output_bytes
        || output_bytes > request.checkpoint_limits.maximum_retained_bytes
    {
        return Err(ObjectiveProposalError::Capacity);
    }
    let mut work = Work {
        remaining: request.limits.maximum_comparisons,
    };
    let mut evidence = Vec::new();
    evidence
        .try_reserve_exact(request.transitions.len())
        .map_err(|_| ObjectiveProposalError::Capacity)?;
    for (position, transition) in request.transitions.iter().enumerate() {
        owner
            .admit_transition(request.current, transition)
            .map_err(ObjectiveProposalError::Policy)?;
        for reference in [
            transition.expected,
            transition.replacement,
            transition.policy,
        ] {
            content(reference, &request, &mut work)?;
        }
        if transition.source.catalog != request.admitted_pins.rules.catalog
            || find(request.inventory.rules, &mut work, |source| {
                source == transition.source
            })?
            .is_none()
        {
            return Err(ObjectiveProposalError::UnadmittedSource);
        }
        if find(
            &state.continuity.recovery.unavailable_sources,
            &mut work,
            |source| source == &transition.source.source,
        )?
        .is_some()
        {
            return Err(ObjectiveProposalError::UnavailableSource);
        }
        if transition.expected == transition.replacement {
            return Err(ObjectiveProposalError::UnchangedObjective);
        }
        for earlier in request.transitions.iter().take(position) {
            work.charge()?;
            if earlier.encounter == transition.encounter
                && earlier.objective_index == transition.objective_index
            {
                return Err(ObjectiveProposalError::ConflictingChanges);
            }
        }
        let encounter = find(&state.encounters, &mut work, |encounter| {
            encounter.id == transition.encounter
        })?
        .ok_or(ObjectiveProposalError::UnknownEncounter)?;
        if encounter.objectives.len() > request.limits.maximum_records
            || encounter.participants.len() > request.limits.maximum_records
            || encounter.turn_order.len() > request.limits.maximum_records
        {
            return Err(ObjectiveProposalError::Capacity);
        }
        let objective = encounter
            .objectives
            .get(transition.objective_index)
            .ok_or(ObjectiveProposalError::UnknownObjective)?;
        work.charge()?;
        if objective != transition.expected {
            return Err(ObjectiveProposalError::StaleObjective);
        }
        let cause = find(&state.facts, &mut work, |fact| {
            fact.id == transition.cause.id
        })?
        .ok_or(ObjectiveProposalError::ForeignCause)?;
        let FactValue::ContentEvent {
            definition,
            subjects,
        } = &cause.value
        else {
            return Err(ObjectiveProposalError::UnsupportedCause);
        };
        if subjects.len() > request.limits.maximum_records {
            return Err(ObjectiveProposalError::Capacity);
        }
        // Charge the full exact comparison before it can inspect nested event/audience data.
        let audience_count = match &cause.audience {
            df_model::checkpoint::AudienceScope::Members(members) => members.len(),
            _ => 0,
        };
        let comparison_work = subjects
            .len()
            .checked_add(audience_count)
            .and_then(|count| count.checked_add(1))
            .ok_or(ObjectiveProposalError::Capacity)?;
        work.remaining = work
            .remaining
            .checked_sub(comparison_work)
            .ok_or(ObjectiveProposalError::Capacity)?;
        if cause != transition.cause {
            return Err(ObjectiveProposalError::ForeignCause);
        }
        content(definition, &request, &mut work)?;
        if find(&encounter.participants, &mut work, |actor| {
            *actor == transition.actor
        })?
        .is_none()
            || find(subjects, &mut work, |actor| *actor == transition.actor)?.is_none()
        {
            return Err(ObjectiveProposalError::ActorMismatch);
        }
        let decision = find(&state.decisions, &mut work, |decision| {
            decision.operation == cause.operation && decision.revision == cause.revision
        })?
        .ok_or(ObjectiveProposalError::UncommittedCause)?;
        if decision.facts.len() > request.limits.maximum_records {
            return Err(ObjectiveProposalError::Capacity);
        }
        if find(&decision.facts, &mut work, |id| *id == cause.id)?.is_none() {
            return Err(ObjectiveProposalError::UncommittedCause);
        }
        evidence.push(ObjectiveChangeEvidence {
            encounter: encounter.id,
            objective_index: transition.objective_index,
            policy: transition.policy,
            source: transition.source,
            cause,
            decision,
        });
    }
    // Keep termination admission behind every exact mapping/cause check, once per affected
    // encounter. A denied final encounter cannot leak an earlier objective or turn change.
    let mut ended = Vec::new();
    ended
        .try_reserve_exact(request.transitions.len())
        .map_err(|_| ObjectiveProposalError::Capacity)?;
    for encounter in &state.encounters {
        if find(request.transitions, &mut work, |transition| {
            transition.encounter == encounter.id
        })?
        .is_none()
        {
            continue;
        }
        work.charge()?;
        if owner
            .admit_end_state(request.current, encounter, request.transitions)
            .map_err(ObjectiveProposalError::Policy)?
        {
            ended.push(encounter.id);
        }
    }
    let mut candidate = state.clone();
    for transition in request.transitions {
        let mut target = None;
        for encounter in &mut candidate.encounters {
            work.charge()?;
            if encounter.id == transition.encounter {
                target = Some(encounter);
                break;
            }
        }
        let objective = target
            .ok_or(ObjectiveProposalError::UnknownEncounter)?
            .objectives
            .get_mut(transition.objective_index)
            .ok_or(ObjectiveProposalError::UnknownObjective)?;
        *objective = transition.replacement.clone();
    }
    for encounter in &mut candidate.encounters {
        if find(&ended, &mut work, |id| *id == encounter.id)?.is_some() {
            encounter.turn_order.clear();
            encounter.active_turn = None;
        }
    }
    let checkpoint = Checkpoint::new(
        request.current.schema(),
        request.current.basis(),
        request.current.pins().clone(),
        candidate,
        request.inventory,
        request.checkpoint_limits,
    )
    .map_err(ObjectiveProposalError::InvalidCandidate)?;
    let retained = checkpoint
        .retained_bytes()
        .and_then(|bytes| {
            evidence
                .capacity()
                .checked_mul(size_of::<ObjectiveChangeEvidence<'a>>())
                .and_then(|heap| bytes.checked_add(heap))
                .and_then(|bytes| bytes.checked_add(size_of::<Vec<ObjectiveChangeEvidence<'a>>>()))
        })
        .ok_or(ObjectiveProposalError::Capacity)?;
    if retained > request.limits.maximum_output_bytes {
        return Err(ObjectiveProposalError::Capacity);
    }
    Ok(ObjectiveProposal {
        checkpoint,
        evidence,
    })
}

//! Bounded authored beat selection over one canonical owner snapshot.
use df_model::checkpoint::{
    AudienceScope, Basis, Checkpoint, CheckpointError, CheckpointLimits, CheckpointPins,
    ContentReference, FactId, FactValue, ReferenceInventory,
};
use df_types::{MemberId, RevisionLabel};

/// The content owner supplies exact existing prerequisites, never generated claims.
pub struct BeatCause<'a> {
    pub fact: FactId,
    pub event: &'a ContentReference,
    pub consumed_by_narrative: bool,
}

/// A source-admitted alternative; this transient view is not an authoring schema.
pub struct AdmittedBeatAlternative<'a> {
    pub selection: &'a ContentReference,
    pub from: &'a ContentReference,
    pub to: &'a ContentReference,
    pub causes: &'a [BeatCause<'a>],
    pub required_threads: &'a [ContentReference],
    pub opened_thread: Option<&'a ContentReference>,
}

pub struct CheckpointBeatRequest<'a> {
    pub expected_basis: Basis,
    pub admitted_pins: &'a CheckpointPins,
    pub policy: &'a RevisionLabel,
    pub expected_policy: &'a RevisionLabel,
    pub recipient: MemberId,
    pub selection: &'a ContentReference,
    pub alternatives: &'a [AdmittedBeatAlternative<'a>],
    pub inventory: ReferenceInventory<'a>,
    pub checkpoint_limits: CheckpointLimits,
}

/// Explicit local traversal limits, separate from canonical checkpoint byte bounds.
#[derive(Clone, Copy)]
pub struct BeatSelectionLimits {
    pub records: usize,
    pub alternatives: usize,
    pub work: usize,
}

#[derive(Debug, Eq, PartialEq)]
pub enum BeatSelectionError {
    Binding(CheckpointError),
    StalePolicy,
    Capacity,
    UnadmittedContent,
    Selection,
    Prerequisite,
    Source,
    Recipient,
    InvalidCandidate(CheckpointError),
}

struct Work(usize);
impl Work {
    fn charge(&mut self) -> Result<(), BeatSelectionError> {
        self.0 = self.0.checked_sub(1).ok_or(BeatSelectionError::Capacity)?;
        Ok(())
    }
    fn contains<T: PartialEq>(
        &mut self,
        values: &[T],
        expected: &T,
    ) -> Result<bool, BeatSelectionError> {
        for value in values {
            self.charge()?;
            if value == expected {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Selects an already authored alternative using accepted events and current access.
/// Only beat history and the explicitly admitted new thread may change. Selection
/// never emits facts, knowledge, draws, effects, mechanical rewards or durable receipts.
/// The registered engine/session owner must combine and commit its actual outcome.
pub fn stage_checkpoint_beat_selection(
    current: &Checkpoint,
    request: CheckpointBeatRequest<'_>,
    limits: BeatSelectionLimits,
) -> Result<Checkpoint, BeatSelectionError> {
    current
        .validate_resume(request.expected_basis, request.admitted_pins)
        .map_err(BeatSelectionError::Binding)?;
    if request.policy != request.expected_policy {
        return Err(BeatSelectionError::StalePolicy);
    }
    let state = current.state();
    if current
        .retained_bytes()
        .is_none_or(|bytes| bytes > request.checkpoint_limits.maximum_retained_bytes)
        || request.alternatives.len() > limits.alternatives
        || [
            state.facts.len(),
            state.decisions.len(),
            state.members.len(),
            state.characters.len(),
            state.narrative.active_beats.len(),
            state.narrative.completed_beats.len(),
            state.narrative.open_threads.len(),
            state.narrative.accepted_facts.len(),
            request.inventory.content.len(),
        ]
        .into_iter()
        .any(|count| count > limits.records)
    {
        return Err(BeatSelectionError::Capacity);
    }
    let mut work = Work(limits.work);
    let mut selected = None;
    for alternative in request.alternatives {
        work.charge()?;
        if alternative.selection == request.selection {
            if selected.is_some() {
                return Err(BeatSelectionError::Selection);
            }
            selected = Some(alternative);
        }
    }
    let selected = selected.ok_or(BeatSelectionError::Selection)?;
    if selected.causes.len() > limits.records || selected.required_threads.len() > limits.records {
        return Err(BeatSelectionError::Capacity);
    }
    for reference in [
        Some(request.selection),
        Some(selected.from),
        Some(selected.to),
        selected.opened_thread,
    ]
    .into_iter()
    .flatten()
    .chain(selected.required_threads.iter())
    .chain(selected.causes.iter().map(|cause| cause.event))
    {
        work.charge()?;
        if reference.package != current.pins().content.package
            || !work.contains(request.inventory.content, reference)?
        {
            return Err(BeatSelectionError::UnadmittedContent);
        }
    }
    if state.narrative.active_beats.as_slice() != [selected.from.clone()]
        || selected.from == selected.to
        || selected.causes.is_empty()
        || state
            .narrative
            .completed_beats
            .len()
            .checked_add(1)
            .is_none_or(|count| count > limits.records)
    {
        return Err(BeatSelectionError::Prerequisite);
    }
    if work.contains(&state.narrative.completed_beats, selected.from)?
        || work.contains(&state.narrative.completed_beats, selected.to)?
    {
        return Err(BeatSelectionError::Prerequisite);
    }
    let mut joined = false;
    for member in &state.members {
        work.charge()?;
        if member.member == request.recipient {
            for character in &state.characters {
                work.charge()?;
                if character.owner == member.member && Some(character.entity) == member.character {
                    joined = true;
                    break;
                }
            }
            break;
        }
    }
    if !joined {
        return Err(BeatSelectionError::Recipient);
    }
    for thread in selected.required_threads {
        if !work.contains(&state.narrative.open_threads, thread)? {
            return Err(BeatSelectionError::Prerequisite);
        }
    }
    if let Some(thread) = selected.opened_thread
        && work.contains(&state.narrative.open_threads, thread)?
    {
        return Err(BeatSelectionError::Prerequisite);
    }
    for (position, cause) in selected.causes.iter().enumerate() {
        for earlier in &selected.causes[..position] {
            work.charge()?;
            if earlier.fact == cause.fact {
                return Err(BeatSelectionError::Source);
            }
        }
        let mut source = None;
        for fact in &state.facts {
            work.charge()?;
            if fact.id == cause.fact {
                source = Some(fact);
                break;
            }
        }
        let source = source.ok_or(BeatSelectionError::Source)?;
        if !matches!(&source.value, FactValue::ContentEvent { definition, .. } if definition == cause.event)
        {
            return Err(BeatSelectionError::Source);
        }
        let mut accepted = false;
        for decision in &state.decisions {
            work.charge()?;
            if decision.operation == source.operation && decision.revision == source.revision {
                work.charge()?;
                accepted = decision.facts.get(source.ordinal as usize) == Some(&source.id);
                break;
            }
        }
        if !accepted
            || (cause.consumed_by_narrative
                && !work.contains(&state.narrative.accepted_facts, &source.id)?)
        {
            return Err(BeatSelectionError::Source);
        }
        // A saved knowledge grant cannot override revoked current source audience.
        let permitted = match &source.audience {
            AudienceScope::Shared => true,
            AudienceScope::Members(members) => work.contains(members, &request.recipient)?,
            AudienceScope::Host => false,
        };
        if !permitted {
            return Err(BeatSelectionError::Recipient);
        }
    }
    let mut state = state.clone();
    state
        .narrative
        .completed_beats
        .append(&mut state.narrative.active_beats);
    state.narrative.active_beats.push(selected.to.clone());
    if let Some(thread) = selected.opened_thread {
        state.narrative.open_threads.push(thread.clone());
    }
    Checkpoint::new(
        current.schema(),
        current.basis(),
        current.pins().clone(),
        state,
        request.inventory,
        request.checkpoint_limits,
    )
    .map_err(BeatSelectionError::InvalidCandidate)
}

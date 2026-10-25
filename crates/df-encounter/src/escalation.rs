//! Source-admitted escalation and reuse over an immutable canonical world.

use crate::challenge::{
    AuthoredChallengeReference, EncounterDirector, EncounterError, EncounterPlan, EncounterRequest,
    EncounterSourceOwner,
};
use df_model::checkpoint::{
    AudienceScope, Basis, Checkpoint, CheckpointError, CheckpointPins, ContentReference,
    EncounterState, EntityId, FactId, ReferenceInventory,
};
use df_types::RevisionLabel;
use std::mem::size_of;

/// Existing encounters carry no inferred lifecycle or completion flag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncounterPurpose<'a> {
    New,
    Escalate(&'a EncounterState),
    Reuse(&'a EncounterState),
}

/// The authored source supplies this expected current identity revision.
pub struct WorldPrerequisite<'a> {
    pub entity: EntityId,
    pub identity_revision: &'a RevisionLabel,
}

/// Caller order is the authored preference order, not a difficulty score.
pub struct EscalationCandidate<'a> {
    pub policy: &'a ContentReference,
    pub purpose: EncounterPurpose<'a>,
    pub challenge: EncounterRequest<'a>,
    pub world: &'a [WorldPrerequisite<'a>],
    pub committed_facts: &'a [FactId],
}

/// Lifecycle admission is evaluated from current canonical committed evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EscalationAdmission {
    Eligible,
    NotEligible,
    CompletedOneShot,
    Denied,
    Unavailable,
    Unsupported,
}

/// Trusted pure source admission; inventory membership cannot qualify a policy.
///
/// The owner must admit the exact policy, purpose, complete source/rights pins,
/// candidate order and current world/participant prerequisites. It must classify
/// one-shot completion from canonical committed records, never narration, turn
/// order, caller booleans or a default. Missing lifecycle evidence is Unavailable.
/// The current argument is authoritative, including during retained-plan validation.
/// Work is bounded by the admitted policy and does no I/O or state mutation.
pub trait EscalationSourceOwner: EncounterSourceOwner {
    fn admit_candidate(
        &self,
        current: &Checkpoint,
        candidate: &EscalationCandidate<'_>,
        order: usize,
    ) -> Result<EscalationAdmission, Self::Error>;
}

/// Explicit whole-selection bounds. Borrowed aliases are counted per declared view.
#[derive(Clone, Copy)]
pub struct EscalationLimits {
    pub maximum_candidates: usize,
    pub maximum_records: usize,
    pub maximum_comparisons: usize,
    pub maximum_input_bytes: usize,
    pub maximum_output_bytes: usize,
}

#[derive(Debug, Eq, PartialEq)]
pub enum EscalationError<OwnerError> {
    Snapshot(CheckpointError),
    Capacity,
    ShadowCheckpoint,
    UnadmittedPolicy,
    DuplicatePrerequisite,
    MissingWorldPrerequisite,
    StaleWorldPrerequisite,
    MissingFactPrerequisite,
    UncommittedFactPrerequisite,
    HiddenPrerequisite,
    UnknownEncounter,
    StaleEncounter,
    Policy(OwnerError),
    PolicyGap(EscalationAdmission),
    Challenge(EncounterError<OwnerError>),
}

/// A private provenance binding and an owned existing challenge proposal.
///
/// This is server-only staged data. It commits nothing, grants no disclosure,
/// spawns no entity/reward and changes no existing encounter. Engine/session own
/// accepted transitions, replay, deduplication and publication.
pub struct SelectedEncounter<'a> {
    candidate: &'a EscalationCandidate<'a>,
    order: usize,
    plan: EncounterPlan,
}

impl SelectedEncounter<'_> {
    pub fn plan(&self) -> &EncounterPlan {
        &self.plan
    }

    pub fn order(&self) -> usize {
        self.order
    }

    /// Recheck source, lifecycle, world, committed causes and participants before use.
    pub fn validate<Owner: EscalationSourceOwner>(
        &self,
        owner: &Owner,
        current: &Checkpoint,
        inventory: ReferenceInventory<'_>,
        limits: EscalationLimits,
    ) -> Result<(), EscalationError<Owner::Error>> {
        current
            .validate_resume(self.plan.basis, &self.plan.pins)
            .map_err(EscalationError::Snapshot)?;
        preflight(current, std::slice::from_ref(self.candidate), limits)?;
        let mut work = Work(limits.maximum_comparisons);
        prerequisites(current, self.candidate, &mut work)?;
        admit_policy(current, self.candidate, inventory, &mut work)?;
        require_eligible(
            owner
                .admit_candidate(current, self.candidate, self.order)
                .map_err(EscalationError::Policy)?,
        )?;
        EncounterDirector::validate(
            owner,
            current,
            &self.plan,
            self.plan.basis,
            &self.plan.pins,
            inventory,
            self.candidate.challenge.budget,
        )
        .map_err(EscalationError::Challenge)
    }
}

/// Select the first eligible authored candidate or return an ordinary no-intervention.
///
/// Every declared input view is bounded before trusted callbacks or output allocation.
/// Completed one-shots and missing source/world evidence refuse the whole result.
/// All plans pass the existing source and participant director boundary.
pub fn select_encounter<'a, Owner: EscalationSourceOwner>(
    owner: &Owner,
    current: &Checkpoint,
    expected_basis: Basis,
    admitted_pins: &CheckpointPins,
    candidates: &'a [EscalationCandidate<'a>],
    limits: EscalationLimits,
) -> Result<Option<SelectedEncounter<'a>>, EscalationError<Owner::Error>> {
    current
        .validate_resume(expected_basis, admitted_pins)
        .map_err(EscalationError::Snapshot)?;
    preflight(current, candidates, limits)?;
    let mut work = Work(limits.maximum_comparisons);
    for (order, candidate) in candidates.iter().enumerate() {
        if !std::ptr::eq(current, candidate.challenge.current) {
            return Err(EscalationError::ShadowCheckpoint);
        }
        candidate
            .challenge
            .current
            .validate_resume(
                candidate.challenge.expected_basis,
                candidate.challenge.admitted_pins,
            )
            .map_err(EscalationError::Snapshot)?;
        prerequisites(current, candidate, &mut work)?;
        admit_policy(current, candidate, candidate.challenge.inventory, &mut work)?;
        let admission = owner
            .admit_candidate(current, candidate, order)
            .map_err(EscalationError::Policy)?;
        if admission == EscalationAdmission::NotEligible {
            continue;
        }
        require_eligible(admission)?;
        let challenge = &candidate.challenge;
        let mut budget = challenge.budget;
        budget.max_output_bytes = budget.max_output_bytes.min(
            limits
                .maximum_output_bytes
                .checked_sub(size_of::<SelectedEncounter<'_>>())
                .ok_or(EscalationError::Capacity)?,
        );
        let plan = EncounterDirector::propose(
            owner,
            EncounterRequest {
                current,
                expected_basis,
                admitted_pins,
                inventory: challenge.inventory,
                mode: challenge.mode,
                definition: challenge.definition,
                participants: challenge.participants,
                objectives: challenge.objectives,
                escape_condition: challenge.escape_condition,
                consequences: challenge.consequences,
                budget,
            },
        )
        .map_err(EscalationError::Challenge)?;
        return Ok(Some(SelectedEncounter {
            candidate,
            order,
            plan,
        }));
    }
    Ok(None)
}

fn require_eligible<E>(admission: EscalationAdmission) -> Result<(), EscalationError<E>> {
    if admission != EscalationAdmission::Eligible {
        return Err(EscalationError::PolicyGap(admission));
    }
    Ok(())
}

struct Work(usize);
impl Work {
    fn charge<E>(&mut self) -> Result<(), EscalationError<E>> {
        self.0 = self.0.checked_sub(1).ok_or(EscalationError::Capacity)?;
        Ok(())
    }
}

fn find<'a, T, E>(
    records: &'a [T],
    work: &mut Work,
    matches: impl Fn(&T) -> bool,
) -> Result<Option<&'a T>, EscalationError<E>> {
    for record in records {
        work.charge()?;
        if matches(record) {
            return Ok(Some(record));
        }
    }
    Ok(None)
}

fn admit_policy<E>(
    current: &Checkpoint,
    candidate: &EscalationCandidate<'_>,
    inventory: ReferenceInventory<'_>,
    work: &mut Work,
) -> Result<(), EscalationError<E>> {
    if candidate.policy.package != current.pins().content.package
        || find(inventory.content, work, |reference| {
            reference == candidate.policy
        })?
        .is_none()
    {
        return Err(EscalationError::UnadmittedPolicy);
    }
    Ok(())
}

fn prerequisites<E>(
    current: &Checkpoint,
    candidate: &EscalationCandidate<'_>,
    work: &mut Work,
) -> Result<(), EscalationError<E>> {
    let state = current.state();
    for (index, required) in candidate.world.iter().enumerate() {
        for earlier in candidate.world.iter().take(index) {
            work.charge()?;
            if earlier.entity == required.entity {
                return Err(EscalationError::DuplicatePrerequisite);
            }
        }
        let entity = find(&state.entities, work, |entity| entity.id == required.entity)?
            .ok_or(EscalationError::MissingWorldPrerequisite)?;
        if &entity.identity_revision != required.identity_revision {
            return Err(EscalationError::StaleWorldPrerequisite);
        }
    }
    for (index, id) in candidate.committed_facts.iter().enumerate() {
        for earlier in candidate.committed_facts.iter().take(index) {
            work.charge()?;
            if earlier == id {
                return Err(EscalationError::DuplicatePrerequisite);
            }
        }
        let fact = find(&state.facts, work, |fact| &fact.id == id)?
            .ok_or(EscalationError::MissingFactPrerequisite)?;
        if fact.audience != AudienceScope::Shared {
            return Err(EscalationError::HiddenPrerequisite);
        }
        let decision = find(&state.decisions, work, |decision| {
            decision.operation == fact.operation && decision.revision == fact.revision
        })?
        .ok_or(EscalationError::UncommittedFactPrerequisite)?;
        if find(&decision.facts, work, |accepted| accepted == id)?.is_none() {
            return Err(EscalationError::UncommittedFactPrerequisite);
        }
    }
    if let EncounterPurpose::Escalate(expected) | EncounterPurpose::Reuse(expected) =
        candidate.purpose
    {
        let actual = find(&state.encounters, work, |encounter| {
            encounter.id == expected.id
        })?
        .ok_or(EscalationError::UnknownEncounter)?;
        if actual != expected
            || expected.definition != candidate.challenge.definition.definition
            || expected.participants != candidate.challenge.participants
            || expected.objectives.len() != candidate.challenge.objectives.len()
        {
            return Err(EscalationError::StaleEncounter);
        }
        for (objective, requested) in expected
            .objectives
            .iter()
            .zip(candidate.challenge.objectives)
        {
            work.charge()?;
            if objective != &requested.definition {
                return Err(EscalationError::StaleEncounter);
            }
        }
    }
    Ok(())
}

fn preflight<E>(
    current: &Checkpoint,
    candidates: &[EscalationCandidate<'_>],
    limits: EscalationLimits,
) -> Result<(), EscalationError<E>> {
    let state = current.state();
    if limits
        .maximum_candidates
        .checked_mul(size_of::<EscalationCandidate<'_>>())
        .is_none()
        || limits
            .maximum_records
            .checked_mul(size_of::<WorldPrerequisite<'_>>())
            .is_none()
        || candidates.len() > limits.maximum_candidates
        || [
            state.entities.len(),
            state.facts.len(),
            state.decisions.len(),
            state.encounters.len(),
        ]
        .into_iter()
        .any(|count| count > limits.maximum_records)
        || state
            .decisions
            .iter()
            .any(|decision| decision.facts.len() > limits.maximum_records)
        || limits.maximum_output_bytes < size_of::<SelectedEncounter<'_>>()
    {
        return Err(EscalationError::Capacity);
    }
    let mut bytes = current.retained_bytes().ok_or(EscalationError::Capacity)?;
    let mut add = |count: usize, size: usize| -> Result<(), EscalationError<E>> {
        bytes = count
            .checked_mul(size)
            .and_then(|amount| bytes.checked_add(amount))
            .filter(|total| *total <= limits.maximum_input_bytes)
            .ok_or(EscalationError::Capacity)?;
        Ok(())
    };
    add(candidates.len(), size_of::<EscalationCandidate<'_>>())?;
    for candidate in candidates {
        let challenge = &candidate.challenge;
        preflight_challenge(
            challenge,
            limits
                .maximum_output_bytes
                .checked_sub(size_of::<SelectedEncounter<'_>>())
                .ok_or(EscalationError::Capacity)?,
        )?;
        let counts = [
            candidate.world.len(),
            candidate.committed_facts.len(),
            challenge.participants.len(),
            challenge.objectives.len(),
            challenge.consequences.len(),
            challenge.inventory.content.len(),
            challenge.inventory.rules.len(),
            challenge.inventory.resources.len(),
            challenge.inventory.assets.len(),
        ];
        if counts
            .into_iter()
            .any(|count| count > limits.maximum_records)
        {
            return Err(EscalationError::Capacity);
        }
        if let EncounterPurpose::Escalate(encounter) | EncounterPurpose::Reuse(encounter) =
            candidate.purpose
        {
            if [
                encounter.participants.len(),
                encounter.turn_order.len(),
                encounter.objectives.len(),
            ]
            .into_iter()
            .any(|count| count > limits.maximum_records)
            {
                return Err(EscalationError::Capacity);
            }
            add(1, size_of::<EncounterState>() + 512)?;
            add(encounter.participants.len(), size_of::<EntityId>())?;
            add(encounter.turn_order.len(), size_of::<EntityId>())?;
            add(
                encounter.objectives.len(),
                size_of::<ContentReference>() + 256,
            )?;
        }
        // RevisionLabel constructors cap labels at 128 bytes. This conservatively
        // includes complete borrowed inventories and every declared source view.
        add(1, size_of::<ContentReference>() + 256)?;
        add(
            candidate.world.len(),
            size_of::<WorldPrerequisite<'_>>() + 128,
        )?;
        add(candidate.committed_facts.len(), size_of::<FactId>())?;
        add(challenge.participants.len(), size_of::<EntityId>())?;
        let references = challenge
            .objectives
            .len()
            .checked_add(challenge.consequences.len())
            .and_then(|count| {
                count.checked_add(1 + usize::from(challenge.escape_condition.is_some()))
            })
            .ok_or(EscalationError::Capacity)?;
        add(references, size_of::<AuthoredChallengeReference>() + 768)?;
        add(
            challenge.inventory.content.len(),
            size_of::<ContentReference>() + 256,
        )?;
        add(
            challenge.inventory.rules.len(),
            size_of::<df_model::checkpoint::RuleReference>() + 512,
        )?;
        add(
            challenge.inventory.resources.len(),
            size_of::<df_model::checkpoint::ResourceConstraint>() + 640,
        )?;
        add(
            challenge.inventory.assets.len(),
            size_of::<df_model::checkpoint::AssetReference>() + 128,
        )?;
        add(1, size_of::<CheckpointPins>() + 11 * 128)?;
    }
    Ok(())
}

fn preflight_challenge<E>(
    request: &EncounterRequest<'_>,
    output_limit: usize,
) -> Result<(), EscalationError<E>> {
    let budget = request.budget;
    if request.participants.len() > budget.max_participants
        || request.objectives.len() > budget.max_objectives
        || usize::from(request.escape_condition.is_some()) > budget.max_escape_references
        || request.consequences.len() > budget.max_consequence_references
    {
        return Err(EscalationError::Capacity);
    }
    let participants = request.participants.len();
    let duplicate_work = if participants < 2 {
        0
    } else {
        participants
            .checked_mul(participants - 1)
            .map(|count| count / 2)
            .ok_or(EscalationError::Capacity)?
    };
    let references = request
        .objectives
        .len()
        .checked_add(request.consequences.len())
        .and_then(|count| count.checked_add(1 + usize::from(request.escape_condition.is_some())))
        .ok_or(EscalationError::Capacity)?;
    let inventory = request
        .inventory
        .content
        .len()
        .checked_add(request.inventory.rules.len())
        .ok_or(EscalationError::Capacity)?;
    let work = references
        .checked_mul(inventory)
        .and_then(|count| count.checked_add(references))
        .and_then(|count| count.checked_add(inventory))
        .and_then(|count| count.checked_add(request.inventory.resources.len()))
        .and_then(|count| count.checked_add(request.inventory.assets.len()))
        .and_then(|count| count.checked_add(duplicate_work))
        .ok_or(EscalationError::Capacity)?;
    if work > budget.max_comparisons {
        return Err(EscalationError::Capacity);
    }
    // Conservative label bounds qualify child byte limits before the lifecycle callback.
    let mut input_bytes = size_of::<CheckpointPins>() + 11 * 128;
    let mut output_bytes = size_of::<EncounterPlan>() + 11 * 128;
    let mut add =
        |count: usize, size: usize, output_view: bool| -> Result<(), EscalationError<E>> {
            let bytes = count.checked_mul(size).ok_or(EscalationError::Capacity)?;
            input_bytes = input_bytes
                .checked_add(bytes)
                .ok_or(EscalationError::Capacity)?;
            if output_view {
                output_bytes = output_bytes
                    .checked_add(bytes)
                    .ok_or(EscalationError::Capacity)?;
            }
            Ok(())
        };
    add(participants, size_of::<EntityId>(), true)?;
    add(
        references,
        size_of::<AuthoredChallengeReference>() + 768,
        true,
    )?;
    add(
        request.inventory.content.len(),
        size_of::<ContentReference>() + 256,
        false,
    )?;
    add(
        request.inventory.rules.len(),
        size_of::<df_model::checkpoint::RuleReference>() + 512,
        false,
    )?;
    add(
        request.inventory.resources.len(),
        size_of::<df_model::checkpoint::ResourceConstraint>() + 640,
        false,
    )?;
    add(
        request.inventory.assets.len(),
        size_of::<df_model::checkpoint::AssetReference>() + 128,
        false,
    )?;
    if input_bytes > budget.max_input_bytes
        || output_bytes > budget.max_output_bytes.min(output_limit)
    {
        return Err(EscalationError::Capacity);
    }
    Ok(())
}

//! Bounded, source-addressed staged challenge proposals.

use crate::participants::{
    ParticipantError, ParticipantLimits, ParticipantOwner, validate_participants,
};
use df_model::checkpoint::{
    AssetReference, Basis, Checkpoint, CheckpointError, CheckpointPins, ContentReference, EntityId,
    ReferenceInventory, ResourceConstraint, RuleReference,
};
use std::mem::size_of;

/// Shared challenge intent; modes do not imply distinct mechanics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChallengeMode {
    Combat,
    Social,
    Chase,
    Hazard,
    Puzzle,
    Survival,
}

/// One exact authored content and rules source pair.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthoredChallengeReference {
    pub definition: ContentReference,
    pub source: RuleReference,
}

/// The source owner decides what the authored reference means.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChallengeReferenceKind {
    Definition,
    Objective,
    EscapeCondition,
    Consequence,
}

/// Results from current source admission. Non-admitted sources are explicit gaps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceAdmission {
    Admitted,
    Denied,
    Unavailable,
    Unsupported,
}

/// Identifies a rejected source slot without retaining source text or private data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChallengeSourceGap {
    pub purpose: ChallengeReferenceKind,
    pub index: usize,
    pub reason: SourceAdmission,
}

/// A source-addressed objective in caller order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncounterObjective {
    pub authored: AuthoredChallengeReference,
}

/// An optional source-addressed escape condition; no local clock or predicate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscapeCondition {
    pub authored: AuthoredChallengeReference,
}

/// A source-addressed consequence, not an executable effect or reward.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChallengeConsequence {
    pub authored: AuthoredChallengeReference,
}

/// Caller-owned limits. Every field is explicit and checked before source callbacks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChallengeBudget {
    pub max_participants: usize,
    pub max_objectives: usize,
    pub max_escape_references: usize,
    pub max_consequence_references: usize,
    pub max_comparisons: usize,
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
}

/// Borrowed proposal inputs. The request does not imply a committed cause or transition.
pub struct EncounterRequest<'a> {
    pub current: &'a Checkpoint,
    pub expected_basis: Basis,
    pub admitted_pins: &'a CheckpointPins,
    pub inventory: ReferenceInventory<'a>,
    pub mode: ChallengeMode,
    pub definition: &'a AuthoredChallengeReference,
    pub participants: &'a [EntityId],
    pub objectives: &'a [AuthoredChallengeReference],
    pub escape_condition: Option<&'a AuthoredChallengeReference>,
    pub consequences: &'a [AuthoredChallengeReference],
    pub budget: ChallengeBudget,
}

/// Source admission is current, pure and mode/purpose specific; it does not execute rules.
pub trait EncounterSourceOwner: ParticipantOwner<Actor = EntityId, Basis = Basis> {
    fn admit_reference(
        &self,
        current: &Checkpoint,
        mode: ChallengeMode,
        purpose: ChallengeReferenceKind,
        reference: &AuthoredChallengeReference,
    ) -> Result<SourceAdmission, Self::Error>;
}

/// Safe refusal classes; a source gap is a refusal, never a successful partial plan.
#[derive(Debug, Eq, PartialEq)]
pub enum EncounterError<OwnerError> {
    Checkpoint(CheckpointError),
    Capacity,
    Participant(ParticipantError<OwnerError>),
    UnadmittedContent,
    UnadmittedSource,
    SourceGap(ChallengeSourceGap),
    Owner(OwnerError),
    PlanContainsSourceGaps,
}

/// Owned candidate only. It cannot commit state or execute its source references.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncounterPlan {
    pub basis: Basis,
    pub pins: CheckpointPins,
    pub mode: ChallengeMode,
    pub definition: AuthoredChallengeReference,
    pub participants: Vec<EntityId>,
    pub objectives: Vec<EncounterObjective>,
    pub escape_condition: Option<EscapeCondition>,
    pub consequences: Vec<ChallengeConsequence>,
    /// Accepted plans have no gaps; gaps are returned as typed refusals.
    pub source_gaps: Vec<ChallengeSourceGap>,
}

/// Proposes and revalidates bounded staged challenges without mutating the checkpoint.
pub struct EncounterDirector;

impl EncounterDirector {
    pub fn propose<Owner: EncounterSourceOwner>(
        owner: &Owner,
        request: EncounterRequest<'_>,
    ) -> Result<EncounterPlan, EncounterError<Owner::Error>> {
        let reference_count = reference_count(
            request.objectives.len(),
            request.escape_condition.is_some(),
            request.consequences.len(),
        )?;
        validate_counts_and_work(
            request.participants.len(),
            request.objectives.len(),
            request.escape_condition.is_some(),
            request.consequences.len(),
            &request.inventory,
            reference_count,
            request.budget,
        )?;
        let (input_bytes, output_bytes) = proposal_bytes(&request)?;
        require_bytes(input_bytes, output_bytes, request.budget)?;

        request
            .current
            .validate_resume(request.expected_basis, request.admitted_pins)
            .map_err(EncounterError::Checkpoint)?;

        let current_basis = request.current.basis();
        validate_participants(
            owner,
            &request.expected_basis,
            &current_basis,
            request.participants,
            ParticipantLimits {
                max_participants: request.budget.max_participants,
                max_identity_comparisons: request.budget.max_comparisons,
            },
        )
        .map_err(EncounterError::Participant)?;

        validate_inventory(&request.inventory, request_references(&request))?;
        admit_references(
            owner,
            request.current,
            request.mode,
            request_references(&request),
        )?;

        Ok(EncounterPlan {
            basis: current_basis,
            pins: request.current.pins().clone(),
            mode: request.mode,
            definition: request.definition.clone(),
            participants: request.participants.to_vec(),
            objectives: request
                .objectives
                .iter()
                .cloned()
                .map(|authored| EncounterObjective { authored })
                .collect(),
            escape_condition: request
                .escape_condition
                .cloned()
                .map(|authored| EscapeCondition { authored }),
            consequences: request
                .consequences
                .iter()
                .cloned()
                .map(|authored| ChallengeConsequence { authored })
                .collect(),
            source_gaps: Vec::new(),
        })
    }

    pub fn validate<Owner: EncounterSourceOwner>(
        owner: &Owner,
        current: &Checkpoint,
        plan: &EncounterPlan,
        expected_basis: Basis,
        admitted_pins: &CheckpointPins,
        inventory: ReferenceInventory<'_>,
        budget: ChallengeBudget,
    ) -> Result<(), EncounterError<Owner::Error>> {
        let reference_count = reference_count(
            plan.objectives.len(),
            plan.escape_condition.is_some(),
            plan.consequences.len(),
        )?;
        validate_counts_and_work(
            plan.participants.len(),
            plan.objectives.len(),
            plan.escape_condition.is_some(),
            plan.consequences.len(),
            &inventory,
            reference_count,
            budget,
        )?;
        if !plan.source_gaps.is_empty() {
            return Err(EncounterError::PlanContainsSourceGaps);
        }
        let (input_bytes, _) = plan_bytes(plan, &inventory)?;
        require_bytes(input_bytes, 0, budget)?;

        current
            .validate_resume(expected_basis, admitted_pins)
            .map_err(EncounterError::Checkpoint)?;
        if plan.basis != expected_basis {
            return Err(EncounterError::Checkpoint(CheckpointError::StaleBasis));
        }
        if plan.pins.rules != admitted_pins.rules {
            return Err(EncounterError::Checkpoint(CheckpointError::RulesMismatch));
        }
        if plan.pins.content != admitted_pins.content {
            return Err(EncounterError::Checkpoint(CheckpointError::ContentMismatch));
        }
        if plan.pins.build != admitted_pins.build {
            return Err(EncounterError::Checkpoint(CheckpointError::BuildMismatch));
        }
        let current_basis = current.basis();
        validate_participants(
            owner,
            &plan.basis,
            &current_basis,
            &plan.participants,
            ParticipantLimits {
                max_participants: budget.max_participants,
                max_identity_comparisons: budget.max_comparisons,
            },
        )
        .map_err(EncounterError::Participant)?;

        validate_inventory(&inventory, plan_references(plan))?;
        admit_references(owner, current, plan.mode, plan_references(plan))
    }
}

struct ReferenceUse<'a> {
    purpose: ChallengeReferenceKind,
    index: usize,
    reference: &'a AuthoredChallengeReference,
}

fn reference_count<OwnerError>(
    objectives: usize,
    has_escape: bool,
    consequences: usize,
) -> Result<usize, EncounterError<OwnerError>> {
    objectives
        .checked_add(if has_escape { 1 } else { 0 })
        .and_then(|count| count.checked_add(consequences))
        .and_then(|count| count.checked_add(1))
        .ok_or(EncounterError::Capacity)
}

fn validate_counts_and_work<OwnerError>(
    participants: usize,
    objectives: usize,
    has_escape: bool,
    consequences: usize,
    inventory: &ReferenceInventory<'_>,
    references: usize,
    budget: ChallengeBudget,
) -> Result<(), EncounterError<OwnerError>> {
    if participants > budget.max_participants
        || objectives > budget.max_objectives
        || (if has_escape { 1 } else { 0 }) > budget.max_escape_references
        || consequences > budget.max_consequence_references
    {
        return Err(EncounterError::Capacity);
    }
    let participant_comparisons =
        duplicate_comparisons(participants).ok_or(EncounterError::Capacity)?;
    let inventory_work = inventory
        .content
        .len()
        .checked_add(inventory.rules.len())
        .and_then(|count| count.checked_add(inventory.resources.len()))
        .and_then(|count| count.checked_add(inventory.assets.len()))
        .ok_or(EncounterError::Capacity)?;
    let source_inventory_count = inventory
        .content
        .len()
        .checked_add(inventory.rules.len())
        .ok_or(EncounterError::Capacity)?;
    let inventory_comparisons = references
        .checked_mul(source_inventory_count)
        .ok_or(EncounterError::Capacity)?;
    let total_comparisons = participant_comparisons
        .checked_add(inventory_comparisons)
        .and_then(|count| count.checked_add(references))
        .and_then(|count| count.checked_add(inventory_work))
        .ok_or(EncounterError::Capacity)?;
    if total_comparisons > budget.max_comparisons {
        return Err(EncounterError::Capacity);
    }
    Ok(())
}

fn duplicate_comparisons(count: usize) -> Option<usize> {
    if count < 2 {
        return Some(0);
    }
    if count.is_multiple_of(2) {
        (count / 2).checked_mul(count - 1)
    } else {
        count.checked_mul((count - 1) / 2)
    }
}

fn proposal_bytes<OwnerError>(
    request: &EncounterRequest<'_>,
) -> Result<(usize, usize), EncounterError<OwnerError>> {
    let mut input = size_of::<CheckpointPins>()
        .checked_add(MAX_PIN_LABEL_BYTES)
        .and_then(|size| {
            size.checked_add(
                request
                    .participants
                    .len()
                    .checked_mul(size_of::<EntityId>())?,
            )
        })
        .ok_or(EncounterError::Capacity)?;
    for usage in request_references(request) {
        add_reference_bytes(&mut input, usage.reference)?;
    }
    for reference in request.inventory.content {
        add_content_bytes(&mut input, reference)?;
    }
    for source in request.inventory.rules {
        add_rule_bytes(&mut input, source)?;
    }
    add_inventory_bytes(&mut input, &request.inventory)?;

    let mut output = size_of::<EncounterPlan>()
        .checked_add(MAX_PIN_LABEL_BYTES)
        .and_then(|size| {
            size.checked_add(
                request
                    .participants
                    .len()
                    .checked_mul(size_of::<EntityId>())?,
            )
        })
        .and_then(|size| {
            size.checked_add(
                request
                    .objectives
                    .len()
                    .checked_mul(size_of::<EncounterObjective>())?,
            )
        })
        .and_then(|size| {
            size.checked_add(
                request
                    .consequences
                    .len()
                    .checked_mul(size_of::<ChallengeConsequence>())?,
            )
        })
        .and_then(|size| {
            size.checked_add(
                (if request.escape_condition.is_some() {
                    1_usize
                } else {
                    0_usize
                })
                .checked_mul(size_of::<EscapeCondition>())?,
            )
        })
        .ok_or(EncounterError::Capacity)?;
    for usage in request_references(request) {
        add_reference_bytes(&mut output, usage.reference)?;
    }
    Ok((input, output))
}

fn plan_bytes<OwnerError>(
    plan: &EncounterPlan,
    inventory: &ReferenceInventory<'_>,
) -> Result<(usize, usize), EncounterError<OwnerError>> {
    let mut input = size_of::<EncounterPlan>()
        .checked_add(MAX_PIN_LABEL_BYTES)
        .and_then(|size| {
            size.checked_add(plan.participants.len().checked_mul(size_of::<EntityId>())?)
        })
        .and_then(|size| {
            size.checked_add(
                plan.objectives
                    .len()
                    .checked_mul(size_of::<EncounterObjective>())?,
            )
        })
        .and_then(|size| {
            size.checked_add(
                plan.consequences
                    .len()
                    .checked_mul(size_of::<ChallengeConsequence>())?,
            )
        })
        .and_then(|size| {
            size.checked_add(
                (if plan.escape_condition.is_some() {
                    1_usize
                } else {
                    0_usize
                })
                .checked_mul(size_of::<EscapeCondition>())?,
            )
        })
        .ok_or(EncounterError::Capacity)?;
    for usage in plan_references(plan) {
        add_reference_bytes(&mut input, usage.reference)?;
    }
    for reference in inventory.content {
        add_content_bytes(&mut input, reference)?;
    }
    for source in inventory.rules {
        add_rule_bytes(&mut input, source)?;
    }
    add_inventory_bytes(&mut input, inventory)?;
    Ok((input, 0))
}

fn require_bytes<OwnerError>(
    input: usize,
    output: usize,
    budget: ChallengeBudget,
) -> Result<(), EncounterError<OwnerError>> {
    if input > budget.max_input_bytes || output > budget.max_output_bytes {
        return Err(EncounterError::Capacity);
    }
    Ok(())
}

/// Revision labels are individually capped at 128 bytes; checkpoint pins retain eleven.
const MAX_PIN_LABEL_BYTES: usize = 11 * 128;

fn add_inventory_bytes<OwnerError>(
    total: &mut usize,
    inventory: &ReferenceInventory<'_>,
) -> Result<(), EncounterError<OwnerError>> {
    for resource in inventory.resources {
        add_resource_bytes(total, resource)?;
    }
    for asset in inventory.assets {
        add_asset_bytes(total, asset)?;
    }
    Ok(())
}

fn add_resource_bytes<OwnerError>(
    total: &mut usize,
    resource: &ResourceConstraint,
) -> Result<(), EncounterError<OwnerError>> {
    add_bytes(total, size_of::<ResourceConstraint>())?;
    add_label_bytes(total, resource.resource.retained_heap_bytes())?;
    add_rule_bytes(total, &resource.source)
}

fn add_asset_bytes<OwnerError>(
    total: &mut usize,
    asset: &AssetReference,
) -> Result<(), EncounterError<OwnerError>> {
    add_bytes(total, size_of::<AssetReference>())?;
    add_label_bytes(total, asset.key.retained_heap_bytes())
}

fn add_reference_bytes<OwnerError>(
    total: &mut usize,
    reference: &AuthoredChallengeReference,
) -> Result<(), EncounterError<OwnerError>> {
    add_content_bytes(total, &reference.definition)?;
    add_rule_bytes(total, &reference.source)
}

fn add_content_bytes<OwnerError>(
    total: &mut usize,
    reference: &ContentReference,
) -> Result<(), EncounterError<OwnerError>> {
    add_bytes(total, size_of::<ContentReference>())?;
    add_label_bytes(total, reference.package.retained_heap_bytes())?;
    add_label_bytes(total, reference.entry.retained_heap_bytes())
}

fn add_rule_bytes<OwnerError>(
    total: &mut usize,
    reference: &RuleReference,
) -> Result<(), EncounterError<OwnerError>> {
    add_bytes(total, size_of::<RuleReference>())?;
    for label in [
        &reference.catalog,
        &reference.source,
        &reference.entry,
        &reference.clause,
    ] {
        add_label_bytes(total, label.retained_heap_bytes())?;
    }
    Ok(())
}

fn add_label_bytes<OwnerError>(
    total: &mut usize,
    bytes: usize,
) -> Result<(), EncounterError<OwnerError>> {
    add_bytes(total, bytes)
}

fn add_bytes<OwnerError>(
    total: &mut usize,
    bytes: usize,
) -> Result<(), EncounterError<OwnerError>> {
    *total = total.checked_add(bytes).ok_or(EncounterError::Capacity)?;
    Ok(())
}

fn request_references<'a>(
    request: &'a EncounterRequest<'_>,
) -> impl Iterator<Item = ReferenceUse<'a>> + 'a {
    std::iter::once(ReferenceUse {
        purpose: ChallengeReferenceKind::Definition,
        index: 0,
        reference: request.definition,
    })
    .chain(
        request
            .objectives
            .iter()
            .enumerate()
            .map(|(index, reference)| ReferenceUse {
                purpose: ChallengeReferenceKind::Objective,
                index,
                reference,
            }),
    )
    .chain(
        request
            .escape_condition
            .iter()
            .map(|reference| ReferenceUse {
                purpose: ChallengeReferenceKind::EscapeCondition,
                index: 0,
                reference,
            }),
    )
    .chain(
        request
            .consequences
            .iter()
            .enumerate()
            .map(|(index, reference)| ReferenceUse {
                purpose: ChallengeReferenceKind::Consequence,
                index,
                reference,
            }),
    )
}

fn plan_references<'a>(plan: &'a EncounterPlan) -> impl Iterator<Item = ReferenceUse<'a>> + 'a {
    std::iter::once(ReferenceUse {
        purpose: ChallengeReferenceKind::Definition,
        index: 0,
        reference: &plan.definition,
    })
    .chain(
        plan.objectives
            .iter()
            .enumerate()
            .map(|(index, objective)| ReferenceUse {
                purpose: ChallengeReferenceKind::Objective,
                index,
                reference: &objective.authored,
            }),
    )
    .chain(plan.escape_condition.iter().map(|escape| ReferenceUse {
        purpose: ChallengeReferenceKind::EscapeCondition,
        index: 0,
        reference: &escape.authored,
    }))
    .chain(
        plan.consequences
            .iter()
            .enumerate()
            .map(|(index, consequence)| ReferenceUse {
                purpose: ChallengeReferenceKind::Consequence,
                index,
                reference: &consequence.authored,
            }),
    )
}

fn validate_inventory<'a, OwnerError>(
    inventory: &ReferenceInventory<'_>,
    references: impl IntoIterator<Item = ReferenceUse<'a>>,
) -> Result<(), EncounterError<OwnerError>> {
    for usage in references {
        if !inventory.content.contains(&usage.reference.definition) {
            return Err(EncounterError::UnadmittedContent);
        }
        if !inventory.rules.contains(&usage.reference.source) {
            return Err(EncounterError::UnadmittedSource);
        }
    }
    Ok(())
}

fn admit_references<'a, Owner: EncounterSourceOwner>(
    owner: &Owner,
    current: &Checkpoint,
    mode: ChallengeMode,
    references: impl IntoIterator<Item = ReferenceUse<'a>>,
) -> Result<(), EncounterError<Owner::Error>> {
    for usage in references {
        let admission = owner
            .admit_reference(current, mode, usage.purpose, usage.reference)
            .map_err(EncounterError::Owner)?;
        if admission != SourceAdmission::Admitted {
            return Err(EncounterError::SourceGap(ChallengeSourceGap {
                purpose: usage.purpose,
                index: usage.index,
                reason: admission,
            }));
        }
    }
    Ok(())
}

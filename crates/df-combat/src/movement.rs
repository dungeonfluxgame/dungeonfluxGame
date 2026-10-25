//! Bounded selection from current, rules-admitted movement paths.
//! The concrete rules path/query contract and its production adapter remain owner gates.

use df_model::checkpoint::{
    Basis, CheckpointPins, ContentReference, EncounterState, EntityId, Position, RecordId,
    RuleReference, WorldEntity,
};

const MAX_CANDIDATES: usize = 256;
const MAX_OBJECTIVES: usize = 32;

/// Immutable canonical context supplied by the engine's current candidate decision.
pub struct MovementContext<'a> {
    pub basis: Basis,
    pub pins: &'a CheckpointPins,
    pub actor: &'a WorldEntity,
    pub encounter: &'a EncounterState,
    pub policy: &'a ContentReference,
    /// Already-admitted policy preference order, highest preference first.
    pub preferred_objectives: &'a [ContentReference],
}

/// A caller-owned path and its canonical offer identity; no path representation is invented.
pub struct SuppliedPath<'a, Path> {
    pub offer: RecordId,
    pub path: &'a Path,
}

/// Canonical geometry and source facts supplied only after current rules admission.
#[derive(Debug, Eq, PartialEq)]
pub struct MovementFacts<'a> {
    pub actor: EntityId,
    pub origin_location: EntityId,
    pub origin_position: Position,
    pub destination: EntityId,
    pub destination_position: Position,
    pub objective: &'a ContentReference,
    pub source: &'a RuleReference,
}

/// Substitution boundary for the missing canonical reachable-result adapter.
///
/// Implementations must be pure and deterministic. `validate_context` checks current
/// session/run, relevant geometry, actor/turn/resources/effects/pending timing,
/// rules/content/source pins, authored policy admission, and perception scope. It
/// must permit unrelated session revisions when the relevant context still agrees.
/// `admit_path` accepts only an exact currently supplied rules offer, verifies its
/// canonical offer ID, path and geometry (including destination existence), source
/// qualification and perception-safe objective attribution. It returns a typed
/// stale/unsupported/ruling/illegal error when admission cannot be established.
/// Neither a client legality flag nor a fixture implementation establishes authority.
/// `point_count` reports bounded structural size only; it assigns no distance or cost.
/// Owner errors must retain safe classifications without undisclosed geometry/payloads.
pub trait MovementOwner {
    type Path;
    type Error;

    fn point_count(&self, path: &Self::Path) -> usize;

    fn validate_context(&self, context: &MovementContext<'_>) -> Result<(), Self::Error>;

    fn admit_path<'a>(
        &self,
        context: &MovementContext<'_>,
        candidate: &SuppliedPath<'a, Self::Path>,
    ) -> Result<MovementFacts<'a>, Self::Error>;
}

/// Explicit caller-admitted work bounds, all finite; no unlimited default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MovementLimits {
    pub candidates: usize,
    pub objectives: usize,
    pub points_per_path: usize,
    pub total_points: usize,
    pub identity_comparisons: usize,
}

/// Refusals contain positions/classes, never private geometry or source payloads.
#[derive(Debug, Eq, PartialEq)]
pub enum MovementError<OwnerError> {
    InvalidLimits,
    CandidateCapacity,
    ObjectiveCapacity,
    PathCapacity { index: usize },
    TotalPointCapacity,
    ComparisonCapacity,
    DuplicateOffer { first: usize, duplicate: usize },
    DuplicatePreference { first: usize, duplicate: usize },
    UnknownPreference { index: usize },
    PolicyMismatch,
    ActorNotActive,
    MissingOrigin,
    OriginMismatch { index: usize },
    UnknownObjective { index: usize },
    SourceMismatch { index: usize },
    Owner(OwnerError),
}

/// An exact borrowed path staged for later engine/rules revalidation and session commit.
/// This output neither spends movement nor applies a world-state change.
#[derive(Debug, Eq, PartialEq)]
pub struct SelectedMovement<'a, Path> {
    pub basis: Basis,
    pub pins: &'a CheckpointPins,
    pub policy: &'a ContentReference,
    pub encounter: RecordId,
    pub offer: RecordId,
    pub path: &'a Path,
    pub facts: MovementFacts<'a>,
    pub objective_preference: usize,
}

/// Absence is successful policy decline; admission failures remain typed errors.
/// Successful selection owns one fixed-size envelope; its path and context stay borrowed.
#[derive(Debug, Eq, PartialEq)]
pub enum MovementSelection<'a, Path> {
    Selected(Box<SelectedMovement<'a, Path>>),
    NoPreferredPath,
}

/// Select by admitted objective preference, then canonical offer-ID order.
///
/// Capacity is checked before context/path admission. Every supplied candidate is
/// admitted before successful output, even when another candidate ranks higher;
/// no rejected path can be silently hidden by an earlier selection. Returned path
/// bytes are exactly caller-owned bytes, with no path search, distance, cost,
/// visibility or resource calculation. One fixed-size selection envelope is boxed
/// after all candidates pass admission; refusal and decline allocate nothing here.
/// Point/candidate/objective caps bound work and retained borrowed input.
/// The native caller owns its execution deadline,
/// cancellation, safe diagnostic emission and revalidation before acceptance.
pub fn select_movement<'a, Owner: MovementOwner>(
    owner: &Owner,
    context: &MovementContext<'a>,
    candidates: &[SuppliedPath<'a, Owner::Path>],
    limits: MovementLimits,
) -> Result<MovementSelection<'a, Owner::Path>, MovementError<Owner::Error>> {
    if limits.candidates == 0
        || limits.candidates > MAX_CANDIDATES
        || limits.objectives == 0
        || limits.objectives > MAX_OBJECTIVES
        || limits.points_per_path == 0
        || limits.total_points < limits.points_per_path
    {
        return Err(MovementError::InvalidLimits);
    }
    if candidates.len() > limits.candidates {
        return Err(MovementError::CandidateCapacity);
    }
    if context.preferred_objectives.len() > limits.objectives
        || context.encounter.objectives.len() > limits.objectives
    {
        return Err(MovementError::ObjectiveCapacity);
    }
    // The hard input caps make this product safe on every supported usize width.
    let comparison_count = candidates.len() * candidates.len().saturating_sub(1) / 2;
    if comparison_count > limits.identity_comparisons {
        return Err(MovementError::ComparisonCapacity);
    }
    let mut total_path_points = 0_usize;
    for (index, candidate) in candidates.iter().enumerate() {
        let path_point_count = owner.point_count(candidate.path);
        if path_point_count > limits.points_per_path {
            return Err(MovementError::PathCapacity { index });
        }
        total_path_points = total_path_points
            .checked_add(path_point_count)
            .ok_or(MovementError::TotalPointCapacity)?;
        if total_path_points > limits.total_points {
            return Err(MovementError::TotalPointCapacity);
        }
        for (first, earlier) in candidates.iter().take(index).enumerate() {
            if earlier.offer == candidate.offer {
                return Err(MovementError::DuplicateOffer {
                    first,
                    duplicate: index,
                });
            }
        }
    }
    owner
        .validate_context(context)
        .map_err(MovementError::Owner)?;
    if context.encounter.active_turn != Some(context.actor.id) {
        return Err(MovementError::ActorNotActive);
    }
    let (Some(origin_location), Some(origin_position)) =
        (context.actor.location, context.actor.position)
    else {
        return Err(MovementError::MissingOrigin);
    };
    if context.policy != &context.encounter.combat_policy
        || context.policy.package != context.pins.content.package
    {
        return Err(MovementError::PolicyMismatch);
    }
    for (duplicate, objective) in context.preferred_objectives.iter().enumerate() {
        if !context.encounter.objectives.contains(objective) {
            return Err(MovementError::UnknownPreference { index: duplicate });
        }
        for (first, earlier) in context
            .preferred_objectives
            .iter()
            .take(duplicate)
            .enumerate()
        {
            if earlier == objective {
                return Err(MovementError::DuplicatePreference { first, duplicate });
            }
        }
    }
    let mut selected: Option<SelectedMovement<'a, Owner::Path>> = None;
    for (index, candidate) in candidates.iter().enumerate() {
        let facts = owner
            .admit_path(context, candidate)
            .map_err(MovementError::Owner)?;
        if facts.actor != context.actor.id
            || facts.origin_location != origin_location
            || facts.origin_position != origin_position
        {
            return Err(MovementError::OriginMismatch { index });
        }
        if !context.encounter.objectives.contains(facts.objective) {
            return Err(MovementError::UnknownObjective { index });
        }
        if facts.source.catalog != context.pins.rules.catalog {
            return Err(MovementError::SourceMismatch { index });
        }
        let Some(preference) = context
            .preferred_objectives
            .iter()
            .position(|objective| objective == facts.objective)
        else {
            continue;
        };
        if selected.as_ref().is_none_or(|current| {
            (preference, candidate.offer) < (current.objective_preference, current.offer)
        }) {
            selected = Some(SelectedMovement {
                basis: context.basis,
                pins: context.pins,
                policy: context.policy,
                encounter: context.encounter.id,
                offer: candidate.offer,
                path: candidate.path,
                facts,
                objective_preference: preference,
            });
        }
    }
    Ok(match selected {
        Some(selected) => MovementSelection::Selected(Box::new(selected)),
        None => MovementSelection::NoPreferredPath,
    })
}

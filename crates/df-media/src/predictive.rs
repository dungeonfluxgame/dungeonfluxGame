//! Bounded ordering of already committed, audience-filtered media demand.
//!
//! The caller owns source and rights approval, complete asset publication, spend admission,
//! provider dispatch and durable job state. Equality here only fences obsolete queued work.

use df_model::checkpoint::{
    AssetDemand, AudienceScope, Basis, ContentDigest, DemandPriority, LogicalTime, PrefetchPolicy,
    RecordId,
};
use df_types::OperationId;

use crate::schedule::{
    DispatchOutcome, DispatchRefusal, MediaDispatch, MediaPriority, MediaRequest, MediaScheduler,
    ScheduleError, ScheduleLimits, ScheduleSnapshot,
};

const MAXIMUM_INPUT_CANDIDATES: usize = 64;

/// Caller-approved current provenance. Possession of these values grants no asset access.
pub struct PredictiveContext<'a, RightsBasis> {
    pub basis: Basis,
    pub source: ContentDigest,
    pub audience: &'a AudienceScope,
    pub rights_basis: &'a RightsBasis,
    pub now: LogicalTime,
}

/// Ordinal policy labels, not calibrated probabilities or a supplier cost quote.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PredictiveHeuristic {
    pub likelihood: u8,
    pub importance: u8,
    pub urgency: u8,
    pub reusable: bool,
    pub estimated_cost: u64,
}

/// Ephemeral scheduling input over the existing canonical AssetDemand record.
/// `prerequisites` are demand IDs; immutable asset-reference edges remain in df-model.
pub struct PredictiveCandidate<RightsBasis> {
    pub demand: AssetDemand,
    pub operation: OperationId,
    pub branch: RecordId,
    pub prerequisites: Vec<RecordId>,
    pub rights_basis: RightsBasis,
    pub heuristic: PredictiveHeuristic,
    pub command: Box<[u8]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PredictiveError {
    InvalidPolicy,
    Capacity,
    InvalidHeuristic,
    InvalidTimebase,
    Expired,
    Stale,
    Duplicate,
    MissingDependency,
    DependencyCycle,
    InvalidReady,
    Schedule(ScheduleError),
}

/// The original queued bytes are returned when a current-fence or expiry check retires work.
pub struct PredictiveBegin {
    pub dispatch: Option<MediaDispatch<RecordId>>,
    pub retired: Vec<MediaRequest<RecordId>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PredictiveSnapshot {
    pub selected_items: usize,
    pub selected_output_bytes: u64,
    pub deferred_items: usize,
    pub schedule: ScheduleSnapshot,
}

/// One owned speculative plan. It never starts a provider or marks an asset Ready.
/// A new plan must be built for new committed demand; existing dispatch tokens retain
/// their scheduler ownership and must receive an explicit terminal outcome.
pub struct PredictivePlan<RightsBasis> {
    candidates: Vec<Planned<RightsBasis>>,
    scheduler: MediaScheduler<RecordId>,
    admitted_now: LogicalTime,
    selected_output_bytes: u64,
    deferred_items: usize,
}

struct Planned<RightsBasis> {
    demand: AssetDemand,
    prerequisites: Vec<RecordId>,
    rights_basis: RightsBasis,
}

impl<RightsBasis: Eq> PredictivePlan<RightsBasis> {
    /// Validate a finite dependency DAG, then select a bounded topological rank prefix.
    /// The caller supplies committed, authorized and audience-filtered demands and policy.
    /// A refusal consumes and drops the unadmitted command bytes without external effects.
    pub fn new(
        current: PredictiveContext<'_, RightsBasis>,
        policy: &PrefetchPolicy,
        candidates: Vec<PredictiveCandidate<RightsBasis>>,
        schedule: ScheduleLimits,
    ) -> Result<Self, PredictiveError> {
        if policy.maximum_candidates == 0
            || policy.maximum_branches == 0
            || policy.maximum_bytes == 0
            || policy.maximum_duration_ticks == 0
        {
            return Err(PredictiveError::InvalidPolicy);
        }
        if candidates.len() > MAXIMUM_INPUT_CANDIDATES {
            return Err(PredictiveError::Capacity);
        }
        for (index, candidate) in candidates.iter().enumerate() {
            if [
                candidate.heuristic.likelihood,
                candidate.heuristic.importance,
                candidate.heuristic.urgency,
            ]
            .into_iter()
            .any(|rank| rank > 100)
            {
                return Err(PredictiveError::InvalidHeuristic);
            }
            if candidates
                .iter()
                .take(index)
                .any(|prior| prior.demand.id == candidate.demand.id)
            {
                return Err(PredictiveError::Duplicate);
            }
            if !same_provenance(&candidate.demand, &candidate.rights_basis, &current) {
                return Err(PredictiveError::Stale);
            }
            if !same_timebase(candidate.demand.expires, current.now) {
                return Err(PredictiveError::InvalidTimebase);
            }
            if current.now.ticks >= candidate.demand.expires.ticks {
                return Err(PredictiveError::Expired);
            }
            if candidate.demand.expires.ticks - current.now.ticks > policy.maximum_duration_ticks {
                return Err(PredictiveError::Capacity);
            }
        }
        validate_dependencies(&candidates)?;

        let candidate_count = candidates.len();
        let mut candidate_order: Vec<usize> = (0..candidate_count).collect();
        candidate_order
            .sort_by(|left, right| compare_candidates(&candidates[*left], &candidates[*right]));
        let mut selected_demand_ids = Vec::new();
        let mut selected_branches = Vec::new();
        let mut selected_order = Vec::new();
        let mut selected_output_bytes = 0u64;
        while selected_order.len()
            < usize::try_from(policy.maximum_candidates).unwrap_or(usize::MAX)
        {
            let next = candidate_order.iter().copied().find(|index| {
                let candidate = &candidates[*index];
                !selected_demand_ids.contains(&candidate.demand.id)
                    && candidate
                        .prerequisites
                        .iter()
                        .all(|dependency| selected_demand_ids.contains(dependency))
                    && (selected_branches.contains(&candidate.branch)
                        || selected_branches.len() < policy.maximum_branches as usize)
                    && selected_output_bytes
                        .checked_add(candidate.demand.maximum_bytes)
                        .is_some_and(|bytes| bytes <= policy.maximum_bytes)
            });
            let Some(index) = next else { break };
            let candidate = &candidates[index];
            selected_output_bytes += candidate.demand.maximum_bytes;
            if !selected_branches.contains(&candidate.branch) {
                selected_branches.push(candidate.branch);
            }
            selected_demand_ids.push(candidate.demand.id);
            selected_order.push(index);
        }

        let mut scheduler = MediaScheduler::new(schedule).map_err(PredictiveError::Schedule)?;
        let mut owned: Vec<_> = candidates.into_iter().map(Some).collect();
        let mut selected = Vec::with_capacity(selected_order.len());
        for index in selected_order {
            let Some(candidate) = owned[index].take() else {
                return Err(PredictiveError::Duplicate);
            };
            let PredictiveCandidate {
                demand,
                operation,
                prerequisites,
                rights_basis,
                command,
                ..
            } = candidate;
            scheduler
                .admit(
                    demand.id,
                    operation,
                    media_priority(demand.priority),
                    command,
                )
                .map_err(|refusal| PredictiveError::Schedule(refusal.reason))?;
            selected.push(Planned {
                demand,
                prerequisites,
                rights_basis,
            });
        }
        Ok(Self {
            candidates: selected,
            scheduler,
            admitted_now: current.now,
            selected_output_bytes,
            deferred_items: candidate_count - selected_demand_ids.len(),
        })
    }

    /// Recheck source, audience, rights basis and expiry before giving out an owned dispatch.
    /// `ready` is supplied by the complete-publication owner, never inferred from a job finish.
    pub fn begin(
        &mut self,
        current: PredictiveContext<'_, RightsBasis>,
        ready: &[RecordId],
    ) -> Result<PredictiveBegin, PredictiveError> {
        if ready.len() > MAXIMUM_INPUT_CANDIDATES
            || ready.iter().enumerate().any(|(index, id)| {
                ready.iter().take(index).any(|prior| prior == id)
                    || !self
                        .candidates
                        .iter()
                        .any(|candidate| candidate.demand.id == *id)
            })
        {
            return Err(PredictiveError::InvalidReady);
        }
        let mut retired = Vec::new();
        for candidate in &self.candidates {
            if (!same_provenance(&candidate.demand, &candidate.rights_basis, &current)
                || !same_timebase(candidate.demand.expires, current.now)
                || current.now.ticks < self.admitted_now.ticks
                || current.now.ticks >= candidate.demand.expires.ticks)
                && let Some(request) = self.scheduler.cancel_queued(candidate.demand.id)
            {
                retired.push(request);
            }
        }
        let candidates = &self.candidates;
        let dispatch = self
            .scheduler
            .begin_eligible(|request| {
                candidates.iter().any(|candidate| {
                    candidate.demand.id == *request.id()
                        && candidate
                            .prerequisites
                            .iter()
                            .all(|dependency| ready.contains(dependency))
                })
            })
            .map_err(PredictiveError::Schedule)?;
        Ok(PredictiveBegin { dispatch, retired })
    }

    /// Terminate an owned dispatch through the same scheduler that admitted it.
    pub fn finish(
        &mut self,
        dispatch: MediaDispatch<RecordId>,
        outcome: DispatchOutcome,
    ) -> Result<MediaRequest<RecordId>, DispatchRefusal<RecordId>> {
        self.scheduler.finish(dispatch, outcome)
    }

    pub fn snapshot(&self) -> PredictiveSnapshot {
        PredictiveSnapshot {
            selected_items: self.candidates.len(),
            selected_output_bytes: self.selected_output_bytes,
            deferred_items: self.deferred_items,
            schedule: self.scheduler.snapshot(),
        }
    }
}

fn same_provenance<RightsBasis: Eq>(
    demand: &AssetDemand,
    rights_basis: &RightsBasis,
    current: &PredictiveContext<'_, RightsBasis>,
) -> bool {
    demand.basis == current.basis
        && demand.key.source == current.source
        && &demand.key.audience == current.audience
        && rights_basis == current.rights_basis
}

fn same_timebase(expiry: LogicalTime, now: LogicalTime) -> bool {
    expiry.ticks_per_second != 0 && expiry.ticks_per_second == now.ticks_per_second
}

fn validate_dependencies<RightsBasis>(
    candidates: &[PredictiveCandidate<RightsBasis>],
) -> Result<(), PredictiveError> {
    // Bound every node before a traversal can copy a later node's prerequisites.
    if candidates
        .iter()
        .any(|candidate| candidate.prerequisites.len() > MAXIMUM_INPUT_CANDIDATES)
    {
        return Err(PredictiveError::Capacity);
    }
    for candidate in candidates {
        for (index, prerequisite) in candidate.prerequisites.iter().enumerate() {
            if *prerequisite == candidate.demand.id
                || candidate
                    .prerequisites
                    .iter()
                    .take(index)
                    .any(|prior| prior == prerequisite)
            {
                return Err(PredictiveError::Duplicate);
            }
            if !candidates
                .iter()
                .any(|item| item.demand.id == *prerequisite)
            {
                return Err(PredictiveError::MissingDependency);
            }
        }
    }
    for candidate in candidates {
        let mut pending_dependencies = candidate.prerequisites.clone();
        let mut visited_dependencies = Vec::new();
        while let Some(dependency_id) = pending_dependencies.pop() {
            if dependency_id == candidate.demand.id {
                return Err(PredictiveError::DependencyCycle);
            }
            if visited_dependencies.contains(&dependency_id) {
                continue;
            }
            visited_dependencies.push(dependency_id);
            if let Some(dependency) = candidates
                .iter()
                .find(|item| item.demand.id == dependency_id)
            {
                pending_dependencies.extend_from_slice(&dependency.prerequisites);
            }
        }
    }
    Ok(())
}

fn compare_candidates<RightsBasis>(
    left: &PredictiveCandidate<RightsBasis>,
    right: &PredictiveCandidate<RightsBasis>,
) -> std::cmp::Ordering {
    priority_rank(left.demand.priority)
        .cmp(&priority_rank(right.demand.priority))
        .then(right.heuristic.likelihood.cmp(&left.heuristic.likelihood))
        .then(right.heuristic.importance.cmp(&left.heuristic.importance))
        .then(right.heuristic.urgency.cmp(&left.heuristic.urgency))
        .then(right.heuristic.reusable.cmp(&left.heuristic.reusable))
        .then(
            left.heuristic
                .estimated_cost
                .cmp(&right.heuristic.estimated_cost),
        )
        .then(left.demand.id.cmp(&right.demand.id))
}

fn priority_rank(priority: DemandPriority) -> u8 {
    match priority {
        DemandPriority::InteractionCritical => 0,
        DemandPriority::SoonLikely => 1,
        DemandPriority::Optional => 2,
    }
}

fn media_priority(priority: DemandPriority) -> MediaPriority {
    match priority {
        DemandPriority::InteractionCritical => MediaPriority::Required,
        DemandPriority::SoonLikely => MediaPriority::Likely,
        DemandPriority::Optional => MediaPriority::Optional,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use df_model::checkpoint::{
        AssetRequestKey, CHECKPOINT_SCHEMA, ContentReference, ExecutionMode,
    };
    use df_types::{RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision};

    fn label(value: &str) -> RevisionLabel {
        RevisionLabel::new(Some(value)).unwrap()
    }

    fn id(value: u8) -> RecordId {
        RecordId::from_bytes(&[value; 16]).unwrap()
    }

    fn basis() -> Basis {
        Basis {
            session: SessionId::from_bytes(&[1; 16]).unwrap(),
            run: RunId::from_bytes(&[2; 16]).unwrap(),
            revision: SessionRevision::new(RecoveryEpoch::new(1).unwrap(), 1),
        }
    }

    fn context<'a>(
        audience: &'a AudienceScope,
        rights_basis: &'a u64,
        ticks: u64,
    ) -> PredictiveContext<'a, u64> {
        PredictiveContext {
            basis: basis(),
            source: ContentDigest([7; 32]),
            audience,
            rights_basis,
            now: LogicalTime {
                ticks,
                ticks_per_second: 1,
            },
        }
    }

    fn policy() -> PrefetchPolicy {
        PrefetchPolicy {
            definition: ContentReference {
                package: label("package"),
                entry: label("prefetch"),
            },
            maximum_candidates: 3,
            maximum_branches: 2,
            maximum_bytes: 64,
            maximum_duration_ticks: 10,
        }
    }

    fn schedule() -> ScheduleLimits {
        ScheduleLimits {
            queue_items: 5,
            queue_bytes: 64,
            speech_items: 1,
            speech_bytes: 8,
            execution_slots: 3,
            speech_slots: 1,
        }
    }

    fn candidate(value: u8, priority: DemandPriority) -> PredictiveCandidate<u64> {
        PredictiveCandidate {
            demand: AssetDemand {
                id: id(value),
                basis: basis(),
                key: AssetRequestKey {
                    schema: CHECKPOINT_SCHEMA,
                    source: ContentDigest([7; 32]),
                    moment: id(30),
                    identity: label("identity"),
                    style: label("style"),
                    voice: None,
                    provider: label("prepared"),
                    model: label("model"),
                    format: label("image"),
                    references: Vec::new(),
                    audience: AudienceScope::Shared,
                    parameters: label("parameters"),
                },
                priority,
                mode: ExecutionMode::PreparedOnly,
                expires: LogicalTime {
                    ticks: 20,
                    ticks_per_second: 1,
                },
                budget_reservation: label("approved-reservation"),
                maximum_bytes: 8,
                policy: policy().definition,
            },
            operation: OperationId::from_bytes(&[value; 16]).unwrap(),
            branch: id(40),
            prerequisites: Vec::new(),
            rights_basis: 9,
            heuristic: PredictiveHeuristic {
                likelihood: 50,
                importance: 50,
                urgency: 50,
                reusable: false,
                estimated_cost: 50,
            },
            command: vec![value; usize::from(value)].into_boxed_slice(),
        }
    }

    #[test]
    fn ranked_plan_uses_real_owned_scheduler_dispatch_and_preserves_bytes() {
        let audience = AudienceScope::Shared;
        let rights = 9;
        let mut low = candidate(2, DemandPriority::SoonLikely);
        low.heuristic.likelihood = 20;
        let mut high = candidate(3, DemandPriority::SoonLikely);
        high.heuristic.likelihood = 90;
        let required = candidate(1, DemandPriority::InteractionCritical);
        let mut plan = PredictivePlan::new(
            context(&audience, &rights, 10),
            &policy(),
            vec![low, high, required],
            schedule(),
        )
        .unwrap();
        assert_eq!(plan.snapshot().selected_items, 3);
        assert_eq!(plan.snapshot().selected_output_bytes, 24);
        assert_eq!(plan.snapshot().schedule.queued_bytes, 6);
        for (expected_id, expected_payload) in
            [(1, &[1][..]), (3, &[3, 3, 3][..]), (2, &[2, 2][..])]
        {
            let begun = plan.begin(context(&audience, &rights, 10), &[]).unwrap();
            assert!(begun.retired.is_empty());
            let dispatch = begun.dispatch.unwrap();
            assert_eq!(dispatch.request().id(), &id(expected_id));
            assert_eq!(dispatch.request().payload(), expected_payload);
            let request = plan.finish(dispatch, DispatchOutcome::Completed).unwrap();
            assert_eq!(request.id(), &id(expected_id));
            assert_eq!(request.payload(), expected_payload);
        }
        assert_eq!(plan.snapshot().schedule.completed_dispatches, 3);
    }

    #[test]
    fn dependency_refusals_and_owner_ready_transition_are_explicit() {
        let audience = AudienceScope::Shared;
        let rights = 9;
        let mut dependent = candidate(2, DemandPriority::SoonLikely);
        dependent.prerequisites.push(id(1));
        let mut plan = PredictivePlan::new(
            context(&audience, &rights, 10),
            &policy(),
            vec![dependent, candidate(1, DemandPriority::Optional)],
            schedule(),
        )
        .unwrap();
        let first = plan.begin(context(&audience, &rights, 10), &[]).unwrap();
        assert_eq!(first.dispatch.as_ref().unwrap().request().id(), &id(1));
        plan.finish(first.dispatch.unwrap(), DispatchOutcome::Completed)
            .unwrap();
        assert!(
            plan.begin(context(&audience, &rights, 10), &[])
                .unwrap()
                .dispatch
                .is_none()
        );
        let second = plan
            .begin(context(&audience, &rights, 10), &[id(1)])
            .unwrap();
        assert_eq!(second.dispatch.as_ref().unwrap().request().id(), &id(2));
        plan.finish(second.dispatch.unwrap(), DispatchOutcome::Completed)
            .unwrap();

        let mut duplicate = candidate(2, DemandPriority::Optional);
        duplicate.prerequisites = vec![id(1), id(1)];
        assert!(matches!(
            PredictivePlan::new(
                context(&audience, &rights, 10),
                &policy(),
                vec![candidate(1, DemandPriority::Optional), duplicate],
                schedule()
            ),
            Err(PredictiveError::Duplicate)
        ));
        let mut missing = candidate(2, DemandPriority::Optional);
        missing.prerequisites.push(id(9));
        assert!(matches!(
            PredictivePlan::new(
                context(&audience, &rights, 10),
                &policy(),
                vec![missing],
                schedule()
            ),
            Err(PredictiveError::MissingDependency)
        ));
        let mut first = candidate(1, DemandPriority::Optional);
        first.prerequisites.push(id(2));
        let mut second = candidate(2, DemandPriority::Optional);
        second.prerequisites.push(id(1));
        assert!(matches!(
            PredictivePlan::new(
                context(&audience, &rights, 10),
                &policy(),
                vec![first, second],
                schedule()
            ),
            Err(PredictiveError::DependencyCycle)
        ));
    }

    #[test]
    fn later_oversized_prerequisites_refuse_before_early_root_traversal() {
        let audience = AudienceScope::Shared;
        let rights = 9;
        for repeated in [id(1), id(9)] {
            let mut first = candidate(1, DemandPriority::Optional);
            first.prerequisites = vec![id(2)];
            let mut later = candidate(2, DemandPriority::Optional);
            later.prerequisites = vec![repeated; MAXIMUM_INPUT_CANDIDATES + 1];
            assert!(matches!(
                PredictivePlan::new(
                    context(&audience, &rights, 10),
                    &policy(),
                    vec![first, later],
                    schedule()
                ),
                Err(PredictiveError::Capacity)
            ));
        }
    }

    #[test]
    fn stale_rights_source_audience_basis_and_expiry_retire_queued_bytes() {
        let audience = AudienceScope::Shared;
        let rights = 9;
        for changed in 0..7 {
            let mut plan = PredictivePlan::new(
                context(&audience, &rights, 10),
                &policy(),
                vec![candidate(3, DemandPriority::Optional)],
                schedule(),
            )
            .unwrap();
            let different_audience = AudienceScope::Host;
            let different_rights = 10;
            let mut current = context(&audience, &rights, 10);
            match changed {
                0 => current.rights_basis = &different_rights,
                1 => current.source = ContentDigest([8; 32]),
                2 => current.audience = &different_audience,
                3 => {
                    current.basis.revision = SessionRevision::new(RecoveryEpoch::new(1).unwrap(), 2)
                }
                4 => current.now.ticks = 20,
                5 => current.now.ticks_per_second = 2,
                _ => current.now.ticks = 9,
            }
            let begun = plan.begin(current, &[]).unwrap();
            assert!(begun.dispatch.is_none());
            assert_eq!(begun.retired.len(), 1);
            assert_eq!(begun.retired[0].id(), &id(3));
            assert_eq!(begun.retired[0].payload(), &[3, 3, 3]);
            assert_eq!(plan.snapshot().schedule.queued_bytes, 0);
        }
    }

    #[test]
    fn caps_and_invalid_heuristics_timebase_and_horizon_refuse() {
        let audience = AudienceScope::Shared;
        let rights = 9;
        let mut limited = policy();
        limited.maximum_candidates = 1;
        let plan = PredictivePlan::new(
            context(&audience, &rights, 10),
            &limited,
            vec![
                candidate(1, DemandPriority::Optional),
                candidate(2, DemandPriority::Optional),
            ],
            schedule(),
        )
        .unwrap();
        assert_eq!(plan.snapshot().selected_items, 1);
        assert_eq!(plan.snapshot().deferred_items, 1);
        let mut invalid = candidate(1, DemandPriority::Optional);
        invalid.heuristic.likelihood = 101;
        assert!(matches!(
            PredictivePlan::new(
                context(&audience, &rights, 10),
                &policy(),
                vec![invalid],
                schedule()
            ),
            Err(PredictiveError::InvalidHeuristic)
        ));
        let mut invalid = candidate(1, DemandPriority::Optional);
        invalid.demand.expires.ticks_per_second = 0;
        assert!(matches!(
            PredictivePlan::new(
                context(&audience, &rights, 10),
                &policy(),
                vec![invalid],
                schedule()
            ),
            Err(PredictiveError::InvalidTimebase)
        ));
        let mut invalid = candidate(1, DemandPriority::Optional);
        invalid.demand.expires.ticks = 21;
        assert!(matches!(
            PredictivePlan::new(
                context(&audience, &rights, 10),
                &policy(),
                vec![invalid],
                schedule()
            ),
            Err(PredictiveError::Capacity)
        ));
    }

    #[test]
    fn each_heuristic_tie_break_and_branch_byte_limits_select_bounded_work() {
        let audience = AudienceScope::Shared;
        let rights = 9;
        let mut top_one = policy();
        top_one.maximum_candidates = 1;
        for winner in 0..5 {
            let first = candidate(1, DemandPriority::Optional);
            let mut second = candidate(2, DemandPriority::Optional);
            match winner {
                0 => second.heuristic.likelihood += 1,
                1 => second.heuristic.importance += 1,
                2 => second.heuristic.urgency += 1,
                3 => second.heuristic.reusable = true,
                _ => second.heuristic.estimated_cost -= 1,
            }
            let mut plan = PredictivePlan::new(
                context(&audience, &rights, 10),
                &top_one,
                vec![first, second],
                schedule(),
            )
            .unwrap();
            assert_eq!(
                plan.begin(context(&audience, &rights, 10), &[])
                    .unwrap()
                    .dispatch
                    .unwrap()
                    .request()
                    .id(),
                &id(2)
            );
        }
        let mut plan = PredictivePlan::new(
            context(&audience, &rights, 10),
            &top_one,
            vec![
                candidate(2, DemandPriority::Optional),
                candidate(1, DemandPriority::Optional),
            ],
            schedule(),
        )
        .unwrap();
        assert_eq!(
            plan.begin(context(&audience, &rights, 10), &[])
                .unwrap()
                .dispatch
                .unwrap()
                .request()
                .id(),
            &id(1)
        );

        let mut limited = policy();
        limited.maximum_branches = 1;
        limited.maximum_bytes = 16;
        let mut different_branch = candidate(3, DemandPriority::Optional);
        different_branch.branch = id(41);
        let plan = PredictivePlan::new(
            context(&audience, &rights, 10),
            &limited,
            vec![
                candidate(1, DemandPriority::Optional),
                candidate(2, DemandPriority::Optional),
                different_branch,
            ],
            schedule(),
        )
        .unwrap();
        assert_eq!(plan.snapshot().selected_items, 2);
        assert_eq!(plan.snapshot().selected_output_bytes, 16);
        assert_eq!(plan.snapshot().deferred_items, 1);
    }

    #[test]
    fn duplicate_demand_self_dependency_input_cap_and_admission_fences_refuse() {
        let audience = AudienceScope::Shared;
        let rights = 9;
        assert!(matches!(
            PredictivePlan::new(
                context(&audience, &rights, 10),
                &policy(),
                vec![
                    candidate(1, DemandPriority::Optional),
                    candidate(1, DemandPriority::Optional)
                ],
                schedule()
            ),
            Err(PredictiveError::Duplicate)
        ));
        let mut self_dependent = candidate(1, DemandPriority::Optional);
        self_dependent.prerequisites.push(id(1));
        assert!(matches!(
            PredictivePlan::new(
                context(&audience, &rights, 10),
                &policy(),
                vec![self_dependent],
                schedule()
            ),
            Err(PredictiveError::Duplicate)
        ));
        let too_many: Vec<_> = (1..=65)
            .map(|value| candidate(value, DemandPriority::Optional))
            .collect();
        assert!(matches!(
            PredictivePlan::new(
                context(&audience, &rights, 10),
                &policy(),
                too_many,
                schedule()
            ),
            Err(PredictiveError::Capacity)
        ));
        let mut stale = candidate(1, DemandPriority::Optional);
        stale.demand.key.source = ContentDigest([8; 32]);
        assert!(matches!(
            PredictivePlan::new(
                context(&audience, &rights, 10),
                &policy(),
                vec![stale],
                schedule()
            ),
            Err(PredictiveError::Stale)
        ));
        let mut stale = candidate(1, DemandPriority::Optional);
        stale.rights_basis = 10;
        assert!(matches!(
            PredictivePlan::new(
                context(&audience, &rights, 10),
                &policy(),
                vec![stale],
                schedule()
            ),
            Err(PredictiveError::Stale)
        ));
        let mut stale = candidate(1, DemandPriority::Optional);
        stale.demand.key.audience = AudienceScope::Host;
        assert!(matches!(
            PredictivePlan::new(
                context(&audience, &rights, 10),
                &policy(),
                vec![stale],
                schedule()
            ),
            Err(PredictiveError::Stale)
        ));
        let mut stale = candidate(1, DemandPriority::Optional);
        stale.demand.basis.revision = SessionRevision::new(RecoveryEpoch::new(1).unwrap(), 2);
        assert!(matches!(
            PredictivePlan::new(
                context(&audience, &rights, 10),
                &policy(),
                vec![stale],
                schedule()
            ),
            Err(PredictiveError::Stale)
        ));
        let mut expired = candidate(1, DemandPriority::Optional);
        expired.demand.expires.ticks = 10;
        assert!(matches!(
            PredictivePlan::new(
                context(&audience, &rights, 10),
                &policy(),
                vec![expired],
                schedule()
            ),
            Err(PredictiveError::Expired)
        ));
    }
}

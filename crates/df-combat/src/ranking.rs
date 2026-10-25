//! Exact utility ordering over rules-owner-admitted offers and scoped evidence.
//! Numeric contributions are supplied by a source-qualified caller policy, never mechanics.
use crate::candidates::AdmittedCandidates;
use df_model::checkpoint::{
    AttributedClaim, Basis, CheckpointPins, ContentReference, EntityId, FactId, KnowledgeGrant,
    LogicalTime, NpcState, RecordId,
};
use df_types::{MemberId, OperationId};

/// The canonical model grants facts to members and retains NPC facts and beliefs.
/// No member-to-NPC permission conversion is implied by this scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KnowledgeObserver {
    Member(MemberId),
    Entity(EntityId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UtilityEvidence {
    Fact(FactId),
    Belief(RecordId),
}

/// Caller-authored policy contribution in exact signed policy units.
/// The caller must derive its value using only the referenced permitted evidence;
/// this primitive does not qualify policy source content or evaluate world mechanics.
#[derive(Debug, Eq, PartialEq)]
pub struct UtilityContribution {
    pub criterion: ContentReference,
    pub evidence: UtilityEvidence,
    pub value: i64,
}

/// Contribution assignments must be aligned with the exact admitted offer references.
pub struct UtilityAssignment<'a, Offer> {
    pub offer: &'a Offer,
    pub contributions: &'a [UtilityContribution],
}

/// Immutable current knowledge snapshot supplied by the trusted session owner.
/// Belief permission grants access to an attributed claim, never its canonical truth.
/// Audience fields govern presentation; they do not change a belief's holder.
pub struct TacticalKnowledge<'a> {
    pub basis: &'a Basis,
    pub pins: &'a CheckpointPins,
    pub observer: KnowledgeObserver,
    pub grants: &'a [KnowledgeGrant],
    pub beliefs: &'a [AttributedClaim],
    pub npcs: &'a [NpcState],
}

/// Source-qualified current policy inventory. No built-in weights or fallback tactics.
pub struct UtilityPolicy<'a> {
    pub basis: &'a Basis,
    pub pins: &'a CheckpointPins,
    pub definition: &'a ContentReference,
    pub criteria: &'a [ContentReference],
}

/// Explicit replay inputs, borrowed unchanged by the result.
/// Basis/pins must match the legal receipt, knowledge snapshot and utility policy.
pub struct RankingObservation<'a> {
    pub basis: &'a Basis,
    pub pins: &'a CheckpointPins,
    pub observer: KnowledgeObserver,
    pub logical_time: LogicalTime,
    pub cause: OperationId,
    pub policy: &'a ContentReference,
}

/// Caller-selected count bounds; output borrows all variable-sized records.
/// At most candidates*(candidates-1)/2 ordering comparisons are performed.
/// Permission checks scan at most knowledge_records records per contribution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UtilityLimits {
    pub candidates: usize,
    pub contributions: usize,
    pub knowledge_records: usize,
    pub criteria: usize,
}

/// Safe errors retain no secret evidence IDs, claims, or policy values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UtilityRankingError {
    StaleBasis,
    ObserverMismatch,
    PolicyMismatch,
    CandidateCapacity,
    ContributionCapacity,
    KnowledgeCapacity,
    PolicyCapacity,
    AssignmentMismatch,
    UndisclosedEvidence,
    UnsupportedCriterion,
    DuplicateContribution,
    UtilityOverflow,
    AllocationCapacity,
}

/// One canonical legal offer with an exact sum and its permitted explanation.
/// This server-side explanation is not an authorized audience projection.
pub struct RankedTactic<'a, Offer> {
    offer: &'a Offer,
    utility: i64,
    contributions: &'a [UtilityContribution],
}

impl<'a, Offer> RankedTactic<'a, Offer> {
    pub fn offer(&self) -> &'a Offer {
        self.offer
    }

    pub fn utility(&self) -> i64 {
        self.utility
    }

    pub fn contributions(&self) -> &'a [UtilityContribution] {
        self.contributions
    }
}

/// Stable descending ranking. Empty input returns an empty successful result.
/// Choosing an action is still a staged proposal; this grants no commit authority.
pub struct TacticalRanking<'a, Offer> {
    observation: &'a RankingObservation<'a>,
    tactics: Vec<RankedTactic<'a, Offer>>,
}

impl<'a, Offer> TacticalRanking<'a, Offer> {
    pub fn observation(&self) -> &'a RankingObservation<'a> {
        self.observation
    }

    pub fn tactics(&self) -> &[RankedTactic<'a, Offer>] {
        &self.tactics
    }

    pub fn preferred(&self) -> Option<&RankedTactic<'a, Offer>> {
        self.tactics.first()
    }
}

// Scoring retains only actor-authorized IDs, never world truth or private claim text.
struct PerceivedKnowledge {
    evidence: Vec<UtilityEvidence>,
}

impl PerceivedKnowledge {
    fn from_current(knowledge: &TacticalKnowledge<'_>) -> Result<Self, UtilityRankingError> {
        let mut perceived = Self {
            evidence: Vec::new(),
        };
        match knowledge.observer {
            KnowledgeObserver::Member(observer) => {
                for grant in knowledge.grants {
                    if grant.observer == observer {
                        perceived.retain_evidence(UtilityEvidence::Fact(grant.fact))?;
                    }
                }
            }
            KnowledgeObserver::Entity(holder) => {
                for npc in knowledge.npcs {
                    if npc.entity == holder {
                        for fact in &npc.known_facts {
                            perceived.retain_evidence(UtilityEvidence::Fact(*fact))?;
                        }
                    }
                }
                for belief in knowledge.beliefs {
                    if belief.holder == holder
                        && knowledge
                            .npcs
                            .iter()
                            .any(|npc| npc.entity == holder && npc.beliefs.contains(&belief.id))
                    {
                        perceived.retain_evidence(UtilityEvidence::Belief(belief.id))?;
                    }
                }
            }
        }
        Ok(perceived)
    }

    fn retain_evidence(&mut self, evidence: UtilityEvidence) -> Result<(), UtilityRankingError> {
        self.evidence
            .try_reserve(1)
            .map_err(|_| UtilityRankingError::AllocationCapacity)?;
        self.evidence.push(evidence);
        Ok(())
    }

    fn permits(&self, evidence: UtilityEvidence) -> bool {
        self.evidence.contains(&evidence)
    }
}

/// Rank caller-authored utility contributions over exact admitted legal offers.
///
/// Every contribution is permission-checked before inspecting its criterion or
/// value. An undisclosed reference rejects the whole batch, including any partial
/// ranking. Equal totals retain admitted order; even negative totals remain eligible.
/// No fact text/truth, dice, resource, initiative or provider state is read or changed.
/// The caller must qualify authored values, current evidence records and policy
/// inventory before calling, then revalidate the basis before commit or disclosure.
pub fn rank_tactics<'a, Offer>(
    observation: &'a RankingObservation<'a>,
    admitted: &AdmittedCandidates<'a, Offer>,
    assignments: &[UtilityAssignment<'a, Offer>],
    knowledge: &TacticalKnowledge<'_>,
    policy: &UtilityPolicy<'_>,
    limits: UtilityLimits,
) -> Result<TacticalRanking<'a, Offer>, UtilityRankingError> {
    if observation.basis != admitted.basis()
        || observation.pins != admitted.pins()
        || observation.basis != knowledge.basis
        || observation.pins != knowledge.pins
        || observation.basis != policy.basis
        || observation.pins != policy.pins
    {
        return Err(UtilityRankingError::StaleBasis);
    }
    if observation.observer != knowledge.observer {
        return Err(UtilityRankingError::ObserverMismatch);
    }
    if observation.policy != policy.definition {
        return Err(UtilityRankingError::PolicyMismatch);
    }
    if admitted.offers().len() > limits.candidates {
        return Err(UtilityRankingError::CandidateCapacity);
    }
    let knowledge_records = knowledge
        .grants
        .len()
        .checked_add(knowledge.beliefs.len())
        .and_then(|count| count.checked_add(knowledge.npcs.len()));
    let knowledge_records = knowledge.npcs.iter().fold(knowledge_records, |count, npc| {
        count
            .and_then(|count| count.checked_add(npc.known_facts.len()))
            .and_then(|count| count.checked_add(npc.beliefs.len()))
    });
    if knowledge_records.is_none_or(|count| count > limits.knowledge_records) {
        return Err(UtilityRankingError::KnowledgeCapacity);
    }
    if policy.criteria.len() > limits.criteria {
        return Err(UtilityRankingError::PolicyCapacity);
    }
    if assignments.len() != admitted.offers().len() {
        return Err(UtilityRankingError::AssignmentMismatch);
    }
    let mut contribution_count = 0_usize;
    for (assignment, offer) in assignments.iter().zip(admitted.offers()) {
        if !std::ptr::eq(assignment.offer, *offer) {
            return Err(UtilityRankingError::AssignmentMismatch);
        }
        contribution_count = contribution_count
            .checked_add(assignment.contributions.len())
            .filter(|count| *count <= limits.contributions)
            .ok_or(UtilityRankingError::ContributionCapacity)?;
    }
    let perceived = PerceivedKnowledge::from_current(knowledge)?;
    // Authorize the complete batch before inspecting any score, including duplicates.
    for assignment in assignments {
        for contribution in assignment.contributions {
            if !perceived.permits(contribution.evidence) {
                return Err(UtilityRankingError::UndisclosedEvidence);
            }
        }
    }
    let mut tactics: Vec<RankedTactic<'a, Offer>> = Vec::new();
    tactics
        .try_reserve_exact(assignments.len())
        .map_err(|_| UtilityRankingError::AllocationCapacity)?;
    for assignment in assignments {
        let mut utility = 0_i64;
        for (index, contribution) in assignment.contributions.iter().enumerate() {
            if !policy.criteria.contains(&contribution.criterion) {
                return Err(UtilityRankingError::UnsupportedCriterion);
            }
            if assignment.contributions.iter().take(index).any(|previous| {
                previous.criterion == contribution.criterion
                    && previous.evidence == contribution.evidence
            }) {
                return Err(UtilityRankingError::DuplicateContribution);
            }
            utility = utility
                .checked_add(contribution.value)
                .ok_or(UtilityRankingError::UtilityOverflow)?;
        }
        let position = tactics
            .iter()
            .position(|previous| utility > previous.utility)
            .unwrap_or(tactics.len());
        tactics.insert(
            position,
            RankedTactic {
                offer: assignment.offer,
                utility,
                contributions: assignment.contributions,
            },
        );
    }
    Ok(TacticalRanking {
        observation,
        tactics,
    })
}

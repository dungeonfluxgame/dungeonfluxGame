//! Voluntary public spotlight proposals. No delivery, consent issuance or mechanical effects.
use std::time::Duration;

use df_model::checkpoint::{
    AudienceScope, Basis, CharacterHook, ContentReference, PresenceKind, RecordId,
};
use df_types::MemberId;

use crate::cooldown::{RefusalEligibility, RefusalError, RefusalMemory, RefusalScope};
use crate::window::{ActivityWindow, WindowError};

const MAXIMUM_PARTICIPANTS: usize = 256;
const MAXIMUM_CANDIDATES: usize = 1024;

/// Current caller-verified participation permission, not a persisted preference schema.
/// The owner supplies the latest explicit opt-in, hook disclosure generation and presence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpotlightParticipant {
    pub member: MemberId,
    pub opted_in: bool,
    pub consent_generation: u64,
    pub presence: PresenceKind,
}

/// An already grounded, source-admitted opportunity from the engine's candidate pass.
/// Relevance includes current narrative/interaction eligibility, never inferred emotion.
pub struct SpotlightCandidate<'a> {
    pub id: RecordId,
    pub hook: &'a CharacterHook,
    pub basis: Basis,
    pub policy: &'a ContentReference,
    pub expires: Duration,
    pub relevant: bool,
}

/// Explicit work bounds. Output contains at most one optional proposal per member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpotlightLimits {
    pub max_participants: usize,
    pub max_candidates: usize,
    pub max_recommendations: usize,
}

/// The owner revalidates these permissions and basis again before committing delivery.
/// Activity must be the current session/run's window, advanced to this supplied logical
/// time. This operation neither advances it nor records a decline when input is absent.
pub struct SpotlightRequest<'a, EventId> {
    pub basis: Basis,
    pub now: Duration,
    pub policy: &'a ContentReference,
    pub participants: &'a [SpotlightParticipant],
    pub candidates: &'a [SpotlightCandidate<'a>],
    pub activity: &'a ActivityWindow<MemberId, EventId>,
    pub refusals: &'a RefusalMemory<MemberId, RecordId>,
    pub limits: SpotlightLimits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpotlightError {
    InvalidLimits,
    Capacity,
    DuplicateParticipant,
    DuplicateOpportunity,
    Refusal(RefusalError),
    Activity(WindowError),
}

/// Diagnostic classifications contain no hook text, secret facts or inferred motivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpotlightReason {
    Selected,
    NotParticipating,
    ConsentChanged,
    Away,
    PrivateHook,
    Ungrounded,
    Irrelevant,
    StaleBasis,
    PolicyChanged,
    Expired,
    CoolingDown,
    Alternative,
    OutputLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpotlightDecision {
    pub opportunity: RecordId,
    pub reason: SpotlightReason,
}

/// A public, optional proposal, with sufficient identity for the owner to revalidate.
/// Ignoring this value is valid; it grants no turn, bonus, disclosure or delivery right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotlightRecommendation {
    pub opportunity: RecordId,
    pub member: MemberId,
    pub hook: RecordId,
    pub source: ContentReference,
    pub policy: ContentReference,
    pub basis: Basis,
    pub consent_generation: u64,
    pub expires: Duration,
    pub accepted_activity: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotlightOutcome {
    pub recommendations: Vec<SpotlightRecommendation>,
    pub decisions: Vec<SpotlightDecision>,
}

/// Ranks eligible opportunities by accepted activity count, then member and opportunity
/// identity. Counts describe interaction only. Extra accepted activity never improves rank.
/// Exact retries return the same proposals and change neither activity nor refusal state.
/// This deliberately excludes every nonpublic hook; private delivery needs a separate
/// audience-authorized consumer, not a fallback to public spotlight.
pub fn recommend<EventId: Copy + Eq>(
    request: SpotlightRequest<'_, EventId>,
) -> Result<SpotlightOutcome, SpotlightError> {
    let limits = request.limits;
    if limits.max_participants == 0
        || limits.max_participants > MAXIMUM_PARTICIPANTS
        || limits.max_candidates == 0
        || limits.max_candidates > MAXIMUM_CANDIDATES
        || limits.max_recommendations == 0
        || limits.max_recommendations > limits.max_participants
    {
        return Err(SpotlightError::InvalidLimits);
    }
    if request.participants.len() > limits.max_participants
        || request.candidates.len() > limits.max_candidates
    {
        return Err(SpotlightError::Capacity);
    }
    request
        .activity
        .validate_current(
            request.basis.session,
            request.basis.run,
            request.basis.revision,
            request.now,
        )
        .map_err(SpotlightError::Activity)?;
    for (index, participant) in request.participants.iter().enumerate() {
        if request
            .participants
            .iter()
            .take(index)
            .any(|prior| prior.member == participant.member)
        {
            return Err(SpotlightError::DuplicateParticipant);
        }
    }
    for (index, candidate) in request.candidates.iter().enumerate() {
        if request
            .candidates
            .iter()
            .take(index)
            .any(|prior| prior.id == candidate.id)
        {
            return Err(SpotlightError::DuplicateOpportunity);
        }
    }
    let mut eligible = Vec::new();
    let mut decisions = Vec::new();
    let mut recommendations = Vec::new();
    eligible
        .try_reserve_exact(request.candidates.len())
        .map_err(|_| SpotlightError::Capacity)?;
    decisions
        .try_reserve_exact(request.candidates.len())
        .map_err(|_| SpotlightError::Capacity)?;
    recommendations
        .try_reserve_exact(limits.max_recommendations)
        .map_err(|_| SpotlightError::Capacity)?;
    let scope = RefusalScope {
        session: request.basis.session,
        run: request.basis.run,
    };
    for candidate in request.candidates {
        let participant = request
            .participants
            .iter()
            .find(|participant| participant.member == candidate.hook.member);
        let reason = if candidate.hook.audience != AudienceScope::Shared {
            Some(SpotlightReason::PrivateHook)
        } else {
            match participant {
                None => Some(SpotlightReason::NotParticipating),
                Some(participant) if !participant.opted_in => {
                    Some(SpotlightReason::NotParticipating)
                }
                Some(participant)
                    if participant.consent_generation == 0
                        || candidate.hook.consent_generation != participant.consent_generation =>
                {
                    Some(SpotlightReason::ConsentChanged)
                }
                Some(participant) if participant.presence != PresenceKind::Connected => {
                    Some(SpotlightReason::Away)
                }
                Some(_) if candidate.hook.source_facts.is_empty() => {
                    Some(SpotlightReason::Ungrounded)
                }
                Some(_) if candidate.basis != request.basis => Some(SpotlightReason::StaleBasis),
                Some(_) if candidate.policy != request.policy => {
                    Some(SpotlightReason::PolicyChanged)
                }
                Some(_) if candidate.expires <= request.now => Some(SpotlightReason::Expired),
                Some(_) if !candidate.relevant => Some(SpotlightReason::Irrelevant),
                Some(_) => match request
                    .refusals
                    .eligibility(scope, candidate.hook.member, request.now)
                    .map_err(SpotlightError::Refusal)?
                {
                    RefusalEligibility::Eligible => None,
                    RefusalEligibility::CoolingDown { .. } => Some(SpotlightReason::CoolingDown),
                },
            }
        };
        if let Some(reason) = reason {
            decisions.push(SpotlightDecision {
                opportunity: candidate.id,
                reason,
            });
        } else {
            eligible.push((
                request.activity.activity_count(&candidate.hook.member),
                candidate.hook.member,
                candidate.id,
                candidate,
            ));
        }
    }
    eligible.sort_unstable_by_key(|(count, member, id, _)| (*count, *member, *id));
    for (count, member, id, candidate) in eligible {
        let reason = if recommendations
            .iter()
            .any(|prior: &SpotlightRecommendation| prior.member == member)
        {
            SpotlightReason::Alternative
        } else if recommendations.len() == limits.max_recommendations {
            SpotlightReason::OutputLimit
        } else {
            recommendations.push(SpotlightRecommendation {
                opportunity: id,
                member,
                hook: candidate.hook.id,
                source: candidate.hook.definition.clone(),
                policy: request.policy.clone(),
                basis: request.basis,
                consent_generation: candidate.hook.consent_generation,
                expires: candidate.expires,
                accepted_activity: count,
            });
            SpotlightReason::Selected
        };
        decisions.push(SpotlightDecision {
            opportunity: id,
            reason,
        });
    }
    decisions.sort_unstable_by_key(|decision| decision.opportunity);
    Ok(SpotlightOutcome {
        recommendations,
        decisions,
    })
}

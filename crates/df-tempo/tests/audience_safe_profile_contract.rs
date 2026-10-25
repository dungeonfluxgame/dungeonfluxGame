//! Original D04 design example at the existing scalar tempo boundary.
//! Equal owner-qualified inputs must not reveal unrelated private/future checkpoint data.
//! This fixture emits no full profile, cue or prefetch demand and does not authorize a target.
mod tempo_fixture;

use std::time::Duration;

use df_model::checkpoint::*;
use df_tempo::elapsed::{ElapsedDisposition, TempoAdvanceError};
use df_tempo::impulse::*;
use df_types::MemberId;
use tempo_fixture::*;

const SOURCE_DECISION: &str = include_str!("support/audience_safe_profile_contract.json");

fn private_fact() -> FactId {
    FactId::from_bytes(&[21; 16]).unwrap()
}

fn hidden_world(
    mode: ExecutionMode,
    audience: AudienceScope,
    progress: u64,
    due_ticks: u64,
) -> Checkpoint {
    let mut state = checkpoint_for(mode, vec![(content(), 7)], 40)
        .state()
        .clone();
    let entity = EntityId::from_bytes(&[20; 16]).unwrap();
    state.entities.push(WorldEntity {
        id: entity,
        definition: content(),
        location: None,
        position: None,
        identity_revision: label("hidden-fixture-entity"),
    });
    state.facts.push(GameFact {
        id: private_fact(),
        revision: state.decisions[0].revision,
        operation: state.decisions[0].operation,
        ordinal: 1,
        cause: Some(fact_id()),
        audience,
        value: FactValue::ContentEvent {
            definition: content(),
            subjects: if progress > 4 { vec![entity] } else { vec![] },
        },
    });
    state.decisions[0].facts.push(private_fact());
    state.schedules.push(ScheduledEvent {
        id: RecordId::from_bytes(&[22; 16]).unwrap(),
        entity,
        due: LogicalTime {
            ticks: due_ticks,
            ticks_per_second: 10,
        },
        definition: content(),
    });
    state.threats.push(ThreatClock {
        id: RecordId::from_bytes(&[23; 16]).unwrap(),
        definition: content(),
        progress,
        capacity: 8,
    });
    checkpoint_from(state)
}

fn impulse_policy(current: &Checkpoint) -> ImpulsePolicy {
    ImpulsePolicy {
        definition: current.state().tempo.policy.clone(),
        minimum_intensity: 0,
        maximum_intensity: 50,
        maximum_absolute_inertia: 20,
        recovery_units_per_tick: 1,
        rise_units_per_tick: 2,
        fall_units_per_tick: 3,
        deadband_units: 2,
        named_impulses: vec![NamedImpulseRule {
            event: content(),
            inertia_units: 40,
            rise_units_per_tick: 10,
            fall_units_per_tick: 10,
        }],
    }
}

fn permitted(
    current: &Checkpoint,
    target_intensity: i64,
    fact: Option<FactId>,
) -> PermittedTempoInput {
    PermittedTempoInput {
        expected_basis: current.basis(),
        target_intensity,
        impulse: fact.map(|fact| ImpulseEvent {
            fact,
            at_presentation_ticks: current.state().tempo.presentation_ticks,
        }),
    }
}

fn propose(
    current: &Checkpoint,
    input: PermittedTempoInput,
    paused: bool,
) -> Result<ImpulseAdvance, ImpulseError> {
    advance_impulse(
        current,
        current.pins(),
        request(current, Duration::from_secs(1), paused),
        Some(&policy(current)),
        Some(&impulse_policy(current)),
        input,
        ImpulseLimits {
            elapsed: limits(),
            maximum_named_impulses: 4,
        },
    )
}

#[test]
fn paired_hidden_future_variants_leave_complete_scalar_proposal_equal() {
    assert!(SOURCE_DECISION.contains("hidden future events never shape visible hints"));
    for audience in [
        AudienceScope::Host,
        AudienceScope::Members(vec![MemberId::from_bytes(&[3; 16]).unwrap()]),
    ] {
        let quiet = hidden_world(ExecutionMode::Live, audience.clone(), 1, 900);
        let imminent = hidden_world(ExecutionMode::Live, audience, 7, 90);
        let quiet_before = quiet.clone();
        let imminent_before = imminent.clone();
        assert_ne!(quiet.state().facts[1], imminent.state().facts[1]);
        assert_ne!(quiet.state().threats, imminent.state().threats);
        assert_ne!(quiet.state().schedules, imminent.state().schedules);
        assert_eq!(quiet.state().tempo, imminent.state().tempo);
        for shock in [None, Some(fact_id())] {
            let left = propose(&quiet, permitted(&quiet, 31, shock), false).unwrap();
            let right = propose(&imminent, permitted(&imminent, 31, shock), false).unwrap();
            assert_eq!(left, right);
            let left_curve = left.curve.unwrap();
            let right_curve = right.curve.unwrap();
            for tick in left_curve.from_ticks()..=left_curve.through_ticks() {
                assert_eq!(
                    left_curve.intensity_at(tick),
                    right_curve.intensity_at(tick)
                );
            }
        }
        assert_eq!(quiet, quiet_before);
        assert_eq!(imminent, imminent_before);
    }
}

#[test]
fn public_committed_cue_changes_output_only_after_admitted_input() {
    let current = hidden_world(ExecutionMode::Live, AudienceScope::Host, 7, 90);
    let before = current.clone();
    let ordinary = propose(&current, permitted(&current, 31, None), false).unwrap();
    let public = propose(&current, permitted(&current, 31, Some(fact_id())), false).unwrap();
    assert_eq!(ordinary.state.intensity, 31);
    assert_eq!(public.state.intensity, 50);
    assert_ne!(ordinary.curve, public.curve);
    let mut candidate = current.state().clone();
    candidate.tempo = public.state;
    assert_eq!(candidate.logical_time, current.state().logical_time);
    assert_eq!(candidate.facts, current.state().facts);
    assert_eq!(candidate.decisions, current.state().decisions);
    assert_eq!(candidate.schedules, current.state().schedules);
    assert_eq!(candidate.threats, current.state().threats);
    candidate.tempo = current.state().tempo.clone();
    assert_eq!(candidate, *current.state());
    assert_eq!(current, before);
}

#[test]
fn host_and_member_facts_never_become_shared_named_impulses() {
    for audience in [
        AudienceScope::Host,
        AudienceScope::Members(vec![MemberId::from_bytes(&[3; 16]).unwrap()]),
    ] {
        let current = hidden_world(ExecutionMode::Live, audience, 7, 90);
        let before = current.clone();
        for paused in [false, true] {
            assert_eq!(
                propose(
                    &current,
                    permitted(&current, 31, Some(private_fact())),
                    paused
                ),
                Err(ImpulseError::HiddenImpulse)
            );
        }
        let mut with_grant = current.state().clone();
        with_grant.knowledge.push(KnowledgeGrant {
            observer: MemberId::from_bytes(&[3; 16]).unwrap(),
            fact: private_fact(),
            source: private_fact(),
        });
        let known_privately = checkpoint_from(with_grant);
        assert_eq!(
            propose(
                &known_privately,
                permitted(&known_privately, 31, Some(private_fact())),
                false
            ),
            Err(ImpulseError::HiddenImpulse)
        );
        assert_eq!(current, before);
    }
}

#[test]
fn scheduled_future_event_without_committed_fact_is_not_an_impulse() {
    let current = hidden_world(ExecutionMode::Live, AudienceScope::Host, 7, 90);
    let before = current.clone();
    let future = FactId::from_bytes(&[22; 16]).unwrap();
    assert_eq!(
        propose(&current, permitted(&current, 31, Some(future)), false),
        Err(ImpulseError::UnknownFact)
    );
    assert_eq!(current, before);
}

#[test]
fn exact_binding_and_anchor_rejections_preserve_canonical_state() {
    let current = hidden_world(ExecutionMode::Live, AudienceScope::Host, 7, 90);
    let before = current.clone();
    let mut input = permitted(&current, 31, Some(fact_id()));
    input.expected_basis.revision = revision(7);
    assert_eq!(
        propose(&current, input, false),
        Err(ImpulseError::StaleInput)
    );
    input = permitted(&current, 31, Some(fact_id()));
    input.impulse.as_mut().unwrap().at_presentation_ticks -= 1;
    assert_eq!(
        propose(&current, input, false),
        Err(ImpulseError::StaleImpulseAnchor)
    );
    let mut stale_request = request(&current, Duration::from_secs(1), false);
    stale_request.expected_basis.revision = revision(7);
    assert!(matches!(
        advance_impulse(
            &current,
            current.pins(),
            stale_request,
            Some(&policy(&current)),
            Some(&impulse_policy(&current)),
            permitted(&current, 31, Some(fact_id())),
            ImpulseLimits {
                elapsed: limits(),
                maximum_named_impulses: 4
            }
        ),
        Err(ImpulseError::Elapsed(TempoAdvanceError::Binding(_)))
    ));
    assert_eq!(current, before);
}

#[test]
fn pause_replay_keep_hidden_variants_equal_and_validate_inputs() {
    for (mode, paused, disposition) in [
        (ExecutionMode::Live, true, ElapsedDisposition::Paused),
        (ExecutionMode::Replay, false, ElapsedDisposition::Replay),
    ] {
        let quiet = hidden_world(mode, AudienceScope::Host, 1, 900);
        let imminent = hidden_world(mode, AudienceScope::Host, 7, 90);
        let left = propose(&quiet, permitted(&quiet, 31, Some(fact_id())), paused).unwrap();
        let right = propose(&imminent, permitted(&imminent, 31, Some(fact_id())), paused).unwrap();
        assert_eq!(left, right);
        assert_eq!(left.disposition, disposition);
        assert_eq!(left.state, quiet.state().tempo);
        assert_eq!(left.curve, None);
        assert_eq!(
            propose(
                &imminent,
                permitted(&imminent, 31, Some(private_fact())),
                paused
            ),
            Err(ImpulseError::HiddenImpulse)
        );
    }
}

#[test]
fn target_sensitivity_retains_owner_authorization_gate() {
    assert!(SOURCE_DECISION.contains("Basis equality proves freshness, not authorization"));
    let current = hidden_world(ExecutionMode::Live, AudienceScope::Host, 7, 90);
    let before = current.clone();
    let low = propose(&current, permitted(&current, 0, None), false).unwrap();
    let high = propose(&current, permitted(&current, 50, None), false).unwrap();
    assert_ne!(low.state.intensity, high.state.intensity);
    assert_ne!(low.curve, high.curve);
    assert_eq!(current, before);
}

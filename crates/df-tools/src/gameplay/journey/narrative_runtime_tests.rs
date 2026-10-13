use super::{
    MEMBERS, PACKET_THREAD, THREAD_POLICY, THREAT_THREAD, accepted, model, offered, phase, set,
    stage_with_supplier,
    tests::{inn_command, input, opening_story},
};
use df_model::checkpoint::*;
use df_persistence::local_demo_scope::LocalDemoRole;
use df_protocol::common as rpc;
use df_session::submission::RepositoryError;
use df_types::MemberId;

fn member(index: usize) -> MemberId {
    MemberId::from_bytes(&MEMBERS[index]).unwrap()
}

fn dialogue(entry: &str) -> Checkpoint {
    let opening = opening_story();
    stage_with_supplier(
        &opening,
        &input(&opening, member(0), 6, entry, vec![]),
        &mut |_| panic!("dialogue cannot draw"),
    )
    .unwrap()
}

fn attack(current: &Checkpoint, index: usize, operation: u8, damage: u32) -> Checkpoint {
    stage_with_supplier(
        current,
        &input(
            current,
            member(index),
            operation,
            "greatsword-attack",
            vec![
                (
                    model::label("savage-attacker").unwrap(),
                    model::label("no").unwrap(),
                ),
                (model::label("graze").unwrap(), model::label("no").unwrap()),
            ],
        ),
        &mut |sides| Ok(if sides == 20 { 10 } else { damage }),
    )
    .unwrap()
}

fn victory(entry: &str) -> Checkpoint {
    let dialogue = dialogue(entry);
    for index in 0..2 {
        assert!(
            offered(&dialogue, member(index))
                .unwrap()
                .iter()
                .any(|(kind, _)| { *kind == rpc::GameplayActionKind::DefendCourier })
        );
    }
    let combat = stage_with_supplier(
        &dialogue,
        &input(&dialogue, member(0), 7, "defend-courier", vec![]),
        &mut |_| Ok(10),
    )
    .unwrap();
    let injured = attack(&combat, 0, 8, 1);
    assert_eq!(phase(&injured).unwrap(), rpc::JourneyPhase::Combat);
    let next = stage_with_supplier(
        &injured,
        &input(&injured, member(0), 9, "end-turn", vec![]),
        &mut |_| panic!("next hero cannot draw"),
    )
    .unwrap();
    attack(&next, 1, 10, 6)
}

fn refuses(current: &Checkpoint, entry: &str) {
    let before = current.clone();
    assert_eq!(phase(current), Err(RepositoryError::InvalidCandidate));
    assert!(offered(current, member(0)).is_err());
    assert!(super::super::wire::journey_view(current, LocalDemoRole::Player, member(0)).is_err());
    assert_eq!(
        stage_with_supplier(
            current,
            &input(current, member(0), 30, entry, vec![]),
            &mut |_| panic!("unowned narrative cannot draw"),
        ),
        Err(RepositoryError::InvalidCandidate)
    );
    assert_eq!(current, &before);
}

#[test]
fn registered_narrative_runtime_retains_both_deliveries_through_recovery_rest_and_inn() {
    for entry in ["ask-courier", "escort-courier"] {
        let current = victory(entry);
        assert_eq!(phase(&current).unwrap(), rpc::JourneyPhase::Complete);
        assert_eq!(
            current.state().narrative.open_threads,
            vec![model::content(PACKET_THREAD).unwrap()]
        );
        let retained = current.state().facts.clone();
        let knowledge = current.state().knowledge.clone();
        let ledger = current.state().narrative.accepted_facts.clone();
        assert_eq!(ledger.len(), 6);
        let recovered = model::checkpoint(current.basis(), current.state().clone()).unwrap();
        assert_eq!(recovered, current);
        // The played fight leaves both heroes untouched, so spend a
        // Second Wind use on paper for the rest to regain.
        let mut spent = recovered.state().clone();
        let hero = spent.characters[0].entity;
        set(&mut spent, hero, "second-wind", 1).unwrap();
        let recovered = model::checkpoint(recovered.basis(), spent).unwrap();
        let rest_input = input(&recovered, member(0), 11, "short-rest", vec![]);
        let rested = stage_with_supplier(&recovered, &rest_input, &mut |_| {
            panic!("recovered rest cannot draw")
        })
        .unwrap();
        assert_eq!(rested.state().narrative.accepted_facts, ledger);
        assert_eq!(rested.state().knowledge, knowledge);
        assert_eq!(&rested.state().facts[..retained.len()], retained);
        assert_eq!(
            stage_with_supplier(&recovered, &rest_input, &mut |_| panic!("same input")).unwrap(),
            rested
        );
        let scene = inn_command(&rested, member(1), 12);
        let arrived =
            stage_with_supplier(&rested, &scene, &mut |_| panic!("inn cannot draw")).unwrap();
        assert_eq!(phase(&arrived).unwrap(), rpc::JourneyPhase::Complete);
        assert_eq!(arrived.state().knowledge, knowledge);
        assert_eq!(&arrived.state().facts[..retained.len()], retained);
        assert_eq!(
            arrived.state().narrative.accepted_facts.len(),
            ledger.len() + 1
        );
        let accepted_source = arrived.state().facts.last().unwrap();
        assert_eq!(
            arrived.state().narrative.accepted_facts.last(),
            Some(&accepted_source.id)
        );
        assert_eq!(accepted_source.audience, AudienceScope::Shared);
        assert!(
            !offered(&arrived, member(1))
                .unwrap()
                .iter()
                .any(|(kind, _)| { *kind == rpc::GameplayActionKind::ChooseHarborScene })
        );
    }
}

#[test]
fn narrative_runtime_refuses_foreign_missing_reordered_and_duplicate_accepted_fact_ownership() {
    let current = victory("escort-courier");
    for case in 0..6 {
        let mut state = current.state().clone();
        match case {
            0 => state.narrative.accepted_facts.insert(0, state.facts[0].id),
            1 => {
                state.narrative.accepted_facts.remove(3);
            }
            2 => state.narrative.accepted_facts.swap(2, 3),
            3 => state
                .narrative
                .accepted_facts
                .push(state.narrative.accepted_facts[3]),
            4 => {
                state
                    .decisions
                    .iter_mut()
                    .find(|decision| decision.operation.as_bytes() == &[8; 16])
                    .unwrap()
                    .source_policy = model::label("foreign-narrative-owner").unwrap();
            }
            _ => {
                let fact = state
                    .facts
                    .iter_mut()
                    .find(|fact| {
                        fact.operation.as_bytes() == &[8; 16]
                            && matches!(&fact.value, FactValue::ContentEvent { definition, .. }
                            if definition.entry.as_str() == "greatsword-attack")
                    })
                    .unwrap();
                let FactValue::ContentEvent { definition, .. } = &mut fact.value else {
                    panic!("attack source")
                };
                *definition = model::content("short-rest").unwrap();
            }
        }
        let forged = model::checkpoint(current.basis(), state).unwrap();
        refuses(&forged, "short-rest");
    }
}

#[test]
fn narrative_runtime_refuses_extra_reordered_or_reopened_threads_without_native_progress() {
    let opening = opening_story();
    let complete = victory("escort-courier");
    for (current, entry, extra) in [
        (&opening, "escort-courier", "harbor-inn"),
        (&complete, "short-rest", THREAT_THREAD),
    ] {
        let mut state = current.state().clone();
        state
            .narrative
            .open_threads
            .push(model::content(extra).unwrap());
        refuses(&model::checkpoint(current.basis(), state).unwrap(), entry);
    }
    let dialogue = dialogue("escort-courier");
    let combat = stage_with_supplier(
        &dialogue,
        &input(&dialogue, member(0), 7, "defend-courier", vec![]),
        &mut |_| Ok(10),
    )
    .unwrap();
    let mut state = combat.state().clone();
    state.narrative.open_threads.swap(0, 1);
    refuses(
        &model::checkpoint(combat.basis(), state).unwrap(),
        "end-turn",
    );
}

#[test]
fn narrative_runtime_preserves_revoked_private_source_and_current_audience_projection() {
    let private = dialogue("ask-courier");
    let display = super::bootstrap_member().unwrap();
    let visible =
        super::super::wire::journey_view(&private, LocalDemoRole::Display, display).unwrap();
    for case in 0..6 {
        let mut state = private.state().clone();
        let source = state.facts.last_mut().unwrap();
        match case {
            0 => source.audience = AudienceScope::Host,
            1 => source.audience = AudienceScope::Shared,
            2 => source.audience = AudienceScope::Members(vec![display]),
            3 => source.audience = AudienceScope::Members(vec![member(0), member(1)]),
            4 => {
                source.value = FactValue::ContentEvent {
                    definition: model::content("harbor").unwrap(),
                    subjects: vec![],
                };
            }
            _ => {
                let FactValue::ContentEvent { subjects, .. } = &mut source.value else {
                    panic!("private source")
                };
                subjects.push(super::room_entity().unwrap());
            }
        }
        let changed = model::checkpoint(private.basis(), state).unwrap();
        let before = changed.clone();
        assert_eq!(phase(&changed).unwrap(), rpc::JourneyPhase::Dialogue);
        assert_eq!(changed.state().narrative, private.state().narrative);
        assert_eq!(
            super::super::wire::journey_view(&changed, LocalDemoRole::Display, display).unwrap(),
            visible
        );
        assert!(
            !offered(&changed, member(0))
                .unwrap()
                .iter()
                .any(|(kind, _)| { *kind == rpc::GameplayActionKind::DefendCourier })
        );
        assert!(
            stage_with_supplier(
                &changed,
                &input(&changed, member(0), 30, "defend-courier", vec![]),
                &mut |_| panic!("revoked disclosure cannot authorize encounter"),
            )
            .is_err()
        );
        assert_eq!(changed, before);
        assert_eq!(
            changed
                .state()
                .decisions
                .last()
                .unwrap()
                .source_policy
                .as_str(),
            THREAD_POLICY
        );
        assert_eq!(
            accepted(changed.state().decisions.last().unwrap())
                .unwrap()
                .phase,
            rpc::JourneyPhase::Dialogue as i32
        );
    }
}

use super::super::{
    MEMBERS, bootstrap_member, offered, phase, stage_with_supplier,
    tests::{input, opening_story, prepared_story},
};
use super::*;
use df_narrative::{BeatSelectionError, stage_checkpoint_beat_selection};
use df_types::MemberId;

#[path = "narrative_transition_session_tests.rs"]
mod session_tests;

#[path = "narrative_budget_tests.rs"]
mod budget_tests;

fn first() -> MemberId {
    MemberId::from_bytes(&MEMBERS[0]).unwrap()
}
fn second() -> MemberId {
    MemberId::from_bytes(&MEMBERS[1]).unwrap()
}
fn run(current: &Checkpoint, command: &GameInput) -> Checkpoint {
    stage_with_supplier(current, command, &mut |_| {
        panic!("story alternatives do not draw")
    })
    .unwrap()
}
fn refuses(current: &Checkpoint, command: &GameInput) {
    let unchanged = current.clone();
    assert!(stage_with_supplier(current, command, &mut |_| panic!("refusal cannot draw")).is_err());
    assert_eq!(current, &unchanged);
}

#[test]
fn registered_story_selector_consumes_current_authored_alternatives_and_retains_exact_causes() {
    for (current, action, next, event) in [
        (prepared_story(), "begin-story", "opening", "begin-story"),
        (
            opening_story(),
            "ask-courier",
            "courier-answer-seal",
            "private-courier-note",
        ),
        (
            opening_story(),
            "escort-courier",
            "courier-answer-escort",
            "escort-courier",
        ),
    ] {
        for member in [first(), second()] {
            let command = input(&current, member, 40, action, vec![]);
            let unchanged = current.clone();
            let selected = select(&current, &command).unwrap();
            assert_eq!(selected.basis(), current.basis());
            assert_eq!(selected.pins(), current.pins());
            assert_eq!(
                selected.state().narrative.active_beats,
                [model::content(next).unwrap()]
            );
            let mut completed = current.state().narrative.completed_beats.clone();
            completed.extend(current.state().narrative.active_beats.clone());
            assert_eq!(selected.state().narrative.completed_beats, completed);
            assert_eq!(
                selected.state().narrative.accepted_facts,
                current.state().narrative.accepted_facts
            );
            let mut protected = selected.state().clone();
            protected.narrative = current.state().narrative.clone();
            assert_eq!(protected, *current.state());
            let accepted = run(&current, &command);
            assert_eq!(
                accepted.state().narrative.active_beats,
                selected.state().narrative.active_beats
            );
            assert_eq!(
                accepted.state().narrative.completed_beats,
                selected.state().narrative.completed_beats
            );
            assert_eq!(
                accepted.state().narrative.open_threads,
                [model::content(PACKET_THREAD).unwrap()]
            );
            let source = accepted.state().facts.last().unwrap();
            assert!(
                matches!(&source.value, FactValue::ContentEvent { definition, subjects }
                if *definition == model::content(event).unwrap() && subjects.is_empty())
            );
            let prior = &accepted.state().facts[accepted.state().facts.len() - 2];
            assert_eq!(source.cause, Some(prior.id));
            assert_eq!(
                source.audience,
                if action == "ask-courier" {
                    AudienceScope::Members(vec![member])
                } else {
                    AudienceScope::Shared
                }
            );
            let decision = accepted.state().decisions.last().unwrap();
            assert_eq!(
                decision.facts.get(source.ordinal as usize),
                Some(&source.id)
            );
            assert_eq!(decision.operation, source.operation);
            assert_eq!(decision.revision, source.revision);
            assert_eq!(
                accepted.state().narrative.accepted_facts.last(),
                Some(&source.id)
            );
            assert_eq!(
                &accepted.state().facts[..current.state().facts.len()],
                current.state().facts
            );
            assert_eq!(
                &accepted.state().decisions[..current.state().decisions.len()],
                current.state().decisions
            );
            assert_eq!(accepted.state().draws, current.state().draws);
            assert_eq!(current, unchanged);
            phase(&accepted).unwrap();
            assert_eq!(select(&current, &command).unwrap(), selected);
        }
    }
}

#[test]
fn registered_story_selector_refuses_missing_foreign_or_replaced_creation_sources_before_draws() {
    let current = prepared_story();
    let create = model::content("create-character").unwrap();
    let source = current.state().facts.iter().find(|fact|
        matches!(&fact.value, FactValue::ContentEvent { definition, .. } if *definition == create)
    ).unwrap();
    let operation = source.operation;
    for case in 0..6 {
        let mut state = current.state().clone();
        let owner = state
            .decisions
            .iter()
            .position(|decision| decision.operation == operation)
            .unwrap();
        match case {
            0 => {
                state.decisions.remove(owner);
            }
            1 => {
                state.decisions[owner].source_policy = model::label("foreign-story-policy").unwrap()
            }
            2 => state.decisions[owner].facts.clear(),
            3 => {
                let fact = state
                    .facts
                    .iter_mut()
                    .find(|fact| fact.id == source.id)
                    .unwrap();
                fact.value = FactValue::ContentEvent {
                    definition: model::content("opening").unwrap(),
                    subjects: vec![],
                };
            }
            4 => {
                state
                    .facts
                    .iter_mut()
                    .find(|fact| fact.id == source.id)
                    .unwrap()
                    .audience = AudienceScope::Host
            }
            5 => state.decisions[owner].semantic_output = None,
            _ => unreachable!(),
        }
        let changed = model::checkpoint(current.basis(), state).unwrap();
        assert_eq!(phase(&changed).unwrap(), rpc::JourneyPhase::Room);
        assert!(
            !story_permitted(&changed, first()).unwrap(),
            "creation case {case}"
        );
        assert!(
            offered(&changed, first()).unwrap().is_empty(),
            "creation case {case}"
        );
        refuses(
            &changed,
            &input(&changed, first(), 41, "begin-story", vec![]),
        );
    }
}

#[test]
fn registered_story_selector_refuses_wrong_command_beat_thread_and_source_pins() {
    let room = prepared_story();
    let opening = opening_story();
    refuses(&room, &input(&room, first(), 42, "ask-courier", vec![]));
    refuses(
        &opening,
        &input(&opening, first(), 42, "begin-story", vec![]),
    );
    let mut state = opening.state().clone();
    state.narrative.open_threads.clear();
    let missing = model::checkpoint(opening.basis(), state).unwrap();
    refuses(
        &missing,
        &input(&missing, first(), 42, "escort-courier", vec![]),
    );
    let mut state = opening.state().clone();
    state.narrative.accepted_facts.clear();
    let omitted = model::checkpoint(opening.basis(), state).unwrap();
    refuses(
        &omitted,
        &input(&omitted, first(), 42, "ask-courier", vec![]),
    );
    let mut command = input(&opening, first(), 42, "ask-courier", vec![]);
    let GameInput::Game(value) = &mut command else {
        panic!("command")
    };
    value.observed_revision = value.observed_revision.next_sequence().unwrap();
    refuses(&opening, &command);
    assert!(select(&opening, &command).is_err());
    let mut pins = opening.pins().clone();
    pins.content.package_digest = ContentDigest([9; 32]);
    let stale = Checkpoint::new(
        opening.schema(),
        opening.basis(),
        pins,
        opening.state().clone(),
        ReferenceInventory {
            rules: &[model::rule().unwrap(), rule().unwrap()],
            content: &model::contents().unwrap(),
            resources: &super::super::resources_for(opening.state()).unwrap(),
            assets: &[],
        },
        model::limits(),
    )
    .unwrap();
    refuses(&stale, &input(&stale, first(), 42, "ask-courier", vec![]));
}

#[test]
fn checkpoint_story_selector_enforces_current_recipient_owner_and_bounded_admission() {
    let opening = opening_story();
    let from = model::content("opening").unwrap();
    let to = model::content("courier-answer-seal").unwrap();
    let selection = model::content("ask-courier").unwrap();
    let event = model::content("begin-story").unwrap();
    let source = opening.state().facts.last().unwrap().id;
    let threads = [model::content(PACKET_THREAD).unwrap()];
    let policy = model::label(THREAD_POLICY).unwrap();
    for case in 0..11 {
        let mut state = opening.state().clone();
        let mut recipient = first();
        let mut cause = source;
        let mut expected = policy.clone();
        let mut checkpoint_limits = model::limits();
        let mut limits = BeatSelectionLimits {
            records: 512,
            alternatives: 2,
            work: 1024 * 1024,
        };
        let expected_error = match case {
            0 => {
                let operation = state.facts.last().unwrap().operation;
                state.decisions.retain(|owner| owner.operation != operation);
                BeatSelectionError::Source
            }
            1 => {
                cause = FactId::from_bytes(&[99; 16]).unwrap();
                BeatSelectionError::Source
            }
            2 => {
                recipient = bootstrap_member().unwrap();
                BeatSelectionError::Recipient
            }
            3 => {
                expected = model::label("changed-narrative-policy").unwrap();
                BeatSelectionError::StalePolicy
            }
            4 => {
                limits.work = 0;
                BeatSelectionError::Capacity
            }
            5 => {
                limits.alternatives = 0;
                BeatSelectionError::Capacity
            }
            6 => {
                state.facts.last_mut().unwrap().audience = AudienceScope::Host;
                BeatSelectionError::Recipient
            }
            7 => {
                state.facts.last_mut().unwrap().audience = AudienceScope::Members(vec![second()]);
                state.knowledge.push(KnowledgeGrant {
                    observer: first(),
                    fact: source,
                    source,
                });
                BeatSelectionError::Recipient
            }
            8 => {
                state.decisions.last_mut().unwrap().facts.clear();
                BeatSelectionError::Source
            }
            9 => {
                limits.records = 1;
                BeatSelectionError::Capacity
            }
            10 => {
                checkpoint_limits.maximum_records = 1;
                BeatSelectionError::InvalidCandidate(CheckpointError::Capacity)
            }
            _ => unreachable!(),
        };
        let current = model::checkpoint(opening.basis(), state).unwrap();
        let unchanged = current.clone();
        let causes = [BeatCause {
            fact: cause,
            event: &event,
            consumed_by_narrative: true,
        }];
        let alternatives = [AdmittedBeatAlternative {
            selection: &selection,
            from: &from,
            to: &to,
            causes: &causes,
            required_threads: &threads,
            opened_thread: None,
        }];
        let result = stage_checkpoint_beat_selection(
            &current,
            CheckpointBeatRequest {
                expected_basis: current.basis(),
                admitted_pins: current.pins(),
                policy: &policy,
                expected_policy: &expected,
                recipient,
                selection: &selection,
                alternatives: &alternatives,
                inventory: ReferenceInventory {
                    rules: &[model::rule().unwrap(), rule().unwrap()],
                    content: &model::contents().unwrap(),
                    resources: &super::super::resources_for(current.state()).unwrap(),
                    assets: &[],
                },
                checkpoint_limits,
            },
            limits,
        );
        assert_eq!(result, Err(expected_error), "bounded recipient case {case}");
        assert_eq!(current, unchanged);
    }
}

#[test]
fn stale_narrative_request_records_observed_basis_mismatch_without_state_change() {
    let current = prepared_story();
    let mut request = input(&current, first(), 40, "begin-story", vec![]);
    let GameInput::Game(command) = &mut request else {
        panic!("game command");
    };
    command.observed_revision = current.basis().revision.next_sequence().unwrap();
    let _ = df_session::submission::take_refusal_diagnostic();
    assert!(select(&current, &request).is_err());
    let detail = df_session::submission::take_refusal_diagnostic().unwrap();
    assert_eq!(detail.step, "narrative-transition");
    assert_eq!(detail.cause, "ObservedBasisMismatch");
}

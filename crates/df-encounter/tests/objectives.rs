use std::cell::Cell;

use df_encounter::objectives::{
    AuthoredObjectiveTransition, ObjectivePolicyError, ObjectivePolicyOwner, ObjectiveProposal,
    ObjectiveProposalError, ObjectiveProposalLimits, ObjectiveProposalRequest,
    propose_objective_transitions,
};

include!("objectives/fixture.rs");

fn replacement() -> ContentReference {
    ContentReference {
        package: content().package,
        entry: label("fixture-authored-next-objective"),
    }
}

fn policy() -> ContentReference {
    ContentReference {
        package: content().package,
        entry: label("fixture-authored-objective-policy"),
    }
}

fn definitions() -> Vec<ContentReference> {
    vec![content(), replacement(), policy()]
}

fn encounter(value: u8) -> EncounterState {
    EncounterState {
        id: RecordId::from_bytes(&[value; 16]).unwrap(),
        definition: content(),
        participants: vec![entity(4)],
        turn_order: vec![entity(4)],
        active_turn: Some(entity(4)),
        objectives: vec![content(), content()],
        combat_policy: content(),
    }
}

fn current(edit: impl FnOnce(&mut GameState)) -> Checkpoint {
    let mut state = state();
    state.encounters = vec![encounter(12), encounter(13)];
    let mut private = fact(7, 0);
    private.audience = AudienceScope::Members(vec![member(3)]);
    state.facts = vec![private, fact(8, 1)];
    state.decisions = vec![AcceptedDecision {
        operation: state.facts.first().unwrap().operation,
        revision: basis().revision,
        facts: state.facts.iter().map(|fact| fact.id).collect(),
        draws: vec![],
        effects: vec![],
        source_policy: label("fixture-committed-source-policy"),
        semantic_output: None,
    }];
    edit(&mut state);
    checkpoint(state).unwrap()
}

// Synthetic authored admission fixture; it implements no rulebook mechanic or real grant.
struct Authority {
    basis: Basis,
    pins: CheckpointPins,
    expected: ContentReference,
    replacement: ContentReference,
    policy: ContentReference,
    source: RuleReference,
    encounter: RecordId,
    objective_indices: Vec<usize>,
    authored_cause: GameFact,
    rejection: Option<ObjectivePolicyError>,
    calls: Cell<usize>,
}

impl Authority {
    fn new(cause: &GameFact) -> Self {
        Self {
            basis: basis(),
            pins: pins(),
            expected: content(),
            replacement: replacement(),
            policy: policy(),
            source: rule(),
            authored_cause: cause.clone(),
            rejection: None,
            encounter: encounter(12).id,
            objective_indices: vec![0, 1],
            calls: Cell::new(0),
        }
    }
}

impl ObjectivePolicyOwner for Authority {
    fn admit_transition(
        &self,
        current: &Checkpoint,
        transition: &AuthoredObjectiveTransition<'_>,
    ) -> Result<(), ObjectivePolicyError> {
        self.calls.set(self.calls.get() + 1);
        if let Some(error) = self.rejection {
            return Err(error);
        }
        if current.basis() != self.basis
            || current.pins() != &self.pins
            || transition.expected != &self.expected
            || transition.replacement != &self.replacement
            || transition.policy != &self.policy
            || transition.source != &self.source
            || transition.encounter != self.encounter
            || !self.objective_indices.contains(&transition.objective_index)
            || transition.cause != &self.authored_cause
        {
            return Err(ObjectivePolicyError::PolicyMismatch);
        }
        if transition.actor != entity(4) {
            return Err(ObjectivePolicyError::ActorDenied);
        }
        Ok(())
    }
}

fn proposal_limits() -> ObjectiveProposalLimits {
    ObjectiveProposalLimits {
        maximum_changes: 4,
        maximum_records: 100,
        maximum_comparisons: 1_000,
        maximum_input_bytes: 1024 * 1024,
        maximum_output_bytes: 1024 * 1024,
    }
}

fn transition<'a>(
    authority: &'a Authority,
    cause: &'a GameFact,
) -> AuthoredObjectiveTransition<'a> {
    AuthoredObjectiveTransition {
        encounter: encounter(12).id,
        objective_index: 0,
        expected: &authority.expected,
        replacement: &authority.replacement,
        policy: &authority.policy,
        source: &authority.source,
        actor: entity(4),
        cause,
    }
}

fn run<'a>(
    authority: &impl ObjectivePolicyOwner,
    current: &'a Checkpoint,
    transitions: &'a [AuthoredObjectiveTransition<'a>],
    inventory: ReferenceInventory<'a>,
    bounded: ObjectiveProposalLimits,
) -> Result<ObjectiveProposal<'a>, ObjectiveProposalError> {
    propose_objective_transitions(
        authority,
        ObjectiveProposalRequest {
            current,
            expected_basis: current.basis(),
            admitted_pins: current.pins(),
            inventory,
            transitions,
            checkpoint_limits: limits(),
            limits: bounded,
        },
    )
}

fn inventory<'a>(
    rules: &'a [RuleReference],
    content: &'a [ContentReference],
    resources: &'a [ResourceConstraint],
) -> ReferenceInventory<'a> {
    ReferenceInventory {
        rules,
        content,
        resources,
        assets: &[],
    }
}

#[test]
fn committed_authored_transition_preserves_exact_canonical_siblings_and_private_evidence() {
    let current = current(|_| {});
    let before = current.clone();
    let cause = current.state().facts.first().unwrap();
    let authority = Authority::new(cause);
    let changes = [transition(&authority, cause)];
    let rules = [rule()];
    let content = definitions();
    let resources = resource_constraints();
    let proposal = run(
        &authority,
        &current,
        &changes,
        inventory(&rules, &content, &resources),
        proposal_limits(),
    )
    .unwrap();
    let mut expected = current.state().clone();
    *expected
        .encounters
        .first_mut()
        .unwrap()
        .objectives
        .first_mut()
        .unwrap() = replacement();
    assert_eq!(proposal.checkpoint.state(), &expected);
    assert_eq!(proposal.checkpoint.basis(), current.basis());
    assert_eq!(proposal.checkpoint.pins(), current.pins());
    assert_eq!(proposal.checkpoint.schema(), current.schema());
    let evidence = proposal.evidence.first().unwrap();
    assert!(std::ptr::eq(evidence.cause, cause));
    assert!(std::ptr::eq(
        evidence.decision,
        current.state().decisions.first().unwrap()
    ));
    assert_eq!(
        evidence.cause.audience,
        AudienceScope::Members(vec![member(3)])
    );
    assert_eq!(evidence.encounter, encounter(12).id);
    assert_eq!(evidence.objective_index, 0);
    assert_eq!(evidence.policy, &policy());
    assert_eq!(evidence.source, &rule());
    assert_eq!(current, before);
    assert_eq!(authority.calls.get(), 1);
}

#[test]
fn stale_session_run_revision_and_all_pins_reject_before_source_admission() {
    let current = current(|_| {});
    let cause = current.state().facts.first().unwrap();
    let authority = Authority::new(cause);
    let changes = [transition(&authority, cause)];
    let rules = [rule()];
    let content = definitions();
    let resources = resource_constraints();
    for variant in 0..6 {
        let mut expected = current.basis();
        let mut admitted = current.pins().clone();
        match variant {
            0 => expected.session = SessionId::from_bytes(&[20; 16]).unwrap(),
            1 => expected.run = RunId::from_bytes(&[20; 16]).unwrap(),
            2 => expected.revision = expected.revision.next_sequence().unwrap(),
            3 => admitted.content.package_digest = ContentDigest([20; 32]),
            4 => admitted.rules.handler_digest = ContentDigest([20; 32]),
            _ => {
                admitted.build = BuildIdentity::new(
                    Some("other"),
                    Some("native"),
                    Some("wasm"),
                    Some("config"),
                    Some("content"),
                )
                .unwrap()
            }
        }
        let error = propose_objective_transitions(
            &authority,
            ObjectiveProposalRequest {
                current: &current,
                expected_basis: expected,
                admitted_pins: &admitted,
                inventory: inventory(&rules, &content, &resources),
                transitions: &changes,
                checkpoint_limits: limits(),
                limits: proposal_limits(),
            },
        )
        .err();
        assert!(matches!(error, Some(ObjectiveProposalError::Snapshot(_))));
    }
    assert_eq!(authority.calls.get(), 0);
}

#[test]
fn current_source_rights_actor_and_policy_admission_cannot_be_replaced_by_inventory_presence() {
    let current = current(|_| {});
    let before = current.clone();
    let cause = current.state().facts.first().unwrap();
    let rules = [rule()];
    let content = definitions();
    let resources = resource_constraints();
    for rejection in [
        ObjectivePolicyError::UnqualifiedSource,
        ObjectivePolicyError::RightsDenied,
        ObjectivePolicyError::ActorDenied,
        ObjectivePolicyError::PolicyMismatch,
    ] {
        let authority = Authority {
            rejection: Some(rejection),
            ..Authority::new(cause)
        };
        let changes = [transition(&authority, cause)];
        let error = run(
            &authority,
            &current,
            &changes,
            inventory(&rules, &content, &resources),
            proposal_limits(),
        )
        .err();
        assert_eq!(error, Some(ObjectiveProposalError::Policy(rejection)));
        assert_eq!(current, before);
    }
}

#[test]
fn foreign_or_mutated_cause_identity_operation_revision_subjects_and_audience_are_not_canonical() {
    let current = current(|_| {});
    let before = current.clone();
    let canonical = current.state().facts.first().unwrap();
    let rules = [rule()];
    let content = definitions();
    let resources = resource_constraints();
    for variant in 0..7 {
        let mut supplied = canonical.clone();
        match variant {
            0 => supplied.id = FactId::from_bytes(&[21; 16]).unwrap(),
            1 => supplied.operation = OperationId::from_bytes(&[21; 16]).unwrap(),
            2 => supplied.revision = supplied.revision.next_sequence().unwrap(),
            3 => {
                if let FactValue::ContentEvent { subjects, .. } = &mut supplied.value {
                    subjects.clear();
                }
            }
            4 => supplied.audience = AudienceScope::Shared,
            5 => supplied.audience = AudienceScope::Members(vec![member(3), member(26)]),
            _ => supplied.cause = Some(fact(8, 1).id),
        }
        // A source-mapped event descriptor still cannot substitute for canonical history.
        let authority = Authority::new(&supplied);
        let changes = [transition(&authority, &supplied)];
        let error = run(
            &authority,
            &current,
            &changes,
            inventory(&rules, &content, &resources),
            proposal_limits(),
        )
        .err();
        assert_eq!(error, Some(ObjectiveProposalError::ForeignCause));
        assert_eq!(current, before);
    }
}

#[test]
fn fact_presence_without_an_accepted_decision_receipt_cannot_change_objectives() {
    for remove_receipt in [false, true] {
        let current = current(|state| {
            if remove_receipt {
                state.decisions.clear();
            } else {
                state.decisions.first_mut().unwrap().facts.clear();
            }
        });
        let before = current.clone();
        let cause = current.state().facts.first().unwrap();
        let authority = Authority::new(cause);
        let changes = [transition(&authority, cause)];
        let rules = [rule()];
        let content = definitions();
        let resources = resource_constraints();
        let error = run(
            &authority,
            &current,
            &changes,
            inventory(&rules, &content, &resources),
            proposal_limits(),
        )
        .err();
        assert_eq!(error, Some(ObjectiveProposalError::UncommittedCause));
        assert_eq!(current, before);
    }
}

#[test]
fn a_bare_time_advance_never_becomes_an_authored_objective_outcome() {
    let current = current(|state| {
        state.facts.first_mut().unwrap().value = FactValue::TimeAdvanced {
            before: LogicalTime {
                ticks: 100,
                ticks_per_second: 10,
            },
            after: LogicalTime {
                ticks: 120,
                ticks_per_second: 10,
            },
        }
    });
    let cause = current.state().facts.first().unwrap();
    let authority = Authority::new(cause);
    let changes = [transition(&authority, cause)];
    let rules = [rule()];
    let content = definitions();
    let resources = resource_constraints();
    let error = run(
        &authority,
        &current,
        &changes,
        inventory(&rules, &content, &resources),
        proposal_limits(),
    )
    .err();
    assert_eq!(error, Some(ObjectiveProposalError::UnsupportedCause));
    assert_eq!(authority.calls.get(), 0);
}

#[test]
fn event_subjects_must_contain_the_exact_encounter_actor() {
    let current = current(|state| {
        if let FactValue::ContentEvent { subjects, .. } =
            &mut state.facts.first_mut().unwrap().value
        {
            subjects.clear();
        }
    });
    let before = current.clone();
    let cause = current.state().facts.first().unwrap();
    let authority = Authority::new(cause);
    let changes = [transition(&authority, cause)];
    let rules = [rule()];
    let content = definitions();
    let resources = resource_constraints();
    assert_eq!(
        run(
            &authority,
            &current,
            &changes,
            inventory(&rules, &content, &resources),
            proposal_limits()
        )
        .err(),
        Some(ObjectiveProposalError::ActorMismatch)
    );
    assert_eq!(current, before);
}

#[test]
fn source_revocation_and_unadmitted_inventory_references_remain_explicit_gaps() {
    let rules = [rule()];
    let content = definitions();
    let resources = resource_constraints();
    for variant in 0..3 {
        let current = current(|state| {
            if variant == 0 {
                state
                    .continuity
                    .recovery
                    .unavailable_sources
                    .push(rule().source);
            }
        });
        let before = current.clone();
        let cause = current.state().facts.first().unwrap();
        let authority = Authority::new(cause);
        let changes = [transition(&authority, cause)];
        let admitted_rules = if variant == 1 { &[][..] } else { &rules[..] };
        let admitted_content = if variant == 2 {
            &content[..1]
        } else {
            &content[..]
        };
        let expected = match variant {
            0 => ObjectiveProposalError::Snapshot(CheckpointError::UnavailableCheckpoint),
            1 => ObjectiveProposalError::UnadmittedSource,
            _ => ObjectiveProposalError::UnadmittedContent,
        };
        assert_eq!(
            run(
                &authority,
                &current,
                &changes,
                inventory(admitted_rules, admitted_content, &resources),
                proposal_limits()
            )
            .err(),
            Some(expected)
        );
        if variant == 0 {
            assert_eq!(authority.calls.get(), 0);
        }
        assert_eq!(current, before);
    }
}

#[test]
fn missing_stale_conflicting_and_no_op_objective_targets_reject_the_complete_batch() {
    let current = current(|_| {});
    let before = current.clone();
    let cause = current.state().facts.first().unwrap();
    let rules = [rule()];
    let content = definitions();
    let resources = resource_constraints();
    for variant in 0..5 {
        let mut authority = Authority::new(cause);
        if variant == 4 {
            authority.expected = replacement();
        }
        if variant == 0 {
            authority.encounter = encounter(22).id;
        }
        if variant == 1 {
            authority.objective_indices = vec![usize::MAX];
        }
        let mut changes = vec![transition(&authority, cause)];
        match variant {
            0 => changes.first_mut().unwrap().encounter = encounter(22).id,
            1 => changes.first_mut().unwrap().objective_index = usize::MAX,
            2 => changes.first_mut().unwrap().expected = &authority.replacement,
            3 => changes.push(transition(&authority, cause)),
            _ => {}
        }
        let expected = match variant {
            0 => ObjectiveProposalError::UnknownEncounter,
            1 => ObjectiveProposalError::UnknownObjective,
            2 => ObjectiveProposalError::Policy(ObjectivePolicyError::PolicyMismatch),
            3 => ObjectiveProposalError::ConflictingChanges,
            _ => ObjectiveProposalError::UnchangedObjective,
        };
        assert_eq!(
            run(
                &authority,
                &current,
                &changes,
                inventory(&rules, &content, &resources),
                proposal_limits()
            )
            .err(),
            Some(expected)
        );
        assert_eq!(current, before);
    }
    let mut authority = Authority::new(cause);
    authority.objective_indices.push(usize::MAX);
    let mut changes = [transition(&authority, cause), transition(&authority, cause)];
    changes.get_mut(1).unwrap().objective_index = usize::MAX;
    assert_eq!(
        run(
            &authority,
            &current,
            &changes,
            inventory(&rules, &content, &resources),
            proposal_limits()
        )
        .err(),
        Some(ObjectiveProposalError::UnknownObjective)
    );
    assert_eq!(current, before);
}

#[test]
fn full_count_work_input_output_and_text_limits_never_publish_partial_changes() {
    let current = current(|_| {});
    let before = current.clone();
    let cause = current.state().facts.first().unwrap();
    let authority = Authority::new(cause);
    let changes = [transition(&authority, cause)];
    let rules = [rule()];
    let content = definitions();
    let resources = resource_constraints();
    for bounded in [
        ObjectiveProposalLimits {
            maximum_changes: 0,
            ..proposal_limits()
        },
        ObjectiveProposalLimits {
            maximum_records: 0,
            ..proposal_limits()
        },
        ObjectiveProposalLimits {
            maximum_comparisons: 1,
            ..proposal_limits()
        },
        ObjectiveProposalLimits {
            maximum_input_bytes: current.retained_bytes().unwrap(),
            ..proposal_limits()
        },
        ObjectiveProposalLimits {
            maximum_output_bytes: current.retained_bytes().unwrap(),
            ..proposal_limits()
        },
    ] {
        assert_eq!(
            run(
                &authority,
                &current,
                &changes,
                inventory(&rules, &content, &resources),
                bounded
            )
            .err(),
            Some(ObjectiveProposalError::Capacity)
        );
        assert_eq!(current, before);
    }
    let mut checkpoint_limits = limits();
    checkpoint_limits.maximum_text_bytes = 1;
    assert_eq!(
        propose_objective_transitions(
            &authority,
            ObjectiveProposalRequest {
                current: &current,
                expected_basis: current.basis(),
                admitted_pins: current.pins(),
                inventory: inventory(&rules, &content, &resources),
                transitions: &changes,
                checkpoint_limits,
                limits: proposal_limits(),
            }
        )
        .err(),
        Some(ObjectiveProposalError::Capacity)
    );
    assert_eq!(current, before);
}

#[test]
fn request_payload_admission_precedes_every_policy_callback() {
    let current = current(|_| {});
    let cause = current.state().facts.first().unwrap();
    let authority = Authority::new(cause);
    let changes = [transition(&authority, cause)];
    let rules = [rule()];
    let content = definitions();
    let resources = resource_constraints();
    let bounded = ObjectiveProposalLimits {
        maximum_input_bytes: current.retained_bytes().unwrap(),
        ..proposal_limits()
    };
    assert_eq!(
        run(
            &authority,
            &current,
            &changes,
            inventory(&rules, &content, &resources),
            bounded
        )
        .err(),
        Some(ObjectiveProposalError::Capacity)
    );
    assert_eq!(authority.calls.get(), 0);
}

#[test]
fn irrelevant_hidden_beliefs_do_not_change_the_authored_visible_transition_or_audiences() {
    let rules = [rule()];
    let content_entries = definitions();
    let resources = resource_constraints();
    for secret in ["hidden-one", "hidden-two"] {
        let current = current(|state| {
            state.beliefs.push(AttributedClaim {
                id: RecordId::from_bytes(&[25; 16]).unwrap(),
                holder: entity(4),
                subject: entity(4),
                claim: secret.to_owned(),
                evidence: vec![fact(8, 1).id],
                audience: AudienceScope::Host,
                source: content(),
            })
        });
        let before = current.clone();
        let cause = current.state().facts.first().unwrap();
        let authority = Authority::new(cause);
        let changes = [transition(&authority, cause)];
        let proposal = run(
            &authority,
            &current,
            &changes,
            inventory(&rules, &content_entries, &resources),
            proposal_limits(),
        )
        .unwrap();
        assert_eq!(
            proposal
                .checkpoint
                .state()
                .encounters
                .first()
                .unwrap()
                .objectives
                .first()
                .unwrap(),
            &replacement()
        );
        assert_eq!(proposal.checkpoint.state().beliefs, current.state().beliefs);
        assert_eq!(
            proposal.evidence.first().unwrap().cause.audience,
            AudienceScope::Members(vec![member(3)])
        );
        assert_eq!(current, before);
    }
}

#[test]
fn intervening_objective_reference_change_refuses_an_otherwise_authored_transition() {
    let current = current(|state| {
        *state
            .encounters
            .first_mut()
            .unwrap()
            .objectives
            .first_mut()
            .unwrap() = replacement();
    });
    let before = current.clone();
    let cause = current.state().facts.first().unwrap();
    let authority = Authority::new(cause);
    let changes = [transition(&authority, cause)];
    let rules = [rule()];
    let content = definitions();
    let resources = resource_constraints();
    assert_eq!(
        run(
            &authority,
            &current,
            &changes,
            inventory(&rules, &content, &resources),
            proposal_limits()
        )
        .err(),
        Some(ObjectiveProposalError::StaleObjective)
    );
    assert_eq!(current, before);
}

#[test]
fn two_authored_changes_preserve_objective_order_and_exact_input_evidence_order() {
    let current = current(|_| {});
    let before = current.clone();
    let cause = current.state().facts.first().unwrap();
    let authority = Authority::new(cause);
    let mut changes = [transition(&authority, cause), transition(&authority, cause)];
    changes.first_mut().unwrap().objective_index = 1;
    let rules = [rule()];
    let content = definitions();
    let resources = resource_constraints();
    let proposal = run(
        &authority,
        &current,
        &changes,
        inventory(&rules, &content, &resources),
        proposal_limits(),
    )
    .unwrap();
    let mut expected = current.state().clone();
    expected.encounters.first_mut().unwrap().objectives = vec![replacement(), replacement()];
    assert_eq!(proposal.checkpoint.state(), &expected);
    assert_eq!(proposal.evidence.len(), 2);
    assert_eq!(
        proposal
            .evidence
            .iter()
            .map(|evidence| evidence.objective_index)
            .collect::<Vec<_>>(),
        vec![1, 0]
    );
    for evidence in &proposal.evidence {
        assert!(std::ptr::eq(evidence.cause, cause));
        assert!(std::ptr::eq(
            evidence.decision,
            current.state().decisions.first().unwrap()
        ));
    }
    assert_eq!(current, before);
}

#[test]
fn an_actor_in_the_committed_event_but_outside_the_current_encounter_is_rejected() {
    let current = current(|state| {
        let target = state.encounters.first_mut().unwrap();
        target.participants.clear();
        target.turn_order.clear();
        target.active_turn = None;
    });
    let before = current.clone();
    let cause = current.state().facts.first().unwrap();
    let authority = Authority::new(cause);
    let changes = [transition(&authority, cause)];
    let rules = [rule()];
    let content = definitions();
    let resources = resource_constraints();
    assert_eq!(
        run(
            &authority,
            &current,
            &changes,
            inventory(&rules, &content, &resources),
            proposal_limits()
        )
        .err(),
        Some(ObjectiveProposalError::ActorMismatch)
    );
    assert_eq!(authority.calls.get(), 1);
    assert_eq!(current, before);
}

struct EndStateAuthority {
    mapping: Authority,
    admission: Result<bool, ObjectivePolicyError>,
    calls: Cell<usize>,
}

impl ObjectivePolicyOwner for EndStateAuthority {
    fn admit_transition(
        &self,
        current: &Checkpoint,
        transition: &AuthoredObjectiveTransition<'_>,
    ) -> Result<(), ObjectivePolicyError> {
        self.mapping.admit_transition(current, transition)
    }

    fn admit_end_state(
        &self,
        current: &Checkpoint,
        target: &EncounterState,
        transitions: &[AuthoredObjectiveTransition<'_>],
    ) -> Result<bool, ObjectivePolicyError> {
        self.calls.set(self.calls.get() + 1);
        if current.basis() != self.mapping.basis
            || target != &encounter(12)
            || transitions.len() != 2
            || ![0, 1].into_iter().all(|index| {
                transitions.iter().any(|transition| {
                    transition.encounter == target.id
                        && transition.objective_index == index
                        && transition.replacement == &self.mapping.replacement
                })
            })
        {
            return Err(ObjectivePolicyError::PolicyMismatch);
        }
        self.admission
    }
}

#[test]
fn source_reviewed_success_failure_and_escape_end_only_the_affected_encounter() {
    for outcome in ["authored-success", "authored-failure", "authored-escape"] {
        let current = current(|_| {});
        let before = current.clone();
        let cause = &current.state().facts[0];
        let mut mapping = Authority::new(cause);
        mapping.replacement.entry = label(outcome);
        let authority = EndStateAuthority {
            mapping,
            admission: Ok(true),
            calls: Cell::new(0),
        };
        let mut changes = [
            transition(&authority.mapping, cause),
            transition(&authority.mapping, cause),
        ];
        changes[0].objective_index = 1;
        let rules = [rule()];
        let content = [content(), authority.mapping.replacement.clone(), policy()];
        let resources = resource_constraints();
        let proposal = run(
            &authority,
            &current,
            &changes,
            inventory(&rules, &content, &resources),
            proposal_limits(),
        )
        .unwrap();
        let mut expected = current.state().clone();
        let target = &mut expected.encounters[0];
        target.objectives = vec![authority.mapping.replacement.clone(); 2];
        target.turn_order.clear();
        target.active_turn = None;
        assert_eq!(proposal.checkpoint.state(), &expected);
        assert_eq!(proposal.evidence.len(), 2);
        assert_eq!(authority.calls.get(), 1);
        assert_eq!(current, before);
    }
}

#[test]
fn continuing_and_empty_batches_preserve_turns_without_inferred_label_meaning() {
    let current = current(|_| {});
    let cause = &current.state().facts[0];
    let authority = EndStateAuthority {
        mapping: Authority::new(cause),
        admission: Ok(false),
        calls: Cell::new(0),
    };
    let mut changes = [
        transition(&authority.mapping, cause),
        transition(&authority.mapping, cause),
    ];
    changes[1].objective_index = 1;
    let rules = [rule()];
    let content = definitions();
    let resources = resource_constraints();
    let proposal = run(
        &authority,
        &current,
        &changes,
        inventory(&rules, &content, &resources),
        proposal_limits(),
    )
    .unwrap();
    assert_eq!(
        proposal.checkpoint.state().encounters[0].turn_order,
        current.state().encounters[0].turn_order
    );
    assert_eq!(
        proposal.checkpoint.state().encounters[0].active_turn,
        current.state().encounters[0].active_turn
    );
    let empty = run(
        &authority,
        &current,
        &[],
        inventory(&rules, &content, &resources),
        proposal_limits(),
    )
    .unwrap();
    assert_eq!(empty.checkpoint, current);
    assert_eq!(authority.calls.get(), 1);
}

#[test]
fn denied_terminal_admission_discards_both_objectives_and_all_turns() {
    let current = current(|_| {});
    let before = current.clone();
    let cause = &current.state().facts[0];
    let authority = EndStateAuthority {
        mapping: Authority::new(cause),
        admission: Err(ObjectivePolicyError::RightsDenied),
        calls: Cell::new(0),
    };
    let mut changes = [
        transition(&authority.mapping, cause),
        transition(&authority.mapping, cause),
    ];
    changes[1].objective_index = 1;
    let rules = [rule()];
    let content = definitions();
    let resources = resource_constraints();
    assert_eq!(
        run(
            &authority,
            &current,
            &changes,
            inventory(&rules, &content, &resources),
            proposal_limits(),
        )
        .err(),
        Some(ObjectiveProposalError::Policy(
            ObjectivePolicyError::RightsDenied
        ))
    );
    assert_eq!(authority.mapping.calls.get(), 2);
    assert_eq!(authority.calls.get(), 1);
    assert_eq!(current, before);
}

#[test]
fn invalid_tail_and_bounded_requests_never_reach_terminal_admission() {
    for case in ["stale-tail", "count", "work", "input", "output"] {
        let current = current(|_| {});
        let before = current.clone();
        let cause = &current.state().facts[0];
        let authority = EndStateAuthority {
            mapping: Authority::new(cause),
            admission: Ok(true),
            calls: Cell::new(0),
        };
        let mut changes = [
            transition(&authority.mapping, cause),
            transition(&authority.mapping, cause),
        ];
        changes[1].objective_index = 1;
        let mut bounded = proposal_limits();
        match case {
            "stale-tail" => changes[1].expected = &authority.mapping.replacement,
            "count" => bounded.maximum_changes = 1,
            "work" => bounded.maximum_comparisons = 1,
            "input" => bounded.maximum_input_bytes = 1,
            "output" => bounded.maximum_output_bytes = 1,
            _ => unreachable!(),
        }
        let rules = [rule()];
        let content = definitions();
        let resources = resource_constraints();
        assert!(
            run(
                &authority,
                &current,
                &changes,
                inventory(&rules, &content, &resources),
                bounded,
            )
            .is_err()
        );
        assert_eq!(authority.calls.get(), 0);
        assert_eq!(current, before);
    }
}

struct EncounterBatchAuthority {
    mappings: Vec<Authority>,
    calls: Cell<usize>,
}

impl ObjectivePolicyOwner for EncounterBatchAuthority {
    fn admit_transition(
        &self,
        current: &Checkpoint,
        transition: &AuthoredObjectiveTransition<'_>,
    ) -> Result<(), ObjectivePolicyError> {
        self.mappings
            .iter()
            .find(|mapping| mapping.encounter == transition.encounter)
            .ok_or(ObjectivePolicyError::PolicyMismatch)?
            .admit_transition(current, transition)
    }

    fn admit_end_state(
        &self,
        _: &Checkpoint,
        target: &EncounterState,
        changes: &[AuthoredObjectiveTransition<'_>],
    ) -> Result<bool, ObjectivePolicyError> {
        self.calls.set(self.calls.get() + 1);
        if changes.len() != 4 {
            return Err(ObjectivePolicyError::PolicyMismatch);
        }
        if target.id == encounter(13).id {
            return Err(ObjectivePolicyError::RightsDenied);
        }
        Ok(true)
    }
}

#[test]
fn a_denied_final_encounter_discards_an_earlier_admitted_terminal_encounter() {
    let current = current(|_| {});
    let before = current.clone();
    let cause = &current.state().facts[0];
    let first = Authority::new(cause);
    let mut second = Authority::new(cause);
    second.encounter = encounter(13).id;
    let authority = EncounterBatchAuthority {
        mappings: vec![first, second],
        calls: Cell::new(0),
    };
    let mut changes = Vec::new();
    for mapping in &authority.mappings {
        for objective_index in [0, 1] {
            changes.push(AuthoredObjectiveTransition {
                encounter: mapping.encounter,
                objective_index,
                expected: &mapping.expected,
                replacement: &mapping.replacement,
                policy: &mapping.policy,
                source: &mapping.source,
                actor: entity(4),
                cause,
            });
        }
    }
    let rules = [rule()];
    let content = definitions();
    let resources = resource_constraints();
    assert_eq!(
        run(
            &authority,
            &current,
            &changes,
            inventory(&rules, &content, &resources),
            proposal_limits(),
        )
        .err(),
        Some(ObjectiveProposalError::Policy(
            ObjectivePolicyError::RightsDenied
        ))
    );
    assert_eq!(authority.calls.get(), 2);
    assert_eq!(current, before);
}

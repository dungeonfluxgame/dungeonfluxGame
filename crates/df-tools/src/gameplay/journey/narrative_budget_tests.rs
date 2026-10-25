//! Explicit source-backed Design examples. The event classification and unit values here are
//! fixture admissions, not approved production creation/social costs or free-play calibration.
use super::*;
use df_content::narrative::{NarrativeBudgetPolicy, NarrativeBudgetRule};
use df_narrative::{
    ConvergenceError, NarrativeBudgetChange, NarrativeBudgetError, NarrativeBudgetLimits,
    NarrativeBudgetOutcome, NarrativeBudgetRequest, stage_checkpoint_narrative_budget,
};

fn policy() -> NarrativeBudgetPolicy<ContentReference> {
    NarrativeBudgetPolicy {
        definition: model::content("narrative-intervention-policy").unwrap(),
        maximum: 8,
        strong_events: vec![NarrativeBudgetRule {
            event: model::content("create-character").unwrap(),
            units: 8,
        }],
        free_play_events: vec![NarrativeBudgetRule {
            event: model::content("courier-escort-contact").unwrap(),
            units: 8,
        }],
    }
}

fn source_fact_id(current: &Checkpoint, entry: &str) -> FactId {
    let event = model::content(entry).unwrap();
    current.state().facts.iter().find(|fact|
        matches!(&fact.value, FactValue::ContentEvent { definition, .. } if *definition == event)
    ).unwrap().id
}

fn budget(
    current: &Checkpoint,
    change: NarrativeBudgetChange,
    supplied: &NarrativeBudgetPolicy<ContentReference>,
    expected: &NarrativeBudgetPolicy<ContentReference>,
    limits: NarrativeBudgetLimits,
) -> Result<df_narrative::NarrativeBudgetProposal, NarrativeBudgetError> {
    stage_checkpoint_narrative_budget(
        current,
        NarrativeBudgetRequest {
            expected_basis: current.basis(),
            admitted_pins: current.pins(),
            policy: supplied,
            expected_policy: expected,
            source_policy: &model::label(THREAD_POLICY).unwrap(),
            change,
            inventory: ReferenceInventory {
                rules: &[model::rule().unwrap(), rule().unwrap()],
                content: &model::contents().unwrap(),
                resources: &super::super::super::resources_for(current.state()).unwrap(),
                assets: &[],
            },
            checkpoint_limits: model::limits(),
        },
        limits,
    )
}

fn limits() -> NarrativeBudgetLimits {
    NarrativeBudgetLimits {
        records: 512,
        work: 1024 * 1024,
    }
}

// These whole-value admissions are Design examples only. Their eight-unit cost/credit
// exercises the registered journey's existing eight-unit starting balance, not a tariff.
pub(super) fn design_pending_budget(
    before: &Checkpoint,
    candidate: &Checkpoint,
    credit: bool,
) -> Result<Checkpoint, NarrativeBudgetError> {
    let policy = NarrativeBudgetPolicy {
        definition: model::content("narrative-intervention-policy").unwrap(),
        maximum: 8,
        strong_events: vec![NarrativeBudgetRule {
            event: model::content("begin-story").unwrap(),
            units: 8,
        }],
        free_play_events: vec![NarrativeBudgetRule {
            event: model::content("escort-courier").unwrap(),
            units: 8,
        }],
    };
    let id = source_fact_id(
        candidate,
        if credit {
            "escort-courier"
        } else {
            "begin-story"
        },
    );
    df_narrative::stage_candidate_narrative_budget(
        before,
        candidate,
        NarrativeBudgetRequest {
            expected_basis: before.basis(),
            admitted_pins: before.pins(),
            policy: &policy,
            expected_policy: &policy,
            source_policy: &model::label(THREAD_POLICY).unwrap(),
            change: if credit {
                NarrativeBudgetChange::ApprovedFreePlay(id)
            } else {
                NarrativeBudgetChange::StrongIntervention(id)
            },
            inventory: ReferenceInventory {
                rules: &[model::rule().unwrap(), rule().unwrap()],
                content: &model::contents().unwrap(),
                resources: &super::super::super::resources_for(before.state()).unwrap(),
                assets: &[],
            },
            checkpoint_limits: model::limits(),
        },
        limits(),
    )
    .map(|proposal| proposal.checkpoint)
}

#[test]
fn design_candidate_budget_preserves_native_thread_ledger_and_refuses_replay_or_history_rewrite() {
    let before = prepared_story();
    let staged = model::HarborEngine
        .decide_admitted(
            &before,
            &input(&before, first(), 90, "begin-story", vec![]),
            df_types::OperationId::from_bytes(&[90; 16]).unwrap(),
        )
        .unwrap();
    let charged = design_pending_budget(&before, &staged, false).unwrap();
    assert_eq!(
        charged.state().narrative.accepted_facts,
        staged.state().narrative.accepted_facts
    );
    assert_eq!(charged.state().facts, staged.state().facts);
    assert_eq!(charged.state().decisions, staged.state().decisions);
    assert_eq!(charged.state().draws, staged.state().draws);
    assert_eq!(phase(&charged).unwrap(), phase(&staged).unwrap());
    assert_eq!(charged.state().narrative.remaining_budget, 0);
    assert_eq!(
        design_pending_budget(&before, &charged, false).unwrap_err(),
        NarrativeBudgetError::Source
    );
    assert!(matches!(
        design_pending_budget(&staged, &staged, false),
        Err(NarrativeBudgetError::Binding(_))
    ));
    let mut forged = staged.state().clone();
    forged.facts[0].audience = AudienceScope::Members(vec![first()]);
    let forged = model::checkpoint(staged.basis(), forged).unwrap();
    assert_eq!(
        design_pending_budget(&before, &forged, false).unwrap_err(),
        NarrativeBudgetError::Source
    );
    assert_eq!(before.state().narrative.remaining_budget, 8);
    assert_eq!(staged.state().narrative.remaining_budget, 8);
}

#[test]
fn design_strong_event_receipt_debits_persisted_units_without_new_facts_or_rules_changes() {
    let current = prepared_story();
    let original = current.clone();
    let policy = policy();
    let id = source_fact_id(&current, "create-character");
    let proposed = budget(
        &current,
        NarrativeBudgetChange::StrongIntervention(id),
        &policy,
        &policy,
        limits(),
    )
    .unwrap();
    assert_eq!(
        proposed.outcome,
        NarrativeBudgetOutcome::Applied {
            before: 8,
            after: 0
        }
    );
    assert_eq!(proposed.checkpoint.basis(), current.basis());
    assert_eq!(proposed.checkpoint.pins(), current.pins());
    assert_eq!(proposed.checkpoint.state().narrative.remaining_budget, 0);
    let mut protected = proposed.checkpoint.state().clone();
    protected.narrative = current.state().narrative.clone();
    assert_eq!(protected, *current.state());
    let mut consumed = current.state().narrative.accepted_facts.clone();
    consumed.push(id);
    assert_eq!(
        proposed.checkpoint.state().narrative.accepted_facts,
        consumed
    );
    let repeated = budget(
        &proposed.checkpoint,
        NarrativeBudgetChange::StrongIntervention(id),
        &policy,
        &policy,
        limits(),
    )
    .unwrap();
    assert_eq!(repeated.outcome, NarrativeBudgetOutcome::AlreadyConsumed);
    assert_eq!(repeated.checkpoint, proposed.checkpoint);
    assert_eq!(current, original);
}

#[test]
fn design_approved_free_play_credits_only_exact_unconsumed_source_event_and_caps_balance() {
    let prepared = prepared_story();
    let staged = run(
        &prepared,
        &input(&prepared, first(), 90, "begin-story", vec![]),
    );
    let opening = design_pending_budget(&prepared, &staged, false).unwrap();
    let earned = run(
        &opening,
        &input(&opening, first(), 91, "escort-courier", vec![]),
    );
    let policy = policy();
    let id = source_fact_id(&earned, "courier-escort-contact");
    let credited = budget(
        &earned,
        NarrativeBudgetChange::ApprovedFreePlay(id),
        &policy,
        &policy,
        limits(),
    )
    .unwrap();
    assert_eq!(
        credited.outcome,
        NarrativeBudgetOutcome::Applied {
            before: 0,
            after: 8
        }
    );
    assert_eq!(
        credited.checkpoint.state().logical_time,
        earned.state().logical_time
    );
    assert_eq!(
        credited.checkpoint.state().resources,
        earned.state().resources
    );
    assert_eq!(credited.checkpoint.state().facts, earned.state().facts);
    assert_eq!(credited.checkpoint.state().draws, earned.state().draws);
    let repeated = budget(
        &credited.checkpoint,
        NarrativeBudgetChange::ApprovedFreePlay(id),
        &policy,
        &policy,
        limits(),
    )
    .unwrap();
    assert_eq!(repeated.outcome, NarrativeBudgetOutcome::AlreadyConsumed);
    assert_eq!(repeated.checkpoint, credited.checkpoint);
    let mut full = earned.state().clone();
    full.narrative.remaining_budget = 8;
    let full = model::checkpoint(earned.basis(), full).unwrap();
    let capped = budget(
        &full,
        NarrativeBudgetChange::ApprovedFreePlay(id),
        &policy,
        &policy,
        limits(),
    )
    .unwrap();
    assert_eq!(
        capped.outcome,
        NarrativeBudgetOutcome::Applied {
            before: 8,
            after: 8
        }
    );
    assert_eq!(capped.checkpoint.state().narrative.remaining_budget, 8);
}

#[test]
fn design_budget_source_policy_capacity_and_exhaustion_fail_without_partial_mutation() {
    let current = prepared_story();
    let policy = policy();
    let id = source_fact_id(&current, "create-character");
    let unchanged = current.clone();
    assert_eq!(
        budget(
            &current,
            NarrativeBudgetChange::ApprovedFreePlay(id),
            &policy,
            &policy,
            limits()
        )
        .unwrap_err(),
        NarrativeBudgetError::UnsupportedEvent
    );
    let mut supplied = policy.clone();
    supplied.maximum = 2;
    assert_eq!(
        budget(
            &current,
            NarrativeBudgetChange::StrongIntervention(id),
            &supplied,
            &policy,
            limits()
        )
        .unwrap_err(),
        NarrativeBudgetError::Policy
    );
    let mut duplicate = policy.clone();
    duplicate
        .free_play_events
        .push(duplicate.strong_events[0].clone());
    assert_eq!(
        budget(
            &current,
            NarrativeBudgetChange::StrongIntervention(id),
            &duplicate,
            &duplicate,
            limits()
        )
        .unwrap_err(),
        NarrativeBudgetError::Policy
    );
    for bounded in [
        NarrativeBudgetLimits {
            records: 0,
            ..limits()
        },
        NarrativeBudgetLimits {
            work: 0,
            ..limits()
        },
    ] {
        assert_eq!(
            budget(
                &current,
                NarrativeBudgetChange::StrongIntervention(id),
                &policy,
                &policy,
                bounded
            )
            .unwrap_err(),
            NarrativeBudgetError::Capacity
        );
    }
    let mut state = current.state().clone();
    state.narrative.remaining_budget = 0;
    let depleted = model::checkpoint(current.basis(), state).unwrap();
    assert_eq!(
        budget(
            &depleted,
            NarrativeBudgetChange::StrongIntervention(id),
            &policy,
            &policy,
            limits()
        )
        .unwrap_err(),
        NarrativeBudgetError::InsufficientBudget
    );
    let mut state = current.state().clone();
    state.narrative.remaining_budget = 9;
    let invalid_balance = model::checkpoint(current.basis(), state).unwrap();
    assert_eq!(
        budget(
            &invalid_balance,
            NarrativeBudgetChange::StrongIntervention(id),
            &policy,
            &policy,
            limits()
        )
        .unwrap_err(),
        NarrativeBudgetError::InvalidBalance
    );
    let mut state = current.state().clone();
    let fact = state.facts.iter().find(|fact| fact.id == id).unwrap();
    let operation = fact.operation;
    state
        .decisions
        .iter_mut()
        .find(|decision| decision.operation == operation)
        .unwrap()
        .source_policy = model::label("unapproved-source-policy").unwrap();
    let changed = model::checkpoint(current.basis(), state).unwrap();
    assert_eq!(
        budget(
            &changed,
            NarrativeBudgetChange::StrongIntervention(id),
            &policy,
            &policy,
            limits()
        )
        .unwrap_err(),
        NarrativeBudgetError::Source
    );
    assert_eq!(current, unchanged);
}

#[test]
fn native_earned_routes_validate_convergence_without_spending_or_replenishing_units() {
    for (current, action) in [
        (prepared_story(), "begin-story"),
        (opening_story(), "ask-courier"),
        (opening_story(), "escort-courier"),
    ] {
        let original = current.clone();
        let selected = select(&current, &input(&current, first(), 92, action, vec![])).unwrap();
        assert_eq!(
            selected.state().narrative.remaining_budget,
            current.state().narrative.remaining_budget
        );
        assert_eq!(selected.state().facts, current.state().facts);
        assert_eq!(selected.state().draws, current.state().draws);
        assert_eq!(current, original);
    }
    let current = prepared_story();
    let policy = native_budget().unwrap();
    assert!(policy.strong_events.is_empty());
    assert!(policy.free_play_events.is_empty());
    assert_eq!(
        budget(
            &current,
            NarrativeBudgetChange::ApprovedFreePlay(source_fact_id(&current, "create-character")),
            &policy,
            &policy,
            limits()
        )
        .unwrap_err(),
        NarrativeBudgetError::UnsupportedEvent
    );
}

#[test]
fn design_convergence_retains_valid_causal_alternatives_and_rejects_missing_strong_receipt() {
    let current = prepared_story();
    let create = model::content("create-character").unwrap();
    let room = model::content("room").unwrap();
    let opening = model::content("opening").unwrap();
    let cause = [BeatCause {
        fact: source_fact_id(&current, "create-character"),
        event: &create,
        consumed_by_narrative: false,
    }];
    let alternatives = [AdmittedBeatAlternative {
        selection: &create,
        from: &room,
        to: &opening,
        causes: &cause,
        required_threads: &[],
        opened_thread: None,
    }];
    let policy = policy();
    let source_policy = model::label(THREAD_POLICY).unwrap();
    let rules = [model::rule().unwrap(), rule().unwrap()];
    let content = model::contents().unwrap();
    let resources = super::super::super::resources_for(current.state()).unwrap();
    let request = |receipt| df_narrative::CheckpointConvergenceRequest {
        beat: CheckpointBeatRequest {
            expected_basis: current.basis(),
            admitted_pins: current.pins(),
            policy: &source_policy,
            expected_policy: &source_policy,
            recipient: first(),
            selection: &create,
            alternatives: &alternatives,
            inventory: ReferenceInventory {
                rules: &rules,
                content: &content,
                resources: &resources,
                assets: &[],
            },
            checkpoint_limits: model::limits(),
        },
        budget: &policy,
        expected_budget: &policy,
        strong_receipt: receipt,
        delivery: None,
    };
    let bounds = df_narrative::ConvergenceLimits {
        beats: BeatSelectionLimits {
            records: 512,
            alternatives: 2,
            work: 1024 * 1024,
        },
        budget: limits(),
        maximum_evidence_bytes: 8192,
    };
    assert!(matches!(
        df_narrative::stage_checkpoint_convergence(&current, request(None), bounds),
        Err(ConvergenceError::MissingInterventionReceipt)
    ));
    let proposed =
        df_narrative::stage_checkpoint_convergence(&current, request(Some(cause[0].fact)), bounds)
            .unwrap();
    assert_eq!(proposed.chosen, create);
    assert_eq!(
        proposed.relevant_alternatives.as_slice(),
        std::slice::from_ref(&create)
    );
    assert_eq!(proposed.causes, [cause[0].fact]);
    assert_eq!(proposed.checkpoint.state().narrative.remaining_budget, 0);
    assert_eq!(proposed.checkpoint.state().facts, current.state().facts);
    assert_eq!(proposed.checkpoint.state().draws, current.state().draws);
    assert!(matches!(
        df_narrative::stage_checkpoint_convergence(
            &current,
            request(Some(cause[0].fact)),
            df_narrative::ConvergenceLimits {
                maximum_evidence_bytes: 0,
                ..bounds
            }
        ),
        Err(ConvergenceError::Capacity)
    ));
}

#[test]
fn design_npc_delivery_requires_current_place_motivation_known_source_and_past_witness() {
    let opening = opening_story();
    let earned = run(
        &opening,
        &input(&opening, first(), 93, "escort-courier", vec![]),
    );
    let id = source_fact_id(&earned, "courier-escort-contact");
    let courier = super::super::super::courier_reaction::courier().unwrap();
    let hero = super::super::super::player_entity(first(), &earned).unwrap();
    let location = super::super::super::player_entity(second(), &earned).unwrap();
    // Explicit Design spatial admission: the coarse native scene has no physical locations.
    // This is a bounded source contract example, not a production spatial-delivery claim.
    let mut spatial = earned.state().clone();
    for world in &mut spatial.entities {
        if world.id == hero || world.id == courier {
            world.location = Some(location);
        }
    }
    let spatial = model::checkpoint(earned.basis(), spatial).unwrap();
    for case in 0..7 {
        let mut state = spatial.state().clone();
        match case {
            0 => {}
            1 => state
                .continuity
                .npcs
                .iter_mut()
                .find(|npc| npc.entity == courier)
                .unwrap()
                .known_facts
                .clear(),
            2 => state
                .continuity
                .npcs
                .iter_mut()
                .find(|npc| npc.entity == courier)
                .unwrap()
                .motivations
                .clear(),
            3 => {
                state
                    .entities
                    .iter_mut()
                    .find(|world| world.id == hero)
                    .unwrap()
                    .location = None
            }
            4 => state.continuity.witnesses.clear(),
            5 => {
                state
                    .facts
                    .iter_mut()
                    .find(|fact| fact.id == id)
                    .unwrap()
                    .audience = AudienceScope::Host
            }
            6 => {
                state
                    .continuity
                    .witnesses
                    .iter_mut()
                    .find(|witness| witness.observer == courier && witness.fact == id)
                    .unwrap()
                    .perceived_at
                    .ticks = state.logical_time.ticks + 1
            }
            _ => unreachable!(),
        }
        let current = model::checkpoint(spatial.basis(), state).unwrap();
        let before = current.clone();
        let contact = model::content("courier-escort-contact").unwrap();
        let selection = model::content("ask-courier").unwrap();
        let from = model::content("courier-answer-escort").unwrap();
        let to = model::content("courier-answer-seal").unwrap();
        let causes = [BeatCause {
            fact: id,
            event: &contact,
            consumed_by_narrative: false,
        }];
        let alternatives = [AdmittedBeatAlternative {
            selection: &selection,
            from: &from,
            to: &to,
            causes: &causes,
            required_threads: &[],
            opened_thread: None,
        }];
        let npc = current
            .state()
            .continuity
            .npcs
            .iter()
            .find(|npc| npc.entity == courier)
            .unwrap();
        let world = current
            .state()
            .entities
            .iter()
            .find(|world| world.id == courier)
            .unwrap();
        let motivation = model::content("deliver-dispatch").unwrap();
        let disclosed = [id];
        let policy = native_budget().unwrap();
        let source_policy = model::label(THREAD_POLICY).unwrap();
        let rules = [model::rule().unwrap(), rule().unwrap()];
        let content = model::contents().unwrap();
        let resources = super::super::super::resources_for(current.state()).unwrap();
        let proposal = df_narrative::stage_checkpoint_convergence(
            &current,
            df_narrative::CheckpointConvergenceRequest {
                beat: CheckpointBeatRequest {
                    expected_basis: current.basis(),
                    admitted_pins: current.pins(),
                    policy: &source_policy,
                    expected_policy: &source_policy,
                    recipient: first(),
                    selection: &selection,
                    alternatives: &alternatives,
                    inventory: ReferenceInventory {
                        rules: &rules,
                        content: &content,
                        resources: &resources,
                        assets: &[],
                    },
                    checkpoint_limits: model::limits(),
                },
                budget: &policy,
                expected_budget: &policy,
                strong_receipt: None,
                delivery: Some(df_narrative::AdmittedConvergenceDelivery {
                    npc,
                    world,
                    motivation: &motivation,
                    disclosed: &disclosed,
                }),
            },
            df_narrative::ConvergenceLimits {
                beats: BeatSelectionLimits {
                    records: 512,
                    alternatives: 2,
                    work: 1024 * 1024,
                },
                budget: limits(),
                maximum_evidence_bytes: 8192,
            },
        );
        if case == 0 {
            let proposed = proposal.unwrap();
            assert_eq!(proposed.causes, [id]);
            assert_eq!(proposed.checkpoint.state().facts, current.state().facts);
            assert_eq!(
                proposed.checkpoint.state().knowledge,
                current.state().knowledge
            );
            assert_eq!(
                proposed.checkpoint.state().narrative.remaining_budget,
                current.state().narrative.remaining_budget
            );
        } else {
            assert!(
                matches!(proposal, Err(ConvergenceError::Delivery)),
                "delivery case {case}"
            );
        }
        assert_eq!(current, before);
    }
}

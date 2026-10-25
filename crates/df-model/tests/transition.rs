#[path = "../../df-session/tests/support/fixture_model.rs"]
mod fixture_model;

use df_model::checkpoint::*;
use df_model::transition::{TransitionError, TransitionResult};
use df_types::OperationId;
use fixture_model::*;

fn pending_candidate(cause_operation: OperationId) -> Checkpoint {
    let mut next = basis();
    next.revision = next.revision.next_sequence().unwrap();
    let mut state = state();
    let fact = FactId::from_bytes(&[81; 16]).unwrap();
    state.facts.push(GameFact {
        id: fact,
        revision: next.revision,
        operation: cause_operation,
        ordinal: 0,
        cause: None,
        audience: AudienceScope::Shared,
        value: FactValue::ContentEvent {
            definition: content(),
            subjects: vec![],
        },
    });
    state.decisions.push(AcceptedDecision {
        operation: operation(),
        revision: next.revision,
        facts: if cause_operation == operation() {
            vec![fact]
        } else {
            vec![]
        },
        draws: vec![],
        effects: vec![],
        source_policy: label("fixture-suspension"),
        semantic_output: None,
    });
    state.pending.push(PendingResolution {
        id: ResolutionId::from_bytes(&[82; 16]).unwrap(),
        basis: next,
        continuation: label("fixture-continuation"),
        window: ResolutionWindow {
            id: WindowId::from_bytes(&[83; 16]).unwrap(),
            phase: TriggerPhase::BeforeConsequence,
            causal_fact: fact,
            source: rule(),
            timer: None,
        },
        next: PendingInput::Choice {
            remaining: vec![OfferedResponse {
                participant: member(3),
                offer: label("fixture-offer"),
                options: vec![label("fixture-option")],
                source: rule(),
            }],
        },
        choices: vec![],
        draw_ordinals: vec![],
        spent: vec![],
        rulings: vec![],
    });
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        next,
        pins(),
        state,
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &resource_constraints(),
            assets: &[],
        },
        limits(),
    )
    .unwrap()
}

#[test]
fn accepted_pending_and_rejected_remain_disjoint() {
    let source_checkpoint = checkpoint(state()).unwrap();
    assert!(matches!(action(), GameInput::Game(command) if command.operation == operation()));
    let accepted: TransitionResult<&str> =
        TransitionResult::classify(&source_checkpoint, accepted(), operation()).unwrap();
    assert!(matches!(accepted, TransitionResult::Accepted(_)));
    accepted.validate(&source_checkpoint, operation()).unwrap();

    let pending: TransitionResult<&str> = TransitionResult::classify(
        &source_checkpoint,
        pending_candidate(operation()),
        operation(),
    )
    .unwrap();
    assert!(matches!(
        &pending,
        TransitionResult::Pending { resolution, window, next: PendingInput::Choice { .. }, .. }
            if *resolution == ResolutionId::from_bytes(&[82; 16]).unwrap()
                && *window == WindowId::from_bytes(&[83; 16]).unwrap()
    ));
    pending.validate(&source_checkpoint, operation()).unwrap();
    let mut wrong_next = pending.clone();
    if let TransitionResult::Pending { next, .. } = &mut wrong_next {
        *next = PendingInput::Roll {
            participant: member(3),
            sides: vec![20],
            source: rule(),
        };
    }
    assert_eq!(
        wrong_next.validate(&source_checkpoint, operation()),
        Err(TransitionError::Disposition)
    );

    let rejected: TransitionResult<&str> = TransitionResult::Rejected("source refused");
    assert_eq!(rejected, TransitionResult::Rejected("source refused"));
    assert_eq!(source_checkpoint, checkpoint(state()).unwrap());
}

#[test]
fn new_unowned_fact_cannot_classify_another_operation_and_variant_is_rechecked() {
    let source_checkpoint = checkpoint(state()).unwrap();
    let unrelated = OperationId::from_bytes(&[84; 16]).unwrap();
    let staged_checkpoint = pending_candidate(unrelated);
    let classified: TransitionResult<&str> =
        TransitionResult::classify(&source_checkpoint, staged_checkpoint.clone(), operation())
            .unwrap();
    assert!(matches!(classified, TransitionResult::Accepted(_)));
    let next = staged_checkpoint.state().pending[0].next.clone();
    let forged: TransitionResult<&str> = TransitionResult::Pending {
        candidate: staged_checkpoint,
        resolution: ResolutionId::from_bytes(&[82; 16]).unwrap(),
        window: WindowId::from_bytes(&[83; 16]).unwrap(),
        next,
    };
    assert_eq!(
        forged.validate(&source_checkpoint, operation()),
        Err(TransitionError::Disposition)
    );
}

#[test]
fn new_resolution_from_retained_causal_fact_is_pending_for_current_operation() {
    let earlier = pending_candidate(operation());
    let mut source_state = earlier.state().clone();
    let mut new_resolution = source_state.pending.pop().unwrap();
    let source_checkpoint = Checkpoint::new(
        CHECKPOINT_SCHEMA,
        earlier.basis(),
        pins(),
        source_state.clone(),
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &resource_constraints(),
            assets: &[],
        },
        limits(),
    )
    .unwrap();
    let mut next = source_checkpoint.basis();
    next.revision = next.revision.next_sequence().unwrap();
    let current_operation = OperationId::from_bytes(&[89; 16]).unwrap();
    source_state.decisions.push(AcceptedDecision {
        operation: current_operation,
        revision: next.revision,
        facts: vec![],
        draws: vec![],
        effects: vec![],
        source_policy: label("retained-fact-window"),
        semantic_output: None,
    });
    new_resolution.id = ResolutionId::from_bytes(&[90; 16]).unwrap();
    new_resolution.window.id = WindowId::from_bytes(&[91; 16]).unwrap();
    new_resolution.basis = next;
    source_state.pending.push(new_resolution);
    let staged_checkpoint = Checkpoint::new(
        CHECKPOINT_SCHEMA,
        next,
        pins(),
        source_state,
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &resource_constraints(),
            assets: &[],
        },
        limits(),
    )
    .unwrap();
    let transition: TransitionResult<&str> =
        TransitionResult::classify(&source_checkpoint, staged_checkpoint, current_operation)
            .unwrap();
    assert!(matches!(
        &transition,
        TransitionResult::Pending {
            resolution,
            window,
            next: PendingInput::Choice { remaining },
            ..
        } if *resolution == ResolutionId::from_bytes(&[90; 16]).unwrap()
            && *window == WindowId::from_bytes(&[91; 16]).unwrap()
            && remaining.len() == 1
    ));
    transition
        .validate(&source_checkpoint, current_operation)
        .unwrap();
}

#[test]
fn advanced_window_is_pending_but_basis_only_historical_carriage_is_accepted() {
    let source_checkpoint = pending_candidate(operation());
    let mut next = source_checkpoint.basis();
    next.revision = next.revision.next_sequence().unwrap();
    let resumed_operation = OperationId::from_bytes(&[86; 16]).unwrap();
    let mut state = source_checkpoint.state().clone();
    state.pending[0].basis = next;
    state.decisions.push(AcceptedDecision {
        operation: resumed_operation,
        revision: next.revision,
        facts: vec![],
        draws: vec![],
        effects: vec![],
        source_policy: label("fixture-resumption"),
        semantic_output: None,
    });
    let carried = Checkpoint::new(
        CHECKPOINT_SCHEMA,
        next,
        pins(),
        state.clone(),
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &resource_constraints(),
            assets: &[],
        },
        limits(),
    )
    .unwrap();
    let classified: TransitionResult<&str> =
        TransitionResult::classify(&source_checkpoint, carried, resumed_operation).unwrap();
    assert!(matches!(classified, TransitionResult::Accepted(_)));

    let advanced_window = WindowId::from_bytes(&[87; 16]).unwrap();
    state.pending[0].window.id = advanced_window;
    let staged_checkpoint = Checkpoint::new(
        CHECKPOINT_SCHEMA,
        next,
        pins(),
        state,
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &resource_constraints(),
            assets: &[],
        },
        limits(),
    )
    .unwrap();
    let classified: TransitionResult<&str> =
        TransitionResult::classify(&source_checkpoint, staged_checkpoint, resumed_operation)
            .unwrap();
    assert!(matches!(
        &classified,
        TransitionResult::Pending { window, .. } if *window == advanced_window
    ));
    classified
        .validate(&source_checkpoint, resumed_operation)
        .unwrap();
}

#[test]
fn old_epoch_and_wrong_operation_refuse_without_candidate() {
    let source_checkpoint = checkpoint(state()).unwrap();
    let wrong = OperationId::from_bytes(&[85; 16]).unwrap();
    let transition: Result<TransitionResult<&str>, _> =
        TransitionResult::classify(&source_checkpoint, accepted(), wrong);
    assert_eq!(transition, Err(TransitionError::Decision));
    let mut restored_basis = basis();
    restored_basis.revision = revision(3, 8);
    let restored = Checkpoint::new(
        CHECKPOINT_SCHEMA,
        restored_basis,
        pins(),
        state(),
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &resource_constraints(),
            assets: &[],
        },
        limits(),
    )
    .unwrap();
    let stale: Result<TransitionResult<&str>, _> =
        TransitionResult::classify(&restored, accepted(), operation());
    assert_eq!(stale, Err(TransitionError::Basis));
    assert_eq!(source_checkpoint, checkpoint(state()).unwrap());
}

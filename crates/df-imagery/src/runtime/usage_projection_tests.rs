use super::*;
use df_types::{AccountId, Currency};

fn limits() -> UsageProjectionLimits {
    UsageProjectionLimits {
        max_attempts: 16,
        max_observations: 32,
        max_unit_lines: 16,
        max_word_bytes: 128,
    }
}

fn owner() -> ImageOwner {
    ImageOwner::Draft {
        account: AccountId::from_bytes(&[1; 16]).unwrap(),
    }
}

fn job() -> ImageJobId {
    ImageJobId::from_bytes(&[2; 16]).unwrap()
}

fn attempt(stage: u8, dispatch: u8, kind: StageKind) -> UsageAttempt {
    UsageAttempt {
        scope: ExecutionScope {
            owner: owner(),
            job: job(),
            stage: ExecutionStageId::from_bytes(&[stage; 16]).unwrap(),
            dispatch: DispatchId::from_bytes(&[dispatch; 16]).unwrap(),
        },
        kind,
        mode: DispatchMode::Local,
    }
}

fn units(category: &str, quantity: u64) -> CostUnits {
    CostUnits {
        category: category.to_owned(),
        quantity,
    }
}

fn usage(attempt: &UsageAttempt, revision: u64) -> RuntimeUsage {
    RuntimeUsage {
        owner: attempt.scope.owner.clone(),
        job: attempt.scope.job,
        stage: attempt.scope.stage,
        dispatch: attempt.scope.dispatch,
        revision: UsageRevision(revision),
        route: CostRoute {
            provider: "openai".to_owned(),
            model: "image-model".to_owned(),
            cause: "scene_plate".to_owned(),
            detail: "generation".to_owned(),
            maxima: vec![units("text_input", 100), units("image_output", 500)],
        },
        evidence: UsageEvidence::Reported,
        units: vec![units("image_output", 100)],
        usable: true,
        at_ms: 1_000,
        raw_output: Some(b"PRIVATE_PROVIDER_OUTPUT".to_vec()),
    }
}

fn money(micros: u128) -> Money {
    Money::new(Currency::parse("USD").unwrap(), micros)
}

fn raw_receipt(usage: &RuntimeUsage) -> RawBookCostReceipt {
    RawBookCostReceipt {
        call_id: pinned_call_id(usage.dispatch.as_bytes()),
        revision: usage.revision,
        route: usage.route.clone(),
        evidence: usage.evidence,
        units: effective_units(usage).to_vec(),
        usable: usage.usable,
        amount: Some(money(3_000)),
        durable: true,
    }
}

fn project(
    attempts: &[UsageAttempt],
    observations: &[RuntimeUsage],
    receipts: &[ActualCostReceipt],
) -> Result<ImageUsageSummary, UsageProjectionError> {
    project_usage(UsageProjectionInput {
        owner: &owner(),
        job: job(),
        attempts,
        observations,
        receipts,
        limits: limits(),
    })
}

#[test]
fn an_unobserved_attempt_is_pending_and_an_empty_job_has_no_fabricated_total() {
    assert!(project(&[], &[], &[]).unwrap().stages.is_empty());
    let attempt = attempt(3, 4, StageKind::Generation);
    let summary = project(std::slice::from_ref(&attempt), &[], &[]).unwrap();
    assert_eq!(summary.stages.len(), 1);
    assert_eq!(summary.stages[0].stage, attempt.scope.stage);
    assert_eq!(summary.stages[0].dispatch, attempt.scope.dispatch);
    assert_eq!(summary.stages[0].kind, StageKind::Generation);
    assert_eq!(
        summary.stages[0].observation,
        StageUsageObservation::Pending
    );
    assert_eq!(summary.stages[0].amount, ImageUsageAmount::Pending);
}

#[test]
fn out_of_order_revisions_and_replays_select_one_latest_attempt_without_copying_raw_output() {
    let attempt = attempt(3, 4, StageKind::Generation);
    let old = usage(&attempt, 1);
    let mut latest = usage(&attempt, 3);
    latest.units = vec![units("image_output", 9)];
    let summary = project(
        &[attempt.clone(), attempt],
        &[latest.clone(), old, latest.clone()],
        &[],
    )
    .unwrap();
    assert_eq!(summary.stages.len(), 1);
    let StageUsageObservation::Recorded(observed) = &summary.stages[0].observation else {
        panic!("recorded");
    };
    assert_eq!(observed.revision, UsageRevision(3));
    assert_eq!(observed.units, latest.units);
    assert_eq!(summary.stages[0].amount, ImageUsageAmount::Pending);
    assert!(!format!("{summary:?}").contains("PRIVATE_PROVIDER_OUTPUT"));
}

#[test]
fn paid_retries_and_heterogeneous_provider_categories_remain_separate() {
    let first = attempt(3, 4, StageKind::Inspection);
    let retry = attempt(3, 5, StageKind::Inspection);
    let generation = attempt(6, 7, StageKind::Generation);
    let mut text = usage(&first, 1);
    text.units = vec![
        units("text_input", u64::MAX),
        units("text_output", u64::MAX),
    ];
    let mut fal = usage(&generation, 1);
    fal.route.provider = "fal".to_owned();
    fal.route.model = "fal-endpoint".to_owned();
    fal.units = vec![units("images", 1)];
    let summary = project(&[first, retry, generation], &[text, fal], &[]).unwrap();
    assert_eq!(summary.stages.len(), 3);
    let recorded: Vec<_> = summary
        .stages
        .iter()
        .filter_map(|entry| match &entry.observation {
            StageUsageObservation::Recorded(usage) => Some(usage),
            _ => None,
        })
        .collect();
    assert_eq!(recorded.len(), 2);
    assert_eq!(
        recorded[0].units,
        [
            units("text_input", u64::MAX),
            units("text_output", u64::MAX)
        ]
    );
    assert_eq!(recorded[1].provider, "fal");
    assert_eq!(recorded[1].units, [units("images", 1)]);
}

#[test]
fn a_matching_durable_book_receipt_preserves_billed_unusable_cost_without_refund_inference() {
    let attempt = attempt(3, 4, StageKind::Generation);
    let mut usage = usage(&attempt, 2);
    usage.usable = false;
    let receipt = ActualCostReceipt::verify(raw_receipt(&usage), &usage, limits()).unwrap();
    let summary = project(&[attempt], &[usage], &[receipt]).unwrap();
    assert_eq!(
        summary.stages[0].amount,
        ImageUsageAmount::Known(money(3_000))
    );
    let StageUsageObservation::Recorded(observed) = &summary.stages[0].observation else {
        panic!("recorded");
    };
    assert!(!observed.usable);
}

#[test]
fn wrong_call_revision_route_evidence_units_usability_and_currency_cannot_mint_a_receipt() {
    let attempt = attempt(3, 4, StageKind::Generation);
    let usage = usage(&attempt, 2);
    for case in 0..11 {
        let mut raw = raw_receipt(&usage);
        let expected = match case {
            0 => {
                raw.call_id = pinned_call_id(&[8; 16]);
                UsageProjectionError::ReceiptCallMismatch
            }
            1 => {
                raw.revision = UsageRevision(1);
                UsageProjectionError::ReceiptRevisionMismatch
            }
            2 => {
                raw.route.provider = "bfl".to_owned();
                UsageProjectionError::RouteConflict
            }
            3 => {
                raw.route.model = "other".to_owned();
                UsageProjectionError::RouteConflict
            }
            4 => {
                raw.route.maxima[0].quantity += 1;
                UsageProjectionError::RouteConflict
            }
            5 => {
                raw.evidence = UsageEvidence::Estimated;
                UsageProjectionError::ReceiptObservationMismatch
            }
            6 => {
                raw.units[0].quantity += 1;
                UsageProjectionError::ReceiptObservationMismatch
            }
            7 => {
                raw.usable = false;
                UsageProjectionError::ReceiptObservationMismatch
            }
            8 => {
                raw.route.cause = "storyteller".to_owned();
                UsageProjectionError::RouteConflict
            }
            9 => {
                raw.route.detail = "different".to_owned();
                UsageProjectionError::RouteConflict
            }
            _ => {
                raw.amount = Some(Money::new(Currency::parse("EUR").unwrap(), 3_000));
                UsageProjectionError::CurrencyMismatch
            }
        };
        assert_eq!(
            ActualCostReceipt::verify(raw, &usage, limits()),
            Err(expected)
        );
    }
}

#[test]
fn a_stale_verified_receipt_cannot_supply_a_corrected_revisions_money() {
    let attempt = attempt(3, 4, StageKind::Generation);
    let old = usage(&attempt, 1);
    let receipt = ActualCostReceipt::verify(raw_receipt(&old), &old, limits()).unwrap();
    let mut corrected = usage(&attempt, 2);
    corrected.units[0].quantity = 1;
    assert_eq!(
        project(
            std::slice::from_ref(&attempt),
            std::slice::from_ref(&corrected),
            std::slice::from_ref(&receipt)
        )
        .unwrap()
        .stages[0]
            .amount,
        ImageUsageAmount::Pending
    );
    corrected.evidence = UsageEvidence::Unknown;
    assert_eq!(
        project(&[attempt], &[corrected], &[receipt])
            .unwrap()
            .stages[0]
            .amount,
        ImageUsageAmount::Unknown
    );
}

#[test]
fn unknown_nonempty_low_units_use_maxima_and_never_turn_book_exposure_into_known_cost() {
    let attempt = attempt(3, 4, StageKind::Generation);
    let mut usage = usage(&attempt, 1);
    usage.evidence = UsageEvidence::Unknown;
    usage.units = vec![units("image_output", 0)];
    let receipt = ActualCostReceipt::verify(raw_receipt(&usage), &usage, limits()).unwrap();
    let summary = project(&[attempt], std::slice::from_ref(&usage), &[receipt]).unwrap();
    assert_eq!(summary.stages[0].amount, ImageUsageAmount::Unknown);
    let StageUsageObservation::Recorded(observed) = &summary.stages[0].observation else {
        panic!("recorded");
    };
    assert_eq!(observed.units, usage.route.maxima);
}

#[test]
fn unacked_receipts_are_pending_and_unpriced_receipts_are_unknown() {
    let attempt = attempt(3, 4, StageKind::Generation);
    let usage = usage(&attempt, 1);
    let mut raw = raw_receipt(&usage);
    raw.durable = false;
    let receipt = ActualCostReceipt::verify(raw, &usage, limits()).unwrap();
    assert_eq!(
        project(
            std::slice::from_ref(&attempt),
            std::slice::from_ref(&usage),
            &[receipt]
        )
        .unwrap()
        .stages[0]
            .amount,
        ImageUsageAmount::Pending
    );
    let mut raw = raw_receipt(&usage);
    raw.amount = None;
    let receipt = ActualCostReceipt::verify(raw, &usage, limits()).unwrap();
    assert_eq!(
        project(&[attempt], &[usage], &[receipt]).unwrap().stages[0].amount,
        ImageUsageAmount::Unknown
    );
}

#[test]
fn an_explicit_reported_zero_requires_actual_matching_book_evidence_and_nonempty_units() {
    let attempt = attempt(3, 4, StageKind::Generation);
    let mut usage = usage(&attempt, 1);
    usage.units[0].quantity = 0;
    let mut raw = raw_receipt(&usage);
    raw.amount = Some(money(0));
    let receipt = ActualCostReceipt::verify(raw, &usage, limits()).unwrap();
    assert_eq!(
        project(
            std::slice::from_ref(&attempt),
            std::slice::from_ref(&usage),
            &[receipt]
        )
        .unwrap()
        .stages[0]
            .amount,
        ImageUsageAmount::Known(money(0))
    );
    usage.units.clear();
    usage.route.maxima.clear();
    let mut raw = raw_receipt(&usage);
    raw.amount = Some(money(0));
    let receipt = ActualCostReceipt::verify(raw, &usage, limits()).unwrap();
    assert_eq!(
        project(&[attempt], &[usage], &[receipt]).unwrap().stages[0].amount,
        ImageUsageAmount::Unknown
    );
}

#[test]
fn duplicate_receipts_count_once_and_contradictory_same_revision_money_is_refused() {
    let attempt = attempt(3, 4, StageKind::Generation);
    let usage = usage(&attempt, 1);
    let durable = ActualCostReceipt::verify(raw_receipt(&usage), &usage, limits()).unwrap();
    let mut pending = raw_receipt(&usage);
    pending.durable = false;
    let pending = ActualCostReceipt::verify(pending, &usage, limits()).unwrap();
    let summary = project(
        std::slice::from_ref(&attempt),
        std::slice::from_ref(&usage),
        &[durable.clone(), pending, durable.clone()],
    )
    .unwrap();
    assert_eq!(summary.stages.len(), 1);
    assert_eq!(
        summary.stages[0].amount,
        ImageUsageAmount::Known(money(3_000))
    );
    let mut conflict = raw_receipt(&usage);
    conflict.amount = Some(money(4_000));
    let conflict = ActualCostReceipt::verify(conflict, &usage, limits()).unwrap();
    assert_eq!(
        project(&[attempt], &[usage], &[durable, conflict]),
        Err(UsageProjectionError::ReceiptConflict)
    );
}

#[test]
fn foreign_scope_unknown_attempt_and_conflicting_stage_identity_are_rejected() {
    let first = attempt(3, 4, StageKind::Generation);
    let mut foreign = usage(&first, 1);
    foreign.owner = ImageOwner::Draft {
        account: AccountId::from_bytes(&[9; 16]).unwrap(),
    };
    assert_eq!(
        project(std::slice::from_ref(&first), &[foreign], &[]),
        Err(UsageProjectionError::ScopeMismatch)
    );
    let other = attempt(5, 6, StageKind::Inspection);
    assert_eq!(
        project(std::slice::from_ref(&first), &[usage(&other, 1)], &[]),
        Err(UsageProjectionError::UnknownAttempt)
    );
    let wrong_stage = attempt(5, 4, StageKind::Inspection);
    assert_eq!(
        project(&[first.clone(), wrong_stage], &[], &[]),
        Err(UsageProjectionError::AttemptConflict)
    );
    let wrong_kind = attempt(3, 6, StageKind::Inspection);
    assert_eq!(
        project(&[first, wrong_kind], &[], &[]),
        Err(UsageProjectionError::AttemptConflict)
    );
}

#[test]
fn conflicting_old_revisions_are_not_hidden_by_a_newer_revision_or_input_order() {
    let attempt = attempt(3, 4, StageKind::Generation);
    let old = usage(&attempt, 1);
    let mut conflict = old.clone();
    conflict.units[0].quantity = 2;
    let latest = usage(&attempt, 3);
    for reports in [
        [latest.clone(), old.clone(), conflict.clone()],
        [old.clone(), latest, conflict],
    ] {
        assert_eq!(
            project(std::slice::from_ref(&attempt), &reports, &[]),
            Err(UsageProjectionError::RevisionConflict)
        );
    }
}

#[test]
fn changed_frozen_routes_and_duplicate_categories_are_not_silently_merged() {
    let attempt = attempt(3, 4, StageKind::Generation);
    let old = usage(&attempt, 1);
    let mut changed = usage(&attempt, 2);
    changed.route.maxima[0].quantity += 1;
    assert_eq!(
        project(std::slice::from_ref(&attempt), &[old, changed], &[]),
        Err(UsageProjectionError::RouteConflict)
    );
    let mut duplicate = usage(&attempt, 1);
    duplicate.units.push(duplicate.units[0].clone());
    assert_eq!(
        project(&[attempt], &[duplicate], &[]),
        Err(UsageProjectionError::InvalidUnits)
    );
}

#[test]
fn finite_read_bounds_return_typed_excess_instead_of_truncating_attempts_or_units() {
    let attempt = attempt(3, 4, StageKind::Generation);
    let usage = usage(&attempt, 1);
    let mut bound = limits();
    bound.max_attempts = 1;
    let attempts = [attempt.clone(), attempt.clone()];
    assert_eq!(
        project_usage(UsageProjectionInput {
            owner: &owner(),
            job: job(),
            attempts: &attempts,
            observations: &[],
            receipts: &[],
            limits: bound
        }),
        Err(UsageProjectionError::BoundExceeded(
            UsageProjectionBound::Attempts
        ))
    );
    bound = limits();
    bound.max_unit_lines = 1;
    assert_eq!(
        project_usage(UsageProjectionInput {
            owner: &owner(),
            job: job(),
            attempts: std::slice::from_ref(&attempt),
            observations: std::slice::from_ref(&usage),
            receipts: &[],
            limits: bound
        }),
        Err(UsageProjectionError::BoundExceeded(
            UsageProjectionBound::UnitLines
        ))
    );
    bound = limits();
    bound.max_word_bytes = 1;
    assert_eq!(
        project_usage(UsageProjectionInput {
            owner: &owner(),
            job: job(),
            attempts: std::slice::from_ref(&attempt),
            observations: &[usage],
            receipts: &[],
            limits: bound
        }),
        Err(UsageProjectionError::BoundExceeded(
            UsageProjectionBound::WordBytes
        ))
    );
    bound = limits();
    bound.max_observations = 0;
    assert_eq!(
        project_usage(UsageProjectionInput {
            owner: &owner(),
            job: job(),
            attempts: &[attempt],
            observations: &[],
            receipts: &[],
            limits: bound
        }),
        Err(UsageProjectionError::InvalidLimits)
    );
}

#[test]
fn receipts_require_raw_authority_and_obey_current_bounds_after_policy_changes() {
    let attempt = attempt(3, 4, StageKind::Generation);
    let usage = usage(&attempt, 1);
    let receipt = ActualCostReceipt::verify(raw_receipt(&usage), &usage, limits()).unwrap();
    assert_eq!(
        project(
            std::slice::from_ref(&attempt),
            &[],
            std::slice::from_ref(&receipt)
        ),
        Err(UsageProjectionError::ReceiptWithoutUsage)
    );
    let mut bound = limits();
    bound.max_observations = 1;
    assert_eq!(
        project_usage(UsageProjectionInput {
            owner: &owner(),
            job: job(),
            attempts: std::slice::from_ref(&attempt),
            observations: std::slice::from_ref(&usage),
            receipts: &[receipt.clone(), receipt.clone()],
            limits: bound
        }),
        Err(UsageProjectionError::BoundExceeded(
            UsageProjectionBound::Receipts
        ))
    );
    assert_eq!(
        project_usage(UsageProjectionInput {
            owner: &owner(),
            job: job(),
            attempts: std::slice::from_ref(&attempt),
            observations: &[usage.clone(), usage.clone()],
            receipts: &[],
            limits: bound
        }),
        Err(UsageProjectionError::BoundExceeded(
            UsageProjectionBound::Observations
        ))
    );
    let mut foreign = usage.clone();
    foreign.job = ImageJobId::from_bytes(&[9; 16]).unwrap();
    assert_eq!(
        project(&[attempt], &[foreign], &[]),
        Err(UsageProjectionError::ScopeMismatch)
    );
    let mut raw = raw_receipt(&usage);
    raw.call_id = "PRIVATE_CALL".to_owned();
    raw.route.model = "PRIVATE_MODEL".to_owned();
    assert!(!format!("{raw:?}").contains("PRIVATE_"));
}

#[test]
fn infrastructure_bridge_requires_the_same_authoritative_call_revision_and_route() {
    let attempt = attempt(3, 4, StageKind::Generation);
    let usage = usage(&attempt, 4);
    let receipt = raw_receipt(&usage);
    let project = |receipt: &PinnedCostReceipt| {
        project_book_usage(UsageBookProjectionInput {
            owner: &owner(),
            job: job(),
            attempts: std::slice::from_ref(&attempt),
            observations: std::slice::from_ref(&usage),
            book_receipts: std::slice::from_ref(receipt),
            limits: limits(),
        })
    };
    assert_eq!(
        project(&receipt).unwrap().stages[0].amount,
        ImageUsageAmount::Known(money(3_000))
    );
    for revision in [1, 5] {
        let mut skewed = receipt.clone();
        skewed.revision = UsageRevision(revision);
        assert_eq!(
            project(&skewed).unwrap().stages[0].amount,
            ImageUsageAmount::Pending
        );
    }
    let mut wrong = receipt.clone();
    wrong.call_id = pinned_call_id(&[99; 16]);
    assert_eq!(
        project(&wrong),
        Err(UsageProjectionError::ReceiptCallMismatch)
    );
    let mut wrong = receipt;
    wrong.route.model = "wrong-model".to_owned();
    assert_eq!(project(&wrong), Err(UsageProjectionError::RouteConflict));
}

#[test]
fn infrastructure_bridge_rejects_receipt_excess_before_verification() {
    let attempt = attempt(3, 4, StageKind::Generation);
    let usage = usage(&attempt, 1);
    let receipts = vec![raw_receipt(&usage); 2];
    let mut limits = limits();
    limits.max_observations = 1;
    assert_eq!(
        project_book_usage(UsageBookProjectionInput {
            owner: &owner(),
            job: job(),
            attempts: &[attempt],
            observations: &[usage],
            book_receipts: &receipts,
            limits
        }),
        Err(UsageProjectionError::BoundExceeded(
            UsageProjectionBound::Receipts
        ))
    );
}

#[test]
fn only_a_stored_local_transform_is_not_applicable_without_fabricated_money() {
    let local = attempt(3, 4, StageKind::LocalTransform);
    let summary = project(std::slice::from_ref(&local), &[], &[]).unwrap();
    assert_eq!(summary.stages[0].amount, ImageUsageAmount::NotApplicable);
    assert_eq!(
        summary.stages[0].observation,
        StageUsageObservation::Pending
    );
    let generation = attempt(5, 6, StageKind::Generation);
    assert_eq!(
        project(std::slice::from_ref(&generation), &[], &[])
            .unwrap()
            .stages[0]
            .amount,
        ImageUsageAmount::Pending
    );
    let observed = usage(&generation, 1);
    let receipt = ActualCostReceipt::verify(raw_receipt(&observed), &observed, limits()).unwrap();
    assert_eq!(
        project(&[generation], &[observed], &[receipt])
            .unwrap()
            .stages[0]
            .amount,
        ImageUsageAmount::Known(money(3_000))
    );
    let raw = usage(&local, 1);
    assert_eq!(
        project(&[local], &[raw], &[]),
        Err(UsageProjectionError::AccountingConflict)
    );
}

#[test]
fn invalid_or_changed_finance_modes_cannot_hide_a_paid_attempt_as_not_applicable() {
    use crate::execution::FinanceTuple;
    let hosted = DispatchMode::Hosted(FinanceTuple::new([7; 16], [8; 16], [9; 16]).unwrap());
    let mut local = attempt(3, 4, StageKind::LocalTransform);
    local.mode = hosted;
    assert_eq!(
        project(&[local], &[], &[]),
        Err(UsageProjectionError::AccountingConflict)
    );
    let generation = attempt(5, 6, StageKind::Generation);
    let mut changed = generation.clone();
    changed.mode = hosted;
    assert_eq!(
        project(&[generation, changed], &[], &[]),
        Err(UsageProjectionError::AccountingConflict)
    );
    let mut invalid = attempt(7, 8, StageKind::Generation);
    invalid.mode = DispatchMode::Hosted(FinanceTuple {
        attempt: [0; 16],
        operation: [8; 16],
        effect: [9; 16],
    });
    assert_eq!(
        project(&[invalid], &[], &[]),
        Err(UsageProjectionError::AccountingConflict)
    );
}

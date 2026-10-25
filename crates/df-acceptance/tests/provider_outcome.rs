//! Plan section 17, JOB-01 and JOB-04, and invariant 15.4 (restore and branch never resend): what
//! the provider-outcome classifier of `df-provider-api` permits after a failure.
//!
//! The classifier is pure guidance, never a send permit; these tests pin the part of the
//! scenarios it decides: an uncertain outcome is reconciled and never resent, and replayed work
//! buys nothing. The two `classifier_*` retry-bound checks are building blocks, not JOB-03: the
//! circuit breaker that stops a retry storm is O1's and has no API yet.

use df_acceptance::{checked_request, operation, request_binding, request_owner_state};
use df_model::checkpoint::ExecutionMode;
use df_provider_api::{
    BudgetMutation, ProviderAttemptObservation, ProviderBillingClass, ProviderFailureClass,
    ProviderLiabilityDisposition, ProviderNextAction, ProviderOutcomeDecision, ProviderResultClass,
    ProviderRetryPolicy, classify_provider_outcome,
};

const ATTEMPT_OPERATION: u64 = 6;

const ALL_FAILURES: [ProviderFailureClass; 8] = [
    ProviderFailureClass::InvalidRequest,
    ProviderFailureClass::Denied,
    ProviderFailureClass::Capacity,
    ProviderFailureClass::Unavailable,
    ProviderFailureClass::Contract,
    ProviderFailureClass::Cancelled,
    ProviderFailureClass::Deadline,
    ProviderFailureClass::Unknown,
];

type Settlement = BudgetMutation<(), (), ()>;

fn classify(
    mode: ExecutionMode,
    result: ProviderResultClass,
    billing: ProviderBillingClass,
    settlement: &Settlement,
    policy: ProviderRetryPolicy,
) -> ProviderOutcomeDecision {
    let current = request_binding(mode, operation(ATTEMPT_OPERATION));
    let request = checked_request(&current);
    classify_provider_outcome(
        &request,
        request_owner_state(&current),
        ProviderAttemptObservation {
            operation: operation(ATTEMPT_OPERATION),
            result,
            billing,
        },
        settlement,
        policy,
    )
    .expect("the observation names the admitted operation")
}

fn bounded_policy(attempts_remaining: u32) -> ProviderRetryPolicy {
    ProviderRetryPolicy {
        attempts_remaining,
        retry: true,
        fallback: true,
    }
}

fn committed() -> Settlement {
    BudgetMutation::Committed {
        result: (),
        replayed: false,
    }
}

#[test]
fn job_01_no_failure_class_with_an_unsettled_attempt_ever_permits_another_paid_branch() {
    let unsettled: [Settlement; 4] = [
        BudgetMutation::Pending,
        BudgetMutation::Unknown,
        BudgetMutation::Refused(()),
        BudgetMutation::Failed(()),
    ];
    for failure in ALL_FAILURES {
        for settlement in &unsettled {
            let decision = classify(
                ExecutionMode::Live,
                ProviderResultClass::Failed(failure),
                ProviderBillingClass::VerifiedUnused,
                settlement,
                bounded_policy(5),
            );

            assert_eq!(decision.next, ProviderNextAction::ReconcileSameOperation);
            assert_eq!(
                decision.liability,
                ProviderLiabilityDisposition::RetainWorstCase,
                "{failure:?} keeps the full exposure until the same attempt is reconciled"
            );
        }
    }
}

#[test]
fn job_01_a_deadline_with_unknown_billing_retains_exposure_even_after_the_ledger_committed() {
    let decision = classify(
        ExecutionMode::Live,
        ProviderResultClass::Failed(ProviderFailureClass::Deadline),
        ProviderBillingClass::MissingOrAmbiguous,
        &committed(),
        bounded_policy(5),
    );

    assert_eq!(
        decision.liability,
        ProviderLiabilityDisposition::RetainWorstCase
    );
    assert_eq!(decision.next, ProviderNextAction::ReconcileSameOperation);
}

#[test]
fn classifier_retry_guidance_after_provider_capacity_loss_is_bounded_by_the_attempt_budget() {
    for failure in [
        ProviderFailureClass::Capacity,
        ProviderFailureClass::Unavailable,
    ] {
        let result = ProviderResultClass::Failed(failure);
        let billing = ProviderBillingClass::VerifiedUnused;

        let with_budget = classify(
            ExecutionMode::Live,
            result,
            billing,
            &committed(),
            bounded_policy(1),
        );
        let exhausted = classify(
            ExecutionMode::Live,
            result,
            billing,
            &committed(),
            bounded_policy(0),
        );
        let retries_off = classify(
            ExecutionMode::Live,
            result,
            billing,
            &committed(),
            ProviderRetryPolicy {
                attempts_remaining: 5,
                retry: false,
                fallback: false,
            },
        );

        assert_eq!(
            with_budget.next,
            ProviderNextAction::FreshAdmissionCandidates {
                retry: true,
                fallback: true
            },
            "guidance asks for a fresh admission; it is not a send permit"
        );
        assert_eq!(exhausted.next, ProviderNextAction::Stop);
        assert_eq!(retries_off.next, ProviderNextAction::Stop);
    }
}

#[test]
fn classifier_only_capacity_and_unavailability_are_ever_retry_candidates() {
    for failure in ALL_FAILURES {
        let decision = classify(
            ExecutionMode::Live,
            ProviderResultClass::Failed(failure),
            ProviderBillingClass::VerifiedUnused,
            &committed(),
            bounded_policy(5),
        );

        let is_candidate = matches!(
            decision.next,
            ProviderNextAction::FreshAdmissionCandidates { .. }
        );
        let expected = matches!(
            failure,
            ProviderFailureClass::Capacity | ProviderFailureClass::Unavailable
        );
        assert_eq!(is_candidate, expected, "{failure:?}");
    }
}

#[test]
fn job_04_billed_invalid_output_records_known_liability_and_stops_without_a_retry() {
    for result in [
        ProviderResultClass::Incomplete,
        ProviderResultClass::Failed(ProviderFailureClass::Contract),
    ] {
        let decision = classify(
            ExecutionMode::Live,
            result,
            ProviderBillingClass::VerifiedLiability,
            &committed(),
            bounded_policy(5),
        );

        assert_eq!(
            decision.liability,
            ProviderLiabilityDisposition::KnownLiability
        );
        assert_eq!(decision.next, ProviderNextAction::Stop);
    }
}

#[test]
fn invariant_replayed_and_prepared_work_never_gets_retry_guidance_that_could_buy_again() {
    for mode in [ExecutionMode::Replay, ExecutionMode::PreparedOnly] {
        let decision = classify(
            mode,
            ProviderResultClass::Failed(ProviderFailureClass::Capacity),
            ProviderBillingClass::VerifiedUnused,
            &committed(),
            bounded_policy(5),
        );

        assert_eq!(decision.next, ProviderNextAction::Stop, "{mode:?}");
    }
}

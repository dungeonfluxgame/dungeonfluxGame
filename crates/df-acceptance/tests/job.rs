//! Plan section 17, JOB-01 to JOB-05: paid provider work with unknown, stale, duplicate and late
//! outcomes (plan 6.2 and 6.3).
//!
//! The real tests drive the supplier-exposure and credit policies through fake owners (the policy
//! half; `credit.rs` retires with decision 30 and they retarget to `df-finance`). The reservation,
//! outbox, attempt-identity and circuit-breaker scenarios need F, O1 and O2 and are blocked
//! scenarios. The generated-resource hub's own stale-completion tests belong to the `df-tools`
//! owners and are cited in the acceptance matrix.

use df_acceptance::{
    CreditApplied, CreditOwner, SpendOwner, blocked_scenario, dollars, empty_credit_state,
    operation, spend_consent, spend_snapshot_limited_by_job, usd_micros,
};
use df_commerce::{
    CreditEvent, CreditRefusal, ObservationAuthority, SpendRefusal, SpendReservationStatus,
    SpendSettlementObservation, SpendSettlementOutcome, SpendSettlementRefusal,
};
use df_types::{LiabilityRate, OperationId, Usage, UsageUnit};

const IMAGE_PRICE_MICROS: u128 = 600_000;

fn owner_with_one_dispatched_dollar() -> (SpendOwner, OperationId) {
    let mut owner = SpendOwner::new(
        spend_snapshot_limited_by_job(dollars(10)),
        spend_consent(dollars(1)),
    );
    let dispatched = operation(1);
    owner
        .admit(dispatched, dollars(1))
        .expect("a one dollar maximum fits");
    (owner, dispatched)
}

fn known_settlement(
    owner: &SpendOwner,
    operation: OperationId,
    images: u128,
    billed_waste_micros: u128,
) -> SpendSettlementObservation {
    let rate = LiabilityRate::new(
        dollars(0).currency(),
        UsageUnit::Image,
        IMAGE_PRICE_MICROS,
        1,
    )
    .expect("one image is a positive denominator");
    SpendSettlementObservation {
        authority: ObservationAuthority::VerifiedLatest,
        operation,
        expected_revision: owner.snapshot().revision,
        outcome: SpendSettlementOutcome::Known {
            actual_usage: Usage::new(images, UsageUnit::Image),
            billed_waste: usd_micros(billed_waste_micros),
            liability_rate: rate,
        },
    }
}

#[test]
fn job_01_an_unknown_outcome_keeps_full_exposure_and_cannot_be_admitted_again() {
    let (mut owner, dispatched) = owner_with_one_dispatched_dollar();
    owner.mark_outcome_unknown(dispatched);
    let exposure_before = owner.snapshot();

    let resend = owner.admit(dispatched, dollars(1));
    let unknown_settlement = owner.settle(SpendSettlementObservation {
        authority: ObservationAuthority::VerifiedLatest,
        operation: dispatched,
        expected_revision: exposure_before.revision,
        outcome: SpendSettlementOutcome::Unknown,
    });

    assert_eq!(resend.err(), Some(SpendRefusal::UnknownOperation));
    assert_eq!(
        unknown_settlement.err(),
        Some(SpendSettlementRefusal::UnknownLiability)
    );
    assert_eq!(owner.snapshot(), exposure_before);
    assert!(
        exposure_before
            .counters
            .iter()
            .all(|c| c.used == dollars(1)),
        "the worst-case maximum stays reserved in every scope"
    );
    assert_eq!(
        owner.reservation(dispatched).map(|r| r.status),
        Some(SpendReservationStatus::UnknownLiability)
    );
}

#[test]
fn job_01_a_recorded_dispatch_is_never_admitted_a_second_time() {
    let (mut owner, dispatched) = owner_with_one_dispatched_dollar();
    let exposure_before = owner.snapshot();

    let second = owner.admit(dispatched, dollars(1));

    assert_eq!(second.err(), Some(SpendRefusal::DuplicateOperation));
    assert_eq!(owner.snapshot(), exposure_before);
}

#[test]
fn job_01_unverified_evidence_cannot_release_exposure() {
    let (mut owner, dispatched) = owner_with_one_dispatched_dollar();
    let exposure_before = owner.snapshot();
    let mut claim = known_settlement(&owner, dispatched, 0, 0);
    claim.authority = ObservationAuthority::Unverified;

    let result = owner.settle(claim);

    assert_eq!(result.err(), Some(SpendSettlementRefusal::Unverified));
    assert_eq!(owner.snapshot(), exposure_before);
}

#[test]
fn settlement_policy_a_second_completion_for_a_settled_operation_is_rejected() {
    let (mut owner, dispatched) = owner_with_one_dispatched_dollar();
    let first = known_settlement(&owner, dispatched, 1, 0);
    owner.settle(first).expect("the first completion settles");
    let settled = owner.snapshot();

    let duplicate = owner.settle(known_settlement(&owner, dispatched, 1, 0));
    let from_before_the_settlement = owner.settle(first);

    assert_eq!(
        duplicate.err(),
        Some(SpendSettlementRefusal::AlreadySettled)
    );
    assert_eq!(
        from_before_the_settlement.err(),
        Some(SpendSettlementRefusal::StaleRevision)
    );
    assert_eq!(owner.snapshot(), settled);
}

blocked_scenario! {
    fn job_01_a_timeout_after_the_supplier_may_have_accepted_retains_exposure_and_never_resends,
    blocked_on: "blocked on O1 and F API: the egress dispatcher's send sequence and record_transport_outcome (OutcomeUnknown), with a fake provider that accepts then drops the reply",
    setup: [
        "fake provider that records every request, accepts the request, then drops the connection before replying",
        "payer A funds a $0.90 retail quote with a $0.60 supplier maximum; the admission is committed and the attempt is DispatchPrepared",
    ],
    steps: [
        "the dispatcher sends the request and observes the timeout; record_transport_outcome moves the attempt to OutcomeUnknown",
        "run the settlement worker for ten more rounds, restart the runner, and recover the owner fence",
        "assert: the provider recorded exactly one request (no blind resend); a second admission for the attempt is refused",
        "assert: supplier exposure stays at the $0.60 maximum in 9100 and the retail hold is shown as unresolved, not as a charge",
        "assert: after hold.unresolved_release_minutes the customer hold is released while supplier exposure stays open (the_customer_hold_is_released_after_fifteen_minutes_while_supplier_exposure_stays_open)",
        "assert: only a same-key reconciliation lookup may change the outcome",
    ],
    outcome: "Unknown exposure retained; no blind resend",
}

blocked_scenario! {
    fn job_02_an_old_completion_after_invalidation_or_replacement_is_rejected,
    blocked_on: "blocked on F and O2 API: one AttemptId spanning the catalog, the hub and settlement, and decide_settlement for a superseded attempt (finance design 12.2, asset design 7)",
    setup: [
        "resource R requested as generation 1 (attempt A1, operation O1), then invalidated and regenerated as generation 2 (attempt A2, operation O2); the two attempts have different attempt and operation ids",
        "the fake provider can complete either attempt at a time the test chooses",
    ],
    steps: [
        "complete A2 first and let it settle and publish",
        "then deliver A1's completion, which is old and carries a valid payload",
        "assert: A1's completion is rejected as stale at staging and at settlement, and logged; R still shows A2's output",
        "assert: the customer is charged nothing for A1 and O2's settlement is unchanged (a_settlement_for_a_superseded_attempt_charges_the_customer_nothing)",
        "repeat with the owner fence advanced between dispatch and completion: the completion from the old fence is rejected too",
    ],
    outcome: "Stale attempt rejected",
}

blocked_scenario! {
    fn job_03_an_exhausted_provider_quota_opens_a_circuit_breaker_without_a_retry_storm,
    blocked_on: "blocked on O1 API: the provider circuit breaker and the customer notice in the production admission path (the classifier's retry bound is checked in provider_outcome.rs)",
    setup: [
        "fake provider that answers quota exhausted (HTTP 429 with no retry-after) for every request",
        "a table with six players generating pictures at the Immersive preset",
    ],
    steps: [
        "request pictures until the breaker opens; keep requesting for five more minutes of controlled time",
        "assert: the provider saw no more than the breaker's configured number of requests (no retry storm)",
        "assert: a refused admission holds nothing and writes no attempt (a_refused_admission_holds_nothing_and_writes_no_attempt) and no customer charge was made",
        "assert: the table sees an actionable notice (optional pictures paused, essential narration continues) and the status is visible to the operator",
        "advance the controlled clock past the half-open interval: exactly one probe request is sent",
    ],
    outcome: "Circuit breaker; actionable notice; no retry storm",
}

#[test]
fn job_04_a_credit_for_a_technical_failure_returns_the_charge_exactly_once() {
    let mut owner = CreditOwner::new(empty_credit_state());
    let charge = operation(2);
    owner
        .apply(
            operation(1),
            CreditEvent::SettledTopup { amount: dollars(5) },
            ObservationAuthority::VerifiedLatest,
        )
        .expect("funded");
    owner
        .apply(
            charge,
            CreditEvent::Consume { amount: dollars(2) },
            ObservationAuthority::VerifiedLatest,
        )
        .expect("the charge is accepted");
    let reversal = CreditEvent::ReverseConsumption {
        original_operation: charge,
        amount: dollars(2),
    };

    let credited = owner.apply(operation(3), reversal, ObservationAuthority::VerifiedLatest);
    let credited_again = owner.apply(operation(4), reversal, ObservationAuthority::VerifiedLatest);

    assert_eq!(credited, Ok(CreditApplied::Committed));
    assert_eq!(credited_again, Err(CreditRefusal::ExceedsOriginalCharge));
    assert_eq!(owner.available(), dollars(5));
    assert_eq!(owner.state().earned_credit, dollars(0));
}

#[test]
fn job_04_a_credit_cannot_exceed_or_invent_the_original_charge() {
    let mut owner = CreditOwner::new(empty_credit_state());
    owner
        .apply(
            operation(1),
            CreditEvent::SettledTopup { amount: dollars(5) },
            ObservationAuthority::VerifiedLatest,
        )
        .expect("funded");
    owner
        .apply(
            operation(2),
            CreditEvent::Consume { amount: dollars(2) },
            ObservationAuthority::VerifiedLatest,
        )
        .expect("the charge is accepted");

    let too_much = owner.apply(
        operation(3),
        CreditEvent::ReverseConsumption {
            original_operation: operation(2),
            amount: usd_micros(2_000_001),
        },
        ObservationAuthority::VerifiedLatest,
    );
    let never_charged = owner.apply(
        operation(4),
        CreditEvent::ReverseConsumption {
            original_operation: operation(9),
            amount: dollars(1),
        },
        ObservationAuthority::VerifiedLatest,
    );

    assert_eq!(too_much, Err(CreditRefusal::ExceedsOriginalCharge));
    assert_eq!(never_charged, Err(CreditRefusal::MissingOriginalCharge));
    assert_eq!(owner.available(), dollars(3));
}

blocked_scenario! {
    fn job_04_invalid_or_duplicate_output_is_credited_or_not_charged_by_the_documented_policy,
    blocked_on: "blocked on F and O2 API: decide_settlement over SupplierOutcome and Usability, with O2's FinalizeOutcome for the media (finance design 12.2, 12.5)",
    setup: [
        "fake provider returning, in turn: a valid image, an empty file, a corrupt image, and the same image bytes for a second distinct request",
        "payer A with $5.00 funded to session S; each request is quoted at $0.90 retail",
    ],
    steps: [
        "settle each of the four requests",
        "assert: the valid image is charged at or below its quote and the surplus is released",
        "assert: invalid output is not charged and its supplier cost is company loss (invalid_output_is_not_charged_and_its_supplier_cost_is_company_loss); output lost after provider success is not charged",
        "assert: the duplicate is charged once (duplicate_output_is_charged_once)",
        "assert: a technical failure credit returns the charge once to the original lots (a_technical_failure_credit_returns_the_charge_once_to_the_original_lots)",
        "assert: every charge or credit references the quote and the usage event, and the journal balances",
    ],
    outcome: "Correct no-charge/credit policy",
}

#[test]
fn settlement_policy_a_late_result_settles_the_unknown_liability_left_by_a_released_customer_hold()
{
    let (mut owner, dispatched) = owner_with_one_dispatched_dollar();
    owner.release_customer_hold(dispatched);

    let late = owner
        .settle(known_settlement(&owner, dispatched, 1, 0))
        .expect("the supplier liability is still open, so the late result settles it");

    assert_eq!(late.actual_supplier_liability, usd_micros(600_000));
    assert_eq!(late.known_unused_release, usd_micros(400_000));
    assert_eq!(late.platform_loss, usd_micros(0));
    assert_eq!(
        owner.reservation(dispatched).map(|r| r.status),
        Some(SpendReservationStatus::SettledKnown)
    );
}

#[test]
fn job_05_a_supplier_overrun_is_company_loss_and_never_raises_customer_exposure() {
    let (mut owner, dispatched) = owner_with_one_dispatched_dollar();
    let before = owner.snapshot();
    // Two images at $0.60 plus $0.10 billed waste is $1.30 against a $1.00 maximum.
    let overrun = known_settlement(&owner, dispatched, 2, 100_000);

    let proposal = owner.settle(overrun).expect("a verified overrun settles");

    assert_eq!(proposal.platform_loss, usd_micros(300_000));
    assert!(proposal.supplier_admissions_blocked);
    let after = owner.snapshot();
    for (scope, (new, old)) in after.counters.iter().zip(&before.counters).enumerate() {
        let is_company_scope = scope < 3;
        let expected_micros = old.used.micros() + if is_company_scope { 300_000 } else { 0 };
        assert_eq!(new.used.micros(), expected_micros, "scope {scope}");
    }
}

blocked_scenario! {
    fn job_05_a_late_result_after_the_customer_hold_is_released_is_company_funded,
    blocked_on: "blocked on F API: the deadline worker (hold.unresolved_release_minutes) and the late_result_after_release loss class 5200 (finance design 12.3, 15.1)",
    setup: [
        "fake provider that completes a $0.60-maximum job 20 minutes after dispatch, with a controlled clock",
        "the job is Sent with an unknown outcome; payer A did not elect to wait",
    ],
    steps: [
        "advance the clock past the disclosed 15 minute deadline; the customer hold is released and A's available credit is restored",
        "deliver the late result and settle it",
        "assert: A is charged nothing; the supplier cost is recorded as company expense (5200 late_result_after_release) against the retained supplier liability",
        "assert: A's statement shows released_not_charged and no new line, and the output is delivered or archived by the documented policy",
        "assert: this is the finance design's a_late_result_after_the_customer_hold_was_released_is_company_funded",
    ],
    outcome: "No surprise retroactive charge",
}

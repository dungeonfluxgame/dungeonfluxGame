//! Plan section 17, REF-01 to REF-03: refunds, timeouts and disputes (plan 6.7 and 5.5).
//!
//! Real tests drive the pure credit policy through the fake credit owner (the policy half;
//! `credit.rs` retires with decision 30 and they retarget to `df-finance`). Multi-lot refunds with
//! tax and processor references, and per-contributor disputes, need F's funding lots and refund
//! and dispute plans and are blocked scenarios. The partial-refund arithmetic check is a
//! building block, not REF-01: it says nothing about lots, tax or processor references.

use df_acceptance::{
    CreditApplied, CreditOwner, blocked_scenario, dollars, empty_credit_state, operation,
};
use df_commerce::{CreditEvent, CreditRefusal, ObservationAuthority, RefundOutcome};
use df_types::{Money, OperationId};

const TOPUP: u64 = 1;
const HOLD: u64 = 2;

fn funded_owner(amount_dollars: u128) -> CreditOwner {
    let mut owner = CreditOwner::new(empty_credit_state());
    apply(
        &mut owner,
        TOPUP,
        CreditEvent::SettledTopup {
            amount: dollars(amount_dollars),
        },
    )
    .expect("funded");
    owner
}

fn apply(
    owner: &mut CreditOwner,
    sequence: u64,
    event: CreditEvent,
) -> Result<CreditApplied, CreditRefusal> {
    owner.apply(
        operation(sequence),
        event,
        ObservationAuthority::VerifiedLatest,
    )
}

fn hold_refund(owner: &mut CreditOwner, amount: Money) {
    apply(owner, HOLD, CreditEvent::HoldRefund { amount }).expect("the refund hold is accepted");
}

fn observe_refund(
    owner: &mut CreditOwner,
    sequence: u64,
    held: Money,
    object_revision: u64,
    outcome: RefundOutcome,
) -> Result<CreditApplied, CreditRefusal> {
    let refund_operation: OperationId = operation(HOLD);
    apply(
        owner,
        sequence,
        CreditEvent::ObserveRefund {
            refund_operation,
            held,
            object_revision,
            outcome,
        },
    )
}

#[test]
fn ref_02_a_refund_that_times_out_keeps_the_amount_out_of_both_cash_and_credit() {
    let mut owner = funded_owner(20);
    hold_refund(&mut owner, dollars(20));

    observe_refund(&mut owner, 3, dollars(20), 1, RefundOutcome::Unknown)
        .expect("an unknown outcome is recorded");

    let state = owner.state();
    assert_eq!(state.refund_hold, dollars(20));
    assert_eq!(
        state.refunded_principal,
        dollars(0),
        "no cash is claimed paid"
    );
    assert_eq!(owner.available(), dollars(0), "and no credit is reusable");
    assert_eq!(
        apply(&mut owner, 4, CreditEvent::Consume { amount: dollars(1) }),
        Err(CreditRefusal::InsufficientAvailableCredit)
    );
}

#[test]
fn ref_02_a_second_refund_request_cannot_hold_what_the_first_already_holds() {
    let mut owner = funded_owner(20);
    apply(
        &mut owner,
        HOLD,
        CreditEvent::HoldRefund {
            amount: dollars(15),
        },
    )
    .expect("the first request holds $15.00");

    let second = apply(
        &mut owner,
        5,
        CreditEvent::HoldRefund {
            amount: dollars(15),
        },
    );

    assert_eq!(second, Err(CreditRefusal::InsufficientAvailableCredit));
    assert_eq!(owner.state().refund_hold, dollars(15));
}

#[test]
fn ref_02_a_definitive_failure_returns_the_hold_to_credit_and_pays_no_cash() {
    let mut owner = funded_owner(20);
    hold_refund(&mut owner, dollars(20));
    observe_refund(&mut owner, 3, dollars(20), 1, RefundOutcome::Unknown).expect("unknown");

    observe_refund(&mut owner, 4, dollars(20), 2, RefundOutcome::KnownFailure)
        .expect("the processor definitively failed the refund");

    assert_eq!(owner.state().refund_hold, dollars(0));
    assert_eq!(owner.state().refunded_principal, dollars(0));
    assert_eq!(owner.available(), dollars(20));
}

#[test]
fn ref_02_a_definitive_success_pays_cash_once_and_a_replayed_observation_changes_nothing() {
    let mut owner = funded_owner(20);
    hold_refund(&mut owner, dollars(20));
    observe_refund(&mut owner, 3, dollars(20), 1, RefundOutcome::Unknown).expect("unknown");
    observe_refund(&mut owner, 4, dollars(20), 2, RefundOutcome::KnownSuccess).expect("paid");
    let paid = owner.state();

    let replay = observe_refund(&mut owner, 4, dollars(20), 2, RefundOutcome::KnownSuccess);
    let second_success = observe_refund(&mut owner, 5, dollars(20), 3, RefundOutcome::KnownSuccess);

    assert_eq!(paid.refunded_principal, dollars(20));
    assert_eq!(paid.deferred_credit, dollars(0));
    assert_eq!(replay, Ok(CreditApplied::Replayed));
    assert_eq!(second_success, Err(CreditRefusal::MissingRefund));
    assert_eq!(owner.state(), paid);
}

#[test]
fn credit_policy_a_partial_refund_leaves_the_remainder_spendable_and_the_principal_balanced() {
    let mut owner = funded_owner(20);
    hold_refund(&mut owner, dollars(5));
    observe_refund(&mut owner, 3, dollars(5), 1, RefundOutcome::KnownSuccess).expect("paid");

    let state = owner.state();

    assert_eq!(state.refunded_principal, dollars(5));
    assert_eq!(owner.available(), dollars(15));
    assert_eq!(
        state
            .deferred_credit
            .checked_add(state.earned_credit)
            .and_then(|sum| sum.checked_add(state.refunded_principal)),
        Ok(state.topup_principal)
    );
}

blocked_scenario! {
    fn ref_01_a_partial_refund_spanning_several_lots_keeps_sources_tax_and_processor_references,
    blocked_on: "blocked on F API: df_commerce::refund::plan_refund over funding lots with per-lot tax evidence and processor references (finance design 14.1)",
    setup: [
        "payer A bought lot L1 ($20.00, tax $1.60, charge ch_1) and lot L2 ($10.00, tax $0.80, charge ch_2) and spent $12.00",
        "fake gateway that records every refund call with its idempotency key, charge reference and amount in cents",
    ],
    steps: [
        "request a refund of $15.00 of the unspent purchased credit",
        "assert: the preview lists the exact lots, amounts, released allocations, pending holds and destination card suffix",
        "assert: the refund takes the newest lots first, each processor call names its own charge and idempotency key (a_partial_refund_across_lots_takes_newest_lots_first_with_their_processor_references_and_tax)",
        "assert: tax is reversed proportionally per lot; sub-cent amounts become an explicit rounding entry that cannot favour repeated micro-refunds",
        "assert: promotional credit is excluded and no refund exceeds a lot's unspent purchased amount",
    ],
    outcome: "Correct original sources, tax, and processor references",
}

blocked_scenario! {
    fn ref_02_the_refund_service_survives_a_timeout_with_one_idempotent_processor_call,
    blocked_on: "blocked on F and B API: the refund intent persisted before the processor call, observe_refund_lot and same-key reconcile against the fake processor (finance design 14.1)",
    setup: [
        "fake gateway that applies the refund and then drops the reply (timeout after possible send)",
        "payer A with $20.00 of unspent purchased credit",
    ],
    steps: [
        "request a $20.00 refund and let the call time out; restart the service and retry the request with the same idempotency key",
        "run the reconciler until the gateway answers definitively",
        "assert: the processor applied exactly one refund (a_repeated_refund_request_makes_one_processor_call) and the ledger shows exactly one refund payable settled",
        "assert: until the answer was definitive the amount was neither available credit nor reported refunded, and the UI said refund pending (a_refund_whose_processor_call_times_out_stays_payable_and_never_returns_to_credit_by_itself)",
        "assert: a failed refund goes to review, not back to cash and credit (a_failed_refund_goes_to_review_not_back_to_cash_and_credit)",
    ],
    outcome: "No double refund or premature reusable credit",
}

blocked_scenario! {
    fn ref_03_one_contributor_disputing_a_charge_never_shifts_cost_to_other_contributors,
    blocked_on: "blocked on F API: df_commerce::dispute::plan_dispute_open freezing one lot and its unspent allocations (finance design 14.2)",
    setup: [
        "payers A, B and C fund session S 50/25/25 and the session consumes $8.00 (A $4.00, B $2.00, C $2.00)",
        "the gateway then reports a dispute on B's purchase lot",
    ],
    steps: [
        "process the dispute event and run the worker",
        "assert: B's lot and B's unspent allocation to S are frozen; no other payer's balance or allocation is frozen",
        "assert: A and C are not debited for B's consumed $2.00; the consumed-service loss is recorded as company loss",
        "assert: the host's balance is untouched and the story, saves and export of the campaign remain available",
        "assert: this is the finance design's one_contributors_lost_dispute_never_debits_another_contributor and a_release_into_a_charged_back_lot_is_a_recovery_not_credit",
    ],
    outcome: "No charge shifted to another contributor",
}

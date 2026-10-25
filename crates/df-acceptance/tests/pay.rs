//! Plan section 17, PAY-01 to PAY-06: purchases, webhooks and refunds must never create credit
//! without authoritative payment success, and never create it twice.
//!
//! The real tests drive the pure credit policy (`df-commerce` `credit.rs`) through the fake credit
//! owner. They are the policy half only. `credit.rs` retires with decision 30 and these tests are
//! retargeted to `df-finance` (finance design 26.1) when it lands. The system halves need B's
//! purchase, inbox and processor adapter and F's `apply_processor_state`, and are blocked scenarios.

use df_acceptance::{
    CreditApplied, CreditOwner, blocked_scenario, dollars, empty_credit_state, operation,
};
use df_commerce::{CreditEvent, CreditRefusal, ObservationAuthority, RefundOutcome};

fn topup(amount_dollars: u128) -> CreditEvent {
    CreditEvent::SettledTopup {
        amount: dollars(amount_dollars),
    }
}

fn owner_with_settled_topup(sequence: u64, amount_dollars: u128) -> CreditOwner {
    let mut owner = CreditOwner::new(empty_credit_state());
    let applied = owner.apply(
        operation(sequence),
        topup(amount_dollars),
        ObservationAuthority::VerifiedLatest,
    );
    assert_eq!(applied, Ok(CreditApplied::Committed));
    owner
}

#[test]
fn pay_01_an_unverified_redirect_mints_no_credit() {
    let mut owner = CreditOwner::new(empty_credit_state());

    let result = owner.apply(operation(1), topup(20), ObservationAuthority::Unverified);

    assert_eq!(result, Err(CreditRefusal::Unverified));
    assert_eq!(owner.state(), empty_credit_state());
}

#[test]
fn pay_01_a_webhook_alone_mints_no_credit_until_the_latest_object_is_refreshed() {
    let mut owner = CreditOwner::new(empty_credit_state());

    let result = owner.apply(operation(1), topup(20), ObservationAuthority::WebhookOnly);

    assert_eq!(result, Err(CreditRefusal::NeedsAuthoritativeRefresh));
    assert_eq!(owner.state(), empty_credit_state());
}

#[test]
fn pay_01_a_payment_whose_outcome_is_unknown_mints_no_credit() {
    let mut owner = CreditOwner::new(empty_credit_state());

    let result = owner.apply(
        operation(1),
        CreditEvent::UnknownTopup,
        ObservationAuthority::VerifiedLatest,
    );

    assert_eq!(result, Err(CreditRefusal::UnknownPayment));
    assert_eq!(owner.state(), empty_credit_state());
}

#[test]
fn pay_01_a_refused_webhook_does_not_burn_the_operation_so_the_verified_refresh_credits_once() {
    let mut owner = CreditOwner::new(empty_credit_state());
    let payment = operation(1);
    assert_eq!(
        owner.apply(payment, topup(20), ObservationAuthority::WebhookOnly),
        Err(CreditRefusal::NeedsAuthoritativeRefresh)
    );

    let refreshed = owner.apply(payment, topup(20), ObservationAuthority::VerifiedLatest);
    let replayed = owner.apply(payment, topup(20), ObservationAuthority::VerifiedLatest);

    assert_eq!(refreshed, Ok(CreditApplied::Committed));
    assert_eq!(replayed, Ok(CreditApplied::Replayed));
    assert_eq!(owner.available(), dollars(20));
}

blocked_scenario! {
    fn pay_01_a_forged_or_replayed_checkout_return_url_credits_nothing,
    blocked_on: "blocked on B and F API: the checkout return route and purchase status read model (B), apply_processor_state (F, finance design 18.6)",
    setup: [
        "signed-in payer A with a created purchase order P for $20.00 whose hosted checkout was never paid; the fake processor reports P unpaid",
        "a second payer B who is also signed in",
    ],
    steps: [
        "A requests the return URL with a forged success parameter, then with the session id of someone else's paid order, then replays a genuine success URL twice",
        "B requests A's return URL",
        "assert: every response says Confirming payment or not found, and the order stays unpaid until a verified webhook or an authoritative lookup says paid",
        "assert: spendable credit is $0.00 for A after every request and no journal entry exists for P",
        "assert: the return path calls no finance command (finance design test no_finance_call_exists_on_the_checkout_return_path)",
        "assert: B learns nothing about A's order",
    ],
    outcome: "No credit without authoritative payment success",
}

#[test]
fn pay_02_a_webhook_delivered_five_times_posts_one_credit() {
    let mut owner = CreditOwner::new(empty_credit_state());
    let payment = operation(1);

    let outcomes: Vec<_> = (0..5)
        .map(|_| owner.apply(payment, topup(20), ObservationAuthority::VerifiedLatest))
        .collect();

    assert_eq!(outcomes[0], Ok(CreditApplied::Committed));
    assert!(
        outcomes[1..]
            .iter()
            .all(|o| *o == Ok(CreditApplied::Replayed))
    );
    let state = owner.state();
    assert_eq!(state.topup_principal, dollars(20));
    assert_eq!(state.deferred_credit, dollars(20));
    assert_eq!(state.revision, 1, "only the first delivery changes state");
}

#[test]
fn pay_02_the_same_operation_with_a_different_amount_is_a_conflict_not_a_second_posting() {
    let mut owner = owner_with_settled_topup(1, 20);

    let result = owner.apply(
        operation(1),
        topup(50),
        ObservationAuthority::VerifiedLatest,
    );

    assert_eq!(result, Err(CreditRefusal::ConflictingOperation));
    assert_eq!(owner.state().deferred_credit, dollars(20));
}

#[test]
fn pay_02_a_replayed_success_after_a_full_refund_posts_nothing() {
    let mut owner = owner_with_settled_topup(1, 20);
    let hold = operation(2);
    owner
        .apply(
            hold,
            CreditEvent::HoldRefund {
                amount: dollars(20),
            },
            ObservationAuthority::VerifiedLatest,
        )
        .expect("a full refund hold is accepted");
    owner
        .apply(
            operation(3),
            CreditEvent::ObserveRefund {
                refund_operation: hold,
                held: dollars(20),
                object_revision: 1,
                outcome: RefundOutcome::KnownSuccess,
            },
            ObservationAuthority::VerifiedLatest,
        )
        .expect("the refund succeeded");
    let refunded = owner.state();

    let replayed_success = owner.apply(
        operation(1),
        topup(20),
        ObservationAuthority::VerifiedLatest,
    );

    assert_eq!(replayed_success, Ok(CreditApplied::Replayed));
    assert_eq!(owner.state(), refunded);
    assert_eq!(owner.available(), dollars(0));
}

blocked_scenario! {
    fn pay_02_the_same_payment_under_a_new_event_id_posts_once,
    blocked_on: "blocked on B and F API: the webhook inbox (B) and apply_processor_state with business-effect uniqueness on the payment, not only the event id (F, finance design 18.6)",
    setup: [
        "fake processor able to sign webhooks, purchase order P for payer A: $20.00, payment intent PI-1",
        "empty durable webhook inbox, empty journal, no funding lot",
    ],
    steps: [
        "deliver payment succeeded event E1 for PI-1 five times, running the inbox worker to quiescence each time",
        "deliver the same business effect again under new event ids E2 and E3, as a processor resend does",
        "assert: exactly one balanced journal posting references PI-1 (processor clearing debit $20.00, customer service liability credit $20.00)",
        "assert: exactly one funding lot exists for P and spendable purchased credit is $20.00",
        "assert: inbox rows are deduplicated by event id and all three event ids end in a terminal state",
        "assert: this is the finance design's the_same_payment_applied_five_times_under_new_event_ids_posts_one_purchase on PostgreSQL",
    ],
    outcome: "One business posting only",
}

#[test]
fn pay_03_a_late_older_refund_observation_cannot_release_a_hold() {
    let mut owner = owner_with_settled_topup(1, 20);
    let hold = operation(2);
    owner
        .apply(
            hold,
            CreditEvent::HoldRefund {
                amount: dollars(20),
            },
            ObservationAuthority::VerifiedLatest,
        )
        .expect("a full refund hold is accepted");
    owner
        .apply(
            operation(3),
            CreditEvent::ObserveRefund {
                refund_operation: hold,
                held: dollars(20),
                object_revision: 5,
                outcome: RefundOutcome::Unknown,
            },
            ObservationAuthority::VerifiedLatest,
        )
        .expect("an unknown refund keeps the hold");

    let older_failure = owner.apply(
        operation(4),
        CreditEvent::ObserveRefund {
            refund_operation: hold,
            held: dollars(20),
            object_revision: 4,
            outcome: RefundOutcome::KnownFailure,
        },
        ObservationAuthority::VerifiedLatest,
    );

    assert_eq!(older_failure, Err(CreditRefusal::StaleRefundObservation));
    assert_eq!(owner.state().refund_hold, dollars(20));
    assert_eq!(owner.available(), dollars(0));
}

#[test]
fn pay_03_a_refund_before_any_credit_exists_is_refused_so_the_owner_must_park_it() {
    // The policy refuses the refund; it cannot remember it. If the owner dropped the refused
    // event, a later success would mint spendable credit for a payment that is already refunded.
    // The blocked scenario below pins that the owner parks the event instead.
    let mut owner = CreditOwner::new(empty_credit_state());

    let result = owner.apply(
        operation(1),
        CreditEvent::HoldRefund {
            amount: dollars(20),
        },
        ObservationAuthority::VerifiedLatest,
    );

    assert_eq!(result, Err(CreditRefusal::InsufficientAvailableCredit));
    assert_eq!(owner.state(), empty_credit_state());
}

blocked_scenario! {
    fn pay_03_a_refund_event_before_the_success_event_never_mints_spendable_credit,
    blocked_on: "blocked on F API: apply_processor_state that reconciles by authoritative processor state, not event order (finance design 18.6)",
    setup: [
        "fake processor holding payment PI-1 ($20.00) whose authoritative state is already refunded in full",
        "empty inbox and ledger; webhook payloads for charge.refunded (R1) and payment succeeded (S1)",
    ],
    steps: [
        "deliver R1 first and run the worker; assert the refund is parked or recorded against PI-1, not dropped and not applied as a negative balance",
        "deliver S1 late and run the worker to quiescence; the state applied is the processor's refunded total",
        "assert: spendable purchased credit for the payer is $0.00 at every step",
        "assert: the funding lot for PI-1 never becomes spendable; the parked refund resolves against it",
        "assert: an older processor state arriving afterwards cannot lower the refunded total (an_older_processor_state_cannot_lower_the_refunded_total)",
        "assert: journal entries, if any, are balanced and net to zero customer liability",
    ],
    outcome: "No resurrected credit",
}

blocked_scenario! {
    fn pay_04_a_payment_that_succeeds_after_the_browser_closes_shows_funds_once_on_return,
    blocked_on: "blocked on B and F API: purchase status lookup by order id and the inbox worker (B), the resulting lot read (F, PurchaseStatusView)",
    setup: [
        "fake processor, signed-in payer A, purchase order P for $20.00 with a hosted checkout session",
        "the browser is closed at the checkout page: no return request is ever made during the payment",
    ],
    steps: [
        "the processor fires the payment succeeded webhook; the worker processes it with no client request open",
        "later, payer A opens the purchase status page and polls the read-only status endpoint twice",
        "assert: status is Succeeded and available purchased credit is exactly $20.00 after both polls",
        "assert: payer B requesting A's order id gets the same answer as for a missing order",
        "assert: while the webhook was unprocessed, the status page said Confirming payment and credit was $0.00",
    ],
    outcome: "Funds appear once on return",
}

#[test]
fn pay_05_a_candidate_built_from_an_old_revision_is_rejected_not_overwritten() {
    let mut owner = owner_with_settled_topup(1, 20);
    let stale_revision = 0;

    let lost_update = owner.apply_expecting(
        stale_revision,
        operation(2),
        topup(10),
        ObservationAuthority::VerifiedLatest,
    );
    let retried = owner.apply(
        operation(2),
        topup(10),
        ObservationAuthority::VerifiedLatest,
    );

    assert_eq!(lost_update, Err(CreditRefusal::StaleRevision));
    assert_eq!(retried, Ok(CreditApplied::Committed));
    assert_eq!(owner.state().topup_principal, dollars(30));
    assert_eq!(owner.available(), dollars(30));
}

blocked_scenario! {
    fn pay_05_two_concurrent_purchases_and_allocations_keep_their_own_sources,
    blocked_on: "blocked on F API: funding lots, allocate and purchase posting on PostgreSQL (finance design 7, 8, 17)",
    setup: [
        "payer A with no credit; session S with host H; two purchases P1 ($20.00) and P2 ($10.00) whose processor states are ready",
        "a second connection that runs the competing statement while the first holds its lock; repeat the run 50 times with shuffled order",
    ],
    steps: [
        "apply P1 and P2 and allocate $15.00 from A to S concurrently (controlled interleaving, asserted through pg_locks, not sleeps)",
        "assert: two funding lots exist, each attributed to its own purchase and payment intent",
        "assert: the allocation draws from lots in the documented order (trial first, then purchased) and records which lot funded which micro-dollars",
        "assert: available + allocated + reserved + consumed equals $30.00 and no update is lost in any of the 50 runs",
        "assert: every journal entry balances in USD (two_concurrent_purchases_and_allocations_keep_their_own_lots_and_lose_no_update)",
    ],
    outcome: "Correct source attribution and no lost updates",
}

blocked_scenario! {
    fn pay_06_one_card_used_by_two_accounts_exposes_nothing_about_the_other_account,
    blocked_on: "blocked on B and F API: card fingerprint linkage in the operator risk view (B) and the operator breakdown that is scoped per account (F)",
    setup: [
        "payers A and B are distinct verified accounts; both purchases are paid with the same card fingerprint F",
        "an operator with the risk-review capability and a support agent without it",
    ],
    steps: [
        "complete both purchases and open the risk review for F as the operator",
        "assert: the operator sees that A and B share F and sees only the fields the risk role allows",
        "assert: A's billing page, receipts, statements, exports and support view contain nothing identifying B, B's balance or B's campaigns, and vice versa",
        "assert: the operator breakdown for A lists none of B's lots or allocations (the_operator_breakdown_for_one_account_lists_none_of_another_accounts_lots_or_allocations)",
        "assert: the support agent without the risk capability cannot open the linkage",
    ],
    outcome: "Risk review does not expose unrelated data",
}

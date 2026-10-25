//! Plan section 17, POT-01 to POT-06: shared pots under concurrency, consent and rounding.
//!
//! The real tests drive the pure credit and supplier-exposure policies through fake owners, with
//! real threads racing for the last funds. Exhaustion is driven through the retail pot (credit)
//! and through the job counter, the only supplier counter that is bounded in hosted play (finance
//! design 3.2); no test pins a campaign, payer or platform cap. The allocation, withdrawal,
//! removal and epoch scenarios need F's pot and allocation model and are blocked scenarios.

use std::sync::{Barrier, Mutex, MutexGuard, PoisonError};
use std::thread;

use df_acceptance::{
    CreditApplied, CreditOwner, SpendOwner, blocked_scenario, dollars, empty_credit_state,
    operation, spend_consent, spend_snapshot_limited_by_job, usd_micros,
};
use df_commerce::{
    CreditEvent, CreditRefusal, ObservationAuthority, SpendOperationObservation,
    SpendOperationStatus, SpendRefusal, SpendRequest, SpendScope, propose_spend,
};

const RACING_REQUESTS: u64 = 6;

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn racer_count() -> usize {
    usize::try_from(RACING_REQUESTS).expect("a small count fits")
}

#[test]
fn pot_01_six_racing_consumptions_of_a_three_and_a_half_dollar_pot_never_go_negative() {
    let mut owner = CreditOwner::new(empty_credit_state());
    owner
        .apply(
            operation(100),
            CreditEvent::SettledTopup {
                amount: usd_micros(3_500_000),
            },
            ObservationAuthority::VerifiedLatest,
        )
        .expect("the pot is funded");
    let owner = Mutex::new(owner);
    let start = Barrier::new(racer_count());

    let outcomes: Vec<_> = thread::scope(|scope| {
        let racers: Vec<_> = (1..=RACING_REQUESTS)
            .map(|sequence| {
                let (owner, start) = (&owner, &start);
                scope.spawn(move || {
                    start.wait();
                    locked(owner).apply(
                        operation(sequence),
                        CreditEvent::Consume { amount: dollars(1) },
                        ObservationAuthority::VerifiedLatest,
                    )
                })
            })
            .collect();
        racers
            .into_iter()
            .map(|racer| racer.join().expect("a racer does not panic"))
            .collect()
    });

    let admitted = outcomes
        .iter()
        .filter(|o| **o == Ok(CreditApplied::Committed))
        .count();
    let refused = outcomes
        .iter()
        .filter(|o| **o == Err(CreditRefusal::InsufficientAvailableCredit))
        .count();
    assert_eq!((admitted, refused), (3, 3));
    let owner = owner.into_inner().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(owner.available(), usd_micros(500_000));
    assert_eq!(owner.state().earned_credit, dollars(3));
}

#[test]
fn pot_01_six_racing_dispatch_admissions_never_exceed_the_job_exposure_bound() {
    let owner = Mutex::new(SpendOwner::new(
        spend_snapshot_limited_by_job(dollars(3)),
        spend_consent(dollars(1)),
    ));
    let start = Barrier::new(racer_count());

    let outcomes: Vec<Result<(), SpendRefusal>> = thread::scope(|scope| {
        let racers: Vec<_> = (1..=RACING_REQUESTS)
            .map(|sequence| {
                let (owner, start) = (&owner, &start);
                scope.spawn(move || -> Result<(), SpendRefusal> {
                    start.wait();
                    loop {
                        let proposal = locked(owner).propose(operation(sequence), dollars(1))?;
                        match locked(owner).commit(proposal) {
                            Err(SpendRefusal::StaleRevision) => continue,
                            other => return other,
                        }
                    }
                })
            })
            .collect();
        racers
            .into_iter()
            .map(|racer| racer.join().expect("a racer does not panic"))
            .collect()
    });

    let admitted = outcomes.iter().filter(|o| o.is_ok()).count();
    let over_bound = outcomes
        .iter()
        .filter(|o| **o == Err(SpendRefusal::BudgetExceeded(SpendScope::Job)))
        .count();
    assert_eq!((admitted, over_bound), (3, 3));
    let owner = owner.into_inner().unwrap_or_else(PoisonError::into_inner);
    let snapshot = owner.snapshot();
    assert!(
        snapshot.counters.iter().all(|c| c.used == dollars(3)),
        "every scope carries exactly the three admitted maxima: {snapshot:?}"
    );
    assert_eq!(snapshot.revision, 3, "one revision per admitted dispatch");
}

blocked_scenario! {
    fn pot_01_six_concurrent_requests_exhaust_a_small_pot_on_the_production_repository,
    blocked_on: "blocked on F API: the admission transaction (decide_admission, stable-order locks over payer allocations) on PostgreSQL, finance design 11.2 and 19",
    setup: [
        "session S with two payers whose allocations total $3.50; each paid request has a $1.00 retail maximum",
        "a fake provider that records every dispatch; six connections that start together; repeat the run 100 times",
    ],
    steps: [
        "submit the six requests concurrently through the production admission path",
        "assert: exactly three are admitted and committed before any provider dispatch; the other three are refused with a typed insufficient-funds refusal",
        "assert: the provider received exactly three requests and no allocation, reservation or available balance is ever negative",
        "assert: lock acquisition order is stable, so no run deadlocks or times out",
        "assert: this is the finance design's six_concurrent_admissions_against_a_small_pot_never_overdraw_or_reserve_more_than_it_holds",
    ],
    outcome: "No negative balance or excess dispatch",
}

blocked_scenario! {
    fn pot_02_a_payer_who_withdraws_while_a_job_is_admitted_releases_only_unreserved_funds,
    blocked_on: "blocked on F API: allocate and withdraw_allocation(WithdrawRequest { amount: All | Exactly }) with an encumbered sub-balance (finance design 8, 18.3)",
    setup: [
        "payer A allocates $10.00 to session S; payer B allocates $10.00; split 50/50",
        "a $4.00 paid job J is admitted: reservation R holds A's $2.00 and B's $2.00 and is committed before dispatch",
    ],
    steps: [
        "A withdraws their whole allocation while J is in flight",
        "assert: A's released amount is exactly $8.00; the $2.00 hold stays attached to R",
        "settle J at $3.00 actual: A is charged $1.50 and B $1.50 from the saved shares; the surplus holds release",
        "assert: A's total available credit afterwards is exactly their purchase minus $1.50",
    ],
    outcome: "Only unreserved funds released",
}

#[test]
fn pot_03_a_request_above_the_payers_consent_is_refused_after_the_host_raises_the_session_limit() {
    let host_raised_job_bound = dollars(100);
    let mut owner = SpendOwner::new(
        spend_snapshot_limited_by_job(host_raised_job_bound),
        spend_consent(dollars(1)),
    );
    let before = owner.snapshot();

    let result = owner.admit(operation(1), dollars(2));

    assert_eq!(result.err(), Some(SpendRefusal::CustomerConsentExceeded));
    assert_eq!(owner.snapshot(), before, "a refusal touches no counter");
    assert_eq!(owner.reservation(operation(1)), None);
}

#[test]
fn pot_03_a_request_built_on_an_older_consent_revision_is_refused() {
    let owner = SpendOwner::new(
        spend_snapshot_limited_by_job(dollars(100)),
        spend_consent(dollars(1)),
    );
    let snapshot = owner.snapshot();
    let current_consent = owner.consent();

    let result = propose_spend(
        &snapshot,
        current_consent,
        SpendRequest {
            operation: operation(1),
            expected_revision: snapshot.revision,
            expected_consent_revision: current_consent.revision - 1,
            maximum_supplier_liability: dollars(1),
        },
        SpendOperationObservation {
            operation: operation(1),
            status: SpendOperationStatus::Unseen,
        },
    );

    assert_eq!(result.err(), Some(SpendRefusal::StaleConsent));
}

blocked_scenario! {
    fn pot_03_a_host_who_raises_the_rate_excludes_payers_who_did_not_consent,
    blocked_on: "blocked on F API: update_session_policy returning ConsentRequired { payers } or Updated { excluded_payers }, with per-payer consent versions (finance design 18.7)",
    setup: [
        "session S at the Story preset ($3.00 per hour); payers A and B consented to that ceiling only",
        "payer C consented to $20.00 per hour",
    ],
    steps: [
        "the host raises the table ceiling to $20.00 per hour",
        "assert: the update returns consent-required for A and B and a new policy version for C",
        "request a $6.00 job that needs the higher ceiling",
        "assert: the job is funded only from C's allocation; A and B are charged $0.00 and the UI shows the reduced funded runway",
        "assert: no pot value attributable to A or B changes (a_policy_raised_above_a_payers_consent_excludes_that_payer_from_new_work_until_they_consent)",
        "assert: a lowered ceiling refuses new work and keeps sent work (a_lowered_ceiling_refuses_new_work_and_keeps_sent_work)",
    ],
    outcome: "Incompatible payer is not charged without approval",
}

blocked_scenario! {
    fn pot_04_a_contributor_removed_mid_job_still_settles_their_original_hold,
    blocked_on: "blocked on F and S1 API: reservation_share saved per payer (F) and S1's campaign_event departure feed consumed by F",
    setup: [
        "payers A and B fund session S 50/50; job J ($4.00 maximum) is admitted and dispatched",
        "the host removes B from the campaign while J is running (S1 campaign_event with the departure)",
    ],
    steps: [
        "complete J at $3.00 actual",
        "assert: J settles against its original reservation shares, B is charged $1.50 and A $1.50",
        "request a new job K after the removal",
        "assert: K is funded only from A; B's remaining allocation is paused, not spent and not lost",
    ],
    outcome: "Original hold settles correctly; new work stops using their funds",
}

blocked_scenario! {
    fn pot_05_a_rate_or_split_change_applies_to_future_operations_only,
    blocked_on: "blocked on F API: funding epochs with reservation_share saved on each reservation (finance design 8.3)",
    setup: [
        "three payers authorize $20.00, $10.00 and $10.00 so the split is 50/25/25 in epoch 1",
        "operation O1 is reserved and settled in epoch 1",
    ],
    steps: [
        "payer C raises their authorization, creating epoch 2 with a new split",
        "reserve and settle operation O2",
        "assert: O1's recorded shares are unchanged and no completed charge is redistributed",
        "assert: O2's shares follow the epoch 2 split and each reservation names the epoch it used (an_epoch_change_mid_session_applies_to_later_reservations_only)",
    ],
    outcome: "New epoch affects future operations only",
}

blocked_scenario! {
    fn pot_06_the_production_allocator_meets_the_allocation_contract,
    blocked_on: "blocked on F API: df_commerce::pot::split_exactly and split_capped (finance design 3.3, 8.3)",
    setup: [
        "call df_acceptance::assert_allocation_contract(charge, weights, shares) with the shares split_exactly returns",
        "grid: every charge 0..=60 micro-dollars, four payers, authorization weights 0..=6, plus the $8.00 over $20/$10/$10 example (4/2/2)",
    ],
    steps: [
        "assert: the contract holds for every grid cell and the result is identical on a second call",
        "assert: permuting the payer order permutes the shares only among payers with equal authorization",
        "assert: a payer with zero authorization is never charged",
        "assert: split_capped at an exhausted payer never charges anyone above their own authorization and still sums to the charge",
        "assert: a thousand one-micro charges across three equal payers lose nothing, and the same code path allocates the reservation hold and the final settlement",
        "assert: this is the finance design's largest_remainder_splits_sum_exactly_and_never_raise_the_table_total",
    ],
    outcome: "Exact table total; no systematic rounding leakage",
}

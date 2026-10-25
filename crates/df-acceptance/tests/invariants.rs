//! Plan section 15.4: the mandatory behavioral invariants, each as an executable check.
//!
//! Real tests state the invariant against the public pure policies and the fake owners; they are
//! the policy half only (the credit policy retires with decision 30 and retargets to
//! `df-finance`). Invariants that need the production journal, runtime or UI are blocked
//! scenarios written against the committed designs.

use df_acceptance::{
    CreditApplied, CreditOwner, DIRECTORY_CAMPAIGN, DIRECTORY_ROLE, DIRECTORY_TENANT,
    MemberDirectory, SpendOwner, blocked_scenario, dollars, empty_credit_state, operation,
    spend_consent, spend_snapshot_limited_by_job, usd_micros,
};
use df_auth::membership::{
    MembershipError, MembershipRequest, authorize_membership, renew_membership,
    revalidate_membership,
};
use df_commerce::{
    CreditEvent, CreditState, ObservationAuthority, RefundOutcome, SpendRefusal, SpendScope,
    SpendSettlementObservation, SpendSettlementOutcome,
};
use df_types::{LiabilityRate, OperationId, Usage, UsageUnit};

const MEMBER: u8 = 10;
const STRANGER: u8 = 11;
const NOW_SECONDS: u64 = 1_000;
const SEEDS: u64 = 8;
const COMMANDS_PER_SEED: u32 = 600;
const MINIMUM_COMMITTED_PER_SEED: u32 = 100;

fn member_request(member: &u8) -> MembershipRequest<'_, MemberDirectory> {
    MembershipRequest {
        principal: member,
        tenant: &DIRECTORY_TENANT,
        campaign: &DIRECTORY_CAMPAIGN,
        role: &DIRECTORY_ROLE,
    }
}

struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

struct GeneratedCommand {
    operation: OperationId,
    event: CreditEvent,
    authority: ObservationAuthority,
    revision_behind: u64,
}

/// Draws random credit commands with a fresh operation id each, mostly verified and current, and
/// sometimes unverified, webhook-only or computed against an older revision.
struct CommandGenerator {
    random: Xorshift,
    next_sequence: u64,
    last_hold: Option<(OperationId, u128, u64)>,
    last_charge: Option<OperationId>,
}

impl CommandGenerator {
    fn new(seed: u64) -> Self {
        Self {
            random: Xorshift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)),
            next_sequence: 1,
            last_hold: None,
            last_charge: None,
        }
    }

    fn next_command(&mut self) -> GeneratedCommand {
        let operation_id = operation(self.next_sequence);
        self.next_sequence += 1;
        let amount = usd_micros(u128::from(self.random.below(5_000_000)) + 1);
        let event = self.event(operation_id, amount);
        let authority = match self.random.below(10) {
            0 => ObservationAuthority::Unverified,
            1 => ObservationAuthority::WebhookOnly,
            _ => ObservationAuthority::VerifiedLatest,
        };
        let revision_behind = u64::from(self.random.below(10) == 0);
        GeneratedCommand {
            operation: operation_id,
            event,
            authority,
            revision_behind,
        }
    }

    fn event(&mut self, operation_id: OperationId, amount: df_types::Money) -> CreditEvent {
        match self.random.below(5) {
            0 => CreditEvent::SettledTopup { amount },
            1 => {
                self.last_charge = Some(operation_id);
                CreditEvent::Consume { amount }
            }
            2 => {
                self.last_hold = Some((operation_id, amount.micros(), 0));
                CreditEvent::HoldRefund { amount }
            }
            3 => self.refund_observation(amount),
            _ => CreditEvent::ReverseConsumption {
                original_operation: self.last_charge.unwrap_or(operation_id),
                amount,
            },
        }
    }

    fn refund_observation(&mut self, fallback_amount: df_types::Money) -> CreditEvent {
        let outcome = match self.random.below(3) {
            0 => RefundOutcome::Unknown,
            1 => RefundOutcome::KnownSuccess,
            _ => RefundOutcome::KnownFailure,
        };
        match self.last_hold.as_mut() {
            Some((hold, held, revision)) => {
                *revision += 1;
                CreditEvent::ObserveRefund {
                    refund_operation: *hold,
                    held: usd_micros(*held),
                    object_revision: *revision,
                    outcome,
                }
            }
            None => CreditEvent::SettledTopup {
                amount: fallback_amount,
            },
        }
    }
}

fn assert_credit_invariants(seed: u64, owner: &CreditOwner) {
    let state: CreditState = owner.state();
    let accounted = state
        .deferred_credit
        .checked_add(state.earned_credit)
        .and_then(|sum| sum.checked_add(state.refunded_principal));
    assert_eq!(
        accounted,
        Ok(state.topup_principal),
        "seed {seed}: {state:?}"
    );
    assert!(
        state.refund_hold.micros() <= state.deferred_credit.micros(),
        "seed {seed}: a refund hold never exceeds the credit it holds"
    );
    assert!(
        owner.available().micros() <= state.deferred_credit.micros(),
        "seed {seed}: spendable credit never exceeds the credit held"
    );
}

#[test]
fn invariant_available_funds_never_go_negative_and_principal_always_balances() {
    for seed in 1..=SEEDS {
        let mut generator = CommandGenerator::new(seed);
        let mut owner = CreditOwner::new(empty_credit_state());
        let mut committed = 0_u32;
        for _ in 0..COMMANDS_PER_SEED {
            let command = generator.next_command();
            let expected_revision = owner
                .state()
                .revision
                .saturating_sub(command.revision_behind);
            let applied = owner.apply_expecting(
                expected_revision,
                command.operation,
                command.event,
                command.authority,
            );
            committed += u32::from(applied == Ok(CreditApplied::Committed));
            assert_credit_invariants(seed, &owner);
        }
        assert!(
            committed > MINIMUM_COMMITTED_PER_SEED,
            "seed {seed} exercised only {committed} commands"
        );
    }
}

blocked_scenario! {
    fn invariant_available_funds_cannot_become_negative_under_simultaneous_requests_on_the_production_path,
    blocked_on: "blocked on F API: the admission transaction on PostgreSQL with six real connections, and the enforced nonnegative balances (finance design 11.2, 17.1, 26.3)",
    setup: [
        "PostgreSQL schema with the finance migrations; payers with purchased and promotional lots and allocations to two sessions",
    ],
    steps: [
        "run random_finance_sequences_never_leave_an_enforced_balance_negative over seeded sequences, each step with a second connection racing the first",
        "assert: no enforced balance (available, allocated, reserved) is ever negative at any step, and a statement that would make one negative fails at commit",
        "assert: six_concurrent_admissions_against_a_small_pot_never_overdraw_or_reserve_more_than_it_holds holds on every run",
    ],
    outcome: "Available funds cannot become negative under simultaneous requests.",
}

blocked_scenario! {
    fn invariant_every_settled_customer_charge_maps_to_one_approved_quote_or_active_time_policy,
    blocked_on: "blocked on F API: settlement records linked to a quote or a meter segment (finance design 12, 13, 26.3 random_finance_sequences_map_every_charge_to_one_attempt_or_one_meter_segment)",
    setup: [
        "session S with price version PV1; payers fund it; one image quote Q1 ($0.90) approved, one active-time segment of 10 minutes",
    ],
    steps: [
        "run a mixed session of quoted jobs and metered time, including a refused job and a released hold",
        "query every settled customer charge in the ledger",
        "assert: each charge references exactly one approved quote id or exactly one meter segment, and that quote's ceiling is not exceeded",
        "assert: no charge exists for a refused or released operation, and no quote backs two charges",
    ],
    outcome: "Every settled customer charge maps to one approved quote or active-time policy.",
}

#[test]
fn invariant_every_admitted_dispatch_adds_exactly_its_maximum_to_every_scope_with_one_reservation()
{
    let mut owner = SpendOwner::new(
        spend_snapshot_limited_by_job(dollars(2)),
        spend_consent(dollars(1)),
    );
    let first = operation(1);
    let second = operation(2);
    let refused = operation(3);
    let third = operation(4);

    owner.admit(first, dollars(1)).expect("fits");
    owner.admit(second, usd_micros(500_000)).expect("fits");
    let over = owner.admit(refused, dollars(1));
    owner
        .admit(third, usd_micros(500_000))
        .expect("fits exactly");

    assert_eq!(
        over.err(),
        Some(SpendRefusal::BudgetExceeded(SpendScope::Job))
    );
    assert!(
        owner
            .snapshot()
            .counters
            .iter()
            .all(|counter| counter.used == dollars(2))
    );
    for admitted in [first, second, third] {
        assert!(owner.reservation(admitted).is_some());
    }
    assert_eq!(
        owner.reservation(refused),
        None,
        "a refusal reserves nothing"
    );
}

#[test]
fn invariant_a_settlement_below_the_maximum_releases_the_surplus_from_every_scope() {
    let mut owner = SpendOwner::new(
        spend_snapshot_limited_by_job(dollars(10)),
        spend_consent(dollars(1)),
    );
    let dispatched = operation(1);
    owner.admit(dispatched, dollars(1)).expect("fits");
    let rate = LiabilityRate::new(dollars(0).currency(), UsageUnit::Image, 600_000, 1)
        .expect("positive denominator");

    let proposal = owner
        .settle(SpendSettlementObservation {
            authority: ObservationAuthority::VerifiedLatest,
            operation: dispatched,
            expected_revision: owner.snapshot().revision,
            outcome: SpendSettlementOutcome::Known {
                actual_usage: Usage::new(1, UsageUnit::Image),
                billed_waste: dollars(0),
                liability_rate: rate,
            },
        })
        .expect("a verified settlement");

    assert_eq!(proposal.actual_supplier_liability, usd_micros(600_000));
    assert_eq!(proposal.known_unused_release, usd_micros(400_000));
    assert!(
        owner
            .snapshot()
            .counters
            .iter()
            .all(|counter| counter.used == usd_micros(600_000))
    );
}

blocked_scenario! {
    fn invariant_every_sent_supplier_attempt_maps_to_one_durable_exposure_record,
    blocked_on: "blocked on F and O1 API: mark_attempt_sent after the admission commit, and the dispatcher's sent-attempt log (finance design 18.2, 26.3 random_finance_sequences_map_every_sent_attempt_to_one_exposure_record)",
    setup: [
        "fake provider that records each received request with the provider idempotency key; the production dispatch path on PostgreSQL",
    ],
    steps: [
        "run 200 mixed jobs with crashes injected before DispatchPrepared, after it, and after Sent",
        "assert: for every request the provider received, exactly one exposure record existed and was committed before the request left the process",
        "assert: a job that crashed before DispatchPrepared produced no provider request",
    ],
    outcome: "Every sent supplier attempt maps to one durable exposure record.",
}

blocked_scenario! {
    fn invariant_every_monetary_journal_transaction_balances_in_its_currency,
    blocked_on: "blocked on F API: the append-only double-entry journal with the deferred balance constraint and reversing entries (finance design 17.1, 26.2)",
    setup: [
        "the full finance fixture run: purchase, fee, payout, allocation, consumption, supplier charge, refund, dispute, rounding and a corrective reversing entry",
    ],
    steps: [
        "read every journal entry through the read model",
        "assert: debits equal credits per entry, book and currency (random_finance_sequences_balance_every_entry_in_every_book_and_currency)",
        "assert: an unbalanced entry cannot commit and a line cannot be updated or deleted after posting; correction is only by a reversing entry",
        "assert: external-event uniqueness rejects a second posting for the same business effect",
    ],
    outcome: "Every monetary journal transaction balances in its currency.",
}

blocked_scenario! {
    fn invariant_a_stale_job_completion_cannot_settle_a_newer_generation_of_the_same_resource,
    blocked_on: "blocked on F and O2 API: one AttemptId across the catalog, the hub and settlement, with decide_settlement for a superseded attempt (see JOB-02)",
    setup: [
        "resource R at generation 2 with attempt A2; an in-flight completion for generation 1 (attempt A1, a different operation id) arrives afterwards",
    ],
    steps: [
        "deliver A1's completion for settlement and for publication",
        "assert: the completion is rejected as stale, nothing is settled or published for R, and the audit line records a stale result",
        "assert: the customer is charged nothing for A1 (a_settlement_for_a_superseded_attempt_charges_the_customer_nothing)",
    ],
    outcome: "A stale job completion cannot settle a newer generation of the same resource.",
}

#[test]
fn invariant_a_revoked_member_obtains_no_new_capability_and_no_old_one_stays_valid() {
    let mut directory = MemberDirectory::default().with_member(MEMBER, None);
    let capability = authorize_membership(&mut directory, member_request(&MEMBER), NOW_SECONDS)
        .expect("an active member is authorized");
    directory.revoke(MEMBER);

    let fresh = authorize_membership(&mut directory, member_request(&MEMBER), NOW_SECONDS);
    let still_valid = revalidate_membership(&mut directory, &capability, NOW_SECONDS);
    let renewed = renew_membership(&mut directory, &capability, NOW_SECONDS);

    assert_eq!(fresh.err(), Some(MembershipError::Revoked));
    assert_eq!(still_valid, Err(MembershipError::Revoked));
    assert_eq!(renewed.err(), Some(MembershipError::Revoked));
}

#[test]
fn invariant_a_principal_who_was_never_a_member_obtains_no_capability() {
    let mut directory = MemberDirectory::default().with_member(MEMBER, None);

    let stranger = authorize_membership(&mut directory, member_request(&STRANGER), NOW_SECONDS);

    assert_eq!(stranger.err(), Some(MembershipError::Missing));
}

#[test]
fn invariant_a_reinstated_member_needs_a_new_capability_because_the_old_basis_is_stale() {
    let mut directory = MemberDirectory::default().with_member(MEMBER, None);
    let old = authorize_membership(&mut directory, member_request(&MEMBER), NOW_SECONDS)
        .expect("an active member is authorized");
    directory.revoke(MEMBER);
    directory.reinstate(MEMBER);

    let old_still_valid = revalidate_membership(&mut directory, &old, NOW_SECONDS);
    let renewed = renew_membership(&mut directory, &old, NOW_SECONDS);

    assert_eq!(old_still_valid, Err(MembershipError::StaleRevision));
    assert!(renewed.is_ok());
}

#[test]
fn invariant_membership_fails_closed_when_the_source_is_down_or_the_grant_has_expired() {
    let mut directory = MemberDirectory::default().with_member(MEMBER, Some(NOW_SECONDS + 60));

    let before_expiry =
        authorize_membership(&mut directory, member_request(&MEMBER), NOW_SECONDS + 59);
    let at_expiry = authorize_membership(&mut directory, member_request(&MEMBER), NOW_SECONDS + 60);
    directory.set_unavailable(true);
    let source_down = authorize_membership(&mut directory, member_request(&MEMBER), NOW_SECONDS);

    assert!(before_expiry.is_ok());
    assert_eq!(at_expiry.err(), Some(MembershipError::Expired));
    assert_eq!(source_down.err(), Some(MembershipError::Source(())));
}

blocked_scenario! {
    fn invariant_every_acknowledged_game_decision_survives_an_ordinary_process_restart,
    blocked_on: "blocked on O1 API: the runner over PostgreSQL with the fenced decision commit and save points (hosted runtime design 9); the df-persistence owned-PostgreSQL suites are the nearest evidence",
    setup: [
        "campaign with six players on the production composition and a PostgreSQL instance the test owns",
    ],
    steps: [
        "submit 100 decisions, recording which ones were acknowledged as Saved before each of 20 forced process kills at random points",
        "restart the runner, resume, and read the authoritative state",
        "assert: every acknowledged decision is present exactly once and in order; an unacknowledged one is either absent or committed once, never duplicated",
    ],
    outcome: "Every acknowledged game decision survives ordinary process restart.",
}

blocked_scenario! {
    fn invariant_a_save_branch_or_restore_cannot_replay_payment_refund_or_supplier_dispatch,
    blocked_on: "blocked on O1 and F API: branch and restore as operations that take no financial input, and the per-campaign freeze after a restore (finance design 22.2, 26.1 no_finance_operation_accepts_a_game_revision_and_a_branch_creates_no_funding)",
    setup: [
        "campaign C with save point 5 and later ones; between them payer A purchased credit, was refunded part of it, and a paid image was dispatched",
        "the finance store and the game store are separate databases or schemas, as designed",
    ],
    steps: [
        "restore save point 5 in the game store and create a branch from it; replay the game events",
        "assert: A's balance, the refund and the dispatched image are unchanged; the restore reopened no consumed authorization",
        "assert: replay used the recorded image and made no provider request and no processor call (the replay-mode half is checked in provider_outcome.rs)",
        "restore the finance store to before the purchase and assert paid work stays frozen until the journal tail is replayed",
    ],
    outcome: "A save branch or restore cannot replay payment, refund, or supplier dispatch.",
}

blocked_scenario! {
    fn invariant_telemetry_failure_keeps_game_state_while_financial_admission_failure_blocks_paid_work,
    blocked_on: "blocked on O1 and F API: the production composition with an injectable telemetry sink and an injectable admission endpoint (hosted runtime design 10, finance design 18.1)",
    setup: [
        "production composition with a telemetry exporter that can fail and a finance admission endpoint that can be made unavailable",
    ],
    steps: [
        "fail telemetry export during play and commit five decisions",
        "assert: all five decisions are durable and the game continues",
        "make the finance admission endpoint unavailable and request a paid picture",
        "assert: no provider request is sent, the table sees the paused-media notice, and free play and saving continue",
    ],
    outcome: "Failure of telemetry does not erase game state, while failure of durable financial admission blocks new paid work.",
}

blocked_scenario! {
    fn invariant_a_saved_or_paid_message_corresponds_to_a_durable_authoritative_outcome,
    blocked_on: "blocked on S1, B, F and O1 API: customer-facing Saved and Paid statuses sourced only from committed records (PurchaseStatusView, the Saved acknowledgement)",
    setup: [
        "fault injection that fails the database commit after the handler has computed its response, for a game decision and for a purchase",
    ],
    steps: [
        "perform each action with the commit failing, then with the commit succeeding but the response lost",
        "assert: with a failed commit neither Saved nor Paid is shown and a retry shows the truth",
        "assert: with a lost response a retry with the same idempotency key returns the original result and the message appears once",
    ],
    outcome: "A customer-visible saved or paid message corresponds to a durable authoritative outcome.",
}

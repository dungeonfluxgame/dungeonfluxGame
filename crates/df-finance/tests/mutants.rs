#![cfg(not(target_arch = "wasm32"))]

//! Mutant pairs (planning/finance-ledger-research.md 26.5): each test removes or breaks one guard of
//! the boundary in a scratch database, runs the property scenario against it, and requires the
//! attack that guard stops to get through, or to be stopped for another reason than the one the
//! catalog pins. A guard no test can show necessary is not known to be a guard.

mod support;

use support::mutants::Mutant;
use support::scenario::scenario;

const SEED: u64 = 0x9e37_79b9_7f4a_7c15;

/// The scenario with `mutants` applied must fail, and its failure must name `expected`.
async fn must_fail(mutants: &[Mutant], expected: &str) {
    match scenario(mutants, SEED).await {
        Ok(log) if log == ["SKIPPED"] => {}
        Ok(log) => {
            panic!("{mutants:?} survived: the scenario passed with the guard removed\n{log:#?}")
        }
        Err(failure) => {
            println!("{mutants:?}: {failure}");
            assert!(
                failure.contains(expected),
                "{mutants:?} was caught, but not by what it should be: {failure}"
            );
        }
    }
}

macro_rules! mutant_pair {
    ($name:ident, $mutant:expr, $expected:expr) => {
        #[test]
        fn $name() {
            support::run(must_fail(&[$mutant], $expected));
        }
    };
}

mutant_pair!(
    without_the_balance_check_an_unbalanced_entry_is_accepted,
    Mutant::NoBalanceCheck,
    "attack accepted: an unbalanced entry"
);
mutant_pair!(
    a_memo_kept_in_session_settings_would_let_a_caller_skip_the_balance_check,
    Mutant::MemoInSessionSettings,
    "attack accepted: an unbalanced entry from a session with the check pre-set"
);
mutant_pair!(
    without_the_allowlist_a_kind_may_post_any_account,
    Mutant::NoAllowlist,
    "attack accepted: an account the kind may not post"
);
mutant_pair!(
    without_lot_conservation_a_lot_may_claim_more_than_its_keys_hold,
    Mutant::NoLotConservation,
    "attack accepted: lot totals that do not add up"
);
mutant_pair!(
    without_the_non_negativity_check_credit_can_be_overdrawn,
    Mutant::NoNonNegativity,
    "attack accepted: an overdraw of the payer's credit"
);
mutant_pair!(
    without_the_append_only_triggers_a_posted_entry_can_be_edited,
    Mutant::NoAppendOnly,
    "attack accepted: a posted entry edited in place"
);
mutant_pair!(
    without_the_write_guard_a_row_can_be_written_outside_commit_event,
    Mutant::NoWriteGuard,
    "attack accepted: a write outside commit_event"
);
mutant_pair!(
    without_the_trial_rules_an_account_can_be_granted_a_second_trial,
    Mutant::NoTrialRules,
    "attack accepted: a second trial grant for an account that has one"
);
mutant_pair!(
    without_the_column_allowlist_an_update_can_rewrite_any_column,
    Mutant::NoColumnAllowlist,
    "attack accepted: an update of a column the change kind does not list"
);
mutant_pair!(
    without_the_anchor_expectations_an_entry_can_carry_forged_correlation,
    Mutant::NoAnchorExpectations,
    "attack accepted: a settlement with forged correlation columns"
);
mutant_pair!(
    without_the_back_dating_rule_an_entry_can_post_into_a_closed_month,
    Mutant::NoBackdatingCheck,
    "attack accepted: a back-dated entry"
);
mutant_pair!(
    without_the_system_actor_rule_a_system_entry_can_name_an_actor,
    Mutant::NoSystemActorCheck,
    "attack accepted: a system entry that names an actor"
);
mutant_pair!(
    without_the_closed_reason_list_an_entry_can_carry_free_text,
    Mutant::NoReasonCheck,
    "attack accepted: an entry with a free-text reason"
);

/// The event-level pass is an optimisation: with it gone, the per-row deferred triggers alone must
/// refuse every attack of the catalog, for the reason it pins, and the invariants must hold.
#[test]
fn the_deferred_backstop_triggers_alone_refuse_every_attack() {
    support::run(async {
        match scenario(&[Mutant::EventLevelPassSkipped], SEED).await {
            Ok(log) if log == ["SKIPPED"] => {}
            Ok(log) => println!("{log:#?}"),
            Err(failure) => panic!("the backstop alone let an attack through: {failure}"),
        }
    });
}

mutant_pair!(
    when_the_counter_write_drifts_the_charged_total_stops_matching_the_usage_events,
    Mutant::ChargedTotalDrifts,
    "charged total differs from the usage events"
);

/// The charged total reset is stopped twice: by E9 first, and behind it by the allocation's column
/// list. With E9 gone the attack still ends in the column list; with both gone it gets through.
#[test]
fn with_equation_e9_gone_the_column_lists_still_refuse_a_charged_total_reset() {
    support::run(must_fail(
        &[Mutant::EquationsAreNoOps(&["eq_e9"])],
        "a session charged total reset in an allocation: FN003 change session_funding_update may not set column",
    ));
}

#[test]
fn with_both_layers_gone_a_charged_total_reset_gets_through() {
    support::run(must_fail(
        &[
            Mutant::NoColumnAllowlist,
            Mutant::EquationsAreNoOps(&["eq_e9"]),
        ],
        "attack accepted: a session charged total reset in an allocation",
    ));
}

/// Each equation on its own: with it replaced by a no-op, the attack aimed at it no longer ends in
/// that equation's message (it is accepted, or another guard stops it, which the message shows).
macro_rules! equation_pair {
    ($name:ident, $function:expr, $attack:expr) => {
        #[test]
        fn $name() {
            support::run(must_fail(
                &[Mutant::EquationsAreNoOps(&[$function])],
                $attack,
            ));
        }
    };
}

equation_pair!(
    without_equation_e1_a_settlement_can_misstate_the_supplier_cost,
    "eq_e1",
    "a settlement that mis-states the supplier cost"
);
equation_pair!(
    without_equation_e2_a_settlement_reserved_debit_can_differ_from_its_shares,
    "eq_e2",
    "a settlement whose reserved debit differs from its shares"
);
equation_pair!(
    without_equation_e3_a_settlement_can_return_to_a_key_outside_the_reservation,
    "eq_e3",
    "a settlement that returns to a key outside the reservation"
);
equation_pair!(
    without_equation_e4_a_settlement_can_return_to_the_wrong_key,
    "eq_e4",
    "a settlement that returns to the wrong key"
);
equation_pair!(
    without_equation_e5_a_credit_can_misstate_the_reversal,
    "eq_e5",
    "a credit that mis-states the reversal"
);
equation_pair!(
    without_equation_e6_a_reservation_can_hold_more_than_its_maximum,
    "eq_e6",
    "a reservation that holds more than its maximum"
);
equation_pair!(
    without_equation_e7_an_allocation_can_move_more_than_it_draws,
    "eq_e7",
    "an allocation that moves more than it draws"
);
equation_pair!(
    without_equation_e8_a_grant_can_credit_more_than_its_lot,
    "eq_e8",
    "credit minted past its lot"
);

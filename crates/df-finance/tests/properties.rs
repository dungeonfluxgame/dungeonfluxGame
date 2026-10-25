#![cfg(not(target_arch = "wasm32"))]

//! The property scenario (planning/finance-ledger-research.md 17, 21): seeded random operations
//! (purchases, trial grants, allocations, admissions, sends, settlements, releases, credits), with
//! every invariant re-derived from the tables after each step (balance, non-negativity, balance rows
//! reconciling to the journal, lot conservation, the hash chain, the held totals), then the whole
//! attack catalog put in front of the boundary. The mutant tests of `tests/mutants.rs` run this
//! same scenario against a database with one guard removed.

mod support;

use support::mutants::Mutant;
use support::scenario::scenario;

#[test]
fn the_invariants_hold_through_random_operations_and_every_attack_is_refused() {
    support::run(async {
        for seed in [
            0x9e37_79b9_7f4a_7c15,
            0x1234_5678_9abc_def1,
            0x0bad_cafe_f00d_1234,
        ] {
            let log = scenario(&[], seed)
                .await
                .unwrap_or_else(|failure| panic!("seed {seed:#x}: {failure}"));
            for line in log {
                println!("{line}");
            }
        }
    });
}

async fn must_fail(mutant: Mutant, expected: &str) {
    match scenario(&[mutant], 0x9e37_79b9_7f4a_7c15).await {
        Ok(log) if log == ["SKIPPED"] => {}
        Ok(log) => {
            panic!("{mutant:?} survived: the scenario passed with the guard removed\n{log:#?}")
        }
        Err(failure) => {
            println!("{mutant:?}: {failure}");
            assert!(
                failure.contains(expected),
                "{mutant:?} was caught, but not by what it should be: {failure}"
            );
        }
    }
}

#[test]
fn with_drifting_balance_rows_the_reconciliation_fails() {
    support::run(must_fail(Mutant::BalanceRowDrifts, "does not conserve"));
}

#[test]
fn with_a_hash_that_skips_the_previous_hash_the_chain_fails() {
    support::run(must_fail(Mutant::HashIgnoresPrevious, "broken hash chain"));
}

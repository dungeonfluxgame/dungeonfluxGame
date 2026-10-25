//! The catalog contract suite run against the in-memory fake.

use crate::InMemoryCatalog;
use crate::catalog_contract as contract;
use crate::object_store_tests::block_on;

macro_rules! fake_contract {
    ($($scenario:ident),+ $(,)?) => {
        $(
            #[test]
            fn $scenario() {
                block_on(contract::$scenario(&InMemoryCatalog::new()));
            }
        )+
    };
}

fake_contract!(
    admission_reserves_once_and_refuses_a_second_live_row_for_the_key,
    admission_beyond_the_limit_is_refused_with_the_room_left_and_clears_when_space_frees,
    uploads_are_limited_by_their_in_flight_count,
    a_step_claim_is_exclusive_until_its_lease_expires_and_each_claim_has_a_new_owner,
    staging_is_recorded_once_and_only_for_the_current_attempt,
    finalize_commits_the_quota_and_reports_ready_exactly_once,
    finalize_names_only_written_winners_and_a_rewrite_under_a_new_write_succeeds,
    a_worker_whose_lease_was_replaced_can_neither_write_nor_finalize,
    an_expired_lease_cannot_start_a_write_even_before_anyone_replaces_it,
    rejection_releases_the_reservation_and_reports_once_and_frees_the_key,
    a_replaced_attempt_refuses_the_old_attempts_late_work_and_reports_superseded,
    the_orphan_sweep_removes_an_older_claims_writes_and_never_a_winner,
    a_write_a_sweep_marked_deleting_makes_finalize_answer_winner_missing,
    a_write_finalize_named_as_a_winner_is_no_longer_deletable,
    a_tombstone_frees_the_bytes_and_a_purge_claim_leaves_the_winners_to_the_purge,
    due_rows_are_those_without_a_live_lease_oldest_first,
    a_quota_full_finalize_rejects_an_upload_instead_of_overcommitting,
    generated_output_commits_even_beyond_its_reservation_and_the_scope_limit,
    a_late_finalize_after_a_takeover_finalized_reports_lease_lost_with_one_outbox_row,
    a_rejection_guard_never_ends_newer_work,
    a_staged_object_outside_quarantine_is_refused,
    a_reservation_too_large_to_add_is_refused_and_never_wraps,
);

#[test]
fn a_live_reference_refuses_the_fakes_tombstone_until_it_is_gone() {
    use crate::catalog_store::{CatalogStore, TombstoneResult};
    use crate::{TombstoneReason, catalog_contract::asset_id};
    let catalog = InMemoryCatalog::new();
    block_on(contract::make_ready(&catalog, 1, 1, 10));
    catalog.hold_live_reference(asset_id(1));
    assert_eq!(
        block_on(catalog.tombstone(asset_id(1), TombstoneReason::UserDelete, 20)).unwrap(),
        TombstoneResult::LiveReferences
    );
    assert_eq!(
        block_on(catalog.quota_usage(contract::scope()))
            .unwrap()
            .committed_bytes,
        60
    );
}

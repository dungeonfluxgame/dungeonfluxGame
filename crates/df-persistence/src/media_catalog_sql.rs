//! The media catalog's statements, one function per operation, each running inside a transaction
//! the caller owns (planning/asset-storage-research.md 6, 10, 16). Keeping the caller in charge of
//! the transaction lets F's admission transaction call `admit_in` directly, and lets the race tests
//! hold two transactions open.
//!
//! Lock order (D25): the quota row of the asset's charge scope first, then the asset row, then
//! variant, reference and other rows. The charge scope is read without a lock to find the quota
//! row; if it changed by the time the asset row is locked the call fails with `ScopeMoved`.

mod admission;
mod leases;
mod locks;
mod rows;
mod sweep;
mod transitions;

pub(crate) use admission::{admit_in, asset_in, lookup_in, variants_in};
pub(crate) use leases::{
    claim_step_in, intend_variant_in, mark_validated_in, mark_variant_written_in, record_staged_in,
    renew_step_in,
};
#[cfg(test)]
pub(crate) use locks::lock_quota;
pub(crate) use locks::quota_usage_in;
pub(crate) use rows::record_failure;
pub(crate) use sweep::{
    begin_variant_deletion_in, due_rows_in, forget_staged_in, forget_variant_in, orphan_staged_in,
    orphan_variants_in, outbox_entry_in, pending_outbox_in,
};
pub(crate) use transitions::{finalize_in, reject_in, supersede_in, tombstone_in};

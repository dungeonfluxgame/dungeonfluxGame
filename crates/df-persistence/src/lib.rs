//! Native PostgreSQL adapter for the canonical session transaction port.
//! No replacement model, authorization issuer, rules implementation or runtime is defined here.
#![forbid(unsafe_code)]

#[cfg(not(target_arch = "wasm32"))]
mod checkpoint_codec;
#[cfg(not(target_arch = "wasm32"))]
mod decision_adapter;
#[cfg(not(target_arch = "wasm32"))]
mod decision_rows;
#[cfg(not(target_arch = "wasm32"))]
pub mod local_demo_scope;
#[cfg(not(target_arch = "wasm32"))]
mod native_bridge;
#[cfg(not(target_arch = "wasm32"))]
mod native_connection;
#[cfg(not(target_arch = "wasm32"))]
mod native_scope;
#[cfg(not(target_arch = "wasm32"))]
mod revision_codec;
#[cfg(not(target_arch = "wasm32"))]
mod sql;
#[cfg(not(target_arch = "wasm32"))]
pub mod story_memory;

#[cfg(not(target_arch = "wasm32"))]
pub use checkpoint_codec::CodecLimits as NativeCodecLimits;
#[cfg(not(target_arch = "wasm32"))]
pub use decision_adapter::RecoverySource as NativeRecoverySource;
#[cfg(not(target_arch = "wasm32"))]
pub use native_bridge::{
    NativeRepositoryOptions, NativeSetupFailure, NativeVerifierSource, PostgresRepository,
};
#[cfg(not(target_arch = "wasm32"))]
pub use native_connection::TransactionBounds as NativeTransactionBounds;
#[cfg(not(target_arch = "wasm32"))]
pub use native_scope::{NativeScope, NativeUncertaintyKey};

#[cfg(all(test, not(target_arch = "wasm32")))]
mod owned_pg_membership_fixture;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod owned_pg_scope_fixture;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod owned_pg_admin_fixture;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod owned_pg_composition_fixture;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod owned_pg_fault_fixture;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod owned_pg_observations_fixture;

#[cfg(not(target_arch = "wasm32"))]
mod owned_pg_commit_proxy_fixture;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod owned_pg_suite_fixture;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod replay_input_hash_contract;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod protected_recovery_overlay_contract;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod story_memory_pg_fixture;

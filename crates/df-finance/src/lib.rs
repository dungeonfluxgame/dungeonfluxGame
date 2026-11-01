//! The money core's native facade (planning/finance-ledger-research.md 3.1): the finance service,
//! its consumer-owned ports with fakes, and the JSON boundary to PostgreSQL.
//!
//! Rust decides (pure plans over snapshots read under locks); PostgreSQL enforces (every write is
//! the `finance.commit_event` definer, which checks the allowlist, the equations and the hash
//! chain). The SQL lives in `df-persistence/migrations/0020-0024`.
//!
//! TODO(FIN-07): the store belongs in `df-persistence::finance_store`; it lives here until that
//! crate's manifest and `lib.rs` are free.

#![cfg(not(target_arch = "wasm32"))]

pub mod admit;
pub mod draft_image;
pub mod exports;
pub mod fakes;
pub mod held;
pub mod host_funding;
pub mod maintenance;
pub mod ports;
pub mod reconcile_bank;
pub mod reconciliation;
pub mod refunds;
pub mod service;
pub mod wire;

mod image_stage;
pub use image_stage::{
    BindingRefusal, ImageAssetId, ImageJobId, ImageStageAdmission, ImageStageBinding,
    ImageStageHook, ImageStageHookOutcome, ImageStageId, ImageStagePhase, ImageStagePort,
    MAX_OWNER_SCOPE_BYTES, OwnerScope, ProviderCancellationOutcome, ProviderSendPort,
    ProviderSettlementOutcome, ProviderSettlementPort, ProviderStageAdmission,
    ProviderStageBinding, ProviderStageCancellationPort, ProviderStagePort,
    ProviderStageSettlement, PublicationAttemptId, StageRefusal, image_stage_quote_digest,
    image_stage_request_digest, provider_stage_request_digest,
};

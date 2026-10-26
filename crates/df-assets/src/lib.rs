//! Verified native byte publication. Metadata visibility belongs to the caller's metadata store.

#[cfg(not(target_arch = "wasm32"))]
mod validation;
#[cfg(not(target_arch = "wasm32"))]
pub use validation::{
    ImageRejection, MAX_IMAGE_PIXELS, MAX_UPLOAD_BYTES, ValidatedImage, validate_image,
};

#[cfg(not(target_arch = "wasm32"))]
mod upload_ticket;
#[cfg(not(target_arch = "wasm32"))]
pub use upload_ticket::{
    MAX_UPLOAD_TICKET_LIFETIME_MS, UploadPurpose, UploadTicket, UploadTicketClaim,
    UploadTicketRejection, validate_upload_ticket, validate_upload_ticket_binding,
};

mod publication;
pub use publication::{
    AssetManifest, AssetMetadataStore, AssetStore, DurableObject, MetadataFailure, Publication,
    PublicationError, PublicationReceipt, PublicationStatus, PublishedBinding, StoreError, publish,
};

#[cfg(not(target_arch = "wasm32"))]
mod native_file_store;
#[cfg(not(target_arch = "wasm32"))]
pub use native_file_store::{NativeFileStore, StagedUpload};

mod range;
pub use range::{
    AccessFailure, AssetReadAuthority, AssetReadStore, AuthorizedBinding, AuthorizedRange,
    ByteRange, ChunkOutcome, MAX_CHUNK_BYTES, RangeError, open_range,
};

#[cfg(not(target_arch = "wasm32"))]
mod resolution;
#[cfg(not(target_arch = "wasm32"))]
pub use resolution::AssetResolver;

#[cfg(not(target_arch = "wasm32"))]
mod outcomes;
#[cfg(not(target_arch = "wasm32"))]
pub use outcomes::{AssetRange, AssetResolutionError, AssetUnavailable, SelectedAsset, StaleAsset};

#[cfg(not(target_arch = "wasm32"))]
mod media_ids;
#[cfg(all(not(target_arch = "wasm32"), any(test, feature = "test-fakes")))]
pub use media_ids::SequentialIds;
#[cfg(not(target_arch = "wasm32"))]
pub use media_ids::{
    AssetId, AttemptId, CharacterTemplateId, ClaimToken, ExportId, IdSource, MediaIdError,
    UploadId, WriteId, mint,
};

#[cfg(not(target_arch = "wasm32"))]
mod native_object_store;
#[cfg(not(target_arch = "wasm32"))]
mod object_store;
#[cfg(not(target_arch = "wasm32"))]
pub use object_store::{
    AssetVariant, CharacterRevision, ConditionalWrites, Deadline, InvalidEnvironment, KeyLayout,
    ListPage, ListedObject, ObjectEnvironment, ObjectKey, ObjectPrefix, ObjectStore,
    ObjectStoreError, ObjectVersion, StorageDomain, StoredObject, VersionListing, VersioningStatus,
};

#[cfg(all(not(target_arch = "wasm32"), any(test, feature = "test-fakes")))]
mod object_store_fake;
#[cfg(all(not(target_arch = "wasm32"), any(test, feature = "test-fakes")))]
pub use object_store_fake::{FakeConfig, FakeObjectStore, Fault};

#[cfg(all(test, not(target_arch = "wasm32")))]
mod object_store_tests;

#[cfg(not(target_arch = "wasm32"))]
mod asset_lifecycle;
#[cfg(not(target_arch = "wasm32"))]
pub use asset_lifecycle::{
    AssetState, FinalizeOutcome, IllegalTransition, RejectReason, SweepRow, SweepWrite, Transition,
    is_staged_deletable, is_write_deletable,
};

#[cfg(not(target_arch = "wasm32"))]
mod audience;
#[cfg(not(target_arch = "wasm32"))]
pub use audience::{Audience, MemberSet};

#[cfg(not(target_arch = "wasm32"))]
mod media_request_key;
#[cfg(not(target_arch = "wasm32"))]
pub use media_request_key::{MEDIA_REQUEST_SCHEMA, MediaRequest, MediaRequestKey};

#[cfg(not(target_arch = "wasm32"))]
mod quota;
#[cfg(not(target_arch = "wasm32"))]
pub use quota::{QuotaPolicy, QuotaRefusal, QuotaScope, lock_order};

#[cfg(not(target_arch = "wasm32"))]
mod catalog_record;
#[cfg(not(target_arch = "wasm32"))]
pub use catalog_record::{
    AssetKind, AssetRecord, DedupeScope, HeldStep, OutboxEntry, Provenance, PublishedFacts,
    QuotaUsage, RetentionClass, StagedObjectRecord, Step, StepLease, TombstoneReason,
    VariantRecord, VariantState,
};

#[cfg(not(target_arch = "wasm32"))]
mod catalog_store;
#[cfg(not(target_arch = "wasm32"))]
pub use catalog_store::{
    Admission, AdmitOutcome, CatalogError, CatalogLookup, CatalogStore, ClaimOutcome, DeletionMark,
    DueRow, FinalizeRequest, FinalizeResult, LeaseOutcome, NotAdmitted, OrphanStaged,
    OrphanVariant, RejectGuard, RejectResult, StageOutcome, StepClaim, SupersedeResult,
    TombstoneResult, VariantIntent, VariantWritten, Winner,
};

#[cfg(all(not(target_arch = "wasm32"), any(test, feature = "test-fakes")))]
mod catalog_fake;
#[cfg(all(not(target_arch = "wasm32"), any(test, feature = "test-fakes")))]
pub use catalog_fake::InMemoryCatalog;

#[cfg(not(target_arch = "wasm32"))]
mod catalog_service;
#[cfg(not(target_arch = "wasm32"))]
pub use catalog_service::{
    Advance, CatalogPorts, CatalogService, GenerationAdmission, GenerationRequest, ServiceFailure,
    StagingOutcome, StagingRefusal,
};

#[cfg(not(target_arch = "wasm32"))]
mod catalog_sweep;
#[cfg(not(target_arch = "wasm32"))]
pub use catalog_sweep::SweepReport;

#[cfg(not(target_arch = "wasm32"))]
mod catalog_config;
#[cfg(not(target_arch = "wasm32"))]
pub use catalog_config::{CatalogConfig, MaximumBytes};

#[cfg(not(target_arch = "wasm32"))]
mod catalog_ports;
#[cfg(not(target_arch = "wasm32"))]
pub use catalog_ports::{
    CancelOutcome, Clock, Dispatcher, FinanceAttempts, FinanceUnavailable, MediaValidator,
    ProviderAttemptState, RefetchAnswer, StagingStream, StreamError, ValidatedMedia,
};

#[cfg(all(not(target_arch = "wasm32"), any(test, feature = "test-fakes")))]
mod catalog_ports_fake;
#[cfg(all(not(target_arch = "wasm32"), any(test, feature = "test-fakes")))]
pub use catalog_ports_fake::{
    ChunkedBytes, FakeDispatcher, FakeFinance, FakeValidator, ManualClock,
};

#[cfg(all(not(target_arch = "wasm32"), any(test, feature = "test-fakes")))]
pub mod catalog_contract;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod catalog_fake_tests;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod catalog_service_tests;

#[cfg(not(target_arch = "wasm32"))]
mod operator_review;
#[cfg(all(not(target_arch = "wasm32"), any(test, feature = "test-fakes")))]
pub use operator_review::LocalReviewBytes;
#[cfg(not(target_arch = "wasm32"))]
pub use operator_review::{
    MAX_PENDING_REVIEWS, MAX_REASON_CHARS, OperatorContext, OperatorReviewStore, PendingReview,
    ReviewByteHandle, ReviewByteStore, ReviewDecision, ReviewError, ReviewReceipt, ReviewRequest,
    approval_permitted, decision_permitted, verify_review_bytes,
};

#[cfg(all(test, not(target_arch = "wasm32")))]
mod operator_review_tests;

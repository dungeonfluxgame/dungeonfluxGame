//! Campaign image-stage admission (IMG-FINANCE-IMAGE-STAGE-A1): the finance half of one
//! immutable image mapping, admitted atomically with the money.
//!
//! What it owns: the typed [`ImageStageBinding`], its domain-separated digest, the preflight that
//! binds it to an [`AdmitCommand`](crate::admit::AdmitCommand), the one-row
//! `provider_attempt.media_asset_id` stamp inside the admission plan, and the per-call
//! [`ImageStagePort`] hook that admits or verifies the catalog linkage in the same transaction.
//! What it never does: no money ledger of its own, no provider or network call, no silent media
//! handling (a legacy [`MediaRequest`](crate::ports::MediaRequest) beside a stage binding is
//! refused, never ignored or merged), and no second finance workflow (the plan still comes from
//! `decide_admission`).
//!
//! Failure keeps evidence, never payloads: database errors keep their SQLSTATE and message, while
//! owner bytes, ids and digests never enter an error.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use df_commerce::digest::business_key;
use df_commerce::finance_ids::{
    AttemptId, CampaignKey, EffectId, OperationKey, SessionKey, TenantKey,
};
use df_commerce::plan::{ChangeKind, FinancePlan, Value as RowValue};
use df_commerce::snapshot::ReceiptRow;
use tokio_postgres::Transaction;

use crate::admit::{AdmitCommand, AdmissionOutcome};
use crate::ports::{FencePort, MediaPort, MemberPort};
use crate::service::{FinanceError, FinanceService};
use crate::wire::WireError;

/// The most owner-scope bytes a binding carries: an opaque caller namespace, bounded so a
/// binding stays small and comparable.
pub const MAX_OWNER_SCOPE_BYTES: usize = 128;

/// The domain tag of the image-stage digest: SHA-256 over the tag and every binding field plus
/// the caller's payload digest, each length-prefixed, so no two different mappings share one.
const DIGEST_DOMAIN: &str = "df-finance/image-stage-v1";

/// An opaque caller namespace for the stage. Empty and oversized scopes are refused at
/// construction, so a binding always carries something comparable and bounded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerScope(Vec<u8>);

impl OwnerScope {
    /// `Err` for an empty or oversized scope: nothing was written.
    pub fn new(bytes: Vec<u8>) -> Result<Self, BindingRefusal> {
        if bytes.is_empty() {
            return Err(BindingRefusal::OwnerScopeEmpty);
        }
        if bytes.len() > MAX_OWNER_SCOPE_BYTES {
            return Err(BindingRefusal::OwnerScopeTooLong);
        }
        Ok(Self(bytes))
    }

    /// The raw scope bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

macro_rules! stage_id {
    ($name:ident, $zero:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 16]);

        impl $name {
            /// `Err` when every byte is zero: an id must name something.
            pub fn new(bytes: [u8; 16]) -> Result<Self, BindingRefusal> {
                if bytes == [0; 16] {
                    return Err(BindingRefusal::$zero);
                }
                Ok(Self(bytes))
            }

            /// The sixteen id bytes.
            pub fn as_bytes(&self) -> &[u8; 16] {
                &self.0
            }
        }
    };
}

/// One imagery job: nonzero and opaque to finance.
stage_id!(ImageJobId, ZeroJob);
/// One logical stage of a job: nonzero and opaque to finance.
stage_id!(ImageStageId, ZeroStage);
/// One immutable asset: its bytes land in `provider_attempt.media_asset_id`.
stage_id!(ImageAssetId, ZeroAsset);
/// One publication attempt of the stage: nonzero and opaque to finance.
stage_id!(PublicationAttemptId, ZeroPublicationAttempt);

/// Why a stage binding failed before any transaction opened. Nothing was written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BindingRefusal {
    OwnerScopeEmpty,
    OwnerScopeTooLong,
    ZeroJob,
    ZeroStage,
    ZeroAsset,
    ZeroPublicationAttempt,
    TenantMismatch,
    CampaignMismatch,
    SessionMismatch,
    OperationMismatch,
    AttemptMismatch,
    EffectMismatch,
    /// A legacy media request beside a stage binding: refused, never merged or ignored, so one
    /// admission cannot book media twice under two mappings.
    LegacyMediaPresent,
}

/// The immutable image mapping one admission pays for. The finance ids repeat the admission's:
/// the preflight refuses a binding that names another operation, attempt, effect, tenant,
/// campaign or session before any transaction opens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageStageBinding {
    pub owner_scope: OwnerScope,
    pub job: ImageJobId,
    pub stage: ImageStageId,
    pub asset: ImageAssetId,
    pub publication_attempt: PublicationAttemptId,
    pub tenant: TenantKey,
    pub campaign: CampaignKey,
    pub session: SessionKey,
    pub operation: OperationKey,
    pub attempt: AttemptId,
    pub effect: EffectId,
}

impl ImageStageBinding {
    /// Checks the binding against the admission it rides on: shared ids must compare equal and
    /// no legacy media request may ride along. Runs before any transaction opens.
    pub fn check_against(&self, admit: &AdmitCommand) -> Result<(), FinanceError> {
        let refused = |refusal| FinanceError::ImageStageBinding(refusal);
        if self.tenant != admit.fence.tenant {
            return Err(refused(BindingRefusal::TenantMismatch));
        }
        if self.campaign != admit.fence.campaign {
            return Err(refused(BindingRefusal::CampaignMismatch));
        }
        if self.session != admit.session {
            return Err(refused(BindingRefusal::SessionMismatch));
        }
        if self.operation != admit.operation {
            return Err(refused(BindingRefusal::OperationMismatch));
        }
        if self.attempt != admit.attempt {
            return Err(refused(BindingRefusal::AttemptMismatch));
        }
        if self.effect != admit.effect {
            return Err(refused(BindingRefusal::EffectMismatch));
        }
        if admit.media.is_some() {
            return Err(refused(BindingRefusal::LegacyMediaPresent));
        }
        Ok(())
    }
}

/// The digest an image admission and its quote share: the full binding plus the caller's payload
/// digest, so a changed mapping changes the fingerprint and the stored business receipt rejects
/// it as a conflict.
pub fn image_stage_request_digest(
    binding: &ImageStageBinding,
    payload_digest: &[u8; 32],
) -> [u8; 32] {
    business_key(
        DIGEST_DOMAIN,
        &[
            binding.owner_scope.as_bytes(),
            binding.job.as_bytes(),
            binding.stage.as_bytes(),
            binding.asset.as_bytes(),
            binding.publication_attempt.as_bytes(),
            binding.tenant.as_bytes(),
            binding.campaign.as_bytes(),
            binding.session.as_bytes(),
            binding.operation.as_bytes(),
            binding.attempt.as_bytes(),
            binding.effect.as_bytes(),
            payload_digest,
        ],
    )
}

/// The digest a quote must store for an image stage: exactly the admission digest, so quote and
/// admission compare equal instead of demanding an impossible quote comparison.
pub fn image_stage_quote_digest(
    binding: &ImageStageBinding,
    payload_digest: &[u8; 32],
) -> [u8; 32] {
    image_stage_request_digest(binding, payload_digest)
}

/// Which finance decision the hook runs under, derived only from `AdmissionDecision`: new work
/// may admit linkage, a replay may only verify it. Never inferred from the port or the database.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageStagePhase {
    New,
    Replay,
}

/// What the hook did: a fresh linkage, or the identical linkage already there. Never a bool.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageStageHookOutcome {
    Accepted,
    Existing,
}

/// Why the catalog refused the stage: nothing was admitted and the finance rows roll back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StageRefusal {
    InFlight,
    Ready,
    AudienceEmpty,
    QuotaFull,
    /// An explicit reason outside the closed four: short and human-readable, never a payload.
    Other {
        reason: String,
    },
}

/// The linkage hook the admission transaction calls with the open transaction, the binding, the
/// phase and the full receipt fields. On [`ImageStagePhase::New`] it atomically admits or
/// verifies the catalog linkage and the exact stage mapping; on
/// [`ImageStagePhase::Replay`] it verifies the identical stored linkage and never admits,
/// repairs, dispatches or sends.
pub trait ImageStageHook: Send + Sync {
    fn on_image_stage<'a>(
        &'a self,
        transaction: &'a Transaction,
        binding: &'a ImageStageBinding,
        phase: ImageStagePhase,
        receipt: &'a ReceiptRow,
    ) -> Pin<Box<dyn Future<Output = Result<ImageStageHookOutcome, FinanceError>> + Send + 'a>>;
}

/// The per-call catalog port one image-stage admission is injected with. The implementor runs
/// its linkage SQL on the open transaction and answers with the typed outcome; in particular
/// `verify_image_link` reads only and never writes. No outgoing provider or network call.
pub trait ImageStagePort: Send + Sync {
    fn admit_image_link<'a>(
        &'a self,
        transaction: &'a Transaction,
        binding: &'a ImageStageBinding,
        receipt: &'a ReceiptRow,
    ) -> Pin<Box<dyn Future<Output = Result<ImageStageHookOutcome, FinanceError>> + Send + 'a>>;

    fn verify_image_link<'a>(
        &'a self,
        transaction: &'a Transaction,
        binding: &'a ImageStageBinding,
        receipt: &'a ReceiptRow,
    ) -> Pin<Box<dyn Future<Output = Result<ImageStageHookOutcome, FinanceError>> + Send + 'a>>;
}

impl<P: ImageStagePort + ?Sized> ImageStageHook for &P {
    fn on_image_stage<'a>(
        &'a self,
        transaction: &'a Transaction,
        binding: &'a ImageStageBinding,
        phase: ImageStagePhase,
        receipt: &'a ReceiptRow,
    ) -> Pin<Box<dyn Future<Output = Result<ImageStageHookOutcome, FinanceError>> + Send + 'a>>
    {
        match phase {
            ImageStagePhase::New => (*self).admit_image_link(transaction, binding, receipt),
            ImageStagePhase::Replay => (*self).verify_image_link(transaction, binding, receipt),
        }
    }
}

/// Stamps the asset id onto the plan's single `provider_attempt` insert, so the asset lands in
/// the same event as the admission. A narrow internal helper: the plan still comes from
/// `decide_admission`, and nothing here updates a finance table or bypasses the definer.
pub(crate) fn stamp_media_asset(
    plan: &mut FinancePlan,
    attempt: &AttemptId,
    asset: &[u8; 16],
) -> Result<(), FinanceError> {
    let shape = || FinanceError::Wire(WireError("plan has no single matching attempt".into()));
    let mut stamped = 0;
    for change in &mut plan.changes {
        if change.kind != ChangeKind::ProviderAttemptInsert {
            continue;
        }
        let expected = match change.row.get("attempt_id") {
            Some(RowValue::Bytes(bytes)) => bytes,
            _ => return Err(shape()),
        };
        if expected.as_slice() != attempt.as_bytes() {
            return Err(shape());
        }
        change
            .row
            .0
            .insert("media_asset_id", RowValue::Bytes(asset.to_vec()));
        stamped += 1;
    }
    if stamped != 1 {
        return Err(shape());
    }
    Ok(())
}

/// A cross-owner call under the existing port deadline: a hook that fails or does not answer in
/// time leaves the admission unwritten, and the caller rolls the transaction back.
pub(crate) async fn bounded_hook(
    deadline: Duration,
    call: Pin<Box<dyn Future<Output = Result<ImageStageHookOutcome, FinanceError>> + Send + '_>>,
) -> Result<ImageStageHookOutcome, FinanceError> {
    match tokio::time::timeout(deadline, call).await {
        Ok(outcome) => outcome,
        Err(_) => Err(FinanceError::PortUnavailable),
    }
}

/// A commit failure after the plan and the linkage step: the outcome is unknown, so the caller
/// replays the same operation with the identical binding to resolve it, and never releases or
/// resends based on this error. Keeps the SQLSTATE evidence, never a payload.
pub(crate) fn commit_unknown(error: tokio_postgres::Error) -> FinanceError {
    match error.as_db_error() {
        Some(db) => FinanceError::ImageStageCommitUnknown {
            sqlstate: Some(db.code().code().to_owned()),
            detail: db.message().to_owned(),
        },
        None => FinanceError::ImageStageCommitUnknown {
            sqlstate: None,
            detail: error.to_string(),
        },
    }
}

/// An image-stage admission: the finance command plus the immutable stage mapping it pays for.
/// `admit.request_digest` is the caller's payload digest; the transaction folds the binding in
/// (see [`image_stage_request_digest`]), so the stored receipt pins the exact mapping.
#[derive(Clone, Debug)]
pub struct ImageStageAdmission {
    pub admit: AdmitCommand,
    pub binding: ImageStageBinding,
}

impl<F: FencePort, M: MediaPort, S: MemberPort> FinanceService<F, M, S> {
    /// Admits one campaign image stage: the preflight binds the mapping to the finance command,
    /// the transaction decides, stamps, fences and hooks exactly like an ordinary admission,
    /// and the hook admits or verifies the catalog linkage in the same transaction.
    ///
    /// A legacy media request beside the binding is refused rather than merged or dropped, so
    /// one admission can never book media twice. A new stage returns the finance receipt; a
    /// replayed operation returns the stored receipt, which never authorizes a send.
    pub async fn admit_image_stage<P: ImageStagePort>(
        &mut self,
        command: &ImageStageAdmission,
        port: &P,
    ) -> Result<AdmissionOutcome, FinanceError> {
        command.binding.check_against(&command.admit)?;
        let hook: &dyn ImageStageHook = port;
        self.admit_with_hook(&command.admit, Some((&command.binding, hook)))
            .await
    }
}

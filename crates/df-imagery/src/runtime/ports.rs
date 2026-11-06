//! Infrastructure edges of the native worker; none permits raw consumer execution.

use crate::execution::{ClaimToken, DispatchId, DispatchMode, OutputCheckpoint, StageView};
use crate::publication::PublishedImage;
use crate::{ImageConfig, ImageJob, ImageOwner, SourceRef};
use std::future::Future;

/// Closed worker failures preserve uncertain dispatch and publication identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    Journal(crate::JournalError),
    Execution(crate::execution::ExecutionError),
    Publication(crate::publication::PublicationError),
    Cost(crate::cost::CostError),
    InvalidConfiguration,
    CredentialUnavailable,
    InvalidRecord,
    SourceDenied,
    UnsupportedCapability,
    Disabled,
    Expired,
    Cancelled,
    ProviderRefused,
    SupplierFailure(crate::execution::FailureCause),
    SupplierQuotaExhausted,
    SupplierRateLimited,
    QuotaBlocked {
        retry_at_ms: Option<i64>,
        notice: Option<crate::execution::QuotaDeferralReason>,
    },
    ProviderUnknown,
    StorageUnavailable,
    CostProjectionPending,
    WorkerFailed,
    TelemetryUnavailable,
}

impl RuntimeError {
    /// Supplier evidence remains independent of whether billing or output is still uncertain.
    pub fn failure_cause(&self) -> Option<crate::execution::FailureCause> {
        use crate::execution::FailureCause;
        match self {
            Self::SupplierFailure(cause) => Some(*cause),
            Self::SupplierQuotaExhausted => Some(FailureCause::QuotaExhausted),
            Self::SupplierRateLimited => Some(FailureCause::RateLimited),
            Self::ProviderRefused => Some(FailureCause::Refused),
            _ => None,
        }
    }
}

/// Exact catalog winner consumed by one execution; versions never imply current authority.
#[derive(Clone, Eq, PartialEq)]
pub struct SourceReceipt {
    pub pin: SourceRef,
    pub attempt: df_assets::AttemptId,
    pub write: df_assets::WriteId,
    pub version: df_assets::ObjectVersion,
    pub suppression_generation: u64,
    pub retention: df_assets::RetentionClass,
}
impl std::fmt::Debug for SourceReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceReceipt")
            .field("asset", &self.pin.asset)
            .field("suppression_generation", &self.suppression_generation)
            .finish_non_exhaustive()
    }
}

/// The authorized bytes and exact winner facts captured under the same source locks.
pub struct AuthorizedImageSource {
    pub bytes: Vec<u8>,
    pub receipt: SourceReceipt,
}

/// Authorized immutable catalog bytes; implementations enforce stored audience and digest pins.
pub trait ImageSources: Send + Sync {
    /// Resolves one exact stored dependency through current owner, audience and source authority.
    fn read_analysis(
        &self,
        _owner: &ImageOwner,
        _handle: &crate::OperationHandle,
    ) -> impl Future<Output = Result<crate::ImageAnalysisObservation, RuntimeError>> + Send {
        async { Err(RuntimeError::UnsupportedCapability) }
    }
    fn read_source(
        &self,
        owner: &ImageOwner,
        source: &SourceRef,
    ) -> impl Future<Output = Result<AuthorizedImageSource, RuntimeError>> + Send;
}

/// The exact claim and dispatch carried through output storage and atomic publication.
#[derive(Clone, Debug)]
pub struct PublicationContext {
    pub job: ImageJob,
    pub stage: StageView,
    pub dispatch: DispatchId,
    pub claim: ClaimToken,
    pub now_ms: i64,
    pub usage_revision: u32,
    pub recovery: Option<crate::execution::RecoveryLease>,
}

/// Exact variants produced by the versioned runtime recipe, without inferred derivatives.
pub struct PreparedImageOutput {
    pub master: Vec<u8>,
    pub display: Option<Vec<u8>>,
}

/// Exact customer disposition read from immutable Ready or no-delivery evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageEconomicDisposition {
    pub usage_revision: u32,
    pub usable: bool,
}

/// Durable output store and catalog publication. Store returns the winning final master object.
pub trait ImagePublisher: Send + Sync {
    /// Rechecks current source authority under publication locks; caller claims no denial verdict.
    fn no_delivery_if_sources_denied(
        &self,
        _terminal: &crate::execution::TerminalCommand,
    ) -> impl Future<Output = Result<crate::publication::SourceClosure, RuntimeError>> + Send {
        async { Err(RuntimeError::UnsupportedCapability) }
    }
    fn read_no_delivery(
        &self,
        _owner: &ImageOwner,
        _job: &crate::ImageJobId,
    ) -> impl Future<Output = Result<Option<crate::publication::NoDeliveryReceipt>, RuntimeError>> + Send
    {
        async { Ok(None) }
    }
    fn no_delivery(
        &self,
        _request: &crate::publication::NoDeliveryRequest<'_>,
    ) -> impl Future<Output = Result<crate::publication::NoDeliveryReceipt, RuntimeError>> + Send
    {
        async { Err(RuntimeError::UnsupportedCapability) }
    }

    fn image_economic_use(
        &self,
        _scope: &crate::execution::ExecutionScope,
    ) -> impl Future<Output = Result<Option<ImageEconomicDisposition>, RuntimeError>> + Send {
        async { Ok(None) }
    }

    fn select_output(
        &self,
        _context: &PublicationContext,
        _selection: &crate::publication::FinalOutputSelection,
    ) -> impl Future<Output = Result<(), RuntimeError>> + Send {
        async { Err(RuntimeError::UnsupportedCapability) }
    }
    fn store_output(
        &self,
        context: &PublicationContext,
        output: &PreparedImageOutput,
    ) -> impl Future<Output = Result<OutputCheckpoint, RuntimeError>> + Send;
    fn publish(
        &self,
        context: &PublicationContext,
    ) -> impl Future<Output = Result<PublishedImage, RuntimeError>> + Send;
}

/// Only a newly committed finance send transition may authorize fresh provider egress.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SendAuthorization {
    New,
    AlreadySent,
}

/// Exact claimed work and the same prepared units route used by finance and cost projection.
#[derive(Clone, Copy)]
pub struct DispatchAdmission<'a> {
    pub job: &'a ImageJob,
    pub stage: &'a StageView,
    pub dispatch: DispatchId,
    pub claim: ClaimToken,
    pub config: &'a ImageConfig,
    pub quote: Option<&'a crate::cost::CostRoute>,
    pub sources: &'a [SourceReceipt],
}

/// A decision settlement carries protected semantic evidence, never a caller usability boolean.
#[derive(Clone, Copy)]
pub struct DispatchDecisionSettlement<'a> {
    pub job: &'a ImageJob,
    pub mode: DispatchMode,
    pub usage: &'a RuntimeUsage,
    pub qualified: &'a super::QualifiedDecision,
}

/// An actual failed standalone supplier stage, with protected non-output evidence and no media
/// allocation.
#[derive(Clone, Copy)]
pub struct DispatchNoOutputSettlement<'a> {
    pub job: &'a ImageJob,
    pub mode: DispatchMode,
    pub usage: &'a RuntimeUsage,
    pub qualified: &'a super::QualifiedNoOutput,
}

/// An authoritative campaign election can finish without creating a supplier dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactAdmission {
    Admitted(DispatchMode),
    ArtifactPending,
    ArtifactResolved,
}

/// Host-owned catalog allocation and finance authority; hosted work never takes a local fallback.
pub trait ImageDispatchAdmission: Send + Sync {
    /// Eligible artwork requires an actual election owner; ordinary requests retain existing
    /// admission.
    fn admit_with_artifact(
        &self,
        input: &DispatchAdmission<'_>,
    ) -> impl Future<Output = Result<ArtifactAdmission, RuntimeError>> + Send {
        async move {
            if crate::service::artifact_recipe::retained_campaign_artifact_recipe(
                &input.job.intent,
                &input.job.config,
            )
            .map_err(RuntimeError::Journal)?
            .is_some()
            {
                return Err(RuntimeError::UnsupportedCapability);
            }
            self.admit(input).await.map(ArtifactAdmission::Admitted)
        }
    }

    fn outcome_no_output(
        &self,
        _input: &DispatchNoOutputSettlement<'_>,
    ) -> impl Future<Output = Result<(), RuntimeError>> + Send {
        async { Err(RuntimeError::UnsupportedCapability) }
    }
    fn release_unused(
        &self,
        _job: &ImageJob,
        _dispatch: DispatchId,
        _receipt: &crate::publication::NoDeliveryReceipt,
    ) -> impl Future<Output = Result<(), RuntimeError>> + Send {
        async { Err(RuntimeError::UnsupportedCapability) }
    }
    fn outcome_decision(
        &self,
        _input: &DispatchDecisionSettlement<'_>,
    ) -> impl Future<Output = Result<(), RuntimeError>> + Send {
        async { Err(RuntimeError::UnsupportedCapability) }
    }
    fn admit(
        &self,
        input: &DispatchAdmission<'_>,
    ) -> impl Future<Output = Result<DispatchMode, RuntimeError>> + Send;
    fn mark_sent(
        &self,
        input: &DispatchAdmission<'_>,
        mode: DispatchMode,
    ) -> impl Future<Output = Result<SendAuthorization, RuntimeError>> + Send;
    fn outcome(
        &self,
        job: &ImageJob,
        dispatch: DispatchId,
        mode: DispatchMode,
        usable: Option<bool>,
    ) -> impl Future<Output = Result<(), RuntimeError>> + Send;
}

/// Fresh entropy from composition; the worker never derives finance or account identities.
pub trait RuntimeIds: Send + Sync {
    fn next_bytes(&self) -> [u8; 16];
}
impl<T: df_assets::IdSource> RuntimeIds for T {
    fn next_bytes(&self) -> [u8; 16] {
        self.next_id_bytes()
    }
}

/// Credentials stay outside persisted recipes and diagnostics.
pub trait ImageCredentials: Send + Sync {
    /// One host-owned account guard survives jobs, pools and configuration rebuilds.
    fn quota_registry(&self) -> Option<std::sync::Arc<super::QuotaRegistry>> {
        None
    }
    /// Optional trusted custody identity groups credentials that spend the same account quota.
    fn quota_identity(
        &self,
        _backend: crate::BackendId,
        _account: &crate::AccountHandle,
    ) -> Option<[u8; 32]> {
        None
    }
    /// Analysis routes have their own configured account; no image-account credential fallback.
    fn analysis_credential(&self, _account: &crate::AccountHandle) -> Result<String, RuntimeError> {
        Err(RuntimeError::UnsupportedCapability)
    }
    fn credential(
        &self,
        backend: crate::BackendId,
        account: &crate::AccountHandle,
    ) -> Result<String, RuntimeError>;
}

/// Structured lifecycle observations forwarded through the host shared telemetry path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimePhase {
    Claimed,
    Admitted,
    Sent,
    SupplierCheckpointed,
    OutputStored,
    Published,
    Unknown,
}

/// Telemetry failure is visible and never turns uncertain work into another dispatch.
pub trait ImageRuntimeTelemetry: Send + Sync {
    /// Best-effort shared telemetry never gates generation or paid-liability durability.
    fn observe(&self, _event: crate::telemetry::ImageEvent) {}
    fn record(
        &self,
        job: &ImageJob,
        stage: &StageView,
        dispatch: Option<DispatchId>,
        phase: RuntimePhase,
        at_ms: i64,
    ) -> Result<(), RuntimeError>;
}

/// Host mapping onto the existing cost book's provider/cause vocabulary and real request maxima.
pub trait ImageRuntimeCostPolicy: Send + Sync {
    /// Analysis children require their own configured priced route, never the image tariff.
    fn stage_route(
        &self,
        job: &ImageJob,
        config: &ImageConfig,
        basis: &RuntimeCostBasis,
    ) -> Result<crate::cost::CostRoute, RuntimeError> {
        if basis.analysis.is_some() {
            return Err(RuntimeError::UnsupportedCapability);
        }
        self.route(
            job,
            config,
            basis.backend.ok_or(RuntimeError::InvalidRecord)?,
            basis.model.as_str(),
        )
    }
    fn route(
        &self,
        job: &ImageJob,
        config: &ImageConfig,
        backend: crate::BackendId,
        model: &str,
    ) -> Result<crate::cost::CostRoute, RuntimeError>;
}

/// One durable raw supplier observation; money and tariffs remain with finance and the cost book.
#[derive(Clone, Eq, PartialEq)]
pub struct RuntimeUsage {
    pub owner: ImageOwner,
    pub job: crate::ImageJobId,
    pub stage: crate::execution::ExecutionStageId,
    pub dispatch: DispatchId,
    pub revision: crate::cost::UsageRevision,
    pub route: crate::cost::CostRoute,
    pub evidence: crate::cost::UsageEvidence,
    pub units: Vec<crate::cost::CostUnits>,
    pub usable: bool,
    pub at_ms: u64,
    pub raw_output: Option<Vec<u8>>,
}

/// Raw observations stay durable even while projection is unavailable or its tariff is missing.
pub trait ImageUsageJournal: Send + Sync {
    fn record_sources(
        &self,
        scope: &crate::execution::ExecutionScope,
        sources: &[SourceReceipt],
    ) -> impl Future<Output = Result<(), RuntimeError>> + Send;
    fn read_sources(
        &self,
        scope: &crate::execution::ExecutionScope,
    ) -> impl Future<Output = Result<Option<Vec<SourceReceipt>>, RuntimeError>> + Send;

    fn record_usage(
        &self,
        usage: &RuntimeUsage,
    ) -> impl Future<Output = Result<(), RuntimeError>> + Send;
    fn latest_usage(
        &self,
        owner: &ImageOwner,
        dispatch: DispatchId,
    ) -> impl Future<Output = Result<Option<RuntimeUsage>, RuntimeError>> + Send;
    /// Reads the semantic receipt's exact retained revision; a newer billing correction is
    /// distinct.
    fn read_usage_revision(
        &self,
        scope: &crate::execution::ExecutionScope,
        revision: crate::cost::UsageRevision,
    ) -> impl Future<Output = Result<Option<RuntimeUsage>, RuntimeError>> + Send {
        async move {
            let latest = self.latest_usage(&scope.owner, scope.dispatch).await?;
            Ok(latest.filter(|usage| {
                usage.job == scope.job && usage.stage == scope.stage && usage.revision == revision
            }))
        }
    }

    fn pending_usage(
        &self,
        scope: &str,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<RuntimeUsage>, RuntimeError>> + Send;
    fn acknowledge_projection(
        &self,
        owner: &ImageOwner,
        dispatch: DispatchId,
        revision: crate::cost::UsageRevision,
    ) -> impl Future<Output = Result<(), RuntimeError>> + Send;
}

impl std::fmt::Debug for RuntimeUsage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeUsage")
            .field("job", &self.job)
            .field("stage", &self.stage)
            .field("dispatch", &self.dispatch)
            .field("revision", &self.revision)
            .field("evidence", &self.evidence)
            .field("usable", &self.usable)
            .field("output_bytes", &self.raw_output.as_ref().map(Vec::len))
            .finish()
    }
}

/// Bounded non-content facts used by the host's existing tariff/maxima policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeCostBasis {
    pub purpose: crate::PurposedUse,
    pub kind: crate::execution::StageKind,
    pub backend: Option<crate::BackendId>,
    pub model: crate::ModelName,
    pub template_version: u32,
    pub prompt_bytes: u64,
    pub reference_count: u64,
    pub generated: Option<crate::Dimensions>,
    pub generation_quality: Option<crate::GenerationQuality>,
    pub analysis: Option<crate::request::AnalysisRoute>,
    pub maximum_output_tokens: Option<u64>,
}

//! Owned consumer demand types for the image service boundary.
//!
//! Views carry only identifiers, revisions, states and the absolute deadline. They never
//! carry snapshots, prompts, account handles, provider names, URLs or fabricated results,
//! so their `Debug` forms are safe for diagnostics.

use crate::journal::{ImageInterestId, ImageJobId, InterestState, JobState};
use crate::policy::Refusal;
use crate::request::{ImageOwner, RequestError};
use df_types::OperationId;

/// The caller-supplied lookup key: owner scope plus stable operation identity.
/// It carries no bearer authority; the service resolves the stored receipt from it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationHandle {
    /// The scope the demand was recorded under.
    pub owner: ImageOwner,
    /// The stable operation within that scope.
    pub operation: OperationId,
}

/// The projected safe view of one demand: identifiers, revisions, states, deadline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageDemandView {
    /// The handle this view was read through.
    pub handle: OperationHandle,
    /// The shared job the receipt points at.
    pub job: ImageJobId,
    /// The caller's own interest in that job.
    pub interest: ImageInterestId,
    /// Current journal state of the job; never a fabricated result state.
    pub job_state: JobState,
    /// Current journal revision of the job; watch compares against this.
    pub job_revision: u64,
    /// Whether this interest is still attached.
    pub interest_state: InterestState,
    /// The retained absolute deadline; replays never recompute it.
    pub absolute_deadline_ms: i64,
    /// Present only after owner-authorized durable catalog publication.
    pub result: Option<crate::publication::PublishedImage>,
    /// Current-authorized catalog artifact resolved through this job's own durable dependency.
    pub artifact: Option<crate::publication::ReadyArtifact>,
    /// Actual typed analytical decision; never a fabricated raster asset.
    pub analysis_result: Option<super::ImageAnalysisResult>,
    /// The actual winning retained inspection on a published image.
    pub inspection: Option<super::PhotoInspectionAssessment>,
    /// Canonical staging identities frozen in the published request.
    pub staging: Vec<crate::StagedIdentity>,
    /// Authorized durable execution state; Unknown never authorizes a replacement request.
    pub execution: ImageExecutionState,
    /// Closed supplier failure on the actual current graph tail, independent of billing uncertainty.
    pub failure: Option<crate::execution::FailureCause>,
    /// Actual unsent queue deferral, separate from supplier failure or financial liability.
    pub operational_notice: Option<crate::execution::QueueDeferralNotice>,
    pub assessment: Option<CreatorAssessment>,
    /// None means unavailable, never zero cost; the existing cost book remains authoritative.
    pub usage: Option<crate::runtime::ImageUsageSummary>,
}

/// What one `request` call decided: recorded demand, an exact replay, or a refusal.
/// Recording means Planning demand only; it never promises a provider launch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RequestOutcome {
    /// Current-authorized immutable shared artwork, with no foreign operation or usage.
    ReusedArtifact(crate::publication::ReadyArtifact),
    /// An authorized immutable Ready result; its original historical handle and interest are retained.
    Reused(ImageDemandView),
    /// A genuinely new Live demand was recorded; the view is its first revision.
    Recorded(ImageDemandView),
    /// The scoped operation already existed with identical canonical intent.
    Replayed(ImageDemandView),
    /// New work was refused without minting ids or touching the journal.
    Refused(Refusal),
}

/// One coherent watch observation: new state, or the still-current revision.
/// This is a single authenticated read, not a push subscription.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WatchObservation {
    /// The job moved past `after_revision`; the view is current as of this read.
    Changed(Box<ImageDemandView>),
    /// Nothing moved; the revision is still current.
    Unchanged { revision: u64 },
}

/// The receipt of one withdrawal: the post-withdrawal view plus the worker count.
/// The count stays crate-visible and out of `Debug` so public formatting
/// never exposes the shared-interest aggregate.
#[derive(Clone, Eq, PartialEq)]
pub struct CancelReceipt {
    /// The demand as reread after the withdrawal.
    pub view: ImageDemandView,
    /// Interests still attached to the job; worker integration reads this.
    pub(crate) attached_remaining: u64,
}

impl std::fmt::Debug for CancelReceipt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CancelReceipt")
            .field("view", &self.view)
            .finish()
    }
}

/// Withdrawing twice is idempotent: the second call changes and revives nothing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CancelOutcome {
    /// This call moved the interest to withdrawn.
    Withdrawn(CancelReceipt),
    /// The interest was already withdrawn; nothing changed.
    AlreadyWithdrawn(CancelReceipt),
}

/// Closed reason a request was refused before any mutation. Variants carry no
/// prompt text, snapshots or counts that could leak private input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestProblem {
    /// The prompt was empty.
    EmptyPrompt,
    /// The prompt exceeds the applicable bound.
    PromptTooLong,
    /// More references than the applicable bound admits.
    TooManyReferences,
    /// Nobody may see this output; it is refused before admission.
    WithheldAudience,
    /// A bound admits no items at all.
    InvalidLimit,
    /// Generate/Edit need a provider profile, Transform a local one.
    ProfileMismatch,
    /// The transform recipe is invalid under the applicable bound.
    InvalidTransform,
    /// The supplied fingerprint is not the canonical intent hash.
    FingerprintMismatch,
    /// Canonical bytes exceed the fixed protocol snapshot bound.
    SnapshotTooLarge,
    /// Canonical bytes are otherwise unrepresentable.
    Malformed,
    /// Versioned analysis metadata is invalid; no private policy text is exposed.
    InvalidPipeline,
    /// Public artwork identity disagrees with the current run, audience or purpose.
    InvalidArtwork,
}

impl RequestProblem {
    /// Projects request validation failures onto the closed service reason set.
    pub(crate) fn from_request(error: RequestError) -> Self {
        match error {
            RequestError::EmptyPrompt => Self::EmptyPrompt,
            RequestError::PromptTooLong { .. } => Self::PromptTooLong,
            RequestError::TooManyReferences { .. } => Self::TooManyReferences,
            RequestError::WithheldAudience => Self::WithheldAudience,
            RequestError::InvalidLimit { .. } => Self::InvalidLimit,
            RequestError::ProfileMismatch => Self::ProfileMismatch,
            RequestError::InvalidTransform(_) => Self::InvalidTransform,
            RequestError::InvalidPipeline(_) => Self::InvalidPipeline,
            RequestError::InvalidArtwork(_) => Self::InvalidArtwork,
        }
    }
}

/// Every way a service call can fail. Denial and absence share one variant so a
/// wrong owner learns nothing; `Unknown` keeps its handle for lookup recovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImageServiceError {
    /// The request or its fingerprint was invalid; nothing was mutated.
    InvalidRequest(RequestProblem),
    /// The principal may not see this, or it does not exist. No details.
    NotAuthorizedOrNotFound,
    /// The scoped operation already exists under different canonical intent.
    Conflict,
    /// A stored row failed to parse or broke a record invariant.
    InvalidRecord,
    /// The call changed nothing; the caller may retry.
    Unavailable,
    /// The commit may have happened; reread the handle before any retry.
    Unknown { handle: OperationHandle },
}

/// Safe execution outcomes without supplier identities, prompts or private aggregate counts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageExecutionState {
    Completed,
    Planning,
    Queued,
    Running,
    Publishing,
    NeedsRevision,
    Uncertain,
    Unknown,
    Failed {
        cause: crate::execution::FailureCause,
    },
    Ready,
    Withdrawn,
    Expired,
}

/// One coherent execution observation at the journal job revision that produced it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageProgress {
    pub revision: u64,
    pub state: ImageExecutionState,
    pub failure: Option<crate::execution::FailureCause>,
    pub operational_notice: Option<crate::execution::QueueDeferralNotice>,
    pub assessment: Option<CreatorAssessment>,
}

/// Closed creator routing evidence; no original appearance, prompt or model response is exposed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreatorAssessment {
    pub category: CreatorCategory,
    pub route: CreatorRoute,
    pub contract_revision: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreatorCategory {
    General,
    AdultNonsexualAnatomyDetail,
    AdultNudity,
    AdultViolence,
    AdultNudityAndViolence,
    Unsupported,
    Uncertain,
}
impl CreatorCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::AdultNonsexualAnatomyDetail => "adult_nonsexual_anatomy_detail",
            Self::AdultNudity => "adult_nudity",
            Self::AdultViolence => "adult_violence",
            Self::AdultNudityAndViolence => "adult_nudity_and_violence",
            Self::Unsupported => "unsupported",
            Self::Uncertain => "uncertain",
        }
    }
    pub fn parse_label(label: &str) -> Option<Self> {
        Some(match label {
            "general" => Self::General,
            "adult_nonsexual_anatomy_detail" => Self::AdultNonsexualAnatomyDetail,
            "adult_nudity" => Self::AdultNudity,
            "adult_violence" => Self::AdultViolence,
            "adult_nudity_and_violence" => Self::AdultNudityAndViolence,
            "unsupported" => Self::Unsupported,
            "uncertain" => Self::Uncertain,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreatorRoute {
    General,
    Bfl,
    Revise,
    Uncertain,
}
impl CreatorRoute {
    pub fn label(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Bfl => "bfl",
            Self::Revise => "revise",
            Self::Uncertain => "uncertain",
        }
    }
    pub fn parse_label(label: &str) -> Option<Self> {
        Some(match label {
            "general" => Self::General,
            "bfl" => Self::Bfl,
            "revise" => Self::Revise,
            "uncertain" => Self::Uncertain,
            _ => return None,
        })
    }
}

/// A revision-coherent authorized usage summary from existing journals and Book receipts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageUsageObservation {
    pub job_revision: u64,
    pub summary: crate::runtime::ImageUsageSummary,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageInspectionObservation {
    pub job_revision: u64,
    pub assessment: super::PhotoInspectionAssessment,
}

/// What an operator revoke changed in one owner scope. Revoke withdraws demand only: work already
/// dispatched to a supplier keeps its uncertainty rules and is never reported as cancelled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevokeReport {
    /// Attached interests withdrawn by this call.
    pub withdrawn: u64,
    /// Interests that were already withdrawn and changed nothing.
    pub already_withdrawn: u64,
    /// Jobs that lost their last attached interest and were retired.
    pub jobs_retired: u64,
}

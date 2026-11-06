//! Narrow injected edges of the image service: clock, ids and authorization.
//!
//! The service performs no provider calls, pixel work, finance or publication. It reads
//! time from [`ServiceClock`], mints journal ids from [`ServiceIds`] and enforces the
//! host-owned [`ImageAuthorizer`]; `df-assets` clock/id machinery is reused through the
//! blanket implementations below where that crate exposes them.

use df_assets::Audience;

use crate::journal::{ImageInterestId, ImageJobId, OperationView};
use crate::output::OutputProfile;
use crate::request::{
    ExecutionMode, Freshness, ImageOperation, ImageOwner, ImageRequest, PurposedUse,
};

use super::types::OperationHandle;

/// Which stored fact the caller wants: read the demand, or withdraw it.
/// Request replay authorizes as `Read` alongside request authorization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceAction {
    /// Observe the demand through a projected safe view.
    Read,
    /// Withdraw this demand's own interest.
    Cancel,
    /// Create new paid demand for an owner. Denied separately from `Read`.
    Generate,
}

/// Opaque authorization denial: denials carry no reason and reveal nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthDenied;

/// Host-owned admission check over a trusted authenticated session principal.
/// The principal is supplied by the host and never deserialized from caller input.
pub trait ImageAuthorizer: Send + Sync {
    /// The host's trusted session type.
    type Principal;
    fn authorize_analysis(
        &self,
        _principal: &Self::Principal,
        _request: &super::ImageAnalysisRequest,
    ) -> Result<(), AuthDenied> {
        Err(AuthDenied)
    }
    fn authorize_stored_analysis(
        &self,
        _principal: &Self::Principal,
        _action: ServiceAction,
        _view: &OperationView,
        _stored: &super::StoredAnalysisAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        Err(AuthDenied)
    }
    /// Permission to submit this owner, purpose, audience and reference pins.
    /// Source pins are claims, not grants: the adapter verifies them against asset
    /// authority where policy depends on them.
    fn authorize_request(
        &self,
        principal: &Self::Principal,
        request: &ImageRequest,
    ) -> Result<(), AuthDenied>;
    /// Coarse scoped lookup gate before any journal access.
    fn authorize_owner(
        &self,
        principal: &Self::Principal,
        owner: &ImageOwner,
        action: ServiceAction,
    ) -> Result<(), AuthDenied>;
    /// Permission to record new demand that this principal could later read back.
    /// The service asks before any journal write, so a host whose stored-read rules
    /// are stricter than its submit rules refuses here instead of leaving a receipt
    /// the requester cannot see. Defaults to the submit check.
    fn authorize_new_visibility(
        &self,
        principal: &Self::Principal,
        request: &ImageRequest,
    ) -> Result<(), AuthDenied> {
        self.authorize_request(principal, request)
    }
    /// Operator permission to revoke every attached interest in one owner scope. Default deny:
    /// no consumer principal holds it unless the host's operator role explicitly grants it.
    fn authorize_revoke(
        &self,
        _principal: &Self::Principal,
        _owner: &ImageOwner,
    ) -> Result<(), AuthDenied> {
        Err(AuthDenied)
    }
    /// Permission to read or withdraw the actual persisted job and interest,
    /// decided from the stored audience and owner rather than the caller's claim.
    fn authorize_stored(
        &self,
        principal: &Self::Principal,
        action: ServiceAction,
        view: &OperationView,
        stored: &StoredImageAuthorization<'_>,
    ) -> Result<(), AuthDenied>;
}

/// Injected wall-clock milliseconds since the Unix epoch.
pub trait ServiceClock: Send + Sync {
    /// The current instant; tests control this exactly.
    fn now_ms(&self) -> i64;
}

/// Borrowed stored authorization facts built only from decoded snapshots.
/// It carries no urgency and no raw bytes; the host authorizes the retained demand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StoredImageAuthorization<'a> {
    /// The handle the stored receipt was read through.
    pub handle: &'a OperationHandle,
    /// The decoded owner the receipt, job and interest must all match.
    pub owner: &'a ImageOwner,
    /// The decoded purpose plus variant role.
    pub purpose: PurposedUse,
    /// The decoded operation with its source pins.
    pub operation: &'a ImageOperation,
    /// The decoded execution mode.
    pub execution: ExecutionMode,
    /// The decoded freshness, including any regenerate nonce.
    pub freshness: Freshness,
    /// The decoded audience the host reauthorizes.
    pub audience: &'a Audience,
    /// The decoded expected source revision.
    pub expected_source_revision: u64,
    /// The decoded output profile.
    pub output: &'a OutputProfile,
}

/// Why a journal id could not be minted; nothing was written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceIdError {
    /// The id source drew zero bytes; the service refuses rather than hanging.
    InvalidGeneratedId,
}

impl std::fmt::Display for ServiceIdError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidGeneratedId => formatter.write_str("the id source drew a zero id"),
        }
    }
}

impl std::error::Error for ServiceIdError {}

/// Injected journal id factory; minted ids are never caller-chosen.
pub trait ServiceIds: Send + Sync {
    /// The next fresh job identity, or a bounded typed failure.
    fn next_job_id(&self) -> Result<ImageJobId, ServiceIdError>;
    /// The next fresh interest identity, or a bounded typed failure.
    fn next_interest_id(&self) -> Result<ImageInterestId, ServiceIdError>;
}

/// Reuses the asset clock where that crate exposes it.
#[cfg(not(target_arch = "wasm32"))]
impl<T> ServiceClock for T
where
    T: df_assets::Clock,
{
    fn now_ms(&self) -> i64 {
        df_assets::Clock::now_ms(self)
    }
}

/// Mints journal ids from the asset id source with one validated draw.
/// A zero draw fails at once so async admission never hangs the owner thread.
#[cfg(not(target_arch = "wasm32"))]
impl<T> ServiceIds for T
where
    T: df_assets::IdSource,
{
    fn next_job_id(&self) -> Result<ImageJobId, ServiceIdError> {
        ImageJobId::from_bytes(&self.next_id_bytes())
            .map_err(|_| ServiceIdError::InvalidGeneratedId)
    }

    fn next_interest_id(&self) -> Result<ImageInterestId, ServiceIdError> {
        ImageInterestId::from_bytes(&self.next_id_bytes())
            .map_err(|_| ServiceIdError::InvalidGeneratedId)
    }
}

/// Durable execution projection. Authorization precedes this read; it cannot widen owner scope.
pub trait ImageObservations: Send + Sync {
    fn read_inspection(
        &self,
        _owner: &ImageOwner,
        _job: crate::ImageJobId,
    ) -> impl std::future::Future<
        Output = Result<Option<super::types::ImageInspectionObservation>, crate::JournalError>,
    > + Send {
        async { Ok(None) }
    }
    fn read_analysis(
        &self,
        _owner: &ImageOwner,
        _job: crate::ImageJobId,
    ) -> impl std::future::Future<
        Output = Result<Option<super::ImageAnalysisObservation>, crate::JournalError>,
    > + Send {
        async { Ok(None) }
    }

    fn read_usage(
        &self,
        _owner: &ImageOwner,
        _job: crate::ImageJobId,
    ) -> impl std::future::Future<
        Output = Result<Option<super::types::ImageUsageObservation>, crate::JournalError>,
    > + Send {
        async { Ok(None) }
    }

    fn read_progress(
        &self,
        owner: &ImageOwner,
        job: crate::ImageJobId,
        now_ms: i64,
    ) -> impl std::future::Future<
        Output = Result<Option<super::types::ImageProgress>, crate::JournalError>,
    > + Send;
}

/// Explicit planning-only construction has no execution observation authority.
pub struct NoImageObservations;
impl ImageObservations for NoImageObservations {
    async fn read_progress(
        &self,
        _owner: &ImageOwner,
        _job: crate::ImageJobId,
        _now_ms: i64,
    ) -> Result<Option<super::types::ImageProgress>, crate::JournalError> {
        Ok(None)
    }
}

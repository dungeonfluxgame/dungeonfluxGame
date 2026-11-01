//! Object-safe erasure of the same consumer boundary, for one injected native service.

use super::*;
use std::future::Future;
use std::pin::Pin;

/// Owned asynchronous service work; no provider execution callback is exposed.
pub type ImageServiceFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ImageServiceError>> + Send + 'a>>;

/// The sole consumer operations, erased for host ownership and faithful boundary tests.
pub trait ImageServiceApi: Send + Sync {
    type Principal: Sync;
    fn canonical_analysis_fingerprint(
        &self,
        _request: &super::ImageAnalysisRequest,
    ) -> Result<[u8; 32], ImageServiceError> {
        Err(ImageServiceError::Unavailable)
    }
    fn request_analysis<'a>(
        &'a self,
        _principal: &'a Self::Principal,
        _request: &'a super::ImageAnalysisRequest,
    ) -> ImageServiceFuture<'a, RequestOutcome> {
        Box::pin(async { Err(ImageServiceError::Unavailable) })
    }
    fn canonical_fingerprint(
        &self,
        request: &crate::ImageRequest,
    ) -> Result<[u8; 32], ImageServiceError>;
    fn request<'a>(
        &'a self,
        principal: &'a Self::Principal,
        request: &'a crate::ImageRequest,
    ) -> ImageServiceFuture<'a, RequestOutcome>;
    fn read<'a>(
        &'a self,
        principal: &'a Self::Principal,
        handle: &'a OperationHandle,
    ) -> ImageServiceFuture<'a, ImageDemandView>;
    fn watch<'a>(
        &'a self,
        principal: &'a Self::Principal,
        handle: &'a OperationHandle,
        after_revision: u64,
    ) -> ImageServiceFuture<'a, WatchObservation>;
    fn cancel<'a>(
        &'a self,
        principal: &'a Self::Principal,
        handle: &'a OperationHandle,
    ) -> ImageServiceFuture<'a, CancelOutcome>;
}
impl<J, C, I, A, R, O> ImageServiceApi for ImageService<J, C, I, A, R, O>
where
    J: ImageJournal,
    C: ServiceClock,
    I: ServiceIds,
    A: ImageAuthorizer,
    R: crate::publication::ImageResults,
    O: ImageObservations,
    A::Principal: Sync,
{
    type Principal = A::Principal;
    fn canonical_analysis_fingerprint(
        &self,
        request: &super::ImageAnalysisRequest,
    ) -> Result<[u8; 32], ImageServiceError> {
        ImageService::canonical_analysis_fingerprint(self, request)
    }
    fn request_analysis<'a>(
        &'a self,
        principal: &'a Self::Principal,
        request: &'a super::ImageAnalysisRequest,
    ) -> ImageServiceFuture<'a, RequestOutcome> {
        Box::pin(ImageService::request_analysis(self, principal, request))
    }
    fn canonical_fingerprint(
        &self,
        request: &crate::ImageRequest,
    ) -> Result<[u8; 32], ImageServiceError> {
        ImageService::canonical_fingerprint(self, request)
    }
    fn request<'a>(
        &'a self,
        principal: &'a Self::Principal,
        request: &'a crate::ImageRequest,
    ) -> ImageServiceFuture<'a, RequestOutcome> {
        Box::pin(ImageService::request(self, principal, request))
    }
    fn read<'a>(
        &'a self,
        principal: &'a Self::Principal,
        handle: &'a OperationHandle,
    ) -> ImageServiceFuture<'a, ImageDemandView> {
        Box::pin(ImageService::read(self, principal, handle))
    }
    fn watch<'a>(
        &'a self,
        principal: &'a Self::Principal,
        handle: &'a OperationHandle,
        after_revision: u64,
    ) -> ImageServiceFuture<'a, WatchObservation> {
        Box::pin(ImageService::watch(self, principal, handle, after_revision))
    }
    fn cancel<'a>(
        &'a self,
        principal: &'a Self::Principal,
        handle: &'a OperationHandle,
    ) -> ImageServiceFuture<'a, CancelOutcome> {
        Box::pin(ImageService::cancel(self, principal, handle))
    }
}

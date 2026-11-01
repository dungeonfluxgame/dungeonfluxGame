//! Async host authority at the object-safe consumer boundary; cached facts never grant access.

use super::super::{image_references::verify_scope, imagery_identity::ImageryPrincipal};
use super::central::ReferenceAuthority;
use df_imagery::{
    CancelOutcome, ImageAnalysisRequest, ImageDemandView, ImageRequest, ImageServiceApi,
    ImageServiceError, ImageServiceFuture, OperationHandle, RequestOutcome, WatchObservation,
};
use std::sync::Arc;

pub(super) struct AuthorizedConsumerService {
    pub(super) inner: Arc<dyn ImageServiceApi<Principal = ImageryPrincipal>>,
    pub(super) references: Arc<dyn ReferenceAuthority>,
}
impl AuthorizedConsumerService {
    async fn current(
        &self,
        expected: &ImageryPrincipal,
        handle: &OperationHandle,
    ) -> Result<ImageryPrincipal, ImageServiceError> {
        let actual = self
            .references
            .authorize_handle(handle)
            .await
            .map_err(|_| ImageServiceError::NotAuthorizedOrNotFound)?;
        if &actual != expected {
            return Err(ImageServiceError::NotAuthorizedOrNotFound);
        }
        Ok(actual)
    }
}
impl ImageServiceApi for AuthorizedConsumerService {
    type Principal = ImageryPrincipal;
    fn canonical_fingerprint(&self, request: &ImageRequest) -> Result<[u8; 32], ImageServiceError> {
        self.inner.canonical_fingerprint(request)
    }
    fn canonical_analysis_fingerprint(
        &self,
        request: &ImageAnalysisRequest,
    ) -> Result<[u8; 32], ImageServiceError> {
        self.inner.canonical_analysis_fingerprint(request)
    }
    fn request<'a>(
        &'a self,
        principal: &'a ImageryPrincipal,
        request: &'a ImageRequest,
    ) -> ImageServiceFuture<'a, RequestOutcome> {
        Box::pin(verify_scope(async move {
            // The existing protocol encoder bounds sources and payload before prefetch.
            self.inner.canonical_fingerprint(request)?;
            let actual = self
                .references
                .authorize(request)
                .await
                .map_err(|_| ImageServiceError::NotAuthorizedOrNotFound)?;
            if &actual != principal {
                return Err(ImageServiceError::NotAuthorizedOrNotFound);
            }
            self.inner.request(&actual, request).await
        }))
    }
    fn request_analysis<'a>(
        &'a self,
        principal: &'a ImageryPrincipal,
        request: &'a ImageAnalysisRequest,
    ) -> ImageServiceFuture<'a, RequestOutcome> {
        Box::pin(verify_scope(async move {
            self.inner.canonical_analysis_fingerprint(request)?;
            if request.identity.owner != *self.references.owner() {
                return Err(ImageServiceError::NotAuthorizedOrNotFound);
            }
            for source in request.operation.sources() {
                self.references
                    .read_reference(source)
                    .await
                    .map_err(|_| ImageServiceError::NotAuthorizedOrNotFound)?;
            }
            for dependency in request.operation.dependencies() {
                self.current(principal, dependency).await?;
            }
            let actual = self
                .references
                .principal()
                .await
                .map_err(|_| ImageServiceError::NotAuthorizedOrNotFound)?;
            if &actual != principal {
                return Err(ImageServiceError::NotAuthorizedOrNotFound);
            }
            self.inner.request_analysis(&actual, request).await
        }))
    }
    fn read<'a>(
        &'a self,
        principal: &'a ImageryPrincipal,
        handle: &'a OperationHandle,
    ) -> ImageServiceFuture<'a, ImageDemandView> {
        Box::pin(verify_scope(async move {
            let actual = self.current(principal, handle).await?;
            self.inner.read(&actual, handle).await
        }))
    }
    fn watch<'a>(
        &'a self,
        principal: &'a ImageryPrincipal,
        handle: &'a OperationHandle,
        after_revision: u64,
    ) -> ImageServiceFuture<'a, WatchObservation> {
        Box::pin(verify_scope(async move {
            let actual = self.current(principal, handle).await?;
            self.inner.watch(&actual, handle, after_revision).await
        }))
    }
    fn cancel<'a>(
        &'a self,
        principal: &'a ImageryPrincipal,
        handle: &'a OperationHandle,
    ) -> ImageServiceFuture<'a, CancelOutcome> {
        Box::pin(verify_scope(async move {
            let actual = self.current(principal, handle).await?;
            self.inner.cancel(&actual, handle).await
        }))
    }
}

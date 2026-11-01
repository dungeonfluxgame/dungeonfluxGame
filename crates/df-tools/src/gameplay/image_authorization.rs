//! Host route authority is captured with the same effective configuration as each new demand.

use super::imagery_identity::{AssetReferenceFacts, ImageryAuthorizer, ImageryPrincipal};
use df_imagery::{
    AuthDenied, ImageAuthorizer, ImageOwner, ImageRequest, OperationView, ServiceAction,
    StoredImageAuthorization,
};

pub(super) struct HostImageAuthorizer<R> {
    scope: ImageryAuthorizer<R>,
    analysis: df_imagery::AnalysisSettings,
    moderation: [Option<df_imagery::ProviderModeration>; df_imagery::Purpose::COUNT],
    facts: R,
}
impl<R: Clone> HostImageAuthorizer<R> {
    pub(super) fn new(facts: R, config: &df_imagery::ImageConfig) -> Self {
        Self {
            scope: ImageryAuthorizer::new(facts.clone()),
            facts,
            analysis: config.analysis.clone(),
            moderation: std::array::from_fn(|index| config.purposes[index].moderation_override),
        }
    }
}
impl<R: AssetReferenceFacts> ImageAuthorizer for HostImageAuthorizer<R> {
    type Principal = ImageryPrincipal;
    fn authorize_analysis(
        &self,
        principal: &ImageryPrincipal,
        request: &df_imagery::ImageAnalysisRequest,
    ) -> Result<(), AuthDenied> {
        let configured = match &request.operation {
            df_imagery::ImageAnalysisOperation::PhotoInspection(_) => {
                self.analysis.inspection.is_some()
            }
            df_imagery::ImageAnalysisOperation::HeadLocations(_) => {
                self.analysis.head_locations.is_some()
            }
            df_imagery::ImageAnalysisOperation::Segmentation(_) => {
                self.analysis.segmentation.is_some()
            }
            // Authorized dependencies may prove a deterministic local decision. Runtime
            // verifies that proof before bypassing supplier admission; missing proof uses gates.
            df_imagery::ImageAnalysisOperation::StagingIdentity(_)
            | df_imagery::ImageAnalysisOperation::StagingFacing(_) => true,
        };
        if !configured {
            return Err(AuthDenied);
        }
        self.authorize_analysis_scope(
            principal,
            &request.identity.owner,
            &request.audience,
            &request.operation,
        )
    }

    fn authorize_stored_analysis(
        &self,
        principal: &ImageryPrincipal,
        action: ServiceAction,
        view: &OperationView,
        stored: &df_imagery::StoredAnalysisAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        if stored.handle.owner != *stored.owner
            || view.job.owner != *stored.owner
            || view.receipt.owner != *stored.owner
            || view.interest.owner != *stored.owner
            || view.receipt.operation != stored.handle.operation
        {
            return Err(AuthDenied);
        }
        self.scope
            .authorize_owner(principal, stored.owner, action)?;
        self.authorize_analysis_scope(principal, stored.owner, stored.audience, stored.operation)
    }

    fn authorize_request(
        &self,
        principal: &ImageryPrincipal,
        request: &ImageRequest,
    ) -> Result<(), AuthDenied> {
        self.scope.authorize_request(principal, request)?;
        if let Some(pipeline) = request.operation.pipeline() {
            if pipeline.moderation.is_some()
                && pipeline.moderation != self.moderation[request.purpose.purpose.index()]
            {
                return Err(AuthDenied);
            }
            if pipeline.creator.as_ref().is_some_and(|creator| {
                self.analysis.classification.as_ref() != Some(&creator.route)
            }) || pipeline.inspection.as_ref().is_some_and(|inspection| {
                self.analysis.inspection.as_ref() != Some(&inspection.route)
            }) {
                return Err(AuthDenied);
            }
        }
        Ok(())
    }
    fn authorize_owner(
        &self,
        principal: &ImageryPrincipal,
        owner: &ImageOwner,
        action: ServiceAction,
    ) -> Result<(), AuthDenied> {
        self.scope.authorize_owner(principal, owner, action)
    }
    fn authorize_stored(
        &self,
        principal: &ImageryPrincipal,
        action: ServiceAction,
        view: &OperationView,
        stored: &StoredImageAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        // Reads retain the old job's route; runtime separately verifies its exact frozen config
        // before any new credential-bearing execution.
        self.scope.authorize_stored(principal, action, view, stored)
    }
}

impl<R: AssetReferenceFacts> HostImageAuthorizer<R> {
    fn authorize_analysis_scope(
        &self,
        principal: &ImageryPrincipal,
        owner: &ImageOwner,
        audience: &df_imagery::Audience,
        operation: &df_imagery::ImageAnalysisOperation,
    ) -> Result<(), AuthDenied> {
        self.scope.authorize_audience(principal, owner, audience)?;
        for source in operation.sources() {
            self.scope
                .authorize_catalog_source(principal, owner, source)?;
            if !self.facts.reference_authorized(source) {
                return Err(AuthDenied);
            }
        }
        for dependency in operation.dependencies() {
            if dependency.owner != *owner {
                return Err(AuthDenied);
            }
        }
        Ok(())
    }
}

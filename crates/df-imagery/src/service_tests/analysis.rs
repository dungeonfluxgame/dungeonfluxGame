//! Same boundary tests for non-raster receipts; these infrastructure fixtures do not execute
//! suppliers.
use super::*;
use crate::{
    AccountHandle, ApiEndpoint, ImageAnalysisObservation, ImageAnalysisOperation,
    ImageAnalysisRequest, ImageAnalysisResult, ImageExecutionState, ModelName,
    SegmentationPreparation, SegmentationPrompt, SegmentationProvider, SegmentationRoute,
    SegmentationSpec, StoredAnalysisAuthorization,
};

#[derive(Clone)]
struct AnalysisAuth(FakeAuth);
impl ImageAuthorizer for AnalysisAuth {
    type Principal = TestPrincipal;
    fn authorize_request(&self, p: &TestPrincipal, r: &ImageRequest) -> Result<(), AuthDenied> {
        self.0.authorize_request(p, r)
    }
    fn authorize_owner(
        &self,
        p: &TestPrincipal,
        o: &ImageOwner,
        a: ServiceAction,
    ) -> Result<(), AuthDenied> {
        self.0.authorize_owner(p, o, a)
    }
    fn authorize_stored(
        &self,
        p: &TestPrincipal,
        a: ServiceAction,
        v: &OperationView,
        s: &StoredImageAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        self.0.authorize_stored(p, a, v, s)
    }
    fn authorize_analysis(
        &self,
        _: &TestPrincipal,
        r: &ImageAnalysisRequest,
    ) -> Result<(), AuthDenied> {
        if r.identity.owner == self.0.request_owner {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }
    fn authorize_stored_analysis(
        &self,
        _: &TestPrincipal,
        _: ServiceAction,
        _: &OperationView,
        s: &StoredAnalysisAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        match self.0.stored_policy {
            StoredPolicy::AllowAll => Ok(()),
            StoredPolicy::DenyAll => Err(AuthDenied),
            StoredPolicy::PublicOnly if *s.audience == Audience::PublicStatic => Ok(()),
            _ => Err(AuthDenied),
        }
    }
}
#[derive(Clone, Default)]
struct AnalysisObservations(Arc<Mutex<Option<ImageAnalysisObservation>>>);
impl crate::ImageObservations for AnalysisObservations {
    async fn read_progress(
        &self,
        _: &ImageOwner,
        _: ImageJobId,
        _: i64,
    ) -> Result<Option<crate::ImageProgress>, JournalError> {
        Ok(None)
    }
    async fn read_analysis(
        &self,
        _: &ImageOwner,
        _: ImageJobId,
    ) -> Result<Option<ImageAnalysisObservation>, JournalError> {
        Ok(self.0.lock().unwrap().clone())
    }
}
fn config() -> ImageConfig {
    let mut config = live_config();
    config.backends[BackendId::Fal.index()].enabled = true;
    config.analysis.segmentation = Some(SegmentationRoute {
        provider: SegmentationProvider::Fal,
        account: AccountHandle::new("analysis-fixture").unwrap(),
        model: ModelName::new("fal-ai/sam-3/image").unwrap(),
        endpoint: ApiEndpoint::new("http://127.0.0.1:1").unwrap(),
    });
    config
}
fn request(operation: OperationId) -> ImageAnalysisRequest {
    ImageAnalysisRequest {
        identity: OperationIdentity {
            owner: draft_owner(),
            operation,
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ReferencePrep,
            role: VariantRole::Reference,
        },
        operation: ImageAnalysisOperation::Segmentation(SegmentationSpec {
            source: source_ref(),
            prompts: vec![SegmentationPrompt::Person],
            preparation: SegmentationPreparation::FigureMask,
            maximum_masks: 8,
            contract_revision: 1,
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Present,
        audience: Audience::PublicStatic,
        expected_source_revision: 7,
    }
}
fn build_service(
    journal: FakeJournal,
    config: ImageConfig,
    auth: FakeAuth,
    observations: AnalysisObservations,
) -> ImageService<
    FakeJournal,
    FakeClock,
    FakeIds,
    AnalysisAuth,
    crate::service::NoImageResults,
    AnalysisObservations,
> {
    ImageService::new(
        journal,
        config,
        FakeClock::new(START_MS),
        FakeIds::new(),
        AnalysisAuth(auth),
    )
    .unwrap()
    .with_observations(observations)
}
#[tokio::test]
async fn analysis_replay_authorization_and_completion_use_the_same_handle() {
    let journal = FakeJournal::new();
    let observations = AnalysisObservations::default();
    let auth = FakeAuth::permitting(draft_owner());
    let service = build_service(
        journal.clone(),
        config(),
        auth.clone(),
        observations.clone(),
    );
    let mut request = request(operation_id(0xa1));
    request.identity.fingerprint = service.canonical_analysis_fingerprint(&request).unwrap();
    let view = recorded_view(
        service
            .request_analysis(&TestPrincipal, &request)
            .await
            .unwrap(),
    );
    assert!(view.result.is_none());
    assert!(view.analysis_result.is_none());
    assert_eq!(
        service.read(&TestPrincipal, &view.handle).await.unwrap(),
        view
    );
    assert!(matches!(
        service
            .watch(&TestPrincipal, &view.handle, 0)
            .await
            .unwrap(),
        WatchObservation::Changed(_)
    ));
    let result = ImageAnalysisResult::Segmentation {
        source: source_ref(),
        contract_revision: 1,
        masks: vec![crate::AnalysisMaskSet {
            prompt: SegmentationPrompt::Person,
            width_px: 1024,
            height_px: 1024,
            instances: Vec::new(),
        }],
    };
    *observations.0.lock().unwrap() = Some(ImageAnalysisObservation {
        selected_at_ms: START_MS,
        job_revision: view.job_revision,
        result: result.clone(),
    });
    let completed = service.read(&TestPrincipal, &view.handle).await.unwrap();
    assert_eq!(completed.execution, ImageExecutionState::Completed);
    assert_eq!(completed.analysis_result, Some(result));
    assert!(completed.result.is_none());
    let mut off = config();
    off.mode = GlobalMode::Off;
    let off = build_service(journal.clone(), off, auth.clone(), observations.clone());
    assert!(matches!(
        off.request_analysis(&TestPrincipal, &request)
            .await
            .unwrap(),
        RequestOutcome::Replayed(_)
    ));
    let mut changed = request.clone();
    if let ImageAnalysisOperation::Segmentation(spec) = &mut changed.operation {
        spec.preparation = SegmentationPreparation::ScenePlate;
    }
    changed.identity.fingerprint = off.canonical_analysis_fingerprint(&changed).unwrap();
    assert_eq!(
        off.request_analysis(&TestPrincipal, &changed)
            .await
            .unwrap_err(),
        ImageServiceError::Conflict
    );
    let mut denied = auth;
    denied.stored_policy = StoredPolicy::DenyAll;
    let denied = build_service(journal, config(), denied, observations);
    assert_eq!(
        denied
            .request_analysis(&TestPrincipal, &changed)
            .await
            .unwrap_err(),
        ImageServiceError::NotAuthorizedOrNotFound
    );
    assert_eq!(
        denied.read(&TestPrincipal, &view.handle).await.unwrap_err(),
        ImageServiceError::NotAuthorizedOrNotFound
    );
}
#[tokio::test]
async fn analysis_result_cannot_claim_a_raster_or_a_stale_revision() {
    let observations = AnalysisObservations::default();
    let service = build_service(
        FakeJournal::new(),
        config(),
        FakeAuth::permitting(draft_owner()),
        observations.clone(),
    );
    let mut request = request(operation_id(0xa2));
    request.identity.fingerprint = service.canonical_analysis_fingerprint(&request).unwrap();
    let view = recorded_view(
        service
            .request_analysis(&TestPrincipal, &request)
            .await
            .unwrap(),
    );
    *observations.0.lock().unwrap() = Some(ImageAnalysisObservation {
        selected_at_ms: START_MS,
        job_revision: view.job_revision + 1,
        result: ImageAnalysisResult::Segmentation {
            source: source_ref(),
            contract_revision: 1,
            masks: vec![crate::AnalysisMaskSet {
                prompt: SegmentationPrompt::Person,
                width_px: 1024,
                height_px: 1024,
                instances: Vec::new(),
            }],
        },
    });
    assert!(matches!(
        service
            .read(&TestPrincipal, &view.handle)
            .await
            .unwrap_err(),
        ImageServiceError::Unknown { .. }
    ));
    let mut wrong = source_ref();
    wrong.digest = AssetDigest::from_bytes([0xab; 32]);
    *observations.0.lock().unwrap() = Some(ImageAnalysisObservation {
        selected_at_ms: START_MS,
        job_revision: view.job_revision,
        result: ImageAnalysisResult::Segmentation {
            source: wrong,
            contract_revision: 1,
            masks: vec![crate::AnalysisMaskSet {
                prompt: SegmentationPrompt::Person,
                width_px: 1024,
                height_px: 1024,
                instances: Vec::new(),
            }],
        },
    });
    assert_eq!(
        service
            .read(&TestPrincipal, &view.handle)
            .await
            .unwrap_err(),
        ImageServiceError::InvalidRecord
    );
}

#[path = "analysis_visibility.rs"]
mod visibility;

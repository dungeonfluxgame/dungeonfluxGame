//! Canonical snapshots produced by the consumer service's actual encoder.
use df_imagery::*;
use df_types::OperationId;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Option<AcceptCommand>>>);
impl ImageJournal for Capture {
    async fn accept(&self, command: &AcceptCommand) -> Result<AcceptOutcome, JournalError> {
        *self.0.lock().unwrap() = Some(command.clone());
        Err(JournalError::Unavailable)
    }
    async fn lookup_operation(
        &self,
        _: &ImageOwner,
        _: OperationId,
    ) -> Result<Option<OperationView>, JournalError> {
        Ok(None)
    }
    async fn read_job(
        &self,
        _: &ImageOwner,
        _: ImageJobId,
    ) -> Result<Option<ImageJob>, JournalError> {
        panic!("snapshot capture read")
    }
    async fn freeze_recipe(&self, _: &FreezeCommand) -> Result<FreezeOutcome, JournalError> {
        panic!("snapshot capture freeze")
    }
    async fn withdraw_interest(
        &self,
        _: &ImageOwner,
        _: ImageInterestId,
        _: i64,
    ) -> Result<WithdrawOutcome, JournalError> {
        panic!("snapshot capture withdraw")
    }
}
struct FixtureClock;
impl ServiceClock for FixtureClock {
    fn now_ms(&self) -> i64 {
        super::NOW
    }
}
struct FixtureIds;
impl ServiceIds for FixtureIds {
    fn next_job_id(&self) -> Result<ImageJobId, ServiceIdError> {
        Ok(ImageJobId::from_bytes(&[1; 16]).unwrap())
    }
    fn next_interest_id(&self) -> Result<ImageInterestId, ServiceIdError> {
        Ok(ImageInterestId::from_bytes(&[15; 16]).unwrap())
    }
}
struct FixtureAuthority(ImageOwner);
impl ImageAuthorizer for FixtureAuthority {
    type Principal = ();
    fn authorize_request(&self, _: &(), request: &ImageRequest) -> Result<(), AuthDenied> {
        (request.identity.owner == self.0)
            .then_some(())
            .ok_or(AuthDenied)
    }
    fn authorize_owner(
        &self,
        _: &(),
        owner: &ImageOwner,
        _: ServiceAction,
    ) -> Result<(), AuthDenied> {
        (*owner == self.0).then_some(()).ok_or(AuthDenied)
    }
    fn authorize_stored(
        &self,
        _: &(),
        _: ServiceAction,
        _: &OperationView,
        stored: &StoredImageAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        (*stored.owner == self.0).then_some(()).ok_or(AuthDenied)
    }
}

pub(super) async fn snapshots(
    owner: &ImageOwner,
    source: Option<SourceRef>,
) -> (VersionedSnapshot, VersionedSnapshot) {
    let audience = match owner {
        ImageOwner::Run { campaign, .. } => Audience::CampaignShared(*campaign),
        ImageOwner::Draft { account } => Audience::OwnerPrivate(*account),
    };
    let source = source.map_or_else(
        || {
            TransformSource::Canvas(
                CanvasSpec::new(
                    1,
                    1,
                    Rgb {
                        red: 10,
                        green: 20,
                        blue: 30,
                    },
                )
                .unwrap(),
            )
        },
        TransformSource::Image,
    );
    let request = ImageRequest {
        identity: OperationIdentity {
            owner: owner.clone(),
            operation: OperationId::from_bytes(&[16; 16]).unwrap(),
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ImportedDerivative,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Transform(TransformSpec {
            source,
            steps: vec![],
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Visible,
        audience,
        expected_source_revision: 1,
        output: OutputProfile::local(
            RasterOutputSpec::new(
                Dimensions::new(1, 1).unwrap(),
                AlphaHandling::Opaque,
                Encoding::Png,
            )
            .unwrap(),
        ),
    };
    encode(owner, request, ImageConfig::defaults().unwrap()).await
}

async fn encode(
    owner: &ImageOwner,
    mut request: ImageRequest,
    config: ImageConfig,
) -> (VersionedSnapshot, VersionedSnapshot) {
    let capture = Capture::default();
    let service = ImageService::new(
        capture.clone(),
        config,
        FixtureClock,
        FixtureIds,
        FixtureAuthority(owner.clone()),
    )
    .unwrap();
    request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
    assert!(service.request(&(), &request).await.is_err());
    let command = capture
        .0
        .lock()
        .unwrap()
        .take()
        .expect("canonical service accept captured");
    (command.intent, command.config)
}

pub(super) fn inspection_pipeline(requirement: InspectionRequirement) -> ImagePipeline {
    let route = AnalysisRoute {
        account: AccountHandle::new("fixture").unwrap(),
        model: ModelName::new("fixture-model").unwrap(),
        endpoint: ApiEndpoint::new("https://fixture.invalid/v1/chat/completions").unwrap(),
        maximum_output_tokens: 128,
    };
    ImagePipeline {
        revision: 1,
        moderation: None,
        rating: ImageContentRating::Restrained,
        creator: None,
        inspection: Some(ImageInspection {
            route,
            requirement,
            contract_revision: 1,
            kind: InspectionKind::CanonicalRoom,
            expected_heroes: None,
            party_reference: None,
            hand_custody: PrivateImageText::new("{}"),
            staging: vec![],
            maximum_corrections: 0,
            allow_visible_text: false,
        }),
    }
}

pub(super) async fn pipeline_snapshots(
    owner: &ImageOwner,
    pipeline: ImagePipeline,
) -> (VersionedSnapshot, VersionedSnapshot) {
    let audience = match owner {
        ImageOwner::Run { campaign, .. } => Audience::CampaignShared(*campaign),
        ImageOwner::Draft { account } => Audience::OwnerPrivate(*account),
    };
    let mut config = ImageConfig::defaults().unwrap();
    config.analysis.classification = pipeline.creator.as_ref().map(|spec| spec.route.clone());
    config.analysis.inspection = pipeline.inspection.as_ref().map(|spec| spec.route.clone());
    let request = ImageRequest {
        identity: OperationIdentity {
            owner: owner.clone(),
            operation: OperationId::from_bytes(&[16; 16]).unwrap(),
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::CanonicalRoom,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("An empty stone room.", 1)
                .unwrap()
                .with_pipeline(pipeline)
                .unwrap(),
            references: vec![],
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Visible,
        audience,
        expected_source_revision: 1,
        output: OutputProfile::provider(
            Dimensions::new(1, 1).unwrap(),
            RasterOutputSpec::new(
                Dimensions::new(1, 1).unwrap(),
                AlphaHandling::Opaque,
                Encoding::Png,
            )
            .unwrap(),
        ),
    };
    encode(owner, request, config).await
}

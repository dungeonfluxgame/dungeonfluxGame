//! Actual service admission verifies standalone identity without creating a media surrogate.
use super::*;
struct AnalysisAuthority;
impl ImageAuthorizer for AnalysisAuthority {
    type Principal = ();
    fn authorize_request(&self, _: &(), _: &ImageRequest) -> Result<(), AuthDenied> {
        Err(AuthDenied)
    }
    fn authorize_analysis(&self, _: &(), request: &ImageAnalysisRequest) -> Result<(), AuthDenied> {
        (request.identity.owner == owner())
            .then_some(())
            .ok_or(AuthDenied)
    }
    fn authorize_owner(
        &self,
        _: &(),
        value: &ImageOwner,
        _: ServiceAction,
    ) -> Result<(), AuthDenied> {
        (*value == owner()).then_some(()).ok_or(AuthDenied)
    }
    fn authorize_stored(
        &self,
        _: &(),
        _: ServiceAction,
        _: &OperationView,
        _: &StoredImageAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        Err(AuthDenied)
    }
    fn authorize_stored_analysis(
        &self,
        _: &(),
        _: ServiceAction,
        _: &OperationView,
        stored: &StoredAnalysisAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        (*stored.owner == owner()).then_some(()).ok_or(AuthDenied)
    }
}
#[tokio::test]
async fn canonical_analysis_admission_has_no_publication_and_refuses_mixed_media_enqueue() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let transport = PostgresTransport::PlaintextLoopbackOnly;
    let ids = TestIds(Arc::new(AtomicU64::new(20_000)));
    let scope = CapacityScope::new("runtime/standalone-analysis").unwrap();
    let execution = PostgresImageExecution::open(scratch.config.clone(), transport.clone(), 2)
        .await
        .unwrap();
    execution
        .set_capacity_bounds(
            scope.as_str(),
            &CapacityBounds::new(2, 4, 4_194_304).unwrap(),
            NOW,
        )
        .await
        .unwrap();
    execution
        .set_execution_gate(scope.as_str(), 1, true, NOW)
        .await
        .unwrap();
    let journal = PostgresRuntimeJournal::open_existing(
        scratch.config.clone(),
        transport,
        2,
        Arc::new(ids.clone()),
        scope.clone(),
        1,
        true,
    )
    .unwrap();
    let mut config = ImageConfig::defaults().unwrap();
    config.analysis.head_locations = Some(AnalysisRoute {
        account: AccountHandle::new("analysis-fixture").unwrap(),
        model: ModelName::new("analysis-fixture-model").unwrap(),
        endpoint: ApiEndpoint::new("http://127.0.0.1:1/never-called").unwrap(),
        maximum_output_tokens: 100,
    });
    let service =
        ImageService::new(journal.clone(), config, TestClock, ids, AnalysisAuthority).unwrap();
    // A source pin is a caller claim; this queue test never reads it or dispatches a supplier call.
    let mut request = ImageAnalysisRequest {
        identity: OperationIdentity {
            owner: owner(),
            operation: OperationId::from_bytes(&[40; 16]).unwrap(),
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ImportedDerivative,
            role: VariantRole::Primary,
        },
        operation: ImageAnalysisOperation::HeadLocations(HeadLocationsSpec {
            source: SourceRef {
                asset: df_assets::AssetId::from_bytes(&[41; 16]).unwrap(),
                digest: AssetDigest::from_bytes([42; 32]),
                audience: Audience::OwnerPrivate(AccountId::from_bytes(&[11; 16]).unwrap()),
            },
            face: None,
            targets: vec![AnalysisTarget {
                id: PrivateImageText::new("fixture-person"),
                description: PrivateImageText::new("adult fictional traveler"),
            }],
            contract_revision: 1,
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Visible,
        audience: Audience::OwnerPrivate(AccountId::from_bytes(&[11; 16]).unwrap()),
        expected_source_revision: 1,
    };
    request.identity.fingerprint = service.canonical_analysis_fingerprint(&request).unwrap();
    let accepted = match service.request_analysis(&(), &request).await.unwrap() {
        RequestOutcome::Recorded(v) => v,
        other => panic!("{other:?}"),
    };
    let replay = service.request_analysis(&(), &request).await.unwrap();
    assert!(matches!(replay, RequestOutcome::Replayed(_)));
    let queued = execution.scan_queue(scope.as_str(), 4, NOW).await.unwrap();
    assert_eq!(queued.len(), 1);
    let stage = execution
        .read_stage(&owner(), &accepted.job, &queued[0].stage)
        .await
        .unwrap();
    assert_eq!(stage.kind, StageKind::HeadLocations);
    assert!(stage.planned_publication.is_none());
    assert!(
        execution
            .read_analysis_checkpoint(&owner(), &accepted.job)
            .await
            .unwrap()
            .is_none()
    );
    let counts=client.query_one("SELECT (SELECT count(*) FROM media.asset),(SELECT count(*) FROM imagery.image_publication_allocation), \
        (SELECT count(*) FROM imagery.image_dispatch_attempt),(SELECT count(*) FROM finance.provider_attempt)",&[]).await.unwrap();
    for index in 0..4 {
        assert_eq!(counts.get::<_, i64>(index), 0);
    }
    let basis = execution
        .read_stage_basis(&owner(), &accepted.job, &stage.stage)
        .await
        .unwrap();
    let image = EnqueueCommand::new(
        StageScope {
            owner: owner(),
            job: accepted.job,
            stage: ExecutionStageId::from_bytes(&[43; 16]).unwrap(),
        },
        StageBasis {
            kind: StageKind::Generation,
            ..basis
        },
        ExecutionIdentity::new([44; 16], [45; 16]).unwrap(),
        scope,
        true,
        NOW,
    )
    .unwrap();
    assert_eq!(
        execution.enqueue(&image).await,
        Err(ExecutionError::PlannedIdentityConflict)
    );
    let tx = client.transaction().await.unwrap();
    let partial = tx
        .execute(
            "UPDATE imagery.image_job SET planned_publication_asset=$2 WHERE job_id=$1",
            &[&accepted.job.as_bytes().as_slice(), &[46_u8; 16].as_slice()],
        )
        .await
        .unwrap_err();
    assert_eq!(partial.code().map(|c| c.code()), Some("23514"));
    drop(tx);
    let row=client.query_one("SELECT planned_publication_asset IS NULL AND planned_publication_attempt IS NULL FROM imagery.image_job WHERE job_id=$1",
        &[&accepted.job.as_bytes().as_slice()]).await.unwrap();
    assert!(row.get::<_, bool>(0));
    drop(service);
    drop(journal);
    drop(execution);
    drop(client);
    scratch.finish().await;
}

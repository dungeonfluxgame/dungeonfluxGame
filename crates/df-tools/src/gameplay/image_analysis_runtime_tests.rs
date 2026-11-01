//! Actual mounted hosts prove standalone analysis durability, no-call staging and current grants.
use super::{
    cost::port::{MeterObserver, MeterStart, MeteredCall, MeteringPort},
    image_test_support::ImageHostFixture,
    imagery::{analysis, central::ConsumerContext},
    resources::SpendLedger,
};
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{Request, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use df_imagery::*;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::{
    net::SocketAddr,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Notify, oneshot};

#[derive(Clone, Copy)]
enum HeadAnswer {
    Located,
    MalformedReported,
    MalformedUnknown,
}
#[derive(Clone)]
struct Captured {
    address: SocketAddr,
    answer: HeadAnswer,
    heads: Arc<AtomicU64>,
    segmentation: Arc<AtomicU64>,
}
pub(super) struct Supplier {
    captured: Captured,
    stop: Option<oneshot::Sender<()>>,
    worker: Option<tokio::task::JoinHandle<()>>,
}
impl Supplier {
    pub(super) async fn open() -> Self {
        Self::open_with_answer(HeadAnswer::Located).await
    }
    async fn open_with_answer(answer: HeadAnswer) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let captured = Captured {
            address: listener.local_addr().unwrap(),
            answer,
            heads: Arc::new(AtomicU64::new(0)),
            segmentation: Arc::new(AtomicU64::new(0)),
        };
        let app = Router::new()
            .fallback(exchange)
            .with_state(captured.clone());
        let (stop, stopped) = oneshot::channel();
        let worker = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _closed = stopped.await;
                })
                .await
                .unwrap();
        });
        Self {
            captured,
            stop: Some(stop),
            worker: Some(worker),
        }
    }
    pub(super) fn address(&self) -> SocketAddr {
        self.captured.address
    }
    pub(super) async fn close(mut self) {
        if let Some(stop) = self.stop.take() {
            let _closed = stop.send(());
        }
        tokio::time::timeout(Duration::from_secs(5), self.worker.take().unwrap())
            .await
            .unwrap()
            .unwrap();
    }
}
impl Drop for Supplier {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.abort();
        }
    }
}
async fn exchange(State(captured): State<Captured>, request: Request) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let body = to_bytes(request.into_body(), 2_097_152).await.unwrap();
    if method == Method::POST && path == "/heads" {
        captured.heads.fetch_add(1, Ordering::SeqCst);
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["model"], "gpt-5.5");
        let content = match captured.answer {
            HeadAnswer::Located => {
                json!({"heads":[{"id":"hero-1","left":0.1,"top":0.1,"right":0.2,"bottom":0.3}]})
            }
            HeadAnswer::MalformedReported | HeadAnswer::MalformedUnknown => {
                json!({"heads":"invalid-schema"})
            }
        }
        .to_string();
        let mut response = json!({"choices":[{"message":{"content":content}}]});
        if !matches!(captured.answer, HeadAnswer::MalformedUnknown) {
            response["usage"] = json!({"prompt_tokens":7,"completion_tokens":3});
        }
        return Json(response).into_response();
    }
    if method == Method::POST && path == "/fixture-sam" {
        captured.segmentation.fetch_add(1, Ordering::SeqCst);
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert!(matches!(body["prompt"].as_str(), Some("person" | "head")));
        let origin = format!("http://{}", captured.address);
        return Json(json!({"request_id":"sam-1","status_url":format!("{origin}/fixture-sam/requests/sam-1/status"),"response_url":format!("{origin}/fixture-sam/requests/sam-1/response")})).into_response();
    }
    if method == Method::GET && path.ends_with("/status") {
        return Json(json!({"status":"COMPLETED"})).into_response();
    }
    if method == Method::GET && path.ends_with("/response") {
        return Json(json!({"rle":[],"scores":[],"metadata":[]})).into_response();
    }
    StatusCode::NOT_FOUND.into_response()
}
pub(super) fn patch(supplier: &Supplier) -> RuntimePatch {
    let mut patch = RuntimePatch::empty();
    patch.mode = Some(GlobalMode::Live);
    patch.backend_enabled = [Some(false); BackendId::COUNT];
    patch.purpose_enabled[Purpose::ReferencePrep.index()] = Some(true);
    patch.scheduler_stage_deadline_secs = Some(5);
    patch.scheduler_job_deadline_secs = Some(30);
    patch.analysis_head_locations = Some(
        AnalysisRoute {
            account: AccountHandle::new("fixture-heads").unwrap(),
            model: ModelName::new("gpt-5.5").unwrap(),
            endpoint: ApiEndpoint::new(&format!("http://{}/heads", supplier.captured.address))
                .unwrap(),
            maximum_output_tokens: 500,
        }
        .into(),
    );
    patch.analysis_segmentation = Some(
        SegmentationRoute {
            provider: SegmentationProvider::Fal,
            account: AccountHandle::new("fixture-sam").unwrap(),
            model: ModelName::new("fixture-sam").unwrap(),
            endpoint: ApiEndpoint::new(&format!("http://{}", supplier.captured.address)).unwrap(),
        }
        .into(),
    );
    patch.analysis_staging_identity = Some(AnalysisRoutePatch::Clear);
    patch.analysis_staging_facing = Some(AnalysisRoutePatch::Clear);
    patch
}
fn source_png() -> Bytes {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            &[40, 60, 80, 255].repeat(16 * 8),
            16,
            8,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    bytes.into()
}
/// A real file-backed Book stays alive through host restarts and drains before file removal.
pub(super) struct DurableCosts {
    pub(super) port: MeteringPort,
    ledger: Arc<SpendLedger>,
    path: PathBuf,
}
impl DurableCosts {
    pub(super) fn open() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "dungeonflux-standalone-{}-{}-{}.jsonl",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        let ledger = Arc::new(SpendLedger::default());
        let prices =
            super::cost::PriceTable::parse(include_str!("../../../../content/pricing.json"))
                .unwrap();
        let pricing = super::cost::PricingStatus::Loaded {
            path: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/pricing.json"),
            entries: prices.len(),
        };
        let opened = ledger
            .costs()
            .open(
                "standalone-analysis-host",
                Some(&path),
                prices,
                pricing,
                super::cost::now_ms(),
            )
            .unwrap();
        assert_eq!(opened.log_error, None, "actual durable cost log must open");
        Self {
            port: MeteringPort::from_ledger(ledger.clone()),
            ledger,
            path,
        }
    }
}
impl Drop for DurableCosts {
    fn drop(&mut self) {
        self.ledger.costs().close();
        std::fs::remove_file(&self.path).unwrap();
    }
}
struct CostObserved(Notify);
impl MeterObserver for CostObserved {
    fn started(&self, _: &MeterStart) {}
    fn settled(&self, call: &MeteredCall) {
        if call.detail == "head_locator"
            && call.source == super::cost::port::MeteredSource::Reported
        {
            self.0.notify_one();
        }
    }
}
async fn connect(
    config: &tokio_postgres::Config,
) -> (
    tokio_postgres::Client,
    tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
) {
    let (client, connection) = config.connect(tokio_postgres::NoTls).await.unwrap();
    (client, tokio::spawn(connection))
}
pub(super) async fn complete(
    context: &ConsumerContext,
    handle: &OperationHandle,
) -> ImageDemandView {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match context.service.read(&context.principal, handle).await {
                Ok(view) if view.analysis_result.is_some() => {
                    assert_eq!(view.execution, ImageExecutionState::Completed);
                    assert!(view.result.is_none());
                    return view;
                }
                Ok(_) | Err(ImageServiceError::Unknown { .. }) => tokio::task::yield_now().await,
                Err(error) => panic!("authorized analysis read failed: {error:?}"),
            }
        }
    })
    .await
    .unwrap()
}
pub(super) fn analysis_request(
    context: &ConsumerContext,
    source: SourceRef,
    operation: u8,
) -> ImageAnalysisRequest {
    let mut request = ImageAnalysisRequest {
        identity: OperationIdentity {
            owner: context.owner.clone(),
            operation: df_types::OperationId::from_bytes(&[operation; 16]).unwrap(),
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ReferencePrep,
            role: VariantRole::Reference,
        },
        operation: ImageAnalysisOperation::HeadLocations(HeadLocationsSpec {
            source,
            face: None,
            targets: vec![AnalysisTarget {
                id: PrivateImageText::new("hero-1"),
                description: PrivateImageText::new("A fictional adult dwarf wearing a red cloak"),
            }],
            contract_revision: 1,
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Visible,
        audience: match context.owner {
            ImageOwner::Run { campaign, .. } => Audience::CampaignShared(campaign),
            ImageOwner::Draft { account } => Audience::OwnerPrivate(account),
        },
        expected_source_revision: context.expected_source_revision,
    };
    request.identity.fingerprint = context
        .service
        .canonical_analysis_fingerprint(&request)
        .unwrap();
    request
}

/// Every command uses the actual selected checkpoint and retained send history.
async fn assert_analysis_sealed(
    client: &mut tokio_postgres::Client,
    owner: &ImageOwner,
    job: ImageJobId,
) {
    use df_imagery::execution::*;
    use df_persistence::imagery_execution as execution;
    let tx = client.transaction().await.unwrap();
    let checkpoint = execution::read_analysis_checkpoint_in(&tx, owner, &job)
        .await
        .unwrap()
        .unwrap();
    let AnalysisStageReference::Supplier(reference) = &checkpoint.stages[0] else {
        panic!("paid actual supplier stage required")
    };
    let stage = reference.scope.stage;
    let dispatch = reference.scope.dispatch;
    let basis = execution::read_stage_basis_in(&tx, owner, &job, &stage)
        .await
        .unwrap();
    let stored = execution::read_dispatch_in(&tx, owner, &dispatch)
        .await
        .unwrap();
    let row = tx.query_one("SELECT s.capacity_scope,s.local_accounting,s.claim_token,s.claim_generation,s.claim_config_revision,s.claim_lease_ms,d.claim_token AS dispatch_claim FROM imagery.image_execution_stage s JOIN imagery.image_dispatch_attempt d ON d.job_id=s.job_id AND d.stage_id=s.stage_id WHERE d.dispatch_id=$1", &[&dispatch.as_bytes().as_slice()]).await.unwrap();
    let scope = StageScope {
        owner: owner.clone(),
        job,
        stage,
    };
    let now_ms = i64::try_from(super::cost::now_ms()).unwrap();
    let capacity = CapacityScope::new(row.get::<_, String>("capacity_scope")).unwrap();
    let enqueue = EnqueueCommand::new(
        scope.clone(),
        basis,
        None,
        capacity,
        row.get("local_accounting"),
        now_ms,
    )
    .unwrap();
    assert!(matches!(
        execution::enqueue_stage_in(&tx, &enqueue).await.unwrap(),
        EnqueueOutcome::Existing(_)
    ));
    let mut new_stage = enqueue.clone();
    new_stage.stage = ExecutionStageId::from_bytes(&[0x75; 16]).unwrap();
    assert_eq!(
        execution::enqueue_stage_in(&tx, &new_stage)
            .await
            .unwrap_err(),
        ExecutionError::OutputSealed
    );
    let claim = ClaimCommand::new(
        owner.clone(),
        job,
        stage,
        u64::try_from(row.get::<_, i64>("claim_config_revision")).unwrap(),
        ClaimToken::from_bytes(&row.get::<_, Vec<u8>>("claim_token")).unwrap(),
        row.get("claim_lease_ms"),
        now_ms,
    )
    .unwrap();
    execution::claim_stage_in(&tx, &claim).await.unwrap();
    let mut new_claim = claim;
    new_claim.claim = ClaimToken::from_bytes(&[0x76; 16]).unwrap();
    assert_eq!(
        execution::claim_stage_in(&tx, &new_claim)
            .await
            .unwrap_err(),
        ExecutionError::OutputSealed
    );
    let stage_claim = StageClaim {
        scope,
        token: ClaimToken::from_bytes(&row.get::<_, Vec<u8>>("dispatch_claim")).unwrap(),
        generation: stored.claim_generation,
    };
    let mode = stored
        .finance
        .map_or(DispatchMode::Local, DispatchMode::Hosted);
    let prepare = PrepareCommand::new(stage_claim.clone(), dispatch, mode, now_ms).unwrap();
    assert_eq!(
        execution::prepare_dispatch_in(&tx, &prepare).await.unwrap(),
        stored
    );
    let mut new_prepare = prepare;
    new_prepare.dispatch = DispatchId::from_bytes(&[0x77; 16]).unwrap();
    assert_eq!(
        execution::prepare_dispatch_in(&tx, &new_prepare)
            .await
            .unwrap_err(),
        ExecutionError::OutputSealed
    );
    let sent = MarkSentCommand::new(
        owner.clone(),
        job,
        stage,
        dispatch,
        stage_claim.token,
        stage_claim.generation,
        now_ms,
    )
    .unwrap();
    assert_eq!(execution::mark_sent_in(&tx, &sent).await.unwrap(), stored);
    assert_eq!(
        execution::read_analysis_checkpoint_in(&tx, owner, &job)
            .await
            .unwrap()
            .unwrap(),
        checkpoint
    );
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn actual_paid_head_decision_recovers_after_projection_ack_failure_without_a_second_supplier_send()
 {
    let supplier = Supplier::open().await;
    let costs = DurableCosts::open();
    let observed = Arc::new(CostObserved(Notify::new()));
    costs.port.observe(observed.clone());
    let Some(mut host) = ImageHostFixture::open(
        supplier.captured.address,
        patch(&supplier),
        costs.port.clone(),
    )
    .await
    else {
        supplier.close().await;
        return;
    };
    let (_, reference) = host
        .import_bytes(df_providers::ImageFormat::Png, source_png())
        .await
        .unwrap();
    let source = reference.canonical_source().unwrap().clone();
    let (mut client, driver) = connect(host.database()).await;
    client.batch_execute("CREATE SEQUENCE imagery.fixture_projection_fault; CREATE FUNCTION imagery.fixture_refuse_projection() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.projection_durable THEN PERFORM nextval('imagery.fixture_projection_fault'); RAISE EXCEPTION 'controlled projection acknowledgement failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER fixture_refuse_projection BEFORE UPDATE OF projection_durable ON imagery.image_dispatch_usage FOR EACH ROW EXECUTE FUNCTION imagery.fixture_refuse_projection()").await.unwrap();
    let context = host.imagery().consumer_context().await.unwrap();
    let request = analysis_request(&context, source, 0x71);
    let accepted = match context
        .service
        .request_analysis(&context.principal, &request)
        .await
        .unwrap()
    {
        RequestOutcome::Recorded(view) => view,
        other => panic!("new actual analysis demand expected: {other:?}"),
    };
    drop(context);
    tokio::time::timeout(Duration::from_secs(10), observed.0.notified())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if client
                .query_one(
                    "SELECT is_called FROM imagery.fixture_projection_fault",
                    &[],
                )
                .await
                .unwrap()
                .get::<_, bool>(0)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the actual injected projection ACK boundary must be reached");
    host.stop_after_worker_failure().await;
    let row=client.query_one("SELECT d.dispatch_state,d.settlement FROM imagery.image_dispatch_attempt d WHERE d.job_id=$1",&[&accepted.job.as_bytes().as_slice()]).await.unwrap();
    assert_eq!(row.get::<_, String>(0), "succeeded");
    assert_eq!(row.get::<_, String>(1), "settled");
    assert_eq!(
        client
            .query_one(
                "SELECT count(*) FROM imagery.image_analysis_checkpoint WHERE job_id=$1",
                &[&accepted.job.as_bytes().as_slice()]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    client.batch_execute("DROP TRIGGER fixture_refuse_projection ON imagery.image_dispatch_usage;DROP FUNCTION imagery.fixture_refuse_projection();DROP SEQUENCE imagery.fixture_projection_fault").await.unwrap();
    client
        .execute(
            "UPDATE imagery.image_execution_stage SET lease_until_ms=1 WHERE job_id=$1",
            &[&accepted.job.as_bytes().as_slice()],
        )
        .await
        .unwrap();
    host.restart_stopped().await;
    let context = host.imagery().consumer_context().await.unwrap();
    let view = complete(&context, &accepted.handle).await;
    assert!(
        matches!(view.analysis_result,Some(ImageAnalysisResult::HeadLocations{heads,..}) if heads.len()==1)
    );
    assert_eq!(supplier.captured.heads.load(Ordering::SeqCst), 1);
    assert_eq!(costs.port.totals().calls, 1);
    assert_analysis_sealed(&mut client, &context.owner, accepted.job).await;
    assert_eq!(
        client
            .query_one(
                "SELECT count(*) FROM imagery.image_job_result WHERE job_id=$1",
                &[&accepted.job.as_bytes().as_slice()]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    assert_eq!(client.query_one("SELECT count(*) FROM imagery.image_dispatch_usage WHERE job_id=$1 AND NOT projection_durable",&[&accepted.job.as_bytes().as_slice()]).await.unwrap().get::<_,i64>(0),0);
    drop(context);
    drop(client);
    driver.abort();
    drop(reference);
    host.close().await;
    supplier.close().await;
}

#[tokio::test]
async fn actual_empty_sam_dependencies_complete_local_identity_and_facing_without_provider_routes_or_money_rows()
 {
    let supplier = Supplier::open().await;
    let costs = DurableCosts::open();
    let Some(mut host) = ImageHostFixture::open(
        supplier.captured.address,
        patch(&supplier),
        costs.port.clone(),
    )
    .await
    else {
        supplier.close().await;
        return;
    };
    let (key, reference) = host
        .import_bytes(df_providers::ImageFormat::Png, source_png())
        .await
        .unwrap();
    let segmentation = analysis::segments(
        host.imagery(),
        &key,
        SegmentationPreparation::ScenePlate,
        vec![SegmentationPrompt::Person, SegmentationPrompt::Head],
        8,
    )
    .await
    .unwrap();
    assert_eq!(segmentation.value.len(), 2);
    assert!(
        segmentation
            .value
            .iter()
            .all(|set| set.instances.is_empty())
    );
    assert_eq!(supplier.captured.segmentation.load(Ordering::SeqCst), 2);
    let (client, driver) = connect(host.database()).await;
    let before_usage = client
        .query_one("SELECT count(*) FROM imagery.image_dispatch_usage", &[])
        .await
        .unwrap()
        .get::<_, i64>(0);
    let before_dispatch = client
        .query_one("SELECT count(*) FROM imagery.image_dispatch_attempt", &[])
        .await
        .unwrap()
        .get::<_, i64>(0);
    let before_assets = client
        .query_one("SELECT count(*) FROM media.asset", &[])
        .await
        .unwrap()
        .get::<_, i64>(0);
    {
        let mut control = host.control().lock().unwrap();
        let revision = control.read_effective().provenance.revision;
        let mut patch = RuntimePatch::empty();
        patch.mode = Some(GlobalMode::LocalOnly);
        control.apply(patch, revision).unwrap();
    }
    host.restart().await;
    let identity = analysis::identity(host.imagery(), &segmentation, None, &[])
        .await
        .unwrap();
    assert!(identity.value.is_empty());
    let facing = analysis::facing(host.imagery(), &segmentation, &identity)
        .await
        .unwrap();
    assert!(facing.value.is_empty());
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM imagery.image_local_analysis", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        2
    );
    assert_eq!(
        client
            .query_one(
                "SELECT count(*) FROM imagery.image_analysis_checkpoint",
                &[]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        3
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM imagery.image_dispatch_usage", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        before_usage
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM imagery.image_dispatch_attempt", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        before_dispatch
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM media.asset", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        before_assets
    );
    host.restart().await;
    let context = host.imagery().consumer_context().await.unwrap();
    let view = complete(&context, &facing.handle).await;
    assert!(
        matches!(view.analysis_result,Some(ImageAnalysisResult::StagingFacing{facings,..}) if facings.is_empty())
    );
    client
        .execute(
            "UPDATE df_local_demo.grants SET active=false WHERE credential=$1",
            &[&host.player_credential().as_slice()],
        )
        .await
        .unwrap();
    assert_eq!(
        context
            .service
            .read(&context.principal, &facing.handle)
            .await
            .unwrap_err(),
        ImageServiceError::NotAuthorizedOrNotFound
    );
    assert_eq!(supplier.captured.segmentation.load(Ordering::SeqCst), 2);
    assert_eq!(supplier.captured.heads.load(Ordering::SeqCst), 0);
    drop(context);
    drop(client);
    driver.abort();
    drop(reference);
    host.close().await;
    supplier.close().await;
}

#[tokio::test]
async fn actual_malformed_head_with_reported_usage_settles_unusable_without_an_analysis_result() {
    assert_malformed_head(
        HeadAnswer::MalformedReported,
        "failed",
        "unusable_settled",
        "reported",
    )
    .await;
}

#[tokio::test]
async fn actual_malformed_head_with_unknown_usage_retains_outstanding_liability() {
    assert_malformed_head(
        HeadAnswer::MalformedUnknown,
        "unknown",
        "unknown_outstanding",
        "unknown",
    )
    .await;
}

async fn assert_malformed_head(
    answer: HeadAnswer,
    dispatch_state: &str,
    settlement: &str,
    evidence: &str,
) {
    let supplier = Supplier::open_with_answer(answer).await;
    let costs = DurableCosts::open();
    let Some(mut host) = ImageHostFixture::open(
        supplier.captured.address,
        patch(&supplier),
        costs.port.clone(),
    )
    .await
    else {
        supplier.close().await;
        return;
    };
    let (_, reference) = host
        .import_bytes(df_providers::ImageFormat::Png, source_png())
        .await
        .unwrap();
    let context = host.imagery().consumer_context().await.unwrap();
    let request = analysis_request(
        &context,
        reference.canonical_source().unwrap().clone(),
        0x78,
    );
    let accepted = match context
        .service
        .request_analysis(&context.principal, &request)
        .await
        .unwrap()
    {
        RequestOutcome::Recorded(view) => view,
        other => panic!("actual new demand expected: {other:?}"),
    };
    let (client, driver) = connect(host.database()).await;
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(row) = client.query_opt("SELECT d.dispatch_state,d.settlement,u.evidence,u.usable,u.raw_output IS NOT NULL AS retained,u.projection_durable FROM imagery.image_dispatch_attempt d JOIN imagery.image_dispatch_usage u ON u.dispatch_id=d.dispatch_id WHERE d.job_id=$1 ORDER BY u.revision DESC LIMIT 1", &[&accepted.job.as_bytes().as_slice()]).await.unwrap()
                && row.get::<_, String>("dispatch_state") == dispatch_state
                && row.get::<_, String>("settlement") == settlement
                && row.get::<_, bool>("projection_durable")
            {
                assert_eq!(row.get::<_, String>("evidence"), evidence);
                assert!(!row.get::<_, bool>("usable"));
                assert!(row.get::<_, bool>("retained"));
                break;
            }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    let view = context
        .service
        .read(&context.principal, &accepted.handle)
        .await
        .unwrap();
    assert!(view.analysis_result.is_none());
    assert!(view.result.is_none());
    assert_eq!(
        view.execution,
        if evidence == "reported" {
            ImageExecutionState::Failed {
                cause: df_imagery::execution::FailureCause::InvalidOutput,
            }
        } else {
            ImageExecutionState::Unknown
        }
    );
    let row = client.query_one("SELECT (SELECT count(*) FROM imagery.image_analysis_checkpoint WHERE job_id=$1),(SELECT count(*) FROM imagery.image_job_result WHERE job_id=$1),(SELECT count(*) FROM imagery.image_no_delivery WHERE job_id=$1)", &[&accepted.job.as_bytes().as_slice()]).await.unwrap();
    for column in 0..3 {
        assert_eq!(row.get::<_, i64>(column), 0);
    }
    assert_eq!(supplier.captured.heads.load(Ordering::SeqCst), 1);
    assert_eq!(costs.port.totals().calls, 1);
    drop(context);
    host.restart().await;
    let context = host.imagery().consumer_context().await.unwrap();
    let replay = context
        .service
        .request_analysis(&context.principal, &request)
        .await
        .unwrap();
    assert!(matches!(replay, RequestOutcome::Replayed(_)));
    assert_eq!(supplier.captured.heads.load(Ordering::SeqCst), 1);
    assert_eq!(costs.port.totals().calls, 1);
    drop(context);
    drop(client);
    driver.abort();
    drop(reference);
    host.close().await;
    supplier.close().await;
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum LocalClaimPhase {
    Idle,
    Armed,
    Paused,
    Released,
}
struct LocalClaimBarrier {
    phase: std::sync::Mutex<LocalClaimPhase>,
    released: std::sync::Condvar,
    arrived: Notify,
}
impl LocalClaimBarrier {
    fn arm(self: &Arc<Self>) -> LocalClaimRelease {
        *self.phase.lock().unwrap() = LocalClaimPhase::Armed;
        LocalClaimRelease(self.clone())
    }
}
struct LocalClaimRelease(Arc<LocalClaimBarrier>);
impl Drop for LocalClaimRelease {
    fn drop(&mut self) {
        *self
            .0
            .phase
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = LocalClaimPhase::Released;
        self.0.released.notify_all();
    }
}
impl df_observe::DiagnosticSink for LocalClaimBarrier {
    fn emit(&self, record: df_observe::DiagnosticRecord) -> Result<(), df_observe::TelemetryError> {
        if record.event_name != "image.claimed"
            || !record.input.attributes.iter().any(|(key, value)| {
                key == "image.stage_kind"
                    && value == &df_observe::AnyValue::String("staging_identity".into())
            })
        {
            return Ok(());
        }
        let mut phase = self.phase.lock().unwrap();
        if *phase != LocalClaimPhase::Armed {
            return Ok(());
        }
        *phase = LocalClaimPhase::Paused;
        self.arrived.notify_one();
        let (_phase, timeout) = self
            .released
            .wait_timeout_while(phase, Duration::from_secs(10), |phase| {
                *phase == LocalClaimPhase::Paused
            })
            .unwrap();
        assert!(
            !timeout.timed_out(),
            "test must release the actual post-claim barrier"
        );
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_local_analysis_cancel_after_claim_releases_capacity_and_next_job_progresses_without_restart()
 {
    let supplier = Supplier::open().await;
    let costs = DurableCosts::open();
    let barrier = Arc::new(LocalClaimBarrier {
        phase: std::sync::Mutex::new(LocalClaimPhase::Idle),
        released: std::sync::Condvar::new(),
        arrived: Notify::new(),
    });
    let observer = Arc::new(df_imagery::telemetry::SharedImageObserver::new(
        barrier.clone(),
    ));
    let mut configuration = patch(&supplier);
    configuration.scheduler_stage_deadline_secs = Some(15);
    let Some(host) = ImageHostFixture::open_with_observer(
        supplier.address(),
        configuration,
        costs.port.clone(),
        observer,
    )
    .await
    else {
        supplier.close().await;
        return;
    };
    let (key, reference) = host
        .import_bytes(df_providers::ImageFormat::Png, source_png())
        .await
        .unwrap();
    let segmentation = analysis::segments(
        host.imagery(),
        &key,
        SegmentationPreparation::ScenePlate,
        vec![SegmentationPrompt::Person, SegmentationPrompt::Head],
        8,
    )
    .await
    .unwrap();
    let context = host.imagery().consumer_context().await.unwrap();
    let source = reference.canonical_source().unwrap().clone();
    let mut request = analysis_request(&context, source.clone(), 0x7a);
    request.operation = ImageAnalysisOperation::StagingIdentity(StagingIdentitySpec {
        source: source.clone(),
        segmentation: segmentation.handle.clone(),
        locations: None,
        references: vec![],
        contract_revision: 1,
    });
    request.identity.fingerprint = context
        .service
        .canonical_analysis_fingerprint(&request)
        .unwrap();
    let release = barrier.arm();
    let first = match context
        .service
        .request_analysis(&context.principal, &request)
        .await
        .unwrap()
    {
        RequestOutcome::Recorded(view) => view,
        other => panic!("new actual local demand expected: {other:?}"),
    };
    tokio::time::timeout(Duration::from_secs(10), barrier.arrived.notified())
        .await
        .unwrap();
    let second_request = analysis_request(&context, source, 0x7b);
    let second = match context
        .service
        .request_analysis(&context.principal, &second_request)
        .await
        .unwrap()
    {
        RequestOutcome::Recorded(view) => view,
        other => panic!("second actual demand expected: {other:?}"),
    };
    let cancelled = context
        .service
        .cancel(&context.principal, &first.handle)
        .await
        .unwrap();
    assert!(matches!(cancelled, CancelOutcome::Withdrawn(_)));
    drop(release);
    let completed = complete(&context, &second.handle).await;
    assert!(matches!(
        completed.analysis_result,
        Some(ImageAnalysisResult::HeadLocations { .. })
    ));
    let (client, driver) = connect(host.database()).await;
    let row=client.query_one("SELECT (SELECT count(*) FROM imagery.image_dispatch_attempt WHERE job_id=$1),(SELECT count(*) FROM imagery.image_local_analysis WHERE job_id=$1),(SELECT count(*) FROM imagery.image_analysis_checkpoint WHERE job_id=$1),(SELECT count(*) FROM imagery.image_execution_stage WHERE job_id=$1 AND capacity_reserved)",&[&first.job.as_bytes().as_slice()]).await.unwrap();
    for column in 0..4 {
        assert_eq!(row.get::<_, i64>(column), 0);
    }
    let cancelled = context
        .service
        .read(&context.principal, &first.handle)
        .await
        .unwrap();
    assert!(cancelled.analysis_result.is_none());
    assert!(cancelled.result.is_none());
    assert_eq!(supplier.captured.segmentation.load(Ordering::SeqCst), 2);
    assert_eq!(supplier.captured.heads.load(Ordering::SeqCst), 1);
    drop(context);
    drop(client);
    driver.abort();
    drop(reference);
    host.close().await;
    supplier.close().await;
}

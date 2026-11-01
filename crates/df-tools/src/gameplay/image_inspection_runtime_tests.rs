//! Actual host photographic corrections retain private raw lineage and publish only a qualified winner.
use super::{
    image_analysis_runtime_tests::DurableCosts, image_test_support::ImageHostFixture,
    imagery::central::ConsumerContext,
};
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{Request, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use df_imagery::*;
use image::ImageEncoder;
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::oneshot;

#[derive(Default)]
struct Captured {
    generations: AtomicU64,
    inspections: AtomicU64,
    image_bytes: Mutex<Vec<Vec<u8>>>,
    generation_requests: Mutex<Vec<Vec<u8>>>,
    inspection_bodies: Mutex<Vec<Vec<u8>>>,
    fail_second: bool,
}
struct Supplier {
    address: SocketAddr,
    captured: Arc<Captured>,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}
impl Supplier {
    async fn open(fail_second: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let captured = Arc::new(Captured {
            fail_second,
            ..Default::default()
        });
        let router = Router::new()
            .fallback(supplier)
            .with_state(captured.clone());
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        Self {
            address,
            captured,
            stop,
            task,
        }
    }
    async fn close(self) {
        let _ = self.stop.send(());
        self.task.await.unwrap();
    }
}
fn picture(red: u8) -> Vec<u8> {
    let pixels = image::RgbaImage::from_pixel(1024, 1024, image::Rgba([red, 60, 80, 255]));
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(pixels.as_raw(), 1024, 1024, image::ExtendedColorType::Rgba8)
        .unwrap();
    bytes
}
async fn supplier(State(captured): State<Arc<Captured>>, request: Request) -> Response {
    assert_eq!(request.method(), Method::POST);
    let path = request.uri().path().to_owned();
    let body = to_bytes(request.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap();
    if matches!(path.as_str(), "/v1/images/edits" | "/v1/images/generations") {
        let ordinal = captured.generations.fetch_add(1, Ordering::SeqCst);
        assert!(
            ordinal < 2,
            "single correction policy dispatched more than two images"
        );
        let bytes = picture(40 + ordinal as u8);
        let encoded = STANDARD.encode(&bytes);
        captured.image_bytes.lock().unwrap().push(bytes);
        captured
            .generation_requests
            .lock()
            .unwrap()
            .push(body.to_vec());
        return Json(json!({"data":[{"b64_json":encoded}],
            "usage":{"input_tokens_details":{"text_tokens":7,"image_tokens":3},"output_tokens":720}})).into_response();
    }
    assert_eq!(path, "/inspection");
    let ordinal = captured.inspections.fetch_add(1, Ordering::SeqCst);
    assert!(
        ordinal < 2,
        "inspection was resent or repeated beyond correction policy"
    );
    let request: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(request["model"], "gpt-5.5");
    assert_eq!(request["max_completion_tokens"], 3000);
    let content = &request["messages"][0]["content"];
    assert_eq!(
        content.as_array().unwrap().len(),
        3,
        "actual candidate plus authorized party reference"
    );
    assert!(
        content[0]["text"]
            .as_str()
            .unwrap()
            .contains("exactly 4 player heroes")
    );
    let rejected = ordinal == 0 || captured.fail_second;
    let assessment = json!({"frame":"single_photograph","medium":"photograph","text":"none",
        "dress":"period","setting":"period","hero_count":if rejected {5}else{4},
        "weapon_custody":"matches_custody","visible_features":[{
            "name":if rejected {"private rejected cart"}else{"accepted fruit stand"},
            "look":"Apples under canvas.","region":[600,200,900,700],"recorded_id":null}]});
    let response = json!({"choices":[{"message":{"content":assessment.to_string()}}],
        "usage":{"prompt_tokens":120,"completion_tokens":20}})
    .to_string()
    .into_bytes();
    captured
        .inspection_bodies
        .lock()
        .unwrap()
        .push(response.clone());
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        response,
    )
        .into_response()
}
fn patch(address: SocketAddr) -> RuntimePatch {
    let mut patch = RuntimePatch::empty();
    patch.mode = Some(GlobalMode::Live);
    patch.backend_enabled = [Some(false); BackendId::COUNT];
    patch.backend_enabled[BackendId::OpenAi.index()] = Some(true);
    patch.quality_provider_quality = Some(Some(GenerationQuality::Medium));
    patch.retry_max_attempts = Some(1);
    patch.scheduler_stage_deadline_secs = Some(5);
    patch.scheduler_job_deadline_secs = Some(30);
    patch.analysis_inspection = Some(
        AnalysisRoute {
            account: AccountHandle::new("fixture-inspection").unwrap(),
            model: ModelName::new("gpt-5.5").unwrap(),
            endpoint: ApiEndpoint::new(&format!("http://{address}/inspection")).unwrap(),
            maximum_output_tokens: 3000,
        }
        .into(),
    );
    patch
}
async fn connection(
    config: &tokio_postgres::Config,
) -> (
    tokio_postgres::Client,
    tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
) {
    let (client, connection) = config.connect(tokio_postgres::NoTls).await.unwrap();
    (client, tokio::spawn(connection))
}
async fn party_source(host: &ImageHostFixture) -> SourceRef {
    let dimensions = Dimensions::new(16, 8).unwrap();
    let reference = host
        .imagery()
        .transform(
            "inspection-party-source",
            PurposedUse {
                purpose: Purpose::ReferencePrep,
                role: VariantRole::Reference,
            },
            TransformSpec {
                source: TransformSource::Canvas(
                    CanvasSpec::new(
                        16,
                        8,
                        Rgb {
                            red: 40,
                            green: 60,
                            blue: 80,
                        },
                    )
                    .unwrap(),
                ),
                steps: vec![],
            },
            RasterOutputSpec::new(dimensions, AlphaHandling::Opaque, Encoding::Png).unwrap(),
        )
        .await
        .unwrap();
    reference
        .canonical_source()
        .expect("actual locally published catalog source")
        .clone()
}
fn demand(context: &ConsumerContext, source: SourceRef) -> df_imagery::ImageRequest {
    let route = context.config.analysis.inspection.clone().unwrap();
    let mut request = df_imagery::ImageRequest {
        identity: OperationIdentity {
            owner: context.owner.clone(),
            operation: df_types::OperationId::from_bytes(&[0x79; 16]).unwrap(),
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ScenePlate,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("private original photographic scene prompt", 1)
                .unwrap()
                .with_pipeline(ImagePipeline {
                    revision: 1,
                    moderation: None,
                    rating: ImageContentRating::Restrained,
                    creator: None,
                    inspection: Some(ImageInspection {
                        route,
                        requirement: InspectionRequirement::Required,
                        contract_revision: 1,
                        kind: InspectionKind::ScenePlate,
                        expected_heroes: Some(4),
                        party_reference: Some(1),
                        hand_custody: PrivateImageText::new(
                            &json!({"hands":"All hands empty.","place":"place-market",
                        "source_hash":"f".repeat(64),"public_records":[]})
                            .to_string(),
                        ),
                        staging: vec![],
                        maximum_corrections: 1,
                        allow_visible_text: false,
                    }),
                })
                .unwrap(),
            references: vec![source],
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Visible,
        audience: match context.owner {
            ImageOwner::Run { campaign, .. } => Audience::CampaignShared(campaign),
            ImageOwner::Draft { account } => Audience::OwnerPrivate(account),
        },
        expected_source_revision: context.expected_source_revision,
        output: OutputProfile::provider(
            Dimensions::new(1024, 1024).unwrap(),
            RasterOutputSpec::new(
                Dimensions::new(1024, 1024).unwrap(),
                AlphaHandling::Opaque,
                Encoding::Png,
            )
            .unwrap(),
        ),
    };
    request.identity.fingerprint = context.service.canonical_fingerprint(&request).unwrap();
    request
}
async fn terminal(context: &ConsumerContext, handle: &OperationHandle) -> ImageDemandView {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match context.service.read(&context.principal, handle).await {
                Ok(view)
                    if matches!(
                        view.execution,
                        ImageExecutionState::Ready | ImageExecutionState::Failed { .. }
                    ) =>
                {
                    return view;
                }
                Ok(_) | Err(ImageServiceError::Unknown { .. }) => {
                    tokio::time::sleep(Duration::from_millis(4)).await
                }
                Err(error) => panic!("authorized inspection observation: {error:?}"),
            }
        }
    })
    .await
    .expect("bounded actual inspection pipeline completion")
}
async fn verify_lineage(
    client: &tokio_postgres::Client,
    job: &ImageJobId,
    captured: &Captured,
    second_passes: bool,
) {
    let raw=client.query("SELECT s.stage_kind,u.raw_output FROM imagery.image_dispatch_usage u JOIN imagery.image_execution_stage s USING(job_id,stage_id) WHERE u.job_id=$1 ORDER BY u.observed_at_ms,u.dispatch_id",&[&job.as_bytes().as_slice()]).await.unwrap();
    let candidates = captured.image_bytes.lock().unwrap().clone();
    assert_eq!(candidates.len(), 2);
    assert_ne!(candidates[0], candidates[1]);
    let images = raw
        .iter()
        .filter(|row| {
            matches!(
                row.get::<_, String>(0).as_str(),
                "generation" | "correction"
            )
        })
        .map(|row| row.get::<_, Vec<u8>>(1))
        .collect::<Vec<_>>();
    assert_eq!(images.len(), 2);
    for bytes in &candidates {
        assert!(
            images.contains(bytes),
            "exact private candidate bytes were not retained"
        );
    }
    let inspections = raw
        .iter()
        .filter(|row| row.get::<_, String>(0) == "inspection")
        .map(|row| row.get::<_, Vec<u8>>(1))
        .collect::<Vec<_>>();
    assert_eq!(inspections.len(), 2);
    for expected in captured.inspection_bodies.lock().unwrap().iter() {
        assert!(
            inspections.iter().any(|raw| {
                let value: Value = serde_json::from_slice(raw).unwrap();
                value["status"] == 200
                    && serde_json::from_value::<Vec<u8>>(value["body"].clone()).unwrap()
                        == *expected
            }),
            "exact raw inspection status/body disappeared"
        );
    }
    let measured = client.query("SELECT u.evidence,u.units::text FROM imagery.image_dispatch_usage u JOIN imagery.image_execution_stage s USING(job_id,stage_id) WHERE u.job_id=$1 AND s.stage_kind='inspection'", &[&job.as_bytes().as_slice()]).await.unwrap();
    assert_eq!(measured.len(), 2);
    for row in measured {
        assert_eq!(row.get::<_, String>(0), "reported");
        let units: Value = serde_json::from_str(&row.get::<_, String>(1)).unwrap();
        assert_eq!(
            units,
            json!([{"category":"text_input","quantity":120},{"category":"text_output","quantity":20}])
        );
    }
    let rows=client.query("SELECT o.payload,octet_length(o.payload)<=j.decision_payload_bytes_bound FROM imagery.image_decision_observation o JOIN imagery.image_job j USING(job_id) WHERE o.job_id=$1",&[&job.as_bytes().as_slice()]).await.unwrap();
    assert_eq!(rows.len(), 2);
    let decisions = rows
        .iter()
        .map(|row| {
            assert!(row.get::<_, bool>(1));
            serde_json::from_slice::<Value>(&row.get::<_, Vec<u8>>(0)).unwrap()
        })
        .collect::<Vec<_>>();
    let failures = decisions
        .iter()
        .filter(|value| value["verdict"] == "failed")
        .collect::<Vec<_>>();
    assert_eq!(failures.len(), if second_passes { 1 } else { 2 });
    for failure in failures {
        assert_eq!(
            failure["failure"],
            json!({"reason":"hero_count","expected":4,"seen":5})
        );
    }
    assert_eq!(
        decisions
            .iter()
            .filter(|value| value["verdict"] == "pass")
            .count(),
        usize::from(second_passes)
    );
    let counts=client.query_one("SELECT (SELECT count(*) FROM imagery.image_execution_stage WHERE job_id=$1 AND stage_kind='correction'),(SELECT count(*) FROM imagery.image_candidate_checkpoint c JOIN imagery.image_dispatch_attempt d USING(dispatch_id) WHERE d.job_id=$1),(SELECT count(*) FROM imagery.image_decision_checkpoint c JOIN imagery.image_dispatch_attempt d USING(dispatch_id) WHERE d.job_id=$1),(SELECT count(*) FROM imagery.image_dispatch_attempt d JOIN imagery.image_execution_stage s USING(job_id,stage_id) WHERE d.job_id=$1 AND s.stage_kind IN ('generation','correction','inspection'))",&[&job.as_bytes().as_slice()]).await.unwrap();
    assert_eq!(counts.get::<_, i64>(0), 1);
    assert_eq!(counts.get::<_, i64>(1), 2);
    assert_eq!(counts.get::<_, i64>(2), 2);
    assert_eq!(counts.get::<_, i64>(3), 4);
    {
        let requests = captured.generation_requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let original = "private original photographic scene prompt";
        assert!(
            requests
                .iter()
                .all(|raw| String::from_utf8_lossy(raw).contains(original))
        );
        assert!(
            !String::from_utf8_lossy(&requests[0])
                .contains("The previous result showed the wrong number")
        );
        assert!(
            String::from_utf8_lossy(&requests[1])
                .contains("The previous result showed the wrong number of player heroes.")
        );
    }
    let public=client.query_one("SELECT count(*) FROM information_schema.role_table_grants WHERE grantee='PUBLIC' AND table_schema='imagery' AND table_name IN ('image_dispatch_usage','image_candidate_observation','image_decision_observation')",&[]).await.unwrap();
    assert_eq!(
        public.get::<_, i64>(0),
        0,
        "supplier raw/candidate/decision evidence must remain private"
    );
}
async fn run_case(second_passes: bool, lose_projection_ack: bool) {
    let supplier = Supplier::open(!second_passes).await;
    let costs = DurableCosts::open();
    let Some(mut host) = ImageHostFixture::open(
        supplier.address,
        patch(supplier.address),
        costs.port.clone(),
    )
    .await
    else {
        supplier.close().await;
        return;
    };
    let source = party_source(&host).await;
    let (client, driver) = connection(host.database()).await;
    if lose_projection_ack {
        client.batch_execute("CREATE SEQUENCE imagery.fixture_inspection_fault;GRANT USAGE,SELECT ON SEQUENCE imagery.fixture_inspection_fault TO df_imagery_usage;CREATE FUNCTION imagery.fixture_inspection_ack() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.projection_durable AND EXISTS(SELECT 1 FROM imagery.image_execution_stage WHERE stage_id=NEW.stage_id AND job_id=NEW.job_id AND stage_kind='inspection') THEN PERFORM nextval('imagery.fixture_inspection_fault');RAISE EXCEPTION 'controlled inspection projection acknowledgement failure';END IF;RETURN NEW;END $$;CREATE TRIGGER fixture_inspection_ack BEFORE UPDATE OF projection_durable ON imagery.image_dispatch_usage FOR EACH ROW EXECUTE FUNCTION imagery.fixture_inspection_ack()").await.unwrap();
    }
    let context = host.imagery().consumer_context().await.unwrap();
    let request = demand(&context, source);
    let accepted = match context
        .service
        .request(&context.principal, &request)
        .await
        .unwrap()
    {
        RequestOutcome::Recorded(view) => view,
        other => panic!("new actual inspection demand: {other:?}"),
    };
    drop(context);
    if lose_projection_ack {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if client
                    .query_one(
                        "SELECT is_called FROM imagery.fixture_inspection_fault",
                        &[],
                    )
                    .await
                    .unwrap()
                    .get::<_, bool>(0)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(4)).await;
            }
        })
        .await
        .expect("actual inspection fault boundary must be reached, not an earlier Book refusal");
        host.stop_after_worker_failure().await;
        assert_eq!(supplier.captured.generations.load(Ordering::SeqCst), 1);
        assert_eq!(supplier.captured.inspections.load(Ordering::SeqCst), 1);
        let retained=client.query_one("SELECT count(*) FROM imagery.image_dispatch_usage u JOIN imagery.image_execution_stage s USING(job_id,stage_id) WHERE u.job_id=$1 AND s.stage_kind='inspection' AND u.raw_output IS NOT NULL",&[&accepted.job.as_bytes().as_slice()]).await.unwrap();
        assert_eq!(retained.get::<_, i64>(0), 1);
        client.batch_execute("DROP TRIGGER fixture_inspection_ack ON imagery.image_dispatch_usage;DROP FUNCTION imagery.fixture_inspection_ack();DROP SEQUENCE imagery.fixture_inspection_fault").await.unwrap();
        host.restart_stopped().await;
    }
    let context = host.imagery().consumer_context().await.unwrap();
    let completed = terminal(&context, &accepted.handle).await;
    assert_eq!(supplier.captured.generations.load(Ordering::SeqCst), 2);
    assert_eq!(supplier.captured.inspections.load(Ordering::SeqCst), 2);
    verify_lineage(&client, &accepted.job, &supplier.captured, second_passes).await;
    if second_passes {
        assert_eq!(completed.execution, ImageExecutionState::Ready);
        assert!(completed.result.is_some());
        let selected = client.query_one("SELECT c.candidate_kind,u.raw_output FROM imagery.image_final_selection f JOIN imagery.image_candidate_observation c ON c.job_id=f.job_id AND c.dispatch_id=f.candidate_dispatch_id JOIN imagery.image_dispatch_usage u ON u.dispatch_id=c.dispatch_id AND u.revision=c.usage_revision WHERE f.job_id=$1", &[&accepted.job.as_bytes().as_slice()]).await.unwrap();
        assert_eq!(selected.get::<_, String>(0), "correction");
        assert_eq!(
            selected.get::<_, Vec<u8>>(1),
            supplier.captured.image_bytes.lock().unwrap()[1]
        );
        let inspection = completed
            .inspection
            .as_ref()
            .expect("winning qualified retained assessment");
        assert_eq!(inspection.verdict, PhotoInspectionVerdict::Pass);
        assert_eq!(inspection.visible_features.len(), 1);
        assert_eq!(
            inspection.visible_features[0].name.text(),
            "accepted fruit stand"
        );
    } else {
        assert!(matches!(
            completed.execution,
            ImageExecutionState::Failed { .. }
        ));
        assert!(completed.result.is_none());
        assert!(completed.inspection.is_none());
        let disposition = client
            .query_one(
                "SELECT reason FROM imagery.image_no_delivery WHERE job_id=$1",
                &[&accepted.job.as_bytes().as_slice()],
            )
            .await
            .unwrap();
        assert_eq!(disposition.get::<_, String>(0), "inspection_rejected");
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
    }
    let safe = format!("{completed:?}");
    assert!(!safe.contains("private original photographic scene prompt"));
    assert!(!safe.contains("private rejected cart"));
    assert!(matches!(
        context
            .service
            .request(&context.principal, &request)
            .await
            .unwrap(),
        RequestOutcome::Replayed(_)
    ));
    drop(context);
    host.restart().await;
    let context = host.imagery().consumer_context().await.unwrap();
    let reread = terminal(&context, &accepted.handle).await;
    assert_eq!(reread.result, completed.result);
    assert_eq!(supplier.captured.generations.load(Ordering::SeqCst), 2);
    assert_eq!(supplier.captured.inspections.load(Ordering::SeqCst), 2);
    verify_lineage(&client, &accepted.job, &supplier.captured, second_passes).await;
    drop(context);
    drop(client);
    driver.abort();
    host.close().await;
    supplier.close().await;
}
#[tokio::test]
async fn hero_count_failure_corrects_once_and_only_the_qualified_retry_becomes_ready() {
    run_case(true, false).await;
}
#[tokio::test]
async fn second_hero_count_failure_retains_private_lineage_and_has_no_delivery() {
    run_case(false, false).await;
}
#[tokio::test]
async fn retained_inspection_recovers_after_projection_ack_loss_without_resending() {
    run_case(true, true).await;
}

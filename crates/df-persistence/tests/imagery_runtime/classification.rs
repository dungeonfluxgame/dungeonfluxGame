//! Actual analysis HTTP -> retained decision -> one image dependency after lost acknowledgement.
use super::*;
use std::io::{Read, Write};

fn classification_server() -> (
    std::net::SocketAddr,
    Arc<AtomicU64>,
    std::thread::JoinHandle<()>,
) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let sends = Arc::new(AtomicU64::new(0));
    let captured = sends.clone();
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(stream) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => panic!("analysis accept {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut raw = Vec::new();
        let mut buffer = [0; 4096];
        let request = loop {
            let count = stream.read(&mut buffer).unwrap();
            assert_ne!(count, 0);
            raw.extend_from_slice(&buffer[..count]);
            if let Some(end) = raw.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let headers = std::str::from_utf8(&raw[..end]).unwrap();
                assert!(headers.starts_with("POST /fixture/systemone "));
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                if raw.len() >= end + 4 + length {
                    break serde_json::from_slice::<serde_json::Value>(
                        &raw[end + 4..end + 4 + length],
                    )
                    .unwrap();
                }
            }
            assert!(raw.len() < 1_048_576);
        };
        assert_eq!(request["model"], "fixture-analysis-model");
        assert_eq!(request["max_output_tokens"], 3000);
        assert_eq!(
            request["state"]["public_appearance"],
            "An adult fictional dwarf in work clothes"
        );
        assert_eq!(
            request["questions"]["q1"]["criteria"]
                .as_object()
                .unwrap()
                .len(),
            7
        );
        assert!(request["state"].get("backstory").is_none());
        captured.fetch_add(1, Ordering::SeqCst);
        let body =
            serde_json::json!({"answers":{"q1":{"type":"choice","choice":"o1","confidence":1.0}},
            "usage":{"input_tokens":120,"output_tokens":4}})
            .to_string();
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
    });
    (address, sends, server)
}

/// The classification future is larger than a default test thread's stack, so each scenario runs
/// on a thread with a 16 MiB stack, the same bound the size check below uses.
fn run_classification_on_large_stack(late_cancelled: Option<bool>) {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(run_classification(late_cancelled));
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn classification_raw_ack_loss_recovers_one_decision_and_one_generation() {
    run_classification_on_large_stack(None);
}

#[test]
fn expired_qualified_candidate_finishes_privately_without_a_second_supplier_call() {
    run_classification_on_large_stack(Some(false));
}

#[test]
fn cancelled_qualified_candidate_finishes_privately_without_a_second_supplier_call() {
    run_classification_on_large_stack(Some(true));
}

async fn run_classification(late_cancelled: Option<bool>) {
    let scratch = Scratch::open().await;
    let config = scratch.config.clone();
    let transport = PostgresTransport::PlaintextLoopbackOnly;
    let client = connect(&config).await;
    client.batch_execute("CREATE TABLE IF NOT EXISTS imagery.schema_migration(version integer PRIMARY KEY); INSERT INTO imagery.schema_migration VALUES(38),(40) ON CONFLICT DO NOTHING").await.unwrap();
    let ids = TestIds(Arc::new(AtomicU64::new(3000)));
    let clock = MovingClock(Arc::new(std::sync::atomic::AtomicI64::new(NOW)));
    let execution = PostgresImageExecution::open(config.clone(), transport.clone(), 2)
        .await
        .unwrap();
    let scope = CapacityScope::new("runtime/classification".to_owned()).unwrap();
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
        config.clone(),
        transport.clone(),
        2,
        Arc::new(ids.clone()),
        scope.clone(),
        1,
        true,
    )
    .unwrap();
    let publication = PostgresImagePublication::open_existing(config.clone(), transport.clone(), 2)
        .await
        .unwrap();
    let (analysis_address, analysis_sends, analysis_server) = classification_server();
    let (image_address, image_sends, image_server) = fixture_server(PNG_FIXTURE);
    let analysis_route = AnalysisRoute {
        account: AccountHandle::new("fixture-analysis").unwrap(),
        model: ModelName::new("fixture-analysis-model").unwrap(),
        endpoint: ApiEndpoint::new(&format!("http://{analysis_address}/fixture/systemone"))
            .unwrap(),
        maximum_output_tokens: 3000,
    };
    let mut image_config = ImageConfig::defaults().unwrap();
    image_config.analysis.classification = Some(analysis_route.clone());
    image_config.default_backend = BackendId::OpenAi;
    image_config.backends[BackendId::OpenAi.index()].enabled = true;
    image_config.backends[BackendId::OpenAi.index()].endpoint =
        Some(ApiEndpoint::new(&format!("http://{image_address}")).unwrap());
    image_config.quality.provider_quality = Some(GenerationQuality::Medium);
    image_config.scheduler.stage_deadline_secs = 10;
    let service = ImageService::new(
        journal.clone(),
        image_config.clone(),
        clock.clone(),
        ids.clone(),
        TestAuthority(owner()),
    )
    .unwrap()
    .with_results(publication.clone())
    .with_observations(
        PostgresImageObservations::open_existing(config.clone(), transport.clone(), 2).unwrap(),
    );
    let mut demand = request();
    demand.operation = ImageOperation::Generate(GenerateSpec {
        prompt: TextPrompt::new("fixture response", 1)
            .unwrap()
            .with_pipeline(ImagePipeline {
                revision: 1,
                rating: ImageContentRating::Restrained,
                moderation: None,
                creator: Some(CreatorClassification {
                    route: analysis_route,
                    general_prompt: None,
                    contract_revision: 3,
                    public_appearance: PrivateImageText::new(
                        "An adult fictional dwarf in work clothes",
                    ),
                }),
                inspection: None,
            })
            .unwrap(),
        references: vec![],
    });
    demand.output = OutputProfile::provider(
        Dimensions::new(1024, 1024).unwrap(),
        demand.output.delivered(),
    );
    demand.identity.fingerprint = service.canonical_fingerprint(&demand).unwrap();
    let accepted = match service.request(&(), &demand).await.unwrap() {
        RequestOutcome::Recorded(view) => view,
        _ => panic!("newclassification"),
    };
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/runtime-test")
        .join(config.get_dbname().unwrap());
    let objects = Arc::new(NativeFileStore::new(&root, 16_777_216).unwrap());
    let catalog = Arc::new(
        PostgresMediaCatalog::open_existing(config.clone(), transport.clone(), 2)
            .await
            .unwrap(),
    );
    let publisher = || {
        NativeImagePublisher::new(
            RuntimePublicationSettings {
                catalog: CatalogConfig::default(),
                layout: KeyLayout::new(ObjectEnvironment::new("runtime-jev").unwrap()),
                decode_concurrency: 1,
            },
            RuntimePublicationPorts {
                catalog: catalog.clone(),
                publication: publication.clone(),
                objects: objects.clone(),
                ids: Arc::new(ids.clone()),
                clock: Arc::new(clock.clone()),
            },
        )
        .unwrap()
    };
    let credentials = FixtureCredentials(
        Arc::new(AtomicU64::new(0)),
        Arc::new(QuotaRegistry::new(Arc::new(clock.clone()))),
    );
    let costs = FixtureCost::default();
    let usage = PostgresImageUsage::open_existing(config.clone(), transport.clone(), 2).unwrap();
    let runtime = NativeImageRuntime {
        journal: journal.clone(),
        execution: execution.clone(),
        sources: NoSources,
        publisher: publisher(),
        admission: publisher(),
        clock: clock.clone(),
        ids: ids.clone(),
        credentials: credentials.clone(),
        costs: costs.clone(),
        cost_policy: FixtureCostPolicy,
        usage: LostUsageAck {
            inner: usage.clone(),
            fail: std::sync::atomic::AtomicBool::new(true),
        },
        telemetry: TestTelemetry,
        settings: RuntimeSettings {
            config: image_config.clone(),
            active_config_revision: 1,
            capacity: scope.clone(),
            poll_interval: Duration::from_millis(10),
        },
    };
    let next = runtime.run_next();
    let next_bytes = std::mem::size_of_val(&next);
    eprintln!("runtime future bytes: run_next={next_bytes}");
    assert!(
        next_bytes <= 8192,
        "owned queue future requires {next_bytes} bytes"
    );
    assert!(next.await.is_err());
    tokio::task::spawn_blocking(move || analysis_server.join().unwrap())
        .await
        .unwrap();
    assert_eq!(analysis_sends.load(Ordering::SeqCst), 1);
    assert_eq!(image_sends.load(Ordering::SeqCst), 0);
    let (control, work) = runtime.start();
    let start_bytes = std::mem::size_of_val(&work);
    eprintln!("runtime future bytes: start={start_bytes}");
    assert!(
        start_bytes <= 8192,
        "owned start future requires {start_bytes} bytes"
    );
    drop(work);
    drop(control);
    // Past the stage budget plus the pre-send lease margin, so the stale claim is reclaimable.
    clock.0.store(NOW + 41_000, Ordering::SeqCst);
    let restarted = NativeImageRuntime {
        journal: journal.clone(),
        execution: execution.clone(),
        sources: NoSources,
        publisher: publisher(),
        admission: publisher(),
        clock: clock.clone(),
        ids: ids.clone(),
        credentials: credentials.clone(),
        costs,
        cost_policy: FixtureCostPolicy,
        usage,
        telemetry: TestTelemetry,
        settings: RuntimeSettings {
            config: image_config,
            active_config_revision: 1,
            capacity: scope,
            poll_interval: Duration::from_millis(10),
        },
    };
    assert!(restarted.run_next().await.unwrap().is_none());
    if let Some(cancelled) = late_cancelled {
        if cancelled {
            service.cancel(&(), &accepted.handle).await.unwrap();
            // The restarted claim of the generation stage leases it for another stage budget plus margin.
            clock.0.store(NOW + 82_000, Ordering::SeqCst);
        } else {
            clock
                .0
                .store(accepted.absolute_deadline_ms + 1, Ordering::SeqCst);
        }
    }
    let result = restarted.run_next().await.unwrap();
    tokio::task::spawn_blocking(move || image_server.join().unwrap())
        .await
        .unwrap();
    assert_eq!(analysis_sends.load(Ordering::SeqCst), 1);
    assert_eq!(image_sends.load(Ordering::SeqCst), 1);
    let read = service.read(&(), &accepted.handle).await.unwrap();
    if late_cancelled.is_some() {
        assert!(read.result.is_none());
        assert!(result.is_none());
        let published = client.query_one(
            "SELECT r.private_reconciliation,a.state FROM imagery.image_job_result r JOIN media.asset a ON a.asset_id=r.asset_id WHERE r.job_id=$1",
            &[&accepted.job.as_bytes().as_slice()],
        ).await.unwrap();
        assert!(published.get::<_, bool>(0));
        assert_eq!(published.get::<_, String>(1), "ready");
        let disposition = client.query_one(
            "SELECT d.settlement,u.usable FROM imagery.image_execution_stage s JOIN imagery.image_stage_cursor c USING(job_id,stage_id) JOIN imagery.image_dispatch_attempt d ON d.dispatch_id=c.current_dispatch JOIN imagery.image_dispatch_usage u ON u.dispatch_id=d.dispatch_id AND u.revision=d.usage_revision WHERE s.job_id=$1 AND s.stage_kind='generation'",
            &[&accepted.job.as_bytes().as_slice()],
        ).await.unwrap();
        assert_eq!(disposition.get::<_, String>(0), "unusable_settled");
        assert!(
            disposition.get::<_, bool>(1),
            "raw technical image validity stays immutable"
        );
    } else {
        assert_eq!(read.result, Some(result.expect("one selected final image")));
        assert_eq!(read.assessment.unwrap().category, CreatorCategory::General);
    }
    assert_eq!(
        client
            .query_one(
                "SELECT count(*) FROM imagery.image_decision_checkpoint",
                &[]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM imagery.image_execution_stage", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        3
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM imagery.image_dispatch_attempt", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        3
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM finance.provider_attempt", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    drop(restarted);
    drop(service);
    drop(client);
    scratch.finish().await;
}

#[test]
fn orchestration_future_fits_a_default_worker_stack() {
    // Construction uses a bounded diagnostic stack so a regression reports a size instead of aborting.
    let bytes = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let future = run_classification(None);
            std::mem::size_of_val(&future)
        })
        .unwrap()
        .join()
        .unwrap();
    eprintln!("runtime future bytes: classification_fixture={bytes}");
    assert!(
        bytes <= 131_072,
        "inline orchestration future requires {bytes} bytes"
    );
}

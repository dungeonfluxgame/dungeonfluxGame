//! Actual central routing and production missing-custody behavior replacing the orphan parser.
use df_imagery::{BackendId, ConfigControl, RuntimePatch};

#[test]
fn central_legacy_provider_labels_preserve_default_explicit_and_redacted_invalid_cases() {
    for has_fal in [false, true] {
        for choice in [None, Some(""), Some("openai")] {
            let mut env = Vec::new();
            if let Some(choice) = choice {
                env.push(("DF_GAMEPLAY_IMAGERY_PROVIDER", choice));
            }
            if has_fal {
                env.push(("FAL_KEY", "fixture-secret"));
            }
            let actual = ConfigControl::build("", &env, &RuntimePatch::empty(), 1).unwrap();
            assert_eq!(
                actual.read_effective().config.default_backend,
                BackendId::OpenAi
            );
        }
    }
    let actual = ConfigControl::build(
        "",
        &[
            ("DF_GAMEPLAY_IMAGERY_PROVIDER", "flux"),
            ("FAL_KEY", "fixture-secret"),
        ],
        &RuntimePatch::empty(),
        1,
    )
    .unwrap();
    assert_eq!(
        actual.read_effective().config.default_backend,
        BackendId::Fal
    );
    for choice in ["OpenAI", "openai ", "private-secret"] {
        let error = ConfigControl::build(
            "",
            &[("DF_GAMEPLAY_IMAGERY_PROVIDER", choice)],
            &RuntimePatch::empty(),
            1,
        )
        .err()
        .unwrap();
        assert!(!error.to_string().contains(choice));
    }
}

struct Endpoint {
    address: std::net::SocketAddr,
    connections: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Endpoint {
    fn open() -> Self {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let connections = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (observed, stopping) = (connections.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((socket, _)) => {
                        observed.fetch_add(1, Ordering::SeqCst);
                        drop(socket);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(1))
                    }
                    Err(error) => panic!("loopback accept: {error}"),
                }
            }
        });
        Self {
            address,
            connections,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}

#[tokio::test]
async fn central_legacy_flux_without_credentials_fails_before_dispatch_supplier_and_book() {
    const CHILD: &str = "DF_TEST_MISSING_IMAGE_CREDENTIAL";
    if std::env::var_os(CHILD).is_none() {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.env_clear();
        for name in [
            "SYSTEMROOT",
            "WINDIR",
            "PATH",
            "TEMP",
            "TMP",
            "DUNGEONFLUX_FINANCE_PG_PORT",
            "DUNGEONFLUX_FINANCE_PG_USER",
            "DUNGEONFLUX_REQUIRE_SCRATCH_PG",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        let status = command.args(["--exact", "gameplay::media_policy::image_routing_tests::central_legacy_flux_without_credentials_fails_before_dispatch_supplier_and_book", "--nocapture"])
            .env(CHILD, "on").status().unwrap();
        assert!(
            status.success(),
            "isolated actual missing-credential child failed"
        );
        return;
    }
    use crate::gameplay::{cost::port::MeteringPort, image_test_support::ImageHostFixture};
    use df_imagery::*;
    use std::{sync::atomic::Ordering, time::Duration};
    let endpoint = Endpoint::open();
    let costs = MeteringPort::new();
    costs
        .open(
            "missing-image-credential",
            include_str!("../../../../../content/pricing.json"),
        )
        .unwrap();
    let mut patch = RuntimePatch::empty();
    patch.default_backend = Some(BackendId::Fal);
    patch.backend_enabled = [Some(false); BackendId::COUNT];
    patch.backend_enabled[BackendId::Fal.index()] = Some(true);
    patch.retry_max_attempts = Some(1);
    patch.scheduler_stage_deadline_secs = Some(5);
    patch.scheduler_job_deadline_secs = Some(10);
    let Some(mut fixture) = ImageHostFixture::open(endpoint.address, patch, costs.clone()).await
    else {
        return;
    };
    fixture.restart_without_fixture_credentials().await;
    let context = fixture.imagery().consumer_context().await.unwrap();
    assert_eq!(context.config.default_backend, BackendId::Fal);
    assert!(context.config.backend(BackendId::Fal).enabled);
    let audience = match &context.owner {
        ImageOwner::Run { campaign, .. } => Audience::CampaignShared(*campaign),
        ImageOwner::Draft { .. } => panic!("actual run fixture"),
    };
    let mut request = ImageRequest {
        identity: OperationIdentity {
            owner: context.owner.clone(),
            operation: df_types::OperationId::from_bytes(&[73; 16]).unwrap(),
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ScenePlate,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("A photographic fantasy market stall.", 1).unwrap(),
            references: Vec::new(),
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Visible,
        audience,
        expected_source_revision: context.expected_source_revision,
        output: context
            .config
            .output_profile_for(Purpose::ScenePlate)
            .unwrap(),
    };
    request.identity.fingerprint = context.service.canonical_fingerprint(&request).unwrap();
    let initial = context
        .service
        .request(&context.principal, &request)
        .await
        .unwrap();
    let handle = match initial {
        RequestOutcome::Recorded(view) | RequestOutcome::Replayed(view) => view.handle,
        other => panic!("valid configured work must reach actual missing custody: {other:?}"),
    };
    drop(context);
    let final_view = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let current = fixture.imagery().consumer_context().await.unwrap();
            let view = current
                .service
                .read(&current.principal, &handle)
                .await
                .unwrap();
            match view.execution {
                ImageExecutionState::Failed { .. } => break view,
                ImageExecutionState::Ready
                | ImageExecutionState::Unknown
                | ImageExecutionState::Uncertain
                | ImageExecutionState::NeedsRevision
                | ImageExecutionState::Expired
                | ImageExecutionState::Withdrawn => {
                    panic!("missing custody must fail visibly before send: {view:?}")
                }
                _ => tokio::time::sleep(Duration::from_millis(2)).await,
            }
        }
    })
    .await
    .expect("missing custody must reach a visible terminal failure");
    assert!(final_view.result.is_none());
    assert_eq!(endpoint.connections.load(Ordering::SeqCst), 0);
    assert_eq!(costs.totals().calls, 0);
    assert_eq!(costs.totals().micro_usd, 0);
    let (client, driver) = fixture
        .database()
        .connect(tokio_postgres::NoTls)
        .await
        .unwrap();
    let driver = tokio::spawn(driver);
    let attempts: i64 = client
        .query_one("SELECT count(*) FROM imagery.image_dispatch_attempt", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        attempts, 0,
        "no dispatch identity means no dispatch finance reservation or supplier attempt"
    );
    drop(client);
    driver.await.unwrap().unwrap();
    fixture.close().await;
}

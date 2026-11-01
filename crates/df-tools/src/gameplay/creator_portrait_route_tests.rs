//! Creator behavior runs the real mounted service, PostgreSQL journal and native loopback worker.
use super::super::image_test_support::ImageHostFixture;
use super::*;
use df_protocol::common::action_service_server::ActionService;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};
use tokio_postgres::{Client, Config, NoTls};
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        result.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        result.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        result.push(if c.len() > 1 {
            ALPHABET[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        result.push(if c.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    result
}

struct Loopback {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    server: Option<thread::JoinHandle<()>>,
}
impl Loopback {
    fn new(choice: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (captured, stopped, choice) = (requests.clone(), stop.clone(), choice.to_owned());
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode(
                &vec![100; 1024 * 1536 * 3],
                1024,
                1536,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        let encoded = base64(&jpeg);
        let server = thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    assert!(bytes.len() < 2 * 1024 * 1024);
                    if let Some(split) = bytes.windows(4).position(|p| p == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..split]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= split + 4 + length {
                            break;
                        }
                    }
                }
                let request = String::from_utf8(bytes).unwrap();
                captured.lock().unwrap().push(request.clone());
                let body = if request.starts_with("POST /v1/systemone ") {
                    assert!(request.contains("jev-latest"));
                    assert!(!request.contains("private-secret-history"));
                    serde_json::json!({"answers":{"q1":{"type":"choice","choice":choice,"confidence":1.0}}}).to_string()
                } else if request.starts_with("POST /fal-ai/flux-pro/v1.1-ultra ") {
                    serde_json::json!({"request_id":"creator-1","status_url":"https://queue.fal.run/fal-ai/flux-pro/requests/creator-1/status","response_url":"https://queue.fal.run/fal-ai/flux-pro/requests/creator-1"}).to_string()
                } else if request.starts_with("GET /fal-ai/flux-pro/requests/creator-1/status ") {
                    r#"{"status":"COMPLETED"}"#.to_owned()
                } else if request.starts_with("GET /fal-ai/flux-pro/requests/creator-1 ") {
                    serde_json::json!({"images":[{"url":format!("data:image/jpeg;base64,{encoded}")}]}).to_string()
                } else if request.starts_with("POST /v1/flux-3-image ") {
                    serde_json::json!({"polling_url":format!("http://{address}/v1/get_result?id=creator-bfl")}).to_string()
                } else if request.starts_with("GET /v1/get_result?id=creator-bfl ") {
                    serde_json::json!({"status":"Ready","result":{"sample":format!("http://{address}/image.jpg")}}).to_string()
                } else if request.starts_with("GET /image.jpg ") {
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",jpeg.len()).unwrap();
                    stream.write_all(&jpeg).unwrap();
                    continue;
                } else {
                    panic!(
                        "unexpected fixture path: {}",
                        request.lines().next().unwrap()
                    )
                };
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        Self {
            address,
            requests,
            stop,
            server: Some(server),
        }
    }
    fn count(&self, prefix: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.starts_with(prefix))
            .count()
    }
}
impl Drop for Loopback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(server) = self.server.take() {
            server.join().unwrap();
        }
    }
}
async fn fixture(address: SocketAddr) -> Option<ImageHostFixture> {
    let endpoint = df_imagery::ApiEndpoint::new(&format!("http://{address}")).unwrap();
    let mut patch = df_imagery::RuntimePatch::empty();
    patch.mode = Some(df_imagery::GlobalMode::Live);
    patch.default_backend = Some(df_imagery::BackendId::Fal);
    patch.accounting_mode = Some(df_imagery::AccountingMode::RecordOnly);
    patch.backend_enabled = [Some(false); df_imagery::BackendId::COUNT];
    for backend in [df_imagery::BackendId::Fal, df_imagery::BackendId::Bfl] {
        patch.backend_enabled[backend.index()] = Some(true);
        patch.backend_endpoint[backend.index()] = Some(Some(endpoint.clone()));
    }
    patch.backend_model[df_imagery::BackendId::Fal.index()] = Some(Some(
        df_imagery::ModelName::new("fal-ai/flux-pro/v1.1-ultra").unwrap(),
    ));
    patch.backend_model[df_imagery::BackendId::Bfl.index()] =
        Some(Some(df_imagery::ModelName::new("flux-3-image").unwrap()));
    patch.purpose_enabled[Purpose::CreatorPortrait.index()] = Some(true);
    patch.purpose_model_override[Purpose::CreatorPortrait.index()] = Some(None);
    patch.analysis_classification = Some(
        df_imagery::AnalysisRoute {
            account: df_imagery::AccountHandle::new("creator-jev").unwrap(),
            model: df_imagery::ModelName::new("jev-latest").unwrap(),
            endpoint,
            maximum_output_tokens: 2048,
        }
        .into(),
    );
    patch.quality_provider_quality = Some(Some(df_imagery::GenerationQuality::Medium));
    patch.retry_max_attempts = Some(1);
    let costs = super::super::cost::port::MeteringPort::new();
    costs
        .open(
            "creator-route",
            include_str!("../../../../content/pricing.json"),
        )
        .unwrap();
    ImageHostFixture::open(address, patch, costs).await
}
async fn connection(
    config: &Config,
) -> (
    Client,
    tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
) {
    let (client, driver) = config.connect(NoTls).await.unwrap();
    (client, tokio::spawn(driver))
}
struct Owner {
    player: [u8; 32],
}
impl df_session::inbox::Reducer<super::super::actor::Call> for Owner {
    fn reduce(&mut self, _: df_session::inbox::AdmissionSequence, call: super::super::actor::Call) {
        if let super::super::actor::Call::View {
            credential, reply, ..
        } = call
        {
            if credential != self.player {
                let _closed = reply.send(Err(Status::permission_denied("fixture player only")));
                return;
            }
            let view = rpc::ViewMessage {
                revision: Some(rpc::SessionRevision::default()),
                audience: Some(rpc::view_message::Audience::Player(
                    rpc::PlayerGameplayView {
                        journey: Some(rpc::JourneyView {
                            creation: Some(rpc::CharacterCreationOffer::default()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            };
            let _closed = reply.send(Ok((super::super::LocalDemoRole::Player, view)));
        } else {
            panic!("unexpected creator actor input")
        }
    }
}
fn service(
    imagery: Imagery,
    player: [u8; 32],
    display: [u8; 32],
) -> (
    super::super::Service,
    thread::JoinHandle<Result<df_session::inbox::DrainOutcome, df_session::inbox::InboxFailure>>,
) {
    let (actor, inbox) = df_session::inbox::bounded_inbox();
    let owner = thread::spawn(move || inbox.run(&mut Owner { player }));
    let (_, updates) = tokio::sync::watch::channel(super::super::model::initial().unwrap());
    (
        super::super::Service {
            imagery: imagery.clone(),
            look_ahead: Default::default(),
            actor,
            display_credential: display,
            updates,
            streams: Arc::new(tokio::sync::Semaphore::new(1)),
            voice: super::super::voice::Voice::disabled(config::VoiceOff::Qualification),
            npc_voice: Arc::new(super::super::npc_reply::NpcVoice::silent()),
            table_log: super::super::table_log::TableLog::new(),
            social: super::super::social::SocialLedger::none(),
            living: super::super::living_scenes::LivingScenes::disabled(),
            layout: super::super::dev_control::layout::LayoutState::new(),
            portraits: Portraits::new(Some(imagery)),
        },
        owner,
    )
}
fn input(operation: u8) -> rpc::HeroPortraitRequest {
    let mut identity = super::super::hero_identity::Identity::from_menus(&[])
        .unwrap()
        .to_rpc();
    identity.backstory = "private-secret-history".into();
    identity.appearance.as_mut().unwrap().public_description = "violet braids".into();
    rpc::HeroPortraitRequest {
        identity: Some(identity),
        species: "dwarf".into(),
        character_class: "fighter-1".into(),
        background: "soldier".into(),
        draft_revision: 7,
        operation_id: Some(rpc::OperationId {
            value: Some(vec![operation; 16]),
        }),
        observed_revision: Some(rpc::SessionRevision::default()),
        ..Default::default()
    }
}
fn request(
    body: &rpc::HeroPortraitRequest,
    binding: [u8; 32],
) -> Request<rpc::HeroPortraitRequest> {
    let mut request = Request::new(body.clone());
    request.metadata_mut().insert(
        "x-df-local-binding",
        super::super::hex(&binding).parse().unwrap(),
    );
    request
}
async fn settled(
    service: &super::super::Service,
    body: &rpc::HeroPortraitRequest,
    credential: [u8; 32],
) -> rpc::HeroPortraitResponse {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let view = service
            .get_hero_portrait(request(body, credential))
            .await
            .unwrap()
            .into_inner();
        if view.state != "pending" {
            return view;
        }
        assert!(Instant::now() < deadline, "creator worker did not settle");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn bytes(response: Response) -> Vec<u8> {
    assert_eq!(response.status(), StatusCode::OK);
    axum::body::to_bytes(response.into_body(), MAXIMUM_PORTRAIT_BYTES as usize + 1)
        .await
        .unwrap()
        .to_vec()
}
#[tokio::test]
async fn explicit_classified_job_uses_the_selected_flux_adapter_and_returns_an_image() {
    let server = Loopback::new("o1");
    let Some(fixture) = fixture(server.address).await else {
        return;
    };
    let (service, owner) = service(
        fixture.imagery().clone(),
        fixture.player_credential(),
        fixture.display_credential(),
    );
    let body = input(5);
    assert_eq!(
        server.requests.lock().unwrap().len(),
        0,
        "startup must not dispatch"
    );
    assert!(
        service
            .get_hero_portrait(request(&body, fixture.player_credential()))
            .await
            .is_err()
    );
    assert_eq!(
        server.requests.lock().unwrap().len(),
        0,
        "poll must not dispatch"
    );
    service
        .request_hero_portrait(request(&body, fixture.player_credential()))
        .await
        .unwrap();
    service
        .request_hero_portrait(request(&body, fixture.player_credential()))
        .await
        .unwrap();
    let ready = settled(&service, &body, fixture.player_credential()).await;
    assert_eq!(ready.state, "ready");
    assert_eq!(ready.draft_revision, 7);
    let again = service
        .get_hero_portrait(request(&body, fixture.player_credential()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        again.image_uri.split('?').next(),
        ready.image_uri.split('?').next()
    );
    assert!(
        service
            .get_hero_portrait(request(&body, [10; 32]))
            .await
            .is_err()
    );
    assert_eq!(server.count("POST /v1/systemone "), 1);
    assert_eq!(server.count("POST /fal-ai/flux-pro/v1.1-ultra "), 1);
    assert_eq!(server.count("POST /v1/flux-3-image "), 0);
    let entry = service
        .portraits
        .requests
        .lock()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    let (reference, observation) = entry.ready_reference().await.unwrap().unwrap();
    let current = entry.read().await.unwrap();
    assert_eq!(observation.operation, entry.handle.operation);
    assert_eq!(observation.job, current.job);
    assert_eq!(observation.asset, current.result.as_ref().unwrap().asset);
    assert_eq!(
        reference.canonical_source().unwrap().asset,
        observation.asset
    );
    assert_eq!(observation.config_revision, None);
    assert_eq!(
        image::load_from_memory(&reference.bytes).unwrap().width(),
        1024
    );
    let audit = df_observe::AuditContext::default();
    assert_eq!(
        bytes(
            serve_draft(
                &service.portraits,
                &audit,
                &super::super::journey::hex(&entry.grant)
            )
            .await
        )
        .await,
        reference.bytes
    );
    assert_eq!(
        serve_draft(&service.portraits, &audit, entry.key().as_str())
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        serve_draft(&service.portraits, &audit, &"1".repeat(64))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let selection = rpc::CharacterSelection {
        identity: body.identity.clone(),
        choices: [
            ("species", "dwarf"),
            ("class", "fighter-1"),
            ("background", "soldier"),
        ]
        .into_iter()
        .map(|(group, option)| rpc::JourneyChoice {
            group_id: group.into(),
            option_id: option.into(),
        })
        .collect(),
        ..Default::default()
    };
    let attached = service
        .portraits
        .accepted(fixture.player_credential(), Some(&selection))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(attached.key, entry.key().as_str());
    assert_eq!(attached.revision, 7);
    assert_eq!(
        attached.digest,
        <[u8; 32]>::from(Sha256::digest(&reference.bytes))
    );
    assert!(
        service
            .portraits
            .accepted([10; 32], Some(&selection))
            .await
            .unwrap()
            .is_none()
    );
    let mut changed = selection.clone();
    changed
        .identity
        .as_mut()
        .unwrap()
        .appearance
        .as_mut()
        .unwrap()
        .public_description = "a changed look".into();
    assert!(
        service
            .portraits
            .accepted(fixture.player_credential(), Some(&changed))
            .await
            .unwrap()
            .is_none()
    );
    let mut pin = df_model::checkpoint::AssetReference {
        key: super::super::model::label(&attached.key).unwrap(),
        digest: df_model::checkpoint::ContentDigest(attached.digest),
        byte_length: attached.bytes,
        kind: df_model::checkpoint::AssetKind::Image,
    };
    assert_eq!(
        approved_reference(&entry.imagery, &pin)
            .await
            .unwrap()
            .bytes,
        reference.bytes
    );
    pin.digest.0[0] ^= 1;
    assert!(approved_reference(&entry.imagery, &pin).await.is_err());
    // Presentation/private-history edits preserve the admitted design and its original revision.
    let mut retry = input(8);
    retry.draft_revision = 8;
    retry.identity.as_mut().unwrap().backstory = "a different private history".into();
    let reused = service
        .request_hero_portrait(request(&retry, fixture.player_credential()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(reused.state, "ready");
    assert_eq!(reused.draft_revision, 8);
    assert_eq!(
        reused.image_uri.split('?').next(),
        ready.image_uri.split('?').next()
    );
    assert_eq!(server.count("POST /v1/systemone "), 1);
    assert_eq!(server.count("POST /fal-ai/flux-pro/v1.1-ultra "), 1);
    assert_eq!(
        service
            .portraits
            .accepted(fixture.player_credential(), Some(&selection))
            .await
            .unwrap()
            .unwrap()
            .revision,
        7
    );
    let mut changed_operation = body.clone();
    changed_operation
        .identity
        .as_mut()
        .unwrap()
        .appearance
        .as_mut()
        .unwrap()
        .public_description = "a changed look".into();
    assert!(
        service
            .request_hero_portrait(request(&changed_operation, fixture.player_credential()))
            .await
            .is_err()
    );
    // An explicit new regenerate operation owns one new durable job, including its replays.
    let mut regenerate = input(9);
    regenerate.draft_revision = 9;
    let mut explicit = request(&regenerate, fixture.player_credential());
    explicit
        .metadata_mut()
        .insert("x-df-portrait-regenerate", "true".parse().unwrap());
    service.request_hero_portrait(explicit).await.unwrap();
    assert_eq!(
        settled(&service, &regenerate, fixture.player_credential())
            .await
            .state,
        "ready"
    );
    service
        .request_hero_portrait(request(&regenerate, fixture.player_credential()))
        .await
        .unwrap();
    assert_eq!(server.count("POST /v1/systemone "), 2);
    assert_eq!(server.count("POST /fal-ai/flux-pro/v1.1-ultra "), 2);
    let (client, driver) = connection(fixture.database()).await;
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM finance.provider_attempt", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM finance.draft_image_consent", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    client
        .execute(
            "UPDATE df_local_demo.grants SET active=false WHERE credential=$1",
            &[&fixture.player_credential().as_slice()],
        )
        .await
        .unwrap();
    assert_eq!(
        serve_draft(
            &service.portraits,
            &audit,
            &super::super::journey::hex(&entry.grant)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    drop(client);
    driver.abort();
    drop(reference);
    drop(entry);
    service.actor.stop().unwrap();
    drop(service);
    owner.join().unwrap().unwrap();
    fixture.close().await;
}
#[tokio::test]
async fn unsupported_and_uncertain_classifications_never_dispatch_an_image() {
    for (choice, expected) in [("o5", "failed"), ("o6", "unknown")] {
        let server = Loopback::new(choice);
        let Some(fixture) = fixture(server.address).await else {
            return;
        };
        let (service, owner) = service(
            fixture.imagery().clone(),
            fixture.player_credential(),
            fixture.display_credential(),
        );
        let body = input(5);
        service
            .request_hero_portrait(request(&body, fixture.player_credential()))
            .await
            .unwrap();
        let result = settled(&service, &body, fixture.player_credential()).await;
        assert_eq!(result.state, expected);
        assert!(result.image_uri.is_empty());
        service
            .request_hero_portrait(request(&body, fixture.player_credential()))
            .await
            .unwrap();
        assert_eq!(server.count("POST /v1/systemone "), 1);
        assert_eq!(server.count("POST /fal-ai/flux-pro/v1.1-ultra "), 0);
        assert_eq!(server.count("POST /v1/flux-3-image "), 0);
        service.actor.stop().unwrap();
        drop(service);
        owner.join().unwrap().unwrap();
        fixture.close().await;
    }
}
#[tokio::test]
async fn adult_anatomy_compatibility_uses_bfl_with_the_exact_restrained_general_prompt() {
    let server = Loopback::new("o7");
    let Some(fixture) = fixture(server.address).await else {
        return;
    };
    let (service, owner) = service(
        fixture.imagery().clone(),
        fixture.player_credential(),
        fixture.display_credential(),
    );
    let body = input(6);
    service
        .request_hero_portrait(request(&body, fixture.player_credential()))
        .await
        .unwrap();
    assert_eq!(
        settled(&service, &body, fixture.player_credential())
            .await
            .state,
        "ready"
    );
    assert_eq!(server.count("POST /fal-ai/flux-pro/v1.1-ultra "), 0);
    assert_eq!(server.count("POST /v1/flux-3-image "), 1);
    let entry = service
        .portraits
        .requests
        .lock()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    let prepared = prepare_portrait(&body).unwrap();
    let operation_key = format!(
        "{:x}",
        Sha256::digest([fixture.player_credential().as_slice(), &[6u8; 16]].concat())
    );
    let expected = imagery::creator_portrait(
        config::ContentRating::Restrained,
        imagery::HeroVisualIdentity {
            identity: &prepared.identity,
            species: &body.species,
            held_gear: &[],
        },
        &format!("{CREATOR_CONTRACT}|{operation_key}"),
        &prepared.design,
    )
    .unwrap();
    let captured = server
        .requests
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.starts_with("POST /v1/flux-3-image "))
        .unwrap()
        .clone();
    let json: serde_json::Value =
        serde_json::from_str(captured.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(json["prompt"], expected.as_str());
    assert!(entry.ready_reference().await.unwrap().is_some());
    drop(entry);
    service.actor.stop().unwrap();
    drop(service);
    owner.join().unwrap().unwrap();
    fixture.close().await;
}

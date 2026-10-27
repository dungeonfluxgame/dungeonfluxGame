//! The production classified job against loopback Jev and fal adapters.
use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::atomic::{AtomicUsize, Ordering},
    thread,
};

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

#[tokio::test]
async fn explicit_classified_job_uses_the_selected_flux_adapter_and_returns_an_image() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
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
        for index in 0..4 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut data = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                data.extend_from_slice(&buffer[..n]);
                if let Some(split) = data.windows(4).position(|b| b == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&data[..split]);
                    let len = head
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if data.len() >= split + 4 + len {
                        break;
                    }
                }
            }
            count.fetch_add(1, Ordering::SeqCst);
            let request = String::from_utf8_lossy(&data);
            let body = match index {
                0 => {
                    assert!(request.starts_with("POST /v1/systemone "));
                    assert!(request.contains("violet braids"));
                    assert!(!request.contains("private-secret-history"));
                    serde_json::json!({"answers":{"q1":{"type":"choice","choice":"o1","confidence":1.0}}}).to_string()
                }
                1 => {
                    assert!(request.starts_with("POST /fal-ai/flux-pro/v1.1-ultra "));
                    assert!(request.contains("violet braids"));
                    assert!(!request.contains("private-secret-history"));
                    serde_json::json!({"request_id":"r1","status_url":"https://queue.fal.run/fal-ai/flux-pro/requests/r1/status","response_url":"https://queue.fal.run/fal-ai/flux-pro/requests/r1"}).to_string()
                }
                2 => {
                    assert!(request.starts_with("GET /fal-ai/flux-pro/requests/r1/status "));
                    r#"{"status":"COMPLETED"}"#.to_owned()
                }
                _ => {
                    assert!(request.starts_with("GET /fal-ai/flux-pro/requests/r1 "));
                    serde_json::json!({"images":[{"url":format!("data:image/jpeg;base64,{encoded}")}]}).to_string()
                }
            };
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let root = super::super::imagery::tests::directory("portrait-flux-route");
    let hub = Hub::new();
    let provider = df_providers::NativeFalImageProvider::new(
        df_providers::FalImageConfig::loopback_fixture(addr).unwrap(),
        "fixture-fal",
        Duration::from_secs(5),
    )
    .unwrap();
    let settings = config::ImageryConfiguration {
        credential: None,
        model: df_providers::ImageEditModel::Gpt25Flare,
        quality: df_providers::ImageQuality::Medium,
        profile: Default::default(),
        interval: Duration::from_secs(30),
        deadline: Duration::from_secs(5),
        cache_dir: root.clone(),
        rating: config::ContentRating::Restrained,
    };
    let primary = Imagery::new_in_hub(
        settings,
        imagery::ImageRoute::Flux(provider),
        None,
        hub.clone(),
        true,
    )
    .unwrap();
    let classifier = Arc::new(
        Classifier::new(
            "fixture-jev",
            &format!("http://{addr}"),
            Arc::clone(hub.ledger()),
        )
        .unwrap(),
    );
    let mut identity = super::super::hero_identity::Identity::from_menus(&[])
        .unwrap()
        .to_rpc();
    identity.backstory = "private-secret-history".into();
    identity.appearance.as_mut().unwrap().public_description = "violet braids".into();
    let input = rpc::HeroPortraitRequest {
        identity: Some(identity),
        species: "dwarf".into(),
        character_class: "fighter-1".into(),
        background: "soldier".into(),
        draft_revision: 1,
        ..Default::default()
    };
    use df_session::inbox::{AdmissionSequence, Reducer};
    use rpc::action_service_server::ActionService;
    struct Owner {
        revision: rpc::SessionRevision,
    }
    impl Reducer<super::super::actor::Call> for Owner {
        fn reduce(&mut self, _: AdmissionSequence, call: super::super::actor::Call) {
            if let super::super::actor::Call::View { reply, .. } = call {
                let view = rpc::ViewMessage {
                    revision: Some(self.revision.clone()),
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
                let _ = reply.send(Ok((super::super::LocalDemoRole::Player, view)));
            } else {
                panic!("unexpected actor call");
            }
        }
    }
    let revision = rpc::SessionRevision::default();
    let (actor, inbox) = df_session::inbox::bounded_inbox();
    let owner = thread::spawn({
        let revision = revision.clone();
        move || inbox.run(&mut Owner { revision })
    });
    let (_, updates) = tokio::sync::watch::channel(super::super::model::initial().unwrap());
    let service = super::super::Service {
        look_ahead: Default::default(),
        actor,
        display_credential: [11; 32],
        updates,
        streams: Arc::new(tokio::sync::Semaphore::new(1)),
        voice: super::super::voice::Voice::disabled(config::VoiceOff::Qualification),
        npc_voice: Arc::new(super::super::npc_reply::NpcVoice::silent()),
        table_log: super::super::table_log::TableLog::new(),
        social: super::super::social::SocialLedger::none(),
        living: super::super::living_scenes::LivingScenes::disabled(),
        layout: super::super::dev_control::layout::LayoutState::new(),
        portraits: Portraits::new(Some(primary.clone()), None, Some(classifier), root.clone()),
    };
    let input = rpc::HeroPortraitRequest {
        observed_revision: Some(revision),
        operation_id: Some(rpc::OperationId {
            value: Some(vec![5; 16]),
        }),
        ..input
    };
    let make_request = |binding: u8| {
        let mut request = Request::new(input.clone());
        request.metadata_mut().insert(
            "x-df-local-binding",
            super::super::hex(&[binding; 32]).parse().unwrap(),
        );
        request
    };
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "construction must not dispatch paid work"
    );
    assert!(service.get_hero_portrait(make_request(9)).await.is_err());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "polling must not dispatch paid work"
    );
    let pending = service
        .request_hero_portrait(make_request(9))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(pending.state, "pending");
    assert_eq!(pending.draft_revision, 1);
    let repeat = service
        .request_hero_portrait(make_request(9))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(repeat.state, "pending");
    assert_eq!(hub.take_jobs(0).len(), 0);
    let mut jobs = hub.take_jobs(2);
    assert_eq!(jobs.len(), 1, "repeat must reuse the existing job");
    hub.complete(jobs.remove(0).execute().await);
    let ready = service
        .get_hero_portrait(make_request(9))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(ready.state, "ready");
    assert_eq!(ready.draft_revision, 1);
    assert!(!ready.image_uri.is_empty());
    assert!(
        service.get_hero_portrait(make_request(10)).await.is_err(),
        "another seat cannot poll this draft"
    );
    let again = service
        .get_hero_portrait(make_request(9))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        again.image_uri.split('?').next(),
        ready.image_uri.split('?').next()
    );
    assert!(hub.take_jobs(2).is_empty());
    service.actor.stop().unwrap();
    drop(service);
    owner.join().unwrap().unwrap();
    server.join().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn unsupported_and_uncertain_classifications_never_dispatch_an_image() {
    for choice in ["o5", "o6"] {
        let jev = TcpListener::bind("127.0.0.1:0").unwrap();
        let jev_addr = jev.local_addr().unwrap();
        let fal = TcpListener::bind("127.0.0.1:0").unwrap();
        fal.set_nonblocking(true).unwrap();
        let fal_addr = fal.local_addr().unwrap();
        let response = choice.to_owned();
        let server = thread::spawn(move || {
            let (mut stream, _) = jev.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut data = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                data.extend_from_slice(&buffer[..n]);
                if let Some(split) = data.windows(4).position(|b| b == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&data[..split]);
                    let len = head
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if data.len() >= split + 4 + len {
                        break;
                    }
                }
            }
            let body = serde_json::json!({"answers":{"q1":{"type":"choice","choice":response,"confidence":1.0}}}).to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        let root = super::super::imagery::tests::directory("portrait-refusal");
        let hub = Hub::new();
        let provider = df_providers::NativeFalImageProvider::new(
            df_providers::FalImageConfig::loopback_fixture(fal_addr).unwrap(),
            "fixture-fal",
            Duration::from_secs(5),
        )
        .unwrap();
        let settings = config::ImageryConfiguration {
            credential: None,
            model: df_providers::ImageEditModel::Gpt25Flare,
            quality: df_providers::ImageQuality::Medium,
            profile: Default::default(),
            interval: Duration::from_secs(30),
            deadline: Duration::from_secs(5),
            cache_dir: root.clone(),
            rating: config::ContentRating::Restrained,
        };
        let primary = Imagery::new_in_hub(
            settings,
            imagery::ImageRoute::Flux(provider),
            None,
            hub.clone(),
            true,
        )
        .unwrap();
        let classifier = Arc::new(
            Classifier::new(
                "fixture-jev",
                &format!("http://{jev_addr}"),
                Arc::clone(hub.ledger()),
            )
            .unwrap(),
        );
        let identity = super::super::hero_identity::Identity::from_menus(&[])
            .unwrap()
            .to_rpc();
        let input = rpc::HeroPortraitRequest {
            identity: Some(identity),
            species: "dwarf".into(),
            character_class: "fighter-1".into(),
            background: "soldier".into(),
            draft_revision: 1,
            ..Default::default()
        };
        let prepared = prepare_portrait(&input).unwrap();
        let prompt = imagery::creator_portrait(
            config::ContentRating::Restrained,
            imagery::HeroVisualIdentity {
                identity: &prepared.identity,
                species: &input.species,
                held_gear: &[],
            },
            "test-operation",
            &prepared.design,
        )
        .unwrap();
        let request = ImageRequest {
            kind: ImageKind::CanonicalCharacter,
            references: vec![],
            prompt,
            output: OutputSpec {
                width: 1024,
                height: 1536,
                transparent: false,
            },
        };
        let job = PortraitJob {
            audit: df_observe::AuditContext::default(),
            classifier,
            source: prepared.public,
            operation: "test-operation".into(),
            primary,
            alternate: None,
            general_request: request.clone(),
            request,
            progress: Arc::new(Mutex::new(String::new())),
        };
        assert!(job.run().await.is_err());
        server.join().unwrap();
        assert_eq!(
            fal.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

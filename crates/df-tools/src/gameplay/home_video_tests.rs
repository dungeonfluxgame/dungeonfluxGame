//! Native checks of the home video: the cache key, the byte ranges the route serves, and the flight
//! against a loopback fal fake. The fake counts what the adapter sends, so "one submit per
//! painting", "the budget is charged before the submit" and "a cached loop costs nothing" are
//! checked against the traffic itself, not against the engine's own bookkeeping.
use super::home_video::{
    self, Endpoints, LoopView, Span, clip_key, clip_key_with, respond, scene_test_engine, span_for,
    test_engine,
};
use super::rpc;
use axum::{
    body::to_bytes,
    http::{HeaderMap, HeaderValue, StatusCode, header},
};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, ToSocketAddrs},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const PAINTING: &str = "assets/ui/generated/scene-thistledown-farm-arrival";
const OTHER: &str = "assets/ui/generated/scene-reedmoor-boardwalk-arrival";

fn asset_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui")
}

fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("df-home-video-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a scratch cache directory");
    directory
}

/// What the fal fake answers, chosen per test.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scenario {
    /// Completed, with bytes that are not an MP4: the adapter must refuse them.
    MalformedClip,
    /// The queue reports the request failed.
    Refused,
    /// The queue never finishes.
    NeverFinishes,
}

struct Fake {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    bodies: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl Fake {
    /// Requests of one kind: `POST` submits, `GET` polls and downloads, `pricing` the price check.
    fn count(&self, what: &str) -> usize {
        self.requests
            .lock()
            .expect("the request log")
            .iter()
            .filter(|line| line.contains(what))
            .count()
    }
}

/// A loopback fal fake: answers the price check, the queue submit, status, result and the
/// download, one connection per request.
fn fake(scenario: Scenario) -> Fake {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
    let address = listener.local_addr().expect("a loopback address");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&requests);
    let captured_bodies = Arc::clone(&bodies);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let log = Arc::clone(&log);
            let captured_bodies = Arc::clone(&captured_bodies);
            thread::spawn(move || {
                let _ = answer(&mut stream, scenario, &log, &captured_bodies);
            });
        }
    });
    Fake {
        address,
        requests,
        bodies,
    }
}

fn answer(
    stream: &mut std::net::TcpStream,
    scenario: Scenario,
    log: &Mutex<Vec<String>>,
    bodies: &Mutex<Vec<Vec<u8>>>,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte)? == 0 {
            return Ok(());
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).to_string();
    let request_line = text.lines().next().unwrap_or_default().to_owned();
    let length = text
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body)?;
    if request_line.starts_with("POST ")
        && let Ok(mut captured) = bodies.lock()
    {
        captured.push(body.clone());
    }
    if let Ok(mut lines) = log.lock() {
        lines.push(request_line.clone());
    }
    let path = request_line.split_whitespace().nth(1).unwrap_or_default();
    let (content_type, payload): (&str, Vec<u8>) = if path.starts_with("/v1/models/pricing") {
        (
            "application/json",
            br#"{"prices":[{"endpoint_id":"fal-ai/kling-video/o1/image-to-video","unit_price":0.112,"unit":"seconds","currency":"USD"}],"next_cursor":null,"has_more":false}"#
                .to_vec(),
        )
    } else if path.ends_with("/clip.mp4") {
        ("video/mp4", b"this is not an mp4 container at all".to_vec())
    } else if request_line.starts_with("POST ") {
        (
            "application/json",
            br#"{"request_id":"req-home-video-1","status_url":"https://queue.fal.run/fal-ai/kling-video/o1/image-to-video/requests/req-home-video-1/status","response_url":"https://queue.fal.run/fal-ai/kling-video/o1/image-to-video/requests/req-home-video-1","cancel_url":"https://queue.fal.run/fal-ai/kling-video/o1/image-to-video/requests/req-home-video-1/cancel"}"#
                .to_vec(),
        )
    } else if path.ends_with("/status") {
        // The queue names the request in every status; a refusal is a completed request that
        // carries an error type, which is how the adapter's contract reads a failed job.
        let status = match scenario {
            Scenario::Refused => {
                r#"{"status":"COMPLETED","error_type":"RunnerError","error":"content refused"}"#
            }
            Scenario::NeverFinishes => r#"{"status":"IN_QUEUE","queue_position":0}"#,
            Scenario::MalformedClip => r#"{"status":"COMPLETED"}"#,
        };
        ("application/json", status.as_bytes().to_vec())
    } else if path.ends_with("/req-home-video-1") {
        // The result is the request itself, as fal's queue schema names it.
        (
            "application/json",
            br#"{"video":{"url":"https://v3b.fal.media/out/clip.mp4","content_type":"video/mp4","file_name":"clip.mp4","file_size":34}}"#
                .to_vec(),
        )
    } else {
        ("application/json", b"{}".to_vec())
    };
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(&payload)?;
    Ok(())
}

fn endpoints(fake: &Fake) -> Endpoints {
    Endpoints::Fixture {
        api: fake.address,
        media: fake.address,
    }
}

/// Waits until the flight for `painting` has an outcome, or fails after a bounded wait.
async fn settle(engine: &Arc<home_video::Engine>, painting: &str) -> String {
    let started = Instant::now();
    loop {
        let stage = engine.stage_of(painting).unwrap_or_default();
        if !stage.is_empty() && stage != "pending" {
            return stage;
        }
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "the flight for {painting} never settled"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn settle_scene(engine: &Arc<home_video::Engine>, plate_key: &str) -> String {
    let started = Instant::now();
    loop {
        let stage = engine.scene_stage_of(plate_key).unwrap_or_default();
        if !stage.is_empty() && stage != "pending" {
            return stage;
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the scene clip for {plate_key} never settled"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn scene_still() -> Vec<u8> {
    let pixels = image::RgbImage::from_pixel(8, 4, image::Rgb([23, 81, 144]));
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95)
        .encode(pixels.as_raw(), 8, 4, image::ExtendedColorType::Rgb8)
        .expect("the synthetic scene plate encodes");
    bytes
}

#[test]
fn the_clip_key_changes_with_the_still_the_model_the_prompt_the_duration_and_the_recipe() {
    let still = b"a still";
    let base = clip_key_with(still, "model-a", "prompt", 5, "recipe-1");
    assert_eq!(base.len(), 64, "a SHA-256 in lowercase hex");
    assert_eq!(
        base,
        clip_key_with(still, "model-a", "prompt", 5, "recipe-1")
    );
    assert_ne!(
        base,
        clip_key_with(b"another still", "model-a", "prompt", 5, "recipe-1")
    );
    assert_ne!(
        base,
        clip_key_with(still, "model-b", "prompt", 5, "recipe-1")
    );
    assert_ne!(
        base,
        clip_key_with(still, "model-a", "prompt!", 5, "recipe-1")
    );
    assert_ne!(
        base,
        clip_key_with(still, "model-a", "prompt", 6, "recipe-1")
    );
    assert_ne!(
        base,
        clip_key_with(still, "model-a", "prompt", 5, "recipe-2")
    );
    assert_eq!(clip_key(still).len(), 64);
}

#[test]
fn a_range_names_a_whole_file_a_middle_a_suffix_or_no_byte_at_all() {
    assert_eq!(span_for(None, 100), Span::Whole);
    assert_eq!(
        span_for(Some("bytes=10-19"), 100),
        Span::Part {
            first: 10,
            last: 19
        }
    );
    assert_eq!(
        span_for(Some("bytes=10-"), 100),
        Span::Part {
            first: 10,
            last: 99
        }
    );
    assert_eq!(
        span_for(Some("bytes=-5"), 100),
        Span::Part {
            first: 95,
            last: 99
        }
    );
    assert_eq!(
        span_for(Some("bytes=90-500"), 100),
        Span::Part {
            first: 90,
            last: 99
        }
    );
    assert_eq!(span_for(Some("bytes=100-"), 100), Span::Unsatisfiable);
    assert_eq!(span_for(Some("bytes=-0"), 100), Span::Unsatisfiable);
    assert_eq!(
        span_for(Some("bytes=0-1,5-6"), 100),
        Span::Whole,
        "several ranges serve the whole file"
    );
    assert_eq!(
        span_for(Some("bytes=9-3"), 100),
        Span::Whole,
        "an inverted range is ignored"
    );
    assert_eq!(
        span_for(Some("items=0-3"), 100),
        Span::Whole,
        "another unit is ignored"
    );
}

#[tokio::test]
async fn the_route_serves_a_whole_clip_a_range_a_suffix_and_refuses_an_unsatisfiable_one() {
    let directory = scratch("route");
    let key = "a".repeat(64);
    let clip: Vec<u8> = (0u8..=199).collect();
    std::fs::write(directory.join(format!("{key}.mp4")), &clip).expect("a cached clip");
    let whole = respond(&directory, &key, &HeaderMap::new()).await;
    assert_eq!(whole.status(), StatusCode::OK);
    assert_eq!(whole.headers()[header::CONTENT_TYPE], "video/mp4");
    assert_eq!(whole.headers()[header::ACCEPT_RANGES], "bytes");
    assert_eq!(whole.headers()[header::ETAG], format!("\"{key}\"").as_str());
    assert_eq!(whole.headers()[header::CONTENT_LENGTH], "200");
    let body = to_bytes(whole.into_body(), usize::MAX)
        .await
        .expect("the whole body");
    assert_eq!(body.as_ref(), clip.as_slice());

    let mut range = HeaderMap::new();
    range.insert(header::RANGE, HeaderValue::from_static("bytes=50-59"));
    let middle = respond(&directory, &key, &range).await;
    assert_eq!(middle.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(middle.headers()[header::CONTENT_RANGE], "bytes 50-59/200");
    let part = to_bytes(middle.into_body(), usize::MAX)
        .await
        .expect("the middle body");
    assert_eq!(part.as_ref(), &clip[50..60]);

    range.insert(header::RANGE, HeaderValue::from_static("bytes=-4"));
    let suffix = respond(&directory, &key, &range).await;
    assert_eq!(suffix.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(suffix.headers()[header::CONTENT_RANGE], "bytes 196-199/200");

    range.insert(header::RANGE, HeaderValue::from_static("bytes=500-"));
    let unsatisfiable = respond(&directory, &key, &range).await;
    assert_eq!(unsatisfiable.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(
        unsatisfiable.headers()[header::CONTENT_RANGE],
        "bytes */200"
    );

    let unknown = respond(&directory, &"b".repeat(64), &HeaderMap::new()).await;
    assert_eq!(
        unknown.status(),
        StatusCode::NOT_FOUND,
        "an uncached key is not served"
    );
    let malformed = respond(&directory, "../../etc/passwd", &HeaderMap::new()).await;
    assert_eq!(
        malformed.status(),
        StatusCode::NOT_FOUND,
        "a path is never a key"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn cache_containment_uses_resolved_paths() {
    let directory = scratch("containment");
    let root = std::fs::canonicalize(&directory).expect("the cache root");
    assert!(home_video::is_within_cache(&root, &root.join("clip.mp4")));
    assert!(!home_video::is_within_cache(
        &root,
        &root.parent().expect("the parent").join("outside.mp4")
    ));
    let _ = std::fs::remove_dir_all(directory);
}

#[cfg(unix)]
#[tokio::test]
async fn a_cached_clip_symlink_to_an_outside_file_is_not_served() {
    let parent = scratch("outside-link");
    let cache = parent.join("cache");
    std::fs::create_dir_all(&cache).expect("the cache directory");
    let key = "a".repeat(64);
    let outside = parent.join("outside.mp4");
    std::fs::write(&outside, [0u8; 13]).expect("the outside file");
    std::os::unix::fs::symlink(&outside, cache.join(format!("{key}.mp4")))
        .expect("the cache symlink");

    let response = respond(&cache, &key, &HeaderMap::new()).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let _ = std::fs::remove_dir_all(parent);
}

#[tokio::test]
async fn a_cached_loop_is_served_at_once_with_no_provider_call() {
    let fake = fake(Scenario::MalformedClip);
    let directory = scratch("cached");
    let root = asset_root();
    let engine = test_engine(
        root.clone(),
        directory.clone(),
        4,
        endpoints(&fake),
        Duration::from_secs(30),
    );
    let still = home_video::prepare_still_for_test(&root, PAINTING)
        .await
        .expect("the still");
    let key = clip_key(&still);
    std::fs::write(directory.join(format!("{key}.mp4")), vec![0u8; 64]).expect("a cached clip");
    let view = engine.observe_for_test(PAINTING);
    assert_eq!(settle(&engine, PAINTING).await, format!("ready:{key}"));
    assert_eq!(
        view,
        LoopView::Pending,
        "the first view is pending while the flight checks the cache"
    );
    assert_eq!(
        engine.observe_for_test(PAINTING),
        LoopView::Ready {
            path: format!("/gameplay/home-video/{key}")
        }
    );
    assert_eq!(fake.count("POST"), 0, "a cached loop makes no submit");
    assert_eq!(fake.count("pricing"), 0, "nor a price check");
    assert_eq!(engine.budget_left(), 4, "nor any charge");
    let _ = std::fs::remove_dir_all(&directory);
}

#[tokio::test]
async fn one_submit_serves_every_display_that_asks_for_the_same_painting() {
    let fake = fake(Scenario::MalformedClip);
    let directory = scratch("one-submit");
    let engine = test_engine(
        asset_root(),
        directory.clone(),
        4,
        endpoints(&fake),
        Duration::from_secs(30),
    );
    for _ in 0..3 {
        assert_eq!(engine.observe_for_test(PAINTING), LoopView::Pending);
    }
    let stage = settle(&engine, PAINTING).await;
    assert!(
        stage.starts_with("failed:"),
        "a malformed clip fails closed: {stage}"
    );
    for _ in 0..3 {
        assert_eq!(engine.observe_for_test(PAINTING), LoopView::Failed);
    }
    assert_eq!(fake.count("POST"), 1, "three displays, one submit");
    assert_eq!(engine.budget_left(), 3, "one clip charged");
    let _ = std::fs::remove_dir_all(&directory);
}

#[tokio::test]
async fn the_budget_is_charged_before_each_submit_and_refused_once_spent() {
    let fake = fake(Scenario::MalformedClip);
    let directory = scratch("budget");
    let engine = test_engine(
        asset_root(),
        directory.clone(),
        1,
        endpoints(&fake),
        Duration::from_secs(30),
    );
    engine.observe_for_test(PAINTING);
    settle(&engine, PAINTING).await;
    assert_eq!(engine.budget_left(), 0);
    engine.observe_for_test(OTHER);
    let refused = settle(&engine, OTHER).await;
    assert!(
        refused.contains("budget spent"),
        "refused by name: {refused}"
    );
    assert_eq!(
        fake.count("POST"),
        1,
        "the refused painting never reached the provider"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[tokio::test]
async fn a_refusal_or_failure_from_the_provider_leaves_the_still_with_a_diagnostic() {
    let fake = fake(Scenario::Refused);
    let directory = scratch("refused");
    let engine = test_engine(
        asset_root(),
        directory.clone(),
        4,
        endpoints(&fake),
        Duration::from_secs(30),
    );
    engine.observe_for_test(PAINTING);
    let stage = settle(&engine, PAINTING).await;
    assert!(
        stage.starts_with("failed:provider"),
        "reported as a provider failure: {stage}"
    );
    assert!(
        stage.contains("content_policy") || stage.contains("refused"),
        "{stage}"
    );
    assert_eq!(
        engine.observe_for_test(PAINTING),
        LoopView::Failed,
        "the view keeps the still"
    );
    assert_eq!(
        LoopView::Failed.path(),
        "",
        "no path is served for a failed loop"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[tokio::test]
async fn a_malformed_clip_fails_closed_and_is_not_cached() {
    let fake = fake(Scenario::MalformedClip);
    let directory = scratch("malformed");
    let engine = test_engine(
        asset_root(),
        directory.clone(),
        4,
        endpoints(&fake),
        Duration::from_secs(30),
    );
    engine.observe_for_test(PAINTING);
    let stage = settle(&engine, PAINTING).await;
    assert!(stage.starts_with("failed:"), "{stage}");
    let cached = std::fs::read_dir(&directory).map_or(0, |entries| entries.count());
    assert_eq!(cached, 0, "a refused file is never written to the cache");
    let _ = std::fs::remove_dir_all(&directory);
}

#[tokio::test]
async fn a_timeout_leaves_the_still_and_the_spend_is_kept() {
    let fake = fake(Scenario::NeverFinishes);
    let directory = scratch("timeout");
    let engine = test_engine(
        asset_root(),
        directory.clone(),
        4,
        endpoints(&fake),
        Duration::from_millis(900),
    );
    engine.observe_for_test(PAINTING);
    let stage = settle(&engine, PAINTING).await;
    assert_eq!(
        stage, "failed:timeout",
        "a clip that never finishes times out"
    );
    assert_eq!(
        engine.budget_left(),
        3,
        "a timed-out clip stays charged, never refunded"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[tokio::test]
async fn the_display_lobby_view_carries_the_loop_once_the_flight_has_landed_it() {
    // Arrange: a cached loop for the farm's still and the engine the view reads.
    let fake = fake(Scenario::MalformedClip);
    let directory = scratch("display-view");
    let root = asset_root();
    let still = home_video::prepare_still_for_test(&root, PAINTING)
        .await
        .expect("the still");
    let key = clip_key(&still);
    std::fs::write(directory.join(format!("{key}.mp4")), vec![0u8; 64]).expect("a cached clip");
    let engine = test_engine(
        root,
        directory.clone(),
        4,
        endpoints(&fake),
        Duration::from_secs(30),
    );
    home_video::install_for_test(Arc::clone(&engine)).expect("the test engine is installed once");
    let lobby = || rpc::ViewMessage {
        audience: Some(rpc::view_message::Audience::Display(
            rpc::DisplayGameplayView {
                narration: String::new(),
                scene_asset: PAINTING.to_owned(),
                scene: None,
                scene_graph: None,
                live_plate_graph_revision: 0,
                check: None,
                encounter_xp: None,
                journey: Some(rpc::JourneyView {
                    phase: rpc::JourneyPhase::Room as i32,
                    ..Default::default()
                }),
            },
        )),
        ..Default::default()
    };
    // Act: the first view starts the flight; a later view, after the flight landed, reads it.
    let mut first = lobby();
    home_video::apply(&mut first);
    assert_eq!(settle(&engine, PAINTING).await, format!("ready:{key}"));
    let mut later = lobby();
    home_video::apply(&mut later);
    // Assert: the display's lobby carries the ready state and the served path.
    let journey = match later.audience {
        Some(rpc::view_message::Audience::Display(display)) => display.journey.expect("a journey"),
        _ => panic!("a display view"),
    };
    assert_eq!(journey.home_video_state, rpc::HomeVideoState::Ready as i32);
    assert_eq!(
        journey.home_video_path,
        format!("/gameplay/home-video/{key}")
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// Development check: one real generation of a painting's loop through the engine, reporting the
/// outcome and the time. It spends one clip, and runs only when `DF_HOME_VIDEO_LIVE_GENERATE` names
/// the painting; the key comes from `FAL_KEY` in the environment of that one command.
#[tokio::test]
#[ignore = "live paid check; set DF_HOME_VIDEO_LIVE_GENERATE to a painting and FAL_KEY in one command"]
async fn live_generation_of_one_painting() {
    let Ok(painting) = std::env::var("DF_HOME_VIDEO_LIVE_GENERATE") else {
        return;
    };
    let credential = std::env::var("FAL_KEY").expect("FAL_KEY is set for this check");
    // The same two CDN hosts the server pins.
    let pins: Vec<df_providers::FalCdnPin> = ["v3.fal.media", "v3b.fal.media"]
        .into_iter()
        .filter_map(|host| {
            let addresses: Vec<std::net::IpAddr> = (host, 443)
                .to_socket_addrs()
                .ok()?
                .map(|address| address.ip())
                .collect();
            df_providers::FalCdnPin::new(host, &addresses).ok()
        })
        .collect();
    let directory = PathBuf::from(
        std::env::var("DF_HOME_VIDEO_LIVE_CACHE").expect("the live cache directory is named"),
    );
    let engine = home_video::live_test_engine(
        asset_root(),
        directory,
        1,
        credential,
        pins,
        Duration::from_secs(20 * 60),
    );
    let started = Instant::now();
    engine.observe_for_test(&painting);
    let stage = settle_long(&engine, &painting).await;
    println!(
        "LIVE GENERATION {painting}: {stage} after {:.0}s",
        started.elapsed().as_secs_f64()
    );
    assert!(
        stage.starts_with("ready:"),
        "the generation did not produce a loop: {stage}"
    );
}

/// Waits for an outcome for as long as a real generation can take.
async fn settle_long(engine: &Arc<home_video::Engine>, painting: &str) -> String {
    let started = Instant::now();
    loop {
        let stage = engine.stage_of(painting).unwrap_or_default();
        if !stage.is_empty() && stage != "pending" {
            return stage;
        }
        assert!(
            started.elapsed() < Duration::from_secs(21 * 60),
            "the generation never settled"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Development check, not a gate: the live price of the endpoint through the adapter, with no
/// submit and no charge. Runs only when `DF_HOME_VIDEO_LIVE_PRICE` is set, with the key in the
/// environment of that one command.
#[tokio::test]
#[ignore = "live provider check; set DF_HOME_VIDEO_LIVE_PRICE and FAL_KEY in one command"]
async fn live_price_check_without_a_submit() {
    if std::env::var_os("DF_HOME_VIDEO_LIVE_PRICE").is_none() {
        return;
    }
    let credential = std::env::var("FAL_KEY").expect("FAL_KEY is set for this check");
    let addresses: Vec<std::net::IpAddr> = ("v3b.fal.media", 443)
        .to_socket_addrs()
        .expect("the CDN host resolves")
        .map(|address| address.ip())
        .collect();
    let pin = df_providers::FalCdnPin::new("v3b.fal.media", &addresses).expect("the pin is public");
    let config = df_providers::FalMediaConfig::live(df_providers::FalMediaFamily::Video, vec![pin])
        .with_video_model(home_video::MODEL)
        .expect("the model is accepted");
    let limits = df_providers::NativeHttpLimits {
        connect_timeout: Duration::from_secs(10),
        read_timeout: Duration::from_secs(60),
        total_timeout: Duration::from_secs(170),
        maximum_request_bytes: 8 * 1024 * 1024,
        maximum_response_bytes: 16 * 1024 * 1024,
        maximum_header_bytes: 16 * 1024,
        maximum_chunks: 65_536,
    };
    let provider = df_providers::registered_fal_media_provider(config, &credential, limits)
        .expect("the provider is constructed");
    let price = provider
        .video_price_per_second(Duration::from_secs(20), std::future::pending())
        .await;
    println!("LIVE PRICE CHECK: {price:?}");
    price.expect("the live price is read");
}

/// Development helper, not a gate: writes the cache key of a painting's still to the file named by
/// `DF_HOME_VIDEO_KEY_FILE`, so a browser check can seed the cache without a paid request.
/// Run with `--ignored`.
#[tokio::test]
#[ignore = "development helper; set DF_HOME_VIDEO_KEY_FILE and DF_HOME_VIDEO_PAINTING"]
async fn print_the_cache_key_of_a_painting_for_seeding() {
    let Some(path) = std::env::var_os("DF_HOME_VIDEO_KEY_FILE") else {
        return;
    };
    let painting = std::env::var("DF_HOME_VIDEO_PAINTING").unwrap_or_else(|_| PAINTING.to_owned());
    let still = home_video::prepare_still_for_test(&asset_root(), &painting)
        .await
        .expect("the still is prepared");
    std::fs::write(path, clip_key(&still)).expect("the key file is writable");
}

fn inputs(
    mode: Option<&str>,
    key: Option<&str>,
    budget: Option<&str>,
    model: Option<&str>,
) -> home_video::HomeVideoInputs {
    inputs_with_scene(mode, None, key, budget, model)
}

fn inputs_with_scene(
    mode: Option<&str>,
    scene_mode: Option<&str>,
    key: Option<&str>,
    budget: Option<&str>,
    model: Option<&str>,
) -> home_video::HomeVideoInputs {
    let value = |name: Option<&str>| match name {
        Some(text) => Ok(text.to_owned()),
        None => Err(std::env::VarError::NotPresent),
    };
    home_video::HomeVideoInputs {
        mode: value(mode),
        scene_mode: value(scene_mode),
        model: value(model),
        budget: value(budget),
        cache: Err(std::env::VarError::NotPresent),
        credential: value(key),
    }
}

#[test]
fn the_scene_video_switch_defaults_from_the_provider_and_can_be_explicitly_disabled_or_enabled() {
    let current = Path::new("/work");
    let off = home_video::home_video_settings(
        inputs_with_scene(None, Some("off"), Some("fixture-key"), None, None),
        current,
        false,
    )
    .expect("scene clips can be disabled");
    assert!(
        home_video::startup_line(&off).contains("SCENE VIDEO off (DF_GAMEPLAY_SCENE_VIDEO=off)")
    );

    let keyless = home_video::home_video_settings(
        inputs_with_scene(None, Some("on"), None, None, None),
        current,
        false,
    )
    .expect("explicit scene mode can report the unavailable provider");
    assert_eq!(
        home_video::startup_line(&keyless),
        "HOME VIDEO off (no FAL_KEY); SCENE VIDEO on model=fal-ai/kling-video/o1/image-to-video duration=5s (no FAL_KEY; request logging only)"
    );

    let invalid = home_video::home_video_settings(
        inputs_with_scene(None, Some("maybe"), Some("fixture-key"), None, None),
        current,
        false,
    )
    .expect_err("an unknown scene video mode is refused");
    assert!(invalid.to_string().contains("DF_GAMEPLAY_SCENE_VIDEO"));
}

#[tokio::test]
async fn a_scene_plate_submits_once_with_its_first_frame_prompt_and_failure_keeps_the_still() {
    let fake = fake(Scenario::Refused);
    let directory = scratch("scene-request");
    let engine = scene_test_engine(
        directory.clone(),
        endpoints(&fake),
        Duration::from_secs(30),
        Some("fixture-private-key".to_owned()),
    );
    let plate_key = "a".repeat(64);
    let still = scene_still();
    let description = "the innkeeper at mid-left rolls dice beside the fire";
    engine.request_scene_for_test(
        &plate_key,
        still.clone(),
        description,
        "current-story-plate",
        false,
    );
    engine.request_scene_for_test(
        &plate_key,
        still.clone(),
        description,
        "current-story-plate",
        false,
    );
    let stage = settle_scene(&engine, &plate_key).await;
    assert!(stage.starts_with("failed:"), "{stage}");
    assert!(engine.scene_view_for_test(&plate_key).path().is_empty());
    assert_eq!(fake.count("POST "), 1, "a plate is submitted only once");

    let body = {
        let bodies = fake.bodies.lock().expect("captured request bodies");
        assert_eq!(bodies.len(), 1);
        bodies[0].clone()
    };
    let request: serde_json::Value = serde_json::from_slice(&body).expect("fal JSON");
    let prepared = home_video::prepare_scene_still_for_test(still)
        .await
        .expect("the first frame is normalized");
    let expected_frame = format!(
        "data:image/jpeg;base64,{}",
        home_video::base64_encode(&prepared)
    );
    assert_eq!(request["start_image_url"], expected_frame);
    assert_eq!(request["end_image_url"], expected_frame);
    assert_eq!(request["duration"], "5");
    let prompt = request["prompt"].as_str().expect("ambient prompt");
    assert!(prompt.contains(description));
    assert!(prompt.contains("camera completely still"));
    assert!(prompt.contains("breathe, blink, and shift their weight"));
    assert!(prompt.contains("dust drifts slowly"));
}

#[tokio::test]
async fn a_cached_scene_plate_is_ready_immediately_and_a_new_plate_edit_gets_its_own_request() {
    let fake = fake(Scenario::Refused);
    let directory = scratch("scene-cache");
    let cached_key = "b".repeat(64);
    std::fs::write(directory.join(format!("{cached_key}.mp4")), vec![0u8; 64])
        .expect("cached scene clip");
    let engine = scene_test_engine(
        directory.clone(),
        endpoints(&fake),
        Duration::from_secs(30),
        Some("fixture-private-key".to_owned()),
    );
    engine.request_scene_for_test(
        &cached_key,
        scene_still(),
        "a dice game beside the fire",
        "current-story-plate",
        false,
    );
    assert_eq!(
        engine.scene_view_for_test(&cached_key),
        home_video::SceneClipView::Ready {
            path: format!("/gameplay/home-video/{cached_key}")
        }
    );
    assert_eq!(
        fake.count("POST "),
        0,
        "a cached plate makes no provider call"
    );

    let edited_key = "c".repeat(64);
    engine.request_scene_for_test(
        &edited_key,
        scene_still(),
        "the same group now carries a tray",
        "current-story-plate-edit",
        false,
    );
    let stage = settle_scene(&engine, &edited_key).await;
    assert!(stage.starts_with("failed:"), "{stage}");
    assert_eq!(
        fake.count("POST "),
        1,
        "the edited plate has its own request"
    );
}

#[tokio::test]
async fn scene_clips_are_not_requested_during_combat_or_a_roll() {
    let fake = fake(Scenario::Refused);
    let directory = scratch("scene-blocked");
    let engine = scene_test_engine(
        directory,
        endpoints(&fake),
        Duration::from_secs(30),
        Some("fixture-private-key".to_owned()),
    );
    engine.request_scene_for_test(
        &"d".repeat(64),
        scene_still(),
        "a character rolls dice",
        "current-story-plate",
        true,
    );
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(fake.count("POST "), 0);
    assert_eq!(engine.scene_stage_of(&"d".repeat(64)), None);
}

#[tokio::test]
async fn keyless_scene_clips_fail_without_provider_traffic() {
    let fake = fake(Scenario::Refused);
    let engine = scene_test_engine(
        scratch("scene-keyless"),
        endpoints(&fake),
        Duration::from_secs(30),
        None,
    );
    let plate_key = "e".repeat(64);
    engine.request_scene_for_test(
        &plate_key,
        scene_still(),
        "the adventurers wait by the hearth",
        "current-story-plate",
        false,
    );
    let stage = settle_scene(&engine, &plate_key).await;
    assert!(stage.contains("no FAL_KEY"), "{stage}");
    assert_eq!(fake.count("POST "), 0, "a keyless engine never submits");
}

#[test]
fn the_setting_is_on_with_a_key_and_the_default_budget_and_model_and_off_without_one() {
    let current = Path::new("/work");
    let on = home_video::home_video_settings(
        inputs(None, Some("fixture-key"), None, None),
        current,
        false,
    )
    .expect("valid settings");
    assert!(
        home_video::startup_line(&on).contains("SCENE VIDEO on"),
        "scene clips are enabled by default when the media provider is configured"
    );
    assert_eq!(
        home_video::startup_line(&on),
        "HOME VIDEO on model=fal-ai/kling-video/o1/image-to-video budget=4 duration=5s; SCENE VIDEO on model=fal-ai/kling-video/o1/image-to-video duration=5s"
    );
    let without = home_video::home_video_settings(inputs(None, None, None, None), current, false)
        .expect("a missing key is a state");
    assert_eq!(
        home_video::startup_line(&without),
        "HOME VIDEO off (no FAL_KEY); SCENE VIDEO off (no FAL_KEY)"
    );
    let disabled = home_video::home_video_settings(
        inputs(Some("off"), Some("fixture-key"), None, None),
        current,
        false,
    )
    .expect("off is a state");
    assert_eq!(
        home_video::startup_line(&disabled),
        "HOME VIDEO off (DF_GAMEPLAY_HOME_VIDEO=off); SCENE VIDEO on model=fal-ai/kling-video/o1/image-to-video duration=5s"
    );
    let qualification = home_video::home_video_settings(
        inputs(None, Some("fixture-key"), None, None),
        current,
        true,
    )
    .expect("a qualification run is a state");
    assert_eq!(
        home_video::startup_line(&qualification),
        "HOME VIDEO off (qualification run); SCENE VIDEO off (qualification run)"
    );
}

#[test]
fn an_invalid_setting_is_refused_by_name_and_its_value_is_never_echoed() {
    let current = Path::new("/work");
    let budget = home_video::home_video_settings(
        inputs(None, Some("fixture-key"), Some("lots"), None),
        current,
        false,
    )
    .expect_err("a non-number budget is refused");
    assert!(
        budget.to_string().contains("DF_GAMEPLAY_VIDEO_BUDGET"),
        "{budget}"
    );
    assert!(
        !budget.to_string().contains("lots"),
        "the value is not echoed"
    );
    let model = home_video::home_video_settings(
        inputs(None, Some("fixture-key"), None, Some("some/other-model")),
        current,
        false,
    )
    .expect_err("a model the adapter cannot request is refused");
    assert!(
        model.to_string().contains("DF_GAMEPLAY_HOME_VIDEO_MODEL"),
        "{model}"
    );
    let over = home_video::home_video_settings(
        inputs(None, Some("fixture-key"), Some("1001"), None),
        current,
        false,
    )
    .expect_err("a budget over the maximum is refused");
    assert!(
        over.to_string().contains("DF_GAMEPLAY_VIDEO_BUDGET"),
        "{over}"
    );
    let mode = home_video::home_video_settings(
        inputs(Some("maybe"), Some("fixture-key"), None, None),
        current,
        false,
    )
    .expect_err("an unknown mode is refused");
    assert!(
        mode.to_string().contains("DF_GAMEPLAY_HOME_VIDEO"),
        "{mode}"
    );
}

#[test]
fn the_view_carries_the_loop_state_and_its_served_path_only_when_ready() {
    use super::rpc::HomeVideoState;
    assert_eq!(LoopView::None.proto_state(), HomeVideoState::None);
    assert_eq!(LoopView::Pending.proto_state(), HomeVideoState::Pending);
    assert_eq!(LoopView::Failed.proto_state(), HomeVideoState::Failed);
    let ready = LoopView::Ready {
        path: "/gameplay/home-video/abc".to_owned(),
    };
    assert_eq!(ready.proto_state(), HomeVideoState::Ready);
    assert_eq!(ready.path(), "/gameplay/home-video/abc");
    assert_eq!(
        LoopView::Pending.path(),
        "",
        "no path is sent before the loop is ready"
    );
}

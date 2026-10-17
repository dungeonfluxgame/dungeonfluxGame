//! Native checks of the home video: the cache key, the byte ranges the route serves, and the flight
//! against a loopback fal fake. The fake counts what the adapter sends, so "one submit per
//! painting", "a clip is charged when it is sent" and "a cached loop costs nothing" are
//! checked against the traffic itself, not against the engine's own bookkeeping.
use super::home_video::{
    self, Endpoints, LoopView, Span, clip_key, respond, scene_clip_key, scene_spec_for_test,
    scene_test_engine, span_for, test_engine,
};
use super::resources::{ResourceKey, ResourceKind, ResourceSpec};
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

const PAINTING: &str = "assets/ui/generated/scene-salt-quay-arrival-documentary-photo";
const OTHER: &str = "assets/ui/generated/scene-salt-quay-candid-documentary-photo";

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
    /// The submit is refused over HTTP with a validation body, as fal answers a bad field.
    Rejected,
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
    let rejected = scenario == Scenario::Rejected && request_line.starts_with("POST ");
    let (content_type, payload): (&str, Vec<u8>) = if rejected {
        (
            "application/json",
            br#"{"detail":[{"msg":"end_image_url is not accepted for this model","loc":["body","end_image_url"],"input":"fixture-private-key"}]}"#
                .to_vec(),
        )
    } else if path.starts_with("/v1/models/pricing") {
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
            Scenario::MalformedClip | Scenario::Rejected => r#"{"status":"COMPLETED"}"#,
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
    let status = if rejected {
        "422 Unprocessable Entity"
    } else {
        "200 OK"
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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
fn a_clip_key_changes_with_the_still_the_prompt_the_model_the_duration_the_size_and_the_recipe() {
    let plate = b"a plate";
    let key = |plate: &[u8], prompt: &str, edit: fn(&mut home_video::clip_spec::ClipRecipe)| {
        let spec = scene_spec_for_test("plate", plate, prompt, edit);
        (
            home_video::clip_spec::file_key(&spec),
            ResourceKey::of(&spec),
        )
    };
    let (base, base_key) = key(plate, "prompt", |_| {});
    assert_eq!(base.len(), 64, "a SHA-256 in lowercase hex");
    assert!(
        base.starts_with(base_key.as_str()),
        "the file name and the hub key are one identity"
    );
    assert_eq!(base, key(plate, "prompt", |_| {}).0);
    for (name, changed) in [
        ("still", key(b"another plate", "prompt", |_| {})),
        ("prompt", key(plate, "prompt!", |_| {})),
        (
            "model",
            key(plate, "prompt", |recipe| {
                recipe.model = "model-b".to_owned()
            }),
        ),
        (
            "duration",
            key(plate, "prompt", |recipe| recipe.duration_seconds += 1),
        ),
        ("width", key(plate, "prompt", |recipe| recipe.width += 2)),
        ("height", key(plate, "prompt", |recipe| recipe.height += 2)),
        (
            "recipe",
            key(plate, "prompt", |recipe| {
                recipe.recipe = "df-scene-video/3".to_owned()
            }),
        ),
        (
            "finish",
            key(plate, "prompt", |recipe| recipe.finish = "tail_crossfade"),
        ),
    ] {
        assert_ne!(base, changed.0, "a changed {name} must be a new file");
        assert_ne!(
            base_key, changed.1,
            "a changed {name} must be a new hub entry"
        );
    }
    let spec = scene_spec_for_test("plate", plate, "prompt", |_| {});
    assert_eq!(
        <home_video::clip_spec::SceneLoopSpec as ResourceSpec>::KIND,
        ResourceKind::VideoClip
    );
    assert!(
        spec.canonical()
            .contains("\"prompt_variant\":\"scene_ambient\"")
    );
    assert!(spec.canonical().contains("first_and_last_frame_same_still"));
    assert_eq!(clip_key(b"a still").len(), 64);
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
    assert_eq!(engine.clips_charged(), 0, "nor any charge");
    let _ = std::fs::remove_dir_all(&directory);
}

#[tokio::test]
async fn one_submit_serves_every_display_that_asks_for_the_same_painting() {
    let fake = fake(Scenario::MalformedClip);
    let directory = scratch("one-submit");
    let engine = test_engine(
        asset_root(),
        directory.clone(),
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
    assert_eq!(engine.clips_charged(), 1, "one clip charged");
    let _ = std::fs::remove_dir_all(&directory);
}

#[tokio::test]
async fn every_clip_that_is_sent_is_charged_and_nothing_is_ever_refused_for_cost() {
    let fake = fake(Scenario::MalformedClip);
    let directory = scratch("no-cap");
    let engine = test_engine(
        asset_root(),
        directory.clone(),
        endpoints(&fake),
        Duration::from_secs(30),
    );
    engine.observe_for_test(PAINTING);
    settle(&engine, PAINTING).await;
    assert_eq!(engine.clips_charged(), 1);
    engine.observe_for_test(OTHER);
    let second = settle(&engine, OTHER).await;
    assert!(
        !second.contains("capped") && !second.contains("budget"),
        "no spend limit exists to refuse it: {second}"
    );
    assert_eq!(fake.count("POST"), 2, "both paintings reached the provider");
    assert_eq!(engine.clips_charged(), 2);
    let _ = std::fs::remove_dir_all(&directory);
}

#[tokio::test]
async fn a_refusal_or_failure_from_the_provider_leaves_the_still_with_a_diagnostic() {
    let fake = fake(Scenario::Refused);
    let directory = scratch("refused");
    let engine = test_engine(
        asset_root(),
        directory.clone(),
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
        endpoints(&fake),
        Duration::from_millis(900),
    );
    engine.observe_for_test(PAINTING);
    let stage = settle(&engine, PAINTING).await;
    assert_eq!(
        stage, "failed:timed_out",
        "a clip that never finishes times out"
    );
    assert_eq!(
        engine.clips_charged(),
        1,
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
                layout_json: String::new(),
                layout_revision: 0,
                place_music: None,
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

#[test]
fn the_lobby_shows_the_generic_landing_plate_whatever_art_the_adventure_names() {
    // Arrange: a lobby view whose adventure names the live room photograph token, and a scene view.
    let view = |phase: rpc::JourneyPhase| rpc::ViewMessage {
        audience: Some(rpc::view_message::Audience::Display(
            rpc::DisplayGameplayView {
                scene_asset: "live-room-photo".to_owned(),
                journey: Some(rpc::JourneyView {
                    phase: phase as i32,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    let asset = |message: rpc::ViewMessage| match message.audience {
        Some(rpc::view_message::Audience::Display(display)) => display.scene_asset,
        _ => panic!("a display view"),
    };
    // Act
    let mut lobby = view(rpc::JourneyPhase::Room);
    home_video::apply(&mut lobby);
    let mut opening = view(rpc::JourneyPhase::Opening);
    home_video::apply(&mut opening);
    // Assert: the landing draws (and loops) the one reviewed plate; a scene keeps its own art.
    assert_eq!(asset(lobby), PAINTING);
    assert_eq!(asset(opening), "live-room-photo");
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
    model: Option<&str>,
) -> home_video::HomeVideoInputs {
    inputs_with_scene(mode, None, key, model)
}

fn inputs_with_scene(
    mode: Option<&str>,
    scene_mode: Option<&str>,
    key: Option<&str>,
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
        cache: Err(std::env::VarError::NotPresent),
        credential: value(key),
    }
}

#[test]
fn the_scene_video_switch_defaults_from_the_provider_and_can_be_explicitly_disabled_or_enabled() {
    let current = Path::new("/work");
    let off = home_video::home_video_settings(
        inputs_with_scene(None, Some("off"), Some("fixture-key"), None),
        current,
        false,
    )
    .expect("scene clips can be disabled");
    assert!(
        home_video::startup_line(&off).contains("SCENE VIDEO off (DF_GAMEPLAY_SCENE_VIDEO=off)")
    );

    let keyless = home_video::home_video_settings(
        inputs_with_scene(None, Some("on"), None, None),
        current,
        false,
    )
    .expect("explicit scene mode can report the unavailable provider");
    assert_eq!(
        home_video::startup_line(&keyless),
        "HOME VIDEO off (no FAL_KEY); SCENE VIDEO on model=fal-ai/kling-video/o1/image-to-video duration=5s (no FAL_KEY; request logging only)"
    );

    let invalid = home_video::home_video_settings(
        inputs_with_scene(None, Some("maybe"), Some("fixture-key"), None),
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
    assert_eq!(
        engine.clips_charged(),
        1,
        "a scene clip is a paid call and is charged like a home loop"
    );

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
async fn a_cached_scene_clip_is_ready_with_no_provider_call_and_an_edited_plate_gets_its_own() {
    let fake = fake(Scenario::Refused);
    let directory = scratch("scene-cache");
    let description = "a dice game beside the fire";
    let cached_key = scene_clip_key(&scene_still(), description).expect("a key");
    std::fs::write(directory.join(format!("{cached_key}.mp4")), vec![0u8; 64])
        .expect("cached scene clip");
    let engine = scene_test_engine(
        directory.clone(),
        endpoints(&fake),
        Duration::from_secs(30),
        Some("fixture-private-key".to_owned()),
    );
    let plate_key = "b".repeat(64);
    engine.request_scene_for_test(
        &plate_key,
        scene_still(),
        description,
        "current-story-plate",
        false,
    );
    assert_eq!(
        settle_scene(&engine, &plate_key).await,
        format!("ready:{cached_key}")
    );
    assert_eq!(
        engine.scene_view_for_test(&plate_key),
        home_video::SceneClipView::Ready {
            path: format!("/gameplay/home-video/{cached_key}")
        }
    );
    assert_eq!(
        fake.count("POST "),
        0,
        "a cached clip makes no provider call"
    );
    assert_eq!(engine.clips_charged(), 0, "and costs nothing");

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
async fn a_changed_description_for_the_same_plate_is_a_new_clip_not_the_old_loop() {
    let fake = fake(Scenario::Refused);
    let directory = scratch("scene-changed-prompt");
    let engine = scene_test_engine(
        directory.clone(),
        endpoints(&fake),
        Duration::from_secs(30),
        Some("fixture-private-key".to_owned()),
    );
    let plate_key = "9".repeat(64);
    let old_key = scene_clip_key(&scene_still(), "the innkeeper rolls dice").expect("a key");
    std::fs::write(directory.join(format!("{old_key}.mp4")), vec![0u8; 64]).expect("an old loop");

    engine.request_scene_for_test(
        &plate_key,
        scene_still(),
        "the innkeeper rolls dice",
        "current-story-plate",
        false,
    );
    assert_eq!(
        settle_scene(&engine, &plate_key).await,
        format!("ready:{old_key}")
    );
    engine.request_scene_for_test(
        &plate_key,
        scene_still(),
        "the innkeeper pours ale",
        "current-story-plate",
        false,
    );
    let stage = settle_scene(&engine, &plate_key).await;

    assert!(
        stage.starts_with("failed:"),
        "the new description is asked for, not served the old loop: {stage}"
    );
    assert_eq!(fake.count("POST "), 1);
    assert_eq!(engine.clips_charged(), 1, "and the new clip is charged");
}

#[tokio::test]
async fn each_scene_clip_is_charged_on_the_ledger_with_a_spend_line() {
    let fake = fake(Scenario::Refused);
    let engine = scene_test_engine(
        scratch("scene-charge"),
        endpoints(&fake),
        Duration::from_secs(30),
        Some("fixture-private-key".to_owned()),
    );
    let first = "7".repeat(64);
    let second = "8".repeat(64);
    engine.request_scene_for_test(&first, scene_still(), "a quiet hearth", "plate", false);
    settle_scene(&engine, &first).await;
    assert_eq!(engine.clips_charged(), 1);

    engine.request_scene_for_test(&second, scene_still(), "a busy bar", "plate", false);
    settle_scene(&engine, &second).await;

    assert_eq!(engine.clips_charged(), 2);
    assert_eq!(fake.count("POST "), 2);
    assert!(
        !crate::gameplay::actor::audit_lines_containing("SPEND commit unit=video_clip").is_empty(),
        "the ledger writes a line for every charge"
    );
}

#[tokio::test]
async fn a_failed_clip_is_not_asked_for_again_and_the_hub_lists_it_until_it_is_invalidated() {
    let fake = fake(Scenario::Refused);
    let engine = scene_test_engine(
        scratch("scene-failed-memory"),
        endpoints(&fake),
        Duration::from_secs(30),
        Some("fixture-private-key".to_owned()),
    );
    let plate_key = "6".repeat(64);
    for _ in 0..3 {
        engine.request_scene_for_test(&plate_key, scene_still(), "a still room", "plate", false);
        settle_scene(&engine, &plate_key).await;
    }
    assert_eq!(
        fake.count("POST "),
        1,
        "a failure is not retried beyond its attempts"
    );

    let clip = scene_clip_key(&scene_still(), "a still room").expect("a key");
    let lines = crate::gameplay::actor::audit_lines_containing(&clip[..32]);
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("RESOURCE request kind=video_clip key=")),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(
            |line| line.starts_with("RESOURCE failed kind=video_clip key=")
                && line.contains("error=provider: refused or failed")
        ),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(
            |line| line.starts_with("RESOURCE failed_detail kind=video_clip key=")
                && line.contains("RunnerError")
        ),
        "the supplier's wording is in the log: {lines:?}"
    );

    let kit = engine.kit_for_test();
    let rows = kit.hub().list(Some(ResourceKind::VideoClip));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, "failed");
    assert!(
        rows[0].subject.starts_with("video:scene:"),
        "{}",
        rows[0].subject
    );
    assert_eq!(
        kit.invalidate(super::resources::Invalidate::Key(&rows[0].key)),
        1
    );

    engine.request_scene_for_test(&plate_key, scene_still(), "a still room", "plate", false);
    settle_scene(&engine, &plate_key).await;
    assert_eq!(
        fake.count("POST "),
        2,
        "invalidating the key lets it be asked for again"
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
fn the_setting_is_on_with_a_key_and_the_default_model_and_off_without_one() {
    let current = Path::new("/work");
    let on =
        home_video::home_video_settings(inputs(None, Some("fixture-key"), None), current, false)
            .expect("valid settings");
    assert!(
        home_video::startup_line(&on).contains("SCENE VIDEO on"),
        "scene clips are enabled by default when the media provider is configured"
    );
    assert_eq!(
        home_video::startup_line(&on),
        "HOME VIDEO on model=fal-ai/kling-video/o1/image-to-video duration=5s; SCENE VIDEO on model=fal-ai/kling-video/o1/image-to-video duration=5s"
    );
    let without = home_video::home_video_settings(inputs(None, None, None), current, false)
        .expect("a missing key is a state");
    assert_eq!(
        home_video::startup_line(&without),
        "HOME VIDEO off (no FAL_KEY); SCENE VIDEO off (no FAL_KEY)"
    );
    let disabled = home_video::home_video_settings(
        inputs(Some("off"), Some("fixture-key"), None),
        current,
        false,
    )
    .expect("off is a state");
    assert_eq!(
        home_video::startup_line(&disabled),
        "HOME VIDEO off (DF_GAMEPLAY_HOME_VIDEO=off); SCENE VIDEO on model=fal-ai/kling-video/o1/image-to-video duration=5s"
    );
    let qualification =
        home_video::home_video_settings(inputs(None, Some("fixture-key"), None), current, true)
            .expect("a qualification run is a state");
    assert_eq!(
        home_video::startup_line(&qualification),
        "HOME VIDEO off (qualification run); SCENE VIDEO off (qualification run)"
    );
}

#[test]
fn an_invalid_setting_is_refused_by_name_and_its_value_is_never_echoed() {
    let current = Path::new("/work");
    let model = home_video::home_video_settings(
        inputs(None, Some("fixture-key"), Some("some/other-model")),
        current,
        false,
    )
    .expect_err("a model the adapter cannot request is refused");
    assert!(
        model.to_string().contains("DF_GAMEPLAY_HOME_VIDEO_MODEL"),
        "{model}"
    );
    let mode = home_video::home_video_settings(
        inputs(Some("maybe"), Some("fixture-key"), None),
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

#[tokio::test]
async fn a_submit_the_provider_rejects_logs_its_http_status_and_redacted_body() {
    let fake = fake(Scenario::Rejected);
    let engine = scene_test_engine(
        scratch("scene-rejected"),
        endpoints(&fake),
        Duration::from_secs(30),
        Some("fixture-private-key".to_owned()),
    );
    let plate_key = "5".repeat(64);
    engine.request_scene_for_test(&plate_key, scene_still(), "a rejected room", "plate", false);

    let stage = settle_scene(&engine, &plate_key).await;

    assert_eq!(stage, "failed:provider: call failed");
    let clip = scene_clip_key(&scene_still(), "a rejected room").expect("a key");
    let lines = crate::gameplay::actor::audit_lines_containing(&clip[..32]);
    let detail = lines
        .iter()
        .find(|line| line.starts_with("RESOURCE failed_detail kind=video_clip key="))
        .unwrap_or_else(|| panic!("no failed_detail line in {lines:?}"));
    assert!(detail.contains("http_status=422"), "{detail}");
    assert!(detail.contains("end_image_url is not accepted"), "{detail}");
    assert!(!detail.contains("fixture-private-key"), "{detail}");
    assert_eq!(
        engine.clips_charged(),
        1,
        "a submit that was sent is charged"
    );
}

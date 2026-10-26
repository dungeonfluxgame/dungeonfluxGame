//! Spec, prompt, mastering, generator and hub-adapter tests: everything that needs no pack, no
//! Jev and no view. A fake ElevenLabs stands in for the provider; no paid call is ever made.

use std::{
    collections::HashSet,
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use df_providers::{ElevenMusicConfig, NativeElevenMusicProvider};
use serde_json::json;

use super::PackMusic;
use super::generator::{MusicGenerator, MusicSetSpec, PROVIDER_CALLS};
use super::hub::{MusicHub as _, MusicStatus};
use super::master::{
    self, Codec, Ffmpeg, frames_of, integrated_lufs, loop_cut, normalise, tone, tone_bytes,
};
use super::resource::HubMusic;
use super::spec::{Closed, Energy, Layer, Mood, Palette, Space, TimeOfDay};
use super::test_support::shop_block;

const FAKE_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

// ---- spec and prompts ----

#[test]
fn closed_labels_match_their_serialized_names_and_the_pack_rejects_unknown_ones() {
    fn names<T: Closed + serde::Serialize>() {
        for value in T::ALL {
            assert_eq!(serde_json::to_value(value).unwrap(), value.label());
        }
    }
    names::<Mood>();
    names::<Energy>();
    names::<TimeOfDay>();
    names::<Space>();
    names::<Palette>();
    let mut block = json!({
        "place_id": "p", "mood": "relaxed", "energy": "low", "setting": "market",
        "time_of_day": "day", "space": "interior", "palette": "light-acoustic"
    });
    assert!(
        serde_json::from_value::<PackMusic>(block.clone())
            .unwrap()
            .spec()
            .is_ok()
    );
    block["mood"] = json!("a calm afternoon");
    assert!(serde_json::from_value::<PackMusic>(block.clone()).is_err());
    block["mood"] = json!("relaxed");
    block["extra"] = json!(1);
    assert!(serde_json::from_value::<PackMusic>(block).is_err());
}

#[test]
fn the_four_layers_share_one_tempo_one_key_one_chord_movement_and_one_loop_length() {
    for mood in Mood::ALL {
        let mut block = shop_block();
        block.mood = *mood;
        let spec = block.spec().unwrap();
        let prompts: Vec<String> = Layer::ALL
            .iter()
            .map(|layer| spec.layer_prompt(*layer))
            .collect();
        for prompt in &prompts {
            assert!(prompt.contains("Exactly 84 BPM"), "{prompt}");
            assert!(prompt.contains("16 bars"));
            assert!(prompt.contains(spec.key_name()), "{mood:?}: {prompt}");
            assert!(prompt.contains("no intro") && prompt.contains("loop seamlessly"));
        }
        let headers: HashSet<&str> = prompts
            .iter()
            .map(|prompt| prompt.split("Starts immediately").next().unwrap())
            .collect();
        assert_eq!(
            headers.len(),
            1,
            "the layers differ only after the shared header"
        );
    }
    let shop = shop_block().spec().unwrap();
    assert!(shop.layer_prompt(Layer::Bed).contains("G Em C D"));
    assert!(shop.layer_prompt(Layer::Bed).contains("lute, recorder"));
    assert!(shop.ambience_prompt().contains("cloth shop"));
    assert!(shop.ambience_prompt().contains("no music"));
}

#[test]
fn the_set_key_is_stable_for_a_spec_and_changes_with_anything_that_shapes_the_audio() {
    let base = shop_block();
    let key = |block: &PackMusic| MusicSetSpec::new(block.spec().unwrap()).key();
    assert_eq!(key(&base), key(&base.clone()));
    assert_eq!(key(&base).len(), 32);
    let mut bed = base.clone();
    bed.fallback_bed = Some("none".into());
    assert_eq!(key(&bed), key(&base), "the fallback bed makes no audio");
    for change in [
        |b: &mut PackMusic| b.mood = Mood::Ominous,
        |b: &mut PackMusic| b.energy = Energy::High,
        |b: &mut PackMusic| b.time_of_day = TimeOfDay::Night,
        |b: &mut PackMusic| b.space = Space::Exterior,
        |b: &mut PackMusic| b.palette = Palette::Folk,
        |b: &mut PackMusic| b.instruments = None,
        |b: &mut PackMusic| b.ambience = None,
        |b: &mut PackMusic| b.setting = "harbour".into(),
    ] {
        let mut changed = base.clone();
        change(&mut changed);
        assert_ne!(key(&changed), key(&base));
    }
}

// ---- mastering ----

#[test]
fn a_loop_cut_is_exactly_the_loop_and_runs_on_into_its_start() {
    let loop_frames = 20_000;
    let source = tone(loop_frames + 3_000, 311.0, 0.5);
    let looped = loop_cut(&source, loop_frames, 2_000).unwrap();
    assert_eq!(looped.len(), loop_frames);
    // The first frame is the audio that followed the cut, so the wrap is as smooth as the tone.
    let step = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs();
    let largest_step = source
        .windows(2)
        .map(|pair| step(pair[0], pair[1]))
        .fold(0.0, f32::max);
    assert!(step(looped[loop_frames - 1], looped[0]) <= largest_step * 1.5);
    // Past the crossfade the audio is untouched.
    assert_eq!(looped[2_500], source[2_500]);
    assert_eq!(
        master::loop_cut(&source[..loop_frames - 1], loop_frames, 2_000),
        Err(master::MasterFailure::TooShort)
    );
    assert_eq!(
        loop_cut(&source[..loop_frames + 100], loop_frames, 2_000),
        Err(master::MasterFailure::TooShort)
    );
}

#[test]
fn loudness_is_measured_to_the_standard_and_set_to_the_shipped_level() {
    // EBU Tech 3341: a stereo 1 kHz sine at -23 dBFS reads -23.0 LUFS.
    let reference = tone(44_100 * 6, 1_000.0, 10f32.powf(-23.0 / 20.0));
    let measured = integrated_lufs(&reference).unwrap();
    assert!((measured + 23.0).abs() < 0.15, "{measured}");
    let mut loud = tone(44_100 * 6, 220.0, 0.5);
    let done = normalise(&mut loud, -23.4).unwrap();
    assert!(done.reached_target);
    assert!((integrated_lufs(&loud).unwrap() + 23.4).abs() < 0.1);
    // The peak ceiling stops a gain that would clip.
    let mut hot = tone(44_100 * 6, 220.0, 0.9);
    let limited = normalise(&mut hot, 5.0).unwrap();
    assert!(!limited.reached_target);
    assert!(hot.iter().flatten().all(|sample| sample.abs() <= 0.8913));
    assert_eq!(
        normalise(&mut vec![[0.0; 2]; 44_100 * 2], -23.4),
        Err(master::MasterFailure::Silent)
    );
}

#[test]
fn ffmpeg_decodes_and_encodes_when_it_is_installed() {
    let binary = std::env::var("DF_FFMPEG").map_or_else(|_| PathBuf::from("ffmpeg"), PathBuf::from);
    if !Ffmpeg::available(&binary) {
        eprintln!("skipped: ffmpeg is not installed");
        return;
    }
    let scratch = std::env::temp_dir().join(format!("df-music-test-{}", std::process::id()));
    let codec = Codec::Ffmpeg(Ffmpeg::new(binary, scratch.clone()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let frames = tone(44_100 * 3, 440.0, 0.2);
    let mp3 = runtime
        .block_on(codec.encode(frames.clone(), Duration::from_secs(30)))
        .unwrap();
    assert!(mp3.len() > 1_000);
    let decoded = runtime
        .block_on(codec.decode(mp3, Duration::from_secs(30)))
        .unwrap();
    // MP3 pads the ends; the length is the length to within a frame or two of codec delay.
    assert!(
        decoded.len().abs_diff(frames.len()) < 4_500,
        "{}",
        decoded.len()
    );
    let _removed = std::fs::remove_dir_all(scratch);
}

// ---- generator against a fake ElevenLabs ----

struct FakeEleven {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
    peak: Arc<AtomicUsize>,
}

impl FakeEleven {
    /// `fail` names a path and a prompt fragment that get this status instead of audio.
    fn start(delay: Duration, fail: Option<(&'static str, &'static str, u16)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let current = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (log, running, highest) = (requests.clone(), current.clone(), peak.clone());
        thread::spawn(move || {
            for socket in listener.incoming() {
                let Ok(mut socket) = socket else { break };
                let (log, running, highest) = (log.clone(), running.clone(), highest.clone());
                thread::spawn(move || {
                    socket.set_read_timeout(Some(FAKE_REQUEST_TIMEOUT)).unwrap();
                    let mut data = Vec::new();
                    let end = loop {
                        let mut block = [0; 2048];
                        let count = socket.read(&mut block).unwrap();
                        data.extend_from_slice(&block[..count]);
                        if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                            break end + 4;
                        }
                    };
                    let head = String::from_utf8(data[..end].to_vec()).unwrap();
                    let length: usize = head
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap())
                        })
                        .unwrap();
                    while data.len() - end < length {
                        let mut block = [0; 2048];
                        let count = socket.read(&mut block).unwrap();
                        data.extend_from_slice(&block[..count]);
                    }
                    let body: serde_json::Value = serde_json::from_slice(&data[end..]).unwrap();
                    let path = head
                        .split_whitespace()
                        .nth(1)
                        .unwrap()
                        .split('?')
                        .next()
                        .unwrap()
                        .to_owned();
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    highest.fetch_max(now, Ordering::SeqCst);
                    thread::sleep(delay);
                    let prompt = body["prompt"]
                        .as_str()
                        .or(body["text"].as_str())
                        .unwrap_or("")
                        .to_owned();
                    log.lock().unwrap().push((path.clone(), body));
                    let failing =
                        fail.filter(|(p, fragment, _)| path == *p && prompt.contains(fragment));
                    running.fetch_sub(1, Ordering::SeqCst);
                    if let Some((_, _, status)) = failing {
                        let _ = write!(
                            socket,
                            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                        );
                        return;
                    }
                    // 48 s of source for a layer, 27 s for the ambience: the loop plus its tail.
                    let frames = if path == "/v1/music" {
                        48 * 44_100
                    } else {
                        27 * 44_100
                    };
                    let audio = tone_bytes(frames, 0.3);
                    let _ = write!(
                        socket,
                        "HTTP/1.1 200 X\r\nContent-Type: audio/mpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        audio.len()
                    );
                    let _ = socket.write_all(&audio);
                });
            }
        });
        Self {
            address,
            requests,
            peak,
        }
    }

    fn generator(&self) -> MusicGenerator {
        MusicGenerator::new(
            NativeElevenMusicProvider::new(
                ElevenMusicConfig::loopback_fixture(self.address).unwrap(),
                "test-key",
                Duration::from_secs(20),
            )
            .unwrap(),
            Codec::Raw,
            // A generator run on its own records on a ledger of its own; `HubMusic::for_tests`
            // rebinds it to the hub's.
            Arc::clone(Hub::new().ledger()),
        )
    }
}

fn shop_set() -> MusicSetSpec {
    MusicSetSpec::new(shop_block().spec().unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_set_is_four_aligned_music_calls_and_one_ambience_call_mastered_to_the_shipped_loop() {
    let fake = FakeEleven::start(Duration::from_millis(30), None);
    let set = fake
        .generator()
        .generate(&shop_set(), Duration::from_secs(60))
        .await
        .unwrap();
    let requests = fake.requests.lock().unwrap();
    let music: Vec<_> = requests
        .iter()
        .filter(|(path, _)| path == "/v1/music")
        .collect();
    let sound: Vec<_> = requests
        .iter()
        .filter(|(path, _)| path == "/v1/sound-generation")
        .collect();
    assert_eq!(
        (music.len(), sound.len()),
        (4, 1),
        "exactly {PROVIDER_CALLS} calls"
    );
    assert_eq!(music.len() + sound.len(), PROVIDER_CALLS as usize);
    assert!(
        fake.peak.load(Ordering::SeqCst) <= 2,
        "bounded concurrency within a set"
    );
    for (_, body) in &music {
        assert_eq!(body["music_length_ms"], 48_000);
        assert_eq!(body["force_instrumental"], true);
        assert!(
            body["prompt"]
                .as_str()
                .unwrap()
                .contains("Exactly 84 BPM in G major")
        );
    }
    assert_eq!(sound[0].1["loop"], true);

    // All four layers: the loop length of the shared score to the sample, at its loudness.
    for layer in &set.layers {
        let frames = frames_of(layer);
        assert_eq!(frames.len(), crate::music_score::LOOP_SAMPLES as usize);
        assert!(
            (integrated_lufs(&frames).unwrap() - f64::from(crate::music_score::LAYER_LUFS)).abs()
                < 0.2
        );
    }
    let ambience = frames_of(set.ambience.as_ref().unwrap());
    assert_eq!(ambience.len(), 24 * 44_100);
    assert!((integrated_lufs(&ambience).unwrap() + 30.0).abs() < 0.2);
    assert_eq!(set.manifest["provider_calls"], 5);
    assert_eq!(set.manifest["loop_samples"], 2_016_000);
    assert_eq!(set.manifest["layers"].as_array().unwrap().len(), 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_failed_music_layer_fails_the_set_with_its_stage_and_a_closed_reason() {
    let fake = FakeEleven::start(Duration::ZERO, Some(("/v1/music", "Rising unease", 500)));
    let failure = match fake
        .generator()
        .generate(&shop_set(), Duration::from_secs(60))
        .await
    {
        Ok(_) => panic!("a set with a failed layer must not be returned"),
        Err(failure) => failure,
    };
    assert_eq!(failure.stage, "tension");
    assert_eq!(failure.reason.label(), "provider_error");
    assert_eq!(failure.reason.detail(), "provider_status_500");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_ambience_leaves_the_set_without_one() {
    let fake = FakeEleven::start(Duration::ZERO, Some(("/v1/sound-generation", "", 429)));
    let set = fake
        .generator()
        .generate(&shop_set(), Duration::from_secs(60))
        .await
        .unwrap();
    assert!(set.ambience.is_none());
    assert_eq!(set.manifest["ambience"]["failed"], "provider_status_429");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_deadline_bounds_the_whole_set() {
    let fake = FakeEleven::start(Duration::from_millis(600), None);
    let started = std::time::Instant::now();
    let failure = match fake
        .generator()
        .generate(&shop_set(), Duration::from_millis(150))
        .await
    {
        Ok(_) => panic!("the deadline must end the set"),
        Err(failure) => failure,
    };
    // Whichever bound fires first (a call's remaining time or the set's), it is a deadline and
    // it ends the set near the deadline, not after the slow provider answers.
    assert_eq!(failure.reason.label(), "deadline", "{failure}");
    assert!(
        started.elapsed() < Duration::from_millis(450),
        "{:?}",
        started.elapsed()
    );
}

// ---- the real hub adapter, over a fake ElevenLabs ----

use crate::gameplay::resources::{Hub, ResourceKind};

fn store_directory(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("df-music-store-{}-{name}", std::process::id()));
    let _cleared = std::fs::remove_dir_all(&directory);
    directory
}

async fn until_status(music: &HubMusic, set: &MusicSetSpec, wanted: &str) -> MusicStatus {
    use crate::gameplay::place_music::hub::MusicHub as _;
    for _ in 0..600 {
        let status = music.status(set);
        if status.label() == wanted {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    music.status(set)
}

fn provider_calls(fake: &FakeEleven) -> usize {
    fake.requests.lock().unwrap().len()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ambient_layers_are_separate_hub_resources_cached_and_reused_across_pacing() {
    use super::ambience::{AmbientSpec, AmbientStatus};
    use crate::music_score::AmbientKind;
    let fake = FakeEleven::start(Duration::ZERO, None);
    let hub = Hub::new();
    let directory = store_directory("hs36-ambient");
    let music = HubMusic::for_tests(hub.clone(), directory.clone(), fake.generator());
    music.start_worker(&tokio::runtime::Handle::current(), || {});
    let spec = AmbientSpec {
        place: "quay".into(),
        kind: AmbientKind::Water,
        facts: "public harbour water".into(),
        activity_permille: 550,
    };
    for _ in 0..5 {
        music.ambient_request(&spec);
    }
    let mut ready = None;
    for _ in 0..600 {
        if let AmbientStatus::Ready(path) = music.ambient_status(&spec) {
            ready = Some(path);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        ready,
        Some(format!("/gameplay/music/{}/ambient", spec.key()))
    );
    assert_eq!(provider_calls(&fake), 1);
    let bytes = std::fs::read(directory.join(spec.key()).join("ambient.mp3")).unwrap();
    assert_eq!(frames_of(&bytes).len(), 24 * 44_100);
    assert_eq!(hub.list(Some(ResourceKind::AmbientLayer)).len(), 1);
    let spend = hub
        .ledger()
        .snapshot(crate::gameplay::resources::Unit::MusicCall);
    assert_eq!(spend.committed, 1);
    assert_eq!(spend.reserved, 0);
    for _ in crate::music_score::PacingBand::ALL {
        music.ambient_request(&spec);
    }
    assert_eq!(
        provider_calls(&fake),
        1,
        "tempo bands must reuse the environmental loop"
    );
    drop(music);
    let next_hub = Hub::new();
    let next = HubMusic::for_tests(next_hub, directory, fake.generator());
    next.start_worker(&tokio::runtime::Handle::current(), || {});
    next.ambient_request(&spec);
    for _ in 0..200 {
        if matches!(next.ambient_status(&spec), AmbientStatus::Ready(_)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(matches!(
        next.ambient_status(&spec),
        AmbientStatus::Ready(_)
    ));
    assert_eq!(
        provider_calls(&fake),
        1,
        "a new hub process must read the disk cache"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pacing_variant_passes_the_original_theme_reference_to_every_fake_provider_layer() {
    use crate::music_score::PacingBand;
    let fake = FakeEleven::start(Duration::ZERO, None);
    let base = shop_set();
    let variant = MusicSetSpec::in_band(base.spec().clone(), PacingBand::Driving);
    let generated = fake
        .generator()
        .generate(&variant, Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(generated.manifest["band"], "driving");
    assert_eq!(generated.manifest["bpm"], 105);
    assert_eq!(generated.manifest["bars"], 20);
    assert_eq!(generated.manifest["energy_permille"], 650);
    assert_eq!(
        generated.manifest["motif_reference"],
        base.motif_reference()
    );
    for (path, body) in fake.requests.lock().unwrap().iter() {
        if path == "/v1/music" {
            let prompt = body["prompt"].as_str().unwrap();
            assert!(prompt.contains("Exactly 105 BPM in G major"));
            assert!(prompt.contains("20 bars"));
            assert!(prompt.contains(&base.motif_reference()));
            assert!(!prompt.contains("Exactly 84 BPM"));
        }
    }
    for layer in &generated.layers {
        assert_eq!(
            frames_of(layer).len(),
            crate::music_score::LOOP_SAMPLES as usize
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_set_is_requested_once_generated_once_cached_on_disk_and_served_by_key() {
    use crate::gameplay::place_music::hub::MusicHub as _;
    let fake = FakeEleven::start(Duration::from_millis(40), None);
    let root = store_directory("once");
    let hub = Hub::new();
    let music = HubMusic::for_tests(hub.clone(), root.clone(), fake.generator());
    music.start_worker(&tokio::runtime::Handle::current(), || {});
    let set = shop_set();

    // Many asks while it is generating start exactly one generation.
    let statuses: Vec<MusicStatus> = (0..10).map(|_| music.get_or_request(&set)).collect();
    assert!(
        statuses
            .iter()
            .all(|status| *status == MusicStatus::Pending)
    );
    let ready = until_status(&music, &set, "ready").await;
    assert_eq!(
        provider_calls(&fake),
        PROVIDER_CALLS as usize,
        "one set, five calls"
    );
    assert!(fake.peak.load(Ordering::SeqCst) <= 2);
    let MusicStatus::Ready(paths) = ready else {
        panic!("ready expected")
    };
    let key = set.key();
    assert_eq!(paths.layer_paths[0], format!("/gameplay/music/{key}/bed"));
    assert_eq!(
        paths.layer_paths[3],
        format!("/gameplay/music/{key}/combat")
    );
    assert_eq!(
        paths.ambience_path,
        Some(format!("/gameplay/music/{key}/ambience"))
    );

    // Asking again, even after the hub entry exists, costs nothing.
    assert!(matches!(music.get_or_request(&set), MusicStatus::Ready(_)));
    assert_eq!(provider_calls(&fake), PROVIDER_CALLS as usize);
    assert_eq!(hub.list(Some(ResourceKind::MusicSet)).len(), 1);

    // The route serves ready keys and nothing else.
    let mut router = music.router();
    async fn get(router: &mut axum::Router, path: String) -> axum::response::Response {
        use tonic::codegen::Service;
        let request = axum::http::Request::get(path)
            .body(axum::body::Body::empty())
            .unwrap();
        Service::call(router, request).await.unwrap()
    }
    // The test codec writes raw audio, far past the real tracks' size; a real MP3 is under 1 MB.
    let oversized = get(&mut router, format!("/gameplay/music/{key}/bed")).await;
    assert_eq!(
        oversized.status(),
        404,
        "a track past the size bound is never served"
    );
    std::fs::write(root.join(&key).join("bed.mp3"), b"MP3-bed-bytes").unwrap();
    let bed = get(&mut router, format!("/gameplay/music/{key}/bed")).await;
    assert_eq!(bed.status(), 200);
    assert_eq!(bed.headers()["content-type"], "audio/mpeg");
    let bytes = axum::body::to_bytes(bed.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&bytes[..], b"MP3-bed-bytes");
    assert_eq!(
        get(
            &mut router,
            format!("/gameplay/music/{}/bed", "0".repeat(32))
        )
        .await
        .status(),
        404
    );
    assert_eq!(
        get(&mut router, format!("/gameplay/music/{key}/secret"))
            .await
            .status(),
        404
    );
    assert_eq!(
        get(&mut router, format!("/gameplay/music/{key}/../bed"))
            .await
            .status(),
        404
    );

    // A new run (a new hub) finds the set on disk and pays nothing.
    let rerun = HubMusic::for_tests(Hub::new(), root.clone(), fake.generator());
    rerun.start_worker(&tokio::runtime::Handle::current(), || {});
    rerun.get_or_request(&set);
    assert_eq!(until_status(&rerun, &set, "ready").await.label(), "ready");
    assert_eq!(
        provider_calls(&fake),
        PROVIDER_CALLS as usize,
        "no second generation"
    );
    let _removed = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_set_is_not_retried_by_asking_again_and_regenerate_spends_again() {
    use crate::gameplay::place_music::hub::MusicHub as _;
    let fake = FakeEleven::start(Duration::ZERO, Some(("/v1/music", "Rising unease", 500)));
    let root = store_directory("failed");
    let hub = Hub::new();
    let music = HubMusic::for_tests(hub.clone(), root.clone(), fake.generator());
    music.start_worker(&tokio::runtime::Handle::current(), || {});
    let set = shop_set();
    music.get_or_request(&set);
    let failed = until_status(&music, &set, "failed").await;
    assert_eq!(
        failed,
        MusicStatus::Failed("tension: provider_status_500".into())
    );
    let calls = provider_calls(&fake);
    // Asking again does not spend again, and nothing partial is served or stored.
    music.get_or_request(&set);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(provider_calls(&fake), calls);
    assert!(std::fs::read_dir(&root).unwrap().next().is_none());
    // `regenerate` is the way to try again, and it is paid for.
    music.regenerate(&set);
    for _ in 0..100 {
        if provider_calls(&fake) > calls {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(provider_calls(&fake) > calls);
    let _removed = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn music_has_no_budget_and_the_ledger_records_every_set() {
    use crate::gameplay::resources::Unit;
    let fake = FakeEleven::start(Duration::ZERO, None);
    let root = store_directory("no-budget");
    let hub = Hub::new();
    let music = HubMusic::for_tests(hub.clone(), root.clone(), fake.generator());
    music.start_worker(&tokio::runtime::Handle::current(), || {});
    let moods = [Mood::Relaxed, Mood::Ominous, Mood::Cheerful, Mood::Solemn];
    for mood in moods {
        let mut block = shop_block();
        block.mood = mood;
        let set = MusicSetSpec::new(block.spec().unwrap());
        music.get_or_request(&set);
        assert_eq!(until_status(&music, &set, "ready").await.label(), "ready");
    }
    // Every set was made, none refused, and every set's calls are on the ledger.
    assert_eq!(provider_calls(&fake), 4 * PROVIDER_CALLS as usize);
    let spent = hub.ledger().snapshot(Unit::MusicCall);
    assert_eq!(spent.committed, 4 * u64::from(PROVIDER_CALLS));
    assert_eq!(spent.reserved, 0);
    let _removed = std::fs::remove_dir_all(root);
}

/// A ledger whose cost book is open with the checked-in price file.
fn metered_ledger() -> Arc<crate::gameplay::resources::SpendLedger> {
    use crate::gameplay::cost::load_prices;
    let path = std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../content/pricing.json"
    ));
    let (prices, status) = load_prices(&path);
    let ledger = Arc::new(crate::gameplay::resources::SpendLedger::default());
    ledger
        .costs()
        .open("music", None, prices, status, 1)
        .expect("the book opens");
    ledger
}

fn cause_events(report: &serde_json::Value, cause: &str) -> u64 {
    report["run"]["by_cause"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["cause"] == cause))
        .map_or(0, |row| row["events"].as_u64().unwrap())
}

#[tokio::test]
async fn a_music_set_is_metered_as_four_layers_and_one_ambience_in_seconds() {
    use crate::gameplay::resources::Unit;
    let fake = FakeEleven::start(Duration::ZERO, None);
    let ledger = metered_ledger();
    let generator = fake.generator().bound_to(Arc::clone(&ledger));
    generator
        .generate(&shop_set(), Duration::from_secs(60))
        .await
        .expect("a set");
    let report = ledger.costs().report(2);
    assert_eq!(report["run"]["events"], u64::from(PROVIDER_CALLS));
    assert_eq!(cause_events(&report, "place_music"), 4);
    assert_eq!(cause_events(&report, "place_ambience"), 1);
    assert_eq!(report["run"]["unpriced_events"], 0);
    assert_eq!(report["run"]["unknown_events"], 0);
    // Each layer asks for 48 seconds of music_v2 at 0.15 a minute, the bed 27 seconds of sound
    // generation at 0.12 a minute.
    assert_eq!(report["run"]["micro_usd"], 4 * 48 * 2_500 + 27 * 2_000);
    let spent = ledger.snapshot(Unit::MusicCall);
    assert_eq!(spent.committed, u64::from(PROVIDER_CALLS));
    assert_eq!(spent.reserved, 0);
}

#[tokio::test]
async fn a_rate_limited_ambience_is_not_billed_and_not_counted() {
    use crate::gameplay::resources::Unit;
    let fake = FakeEleven::start(Duration::ZERO, Some(("/v1/sound-generation", "", 429)));
    let ledger = metered_ledger();
    let generator = fake.generator().bound_to(Arc::clone(&ledger));
    let made = generator
        .generate(&shop_set(), Duration::from_secs(60))
        .await
        .expect("the layers are made without the bed");
    assert!(made.ambience.is_none());
    let report = ledger.costs().report(2);
    assert_eq!(cause_events(&report, "place_music"), 4);
    assert_eq!(
        cause_events(&report, "place_ambience"),
        0,
        "a 429 is not billed"
    );
    assert_eq!(
        ledger.snapshot(Unit::MusicCall).committed,
        u64::from(PROVIDER_CALLS) - 1,
        "and its unit is given back"
    );
}

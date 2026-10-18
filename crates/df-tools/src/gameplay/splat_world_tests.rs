//! Tests for the splat-world generator, store, asset route and seed import.
//! Every maker here is a fake: no test calls the live World Labs API.
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use tonic::codegen::Service;

use super::*;
use crate::gameplay::resources::{
    Hub, Limits, ResourceFailure, ResourceKind, SplatWorldFormat, SplatWorldSpec, Unit,
};

fn scratch(label: &str) -> PathBuf {
    let lane = std::env::current_dir()
        .unwrap()
        .join("artifacts/tmp/lanes/W3");
    std::fs::create_dir_all(&lane).unwrap();
    let directory = lane.join(format!("splat-{label}-{}", std::process::id()));
    let _removed = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn battle_view_world_is_present_only_for_an_open_fight_with_ready_metric_assets() {
    let ready = ReadyWorld {
        ply_url: "/assets/splat/worlds/test/world.ply".to_owned(),
        collider_url: "/assets/splat/worlds/test/collider.glb".to_owned(),
        metric_scale_factor: Some(1.5014322),
        ground_plane_offset: Some(1.2088797),
    };
    assert!(battle_world(true, None).is_none());
    assert!(battle_world(false, Some(ready.clone())).is_none());
    let view = battle_world(true, Some(ready.clone())).unwrap();
    assert_eq!(view.ply_url, ready.ply_url);
    assert_eq!(view.collider_url, ready.collider_url);
    assert_eq!(view.metric_scale_factor, 1.5014322);
    assert_eq!(view.ground_plane_offset, 1.2088797);
    assert!(
        battle_world(
            true,
            Some(ReadyWorld {
                metric_scale_factor: None,
                ..ready.clone()
            })
        )
        .is_none()
    );
    assert!(
        battle_world(
            true,
            Some(ReadyWorld {
                metric_scale_factor: Some(f64::NAN),
                ..ready
            })
        )
        .is_none()
    );
}

/// This is the exact pure transform used by the browser's world loader.
#[test]
fn battle_world_opencv_transform_scales_axes_sizes_and_aligns_the_floor() {
    use df_render::battle::{Vec3, marble_world_transform};
    let transform = marble_world_transform(1.501_432_2, 1.208_879_7).unwrap();
    let floor_raw_y = 1.208_879_7 / 1.501_432_2;
    let point = transform.to_world(Vec3::new(2.0, floor_raw_y, 3.0));
    assert!(point.y.abs() < 0.00001);
    assert!((point.x - 3.002_864_4).abs() < 0.00001);
    assert!((point.z + 4.504_297).abs() < 0.00001);
    assert!(transform.to_world(Vec3::new(0.0, floor_raw_y - 1.0, 0.0)).y > 0.0);
    assert_eq!(transform.to_world(Vec3::ZERO).y, 1.208_879_7);
    let placed = df_render::battle::SplatTransform {
        translate: Vec3::new(5.0, 0.0, -7.0),
        ..transform
    };
    let shifted = placed.to_world(Vec3::new(2.0, floor_raw_y, 3.0));
    assert!((shifted - point - Vec3::new(5.0, 0.0, -7.0)).length() < 0.00001);
    assert_eq!(placed.scale, transform.scale); // Floor and placement never offset Gaussian sizes.
}

#[tokio::test]
async fn cached_worlds_remain_available_without_a_worldlabs_maker() {
    let directory = scratch("cached-only");
    let spec = SplatWorldSpec::new("saved-shop", vec!["Shop".to_owned()], SplatWorldFormat::Ply);
    let result = WorldResult {
        world_id: "stored-shop".to_owned(),
        collider_url: String::new(),
        caption: "A photographic shop".to_owned(),
        metric_scale_factor: Some(1.5),
        ground_plane_offset: Some(1.2),
        credits: 0,
    };
    store_world(
        &directory,
        &ResourceKey::of(&spec),
        &result,
        &ply_fixture(1),
        b"glTF",
    )
    .unwrap();
    let service = SplatWorlds::cached(
        Hub::new(),
        Arc::new(|_| Box::pin(async { None })),
        directory.clone(),
    );
    service.request_world(&spec);
    assert!(service.ready_world("saved-shop").is_some());
    service.run_queued().await;
    assert_eq!(
        service.hub().ledger().snapshot(Unit::SplatCredit).committed,
        0
    );
    let restarted = SplatWorlds::cached(
        Hub::new(),
        Arc::new(|_| Box::pin(async { None })),
        directory.clone(),
    );
    assert_eq!(
        restarted.ready_world("saved-shop"),
        service.ready_world("saved-shop")
    );
    std::fs::remove_dir_all(directory).unwrap();
}

fn ply_fixture(splats: u64) -> Vec<u8> {
    let mut bytes =
        format!("ply\nformat binary_little_endian 1.0\nelement vertex {splats}\nend_header\n")
            .into_bytes();
    bytes.extend_from_slice(&[0u8; 56]);
    bytes
}

fn jpeg_fixture() -> Vec<u8> {
    let image = image::DynamicImage::new_rgb8(32, 24);
    let mut jpeg = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut jpeg),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    jpeg
}

struct FakeMaker {
    polls: Mutex<VecDeque<Result<WorldPoll, WorldMakerError>>>,
    submit_error: Option<WorldMakerError>,
    submits: Mutex<u32>,
    exports: Mutex<u32>,
    downloads: Mutex<u32>,
    prompt: Mutex<String>,
}

impl FakeMaker {
    fn succeeding(pending_polls: usize) -> Arc<Self> {
        let mut polls: VecDeque<_> = (0..pending_polls).map(|_| Ok(WorldPoll::Pending)).collect();
        polls.push_back(Ok(WorldPoll::Done(WorldResult {
            world_id: "world-1".to_owned(),
            collider_url: "https://fake/collider.glb".to_owned(),
            caption: "A photographic cloth shop.".to_owned(),
            metric_scale_factor: Some(1.5),
            ground_plane_offset: Some(1.2),
            credits: 1580,
        })));
        Arc::new(Self {
            polls: Mutex::new(polls),
            submit_error: None,
            submits: Mutex::new(0),
            exports: Mutex::new(0),
            downloads: Mutex::new(0),
            prompt: Mutex::new(String::new()),
        })
    }
}

impl WorldMaker for FakeMaker {
    fn submit<'a>(&'a self, request: &'a WorldRequest) -> MakerFuture<'a, OperationId> {
        *self.submits.lock().unwrap() += 1;
        *self.prompt.lock().unwrap() = request.text_prompt.clone();
        Box::pin(async move {
            if let Some(error) = &self.submit_error {
                return Err(WorldMakerError {
                    label: error.label,
                    permanent: error.permanent,
                });
            }
            assert_eq!(request.model, MarbleModel::Marble11);
            assert!(!request.image_jpeg.is_empty());
            Ok(OperationId("op-1".to_owned()))
        })
    }

    fn poll<'a>(&'a self, _operation: &'a OperationId) -> MakerFuture<'a, WorldPoll> {
        Box::pin(async move {
            self.polls
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(WorldPoll::Pending))
        })
    }

    fn export_ply<'a>(
        &'a self,
        _world_id: &'a str,
        resolution: PlyResolution,
    ) -> MakerFuture<'a, String> {
        *self.exports.lock().unwrap() += 1;
        Box::pin(async move {
            assert_eq!(resolution, PlyResolution::FiveHundredK);
            Ok("https://fake/world.ply".to_owned())
        })
    }

    fn download<'a>(&'a self, url: &'a str, _max_bytes: usize) -> MakerFuture<'a, Vec<u8>> {
        *self.downloads.lock().unwrap() += 1;
        let bytes = if url.ends_with(".ply") {
            ply_fixture(7)
        } else {
            b"glb-bytes".to_vec()
        };
        Box::pin(async move { Ok(bytes) })
    }
}

fn service(maker: Arc<FakeMaker>, cache_dir: PathBuf) -> SplatWorlds {
    let photograph: RoomPhotograph = Arc::new(|_place| Box::pin(async { Some(jpeg_fixture()) }));
    SplatWorlds::with_timing(
        Hub::new(),
        maker,
        photograph,
        cache_dir,
        PollTiming {
            interval: Duration::ZERO,
            timeout: Duration::from_secs(5),
        },
    )
}

fn cloth_shop() -> SplatWorldSpec {
    SplatWorldSpec::new(
        "place-mareks-cloth-shop",
        vec!["A cloth shop interior with a wooden counter.".to_owned()],
        SplatWorldFormat::Ply,
    )
}

#[test]
fn store_does_not_replace_an_existing_incomplete_key() {
    let cache_dir = scratch("immutable-key");
    let spec = cloth_shop();
    let key = ResourceKey::of(&spec);
    let directory = world_directory(&cache_dir, &key);
    std::fs::create_dir_all(&directory).unwrap();
    let marker = directory.join("partial-marker");
    std::fs::write(&marker, b"keep").unwrap();
    let result = WorldResult {
        world_id: "new-world".to_owned(),
        collider_url: "https://fake/collider.glb".to_owned(),
        caption: "A photographic cloth shop.".to_owned(),
        metric_scale_factor: Some(1.5),
        ground_plane_offset: Some(1.2),
        credits: 1580,
    };

    assert_eq!(
        store_world(&cache_dir, &key, &result, &ply_fixture(7), b"glb"),
        Err(ResourceFailure::Permanent("world_store_conflict"))
    );
    assert_eq!(std::fs::read(marker).unwrap(), b"keep");
}

#[tokio::test]
async fn generator_submits_polls_stores_and_reuses_by_key() {
    let cache_dir = scratch("generate");
    let maker = FakeMaker::succeeding(2);
    let worlds = service(maker.clone(), cache_dir.clone());
    let spec = cloth_shop();
    let key = worlds.request_world(&spec);
    assert!(worlds.ready_world("place-mareks-cloth-shop").is_none());
    worlds.run_queued().await;

    let ready = worlds
        .ready_world("place-mareks-cloth-shop")
        .expect("world is ready");
    assert_eq!(
        ready.ply_url,
        format!("/assets/splat/worlds/{}/world.ply", key.as_str())
    );
    assert_eq!(ready.metric_scale_factor, Some(1.5));
    assert_eq!(*maker.submits.lock().unwrap(), 1);
    assert_eq!(*maker.exports.lock().unwrap(), 1);
    assert_eq!(*maker.downloads.lock().unwrap(), 2);
    assert!(maker.prompt.lock().unwrap().contains("photographic"));
    assert_eq!(
        worlds.hub().ledger().snapshot(Unit::SplatCredit).committed,
        1580
    );

    let directory = world_directory(&cache_dir, &key);
    let stored: StoredWorld =
        serde_json::from_slice(&std::fs::read(directory.join("world.json")).unwrap()).unwrap();
    assert_eq!(stored.splats, 7);
    assert_eq!(stored.world_id, "world-1");
    assert!(directory.join("world.ply").is_file());

    // A second ask for the same place reuses the stored world: no new submit.
    worlds.request_world(&spec);
    worlds.run_queued().await;
    assert_eq!(*maker.submits.lock().unwrap(), 1);

    // A fresh hub also adopts the stored files under their existing resource key.
    let restarted = service(maker.clone(), cache_dir.clone());
    restarted.request_world(&spec);
    restarted.run_queued().await;
    assert_eq!(*maker.submits.lock().unwrap(), 1);
    assert!(restarted.ready_world("place-mareks-cloth-shop").is_some());
    std::fs::remove_dir_all(cache_dir).unwrap();
}

#[tokio::test]
async fn generator_maps_failures_and_never_stores_them() {
    let cache_dir = scratch("failure");
    let mut maker = FakeMaker::succeeding(0);
    Arc::get_mut(&mut maker).unwrap().polls = Mutex::new(
        [Ok(WorldPoll::Failed("refused".to_owned()))]
            .into_iter()
            .collect(),
    );
    let worlds = service(maker, cache_dir.clone());
    let key = worlds.request_world(&cloth_shop());
    worlds.run_queued().await;
    assert!(worlds.ready_world("place-mareks-cloth-shop").is_none());
    assert!(read_stored_world(&cache_dir, &key).is_none());
    let rows = worlds.hub().list(None);
    assert_eq!(rows[0].state, "failed");
    assert_eq!(rows[0].last_error, "generation_failed");
    std::fs::remove_dir_all(cache_dir).unwrap();
}

#[tokio::test]
async fn generator_maps_transient_provider_errors_into_the_hub_failure() {
    let cache_dir = scratch("provider-failure");
    let mut maker = FakeMaker::succeeding(0);
    Arc::get_mut(&mut maker).unwrap().submit_error =
        Some(WorldMakerError::transient("rate_limited"));
    let hub = Hub::new();
    hub.set_limits(
        ResourceKind::SplatWorld,
        Limits {
            timeout: Duration::from_secs(5),
            attempts: 1,
        },
    );
    let photograph: RoomPhotograph = Arc::new(|_place| Box::pin(async { Some(jpeg_fixture()) }));
    let worlds = SplatWorlds::with_timing(
        hub,
        maker,
        photograph,
        cache_dir.clone(),
        PollTiming {
            interval: Duration::ZERO,
            timeout: Duration::from_secs(5),
        },
    );
    worlds.request_world(&cloth_shop());
    worlds.run_queued().await;
    let rows = worlds.hub().list(None);
    assert_eq!(rows[0].last_error, "rate_limited");
    assert_eq!(rows[0].state, "failed");
    std::fs::remove_dir_all(cache_dir).unwrap();
}

#[test]
fn a_place_request_keeps_one_resource_key_even_if_later_facts_change() {
    let cache_dir = scratch("place-idempotent");
    let worlds = service(FakeMaker::succeeding(0), cache_dir.clone());
    let original = cloth_shop();
    let first_key = worlds.request_world(&original);
    let changed = SplatWorldSpec::new(
        "place-mareks-cloth-shop",
        vec!["A different description.".to_owned()],
        SplatWorldFormat::Ply,
    );
    assert_eq!(worlds.request_world(&changed), first_key);
    assert_eq!(worlds.hub().list(None).len(), 1);
    std::fs::remove_dir_all(cache_dir).unwrap();
}

#[tokio::test]
async fn generator_reports_a_missing_photograph_as_permanent() {
    let cache_dir = scratch("no-photo");
    let maker = FakeMaker::succeeding(0);
    let photograph: RoomPhotograph = Arc::new(|_place| Box::pin(async { None }));
    let worlds = SplatWorlds::with_timing(
        Hub::new(),
        maker,
        photograph,
        cache_dir.clone(),
        PollTiming {
            interval: Duration::ZERO,
            timeout: Duration::from_secs(5),
        },
    );
    worlds.request_world(&cloth_shop());
    worlds.run_queued().await;
    let rows = worlds.hub().list(None);
    assert_eq!(rows[0].last_error, "no_room_photograph");
    std::fs::remove_dir_all(cache_dir).unwrap();
}

#[test]
fn prompt_and_source_image_stay_photographic_and_bounded() {
    let prompt = build_text_prompt(&["A cloth shop.".to_owned()]);
    assert!(prompt.chars().count() <= 1_000);
    for word in crate::gameplay::imagery::NON_PHOTOGRAPHIC_WORDS {
        assert!(!prompt.contains(word), "{word}");
    }
    let jpeg = prepare_source_image(&jpeg_fixture()).unwrap();
    assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
    let mut oversized = Vec::new();
    image::DynamicImage::new_rgb8(2400, 1200)
        .write_to(
            &mut std::io::Cursor::new(&mut oversized),
            image::ImageFormat::Png,
        )
        .unwrap();
    let resized = image::load_from_memory(&prepare_source_image(&oversized).unwrap()).unwrap();
    assert_eq!((resized.width(), resized.height()), (1600, 800));
    assert_eq!(ply_splat_count(&ply_fixture(500_000)), Some(500_000));
    assert_eq!(ResourceFailure::Permanent("x").label(), "x");
}

#[tokio::test]
async fn asset_route_serves_a_world_stored_after_startup() {
    let cache_dir = scratch("route");
    let spec = SplatWorldSpec::new(
        "place-route-fixture",
        vec!["A route-only photographic fixture.".to_owned()],
        SplatWorldFormat::Ply,
    );
    let key = ResourceKey::of(&spec);
    let result = WorldResult {
        world_id: "world-1".to_owned(),
        collider_url: String::new(),
        caption: String::new(),
        metric_scale_factor: Some(1.5),
        ground_plane_offset: Some(1.2),
        credits: 0,
    };
    store_world(&cache_dir, &key, &result, &ply_fixture(3), b"glb").unwrap();

    let empty = scratch("route-empty");
    let mut router = crate::gameplay::splat_assets::router(&empty).unwrap();
    for (path, status, expected_body) in [
        (
            format!("/assets/splat/worlds/{}/world.ply", key.as_str()),
            StatusCode::OK,
            Some(ply_fixture(3)),
        ),
        (
            format!("/assets/splat/worlds/{}/collider.glb", key.as_str()),
            StatusCode::OK,
            Some(b"glb".to_vec()),
        ),
        (
            "/assets/splat/worlds/deadbeef/world.ply".to_owned(),
            StatusCode::NOT_FOUND,
            None,
        ),
    ] {
        let request = Request::builder()
            .method(Method::GET)
            .uri(&path)
            .body(Body::empty())
            .unwrap();
        let response = Service::call(&mut router, request).await.unwrap();
        assert_eq!(response.status(), status, "{path}");
        if let Some(expected_body) = expected_body {
            let body = axum::body::to_bytes(response.into_body(), MAXIMUM_PLY_BYTES)
                .await
                .unwrap();
            assert_eq!(body.as_ref(), expected_body, "{path}");
        }
    }
    std::fs::remove_dir_all(cache_dir).unwrap();
    std::fs::remove_dir_all(empty).unwrap();
}

#[test]
fn seed_import_copies_the_proof_world_only_when_named() {
    let cache_dir = scratch("seed-cache");
    // A stand-in for artifacts/tmp/worldlabs/out/m11 (gitignored, absent from a clean checkout):
    // the same file names with a small readable PLY and collider.
    let proof = scratch("seed-proof");
    let ply = ply_fixture(4);
    std::fs::write(proof.join("m11_500k.ply"), &ply).unwrap();
    std::fs::write(proof.join("m11_collider.glb"), b"glTF collider").unwrap();

    // Without the switch, nothing is seeded. This test owns the variable.
    unsafe { std::env::remove_var("DF_GAMEPLAY_SPLAT_SEED") };
    assert!(seed_proof_world(&cache_dir, &proof).unwrap().is_none());

    unsafe { std::env::set_var("DF_GAMEPLAY_SPLAT_SEED", "m11") };
    let ready = seed_proof_world(&cache_dir, &proof).unwrap();
    unsafe { std::env::remove_var("DF_GAMEPLAY_SPLAT_SEED") };
    let ready = ready.expect("proof world seeded");
    assert_eq!(ready.metric_scale_factor, Some(1.5014322));
    assert!(ready.ply_url.contains("world.ply"));
    assert_eq!(
        std::fs::metadata(
            world_directory(
                &cache_dir,
                &ResourceKey::of(&SplatWorldSpec::new(
                    PROOF_PLACE_ID,
                    Vec::new(),
                    SplatWorldFormat::Ply,
                ))
            )
            .join("world.ply")
        )
        .unwrap()
        .len(),
        ply.len() as u64
    );
    let worlds = service(FakeMaker::succeeding(0), cache_dir.clone());
    assert_eq!(
        worlds.ready_world(PROOF_PLACE_ID),
        Some(ready.clone()),
        "the seeded world is discoverable before any generation request"
    );
    std::fs::remove_dir_all(cache_dir).unwrap();
    std::fs::remove_dir_all(proof).unwrap();
}

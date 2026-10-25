use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

// Offline ffmpeg fixtures: 640x960, 24 fps, five seconds. The guard is gray64,
// the contact flash white at 1.0s, follow-through gray128, then guard at 1.5s.
// The fallen fixture keeps gray128 after the flash. No provider output is embedded.
const ATTACK_FIXTURE: &[u8] = include_bytes!("character_clips_fixture.mp4");
const FALLEN_FIXTURE: &[u8] = include_bytes!("character_clips_fallen_fixture.mp4");

fn directory(label: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/P3/tests")
        .join(format!(
            "{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn fighter() -> Fighter {
    Fighter {
        identity: "entity:test-fighter".to_owned(),
        identity_revision: "identity-v1".to_owned(),
        depiction_subject: "hero:test".to_owned(),
        reviewed_figure: None,
        name: "Brakka".to_owned(),
        weapon: "greatsword".to_owned(),
        attack_style: AttackStyle::Melee,
    }
}

fn alpha(fallen: bool) -> Vec<u8> {
    let mut image = image::RgbaImage::from_pixel(1024, 1536, image::Rgba([0, 0, 0, 0]));
    for y in 300..1200 {
        for x in 350..650 {
            image.put_pixel(
                x,
                y,
                if fallen {
                    image::Rgba([180, 0, 0, 255])
                } else {
                    image::Rgba([0, 0, 180, 255])
                },
            );
        }
    }
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    encoded.into_inner()
}

struct FakeFrames {
    calls: AtomicUsize,
    refuse: bool,
    prompts: Mutex<Vec<String>>,
    /// When set, every call waits here, so a test can require calls to be in flight together.
    rendezvous: Option<Arc<tokio::sync::Barrier>>,
}

impl FrameProvider for FakeFrames {
    fn frame<'a>(
        &'a self,
        source: &'a Depiction,
        prompt: &'a str,
    ) -> ProviderFuture<'a, GeneratedImage> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert!(!source.bytes.is_empty());
            locked(&self.prompts).push(prompt.to_owned());
            if let Some(rendezvous) = &self.rendezvous {
                rendezvous.wait().await;
            }
            if self.refuse {
                return Err(failure("fixture_image_refused"));
            }
            let fallen = prompt.contains("lies fallen");
            Ok(GeneratedImage {
                bytes: alpha(fallen),
                format: ImageFormat::Png,
                width: 1024,
                height: 1536,
                has_alpha_channel: true,
                usage: df_providers::ImageUsage {
                    text_input_tokens: 10,
                    image_input_tokens: 20,
                    output_tokens: 30,
                },
            })
        })
    }
}

struct FakeVideos {
    calls: AtomicUsize,
    refuse: bool,
    prompts: Mutex<Vec<String>>,
}

impl VideoProvider for FakeVideos {
    fn video<'a>(
        &'a self,
        _key: &'a ResourceKey,
        start: &'a [u8],
        end: &'a [u8],
        prompt: &'a str,
    ) -> ProviderFuture<'a, Video> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            locked(&self.prompts).push(prompt.to_owned());
            if self.refuse {
                return Err(failure("fixture_video_refused"));
            }
            let fallen = prompt.contains("final blow");
            assert_eq!(start == end, !fallen);
            let start_image = image::load_from_memory(start).unwrap().to_rgb8();
            assert_eq!(start_image.dimensions(), (768, 1152));
            assert_eq!(start_image.get_pixel(0, 0).0, [0, 177, 64]);
            Ok(Video {
                bytes: if fallen {
                    FALLEN_FIXTURE
                } else {
                    ATTACK_FIXTURE
                }
                .to_vec(),
            })
        })
    }
}

/// A hub whose cost book is open with the two rates a clip uses: the Kling second and the
/// gpt-image tokens of a frame.
fn priced_hub() -> Hub {
    let prices = crate::gameplay::cost::PriceTable::parse(
        r#"{"schema": "dungeonflux.pricing/1", "currency": "USD", "prices": [
            {"provider": "fal", "model": "fal-ai/kling-video/o1/image-to-video",
             "category": "video_seconds", "usd": "0.112", "per": 1,
             "as_of": "2026-10-24", "source": "https://fal.ai/models/kling"},
            {"provider": "openai", "model": "gpt-image-2.5-sunburst", "category": "text_input",
             "usd": "5", "per": 1000000, "as_of": "2026-10-24", "source": "https://example.test/p"},
            {"provider": "openai", "model": "gpt-image-2.5-sunburst", "category": "image_input",
             "usd": "8", "per": 1000000, "as_of": "2026-10-24", "source": "https://example.test/p"},
            {"provider": "openai", "model": "gpt-image-2.5-sunburst", "category": "image_output",
             "usd": "30", "per": 1000000, "as_of": "2026-10-24", "source": "https://example.test/p"}
        ]}"#,
    )
    .expect("the clip prices parse");
    let hub = Hub::new();
    hub.ledger()
        .costs()
        .open(
            "clips",
            None,
            prices,
            crate::gameplay::cost::PricingStatus::NotLoaded,
            1,
        )
        .expect("the book opens");
    hub
}

fn setup(
    image_refuse: bool,
    video_refuse: bool,
) -> (Arc<CharacterClips>, Arc<FakeFrames>, Arc<FakeVideos>) {
    setup_with_rendezvous(image_refuse, video_refuse, None)
}

fn setup_with_rendezvous(
    image_refuse: bool,
    video_refuse: bool,
    rendezvous: Option<Arc<tokio::sync::Barrier>>,
) -> (Arc<CharacterClips>, Arc<FakeFrames>, Arc<FakeVideos>) {
    let frames = Arc::new(FakeFrames {
        calls: AtomicUsize::new(0),
        refuse: image_refuse,
        prompts: Mutex::new(Vec::new()),
        rendezvous,
    });
    let videos = Arc::new(FakeVideos {
        calls: AtomicUsize::new(0),
        refuse: video_refuse,
        prompts: Mutex::new(Vec::new()),
    });
    let source: Source = Arc::new(|_| {
        Box::pin(async {
            Some(Depiction {
                bytes: vec![1, 2, 3],
                format: ImageFormat::Png,
                revision: "canonical-v1".to_owned(),
            })
        })
    });
    let service = CharacterClips::new(
        priced_hub(),
        Some(frames.clone()),
        Some(videos.clone()),
        source,
        directory("service"),
    );
    (Arc::new(service), frames, videos)
}

async fn finish_queued(service: &CharacterClips) {
    let jobs = service.generation.hub.take_jobs(16);
    let completed = futures::future::join_all(jobs.into_iter().map(|job| job.execute())).await;
    for done in completed {
        service.generation.hub.complete(done);
    }
}

#[test]
fn character_clips_keys_pin_all_pixel_dependencies_and_share_the_guard() {
    let spec = CharacterClipSpec::new(
        fighter(),
        "depiction-v1:bytes-hash".to_owned(),
        ClipKind::Idle,
    );
    let key = ResourceKey::of(&spec);
    assert_eq!(key, ResourceKey::of(&spec.clone()));
    let mut renamed = spec.clone();
    renamed.fighter.name = "A display rename".to_owned();
    assert_eq!(key, ResourceKey::of(&renamed));
    let variants = [
        {
            let mut value = spec.clone();
            value.fighter.identity = "entity:other".to_owned();
            value
        },
        {
            let mut value = spec.clone();
            value.fighter.identity_revision = "v2".to_owned();
            value
        },
        {
            let mut value = spec.clone();
            value.depiction_revision = "new-bytes".to_owned();
            value
        },
        {
            let mut value = spec.clone();
            value.kind = ClipKind::Attack;
            value
        },
        {
            let mut value = spec.clone();
            value.fighter.weapon = "javelin".to_owned();
            value
        },
        {
            let mut value = spec.clone();
            value.fighter.attack_style = AttackStyle::Thrown;
            value
        },
        {
            let mut value = spec.clone();
            value.home_angle_version = "home-v2".to_owned();
            value
        },
    ];
    for changed in variants {
        assert_ne!(key, ResourceKey::of(&changed));
    }
    let keys = ClipKind::ALL.map(|kind| {
        let mut value = spec.clone();
        value.kind = kind;
        ResourceKey::of(&FrameSpec {
            clip: value,
            fallen: false,
        })
    });
    assert_eq!(keys[0], keys[1]);
    assert_eq!(keys[1], keys[2]);
    assert_ne!(
        keys[0],
        ResourceKey::of(&FrameSpec {
            clip: spec.clone(),
            fallen: true
        })
    );
    assert_ne!(key, keys[0]);
    assert_eq!(ResourceKind::CharacterClip.label(), "character_clip");
}

#[tokio::test]
async fn character_clips_request_the_three_clips_once_and_reuse_one_guard() {
    let (service, frames, videos) = setup(false, false);
    for _ in 0..3 {
        service.request(fighter(), None, false);
    }
    service.resolve_sources().await;
    assert_eq!(
        service
            .generation
            .hub
            .list(Some(ResourceKind::CharacterClip))
            .len(),
        3
    );
    finish_queued(&service).await;
    service.request(fighter(), None, false);
    service.resolve_sources().await;
    assert!(service.generation.hub.take_jobs(16).is_empty());
    assert_eq!(
        frames.calls.load(Ordering::SeqCst),
        2,
        "one guard and one fallen frame"
    );
    assert_eq!(videos.calls.load(Ordering::SeqCst), 3);
    assert_eq!(
        service
            .generation
            .hub
            .ledger()
            .snapshot(Unit::ImageCall)
            .committed,
        2
    );
    assert_eq!(
        service
            .generation
            .hub
            .ledger()
            .snapshot(Unit::VideoClip)
            .committed,
        3
    );
    let document = service.list(&fighter());
    assert_eq!(document["resources"].as_array().unwrap().len(), 5);
    assert!(
        document["resources"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["state"] == "ready")
    );
    let prompts = locked(&frames.prompts);
    assert!(prompts.iter().all(|prompt| prompt.contains("15 degrees")
        && prompt.contains("38 degree")
        && prompt.contains("greatsword")));
}

#[test]
fn character_clips_measure_count_clock_size_contact_guard_return_and_fall_settle() {
    let root = directory("measurement");
    let attack = root.join("attack.mp4");
    std::fs::write(&attack, ATTACK_FIXTURE).unwrap();
    let metadata = measure_clip(&attack, ClipKind::Attack).unwrap();
    assert_eq!(
        (metadata.frame_count, metadata.width, metadata.height),
        (120, 640, 960)
    );
    assert!((metadata.fps - 24.0).abs() < 0.01);
    assert_eq!(metadata.duration_ms, 5000);
    assert_eq!(metadata.bytes, ATTACK_FIXTURE.len() as u64);
    assert_eq!(metadata.contact_ms, Some(1000));
    assert_eq!(metadata.return_to_guard_ms, Some(1542));
    let fallen = root.join("fallen.mp4");
    std::fs::write(&fallen, FALLEN_FIXTURE).unwrap();
    let metadata = measure_clip(&fallen, ClipKind::LastHit).unwrap();
    assert_eq!(metadata.settle_ms, Some(1125));
    assert!(metadata.contact_ms.is_none());
    assert!(
        measure_clip(&attack, ClipKind::LastHit).is_err(),
        "a return to guard is not a fallen last hit"
    );
}

#[tokio::test]
async fn guard_frames_of_different_fighters_are_drawn_at_the_same_time() {
    // The barrier opens only when both guards are inside the provider together. A gate shared by
    // every frame would hold the second fighter behind the first and time out here.
    let (service, frames, _) =
        setup_with_rendezvous(false, false, Some(Arc::new(tokio::sync::Barrier::new(2))));
    let second = Fighter {
        identity: "entity:second-fighter".to_owned(),
        name: "Osric".to_owned(),
        ..fighter()
    };
    let source = Arc::new(Depiction {
        bytes: vec![1, 2, 3],
        format: ImageFormat::Png,
        revision: "canonical-v1".to_owned(),
    });
    let first_spec = CharacterClipSpec::new(fighter(), "canonical-v1".to_owned(), ClipKind::Idle);
    let second_spec = CharacterClipSpec::new(second, "canonical-v1".to_owned(), ClipKind::Idle);
    let (first, other) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(
            service
                .generation
                .frame(&first_spec, Arc::clone(&source), false, false),
            service
                .generation
                .frame(&second_spec, Arc::clone(&source), false, false),
        )
    })
    .await
    .expect("guard frames of two fighters are drawn concurrently");
    assert!(first.is_ok() && other.is_ok());
    assert_eq!(frames.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn character_clips_image_failure_is_shared_and_leaves_cutout_fallback() {
    let (service, frames, videos) = setup(true, false);
    service.request(fighter(), None, false);
    service.resolve_sources().await;
    finish_queued(&service).await;
    service.request(fighter(), None, false);
    service.resolve_sources().await;
    assert!(service.generation.hub.take_jobs(16).is_empty());
    assert_eq!(frames.calls.load(Ordering::SeqCst), 1);
    assert_eq!(videos.calls.load(Ordering::SeqCst), 0);
    let rows = service
        .generation
        .hub
        .list(Some(ResourceKind::CharacterClip));
    assert_eq!(rows.len(), 4);
    assert!(
        rows.iter()
            .all(|row| row.state == "failed" && row.handle.is_empty())
    );
    assert!(rows.iter().all(|row| matches!(
        service.generation.hub.status(&row.key),
        super::super::resources::ResourceStatus::Failed(_, Chrome::Nothing)
    )));
}

#[tokio::test]
async fn character_clips_video_failure_retains_spend_and_never_admits_broken_media() {
    let (service, _, videos) = setup(false, true);
    service.request(fighter(), Some(ClipKind::Attack), false);
    service.resolve_sources().await;
    finish_queued(&service).await;
    assert_eq!(videos.calls.load(Ordering::SeqCst), 1);
    let document = service.list(&fighter());
    let failed = document["resources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["state"] == "failed")
        .unwrap();
    assert_eq!(failed["cost"]["video_calls"], 1);
    assert!(failed["timings"].is_null());
    assert!(!Path::new(failed["clip"].as_str().unwrap()).exists());
}

#[tokio::test]
async fn character_clips_absent_source_waits_without_spend_or_invented_spec() {
    let (mut service, frames, videos) = setup(false, false);
    Arc::get_mut(&mut service).unwrap().source = Arc::new(|_| Box::pin(async { None }));
    service.request(fighter(), None, false);
    service.resolve_sources().await;
    assert!(
        service.list(&fighter())["waiting_for_depiction"]
            .as_bool()
            .unwrap()
    );
    assert!(
        service
            .generation
            .hub
            .list(Some(ResourceKind::CharacterClip))
            .is_empty()
    );
    assert_eq!(frames.calls.load(Ordering::SeqCst), 0);
    assert_eq!(videos.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn character_clips_missing_source_is_called_once_until_revision_wake_or_force() {
    let (mut service, _, _) = setup(false, false);
    let calls = Arc::new(AtomicUsize::new(0));
    let available = Arc::new(Mutex::new(None::<(Vec<u8>, String)>));
    let (counted_calls, counted_available) = (Arc::clone(&calls), Arc::clone(&available));
    Arc::get_mut(&mut service).unwrap().source = Arc::new(move |_| {
        let calls = Arc::clone(&counted_calls);
        let available = Arc::clone(&counted_available);
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            locked(&available)
                .clone()
                .map(|(bytes, revision)| Depiction {
                    bytes,
                    format: ImageFormat::Png,
                    revision,
                })
        })
    });
    for _ in 0..5 {
        service.request(fighter(), None, false);
        service.resolve_sources().await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1, "many ticks, one miss call");
    assert!(
        service.list(&fighter())["waiting_for_depiction"]
            .as_bool()
            .unwrap()
    );
    let mut changed = fighter();
    changed.identity_revision = "identity-v2".to_owned();
    for _ in 0..3 {
        service.request(changed.clone(), None, false);
        service.resolve_sources().await;
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "changed revision, one new call"
    );
    *locked(&available) = Some((vec![1, 2, 3], "canonical-v2".to_owned()));
    service.note_source_changed();
    service.request(changed.clone(), None, false);
    service.resolve_sources().await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "wake re-admits the miss once"
    );
    assert_eq!(
        service
            .generation
            .hub
            .list(Some(ResourceKind::CharacterClip))
            .len(),
        3
    );
    for _ in 0..3 {
        service.request(changed.clone(), None, false);
        service.resolve_sources().await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3, "resolved source is cached");
    service.request(changed.clone(), Some(ClipKind::Idle), true);
    service.resolve_sources().await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        4,
        "force retries exactly once"
    );
    service.request(changed.clone(), Some(ClipKind::Idle), false);
    service.resolve_sources().await;
    assert_eq!(calls.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn character_clips_explicit_request_regenerates_only_selected_clip_and_retains_cost() {
    let (service, frames, videos) = setup(false, false);
    service.request(fighter(), Some(ClipKind::Attack), false);
    service.resolve_sources().await;
    finish_queued(&service).await;
    service.request(fighter(), Some(ClipKind::Attack), true);
    service.resolve_sources().await;
    finish_queued(&service).await;
    assert_eq!(videos.calls.load(Ordering::SeqCst), 2);
    assert_eq!(frames.calls.load(Ordering::SeqCst), 1);
    let document = service.list(&fighter());
    let rows = document["resources"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "one attack and its shared guard");
    let clip = rows.iter().find(|row| row["kind"] == "attack").unwrap();
    assert_eq!(clip["state"], "ready");
    assert_eq!(clip["cost"]["video_calls"], 2);
    assert_eq!(clip["cost"]["video_micro_dollars"], 1_120_000);
    assert_eq!(clip["cost"]["video_cost_basis"], "price_file");
    // The cost book metered the same calls: two clips of five seconds at 0.112 a second and the
    // one shared frame, at the tokens the frame reported (10 + 20 + 30 tokens).
    let book = service.generation.hub.ledger().costs().report(2);
    let by_cause = |cause: &str| {
        book["run"]["by_cause"]
            .as_array()
            .and_then(|causes| causes.iter().find(|row| row["cause"] == cause))
            .map(|row| (row["events"].clone(), row["micro_usd"].clone()))
    };
    assert_eq!(
        by_cause("character_clip"),
        Some((serde_json::json!(2), serde_json::json!(1_120_000)))
    );
    assert_eq!(
        by_cause("clip_frame"),
        Some((
            serde_json::json!(1),
            serde_json::json!(10 * 5 + 20 * 8 + 30 * 30)
        ))
    );
    assert!(clip["timings"]["contact_ms"].as_u64().is_some());
}

#[tokio::test]
async fn character_clips_queued_makers_do_not_retain_the_service_or_its_credentials() {
    let (service, frames, videos) = setup(false, false);
    service.request(fighter(), None, false);
    service.resolve_sources().await;
    let weak = Arc::downgrade(&service);
    let hub = service.generation.hub.clone();
    drop(service);
    assert!(weak.upgrade().is_none());
    for job in hub.take_jobs(3) {
        hub.complete(job.execute().await);
    }
    assert!(
        hub.list(Some(ResourceKind::CharacterClip))
            .iter()
            .all(|row| row.last_error == "interrupted")
    );
    assert_eq!(frames.calls.load(Ordering::SeqCst), 0);
    assert_eq!(videos.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn character_clips_recovered_failed_dispatch_requires_an_explicit_request() {
    let (previous, _, _) = setup(false, true);
    previous.request(fighter(), Some(ClipKind::Attack), false);
    previous.resolve_sources().await;
    finish_queued(&previous).await;
    let (fresh, frames, videos) = setup(false, false);
    let recovered = Arc::new(CharacterClips::new(
        Hub::new(),
        Some(frames.clone()),
        Some(videos.clone()),
        Arc::clone(&fresh.source),
        previous.generation.directory.clone(),
    ));
    recovered.request(fighter(), Some(ClipKind::Attack), false);
    recovered.resolve_sources().await;
    finish_queued(&recovered).await;
    assert_eq!(videos.calls.load(Ordering::SeqCst), 0);
    assert!(
        recovered
            .generation
            .hub
            .list(Some(ResourceKind::CharacterClip))
            .iter()
            .any(|row| row.last_error == "clip_prior_attempt_requires_explicit_retry")
    );
    recovered.request(fighter(), Some(ClipKind::Attack), true);
    recovered.resolve_sources().await;
    finish_queued(&recovered).await;
    assert_eq!(videos.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        frames.calls.load(Ordering::SeqCst),
        0,
        "the previous guard is reused"
    );
    let document = recovered.list(&fighter());
    assert!(
        document["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["kind"] == "attack"
                && row["state"] == "ready"
                && row["cost"]["video_calls"] == 2)
    );
}

#[test]
fn character_clips_native_binding_preserves_distinct_bookends_and_weapon_action() {
    let spec = CharacterClipSpec::new(fighter(), "depiction".to_owned(), ClipKind::LastHit);
    let binding = video_binding(
        &ResourceKey::of(&spec),
        b"guard",
        b"fallen",
        &motion_prompt(&spec),
    )
    .unwrap();
    let frames = binding.semantic_basis.semantic.reference_frames;
    assert_eq!(frames.len(), 2);
    assert_ne!(frames[0].locator, frames[1].locator);
    assert_ne!(frames[0].asset.digest, frames[1].asset.digest);
    for kind in ClipKind::ALL {
        let mut spec = spec.clone();
        spec.kind = kind;
        assert!(motion_prompt(&spec).chars().count() <= 1500);
    }
    let mut thrown = spec.clone();
    thrown.kind = ClipKind::Attack;
    thrown.fighter.attack_style = AttackStyle::Thrown;
    assert!(motion_prompt(&thrown).contains("visibly RELEASE"));
    assert_eq!(
        AttackStyle::weapon(df_rules::tactical::Weapon::Javelin),
        AttackStyle::Thrown
    );
    assert_eq!(
        AttackStyle::weapon(df_rules::tactical::Weapon::Longbow),
        AttackStyle::Ranged
    );
    assert_eq!(
        reviewed_figure("sprite-patron-merchant"),
        Some("sprite-patron-merchant-documentary-photo".to_owned())
    );
    assert!(reviewed_figure("portrait-patron-merchant").is_none());
}

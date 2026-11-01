//! Operator-run live checks of the image service against the real provider. Ignored by default:
//! they spend money. `DF_L01_PLAN` names a JSON plan, `DF_L01_OUT` the directory results are
//! copied to, and `OPENAI_API_KEY` the credential (read from the environment by the launcher
//! script, never from this file). Results are copied before the disposable host is closed.
//! Reuse within one retained host is durable; a new disposable owner starts a new run.

use std::time::Instant;

use serde::Deserialize;

use super::{
    super::{
        npc_dossier_tests::adventure,
        pack::{CastRole, SceneKey},
        scene_mood::{Expression, HealthTier, Lighting, Look, Mood, Shot},
    },
    *,
};

#[derive(Deserialize)]
struct Job {
    name: String,
    /// `expression`, `refresh` or `close_up`.
    kind: String,
    seed: u64,
    /// The pack scene key, as its kebab-case name.
    scene: String,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    expression: Option<String>,
    #[serde(default)]
    tier: Option<String>,
    #[serde(default)]
    mood: Option<String>,
    #[serde(default)]
    shot: Option<String>,
    #[serde(default)]
    lighting: Option<String>,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    rating: Option<String>,
}

fn pick<T: Copy>(all: &[T], label: impl Fn(T) -> &'static str, wanted: &str) -> T {
    *all.iter()
        .find(|item| label(**item) == wanted)
        .unwrap_or_else(|| panic!("unknown label {wanted}"))
}

fn live_patch(backend: df_imagery::BackendId, deadline_secs: u64) -> df_imagery::RuntimePatch {
    let mut patch = df_imagery::RuntimePatch::empty();
    patch.mode = Some(df_imagery::GlobalMode::Live);
    patch.accounting_mode = Some(df_imagery::AccountingMode::RecordOnly);
    patch.backend_enabled = [Some(false); df_imagery::BackendId::COUNT];
    patch.backend_enabled[backend.index()] = Some(true);
    patch.backend_account[backend.index()] =
        Some(df_imagery::AccountHandle::new("default").unwrap());
    patch.default_backend = Some(backend);
    patch.retry_max_attempts = Some(1);
    patch.scheduler_stage_deadline_secs = Some(deadline_secs);
    patch.scheduler_job_deadline_secs = Some(deadline_secs + 30);
    for purpose in 0..df_imagery::Purpose::COUNT {
        patch.purpose_model_override[purpose] = Some(None);
        patch.purpose_quality_override[purpose] = Some(None);
    }
    patch
}

fn live_costs() -> super::super::cost::port::MeteringPort {
    let costs = super::super::cost::port::MeteringPort::new();
    costs
        .open(
            "operator-live-images",
            include_str!("../../../../content/pricing.json"),
        )
        .unwrap();
    costs
}

async fn service(rating: ContentRating) -> super::super::image_test_support::ImageHostFixture {
    use super::super::image_test_support::{ImageHostFixture, LiveImageOptIn};
    let mut patch = live_patch(df_imagery::BackendId::OpenAi, 90);
    let model = std::env::var("DF_L01_MODEL").unwrap_or_else(|_| "gpt-image-2.5-flare".to_owned());
    patch.backend_model[df_imagery::BackendId::OpenAi.index()] =
        Some(Some(df_imagery::ModelName::new(&model).unwrap()));
    patch.backend_endpoint[df_imagery::BackendId::OpenAi.index()] = Some(Some(
        df_imagery::ApiEndpoint::new("https://api.openai.com").unwrap(),
    ));
    patch.quality_provider_quality = Some(Some(match std::env::var("DF_L01_QUALITY").as_deref() {
        Ok("low") => df_imagery::GenerationQuality::Low,
        Ok("high") => df_imagery::GenerationQuality::High,
        _ => df_imagery::GenerationQuality::Medium,
    }));
    let rating = match rating {
        ContentRating::Mature => df_imagery::ImageContentRating::Mature,
        ContentRating::Restrained => df_imagery::ImageContentRating::Restrained,
    };
    ImageHostFixture::open_live(patch, live_costs(), rating, LiveImageOptIn::Plan)
        .await
        .expect("operator local PG bootstrap")
}

#[tokio::test]
#[ignore = "spends money against the real provider; run by the operator"]
async fn live_plan() {
    let plan = std::env::var("DF_L01_PLAN").expect("DF_L01_PLAN");
    let out = PathBuf::from(std::env::var("DF_L01_OUT").expect("DF_L01_OUT"));
    std::fs::create_dir_all(out.join("live")).unwrap();
    let jobs: Vec<Job> = serde_json::from_str(&std::fs::read_to_string(plan).unwrap()).unwrap();
    let store = ReferenceStore::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated"),
    );
    let mature = service(ContentRating::Mature).await;
    let restrained = service(ContentRating::Restrained).await;
    for job in jobs {
        let rating = match job.rating.as_deref() {
            Some("restrained") => ContentRating::Restrained,
            _ => ContentRating::Mature,
        };
        let selected = if rating == ContentRating::Mature {
            &mature
        } else {
            &restrained
        };
        let service = selected.imagery();
        let generated = adventure(job.seed);
        let scene_key: SceneKey =
            serde_json::from_value(serde_json::Value::String(job.scene.clone()))
                .unwrap_or_else(|_| panic!("scene {}", job.scene));
        let scene = generated
            .pack
            .scenes
            .iter()
            .find(|scene| scene.key == scene_key)
            .expect("scene");
        let setting = scene.setting.as_ref().expect("setting");
        let place = PlaceFacts::new(&setting.name, &setting.short).expect("place facts");
        let request = match job.kind.as_str() {
            "expression" => {
                let wanted = match job.role.as_deref() {
                    Some("antagonist") => CastRole::Antagonist,
                    Some("contact") => CastRole::Contact,
                    _ => CastRole::Patron,
                };
                let member = scene
                    .cast
                    .iter()
                    .find(|member| member.role == wanted)
                    .expect("cast member");
                let facts = CharacterFacts {
                    role: match wanted {
                        CastRole::Patron => CharacterRole::Patron,
                        CastRole::Antagonist => CharacterRole::Antagonist,
                        CastRole::Contact => CharacterRole::Contact,
                        CastRole::Bystander => CharacterRole::Bystander,
                    },
                    life_stage: LifeStage::from_age(member.age),
                };
                let sprite = store.load(&member.sprite).expect("sprite");
                ImageRequest {
                    kind: ImageKind::CastExpression,
                    references: vec![sprite],
                    prompt: cast_expression(
                        rating,
                        facts,
                        pick(
                            &Expression::ALL,
                            Expression::label,
                            job.expression.as_deref().unwrap_or("neutral"),
                        ),
                        pick(
                            &HealthTier::ALL,
                            HealthTier::label,
                            job.tier.as_deref().unwrap_or("unhurt"),
                        ),
                        false,
                    )
                    .unwrap(),
                    output: OutputSpec::CUTOUT,
                }
            }
            "refresh" => {
                let look = Look {
                    mood: pick(
                        &Mood::ALL,
                        Mood::label,
                        job.mood.as_deref().unwrap_or("calm"),
                    ),
                    shot: pick(
                        &Shot::ALL,
                        Shot::label,
                        job.shot.as_deref().unwrap_or("wide_establishing"),
                    ),
                    lighting: pick(
                        &Lighting::ALL,
                        Lighting::label,
                        job.lighting.as_deref().unwrap_or("unchanged"),
                    ),
                };
                ImageRequest {
                    kind: ImageKind::SceneRefresh,
                    references: vec![store.load(&scene.asset).expect("backdrop")],
                    prompt: scene_refresh(rating, &place, look),
                    output: OutputSpec::BACKDROP,
                }
            }
            "close_up" => {
                let subject =
                    ArtifactSubject::vet(job.subject.as_deref().expect("subject"), &place)
                        .expect("subject passes the filter");
                close_up_request(
                    rating,
                    &place,
                    &subject,
                    store.load(&scene.asset).expect("backdrop"),
                )
            }
            other => panic!("kind {other}"),
        };
        let started = Instant::now();
        let before = selected.metering_port().totals();
        let result = service.render(request).await;
        let seconds = started.elapsed().as_secs_f32();
        let after = selected.metering_port().totals();
        match result {
            Ok(key) => {
                let Lookup::Image { bytes, format } = service.image(key.as_str()).await else {
                    panic!("cached");
                };
                let extension = match format {
                    ImageFormat::Png => "png",
                    ImageFormat::Jpeg => "jpg",
                    ImageFormat::Webp => "webp",
                };
                std::fs::write(
                    out.join("live").join(format!("{}.{extension}", job.name)),
                    &bytes,
                )
                .unwrap();
                println!(
                    "LIVE ok {} {:.1}s calls+{} cost+${:.4} bytes={}",
                    job.name,
                    seconds,
                    after.calls - before.calls,
                    (after.micro_usd - before.micro_usd) as f64 / 1e6,
                    bytes.len()
                );
            }
            Err(failure) => println!(
                "LIVE failed {} {:.1}s {}",
                job.name,
                seconds,
                failure.label()
            ),
        }
    }
    let (a, b) = (
        mature.metering_port().totals(),
        restrained.metering_port().totals(),
    );
    println!(
        "LIVE totals calls={} cost=${:.4}",
        a.calls + b.calls,
        (a.micro_usd + b.micro_usd) as f64 / 1e6
    );
    mature.close().await;
    restrained.close().await;
}

#[test]
#[ignore = "prints how each adventure's patron answers flattery and flirting"]
fn live_temperaments() {
    for seed in 0..60 {
        let generated = adventure(seed);
        for dossier in &generated.dossiers {
            if dossier.npc_id.starts_with("patron:") {
                println!(
                    "TEMPER seed={seed} npc={} start={:?} flattery={:?} flirting={:?} threats={:?}",
                    dossier.npc_id,
                    dossier.starting_attitude,
                    dossier.temperament.flattery.effect,
                    dossier.temperament.flirting.effect,
                    dossier.temperament.threats.effect,
                );
            }
        }
    }
}

#[test]
#[ignore = "prints candidate adventures for the operator"]
fn live_candidates() {
    for seed in 0..40 {
        let generated = adventure(seed);
        for scene in &generated.pack.scenes {
            let Some(setting) = &scene.setting else {
                continue;
            };
            let cast: Vec<String> = scene
                .cast
                .iter()
                .map(|member| format!("{:?}:{}:{:?}", member.role, member.sprite, member.age))
                .collect();
            println!(
                "CAND seed={seed} scene={:?} kind={:?} place={:?} asset={} cast={}",
                scene.key,
                setting.kind,
                setting.name,
                scene.asset,
                cast.join(",")
            );
        }
    }
}

/// One admitted opaque FLUX plate; the actual native host owns send, usage and publication.
async fn flux_frame(
    host: &super::super::image_test_support::ImageHostFixture,
    prompt: &AssembledPrompt,
    references: &[ReferenceImage],
) -> Vec<u8> {
    let key = host
        .imagery()
        .render(ImageRequest {
            kind: ImageKind::ScenePlate,
            prompt: prompt.clone(),
            references: references.to_vec(),
            output: OutputSpec::PLATE,
        })
        .await
        .expect("the admitted FLUX plate call succeeds");
    let Lookup::Image { bytes, format } = host.imagery().image(key.as_str()).await else {
        panic!("published FLUX plate")
    };
    assert_eq!(format, ImageFormat::Jpeg);
    bytes.to_vec()
}

async fn photo(
    host: &super::super::image_test_support::ImageHostFixture,
    repo: &Path,
    relative: &str,
    format: ImageFormat,
) -> ReferenceImage {
    let bytes = std::fs::read(repo.join(relative)).expect("a reference photograph");
    host.import_bytes(format, bytes.into())
        .await
        .expect("checked authored photograph")
        .1
}

/// Operator-run proof (spends three FLUX calls): a fresh merchant plate, then two refreshes that
/// change the shot, the focus and the pose. `DF_FAL_KEY` is read by the launcher from `.env`.
#[tokio::test]
#[ignore = "spends money against the real FLUX provider; run once by the operator"]
async fn flux_plate_sequence_keeps_the_merchant_cast_and_changes_the_shot() {
    std::env::var("DF_FAL_KEY").expect("DF_FAL_KEY");
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = repo.join("artifacts/tmp/plate-identity");
    std::fs::create_dir_all(&out).expect("the proof directory");
    use super::super::image_test_support::{ImageHostFixture, LiveImageOptIn};
    let mut patch = live_patch(df_imagery::BackendId::Fal, 300);
    let index = df_imagery::BackendId::Fal.index();
    patch.backend_endpoint[index] = Some(Some(
        df_imagery::ApiEndpoint::new("https://queue.fal.run").unwrap(),
    ));
    patch.backend_generation_model[index] = Some(Some(
        df_imagery::ModelName::new("fal-ai/flux-pro/v1.1-ultra").unwrap(),
    ));
    patch.backend_edit_model[index] = Some(Some(
        df_imagery::ModelName::new("fal-ai/flux-pro/kontext/max/multi").unwrap(),
    ));
    let host = ImageHostFixture::open_live(
        patch,
        live_costs(),
        df_imagery::ImageContentRating::Mature,
        LiveImageOptIn::FluxPlateSequence,
    )
    .await
    .expect("operator local PG bootstrap");

    let model: super::super::adventure::scene_model::SceneModel = serde_json::from_slice(
        include_bytes!("../../../../content/merchant-shop/scene-model.json"),
    )
    .expect("the merchant shop scene model");
    let facts = model.picture_state().expect("the scene picture state");
    let mut people: Vec<_> = model
        .people
        .iter()
        .filter(|person| ["merchant", "shop-soldier"].contains(&person.id.as_str()))
        .collect();
    people.sort_by(|left, right| left.id.cmp(&right.id));
    assert_eq!(people.len(), 2, "the merchant and Jessa");
    assert_eq!(people[0].name, "Marek", "the seed's merchant is Marek");
    assert_eq!(people[1].name, "Jessa", "the seed's bystander is Jessa");
    let sheet: Vec<FluxPerson> = people
        .iter()
        .map(|person| FluxPerson {
            name: person.name.clone(),
            appearance: Some(person.description.clone()),
        })
        .collect();

    let room = photo(
        &host,
        &repo,
        "artifacts/tmp/play-r6/room.jpg",
        ImageFormat::Jpeg,
    )
    .await;
    let marek = photo(
        &host,
        &repo,
        "artifacts/tmp/play-r6/marek.jpg",
        ImageFormat::Jpeg,
    )
    .await;
    let jessa = photo(
        &host,
        &repo,
        "assets/ui/generated/sprite-shop-soldier-documentary-photo.png",
        ImageFormat::Png,
    )
    .await;
    let (sheet_image, legend) = cast_sheet(
        host.imagery(),
        vec![
            CastIdentity {
                id: "npc:merchant".to_owned(),
                figure: marek.clone(),
            },
            CastIdentity {
                id: "npc:shop-soldier".to_owned(),
                figure: jessa.clone(),
            },
        ],
    )
    .await
    .expect("the identity sheet");
    assert!(legend.starts_with("slot 1 = npc:merchant"));
    let place = PlaceFacts::new("Marek's Cloth Shop", "").expect("the shop name passes vetting");

    // Frame 1 is the proof; frames 2 and 3 change the camera, the focus and the pose.
    let frames = [
        (
            "proof.jpg",
            super::super::shot::Camera::DialogueMedium,
            &sheet[0],
            &marek,
        ),
        (
            "refresh-2.jpg",
            super::super::shot::Camera::SpeakerClose,
            &sheet[1],
            &jessa,
        ),
        (
            "refresh-3.jpg",
            super::super::shot::Camera::WideEstablishing,
            &sheet[0],
            &marek,
        ),
    ];
    // `DF_PLATE_ONLY` names one frame to render again, so a retry spends one call.
    for (name, camera, speaker, portrait) in frames {
        if std::env::var("DF_PLATE_ONLY").is_ok_and(|only| only != name) {
            continue;
        }
        let cast = FluxCast {
            sheet: sheet.clone(),
            speaker: Some(speaker.clone()),
            portrait: true,
        };
        let prompt = flux_scene_plate(
            ContentRating::Mature,
            camera,
            Some(&place),
            &cast,
            &facts,
            None,
        )
        .expect("the FLUX plate prompt");
        std::fs::write(out.join(format!("{name}.prompt.txt")), prompt.as_str())
            .expect("the prompt is kept beside the image");
        let references = [room.clone(), sheet_image.clone(), portrait.clone()];
        let bytes = flux_frame(&host, &prompt, &references).await;
        std::fs::write(out.join(name), &bytes).expect("the frame is saved");
        println!(
            "PLATE_FRAME {name} bytes={} chars={}",
            bytes.len(),
            prompt.character_count()
        );
    }
    host.close().await;
}

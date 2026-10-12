//! Operator-run live checks of the image service against the real provider. Ignored by default:
//! they spend money. `DF_L01_PLAN` names a JSON plan, `DF_L01_OUT` the directory results are
//! copied to, and `OPENAI_API_KEY` the credential (read from the environment by the launcher
//! script, never from this file). The cache lives in `DF_L01_OUT/cache`, so a repeated plan
//! costs nothing.

use std::time::Instant;

use serde::Deserialize;

use super::{
    super::{
        npc_dossier_tests::adventure,
        pack::{CastRole, SceneKey},
        scene_mood::{Appearance, Expression, Lighting, Look, Mood, Shot},
        startup_configuration::{ImageryCredential, ImagerySettings},
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
    appearance: Option<String>,
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

fn service(rating: ContentRating, out: &Path) -> Imagery {
    let credential = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY");
    Imagery::start(
        ImagerySettings::On(ImageryConfiguration {
            credential: ImageryCredential::for_test(&credential),
            model: ImageEditModel::from_model_id(
                &std::env::var("DF_L01_MODEL").unwrap_or_else(|_| "gpt-image-2.5-flare".to_owned()),
            )
            .unwrap(),
            quality: match std::env::var("DF_L01_QUALITY").as_deref() {
                Ok("low") => ImageQuality::Low,
                Ok("high") => ImageQuality::High,
                _ => ImageQuality::Medium,
            },
            budget: 30,
            interval: Duration::from_secs(30),
            deadline: Duration::from_secs(90),
            cache_dir: out.join("cache"),
            rating,
        }),
        Some(out),
    )
    .unwrap()
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
    let mature = service(ContentRating::Mature, &out);
    let restrained = service(ContentRating::Restrained, &out);
    for job in jobs {
        let rating = match job.rating.as_deref() {
            Some("restrained") => ContentRating::Restrained,
            _ => ContentRating::Mature,
        };
        let service = if rating == ContentRating::Mature {
            &mature
        } else {
            &restrained
        };
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
                            &Appearance::ALL,
                            Appearance::label,
                            job.appearance.as_deref().unwrap_or("unhurt"),
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
        let before = service.stats();
        let result = service.render(request).await;
        let seconds = started.elapsed().as_secs_f32();
        let after = service.stats();
        match result {
            Ok(key) => {
                let Lookup::Image { bytes, format } = service.image(key.as_str()).await else {
                    panic!("cached");
                };
                let extension = if format == ImageFormat::Png {
                    "png"
                } else {
                    "jpg"
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
                    (after.micro_dollars - before.micro_dollars) as f64 / 1e6,
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
    let (a, b) = (mature.stats(), restrained.stats());
    println!(
        "LIVE totals calls={} cost=${:.4}",
        a.calls + b.calls,
        (a.micro_dollars + b.micro_dollars) as f64 / 1e6
    );
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

use super::ambience::{AmbientSpec, AmbientStatus};
use super::*;
use crate::gameplay::{
    startup_configuration::ApproachSettings,
    storyteller::tests::{FakeEndpoint, Reply},
};
use serde_json::json;
use std::collections::HashSet;
use tokio::sync::Notify;

#[derive(Default)]
struct PreparationHub {
    sets: Mutex<HashSet<String>>,
    layers: Mutex<HashSet<String>>,
    ready: Mutex<bool>,
    failed: Mutex<bool>,
}

impl MusicHub for PreparationHub {
    fn get_or_request(&self, set: &MusicSetSpec) -> MusicStatus {
        locked(&self.sets).insert(set.key());
        self.status(set)
    }

    fn status(&self, set: &MusicSetSpec) -> MusicStatus {
        if *locked(&self.failed) {
            MusicStatus::Failed("fixture_failed".into())
        } else if *locked(&self.ready) {
            MusicStatus::Ready(ReadyMusicSet {
                layer_paths: ["bed", "motif", "tension", "combat"]
                    .map(|layer| format!("/gameplay/music/{}/{layer}", set.key())),
                ambience_path: None,
            })
        } else {
            MusicStatus::Pending
        }
    }

    fn ambient_request(&self, spec: &AmbientSpec) -> AmbientStatus {
        locked(&self.layers).insert(spec.key());
        self.ambient_status(spec)
    }

    fn ambient_status(&self, spec: &AmbientSpec) -> AmbientStatus {
        if *locked(&self.failed) {
            AmbientStatus::Failed("fixture_failed".into())
        } else if *locked(&self.ready) {
            AmbientStatus::Ready(format!("/gameplay/music/{}/ambient", spec.key()))
        } else {
            AmbientStatus::Pending
        }
    }

    fn invalidate(&self, _: &MusicSetSpec) {}
    fn regenerate(&self, _: &MusicSetSpec) {}
}

fn scene() -> pack::Scene {
    let loaded = pack::load(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/merchant-shop"),
    )
    .unwrap();
    let mut scene = loaded
        .pack
        .scenes
        .into_iter()
        .find(|scene| scene.setting.is_some())
        .unwrap();
    scene.key = pack::SceneKey::Opening;
    let setting = scene.setting.as_mut().unwrap();
    setting.place_id = "hs36-prepared-quay".into();
    setting.name = "Dust Cellar".into();
    setting.short = "Public quay with water and gulls".into();
    setting.exterior = true;
    setting.model = Some(
        serde_json::from_value(json!({
            "inventory_version":1,"time_of_day":"evening","weather":"rain",
            "people":[{
                "id":"docker","name":"Public Dock Worker","role":"docker",
                "description":"A docker unloads a boat","relation_to_speaker":"none",
                "position":"pier","aliases":[],"is_background":true,
                "present_in":["opening"],"attitude":"indifferent","attitudes":{},
                "knowledge":[],"wants":"private-intention-sentinel"
            },{
                "id":"later","name":"future-person-sentinel","role":"guard",
                "description":"A guard waits","relation_to_speaker":"none",
                "position":"door","aliases":[],"is_background":true,
                "present_in":["combat"],"attitude":"indifferent","attitudes":{},
                "knowledge":[],"wants":"private-intention-sentinel"
            }],"things":[],"ways_out":[],"offers":[]
        }))
        .unwrap(),
    );
    scene.narration = "unrevealed-story-sentinel".into();
    scene
}

fn fixture(choices: &[&str]) -> (FakeEndpoint, PlaceMusic, Arc<PreparationHub>, Arc<Notify>) {
    let endpoint = FakeEndpoint::start(
        choices
            .iter()
            .map(|choice| {
                Reply::Status(
                    200,
                    json!({"answers":{"q1":{"type":"choice","choice":choice,"confidence":0.9}},
            "model":"jev-1.13.0","usage":{"input_tokens":10,"output_tokens":1}}),
                )
            })
            .collect(),
    );
    let jev = ApproachModel::configured(Some(ApproachSettings {
        credential: "jev-fixture-sentinel".into(),
        base_url: format!("http://{}", endpoint.address),
    }));
    let hub = Arc::new(PreparationHub::default());
    let changed = Arc::new(Notify::new());
    let signal = changed.clone();
    let music = PlaceMusic::new(
        jev,
        Some(hub.clone()),
        tokio::runtime::Handle::current(),
        move || {
            signal.notify_one();
        },
    );
    (endpoint, music, hub, changed)
}

async fn soundscape_settled(music: &PlaceMusic, changed: &Notify, place: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let notified = changed.notified();
            if music.ambient_report(place)["status"] == "decided" {
                return;
            }
            notified.await;
        }
    })
    .await
    .expect("prepared scene must finish without a display view");
}

#[tokio::test]
async fn prepared_scene_finishes_jev_and_hub_requests_before_any_arrival_view() {
    // Six score choices (the exterior is explicit), then reveal, water, busy, stop.
    let (endpoint, music, hub, changed) =
        fixture(&["o1", "o1", "o2", "o1", "o1", "o2", "o1", "o2", "o3", "o1"]);
    let scene = scene();
    music.set_band(PacingBand::Driving);
    let pending = music.prepare_scene(&scene).unwrap().unwrap();
    assert_eq!(pending.fallback_note, "choosing the music");
    soundscape_settled(&music, &changed, "hs36-prepared-quay").await;
    assert_eq!(
        locked(&hub.sets).len(),
        1,
        "no view needed to request the score"
    );
    assert_eq!(locked(&hub.layers).len(), 1);
    let report = music.ambient_report("hs36-prepared-quay");
    assert_eq!(report["layers"][0]["status"], "pending");
    assert_eq!(report["layers"][0]["kind"], "water");
    let key = report["layers"][0]["key"].clone();
    let bodies = endpoint.bodies();
    assert_eq!(bodies.len(), 10);
    for body in bodies {
        assert_eq!(body["state"]["place"]["weather"], "rain");
        assert_eq!(body["state"]["place"]["time_of_day"], "evening");
        assert!(body.to_string().contains("Public Dock Worker"));
        assert!(!body.to_string().contains("future-person-sentinel"));
        assert!(!body.to_string().contains("private-intention-sentinel"));
        assert!(!body.to_string().contains("unrevealed-story-sentinel"));
    }
    *locked(&hub.failed) = true;
    let failed = music.prepare_scene(&scene).unwrap().unwrap();
    assert!(failed.fallback_note.contains("fixture_failed"));
    assert_eq!(
        music.ambient_report("hs36-prepared-quay")["layers"][0]["status"],
        "failed"
    );
    *locked(&hub.failed) = false;
    *locked(&hub.ready) = true;
    let ready = music.prepare_scene(&scene).unwrap().unwrap();
    assert_eq!(ready.state, rpc::PlaceMusicState::Ready as i32);
    assert_eq!(ready.bpm, PacingBand::Driving.bpm());
    let report = music.ambient_report("hs36-prepared-quay");
    assert_eq!(report["layers"][0]["status"], "ready");
    assert_eq!(report["layers"][0]["key"], key);
    assert_eq!(locked(&hub.layers).len(), 1);
    assert_eq!(
        endpoint.bodies().len(),
        10,
        "prepared revisits reuse the judgement"
    );
    assert_eq!(music.band(), PacingBand::Driving);
    assert!(
        locked(&music.inner.state).audio_place.is_empty(),
        "prefetch never changes the active place"
    );
    let mut arrival = rpc::ViewMessage {
        audience: Some(rpc::view_message::Audience::Display(
            rpc::DisplayGameplayView {
                scene: Some(rpc::GameplayScene {
                    location_id: "hs36-prepared-quay".into(),
                    title: scene.title,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    for _ in 0..3 {
        music.apply(&mut arrival);
        let Some(rpc::view_message::Audience::Display(display)) = &arrival.audience else {
            panic!("arrival must stay a display view");
        };
        assert_eq!(
            display.place_music.as_ref().unwrap().spec_key,
            ready.spec_key
        );
        let audio = crate::music_score::place_audio_of(display.scene.as_ref().unwrap()).unwrap();
        assert_eq!(audio.layers[0].key, key.as_str().unwrap());
        assert_eq!(audio.layers[0].status, "ready");
    }
    assert_eq!(
        endpoint.bodies().len(),
        10,
        "arrival and lines reuse prepared choices"
    );
    assert_eq!(
        locked(&hub.layers).len(),
        1,
        "arrival preserves the prepared ambient key"
    );
}

#[tokio::test]
async fn authored_preparation_uses_the_same_score_key_and_only_judges_ambience() {
    let (endpoint, music, hub, changed) = fixture(&["o1", "o2", "o2", "o1"]);
    let loaded = pack::load(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/merchant-shop"),
    )
    .unwrap();
    let mut scene = scene();
    scene.setting.as_mut().unwrap().place_id = test_support::shop_block().place_id.clone();
    let plan = pack::with_test_pack(loaded, || music.prepare_scene(&scene))
        .unwrap()
        .unwrap();
    assert_eq!(plan.fallback_note, "generating");
    assert_eq!(
        plan.spec_key,
        MusicSetSpec::new(test_support::shop_block().spec().unwrap()).key()
    );
    soundscape_settled(&music, &changed, &scene.setting.as_ref().unwrap().place_id).await;
    assert_eq!(endpoint.bodies().len(), 4);
    assert_eq!(locked(&hub.sets).len(), 1);
    assert_eq!(locked(&hub.layers).len(), 1);
}

#[tokio::test]
async fn unavailable_preparation_reports_no_spec_without_requesting_fabricated_resources() {
    let hub = Arc::new(PreparationHub::default());
    let changed = Arc::new(Notify::new());
    let signal = changed.clone();
    let music = PlaceMusic::new(
        ApproachModel::default(),
        Some(hub.clone()),
        tokio::runtime::Handle::current(),
        move || {
            signal.notify_one();
        },
    );
    let scene = scene();
    music.prepare_scene(&scene).unwrap();
    tokio::time::timeout(Duration::from_secs(5), changed.notified())
        .await
        .unwrap();
    let plan = music.prepare_scene(&scene).unwrap().unwrap();
    assert!(plan.fallback_note.starts_with("no spec:"));
    assert!(locked(&hub.sets).is_empty());
    assert!(locked(&hub.layers).is_empty());
    assert_eq!(
        music.ambient_report("hs36-prepared-quay")["status"],
        "not_requested"
    );
    let mut no_place = scene;
    no_place.setting = None;
    assert!(music.prepare_scene(&no_place).unwrap().is_none());
}

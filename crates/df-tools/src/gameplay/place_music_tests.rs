//! The place-music service: where a spec comes from (content or Jev), what the plan says, and
//! the dev surface. Generation itself is tested in `place_music/generation_tests.rs`.

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::json;

use super::generator::MusicSetSpec;
use super::spec::{Mood, TimeOfDay};
use super::test_support::shop_block;
use super::*;
use crate::gameplay::startup_configuration::ApproachSettings;
use crate::gameplay::storyteller::tests::{FakeEndpoint, Reply};
use crate::music_score::{self, Bed, PlaceMusic as ClientPlace};

const SHOP: &str = "place-mareks-cloth-shop";

fn shop_pack() -> pack::LoadedPack {
    pack::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/merchant-shop"))
        .expect("the shipped merchant-shop pack loads")
}

fn view(location_id: &str, place: &str, asset: &str) -> rpc::ViewMessage {
    rpc::ViewMessage {
        revision: None,
        audience: Some(rpc::view_message::Audience::Display(
            rpc::DisplayGameplayView {
                scene: Some(rpc::GameplayScene {
                    location_id: location_id.to_owned(),
                    place: place.to_owned(),
                    title: place.to_owned(),
                    scene_asset: asset.to_owned(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )),
    }
}

fn plan_of(message: &rpc::ViewMessage) -> Option<rpc::PlaceMusicPlan> {
    match message.audience.as_ref() {
        Some(rpc::view_message::Audience::Display(display)) => display.place_music.clone(),
        _ => None,
    }
}

/// A hub stand-in that records what place music asks for and answers from a table. It
/// deliberately has no cache or generation: those belong to the real hub.
#[derive(Default)]
struct FakeHub {
    requested: Mutex<Vec<String>>,
    invalidated: Mutex<Vec<String>>,
    regenerated: Mutex<Vec<String>>,
    answers: Mutex<Option<MusicStatus>>,
}

impl FakeHub {
    fn answering(status: MusicStatus) -> Arc<Self> {
        let hub = Arc::new(Self::default());
        *hub.answers.lock().unwrap() = Some(status);
        hub
    }

    fn answer(&self) -> MusicStatus {
        self.answers
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(MusicStatus::Pending)
    }

    fn unique_requests(&self) -> usize {
        self.requested
            .lock()
            .unwrap()
            .iter()
            .collect::<HashSet<_>>()
            .len()
    }
}

impl MusicHub for FakeHub {
    fn get_or_request(&self, set: &MusicSetSpec) -> MusicStatus {
        self.requested.lock().unwrap().push(set.key());
        self.answer()
    }

    fn status(&self, _set: &MusicSetSpec) -> MusicStatus {
        self.answer()
    }

    fn invalidate(&self, set: &MusicSetSpec) {
        self.invalidated.lock().unwrap().push(set.key());
    }

    fn regenerate(&self, set: &MusicSetSpec) {
        self.regenerated.lock().unwrap().push(set.key());
    }
}

fn music_with(jev: ApproachModel, hub: &Arc<FakeHub>) -> PlaceMusic {
    let hub: Arc<dyn MusicHub> = hub.clone();
    PlaceMusic::new(jev, Some(hub), tokio::runtime::Handle::current(), || {})
}

fn fake_jev(replies: Vec<&str>) -> (FakeEndpoint, ApproachModel) {
    let endpoint = FakeEndpoint::start(
        replies
            .into_iter()
            .map(|choice| {
                Reply::Status(
                    200,
                    json!({"answers": {"q1": {"type": "choice", "choice": choice, "confidence": 0.9}},
                        "model": "jev-1.13.0", "usage": {"input_tokens": 10, "output_tokens": 1}}),
                )
            })
            .collect(),
    );
    let jev = ApproachModel::configured(Some(ApproachSettings {
        credential: "jev-fixture-sentinel".into(),
        base_url: format!("http://{}", endpoint.address),
    }));
    (endpoint, jev)
}

async fn until_decided(music: &PlaceMusic, place: &str) -> Result<SpecReport, NoSpec> {
    for _ in 0..200 {
        match music.spec_report(place) {
            Err(NoSpec::Deciding | NoSpec::UnknownPlace) => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            settled => return settled,
        }
    }
    music.spec_report(place)
}

// ---- spec from content ----

#[tokio::test(flavor = "current_thread")]
async fn the_merchant_shop_spec_comes_from_content_and_is_relaxed_daytime_and_interior() {
    let block = shop_block();
    let spec = block.spec().unwrap();
    assert_eq!(spec.label(), "relaxed low day interior market");
    assert!(spec.is_interior() && spec.is_daytime());
    assert_eq!(spec.fallback_bed(), Some(Bed::Market));
    assert_eq!(
        spec.key_name(),
        "G major",
        "a relaxed place gets a major key"
    );

    let hub = FakeHub::answering(MusicStatus::Pending);
    let music = music_with(ApproachModel::default(), &hub);
    let mut message = view(SHOP, "Marek's Cloth Shop", "scene-place-cloth-shop");
    pack::with_test_pack(shop_pack(), || music.apply(&mut message));
    let plan = plan_of(&message).expect("a plan for a known place");
    assert_eq!(plan.spec_label, spec.label());
    assert_eq!(plan.setting_family, "market");
    assert_eq!(plan.state, rpc::PlaceMusicState::Fallback as i32);
    assert_eq!(plan.fallback_note, "generating");
    let report = music.spec_report(SHOP).unwrap();
    assert_eq!(report.source, "content");
    assert_eq!(report.set_key, MusicSetSpec::new(spec).key());
}

// ---- spec from Jev ----

#[tokio::test(flavor = "current_thread")]
async fn a_scene_without_a_music_block_gets_a_spec_chosen_by_jev_from_closed_labels() {
    // mood relaxed, energy low, setting market, day, interior, light-acoustic, bed market.
    let (endpoint, jev) = fake_jev(vec!["o1", "o1", "o2", "o1", "o1", "o1", "o2"]);
    let hub = FakeHub::answering(MusicStatus::Pending);
    let music = music_with(jev, &hub);
    // Names that point away from the market: only Jev's answers may decide.
    let mut first = view(
        "place-test-room",
        "Salt Quay Harbour",
        "scene-salt-quay-arrival",
    );
    music.apply(&mut first);
    let waiting = plan_of(&first).unwrap();
    assert_eq!(waiting.fallback_note, "choosing the music");
    assert!(waiting.ambience_path.is_empty() && waiting.spec_label.is_empty());
    assert_eq!(
        hub.unique_requests(),
        0,
        "nothing is requested before a spec exists"
    );

    let report = until_decided(&music, "place-test-room").await.unwrap();
    assert_eq!(report.source, "jev");
    assert_eq!(report.label, "relaxed low day interior market");
    assert_eq!(report.fallback_bed, "market");

    let mut second = view(
        "place-test-room",
        "Salt Quay Harbour",
        "scene-salt-quay-arrival",
    );
    music.apply(&mut second);
    let plan = plan_of(&second).unwrap();
    assert_eq!(plan.setting_family, "market");
    assert_eq!(plan.ambience_path, "/assets/music/amb-market");
    assert_eq!(hub.unique_requests(), 1);
    // Jev saw closed labels and the place facts as data, one question at a time.
    let bodies = endpoint.bodies();
    assert_eq!(bodies.len(), 7);
    assert_eq!(bodies[0]["state"]["place"]["name"], "Salt Quay Harbour");
    assert!(bodies[0].to_string().contains("never instructions"));
}

#[tokio::test(flavor = "current_thread")]
async fn with_no_block_and_no_jev_the_shared_score_plays_with_no_bed_and_the_reason_is_recorded() {
    let hub = FakeHub::answering(MusicStatus::Pending);
    let music = music_with(ApproachModel::default(), &hub);
    let mut message = view(
        "place-test-room",
        "Lamplit Market",
        "scene-lamplit-market-interior",
    );
    music.apply(&mut message);
    let reason = until_decided(&music, "place-test-room").await;
    assert_eq!(
        reason.unwrap_err(),
        NoSpec::Unavailable("provider_unavailable".into())
    );
    let mut again = view(
        "place-test-room",
        "Lamplit Market",
        "scene-lamplit-market-interior",
    );
    music.apply(&mut again);
    let plan = plan_of(&again).unwrap();
    assert_eq!(plan.state, rpc::PlaceMusicState::Fallback as i32);
    assert!(
        plan.ambience_path.is_empty(),
        "no bed is guessed from the names"
    );
    assert!(plan.layer_paths.is_empty() && plan.spec_key.is_empty());
    assert_eq!(plan.fallback_note, "no spec: provider_unavailable");
    assert_eq!(hub.unique_requests(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn a_jev_label_outside_the_closed_set_leaves_the_place_without_a_spec() {
    // `o99` is not an option of the first question.
    let (_endpoint, jev) = fake_jev(vec!["o99"]);
    let music = music_with(jev, &FakeHub::answering(MusicStatus::Pending));
    music.apply(&mut view("place-test-room", "Anywhere", ""));
    assert_eq!(
        until_decided(&music, "place-test-room").await.unwrap_err(),
        NoSpec::Unavailable("unknown_choice".into())
    );
}

#[test]
fn bed_selection_never_inspects_asset_or_place_names() {
    // The decision code has no string matching of any kind on content. Only the production part
    // of each file is read; the tests may use names freely.
    let sources = [
        ("music_score.rs", include_str!("../music_score.rs")),
        ("place_music.rs", include_str!("place_music.rs")),
        ("spec.rs", include_str!("place_music/spec.rs")),
        ("generator.rs", include_str!("place_music/generator.rs")),
        ("hub.rs", include_str!("place_music/hub.rs")),
    ];
    for (name, source) in sources {
        let production = source.split("#[cfg(test)]").next().unwrap();
        for forbidden in [
            ".contains(\"",
            ".contains(&[",
            "to_lowercase",
            "to_ascii_lowercase",
            "fn bed_of",
            "fn bed_for_scene",
        ] {
            assert!(
                !production.contains(forbidden),
                "{name} must not use {forbidden} in the music decision"
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn the_fallback_bed_for_a_daytime_interior_shop_is_the_market_not_night() {
    let hub = FakeHub::answering(MusicStatus::Pending);
    let music = music_with(ApproachModel::default(), &hub);
    let mut message = view(SHOP, "Marek's Cloth Shop", "scene-place-cloth-shop");
    pack::with_test_pack(shop_pack(), || music.apply(&mut message));
    let plan = plan_of(&message).unwrap();
    assert_eq!(plan.ambience_path, "/assets/music/amb-market");
    assert_ne!(
        plan.ambience_path,
        format!("/assets/music/{}", Bed::Night.track())
    );
    // The display reads the same plan the same way.
    assert_eq!(
        music_score::place_music_of(Some(&plan)),
        ClientPlace::Fallback {
            ambience: Some("/assets/music/amb-market".to_owned())
        }
    );
}

// ---- the plan follows the hub ----

#[tokio::test(flavor = "current_thread")]
async fn the_plan_reports_what_the_hub_reports() {
    let ready = ReadyMusicSet {
        layer_paths: ["bed", "motif", "tension", "combat"]
            .map(|layer| format!("/gameplay/music/{}/{layer}", "ab".repeat(16))),
        ambience_path: Some(format!("/gameplay/music/{}/ambience", "ab".repeat(16))),
    };
    let cases = [
        (
            MusicStatus::Ready(ready.clone()),
            rpc::PlaceMusicState::Ready,
            "",
        ),
        (
            MusicStatus::Pending,
            rpc::PlaceMusicState::Fallback,
            "generating",
        ),
        (
            MusicStatus::Failed("provider_status_429".into()),
            rpc::PlaceMusicState::Fallback,
            "failed: provider_status_429",
        ),
        (
            MusicStatus::Fallback("over budget".into()),
            rpc::PlaceMusicState::Fallback,
            "fallback: over budget",
        ),
    ];
    for (status, state, note) in cases {
        let hub = FakeHub::answering(status);
        let music = music_with(ApproachModel::default(), &hub);
        let mut message = view(SHOP, "Marek's Cloth Shop", "");
        pack::with_test_pack(shop_pack(), || music.apply(&mut message));
        let plan = plan_of(&message).unwrap();
        assert_eq!(plan.state, state as i32, "{note}");
        assert_eq!(plan.fallback_note, note);
        assert_eq!(
            (plan.bpm, plan.loop_samples, plan.sample_rate),
            (84, 2_016_000, 44_100)
        );
        if state == rpc::PlaceMusicState::Ready {
            assert_eq!(plan.layer_paths.len(), 4);
            assert!(plan.ambience_path.ends_with("/ambience"));
            assert!(matches!(
                music_score::place_music_of(Some(&plan)),
                ClientPlace::Ready { .. }
            ));
        } else {
            assert!(plan.layer_paths.is_empty());
            assert_eq!(plan.ambience_path, "/assets/music/amb-market");
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn places_with_the_same_spec_ask_the_hub_for_one_set_and_views_only_ask_never_generate() {
    let hub = FakeHub::answering(MusicStatus::Pending);
    let music = music_with(ApproachModel::default(), &hub);
    let block = shop_block();
    for place in ["place-a", "place-b"] {
        music.override_spec(place, &block).unwrap();
        for _ in 0..3 {
            let mut message = view(place, "Somewhere", "");
            music.apply(&mut message);
            assert!(plan_of(&message).is_some());
        }
    }
    assert_eq!(hub.requested.lock().unwrap().len(), 6);
    assert_eq!(
        hub.unique_requests(),
        1,
        "one key, so the hub makes one request"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_player_view_carries_no_place_music() {
    let music = music_with(
        ApproachModel::default(),
        &FakeHub::answering(MusicStatus::Pending),
    );
    let mut message = rpc::ViewMessage {
        revision: None,
        audience: Some(rpc::view_message::Audience::Player(Default::default())),
    };
    music.apply(&mut message);
    assert!(plan_of(&message).is_none());
}

// ---- dev surface ----

#[tokio::test(flavor = "current_thread")]
async fn the_dev_surface_reads_spec_and_tracks_and_drives_the_hub() {
    let hub = FakeHub::answering(MusicStatus::Failed("provider_status_500".into()));
    let music = music_with(ApproachModel::default(), &hub);
    let mut message = view(SHOP, "Marek's Cloth Shop", "");
    pack::with_test_pack(shop_pack(), || music.apply(&mut message));
    let report = music.spec_report(SHOP).unwrap();
    assert_eq!(report.source, "content");
    assert_eq!(report.fallback_bed, "market");
    assert_eq!(report.spec["mood"], "relaxed");

    let tracks = music.track_status(SHOP).unwrap();
    assert_eq!(tracks.len(), 5);
    assert!(
        tracks
            .iter()
            .all(|t| t.status == "failed" && t.detail == "provider_status_500")
    );
    let places = music.places();
    assert_eq!(places.len(), 1);
    assert_eq!((places[0].source, places[0].status), ("content", "failed"));

    music.clear(SHOP).unwrap();
    music.regenerate(SHOP).unwrap();
    assert_eq!(
        music.request_now(SHOP).unwrap(),
        MusicStatus::Failed("provider_status_500".into())
    );
    assert_eq!(
        *hub.invalidated.lock().unwrap(),
        vec![report.set_key.clone()]
    );
    assert_eq!(
        *hub.regenerated.lock().unwrap(),
        vec![report.set_key.clone()]
    );

    assert_eq!(
        music.spec_report("place-unknown").unwrap_err(),
        NoSpec::UnknownPlace
    );

    // An override changes the spec (and so the set key) until cleared.
    let mut dusk = shop_block();
    dusk.time_of_day = TimeOfDay::Dusk;
    dusk.mood = Mood::Melancholy;
    music.override_spec(SHOP, &dusk).unwrap();
    let overridden = music.spec_report(SHOP).unwrap();
    assert_eq!(overridden.source, "override");
    assert_ne!(overridden.set_key, report.set_key);
    music.clear_override(SHOP);
    assert_eq!(music.spec_report(SHOP).unwrap().set_key, report.set_key);
    let mut unknown = shop_block();
    unknown.setting = "a market square".into();
    assert_eq!(
        music.override_spec(SHOP, &unknown),
        Err(SpecError::UnknownLabel("setting"))
    );
}

//! The frame is built once for each change and shared; it carries no kinds; the copy of the canvas
//! finds each element's problem as it goes; and the host's clock sleeps while nothing moves.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use df_table::canvas::{Canvas, CanvasEffect};
use df_table::element::{ElementId, FieldValue, KindName, Position};
use df_table::kind::KindRegistry;
use df_table::testkit::RecordingSink;

use crate::client::get;
use crate::command::split_words;
use crate::dev::run_dev;
use crate::test_support::{Running, STARTER_FILES, start};
use crate::view::{CanvasView, ViewError, frame_canvas, value_json};
use df_playground::frame::Frame;

async fn run(running: &Running, line: &str) {
    run_dev(running.port, &split_words(line).expect("the line splits"))
        .await
        .expect("the command runs");
}

#[tokio::test]
async fn one_change_count_is_built_once_and_every_reader_gets_the_same_bytes() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await;
    let first = running.state.frame().await.expect("builds");
    let second = running.state.frame().await.expect("builds");
    assert!(
        Arc::ptr_eq(&first.canvas, &second.canvas),
        "the second reader shares the first's canvas bytes"
    );
    run(&running, "spawn marker m2").await;
    let third = running.state.frame().await.expect("builds");
    assert!(
        !Arc::ptr_eq(&first.canvas, &third.canvas),
        "a change builds a new canvas"
    );
}

#[tokio::test]
async fn five_pages_asking_for_the_same_frame_are_sent_identical_bytes() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await;
    let mut bodies = Vec::new();
    for _ in 0..5 {
        bodies.push(
            get(running.port, "/playground/frame")
                .await
                .expect("answers")
                .body,
        );
    }
    assert!(bodies.windows(2).all(|pair| pair[0] == pair[1]));
}

#[tokio::test]
async fn a_frame_carries_the_revision_and_elements_but_not_the_kinds() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await;
    let body = get(running.port, "/playground/frame")
        .await
        .expect("answers")
        .body;
    let json: serde_json::Value = serde_json::from_str(&body).expect("json");
    let canvas = json
        .get("canvas")
        .and_then(|canvas| canvas.as_object())
        .expect("a canvas");
    let mut keys: Vec<&str> = canvas.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["elements", "revision"]);
    assert!(!body.contains("\"kinds\""));
    let frame: Frame = serde_json::from_str(&body).expect("a frame");
    assert_eq!(frame.canvas.elements.len(), 1);
    let full = get(running.port, "/canvas.json")
        .await
        .expect("answers")
        .body;
    assert!(
        full.contains("\"kinds\""),
        "canvas.json still lists the kinds"
    );
}

#[test]
fn every_element_of_a_copy_names_its_own_problem() {
    let mut canvas = Canvas::new(df_table::testkit::registry_from(&[
        df_table::testkit::CHARACTER_KIND,
        df_table::testkit::MARKER_KIND,
    ]));
    let sink = RecordingSink::default();
    for index in 0..300 {
        canvas
            .apply(
                CanvasEffect::Spawn {
                    id: ElementId::new(&format!("m{index}")).expect("valid id"),
                    kind: KindName::new("marker").expect("valid kind"),
                    position: Position::new(10.0, 10.0),
                    fields: BTreeMap::new(),
                },
                &sink,
            )
            .expect("spawns");
    }
    let healthy = frame_canvas(&canvas).expect("copies");
    assert!(
        healthy
            .elements
            .iter()
            .all(|element| element.problem.is_none())
    );
    canvas.replace_registry(KindRegistry::default(), &sink);
    let broken = CanvasView::from_canvas(&canvas).expect("copies");
    assert_eq!(broken.elements.len(), 300);
    assert!(
        broken.elements.iter().all(|element| element
            .problem
            .as_deref()
            .is_some_and(|text| text.contains("no kind"))),
        "each element carries its problem"
    );
    assert!(broken.kinds.is_empty());
}

#[test]
fn a_number_that_is_not_finite_is_not_written_as_null() {
    assert_eq!(value_json(&FieldValue::Number(f64::NAN)), None);
    assert_eq!(value_json(&FieldValue::Number(f64::INFINITY)), None);
    assert_eq!(
        value_json(&FieldValue::Number(2.5)),
        Some(serde_json::json!(2.5))
    );
    let error = ViewError::NotWritable {
        element: "pip".to_owned(),
        field: "hit_points".to_owned(),
    };
    assert!(error.to_string().contains("pip") && error.to_string().contains("hit_points"));
}

#[tokio::test]
async fn an_idle_clock_never_takes_the_canvas_lock() {
    let mut running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await;
    running.start_clock();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        running.state.tick_count(),
        0,
        "about twelve ticks would have run if the clock still woke 30 times a second"
    );
}

#[tokio::test]
async fn the_clock_wakes_for_a_travel_and_goes_back_to_sleep_when_it_arrives() {
    let mut running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1 x=10 y=10").await;
    running.start_clock();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(running.state.tick_count(), 0);
    run(&running, "travel m1 14 10 speed=40").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let ticks = running.state.tick_count();
    assert!(ticks > 0, "the travel woke the clock");
    assert!(!running.state.is_moving(), "and the element arrived");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        running.state.tick_count(),
        ticks,
        "an arrived canvas lets the clock sleep again"
    );
    run(&running, "travel m1 10 10 speed=40").await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        running.state.tick_count() > ticks,
        "a second travel wakes it again"
    );
}

async fn frame_now(running: &Running) -> Frame {
    let body = get(running.port, "/playground/frame")
        .await
        .expect("answers")
        .body;
    serde_json::from_str(&body).expect("a frame")
}

#[tokio::test]
async fn a_frame_carries_the_files_revision_and_a_changed_file_is_one_announced_change() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("theme.css", "body { color: red; }");
    let before = frame_now(&running).await;
    let files = running.state.playground().files().clone();
    assert_eq!(
        before.asset_revision,
        crate::playground::revision(&files).expect("reads")
    );
    assert_eq!(
        running.state.refresh_files_revision().await.expect("reads"),
        None,
        "files that did not change announce nothing"
    );
    running.write_asset("theme.css", "body { color: blue; margin: 0; }");
    let changed = running
        .state
        .refresh_files_revision()
        .await
        .expect("reads")
        .expect("a new revision");
    let after = frame_now(&running).await;
    assert_eq!(after.asset_revision, changed);
    assert!(
        after.frame > before.frame,
        "the change is announced, so a page waiting for a frame is answered"
    );
}

/// A scene to put the party's cards on.
const PARTY_FIXTURE: &str = r#"{
    "name": "Party",
    "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": []},
    "phone": {"hero": "Mira", "status": {"kind": "ok"}}
}"#;

#[tokio::test]
async fn a_hero_card_without_a_sheet_shows_no_hit_points_and_a_card_without_a_hero_keeps_its_own() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("fixtures/party.json", PARTY_FIXTURE);
    run(
        &running,
        "scene party add Mira character=hero-99 hp=10 max=12",
    )
    .await;
    run(&running, "scene party add Bram hp=7 max=9").await;
    let frame = frame_now(&running).await;
    let df_playground::overlay::Edit::Set(party) = &frame.controls.scene.party else {
        panic!("a party");
    };
    let mira = party
        .iter()
        .find(|member| member.name == "Mira")
        .expect("Mira");
    assert_eq!(mira.health(), None, "no sheet: no sample hit points");
    assert_eq!(mira.armor_class, None);
    assert_eq!(mira.portrait, None, "no pack: the card keeps its initial");
    let bram = party
        .iter()
        .find(|member| member.name == "Bram")
        .expect("Bram");
    assert_eq!(bram.health(), Some((7, 9)));
    // TODO(DEV-12): a hero with a legal build in the creation record shows its sheet's hit points
    // and Armor Class on the card.
}

#[tokio::test]
async fn the_party_heroes_are_read_again_on_the_watch_beat_and_a_difference_is_announced() {
    let running = start(&STARTER_FILES).await;
    assert!(
        !running.state.refresh_party_heroes().await,
        "no party: nothing to read"
    );
    running.write_asset("fixtures/party.json", PARTY_FIXTURE);
    run(&running, "scene party add Mira character=hero-99").await;
    let before = running.state.change_count();
    assert!(
        running.state.refresh_party_heroes().await,
        "the first reading is kept and announced"
    );
    assert!(running.state.change_count() > before);
    assert!(
        !running.state.refresh_party_heroes().await,
        "the same heroes announce nothing"
    );
    // TODO(DEV-12): a headshot made in the background shows on the card within the watch's beat.
}

#[tokio::test]
async fn a_frame_carrying_a_roll_reads_on_the_page_and_its_clear_too() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("fixtures/party.json", PARTY_FIXTURE);
    run(
        &running,
        concat!(
            r#"scene roll melee_attack "Melee attack" id=r1 dice=20:19:test "#,
            "mods=Strength:3,Proficiency:2 target=AC:15 outcome=hit hold=8000"
        ),
    )
    .await;
    let body = get(running.port, "/playground/frame")
        .await
        .expect("answers")
        .body;
    let read = df_playground::frame::parse_frame_reply(&body);
    let Ok(df_playground::frame::FrameReply::Frame(frame)) = read else {
        panic!("the page reads a frame with a roll: {read:?}\n{body}");
    };
    let df_playground::overlay::Edit::Set(roll) = &frame.controls.scene.roll else {
        panic!("a roll is set");
    };
    assert_eq!(roll.view.id, "r1");
    run(&running, "scene roll-clear").await;
    let frame = frame_now(&running).await;
    assert_eq!(
        frame.controls.scene.roll,
        df_playground::overlay::Edit::Clear
    );
}

#[tokio::test]
async fn a_scene_change_made_inside_a_long_request_wakes_the_frames_at_once() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("fixtures/party.json", PARTY_FIXTURE);
    let before = running.state.change_count();
    // A check sets the roll card in the middle of its request, long before the request ends.
    crate::playground::apply_scene_changes(
        &running.state,
        vec![df_playground::scene::SceneChange::Caption {
            speaker: None,
            text: "The die is cast.".to_owned(),
        }],
    )
    .await
    .expect("applies");
    assert!(
        running.state.change_count() > before,
        "the pages, which follow only the frames, are woken by the change itself"
    );
}

#[tokio::test]
async fn a_change_of_the_devices_answers_the_party_as_a_frame_does() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("fixtures/party.json", PARTY_FIXTURE);
    run(
        &running,
        "scene party add Mira character=hero-99 hp=10 max=12",
    )
    .await;
    let change =
        df_playground::controls::ControlsChange::Device(df_playground::controls::Device::Tv);
    let body = serde_json::to_string(&change).expect("writes");
    let response =
        crate::client::post_json(running.port, df_playground::path::CONTROLS_CHANGE, &body)
            .await
            .expect("answers");
    let answer: serde_json::Value = serde_json::from_str(&response.body).expect("json");
    let party = &answer["value"]["scene"]["party"]["set"];
    assert_eq!(party[0]["name"], "Mira");
    assert_eq!(
        party[0]["hit_points"],
        serde_json::Value::Null,
        "the answer the page draws from carries the hero's numbers, not the card's own: {answer}"
    );
}

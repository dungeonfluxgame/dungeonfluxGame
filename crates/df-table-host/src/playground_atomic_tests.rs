//! A change of the playground that has several parts is one change; a read changes nothing; a
//! scene change is applied only to the fixture it was checked against; and every scene change
//! writes one audit line that names what it did.

use df_playground::controls::{ControlsChange, Device, TransitionStyle};
use df_playground::fixture::Fixture;
use df_playground::scene::SceneChange;

use crate::client::{get, post_json};
use crate::command::split_words;
use crate::dev::{DevError, run_dev};
use crate::playground::{
    ControlsRefusal, PlaygroundFiles, PlaygroundState, SceneRefusal, reconcile_fixtures,
};
use crate::test_support::{Running, STARTER_FILES, start};
use crate::wire::{ControlsBatch, ControlsReply, EventsReply, Frame};

const SCENE: &str = r#"{
    "name": "A scene",
    "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": []},
    "phone": {"hero": "Pip", "status": {"kind": "ok"}}
}"#;

async fn run(running: &Running, line: &str) -> Result<String, DevError> {
    run_dev(running.port, &split_words(line).expect("the line splits")).await
}

async fn shown(running: &Running) -> ControlsReply {
    let response = get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
    serde_json::from_str(&response.body).expect("controls")
}

async fn log_lines(running: &Running) -> Vec<String> {
    let response = get(running.port, "/events.json").await.expect("answers");
    let reply: EventsReply = serde_json::from_str(&response.body).expect("events");
    reply.events.into_iter().map(|line| line.text).collect()
}

fn with_two_scenes(running: &Running) {
    running.write_asset("fixtures/quay.json", SCENE);
    running.write_asset("fixtures/tavern.json", SCENE);
}

#[tokio::test]
async fn go_to_a_scene_that_does_not_exist_with_a_transition_changes_nothing() {
    let running = start(&STARTER_FILES).await;
    with_two_scenes(&running);
    let before = shown(&running).await;
    let lines_before = log_lines(&running).await.len();
    let error = run(&running, "playground go castle transition=wipe ms=2000")
        .await
        .expect_err("refused");
    assert!(
        matches!(&error, DevError::Rejected(text) if text.contains("castle.json") && text.contains("quay.json")),
        "{error}"
    );
    let after = shown(&running).await;
    assert_eq!(after, before, "the transition was not applied either");
    assert_eq!(after.controls.transition.style, TransitionStyle::Fade);
    let lines = log_lines(&running).await;
    assert_eq!(
        lines.len(),
        lines_before + 1,
        "one reject line and nothing else"
    );
    assert!(
        lines
            .last()
            .is_some_and(|line| line.starts_with("PLAYGROUND reject cause=invalid_value"))
    );
    let text = run(&running, "playground").await.expect("shows");
    assert!(text.contains("transition: fade 700 ms"), "{text}");
}

#[tokio::test]
async fn a_go_is_one_numbered_change_with_one_audit_line_that_names_every_part() {
    let running = start(&STARTER_FILES).await;
    with_two_scenes(&running);
    let before = shown(&running).await.revision;
    run(&running, "playground go tavern transition=wipe ms=900")
        .await
        .expect("goes");
    let after = shown(&running).await;
    assert_eq!(after.revision, before + 1, "one change, not three");
    assert_eq!(after.controls.fixture.as_deref(), Some("tavern.json"));
    assert_eq!(after.controls.transition.duration_ms, 900);
    let expected = format!(
        "PLAYGROUND set transition_style=wipe transition_ms=900 fixture=tavern.json revision={}",
        after.revision
    );
    assert!(log_lines(&running).await.contains(&expected));
}

#[tokio::test]
async fn a_batch_with_one_refused_part_applies_none_of_it() {
    let running = start(&STARTER_FILES).await;
    with_two_scenes(&running);
    let before = shown(&running).await;
    let batch = ControlsBatch {
        changes: vec![
            ControlsChange::Device(Device::Tv),
            ControlsChange::Status("sleepy".to_owned()),
        ],
    };
    let response = post_json(
        running.port,
        "/dev/playground/controls/batch",
        &serde_json::to_string(&batch).expect("writes"),
    )
    .await
    .expect("answers");
    assert_eq!(response.status, 400);
    assert_eq!(shown(&running).await, before, "the device did not change");
}

#[tokio::test]
async fn a_cycle_step_that_cannot_reach_its_scene_leaves_that_step_unchanged() {
    let running = start(&STARTER_FILES).await;
    with_two_scenes(&running);
    let before = shown(&running).await;
    let error = run(
        &running,
        "playground cycle castle quay times=2 hold=0 transition=wipe ms=2000",
    )
    .await
    .expect_err("stops at the first step");
    assert!(error.to_string().contains("1: castle failed"), "{error}");
    assert_eq!(shown(&running).await, before);
}

#[tokio::test]
async fn reading_the_controls_changes_nothing_even_when_the_fixture_was_removed_behind_its_back() {
    let running = start(&STARTER_FILES).await;
    with_two_scenes(&running);
    run(&running, "playground go tavern").await.expect("goes");
    std::fs::remove_file(running.playground_path("assets/fixtures/tavern.json"))
        .expect("removes the file outside the host");
    let before = shown(&running).await;
    let lines_before = log_lines(&running).await;
    let changes_before = running.state.change_count();
    for _ in 0..3 {
        assert_eq!(shown(&running).await, before);
    }
    run(&running, "playground").await.expect("shows");
    run(&running, "fixtures").await.expect("lists");
    assert_eq!(shown(&running).await, before, "reads left it alone");
    assert_eq!(log_lines(&running).await, lines_before, "and wrote no line");
    assert_eq!(
        running.state.change_count(),
        changes_before,
        "and woke nobody"
    );
}

#[tokio::test]
async fn removing_the_fixture_shown_through_the_host_is_a_recorded_waking_change() {
    let running = start(&STARTER_FILES).await;
    with_two_scenes(&running);
    run(&running, "playground go tavern").await.expect("goes");
    let before = shown(&running).await;
    let waking = running.state.change_count();
    run(&running, "fixture remove tavern")
        .await
        .expect("removes");
    let after = shown(&running).await;
    assert_eq!(after.controls.fixture.as_deref(), Some("quay.json"));
    assert_eq!(after.revision, before.revision + 1);
    assert!(running.state.change_count() > waking, "pages are woken");
    let expected = format!(
        "PLAYGROUND reconcile fixture=quay.json revision={}",
        after.revision
    );
    assert!(log_lines(&running).await.contains(&expected));
}

#[tokio::test]
async fn the_frame_and_the_controls_agree_after_a_removal() {
    let running = start(&STARTER_FILES).await;
    with_two_scenes(&running);
    run(&running, "playground go tavern").await.expect("goes");
    run(&running, "fixture remove tavern")
        .await
        .expect("removes");
    let frame: Frame = serde_json::from_str(
        &get(running.port, "/playground/frame")
            .await
            .expect("answers")
            .body,
    )
    .expect("a frame");
    assert_eq!(frame.controls, shown(&running).await);
    assert_eq!(
        frame.controls.controls.fixture.as_deref(),
        Some("quay.json")
    );
}

#[tokio::test]
async fn removing_the_last_fixture_leaves_none_shown_and_writing_one_shows_it() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("fixtures/only.json", SCENE);
    assert_eq!(
        shown(&running).await.controls.fixture.as_deref(),
        Some("only.json")
    );
    run(&running, "fixture remove only").await.expect("removes");
    assert_eq!(shown(&running).await.controls.fixture, None);
    running.write_asset("fixtures/next.json", SCENE);
    assert_eq!(
        shown(&running).await.controls.fixture.as_deref(),
        Some("next.json")
    );
}

#[tokio::test]
async fn startup_shows_the_first_fixture_as_a_recorded_change() {
    let running = start(&STARTER_FILES).await;
    let directory = running.playground_path("assets/fixtures");
    std::fs::create_dir_all(&directory).expect("makes the directory");
    std::fs::write(directory.join("b.json"), SCENE).expect("writes");
    std::fs::write(directory.join("a.json"), SCENE).expect("writes");
    let reply = reconcile_fixtures(&running.state)
        .expect("lists")
        .expect("something was replaced");
    assert_eq!(reply.controls.fixture.as_deref(), Some("a.json"));
    assert!(
        log_lines(&running)
            .await
            .contains(&"PLAYGROUND reconcile fixture=a.json revision=1".to_owned())
    );
    assert_eq!(
        reconcile_fixtures(&running.state).expect("lists"),
        None,
        "nothing to do the second time"
    );
}

fn state_showing(names: &[&str]) -> (PlaygroundState, Vec<String>) {
    let state = PlaygroundState::new(
        PlaygroundFiles::new("assets".into(), "pkg".into()),
        "test-boot".to_owned(),
    );
    let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    state.reconcile(&names, |_| {});
    (state, names)
}

/// One change of the controls against the fixture names as read just now.
fn choose(
    state: &PlaygroundState,
    change: ControlsChange,
    names: &[String],
) -> Result<ControlsReply, ControlsRefusal> {
    state.change_all(vec![change], names, state.generation(), |_| {})
}

fn caption() -> SceneChange {
    SceneChange::Caption {
        speaker: None,
        text: "The door opens.".to_owned(),
    }
}

#[test]
fn a_scene_change_checked_against_one_fixture_is_refused_after_another_is_shown() {
    let (state, names) = state_showing(&["camera_demo.json", "scene_quay.json"]);
    let base = Fixture::parse(SCENE).expect("parses");
    let (shown, generation) = state.scene_target().expect("a fixture is shown");
    assert_eq!(shown, "camera_demo.json");
    choose(
        &state,
        ControlsChange::Fixture("scene_quay.json".to_owned()),
        &names,
    )
    .expect("another scene is chosen while the check was running");
    let revision = state.revision();
    assert_eq!(
        state.change_scene(caption(), &base, generation, |_| {}),
        Err(SceneRefusal::FixtureChanged)
    );
    assert_eq!(state.revision(), revision, "nothing was applied");
    assert_eq!(state.snapshot().scene, Default::default());
    let refusal = SceneRefusal::FixtureChanged;
    assert!(refusal.to_string().contains("try again"));
    let (_, fresh) = state.scene_target().expect("shown");
    assert!(state.change_scene(caption(), &base, fresh, |_| {}).is_ok());
}

#[test]
fn a_scene_change_is_refused_when_the_fixture_file_was_written_after_it_was_read() {
    let (state, _) = state_showing(&["a.json"]);
    let base = Fixture::parse(SCENE).expect("parses");
    let (_, generation) = state.scene_target().expect("shown");
    state.fixture_files_changed();
    assert_eq!(
        state.change_scene(caption(), &base, generation, |_| {}),
        Err(SceneRefusal::FixtureChanged)
    );
}

#[test]
fn choosing_a_fixture_that_is_not_there_is_a_typed_refusal_that_names_the_ones_that_are() {
    let (state, names) = state_showing(&["a.json", "b.json"]);
    let refusal =
        choose(&state, ControlsChange::Fixture("c.json".to_owned()), &names).expect_err("refused");
    assert_eq!(
        refusal,
        ControlsRefusal::NoSuchFixture {
            name: "c.json".to_owned(),
            available: names,
        }
    );
    assert!(refusal.to_string().contains("a.json, b.json"));
}

// The scene audit lines (O.14 camera rule 9).

const PICTURE_MEDIA: &str = "not read, only listed";

fn with_media(running: &Running) {
    running.write_asset("media/portrait.png", PICTURE_MEDIA);
    running.write_asset("media/bell.mp3", PICTURE_MEDIA);
}

const STAGE: &str = r#"{
    "name": "Stage",
    "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": [
        {"id": "finn", "kind": "avatar", "label": "Finn", "x": 20.0, "y": 20.0,
         "color": null, "shape": "circle", "width_units": 6.0, "height_units": 6.0}
    ]},
    "phone": {"hero": "Pip", "status": {"kind": "ok"}}
}"#;

fn scene_lines(lines: &[String]) -> Vec<&str> {
    lines
        .iter()
        .map(String::as_str)
        .filter(|line| {
            line.starts_with("PLAYGROUND ")
                && !line.contains(" set ")
                && !line.contains("reconcile")
                && !line.contains("fixture_write")
        })
        .collect()
}

#[tokio::test]
async fn camera_moves_write_their_target_zoom_duration_and_serial() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("fixtures/stage.json", STAGE);
    run(&running, "scene camera finn zoom=250 over=1500")
        .await
        .expect("moves");
    run(&running, "scene camera-reset over=1200")
        .await
        .expect("returns");
    let lines = log_lines(&running).await;
    let scene = scene_lines(&lines);
    let revision = shown(&running).await.revision;
    assert_eq!(
        scene,
        vec![
            format!(
                "PLAYGROUND camera element=finn zoom=250 over=1500 serial=1 revision={}",
                revision - 1
            )
            .as_str(),
            format!("PLAYGROUND camera_wide over=1200 serial=2 revision={revision}").as_str(),
        ]
    );
}

#[tokio::test]
async fn pictures_slides_and_sounds_write_the_media_slot_rotation_volume_and_cue() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("fixtures/stage.json", STAGE);
    with_media(&running);
    run(
        &running,
        "scene slide tv-card portrait.png from=15 to=85 over=5000 rotate=180",
    )
    .await
    .expect("slides");
    run(&running, "scene sound bell.mp3 on=tv volume=80")
        .await
        .expect("plays");
    run(&running, "scene music bell.mp3 volume=30 loop=false")
        .await
        .expect("plays");
    run(&running, "scene image-clear tv-card")
        .await
        .expect("clears");
    let lines = log_lines(&running).await;
    let scene = scene_lines(&lines);
    assert_eq!(scene.len(), 4, "{scene:?}");
    assert!(scene[0].starts_with(
        "PLAYGROUND image slot=tv-card media=portrait.png rotate=180 slide=15-85/5000 serial=1 revision="
    ), "{}", scene[0]);
    assert!(
        scene[1].starts_with("PLAYGROUND sound media=bell.mp3 volume=80 on=tv cue=1 revision="),
        "{}",
        scene[1]
    );
    assert!(
        scene[2]
            .starts_with("PLAYGROUND music media=bell.mp3 volume=30 on=both loop=false revision="),
        "{}",
        scene[2]
    );
    assert!(
        scene[3].starts_with("PLAYGROUND image_clear slot=tv-card revision="),
        "{}",
        scene[3]
    );
}

#[tokio::test]
async fn a_caption_line_names_its_speaker_and_length_but_never_its_text() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("fixtures/stage.json", STAGE);
    run(
        &running,
        "scene caption \"The door creaks open slowly\" speaker=\"Old Tom\"",
    )
    .await
    .expect("sets");
    let lines = log_lines(&running).await;
    let scene = scene_lines(&lines);
    assert_eq!(scene.len(), 1);
    assert!(
        scene[0].starts_with("PLAYGROUND caption speaker=\"Old Tom\" chars=27 revision="),
        "{}",
        scene[0]
    );
    assert!(!lines.iter().any(|line| line.contains("creaks")));
}

#[tokio::test]
async fn two_camera_moves_to_different_places_no_longer_write_the_same_line() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("fixtures/stage.json", STAGE);
    run(&running, "scene camera finn zoom=250 over=1500")
        .await
        .expect("moves");
    run(&running, "scene camera finn").await.expect("moves");
    let lines = log_lines(&running).await;
    let scene = scene_lines(&lines);
    assert_eq!(scene.len(), 2);
    assert_ne!(
        scene[0].replace("serial=1", ""),
        scene[1].replace("serial=2", "")
    );
    assert!(scene[0].contains("zoom=250") && scene[1].contains("zoom=200"));
}

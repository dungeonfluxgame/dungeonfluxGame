//! Moving between scenes from the command line: `playground go`, `playground cycle` and `wait`.

use std::time::{Duration, Instant};

use df_playground::controls::{ControlsChange, TransitionStyle};

use crate::client::get;
use crate::command::{
    Command, CommandError, DEFAULT_CYCLES, DEFAULT_HOLD_MS, parse, split_words, usage_text,
};
use crate::dev::{DevError, run_dev};
use crate::limits::{MAXIMUM_CYCLES, MAXIMUM_WAIT_MS};
use crate::test_support::{Running, STARTER_FILES, start};
use crate::wire::ControlsReply;

const SCENE: &str = r#"{
    "name": "A scene",
    "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": []},
    "phone": {"hero": "Pip", "status": {"kind": "ok"}}
}"#;

fn words(line: &str) -> Vec<String> {
    split_words(line).expect("the line splits")
}

fn go(line: &str) -> Vec<ControlsChange> {
    match parse(&words(line)).expect("parses") {
        Command::PlaygroundSet(changes) | Command::PlaygroundGo(changes) => changes,
        other => panic!("expected playground changes, got {other:?}"),
    }
}

#[test]
fn go_sets_the_transition_first_and_the_scene_last_whatever_order_they_were_typed_in() {
    assert_eq!(
        go("playground go tavern ms=900 transition=wipe"),
        vec![
            ControlsChange::TransitionStyle(TransitionStyle::Wipe),
            ControlsChange::TransitionMs(900),
            ControlsChange::Fixture("tavern".into()),
        ]
    );
    assert_eq!(
        go("playground go tavern"),
        vec![ControlsChange::Fixture("tavern".into())]
    );
}

#[test]
fn go_refuses_a_missing_name_an_unknown_key_a_repeat_and_a_bad_value() {
    assert!(matches!(
        parse(&words("playground go")),
        Err(CommandError::Usage(_))
    ));
    assert!(matches!(
        parse(&words("playground go a volume=3")),
        Err(CommandError::BadChoice { .. })
    ));
    assert_eq!(
        parse(&words("playground go a ms=1 ms=2")),
        Err(CommandError::DuplicateKey("ms".into()))
    );
    assert!(matches!(
        parse(&words("playground go a transition=spin")),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        parse(&words("playground go a ms=soon")),
        Err(CommandError::BadCount { .. })
    ));
}

#[test]
fn the_transition_can_also_be_set_without_going_anywhere() {
    assert_eq!(
        go("playground set transition=cut ms=0"),
        vec![
            ControlsChange::TransitionStyle(TransitionStyle::Cut),
            ControlsChange::TransitionMs(0)
        ]
    );
}

#[test]
fn cycle_names_two_scenes_and_has_defaults_for_rounds_and_pause() {
    let Ok(Command::PlaygroundCycle {
        first,
        second,
        times,
        hold_ms,
        transition,
    }) = parse(&words("playground cycle quay tavern"))
    else {
        panic!("expected a cycle");
    };
    assert_eq!((first.as_str(), second.as_str()), ("quay", "tavern"));
    assert_eq!((times, hold_ms), (DEFAULT_CYCLES, DEFAULT_HOLD_MS));
    assert!(transition.is_empty());
}

#[test]
fn cycle_reads_rounds_pause_and_transition() {
    let Ok(Command::PlaygroundCycle {
        times,
        hold_ms,
        transition,
        ..
    }) = parse(&words(
        "playground cycle a b times=5 hold=1500 transition=fade ms=400",
    ))
    else {
        panic!("expected a cycle");
    };
    assert_eq!((times, hold_ms), (5, 1500));
    assert_eq!(
        transition,
        vec![
            ControlsChange::TransitionStyle(TransitionStyle::Fade),
            ControlsChange::TransitionMs(400)
        ]
    );
}

#[test]
fn cycle_refuses_bad_rounds_pauses_and_keys() {
    for line in [
        "playground cycle a b times=0",
        &format!(
            "playground cycle a b times={}",
            u64::from(MAXIMUM_CYCLES) + 1
        ),
        "playground cycle a b times=many",
        &format!("playground cycle a b hold={}", MAXIMUM_WAIT_MS + 1),
        "playground cycle a b hold=-1",
        "playground cycle a b speed=3",
    ] {
        assert!(
            matches!(parse(&words(line)), Err(CommandError::BadChoice { .. })),
            "{line}"
        );
    }
    assert!(matches!(
        parse(&words("playground cycle a")),
        Err(CommandError::Usage(_))
    ));
}

#[test]
fn wait_takes_one_whole_number_of_milliseconds_within_the_limit() {
    assert_eq!(parse(&words("wait 250")), Ok(Command::Wait(250)));
    assert_eq!(parse(&words("wait 0")), Ok(Command::Wait(0)));
    assert!(matches!(parse(&words("wait")), Err(CommandError::Usage(_))));
    assert!(matches!(
        parse(&words("wait 1 2")),
        Err(CommandError::Usage(_))
    ));
    assert!(matches!(
        parse(&words("wait soon")),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        parse(&words(&format!("wait {}", MAXIMUM_WAIT_MS + 1))),
        Err(CommandError::BadChoice { .. })
    ));
}

#[test]
fn the_help_lists_go_cycle_wait_and_the_transition_keys() {
    let help = usage_text();
    for text in [
        "go <fixture>",
        "cycle <a> <b>",
        "wait <milliseconds>",
        "transition=cut|fade|wipe",
    ] {
        assert!(help.contains(text), "{text}");
    }
}

async fn run(running: &Running, line: &str) -> Result<String, DevError> {
    run_dev(running.port, &words(line)).await
}

async fn shown(running: &Running) -> ControlsReply {
    let response = get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
    serde_json::from_str(&response.body).expect("controls")
}

async fn with_two_scenes(running: &Running) {
    running.write_asset("fixtures/quay.json", SCENE);
    running.write_asset("fixtures/tavern.json", SCENE);
    get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
}

#[tokio::test]
async fn go_changes_the_scene_by_name_without_its_extension_and_sets_the_transition() {
    let running = start(&STARTER_FILES).await;
    with_two_scenes(&running).await;
    run(&running, "playground go tavern transition=wipe ms=900")
        .await
        .expect("goes");
    let controls = shown(&running).await.controls;
    assert_eq!(controls.fixture.as_deref(), Some("tavern.json"));
    assert_eq!(controls.transition.style, TransitionStyle::Wipe);
    assert_eq!(controls.transition.duration_ms, 900);
    let text = run(&running, "playground").await.expect("shows");
    assert!(text.contains("transition: wipe 900 ms"), "{text}");
}

#[tokio::test]
async fn going_to_a_scene_that_does_not_exist_says_which_do() {
    let running = start(&STARTER_FILES).await;
    with_two_scenes(&running).await;
    let error = run(&running, "playground go castle")
        .await
        .expect_err("refused");
    assert!(
        matches!(&error, DevError::Rejected(text) if text.contains("quay.json") && text.contains("tavern.json")),
        "{error}"
    );
}

#[tokio::test]
async fn a_transition_longer_than_the_limit_is_refused_before_anything_changes() {
    let running = start(&STARTER_FILES).await;
    with_two_scenes(&running).await;
    let before = shown(&running).await;
    let too_long = format!(
        "playground go tavern ms={}",
        df_playground::limits::MAXIMUM_TRANSITION_MS + 1
    );
    let error = run(&running, &too_long).await.expect_err("refused");
    assert!(
        matches!(&error, DevError::Command(CommandError::BadChoice { key, .. }) if key == "ms"),
        "{error}"
    );
    assert_eq!(shown(&running).await, before, "nothing was sent");
}

#[tokio::test]
async fn wait_pauses_for_about_the_time_asked() {
    let running = start(&STARTER_FILES).await;
    let started = Instant::now();
    assert_eq!(
        run(&running, "wait 60").await.expect("waits"),
        "waited 60 ms"
    );
    assert!(started.elapsed() >= Duration::from_millis(55));
}

#[tokio::test]
async fn a_cycle_goes_back_and_forth_and_ends_on_the_second_scene() {
    let running = start(&STARTER_FILES).await;
    with_two_scenes(&running).await;
    let before = shown(&running).await.revision;
    let text = run(
        &running,
        "playground cycle quay tavern times=3 hold=0 transition=fade ms=200",
    )
    .await
    .expect("cycles");
    let steps: Vec<&str> = text.lines().collect();
    assert_eq!(steps.len(), 7, "six moves and a summary: {text}");
    for (index, scene) in ["quay", "tavern", "quay", "tavern", "quay", "tavern"]
        .iter()
        .enumerate()
    {
        assert!(
            steps[index].starts_with(&format!("{}: {scene} ", index + 1)),
            "{}",
            steps[index]
        );
    }
    assert!(steps[6].contains("cycled 3 rounds between quay and tavern"));
    let controls = shown(&running).await;
    assert_eq!(controls.controls.fixture.as_deref(), Some("tavern.json"));
    assert!(
        controls.revision >= before + 6,
        "each move is a numbered change"
    );
}

#[tokio::test]
async fn a_cycle_pauses_between_scenes_but_not_after_the_last() {
    let running = start(&STARTER_FILES).await;
    with_two_scenes(&running).await;
    let started = Instant::now();
    run(&running, "playground cycle quay tavern times=1 hold=700")
        .await
        .expect("cycles");
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(650),
        "one hold of 700 ms between the two scenes: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(1_300),
        "and none after the last scene (it used to be two): {elapsed:?}"
    );
}

#[tokio::test]
async fn a_cycle_that_cannot_reach_a_scene_stops_there_and_says_which_step() {
    let running = start(&STARTER_FILES).await;
    with_two_scenes(&running).await;
    let error = run(&running, "playground cycle quay castle times=3 hold=0")
        .await
        .expect_err("stops");
    let DevError::Rejected(text) = error else {
        panic!("expected a rejection");
    };
    assert!(text.contains("1: quay ok"), "{text}");
    assert!(text.contains("2: castle failed"), "{text}");
    assert_eq!(
        text.lines().count(),
        2,
        "it stopped at the failing step: {text}"
    );
}

#[tokio::test]
async fn a_script_can_stage_a_scene_change_with_a_pause() {
    let running = start(&STARTER_FILES).await;
    with_two_scenes(&running).await;
    let directory = tempfile::TempDir::new().expect("makes a directory");
    let path = directory.path().join("stage.txt");
    std::fs::write(
        &path,
        "playground go quay\nwait 20\nplayground go tavern transition=cut\n",
    )
    .expect("writes");
    let words = ["script".to_owned(), path.to_string_lossy().into_owned()];
    let said = run_dev(running.port, &words).await.expect("runs");
    assert!(
        said.contains("2: waited 20 ms") && said.ends_with("ran 3 commands, all ok"),
        "{said}"
    );
    assert_eq!(
        shown(&running).await.controls.fixture.as_deref(),
        Some("tavern.json")
    );
}

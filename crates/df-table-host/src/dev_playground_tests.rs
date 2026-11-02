//! The `playground` and `fixture` commands, run the way a person types them.

use std::path::PathBuf;

use tempfile::TempDir;

use crate::command::split_words;
use crate::dev::{DevError, run_dev};
use crate::test_support::{Running, STARTER_FILES, start};

const FIXTURE: &str = r#"{
    "name": "One",
    "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": []},
    "phone": {"hero": "Pip", "status": {"kind": "ok"}}
}"#;

async fn run(running: &Running, line: &str) -> Result<String, DevError> {
    run_dev(running.port, &split_words(line).expect("the line splits")).await
}

fn file_with(text: &str) -> (TempDir, PathBuf) {
    let directory = TempDir::new().expect("makes a temporary directory");
    let path = directory.path().join("scene.json");
    std::fs::write(&path, text).expect("writes the file");
    (directory, path)
}

async fn write_fixture(running: &Running, name: &str) {
    let (_directory, path) = file_with(FIXTURE);
    let words = [
        "fixture".to_owned(),
        "write".to_owned(),
        name.to_owned(),
        path.to_string_lossy().into_owned(),
    ];
    run_dev(running.port, &words).await.expect("writes");
}

#[tokio::test]
async fn playground_shows_each_setting_and_the_fixtures() {
    let running = start(&STARTER_FILES).await;
    write_fixture(&running, "one").await;
    let text = run(&running, "playground").await.expect("shows");
    assert!(text.contains("fixture: one.json") && text.contains("fixtures: one.json"));
    assert!(text.contains("device: both") && text.contains("status: fixture"));
}

#[tokio::test]
async fn playground_set_changes_several_things_in_order() {
    let running = start(&STARTER_FILES).await;
    write_fixture(&running, "one").await;
    let said = run(
        &running,
        "playground set device=phone pace=slow status=rolling grid=on",
    )
    .await
    .expect("sets");
    assert!(
        said.contains("change 1: ok") && said.contains("change 4: ok"),
        "{said}"
    );
    let text = run(&running, "playground").await.expect("shows");
    for line in [
        "device: phone",
        "pace: slow",
        "status: rolling",
        "grid: true",
    ] {
        assert!(text.contains(line), "{line}");
    }
}

#[tokio::test]
async fn one_refused_setting_is_reported_and_the_others_still_apply() {
    let running = start(&STARTER_FILES).await;
    write_fixture(&running, "one").await;
    let error = run(&running, "playground set status=sleepy device=tv")
        .await
        .expect_err("one is refused");
    let DevError::Rejected(text) = error else {
        panic!("expected a rejection");
    };
    assert!(
        text.contains("change 1:") && text.contains("change 2: ok"),
        "{text}"
    );
    assert!(
        run(&running, "playground")
            .await
            .expect("shows")
            .contains("device: tv")
    );
}

#[tokio::test]
async fn a_fixture_is_written_from_a_file_listed_shown_and_removed() {
    let running = start(&STARTER_FILES).await;
    write_fixture(&running, "one").await;
    assert_eq!(run(&running, "fixtures").await.expect("lists"), "one.json");
    assert_eq!(
        run(&running, "fixture show one").await.expect("shows"),
        FIXTURE
    );
    run(&running, "fixture remove one").await.expect("removes");
    assert_eq!(run(&running, "fixtures").await.expect("lists"), "");
}

#[tokio::test]
async fn showing_or_removing_a_fixture_that_is_not_there_says_so() {
    let running = start(&STARTER_FILES).await;
    for line in ["fixture show ghost", "fixture remove ghost"] {
        let error = run(&running, line).await.expect_err("refused");
        assert!(
            matches!(&error, DevError::Rejected(text) if text.contains("ghost.json")),
            "{line}"
        );
    }
}

#[tokio::test]
async fn a_fixture_file_that_does_not_read_is_refused_with_its_position() {
    let running = start(&STARTER_FILES).await;
    let (_directory, path) = file_with(&FIXTURE.replace("\"hero\"", "\"heroo\""));
    let words = [
        "fixture".to_owned(),
        "write".to_owned(),
        "bad".to_owned(),
        path.to_string_lossy().into_owned(),
    ];
    let error = run_dev(running.port, &words).await.expect_err("refused");
    assert!(
        matches!(&error, DevError::Rejected(text) if text.contains("bad.json, line")),
        "{error}"
    );
    assert_eq!(run(&running, "fixtures").await.expect("lists"), "");
}

#[tokio::test]
async fn a_fixture_file_that_cannot_be_read_names_its_path() {
    let running = start(&STARTER_FILES).await;
    let words = [
        "fixture".to_owned(),
        "write".to_owned(),
        "x".to_owned(),
        "no/such/file.json".to_owned(),
    ];
    let error = run_dev(running.port, &words).await.expect_err("refused");
    assert!(matches!(&error, DevError::UnreadableFile { path, .. } if path.ends_with("file.json")));
}

#[tokio::test]
async fn a_script_can_set_up_the_playground_and_the_canvas_together() {
    let running = start(&STARTER_FILES).await;
    write_fixture(&running, "one").await;
    let (_directory, path) =
        file_with("# a scene\nplayground set device=tv\nspawn marker m1 x=10 y=10\n");
    let words = ["script".to_owned(), path.to_string_lossy().into_owned()];
    let said = run_dev(running.port, &words).await.expect("runs");
    assert!(said.ends_with("ran 2 commands, all ok"), "{said}");
    assert!(
        run(&running, "playground")
            .await
            .expect("shows")
            .contains("device: tv")
    );
    assert!(run(&running, "canvas").await.expect("reads").contains("m1"));
}

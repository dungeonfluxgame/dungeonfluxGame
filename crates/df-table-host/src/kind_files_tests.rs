use df_table::testkit::{CHARACTER_KIND, MARKER_KIND};
use tempfile::TempDir;

use crate::kind_files::{KindFileProblem, load_kind_files, load_kind_files_async};
use crate::limits::{MAXIMUM_KIND_FILE_BYTES, MAXIMUM_KIND_FILES};

fn directory_with(files: &[(&str, &str)]) -> TempDir {
    let directory = TempDir::new().expect("makes a temporary directory");
    for (name, text) in files {
        std::fs::write(directory.path().join(name), text).expect("writes a file");
    }
    directory
}

#[test]
fn every_json_file_in_the_directory_becomes_a_kind_and_other_files_are_ignored() {
    let directory = directory_with(&[
        ("character.json", CHARACTER_KIND),
        ("marker.json", MARKER_KIND),
        ("notes.txt", "not a kind"),
    ]);
    assert_eq!(load_kind_files(directory.path()).expect("loads").len(), 2);
}

#[test]
fn the_extension_is_matched_without_regard_to_case_and_a_directory_named_like_a_file_is_skipped() {
    let directory = directory_with(&[("MARKER.JSON", MARKER_KIND)]);
    std::fs::create_dir(directory.path().join("folder.json")).expect("makes a directory");
    assert_eq!(load_kind_files(directory.path()).expect("loads").len(), 1);
}

#[test]
fn a_file_that_is_not_valid_json_is_named_in_the_problems() {
    let directory = directory_with(&[("broken.json", "{ not json"), ("marker.json", MARKER_KIND)]);
    let problems = load_kind_files(directory.path()).expect_err("refused");
    assert_eq!(problems.len(), 1);
    assert!(matches!(&problems.0[0], KindFileProblem::Parse { file, .. } if file == "broken.json"));
}

#[test]
fn problems_in_several_files_are_all_reported_together() {
    let directory = directory_with(&[
        ("broken.json", "{ not json"),
        (
            "wall.json",
            r#"{"name": "wall", "shape": "rect", "width": 0, "height": 5}"#,
        ),
    ]);
    let problems = load_kind_files(directory.path()).expect_err("refused");
    assert_eq!(problems.len(), 2);
    assert!(
        problems
            .messages()
            .iter()
            .any(|message| message.contains("wall"))
    );
}

#[test]
fn a_kind_defined_in_two_files_is_reported() {
    let directory = directory_with(&[("a.json", MARKER_KIND), ("b.json", MARKER_KIND)]);
    let problems = load_kind_files(directory.path()).expect_err("refused");
    assert!(problems.messages()[0].contains("defined twice"));
}

#[test]
fn a_missing_directory_is_a_problem_not_a_panic() {
    let directory = TempDir::new().expect("makes a temporary directory");
    let problems = load_kind_files(&directory.path().join("absent")).expect_err("refused");
    assert!(matches!(
        problems.0[0],
        KindFileProblem::UnreadableDirectory { .. }
    ));
}

#[test]
fn a_file_larger_than_the_limit_is_refused_without_being_read() {
    let big = format!(
        r#"{{"name": "big", "shape": "rect", "width": 4, "height": 4, "description": "{}"}}"#,
        "x".repeat(usize::try_from(MAXIMUM_KIND_FILE_BYTES).expect("fits"))
    );
    let directory = directory_with(&[("big.json", &big)]);
    let problems = load_kind_files(directory.path()).expect_err("refused");
    assert!(matches!(
        problems.0[0],
        KindFileProblem::FileTooLarge { .. }
    ));
}

#[test]
fn more_files_than_the_limit_are_reported_and_only_the_limit_is_read() {
    let directory = TempDir::new().expect("makes a temporary directory");
    for index in 0..=MAXIMUM_KIND_FILES {
        let text = format!(r#"{{"name": "k{index}", "shape": "rect", "width": 2, "height": 2}}"#);
        std::fs::write(directory.path().join(format!("k{index:03}.json")), text).expect("writes");
    }
    let problems = load_kind_files(directory.path()).expect_err("refused");
    assert!(
        matches!(problems.0[0], KindFileProblem::TooManyFiles { found } if found == MAXIMUM_KIND_FILES + 1)
    );
}

#[tokio::test]
async fn the_async_loader_gives_the_same_answer_as_the_blocking_one() {
    let directory = directory_with(&[("marker.json", MARKER_KIND)]);
    let registry = load_kind_files_async(directory.path().to_path_buf())
        .await
        .expect("loads");
    assert_eq!(registry.len(), 1);
}

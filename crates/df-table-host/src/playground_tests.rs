use tempfile::TempDir;

use crate::client::get;
use crate::limits::MAXIMUM_ASSET_FILE_BYTES;
use crate::playground::{
    FileProblem, PlaygroundFiles, content_type, fixture_names, is_plain_file_name, read_file,
    revision,
};
use crate::test_support::{STARTER_FILES, raw_exchange, start};

fn files_in(directory: &TempDir) -> PlaygroundFiles {
    PlaygroundFiles::new(
        directory.path().join("assets"),
        directory.path().join("pkg"),
    )
}

fn write(directory: &TempDir, relative_path: &str, text: &str) {
    let path = directory.path().join(relative_path);
    std::fs::create_dir_all(path.parent().expect("has a parent")).expect("makes a directory");
    std::fs::write(path, text).expect("writes a file");
}

#[test]
fn only_plain_names_can_reach_the_disk() {
    for good in ["scene.json", "a-b_c.1.json", "df_playground_bg.wasm"] {
        assert!(is_plain_file_name(good), "{good}");
    }
    for bad in [
        "",
        ".hidden.json",
        "..",
        "a..b.json",
        "../x.json",
        "a/b.json",
        "a\\b.json",
        "a b.json",
        "c:x.json",
        "x\0.json",
        &"a".repeat(df_playground::media::MAXIMUM_NAME_LENGTH + 1),
    ] {
        assert!(!is_plain_file_name(bad), "{bad:?}");
    }
}

#[test]
fn a_file_is_served_only_with_an_allowed_extension() {
    let directory = TempDir::new().expect("makes a directory");
    write(&directory, "pkg/a.js", "x");
    write(&directory, "pkg/a.txt", "x");
    assert_eq!(
        read_file(&directory.path().join("pkg"), "a.js", &["js"]),
        Ok(b"x".to_vec())
    );
    assert_eq!(
        read_file(&directory.path().join("pkg"), "a.txt", &["js"]),
        Err(FileProblem::BadName)
    );
    assert_eq!(
        read_file(&directory.path().join("pkg"), "../a.js", &["js"]),
        Err(FileProblem::BadName)
    );
    assert_eq!(
        read_file(&directory.path().join("pkg"), "missing.js", &["js"]),
        Err(FileProblem::NotFound)
    );
}

#[test]
fn a_directory_named_like_a_file_is_not_served() {
    let directory = TempDir::new().expect("makes a directory");
    std::fs::create_dir_all(directory.path().join("folder.json")).expect("makes a directory");
    assert_eq!(
        read_file(directory.path(), "folder.json", &["json"]),
        Err(FileProblem::NotFound)
    );
}

#[test]
fn the_size_limit_is_stated_in_the_problem() {
    assert!(
        FileProblem::TooLarge {
            maximum_bytes: MAXIMUM_ASSET_FILE_BYTES
        }
        .message()
        .contains(&MAXIMUM_ASSET_FILE_BYTES.to_string())
    );
}

#[test]
fn each_problem_has_a_status_and_a_message() {
    for (problem, status) in [
        (FileProblem::BadName, 404),
        (FileProblem::NotFound, 404),
        (FileProblem::TooLarge { maximum_bytes: 1 }, 413),
        (FileProblem::Unreadable("denied".into()), 500),
    ] {
        assert_eq!(problem.status().as_u16(), status);
        assert!(!problem.message().is_empty());
    }
}

#[test]
fn media_types_follow_the_extension() {
    assert_eq!(content_type("a.wasm"), "application/wasm");
    assert_eq!(content_type("a.js"), "text/javascript; charset=utf-8");
    assert_eq!(content_type("a.css"), "text/css; charset=utf-8");
    assert_eq!(content_type("a.json"), "application/json");
    assert_eq!(content_type("a.html"), "text/html; charset=utf-8");
    assert_eq!(content_type("a.bin"), "application/octet-stream");
}

#[test]
fn fixture_names_are_the_json_files_sorted_and_a_missing_directory_has_none() {
    let directory = TempDir::new().expect("makes a directory");
    assert_eq!(fixture_names(&files_in(&directory)), Ok(Vec::new()));
    write(&directory, "assets/fixtures/b.json", "{}");
    write(&directory, "assets/fixtures/a.json", "{}");
    write(&directory, "assets/fixtures/notes.txt", "");
    write(&directory, "assets/fixtures/.hidden.json", "{}");
    assert_eq!(
        fixture_names(&files_in(&directory)),
        Ok(vec!["a.json".to_owned(), "b.json".to_owned()])
    );
}

#[test]
fn the_revision_changes_with_an_asset_and_with_the_package_independently() {
    let directory = TempDir::new().expect("makes a directory");
    write(&directory, "assets/theme.css", "a");
    write(&directory, "pkg/app.js", "a");
    let files = files_in(&directory);
    let before = revision(&files).expect("reads");
    assert_eq!(
        revision(&files).expect("reads"),
        before,
        "stable while nothing changes"
    );
    let (assets_before, package_before) = before.split_once('.').expect("has two parts");

    write(&directory, "assets/theme.css", "a longer theme");
    let (assets_after, package_after) = revision(&files)
        .expect("reads")
        .split_once('.')
        .map(|(a, p)| (a.to_owned(), p.to_owned()))
        .expect("has two parts");
    assert_ne!(assets_after, assets_before);
    assert_eq!(package_after, package_before);

    write(&directory, "pkg/app.js", "a longer package");
    let (assets_last, package_last) = revision(&files)
        .expect("reads")
        .split_once('.')
        .map(|(a, p)| (a.to_owned(), p.to_owned()))
        .expect("has two parts");
    assert_eq!(assets_last, assets_after);
    assert_ne!(package_last, package_after);
}

#[test]
fn a_new_fixture_changes_the_revision() {
    let directory = TempDir::new().expect("makes a directory");
    let files = files_in(&directory);
    let before = revision(&files).expect("reads");
    write(&directory, "assets/fixtures/a.json", "{}");
    assert_ne!(revision(&files).expect("reads"), before);
}

#[tokio::test]
async fn the_host_serves_the_page_theme_fixtures_and_package_with_their_media_types() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("index.html", "<!doctype html><p>page</p>");
    running.write_asset("theme.css", "body { color: red; }");
    running.write_asset("fixtures/one.json", "{\"name\": \"one\"}");
    running.write_package_file("df_playground.js", b"export default 1;");
    running.write_package_file("df_playground_bg.wasm", b"\0asm");

    for path in ["/playground", "/playground/"] {
        let page = get(running.port, path).await.expect("answers");
        assert_eq!(
            (page.status, page.body.as_str()),
            (200, "<!doctype html><p>page</p>")
        );
    }
    assert_eq!(
        get(running.port, "/playground/theme.css")
            .await
            .expect("answers")
            .body,
        "body { color: red; }"
    );
    assert_eq!(
        get(running.port, "/playground/fixtures.json")
            .await
            .expect("answers")
            .body,
        r#"["one.json"]"#
    );
    assert_eq!(
        get(running.port, "/playground/fixtures/one.json")
            .await
            .expect("answers")
            .body,
        "{\"name\": \"one\"}"
    );
    assert_eq!(
        get(running.port, "/playground/pkg/df_playground.js")
            .await
            .expect("answers")
            .status,
        200
    );

    let raw = raw_exchange(
        running.port,
        "GET /playground/pkg/df_playground_bg.wasm HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    )
    .await;
    let head = raw.to_ascii_lowercase();
    assert!(head.contains("content-type: application/wasm"), "{head}");
    assert!(head.contains("cache-control: no-store"), "{head}");
}

#[tokio::test]
async fn a_missing_page_is_a_404_that_says_where_to_look() {
    let running = start(&STARTER_FILES).await;
    let page = get(running.port, "/playground").await.expect("answers");
    assert_eq!(page.status, 404);
    assert!(page.body.contains("missing"));
}

#[tokio::test]
async fn names_that_try_to_leave_the_directory_are_refused() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("fixtures/one.json", "{}");
    for path in [
        "/playground/fixtures/..%2Findex.html",
        "/playground/fixtures/%2e%2e%2f%2e%2e%2fsecret.json",
        "/playground/pkg/..%5Cx.js",
        "/playground/fixtures/one.txt",
    ] {
        let answer = get(running.port, path).await.expect("answers");
        assert_eq!(answer.status, 404, "{path}");
    }
}

#[tokio::test]
async fn the_revision_route_answers_two_parts_and_follows_a_change() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("theme.css", "a");
    let before = get(running.port, "/playground/revision")
        .await
        .expect("answers")
        .body;
    assert!(before.contains('.'));
    running.write_asset("theme.css", "a longer theme");
    let after = get(running.port, "/playground/revision")
        .await
        .expect("answers")
        .body;
    assert_ne!(before, after);
}

#[tokio::test]
async fn the_playground_is_behind_the_same_host_check_as_the_rest() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("theme.css", "a");
    let answer = raw_exchange(
        running.port,
        "GET /playground/theme.css HTTP/1.1\r\nHost: evil.example\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(answer.starts_with("HTTP/1.1 403"));
}

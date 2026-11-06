use axum::http::StatusCode;
use tempfile::TempDir;

use crate::client::get;
use crate::limits::MAXIMUM_ASSET_FILE_BYTES;
use crate::playground::{
    FileError, PlaygroundFiles, content_type, fixture_names, is_plain_file_name, read_file,
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
        &"a".repeat(df_playground::limits::MAXIMUM_MEDIA_NAME_BYTES + 1),
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
        Err(FileError::InvalidName)
    );
    assert_eq!(
        read_file(&directory.path().join("pkg"), "../a.js", &["js"]),
        Err(FileError::InvalidName)
    );
    assert_eq!(
        read_file(&directory.path().join("pkg"), "missing.js", &["js"]),
        Err(FileError::NotFound)
    );
}

#[test]
fn a_directory_named_like_a_file_is_not_served() {
    let directory = TempDir::new().expect("makes a directory");
    std::fs::create_dir_all(directory.path().join("folder.json")).expect("makes a directory");
    assert_eq!(
        read_file(directory.path(), "folder.json", &["json"]),
        Err(FileError::NotFound)
    );
}

#[test]
fn the_size_limit_is_stated_in_the_problem() {
    assert!(
        FileError::TooLarge {
            maximum_bytes: MAXIMUM_ASSET_FILE_BYTES
        }
        .to_string()
        .contains(&MAXIMUM_ASSET_FILE_BYTES.to_string())
    );
}

#[test]
fn each_file_error_has_a_status_and_a_sentence() {
    for (error, status) in [
        (FileError::InvalidName, 404),
        (FileError::NotFound, 404),
        (FileError::TooLarge { maximum_bytes: 1 }, 413),
        (FileError::Io(std::io::ErrorKind::PermissionDenied), 500),
        (FileError::ModifiedBeforeEpoch, 500),
        (FileError::Stopped, 500),
    ] {
        assert_eq!(error.status().as_u16(), status);
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn an_io_error_keeps_its_kind_and_a_missing_file_is_not_found() {
    use std::io::{Error, ErrorKind};
    assert_eq!(
        FileError::from(Error::from(ErrorKind::NotFound)),
        FileError::NotFound
    );
    assert_eq!(
        FileError::from(Error::from(ErrorKind::PermissionDenied)),
        FileError::Io(ErrorKind::PermissionDenied)
    );
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
            (StatusCode::OK, "<!doctype html><p>page</p>")
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
        StatusCode::OK
    );

    let raw = raw_exchange(
        running.port,
        "GET /playground/pkg/df_playground_bg.wasm HTTP/1.1\r\n\
         Host: 127.0.0.1\r\nConnection: close\r\n\r\n",
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
    assert_eq!(page.status, StatusCode::NOT_FOUND);
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
        assert_eq!(answer.status, StatusCode::NOT_FOUND, "{path}");
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

#[tokio::test]
async fn a_fixture_body_that_is_not_json_is_refused_with_its_typed_cause() {
    use df_table::event::RejectionCause;

    use crate::wire::{Reply, path};
    let running = start(&STARTER_FILES).await;
    for route in [
        path::DEV_FIXTURE_WRITE,
        path::DEV_FIXTURE_REMOVE,
        path::DEV_MEDIA_REMOVE,
    ] {
        let response = crate::client::post_json(running.port, route, r#"{"name": 5}"#)
            .await
            .expect("answers");
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{route}");
        let reply: Reply = serde_json::from_str(&response.body).expect("a reply");
        let Reply::Rejected { cause, .. } = reply else {
            panic!("{route}: a body that is not the request is refused");
        };
        assert_eq!(cause, Some(RejectionCause::MalformedRequest), "{route}");
    }
    let rejects: Vec<String> = playground_lines(&running)
        .into_iter()
        .filter(|line| line.starts_with("PLAYGROUND reject "))
        .collect();
    assert_eq!(
        rejects,
        vec!["PLAYGROUND reject cause=malformed_request reason=data".to_owned(); 3]
    );
}

/// The playground's audit lines, oldest first.
fn playground_lines(running: &crate::test_support::Running) -> Vec<String> {
    running
        .state
        .log()
        .newest(1_000)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("PLAYGROUND "))
        .collect()
}

/// Posts `body` to `path` with `headers` (each ending in `\r\n`) and returns the status line.
async fn post_with(port: u16, path: &str, body: &str, headers: &str) -> String {
    let answer = raw_exchange(
        port,
        &crate::test_support::post_request(path, body, headers),
    )
    .await;
    answer.lines().next().unwrap_or("").to_owned()
}

#[test]
fn every_closed_key_reads_back_and_matches_its_serde_name() {
    use crate::playground::{BodyReadCause, ChangeSource, ServedFileKind};
    for kind in ServedFileKind::ALL {
        assert_eq!(ServedFileKind::from_key(kind.key()), Some(kind));
        assert_eq!(serde_json::to_value(kind).expect("writes"), kind.key());
    }
    for source in ChangeSource::ALL {
        assert_eq!(ChangeSource::from_key(source.key()), Some(source));
        assert_eq!(serde_json::to_value(source).expect("writes"), source.key());
    }
    for cause in BodyReadCause::ALL {
        assert_eq!(BodyReadCause::from_key(cause.key()), Some(cause));
        assert_eq!(serde_json::to_value(cause).expect("writes"), cause.key());
    }
}

#[test]
fn each_file_error_has_a_closed_key() {
    let keys: Vec<&str> = [
        FileError::InvalidName,
        FileError::NotFound,
        FileError::TooLarge { maximum_bytes: 1 },
        FileError::Io(std::io::ErrorKind::PermissionDenied),
        FileError::ModifiedBeforeEpoch,
        FileError::Stopped,
    ]
    .iter()
    .map(FileError::key)
    .collect();
    assert_eq!(
        keys,
        vec![
            "invalid_name",
            "not_found",
            "too_large",
            "io",
            "modified_before_epoch",
            "stopped"
        ]
    );
}

#[tokio::test]
async fn a_change_of_the_controls_names_who_asked_from_the_client_header() {
    let running = start(&STARTER_FILES).await;
    let body = r#"{"change": "show_grid", "value": true}"#;
    let off = r#"{"change": "show_grid", "value": false}"#;
    let path = "/dev/playground/controls";
    for (body, header) in [
        (body, "X-Df-Client: page-1a2b3c4d\r\n"),
        (off, "X-Df-Client: cli\r\n"),
        (body, "X-Df-Client: not a name\r\n"),
        (off, ""),
    ] {
        let status = post_with(running.port, path, body, header).await;
        assert!(status.starts_with("HTTP/1.1 200"), "{header}: {status}");
    }
    let origins: Vec<String> = playground_lines(&running)
        .into_iter()
        .filter(|line| line.starts_with("PLAYGROUND set show_grid="))
        .map(|line| {
            line.split_once(" source=")
                .expect("has a source")
                .1
                .to_owned()
        })
        .collect();
    assert_eq!(
        origins,
        vec![
            "page client=page-1a2b3c4d",
            "cli client=-",
            "- client=-",
            "- client=-"
        ]
    );
}

#[tokio::test]
async fn a_fixture_name_that_is_not_one_writes_exactly_one_reject_line() {
    let running = start(&STARTER_FILES).await;
    let status = post_with(
        running.port,
        "/dev/playground/controls",
        r#"{"change": "fixture", "value": "../escape"}"#,
        "X-Df-Client: cli\r\n",
    )
    .await;
    assert!(status.starts_with("HTTP/1.1 400"), "{status}");
    assert_eq!(
        playground_lines(&running),
        vec![
            "PLAYGROUND reject cause=invalid_value reason=invalid_fixture_name source=cli client=-"
                .to_owned()
        ]
    );
}

#[tokio::test]
async fn a_controls_body_that_is_not_json_names_its_body_cause() {
    let running = start(&STARTER_FILES).await;
    let status = post_with(running.port, "/dev/playground/controls", "{", "").await;
    assert!(status.starts_with("HTTP/1.1 400"), "{status}");
    assert_eq!(
        playground_lines(&running),
        vec![
            "PLAYGROUND reject cause=malformed_request reason=truncated source=- client=-"
                .to_owned()
        ]
    );
}

#[tokio::test]
async fn a_scene_change_with_no_fixture_shown_writes_exactly_one_reject_line() {
    let running = start(&STARTER_FILES).await;
    let status = post_with(
        running.port,
        "/dev/playground/scene",
        r#"{"change": "caption", "speaker": null, "text": "Hello"}"#,
        "",
    )
    .await;
    assert!(status.starts_with("HTTP/1.1 400"), "{status}");
    assert_eq!(
        playground_lines(&running),
        vec!["PLAYGROUND reject cause=invalid_value reason=no_fixture_shown".to_owned()]
    );
    let status = post_with(running.port, "/dev/playground/scene", "{", "").await;
    assert!(status.starts_with("HTTP/1.1 400"), "{status}");
    assert_eq!(
        playground_lines(&running).last(),
        Some(&"PLAYGROUND reject cause=malformed_request reason=truncated".to_owned())
    );
}

#[tokio::test]
async fn a_missing_package_file_writes_one_file_unreadable_line_and_a_missing_fixture_none() {
    let running = start(&STARTER_FILES).await;
    let package = get(running.port, "/playground/pkg/df_playground.js")
        .await
        .expect("answers");
    assert_eq!(package.status, StatusCode::NOT_FOUND);
    let fixture = get(running.port, "/playground/fixtures/ghost.json")
        .await
        .expect("answers");
    assert_eq!(fixture.status, StatusCode::NOT_FOUND);
    assert_eq!(
        playground_lines(&running),
        vec!["PLAYGROUND file_unreadable kind=package name=df_playground.js status=404".to_owned()]
    );
}

#[tokio::test]
async fn fixture_refusals_name_their_reason_and_a_write_its_duration() {
    let running = start(&STARTER_FILES).await;
    let remove = "/dev/playground/fixture/remove";
    let status = post_with(running.port, remove, r#"{"name": "ghost"}"#, "").await;
    assert!(status.starts_with("HTTP/1.1 400"), "{status}");
    let status = post_with(running.port, remove, r#"{"name": "../up"}"#, "").await;
    assert!(status.starts_with("HTTP/1.1 400"), "{status}");
    let write = r#"{"name": "../up", "text": "{}"}"#;
    let status = post_with(running.port, "/dev/playground/fixture", write, "").await;
    assert!(status.starts_with("HTTP/1.1 400"), "{status}");
    assert_eq!(
        playground_lines(&running),
        vec![
            "PLAYGROUND reject cause=invalid_value reason=no_such_fixture fixture=ghost.json"
                .to_owned(),
            "PLAYGROUND reject cause=invalid_value reason=invalid_fixture_name fixture=-"
                .to_owned(),
            "PLAYGROUND reject cause=invalid_value reason=invalid_fixture_name fixture=-"
                .to_owned(),
        ]
    );
    let fixture = r#"{"name": "One", "tv": {"place": "Here", "status": {"kind": "ok"},
        "elements": []}, "phone": {"hero": "Pip", "status": {"kind": "ok"}}}"#;
    let write = serde_json::json!({"name": "one", "text": fixture}).to_string();
    let status = post_with(running.port, "/dev/playground/fixture", &write, "").await;
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    let written: Vec<String> = playground_lines(&running)
        .into_iter()
        .filter(|line| line.starts_with("PLAYGROUND fixture_write "))
        .collect();
    assert_eq!(written.len(), 1, "{written:?}");
    let (head, duration) = written[0]
        .rsplit_once(" duration_ms=")
        .expect("has a duration");
    assert_eq!(
        head,
        format!(
            "PLAYGROUND fixture_write name=one.json bytes={}",
            fixture.len()
        )
    );
    assert!(duration.parse::<u64>().is_ok(), "{written:?}");
}

/// A fixture the page can show.
const ONE_FIXTURE: &str = r#"{"name": "One", "tv": {"place": "Here", "status": {"kind": "ok"},
    "elements": []}, "phone": {"hero": "Pip", "status": {"kind": "ok"}}}"#;

/// Puts a plain file where the playground keeps the directory `relative_path`, so reading that
/// directory fails with an error that is not "missing".
fn file_in_place_of_directory(running: &crate::test_support::Running, relative_path: &str) {
    let path = running.playground_path(relative_path);
    match std::fs::remove_dir_all(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("removes the directory: {error}"),
    }
    let parent = path.parent().expect("has a parent");
    std::fs::create_dir_all(parent).expect("makes the parent directory");
    std::fs::write(&path, "not a directory").expect("writes a file in its place");
}

const CAPTION: &str = r#"{"change": "caption", "speaker": null, "text": "Hello"}"#;

#[tokio::test]
async fn a_scene_change_on_a_fixture_removed_behind_the_hosts_back_names_the_fixture() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("fixtures/one.json", ONE_FIXTURE);
    std::fs::remove_file(running.playground_path("assets/fixtures/one.json"))
        .expect("removes the file outside the host");
    let status = post_with(running.port, "/dev/playground/scene", CAPTION, "").await;
    assert!(status.starts_with("HTTP/1.1 404"), "{status}");
    assert_eq!(
        playground_lines(&running).last(),
        Some(
            &"PLAYGROUND reject cause=file_error reason=fixture_unreadable fixture=one.json"
                .to_owned()
        )
    );
}

#[tokio::test]
async fn a_media_library_that_cannot_be_read_is_recorded_by_the_listing_and_a_scene_change() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("fixtures/one.json", ONE_FIXTURE);
    file_in_place_of_directory(&running, "assets/media");
    let listing = get(running.port, "/playground/media.json")
        .await
        .expect("answers");
    assert_eq!(listing.status, StatusCode::INTERNAL_SERVER_ERROR);
    let status = post_with(running.port, "/dev/playground/scene", CAPTION, "").await;
    assert!(status.starts_with("HTTP/1.1 500"), "{status}");
    let lines = playground_lines(&running);
    assert_eq!(
        lines[lines.len() - 2..],
        [
            "PLAYGROUND media_unreadable name=- reason=io".to_owned(),
            "PLAYGROUND reject cause=file_error reason=media_unreadable fixture=one.json"
                .to_owned(),
        ]
    );
}

#[tokio::test]
async fn a_fixture_directory_that_cannot_be_read_is_recorded_by_each_route_that_reads_it() {
    use crate::wire::path;
    let running = start(&STARTER_FILES).await;
    file_in_place_of_directory(&running, "assets/fixtures");
    let body = r#"{"change": "show_grid", "value": true}"#;
    let status = post_with(running.port, "/dev/playground/controls", body, "").await;
    assert!(status.starts_with("HTTP/1.1 500"), "{status}");
    for route in [path::PLAYGROUND_FIXTURE_LIST, path::PLAYGROUND_REVISION] {
        let answer = get(running.port, route).await.expect("answers");
        assert_eq!(answer.status, StatusCode::INTERNAL_SERVER_ERROR, "{route}");
    }
    let write = serde_json::json!({"name": "one", "text": ONE_FIXTURE}).to_string();
    let status = post_with(running.port, "/dev/playground/fixture", &write, "").await;
    assert!(status.starts_with("HTTP/1.1 "), "{status}");
    let lines = playground_lines(&running);
    assert_eq!(
        lines[..3],
        [
            "PLAYGROUND reject cause=file_error reason=io source=- client=-".to_owned(),
            "PLAYGROUND file_unreadable kind=fixture_list name=- status=500".to_owned(),
            "PLAYGROUND file_unreadable kind=revision name=- status=500".to_owned(),
        ],
        "{lines:?}"
    );
    // Which error the write meets depends on the system (a file where its directory should be).
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(
        lines[3].starts_with("PLAYGROUND reject cause=file_error reason=")
            && lines[3].ends_with(" fixture=one.json"),
        "{lines:?}"
    );
}

#[tokio::test]
async fn a_fixture_text_that_is_not_a_fixture_names_its_reason() {
    let running = start(&STARTER_FILES).await;
    let write = r#"{"name": "bad", "text": "not a fixture"}"#;
    let status = post_with(running.port, "/dev/playground/fixture", write, "").await;
    assert!(status.starts_with("HTTP/1.1 400"), "{status}");
    assert_eq!(
        playground_lines(&running),
        vec![
            "PLAYGROUND reject cause=invalid_value reason=invalid_fixture_text fixture=bad.json"
                .to_owned()
        ]
    );
}

#[tokio::test]
async fn a_controls_body_that_breaks_off_is_recorded_as_unreadable() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let running = start(&STARTER_FILES).await;
    // The body declares more bytes than are sent before the sender stops writing.
    let request = "POST /dev/playground/controls HTTP/1.1\r\nHost: 127.0.0.1\r\n\
                   Content-Type: application/json\r\nContent-Length: 100\r\n\
                   Connection: close\r\n\r\n{\"change\"";
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", running.port))
        .await
        .expect("connects");
    stream.write_all(request.as_bytes()).await.expect("sends");
    stream.shutdown().await.expect("stops writing");
    let mut answer = Vec::new();
    // The host may answer or close; either way the refusal was recorded first.
    let _ = stream.read_to_end(&mut answer).await;
    let expected =
        "PLAYGROUND reject cause=malformed_request reason=unreadable source=- client=-".to_owned();
    for _ in 0..200 {
        if playground_lines(&running).contains(&expected) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("no unreadable line: {:?}", playground_lines(&running));
}

use axum::http::StatusCode;
use df_scene_layout::{PictureBinding, RegionGeometry, SceneLayout, SceneRegion};

use super::request::{LocateError, LocateReply, StoredLayout};
use super::routes::path;
use crate::dev_client;
use crate::dev_http::{BodyCause, DevReply};
use crate::test_support::{STARTER_FILES, start};

fn one_region_layout(picture: &str, bytes: &[u8]) -> SceneLayout {
    let binding = PictureBinding {
        name: picture.to_owned(),
        sha256: df_imagery::oneshot_segment::picture_sha256(bytes),
        width_px: 100,
        height_px: 50,
    };
    let geometry = RegionGeometry {
        left_permille: 300,
        top_permille: 200,
        right_permille: 600,
        bottom_permille: 1000,
        head_x_permille: 450,
        head_y_permille: 200,
    };
    let elf = SceneRegion::new(df_scene_layout::region_id(b"elf"), geometry, 900)
        .unwrap()
        .with_label(Some("elf".to_owned()));
    SceneLayout::new(binding, "sam-3.1", "person", vec![elf])
}

fn rejected<T: std::fmt::Debug>(reply: LocateReply<T>) -> LocateError {
    match reply {
        DevReply::Rejected { error, message } => {
            assert_eq!(message, error.to_string());
            error
        }
        DevReply::Done { value } => panic!("not refused: {value:?}"),
    }
}

#[tokio::test]
async fn show_is_a_get_whose_answer_is_the_envelope_with_a_typed_error() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("media/plate.png", "plate");
    let layout = one_region_layout("plate.png", b"plate");
    let layouts = running.playground_path("assets/layouts");
    crate::locate::write_layout(&layouts, "plate.png", &layout).unwrap();
    let port = running.port;
    let show = |query: &'static str| async move {
        let address = format!("{}{query}", path::LOCATE_SHOW);
        dev_client::get_json::<LocateReply<StoredLayout>>(port, &address)
            .await
            .unwrap()
    };

    let DevReply::Done { value } = show("?picture=plate.png").await else {
        panic!("the stored layout is shown");
    };
    assert_eq!(value.layout, layout);
    assert_eq!(value.layout_file, "plate.png.layout.json");

    assert_eq!(
        rejected(show("?picture=other.png").await),
        LocateError::NoLayout {
            picture: "other.png".to_owned()
        }
    );
    assert_eq!(
        rejected(show("?picture=a%20b.png").await),
        LocateError::InvalidPictureName {
            name: "a b.png".to_owned()
        }
    );
    assert!(matches!(
        rejected(show("?picture=plate.png&extra=1").await),
        LocateError::InvalidQuery { .. }
    ));
    assert!(matches!(
        rejected(show("").await),
        LocateError::InvalidQuery { .. }
    ));
    let response = dev_client::get(port, &format!("{}?picture=x.png", path::LOCATE_SHOW))
        .await
        .unwrap();
    assert_eq!(response.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_label_with_an_unknown_field_is_an_invalid_body_and_one_audit_line() {
    let running = start(&STARTER_FILES).await;
    let port = running.port;
    let reply: LocateReply<StoredLayout> = dev_client::post_json(
        port,
        path::LOCATE_LABEL,
        &serde_json::json!({"picture": "plate.png", "region": "0", "colour": "red"}),
    )
    .await
    .unwrap();
    let LocateError::InvalidBody(body) = rejected(reply) else {
        panic!("an unknown field is a body error");
    };
    assert_eq!(body.cause, BodyCause::Data);
    let refusals: Vec<String> = running
        .state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("LOCATE refused"))
        .collect();
    assert_eq!(
        refusals,
        vec!["LOCATE refused verb=label picture=- error=invalid_body".to_owned()]
    );
}

#[tokio::test]
async fn the_cli_shows_a_layout_and_reports_a_refusal() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("media/plate.png", "plate");
    let layouts = running.playground_path("assets/layouts");
    crate::locate::write_layout(
        &layouts,
        "plate.png",
        &one_region_layout("plate.png", b"plate"),
    )
    .unwrap();
    let words =
        |text: &str| -> Vec<String> { text.split_whitespace().map(str::to_owned).collect() };
    let shown = crate::dev::run_dev(running.port, &words("media locate show plate.png"))
        .await
        .unwrap();
    assert!(shown.contains("0: elf"), "{shown}");
    let points = crate::dev::run_dev(running.port, &words("media locate points plate.png"))
        .await
        .unwrap();
    assert!(points.starts_with("elf:"), "{points}");
    let refused = crate::dev::run_dev(running.port, &words("media locate show missing.png"))
        .await
        .unwrap_err();
    assert!(
        matches!(
            &refused,
            crate::dev::DevError::Rejected {
                error: crate::dev::ReplyError::Locate(LocateError::NoLayout { picture }),
                ..
            } if picture == "missing.png"
        ),
        "{refused:?}"
    );
}

#[tokio::test]
async fn a_scan_whose_request_is_dropped_still_runs_to_the_end() {
    let (release, released) = tokio::sync::oneshot::channel::<()>();
    let (stored, stored_seen) = tokio::sync::oneshot::channel::<&str>();
    let scan = super::routes::run_scan_task(async move {
        released.await.expect("the test releases the scan");
        stored.send("layout stored").expect("the test waits for it");
        Ok::<_, super::request::LocateStageError>(())
    });
    // One poll starts the scan; the elapsed deadline then drops the request, as a client that
    // leaves does.
    let dropped = tokio::time::timeout(std::time::Duration::ZERO, scan).await;
    assert!(
        dropped.is_err(),
        "the scan was still running when its request was dropped"
    );
    release.send(()).expect("the scan is still waiting");
    assert_eq!(stored_seen.await, Ok("layout stored"));
}

fn locate_lines(running: &crate::test_support::Running) -> Vec<String> {
    running
        .state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("LOCATE"))
        .collect()
}

/// A real 100 by 50 PNG: the mask of one block, as the imagery API paints it.
fn png_picture() -> Vec<u8> {
    use base64::Engine as _;
    use df_imagery::oneshot_segment::{AnalysisMask, AnalysisMaskRun, AnalysisMaskSet};
    let set = AnalysisMaskSet {
        prompt: df_imagery::oneshot_segment::SegmentationPrompt::Person,
        width_px: 100,
        height_px: 50,
        instances: vec![AnalysisMask {
            index: 0,
            score: df_imagery::oneshot_segment::AnalysisUnit::new(0.9).unwrap(),
            runs: (10..50)
                .map(|row| AnalysisMaskRun {
                    start: row * 100 + 30,
                    length: 30,
                })
                .collect(),
        }],
    };
    let url = df_imagery::oneshot_segment::mask_data_url(&set, &set.instances[0]).unwrap();
    let encoded = url.strip_prefix("data:image/png;base64,").unwrap();
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap()
}

#[tokio::test]
async fn a_scan_that_fails_after_it_began_is_one_failed_line_with_its_stage_and_time() {
    let running = start(&STARTER_FILES).await;
    let reply: LocateReply<super::request::LocatedLayout> = dev_client::post_json(
        running.port,
        path::LOCATE,
        &serde_json::json!({"picture": "absent.png"}),
    )
    .await
    .unwrap();
    assert_eq!(
        rejected(reply),
        LocateError::PictureMissing {
            picture: "absent.png".to_owned()
        }
    );
    let lines = locate_lines(&running);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with("LOCATE started picture=absent.png "));
    assert!(
        lines[1].starts_with(concat!(
            "LOCATE failed verb=scan picture=absent.png error=picture_missing cause=- ",
            "stage=scan duration_ms="
        )),
        "{lines:?}"
    );
    assert!(!lines[1].contains("operation="), "{lines:?}");
}

#[tokio::test]
async fn an_identification_without_a_layout_fails_at_its_prepare_stage() {
    let running = start(&STARTER_FILES).await;
    let reply: LocateReply<super::request::IdentifiedLayout> = dev_client::post_json(
        running.port,
        path::LOCATE_IDENTIFY,
        &serde_json::json!({"picture": "plate.png", "characters": [{"name": "elf"}]}),
    )
    .await
    .unwrap();
    assert_eq!(
        rejected(reply),
        LocateError::NoLayout {
            picture: "plate.png".to_owned()
        }
    );
    let lines = locate_lines(&running);
    assert_eq!(lines[0], "LOCATE identify picture=plate.png characters=1");
    assert!(
        lines[1].starts_with(concat!(
            "LOCATE failed verb=identify picture=plate.png error=no_layout cause=- ",
            "stage=prepare duration_ms="
        )),
        "{lines:?}"
    );
    assert_eq!(lines.len(), 2, "{lines:?}");
}

#[tokio::test]
async fn an_identification_the_text_service_cannot_answer_fails_at_ask_with_its_operation() {
    let running = start(&STARTER_FILES).await;
    let picture = png_picture();
    std::fs::create_dir_all(running.playground_path("assets/media")).unwrap();
    std::fs::write(running.playground_path("assets/media/plate.png"), &picture).unwrap();
    let layouts = running.playground_path("assets/layouts");
    crate::locate::write_layout(
        &layouts,
        "plate.png",
        &one_region_layout("plate.png", &picture),
    )
    .unwrap();
    let work_root = tempfile::tempdir().unwrap();
    let route = crate::text::codex_text::CodexRoute {
        executable: std::path::PathBuf::from("Z:/no/such/codex.exe"),
        work_root: work_root.path().to_path_buf(),
        model: None,
    };
    let recorder = std::sync::Arc::clone(&running.state);
    let text = crate::text::TextRuntime::new(
        crate::text::BackendChoice::Codex(Some(route)),
        std::sync::Arc::new(move |line| recorder.record_line(line)),
    );
    let app = super::routes::router_with(
        std::sync::Arc::clone(&running.state),
        std::sync::Arc::new(text),
    );
    let listener = crate::server::bind(0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        // The server runs until the test's runtime stops; its end is the test's end.
        let _ = axum::serve(listener, app).await;
    });

    let reply: LocateReply<super::request::IdentifiedLayout> = dev_client::post_json(
        port,
        path::LOCATE_IDENTIFY,
        &serde_json::json!({"picture": "plate.png", "characters": [{"name": "elf"}]}),
    )
    .await
    .unwrap();
    assert!(
        matches!(rejected(reply), LocateError::TextService { .. }),
        "the unstartable backend gives no answer"
    );
    let lines = locate_lines(&running);
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(
        lines[1].starts_with("LOCATE identify_stage picture=plate.png stage=prepare duration_ms="),
        "{lines:?}"
    );
    let failed: Vec<&str> = lines[2].split(' ').collect();
    assert_eq!(
        failed[..7],
        [
            "LOCATE",
            "failed",
            "verb=identify",
            "picture=plate.png",
            "error=text_service",
            "cause=no_answer",
            "stage=ask"
        ],
        "{lines:?}"
    );
    assert!(failed[7].starts_with("duration_ms="), "{lines:?}");
    assert!(failed[8].starts_with("operation=identify:"), "{lines:?}");
    assert_eq!(failed.len(), 9, "{lines:?}");
    let text_lines: Vec<String> = running
        .state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("TEXT started "))
        .collect();
    assert_eq!(text_lines.len(), 1, "{text_lines:?}");
    assert!(text_lines[0].contains(failed[8].trim_start_matches("operation=")));
}

#[test]
fn every_locate_stage_key_is_its_serde_name() {
    for stage in super::request::LocateStage::ALL {
        assert_eq!(serde_json::to_value(stage).unwrap(), stage.key());
        assert_eq!(
            super::request::LocateStage::from_key(stage.key()),
            Some(stage)
        );
    }
}

#[test]
fn a_scan_the_supplier_rate_limited_fails_at_scan_with_its_cause_and_send_state() {
    use super::request::{LocateStage, LocateStageError, ScanCause, ScanSendState};
    let failed = LocateStageError {
        stage: LocateStage::Scan,
        error: LocateError::ScanFailed {
            cause: ScanCause::RateLimited,
            send_state: ScanSendState::KnownSent,
        },
    };
    assert_eq!(
        super::routes::failed_line("scan", "plate.png", &failed, 1200, None),
        concat!(
            "LOCATE failed verb=scan picture=plate.png error=scan_failed cause=rate_limited ",
            "stage=scan duration_ms=1200 send_state=known_sent"
        )
    );
    // A picture name that is not a library name never reaches the line.
    let line = super::routes::failed_line("scan", "a b\nLOCATE x", &failed, 0, None);
    assert!(line.contains(" picture=- "), "{line}");
}

/// A supplier on a local port that reads one whole request and answers it `429`.
fn rate_limiting_supplier() -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let (mut bytes, mut chunk) = (Vec::new(), [0_u8; 65_536]);
        let header_end = loop {
            let count = stream.read(&mut chunk).unwrap();
            assert_ne!(count, 0);
            bytes.extend_from_slice(&chunk[..count]);
            if let Some(at) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break at + 4;
            }
        };
        let head = String::from_utf8_lossy(&bytes[..header_end]).to_ascii_lowercase();
        let length: usize = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .map_or(0, |value| value.trim().parse().unwrap());
        while bytes.len() < header_end + length {
            let count = stream.read(&mut chunk).unwrap();
            assert_ne!(count, 0);
            bytes.extend_from_slice(&chunk[..count]);
        }
        stream
            .write_all(
                b"HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\n\
                  Content-Length: 2\r\nConnection: close\r\n\r\n{}",
            )
            .unwrap();
    });
    (format!("http://{address}"), server)
}

#[tokio::test]
async fn a_fixture_scan_the_supplier_rate_limits_is_one_failed_line_at_scan_with_its_cause() {
    let running = start(&STARTER_FILES).await;
    std::fs::create_dir_all(running.playground_path("assets/media")).unwrap();
    std::fs::write(
        running.playground_path("assets/media/plate.png"),
        png_picture(),
    )
    .unwrap();
    let (endpoint, server) = rate_limiting_supplier();
    let supplier = super::scan::ScanSupplier::Fal {
        endpoint,
        credential: Some("fixture-key".to_owned()),
    };

    let response =
        super::routes::scan_answer(&running.state, br#"{"picture":"plate.png"}"#, supplier).await;
    server.join().unwrap();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let reply: LocateReply<super::request::LocatedLayout> = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        rejected(reply),
        LocateError::ScanFailed {
            cause: super::request::ScanCause::RateLimited,
            send_state: super::request::ScanSendState::KnownSent,
        }
    );
    let lines = locate_lines(&running);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with("LOCATE started picture=plate.png "));
    let failed: Vec<&str> = lines[1].split(' ').collect();
    assert_eq!(
        failed[..7],
        [
            "LOCATE",
            "failed",
            "verb=scan",
            "picture=plate.png",
            "error=scan_failed",
            "cause=rate_limited",
            "stage=scan"
        ],
        "{lines:?}"
    );
    assert!(failed[7].starts_with("duration_ms="), "{lines:?}");
    assert_eq!(failed[8..], ["send_state=known_sent"], "{lines:?}");
    // No layout was stored, and the key never reaches a line.
    assert!(
        !running
            .playground_path("assets/layouts/plate.png.layout.json")
            .exists()
    );
    assert!(lines.iter().all(|line| !line.contains("fixture-key")));
}

#[test]
fn a_stored_scan_line_carries_the_time_of_each_phase_of_the_scan() {
    let scanned = crate::locate::Scanned {
        layout: one_region_layout("plate.png", b"plate"),
        duration_ms: 2500,
        kept_labels: 1,
        left_out: 0,
        old_layout_unreadable: false,
        phases: df_imagery::oneshot_segment::SegmentPhases {
            prepare_ms: 40,
            submit_ms: 300,
            resume_ms: 2100,
        },
    };
    assert_eq!(
        super::routes::stored_line("plate.png", &scanned),
        concat!(
            "LOCATE stored picture=plate.png file=plate.png.layout.json model=sam-3.1 ",
            "prompt=person regions=1 left_out=0 kept_labels=1 old_layout=read size=100x50 ",
            "duration_ms=2500 prepare_ms=40 submit_ms=300 resume_ms=2100"
        )
    );
}

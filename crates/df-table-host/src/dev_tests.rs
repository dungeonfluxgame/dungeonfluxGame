use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use df_table::event::RejectionCause;
use tempfile::TempDir;

use crate::cast::request::CastError;
use crate::client::ClientError;
use crate::command::{CommandError, split_words};
use crate::dev::{DevError, ReplyError, run_dev};
use crate::playground::{ClientsError, PlaygroundChangeError};
use crate::test_support::{Running, STARTER_FILES, canned_server, closed_port, start};

async fn run(running: &Running, line: &str) -> Result<String, DevError> {
    run_dev(running.port, &split_words(line).expect("the line splits")).await
}

fn script(text: &str) -> (TempDir, PathBuf) {
    let directory = TempDir::new().expect("makes a temporary directory");
    let path = directory.path().join("scene.txt");
    std::fs::write(&path, text).expect("writes the script");
    (directory, path)
}

async fn run_script_file(running: &Running, path: &Path) -> Result<String, DevError> {
    run_dev(
        running.port,
        &["script".to_owned(), path.to_string_lossy().into_owned()],
    )
    .await
}

#[tokio::test]
async fn a_spawn_is_applied_and_shows_in_the_canvas() {
    let running = start(&STARTER_FILES).await;
    let said = run(
        &running,
        "spawn character pip x=20 y=30 name=Pip hit_points=7",
    )
    .await
    .expect("spawns");
    assert!(said.starts_with("ok (revision 1)"));
    let canvas = run(&running, "canvas").await.expect("reads");
    assert!(canvas.contains("\"pip\"") && canvas.contains("\"Pip\""));
}

#[tokio::test]
async fn a_refused_change_comes_back_as_a_rejection_with_the_reason() {
    let running = start(&STARTER_FILES).await;
    let error = run(&running, "spawn dragon smaug")
        .await
        .expect_err("refused");
    assert!(
        matches!(
            &error,
            DevError::Rejected {
                error: ReplyError::Canvas {
                    cause: Some(RejectionCause::UnknownKind),
                    ..
                },
                ..
            }
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_command_that_sends_several_effects_applies_each_in_order_and_reports_every_failure() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn character pip name=Pip")
        .await
        .expect("spawns");
    let error = run(&running, "set pip hit_points=5000 name=Pippin")
        .await
        .expect_err("the first is refused");
    let DevError::SomeRejected { label, answers } = &error else {
        panic!("expected some rejected: {error:?}");
    };
    assert_eq!(*label, "effect");
    assert!(
        matches!(
            answers.as_slice(),
            [
                Err(DevError::Rejected {
                    error: ReplyError::Canvas {
                        cause: Some(RejectionCause::InvalidValue),
                        ..
                    },
                    ..
                }),
                Ok(_),
            ]
        ),
        "{error:?}"
    );
    let text = error.to_string();
    assert!(
        text.contains("effect 1:") && text.contains("effect 2: ok"),
        "{text}"
    );
    let canvas = run(&running, "canvas").await.expect("reads");
    assert!(canvas.contains("Pippin"), "the second effect still ran");
}

#[tokio::test]
async fn a_command_the_parser_refuses_never_reaches_the_host() {
    let running = start(&STARTER_FILES).await;
    let error = run(&running, "dance").await.expect_err("refused");
    assert_eq!(
        error,
        DevError::Command(CommandError::Unknown("dance".into()))
    );
    let events = run(&running, "events area=CANVAS").await.expect("reads");
    assert_eq!(events, "no events");
}

#[tokio::test]
async fn kinds_and_help_and_events_print_text() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker m1").await.expect("spawns");
    assert!(
        run(&running, "kinds")
            .await
            .expect("reads")
            .contains("character")
    );
    assert!(
        run(&running, "help")
            .await
            .expect("prints")
            .contains("spawn")
    );
    assert!(
        run(&running, "events")
            .await
            .expect("reads")
            .contains("spawn element=m1")
    );
}

#[tokio::test]
async fn events_after_a_sequence_show_only_newer_lines() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker a").await.expect("spawns");
    run(&running, "spawn marker b").await.expect("spawns");
    let canvas_lines = run(&running, "events area=CANVAS").await.expect("reads");
    let first: u64 = canvas_lines
        .split_whitespace()
        .next()
        .and_then(|sequence| sequence.parse().ok())
        .expect("the first canvas line has a sequence");
    let text = run(&running, &format!("events {first}"))
        .await
        .expect("reads");
    assert!(
        text.contains("element=b") && !text.contains("element=a"),
        "{text}"
    );
}

#[tokio::test]
async fn reload_picks_up_a_new_kind_file_and_a_broken_one_leaves_the_kinds_alone() {
    let running = start(&STARTER_FILES).await;
    running.write_file(
        "wall.json",
        r#"{"name": "wall", "shape": "rect", "width": 10, "height": 2}"#,
    );
    run(&running, "reload").await.expect("reloads");
    assert!(
        run(&running, "kinds")
            .await
            .expect("reads")
            .contains("wall")
    );
    running.write_file("broken.json", "{ not json");
    let error = run(&running, "reload").await.expect_err("refused");
    // A rejected reload has no single cause; its problems name each file that does not read.
    let DevError::Rejected {
        error: ReplyError::Canvas {
            cause: None,
            problems,
        },
        ..
    } = &error
    else {
        panic!("expected a rejected reload, got {error:?}");
    };
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("broken.json")),
        "{error:?}"
    );
    assert!(
        run(&running, "kinds")
            .await
            .expect("reads")
            .contains("wall")
    );
}

#[tokio::test]
async fn a_script_runs_its_lines_in_order_skipping_blank_lines_and_comments() {
    let running = start(&STARTER_FILES).await;
    let (_directory, path) =
        script("# a small scene\n\nspawn marker a x=10 y=10\nspawn marker b x=20 y=10\n");
    let said = run_script_file(&running, &path).await.expect("runs");
    assert!(said.contains("3: ok") && said.contains("4: ok"), "{said}");
    assert!(said.ends_with("ran 2 commands, all ok"));
}

#[tokio::test]
async fn a_script_with_a_typo_is_refused_before_anything_in_it_runs() {
    let running = start(&STARTER_FILES).await;
    let (_directory, path) = script("spawn marker a\nspawn marker b\ndance\n");
    let error = run_script_file(&running, &path).await.expect_err("refused");
    assert!(
        matches!(&error, DevError::ScriptLine { line: 3, .. }),
        "{error}"
    );
    assert_eq!(
        run(&running, "events area=CANVAS").await.expect("reads"),
        "no events"
    );
}

#[tokio::test]
async fn a_script_may_not_run_another_script() {
    let running = start(&STARTER_FILES).await;
    let (_directory, path) = script("script other.txt\n");
    let error = run_script_file(&running, &path).await.expect_err("refused");
    assert!(matches!(
        error,
        DevError::ScriptLine {
            error: CommandError::NotAllowedInScript("script"),
            ..
        }
    ));
}

#[tokio::test]
async fn a_script_stops_at_its_first_failing_line_and_the_lines_before_it_stay_applied() {
    let running = start(&STARTER_FILES).await;
    let (_directory, path) = script("spawn marker a\nspawn marker a\nspawn marker c\n");
    let error = run_script_file(&running, &path).await.expect_err("stops");
    assert!(
        matches!(&error, DevError::ScriptStopped { line: 2, .. }),
        "{error}"
    );
    let canvas = run(&running, "canvas").await.expect("reads");
    assert!(canvas.contains("\"a\"") && !canvas.contains("\"c\""));
}

#[tokio::test]
async fn a_script_that_cannot_be_read_names_its_path() {
    let running = start(&STARTER_FILES).await;
    let error = run_script_file(&running, Path::new("no/such/script.txt"))
        .await
        .expect_err("refused");
    assert!(
        matches!(&error, DevError::UnreadableFile { path, .. } if path.ends_with("script.txt"))
    );
    assert!(error.to_string().contains("script.txt"));
}

#[tokio::test]
async fn with_nothing_serving_the_error_says_how_to_start_it() {
    let port = closed_port().await;
    let error = run_dev(port, &split_words("canvas").expect("splits"))
        .await
        .expect_err("fails");
    assert_eq!(error, DevError::Client(ClientError::NotServing { port }));
}

#[tokio::test]
async fn an_answer_the_program_does_not_understand_is_reported_with_its_status_and_start() {
    let body = "x".repeat(crate::limits::MAXIMUM_ECHOED_ANSWER_CHARACTERS + 300);
    let answer = format!(
        "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let port = canned_server(answer.into_bytes()).await;
    let error = run_dev(port, &split_words("canvas").expect("splits"))
        .await
        .expect_err("fails");
    assert!(
        error
            .to_string()
            .starts_with("unexpected answer (status 500): "),
        "{error}"
    );
    let DevError::UnexpectedAnswer { status, body, .. } = error else {
        panic!("expected an unexpected answer");
    };
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body.chars().count(),
        crate::limits::MAXIMUM_ECHOED_ANSWER_CHARACTERS
    );
}

#[tokio::test]
async fn a_changed_controls_answer_that_does_not_read_says_why() {
    let body = r#"{"outcome":"done","value":{"revision":3,"screens":{}}}"#;
    let answer = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let port = canned_server(answer.into_bytes()).await;
    let error = run_dev(
        port,
        &split_words("playground set device=tv").expect("splits"),
    )
    .await
    .expect_err("fails");
    assert!(
        error
            .to_string()
            .starts_with("unexpected answer (status 200, missing field"),
        "{error}"
    );
    let DevError::UnexpectedAnswer { status, reason, .. } = error else {
        panic!("expected an unexpected answer");
    };
    assert_eq!(status, StatusCode::OK);
    let reason = reason.expect("a reason");
    assert!(reason.contains("missing field `controls`"), "{reason}");
}

#[tokio::test]
async fn a_guard_refusal_of_a_controls_change_is_reported_as_its_sentence() {
    let body = serde_json::json!({
        "outcome": "rejected",
        "cause": "forbidden_request",
        "message": "requests are not accepted from another site",
        "problems": [],
    })
    .to_string();
    let answer = format!(
        "HTTP/1.1 403 Forbidden\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let port = canned_server(answer.into_bytes()).await;
    let error = run_dev(
        port,
        &split_words("playground set device=tv").expect("splits"),
    )
    .await
    .expect_err("refused");
    assert_eq!(
        error,
        DevError::Rejected {
            error: ReplyError::Canvas {
                cause: Some(RejectionCause::ForbiddenRequest),
                problems: Vec::new(),
            },
            message: "requests are not accepted from another site".to_owned(),
        }
    );
    assert_eq!(
        error.to_string(),
        "requests are not accepted from another site"
    );
}

#[tokio::test]
async fn the_command_line_refuses_its_own_mistakes_by_type_before_asking_the_canvas() {
    let running = start(&STARTER_FILES).await;
    assert_eq!(
        run(&running, "repl").await,
        Err(DevError::ReplOutsideCommandLine)
    );
    assert!(
        matches!(
            run(&running, "fixture show ../escape").await,
            Err(DevError::InvalidFixtureName(_))
        ),
        "a name that leaves the fixture directory is refused here"
    );
    let words = ["media".to_owned(), "add".to_owned(), "..".to_owned()];
    assert_eq!(
        run_dev(running.port, &words).await,
        Err(DevError::InvalidMediaPath {
            path: PathBuf::from("..")
        })
    );
}

#[test]
fn a_refusal_prints_the_hosts_sentence_with_the_key_its_routes_show() {
    let cast = DevError::Rejected {
        error: ReplyError::Cast(CastError::Shown {
            cause: crate::shown_scene::ShownSceneError::NoBackdrop,
        }),
        message: "the shown scene has no backdrop picture".to_owned(),
    };
    assert_eq!(
        cast.to_string(),
        "refused (no_backdrop): the shown scene has no backdrop picture"
    );
    let canvas = DevError::Rejected {
        error: ReplyError::Canvas {
            cause: None,
            problems: vec!["broken.json: not json".to_owned()],
        },
        message: "kinds unchanged".to_owned(),
    };
    assert_eq!(
        canvas.to_string(),
        "kinds unchanged\n  broken.json: not json"
    );
    let playground = DevError::Rejected {
        error: ReplyError::Playground(PlaygroundChangeError::InvalidChange {
            detail: "no fixture is shown".to_owned(),
        }),
        message: "no fixture is shown".to_owned(),
    };
    assert_eq!(playground.to_string(), "no fixture is shown");
    let some = DevError::SomeRejected {
        label: "effect",
        answers: vec![Err(canvas.clone()), Ok("ok (revision 2)".to_owned())],
    };
    assert_eq!(
        some.to_string(),
        "effect 1: kinds unchanged\n  broken.json: not json\neffect 2: ok (revision 2)"
    );
    let cycle = DevError::CycleStopped {
        done: vec!["1: quay ok (revision 1)".to_owned()],
        step: 2,
        scene: "castle".to_owned(),
        error: Box::new(playground),
    };
    assert_eq!(
        cycle.to_string(),
        "1: quay ok (revision 1)\n2: castle failed: no fixture is shown"
    );
}

// ---- the host reads

#[tokio::test]
async fn host_status_reads_the_log_the_sink_and_the_clock() {
    let running = start(&STARTER_FILES).await;
    let text = run(&running, "host status").await.expect("reads");
    assert!(text.contains("log: latest="), "{text}");
    assert!(text.contains("clock: ticks="), "{text}");
    assert!(text.contains("sink: level="), "{text}");
}

#[tokio::test]
async fn events_are_read_with_their_time_and_filtered_by_area_and_limit() {
    let running = start(&STARTER_FILES).await;
    run(&running, "spawn marker a").await.expect("spawns");
    run(&running, "spawn marker b").await.expect("spawns");
    let both = run(&running, "events area=CANVAS").await.expect("reads");
    assert_eq!(both.lines().count(), 2, "{both}");
    let newest = run(&running, "events area=CANVAS limit=1")
        .await
        .expect("reads the newest");
    assert_eq!(newest.lines().count(), 1, "{newest}");
    assert!(newest.contains("element=b"), "{newest}");
    let words: Vec<&str> = newest.split_whitespace().collect();
    let at_ms: u64 = words[1].parse().expect("the second column is the time");
    assert!(at_ms > 0, "{newest}");
    let other = run(&running, "events area=HTTP").await.expect("reads");
    assert!(!other.contains("CANVAS"), "{other}");
}

#[tokio::test]
async fn the_event_level_is_read_set_and_read_back() {
    let running = start(&STARTER_FILES).await;
    assert_eq!(
        run(&running, "events level always").await.expect("sets"),
        "event level set to always"
    );
    assert_eq!(
        run(&running, "events level").await.expect("reads"),
        "event level is always"
    );
    assert_eq!(
        run(&running, "events level verbose").await.expect("sets"),
        "event level set to verbose"
    );
    assert_eq!(
        run(&running, "events level").await.expect("reads"),
        "event level is verbose"
    );
    let events = run(&running, "events area=HOST").await.expect("reads");
    assert!(
        events.contains("HOST sink_level from=always to=verbose"),
        "{events}"
    );
}

#[tokio::test]
async fn frame_prints_the_count_the_run_and_the_keys_and_a_known_count_returns_at_once() {
    let running = start(&STARTER_FILES).await;
    let text = run(&running, "frame").await.expect("reads");
    assert!(text.starts_with("frame: "), "{text}");
    assert!(text.contains("keys: canvas, controls, frame"), "{text}");
    let other = run(&running, "frame after=99999")
        .await
        .expect("answers at once");
    assert!(other.starts_with("frame: "), "{other}");
}

#[tokio::test]
async fn a_page_that_is_not_open_is_refused_by_its_typed_error() {
    let running = start(&STARTER_FILES).await;
    for line in ["clients page-none", "clients timeline page-none"] {
        let error = run(&running, line).await.expect_err("refused");
        assert!(
            matches!(
                &error,
                DevError::Rejected {
                    error: ReplyError::Clients(ClientsError::NoSuchClient { client }),
                    ..
                } if client == "page-none"
            ),
            "{line}: {error:?}"
        );
        assert!(
            error.to_string().starts_with("refused (no_such_client)"),
            "{error}"
        );
    }
}

#[tokio::test]
async fn an_open_pages_timeline_is_read_with_its_kind_filter() {
    let running = start(&STARTER_FILES).await;
    let report = serde_json::json!({
        "client": "page-1",
        "report": df_playground::media::ClientReport {
            timeline: vec![df_playground::timeline::TimelineRecord {
                seq: 1,
                kind: df_playground::timeline::TimelineKind::Line,
                id: "line-0badcafe".to_owned(),
                start_page_ms: 10,
                end_page_ms: Some(40),
                end_reason: Some(df_playground::timeline::EndReason::Replaced),
                planned_ms: Some(4000),
                detail: None,
            }],
            ..df_playground::media::ClientReport::default()
        },
    })
    .to_string();
    let posted = crate::client::post_json(running.port, "/dev/playground/report", &report)
        .await
        .expect("answers");
    assert_eq!(posted.status, StatusCode::OK, "{}", posted.body);
    let line = run(&running, "clients timeline page-1 kind=line")
        .await
        .expect("reads");
    assert!(
        line.contains("line-0badcafe") && line.contains("reason=replaced"),
        "{line}"
    );
    assert!(line.contains("short=true"), "{line}");
    let plate = run(&running, "clients timeline page-1 kind=plate")
        .await
        .expect("reads");
    assert!(plate.contains("no timeline entries"), "{plate}");
    let one = run(&running, "clients page-1").await.expect("reads");
    assert!(
        one.contains("client page-1") && one.contains("frame "),
        "{one}"
    );
}

#[tokio::test]
async fn media_head_prints_status_type_length_and_a_missing_file_is_a_404() {
    let running = start(&STARTER_FILES).await;
    let missing = run(&running, "media head ghost.png")
        .await
        .expect("answers");
    assert!(missing.starts_with("ghost.png: status 404"), "{missing}");
    let directory = TempDir::new().expect("makes a directory");
    let path = directory.path().join("tavern.png");
    std::fs::write(&path, b"not really a picture").expect("writes");
    let words = vec![
        "media".to_owned(),
        "add".to_owned(),
        path.to_string_lossy().into_owned(),
    ];
    run_dev(running.port, &words).await.expect("adds");
    let head = run(&running, "media head tavern.png")
        .await
        .expect("answers");
    assert!(
        head.starts_with("tavern.png: status 200 type image/png length 20 etag "),
        "{head}"
    );
}

#[tokio::test]
async fn playground_prints_the_files_revision_beside_the_controls() {
    let running = start(&STARTER_FILES).await;
    let text = run(&running, "playground").await.expect("reads");
    assert!(text.contains("\nfiles: revision="), "{text}");
}

fn canned_refusal(status_line: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

async fn run_on(port: u16, line: &str) -> Result<String, DevError> {
    run_dev(port, &split_words(line).expect("the line splits")).await
}

#[tokio::test]
async fn a_refused_events_query_and_a_refused_host_status_come_back_as_the_hosts_typed_error() {
    let body = r#"{"outcome":"rejected","error":{"error":"invalid_query","detail":"bad"},
        "message":"the query is not valid: bad"}"#;
    for line in ["events 3", "host status"] {
        let port = canned_server(canned_refusal("400 Bad Request", body)).await;
        let error = run_on(port, line).await.expect_err("refused");
        assert!(
            matches!(
                &error,
                DevError::Rejected {
                    error: ReplyError::Host(crate::server::request::HostError::InvalidQuery { .. }),
                    ..
                }
            ),
            "{line}: {error:?}"
        );
        assert_eq!(
            error.to_string(),
            "refused (invalid_query): the query is not valid: bad"
        );
    }
}

#[tokio::test]
async fn a_refused_services_read_and_a_refused_timeline_read_keep_their_error_keys() {
    let services = r#"{"outcome":"rejected","error":{"error":"invalid_query"},
        "message":"the services report takes no query"}"#;
    let port = canned_server(canned_refusal("400 Bad Request", services)).await;
    let error = run_on(port, "services").await.expect_err("refused");
    assert!(
        matches!(
            &error,
            DevError::Rejected {
                error: ReplyError::Services(crate::services::ServicesError::InvalidQuery),
                ..
            }
        ),
        "{error:?}"
    );
    let timeline = r#"{"outcome":"rejected","error":{"error":"invalid_query","detail":"kind"},
        "message":"the timeline query is not valid: kind"}"#;
    let port = canned_server(canned_refusal("400 Bad Request", timeline)).await;
    let error = run_on(port, "clients timeline page-1")
        .await
        .expect_err("refused");
    assert!(
        matches!(
            &error,
            DevError::Rejected {
                error: ReplyError::Clients(ClientsError::InvalidQuery { .. }),
                ..
            }
        ),
        "{error:?}"
    );
    assert!(
        error.to_string().starts_with("refused (invalid_query)"),
        "{error}"
    );
}

#[tokio::test]
async fn a_refused_event_level_change_keeps_the_engines_typed_error() {
    let body = r#"{"outcome":"rejected","error":{"error":"mailbox_full"},
        "message":"the owner is busy"}"#;
    let port = canned_server(canned_refusal("400 Bad Request", body)).await;
    let error = run_on(port, "events level verbose")
        .await
        .expect_err("refused");
    assert!(
        matches!(
            &error,
            DevError::Rejected {
                error: ReplyError::Engine(crate::engine::report::EngineError::MailboxFull),
                ..
            }
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn services_reads_the_report_without_a_key_value() {
    let running = start(&STARTER_FILES).await;
    let text = run(&running, "services").await.expect("reads");
    assert!(text.to_ascii_lowercase().contains("text"), "{text}");
}

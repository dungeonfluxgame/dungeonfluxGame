use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use df_table::event::RejectionCause;
use tempfile::TempDir;

use crate::cast::request::CastError;
use crate::client::ClientError;
use crate::command::{CommandError, split_words};
use crate::dev::{DevError, ReplyError, run_dev};
use crate::playground::PlaygroundChangeError;
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
    let events = run(&running, "events").await.expect("reads");
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
    let text = run(&running, "events 1").await.expect("reads");
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
    assert_eq!(run(&running, "events").await.expect("reads"), "no events");
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
        error: ReplyError::Cast(CastError::NoBackdrop),
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

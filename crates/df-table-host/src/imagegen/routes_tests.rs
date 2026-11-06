//! The route and the verb over a running host: a scripted backend behind the route, and the host's
//! real router for what the guard and the registration decide. No test here can reach a provider:
//! a request that would start the real backend is never sent to the real router.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;

use super::backend::BackendError;
use super::fixtures::{FakeBackend, Script, picture, png_bytes};
use super::model::GenerateRequestError;
use super::request::{GenerateError, GenerateReply, JobState, JobView};
use super::routes::{path, router_with};
use super::service::ImageGenerator;
use crate::client::{get, post_json};
use crate::dev::{DevError, ReplyError, run_dev};
use crate::dev_http::{BodyCause, DevReply};
use crate::guard;
use crate::server;
use crate::test_support::{Running, post_request, raw_exchange, start};
use crate::wire::route;

/// The route over `generator`, behind the host's own guard check, on an ephemeral port.
async fn serve_route(running: &Running, generator: Arc<ImageGenerator>) -> u16 {
    async fn guarded(request: Request, next: Next) -> Response {
        if let Err(refusal) = guard::check(request.method(), request.headers()) {
            return (refusal.status(), refusal.message()).into_response();
        }
        next.run(request).await
    }
    let app =
        router_with(Arc::clone(&running.state), generator).layer(middleware::from_fn(guarded));
    let listener = server::bind(0).await.expect("binds an ephemeral port");
    let port = listener.local_addr().expect("has an address").port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    port
}

fn generator(backend: Arc<FakeBackend>) -> Arc<ImageGenerator> {
    Arc::new(ImageGenerator::new(backend))
}

fn body(name: &str, extra: &str) -> String {
    format!(r#"{{"prompt":"a candlelit hall","name":"{name}"{extra}}}"#)
}

fn reply<T: DeserializeOwned>(text: &str) -> GenerateReply<T> {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("a reply: {error}: {text}"))
}

/// The value of a reply that must be `Done`.
fn done<T: DeserializeOwned>(text: &str) -> T {
    match reply(text) {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, .. } => panic!("refused: {error:?}"),
    }
}

/// The error of a reply that must be `Rejected`.
fn rejected(text: &str) -> GenerateError {
    match reply::<serde_json::Value>(text) {
        DevReply::Rejected { error, .. } => error,
        DevReply::Done { value } => panic!("not refused: {value}"),
    }
}

fn words(line: &str) -> Vec<String> {
    crate::command::split_words(line).expect("splits")
}

fn imagegen_lines(running: &Running) -> Vec<String> {
    running
        .state
        .log()
        .newest(20)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("IMAGEGEN"))
        .collect()
}

fn media_listing_names(text: &str) -> Vec<String> {
    let entries: Vec<serde_json::Value> = serde_json::from_str(text).expect("a list");
    entries
        .iter()
        .map(|entry| entry["name"].as_str().expect("a name").to_owned())
        .collect()
}

#[tokio::test]
async fn a_waited_generation_is_stored_and_the_media_command_lists_it() {
    let running = start(&[]).await;
    let port = serve_route(
        &running,
        generator(FakeBackend::new(Script::Picture(picture()))),
    )
    .await;
    let response = post_json(port, path::GENERATE, &body("hall", r#","wait":true"#))
        .await
        .expect("answers");
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    let job: JobView = done(&response.body);
    assert!(matches!(job.state, JobState::Stored { .. }), "{job:?}");
    assert_eq!(job.job, 1);

    let stored = std::fs::read(running.playground_path("assets/media/hall.png")).expect("reads");
    assert_eq!(stored, png_bytes());
    let listing = get(running.port, route::PLAYGROUND_MEDIA_LIST)
        .await
        .expect("lists");
    assert_eq!(
        media_listing_names(&listing.body),
        vec!["hall.png".to_owned()]
    );
    let lines = imagegen_lines(&running);
    assert_eq!(lines.len(), 2, "{lines:?}");
}

#[tokio::test]
async fn a_generation_that_is_not_waited_for_runs_on_and_status_shows_it() {
    let running = start(&[]).await;
    let port = serve_route(
        &running,
        generator(FakeBackend::slow(
            Script::Picture(picture()),
            Duration::from_millis(300),
        )),
    )
    .await;
    let started = post_json(port, path::GENERATE, &body("hall", ""))
        .await
        .expect("answers");
    assert_eq!(started.status, StatusCode::OK);
    let job: JobView = done(&started.body);
    assert_eq!((job.job, &job.state), (1, &JobState::Running));
    let mut finished = false;
    for _ in 0..100 {
        let status = get(port, path::GENERATE).await.expect("answers");
        let jobs: Vec<JobView> = done(&status.body);
        assert_eq!(jobs.len(), 1);
        if matches!(jobs[0].state, JobState::Stored { .. }) {
            finished = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(finished, "the job finished without anyone waiting");
    assert!(running.playground_path("assets/media/hall.png").is_file());
}

#[tokio::test]
async fn each_refusal_has_its_typed_error_and_status() {
    let running = start(&[]).await;
    let home = PathBuf::from("codex-home");
    let signed_out = serve_route(
        &running,
        generator(FakeBackend::failing_check(BackendError::NoLogin {
            home: home.clone(),
        })),
    )
    .await;
    let refused = post_json(signed_out, path::GENERATE, &body("hall", ""))
        .await
        .expect("answers");
    assert_eq!(refused.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        rejected(&refused.body),
        GenerateError::Backend {
            job: None,
            backend: BackendError::NoLogin { home }
        }
    );
    assert_eq!(
        imagegen_lines(&running),
        vec![
            "IMAGEGEN refused verb=generate name=hall error=backend cause=no_login backend=fake"
                .to_owned()
        ]
    );

    let timed_out = serve_route(
        &running,
        generator(FakeBackend::new(Script::Fail(BackendError::Timeout))),
    )
    .await;
    let response = post_json(timed_out, path::GENERATE, &body("hall", r#","wait":true"#))
        .await
        .expect("answers");
    assert_eq!(response.status, StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(
        rejected(&response.body),
        GenerateError::Backend {
            job: Some(1),
            backend: BackendError::Timeout
        }
    );

    let service_refused = serve_route(
        &running,
        generator(FakeBackend::new(Script::Fail(BackendError::Refused {
            detail: "refused by the service".to_owned(),
        }))),
    )
    .await;
    let response = post_json(
        service_refused,
        path::GENERATE,
        &body("hall", r#","wait":true"#),
    )
    .await
    .expect("answers");
    assert_eq!(response.status, StatusCode::BAD_GATEWAY);
    assert!(matches!(
        rejected(&response.body),
        GenerateError::Backend {
            job: Some(1),
            backend: BackendError::Refused { .. }
        }
    ));
}

#[tokio::test]
async fn a_body_that_cannot_start_a_generation_is_a_typed_400() {
    let running = start(&[]).await;
    let port = serve_route(
        &running,
        generator(FakeBackend::new(Script::Picture(picture()))),
    )
    .await;
    let unreadable = [
        ("not json".to_owned(), BodyCause::Syntax),
        (r#"{"prompt":"a"}"#.to_owned(), BodyCause::Data),
        (body("hall", r#","purpose":"poster""#), BodyCause::Data),
        (body("hall", r#","aspect":"wide""#), BodyCause::Data),
        (body("hall", r#","colour":"red""#), BodyCause::Data),
    ];
    for (text, cause) in unreadable {
        let response = post_json(port, path::GENERATE, &text)
            .await
            .expect("answers");
        assert_eq!(
            response.status,
            StatusCode::BAD_REQUEST,
            "{text}: {}",
            response.body
        );
        let error = rejected(&response.body);
        assert!(
            matches!(&error, GenerateError::InvalidBody(body) if body.cause == cause),
            "{text}: {error:?}"
        );
    }
    let invalid = [
        (
            body("bad name", ""),
            GenerateRequestError::InvalidName {
                name: "bad name".to_owned(),
            },
        ),
        (
            r#"{"prompt":"  ","name":"hall"}"#.to_owned(),
            GenerateRequestError::EmptyPrompt,
        ),
    ];
    for (text, expected) in invalid {
        let response = post_json(port, path::GENERATE, &text)
            .await
            .expect("answers");
        assert_eq!(
            response.status,
            StatusCode::BAD_REQUEST,
            "{text}: {}",
            response.body
        );
        assert_eq!(
            rejected(&response.body),
            GenerateError::InvalidRequest(expected)
        );
    }
    assert_eq!(
        imagegen_lines(&running).len(),
        7,
        "each refusal is recorded once"
    );
    assert!(
        running.playground_path("assets/media").read_dir().is_err(),
        "nothing was made"
    );
}

#[tokio::test]
async fn the_route_sits_behind_the_guard() {
    let running = start(&[]).await;
    let port = serve_route(
        &running,
        generator(FakeBackend::new(Script::Picture(picture()))),
    )
    .await;
    let foreign = raw_exchange(
        port,
        &format!(
            "POST {} HTTP/1.1\r\nHost: example.com\r\nContent-Type: application/json\r\n\
             Content-Length: 2\r\nConnection: close\r\n\r\n{{}}",
            path::GENERATE
        ),
    )
    .await;
    assert!(foreign.starts_with("HTTP/1.1 403"), "{foreign}");
    let form = raw_exchange(
        port,
        &format!(
            "POST {} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\n\
             Content-Length: 2\r\nConnection: close\r\n\r\n{{}}",
            path::GENERATE
        ),
    )
    .await;
    assert!(form.starts_with("HTTP/1.1 415"), "{form}");
    assert!(
        running.playground_path("assets/media").read_dir().is_err(),
        "nothing was made"
    );
}

#[tokio::test]
async fn the_hosts_real_router_registers_the_route_behind_its_guard() {
    // The real router builds its backend from the environment, so nothing is sent to it that
    // could start one: only requests the guard or the request check refuses.
    let running = start(&[]).await;
    let foreign = raw_exchange(
        running.port,
        &post_request(path::GENERATE, "{}", "Origin: http://example.com\r\n"),
    )
    .await;
    assert!(foreign.starts_with("HTTP/1.1 403"), "{foreign}");
    let form = raw_exchange(
        running.port,
        &format!(
            "POST {} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\n\
             Content-Length: 2\r\nConnection: close\r\n\r\n{{}}",
            path::GENERATE
        ),
    )
    .await;
    assert!(form.starts_with("HTTP/1.1 415"), "{form}");
    let bad = post_json(running.port, path::GENERATE, "not json")
        .await
        .expect("answers");
    assert_eq!(
        bad.status,
        StatusCode::BAD_REQUEST,
        "the route exists: {}",
        bad.body
    );
    assert!(matches!(rejected(&bad.body), GenerateError::InvalidBody(_)));
    let status = get(running.port, path::GENERATE).await.expect("answers");
    assert_eq!(status.status, StatusCode::OK);
    let _: Vec<JobView> = done(&status.body);
}

#[tokio::test]
async fn the_verb_numbers_its_answer_and_reports_refusals() {
    let running = start(&[]).await;
    let port = serve_route(
        &running,
        generator(FakeBackend::new(Script::Picture(picture()))),
    )
    .await;
    let made = run_dev(
        port,
        &words("media generate a candlelit hall as=hall purpose=background aspect=16:9 wait=true"),
    )
    .await
    .expect("generates");
    assert!(
        made.starts_with("ok (generation 1) stored hall.png in the media library"),
        "{made}"
    );
    assert!(made.contains("made by fake"), "{made}");
    assert!(made.contains("PNG stays a PNG"), "{made}");

    let started = run_dev(port, &words("media generate a second hall as=second"))
        .await
        .expect("starts");
    assert!(
        started.starts_with("ok (generation 2) started second"),
        "{started}"
    );

    let status = run_dev(port, &words("media generate status"))
        .await
        .expect("lists");
    assert!(status.contains("generation 1: hall done"), "{status}");
    assert!(status.contains("-> hall.png 1672x941"), "{status}");

    let signed_out = serve_route(
        &running,
        generator(FakeBackend::failing_check(BackendError::NoCodexHome)),
    )
    .await;
    let error = run_dev(
        signed_out,
        &words("media generate a hall as=hall wait=true"),
    )
    .await
    .expect_err("refused");
    assert!(
        matches!(
            error,
            DevError::Rejected {
                error: ReplyError::Generate(_),
                ..
            }
        ),
        "{error:?}"
    );

    let bad = run_dev(port, &words("media generate a hall as=hall.png"))
        .await
        .expect_err("not sent");
    assert!(matches!(bad, DevError::Command(_)), "{bad:?}");
}

#[tokio::test]
async fn a_waited_generation_without_a_picture_prints_its_name_and_how_long_it_ran() {
    let running = start(&[]).await;
    let port = serve_route(
        &running,
        generator(FakeBackend::new(Script::Fail(BackendError::Timeout))),
    )
    .await;
    let error = run_dev(port, &words("media generate a hall as=hall wait=true"))
        .await
        .expect_err("no picture");
    let DevError::GenerationEnded {
        job,
        error: ended_by,
        ..
    } = &error
    else {
        panic!("not an ended generation: {error:?}");
    };
    assert_eq!((job.job, job.name.as_str()), (1, "hall"));
    assert_eq!(
        *ended_by,
        GenerateError::Backend {
            job: Some(1),
            backend: BackendError::Timeout,
        }
    );
    // The line it prints names the job and how long it ran, then the host's sentence.
    let text = error.to_string();
    assert!(
        text.starts_with("generation 1 of hall ended without a picture (backend: timeout) after "),
        "{text}"
    );
    assert!(text.contains("\n  "), "{text}");
}

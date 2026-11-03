//! The route and the verb over a running host: a scripted backend behind the route, and the host's
//! real router for what the guard and the registration decide. No test here can reach a provider:
//! a request that would start the real backend is never sent to the real router.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::Request;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};

use super::fake::{FakeBackend, Script, picture, png_bytes};
use super::model::{BackendError, GenerateReply, JobState, RefusalCause};
use super::routes::{path, router_with};
use super::service::ImageGenerator;
use crate::client::{get, post_json};
use crate::dev::{DevError, run_dev};
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

fn reply(text: &str) -> GenerateReply {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("a reply: {error}: {text}"))
}

fn words(line: &str) -> Vec<String> {
    crate::command::split_words(line).expect("splits")
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
    assert_eq!(response.status, 200, "{}", response.body);
    let GenerateReply::Finished { job } = reply(&response.body) else {
        panic!("expected a finished job: {}", response.body);
    };
    assert_eq!(job.state, JobState::Done);
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

    let lines: Vec<String> = running
        .state
        .log()
        .newest(10)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("IMAGEGEN"))
        .collect();
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
    assert_eq!(started.status, 200);
    assert!(matches!(
        reply(&started.body),
        GenerateReply::Started { job: 1, .. }
    ));
    let mut finished = false;
    for _ in 0..100 {
        let status = get(port, path::GENERATE).await.expect("answers");
        let GenerateReply::Jobs { jobs } = reply(&status.body) else {
            panic!("expected jobs: {}", status.body);
        };
        assert_eq!(jobs.len(), 1);
        if jobs[0].state == JobState::Done {
            finished = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(finished, "the job finished without anyone waiting");
    assert!(running.playground_path("assets/media/hall.png").is_file());
}

#[tokio::test]
async fn each_refusal_has_its_typed_cause_and_status() {
    let running = start(&[]).await;
    let signed_out = serve_route(
        &running,
        generator(FakeBackend::failing_check(BackendError::NotSignedIn(
            "sign in with `CODEX_HOME=<home> codex login`".to_owned(),
        ))),
    )
    .await;
    let refused = post_json(signed_out, path::GENERATE, &body("hall", ""))
        .await
        .expect("answers");
    assert_eq!(refused.status, 503);
    let GenerateReply::Refused { cause, message, .. } = reply(&refused.body) else {
        panic!("expected a refusal: {}", refused.body);
    };
    assert_eq!(cause, RefusalCause::NotSignedIn);
    assert!(message.contains("codex login"));

    let timed_out = serve_route(
        &running,
        generator(FakeBackend::new(Script::Fail(BackendError::Timeout))),
    )
    .await;
    let response = post_json(timed_out, path::GENERATE, &body("hall", r#","wait":true"#))
        .await
        .expect("answers");
    assert_eq!(response.status, 504);
    assert!(matches!(
        reply(&response.body),
        GenerateReply::Finished { job } if job.cause == Some(RefusalCause::Timeout)
    ));

    let service_refused = serve_route(
        &running,
        generator(FakeBackend::new(Script::Fail(BackendError::Refused(
            "refused by the service".to_owned(),
        )))),
    )
    .await;
    let response = post_json(
        service_refused,
        path::GENERATE,
        &body("hall", r#","wait":true"#),
    )
    .await
    .expect("answers");
    assert_eq!(response.status, 502);

    for (text, label) in [
        ("not json", "not JSON"),
        (r#"{"prompt":"a"}"#, "no name"),
        (&body("bad name", ""), "a bad name"),
        (&body("hall", r#","purpose":"poster""#), "a bad purpose"),
        (&body("hall", r#","aspect":"wide""#), "a bad aspect"),
        (r#"{"prompt":"  ","name":"hall"}"#, "an empty prompt"),
    ] {
        let response = post_json(timed_out, path::GENERATE, text)
            .await
            .expect("answers");
        assert_eq!(response.status, 400, "{label}: {}", response.body);
        assert!(
            matches!(
                reply(&response.body),
                GenerateReply::Refused {
                    cause: RefusalCause::BadRequest,
                    ..
                }
            ),
            "{label}"
        );
    }
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
    assert_eq!(bad.status, 400, "the route exists: {}", bad.body);
    assert!(matches!(
        reply(&bad.body),
        GenerateReply::Refused {
            cause: RefusalCause::BadRequest,
            ..
        }
    ));
    let status = get(running.port, path::GENERATE).await.expect("answers");
    assert_eq!(status.status, 200);
    assert!(matches!(reply(&status.body), GenerateReply::Jobs { .. }));
}

#[tokio::test]
async fn the_verb_numbers_its_answer_and_types_its_refusals() {
    let running = start(&[]).await;
    let port = serve_route(
        &running,
        generator(FakeBackend::new(Script::Picture(picture()))),
    )
    .await;
    let done = run_dev(
        port,
        &words("media generate a candlelit hall as=hall purpose=background aspect=16:9 wait=true"),
    )
    .await
    .expect("generates");
    assert!(
        done.starts_with("ok (generation 1) stored hall.png in the media library"),
        "{done}"
    );
    assert!(done.contains("made by fake"), "{done}");
    assert!(done.contains("PNG stays a PNG"), "{done}");

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
        generator(FakeBackend::failing_check(BackendError::NotSignedIn(
            "no login: run `CODEX_HOME=<home> codex login`".to_owned(),
        ))),
    )
    .await;
    let error = run_dev(
        signed_out,
        &words("media generate a hall as=hall wait=true"),
    )
    .await
    .expect_err("refused");
    let DevError::Rejected(text) = error else {
        panic!("expected a typed refusal: {error:?}");
    };
    assert!(text.contains("refused (not_signed_in)"), "{text}");
    assert!(text.contains("codex login"), "{text}");

    let timed_out = serve_route(
        &running,
        generator(FakeBackend::new(Script::Fail(BackendError::Timeout))),
    )
    .await;
    let error = run_dev(timed_out, &words("media generate a hall as=hall wait=true"))
        .await
        .expect_err("timed out");
    let DevError::Rejected(text) = error else {
        panic!("expected a typed refusal: {error:?}");
    };
    assert!(
        text.contains("generation 1 of hall ended without a picture (timeout)"),
        "{text}"
    );

    let bad = run_dev(port, &words("media generate a hall as=hall.png"))
        .await
        .expect_err("not sent");
    assert!(matches!(bad, DevError::Command(_)), "{bad:?}");
}

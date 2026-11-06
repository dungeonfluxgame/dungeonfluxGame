//! The services report over a running route: what it names, that it never shows a secret, how it
//! refuses, the verb's grammar and what the verb prints. No test here can reach a provider.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;

use super::cli::{SERVICES_USAGE, Services, fetch, parse};
use super::format::format_services;
use super::request::{
    ImageMode, ImagegenReport, LocateReport, ServicesError, ServicesReply, ServicesReport,
    TextReport,
};
use super::routes::{path, router_with};
use crate::command::CommandError;
use crate::dev_http::DevReply;
use crate::imagegen::{CodexBackend, CodexSettings, ImageGenerator, SwitchedOff};
use crate::server;
use crate::test_support::{Running, start};
use crate::text::responses_text::ResponsesRoute;
use crate::text::{BackendChoice, BackendKind, TextRuntime};

const SECRET_KEY: &str = "sk-test-never-in-a-report";
const SECRET_HOME: &str = "C:/private/codex-home-never-in-a-report";

/// The services routes over `text` and `generator`, on an ephemeral port.
async fn serve(running: &Running, text: TextRuntime, generator: ImageGenerator) -> u16 {
    let app = router_with(
        Arc::clone(&running.state),
        Arc::new(text),
        Arc::new(generator),
    );
    let listener = server::bind(0).await.expect("binds an ephemeral port");
    let port = listener.local_addr().expect("has an address").port();
    tokio::spawn(async move {
        // The server runs until the test's runtime stops; its end is the test's end.
        let _ = axum::serve(listener, app).await;
    });
    port
}

fn silent() -> Arc<dyn Fn(String) + Send + Sync> {
    Arc::new(|_line: String| {})
}

fn configured_text() -> TextRuntime {
    let route = ResponsesRoute {
        model: "gpt-6-luna".to_owned(),
        key: SECRET_KEY.to_owned(),
    };
    TextRuntime::new(BackendChoice::Responses(Some(route)), silent())
}

fn codex_generator() -> ImageGenerator {
    let settings = CodexSettings {
        executable: None,
        path_directories: Vec::new(),
        codex_home: Some(PathBuf::from(SECRET_HOME)),
        work_root: PathBuf::from("C:/private/work"),
        agent_model: None,
    };
    ImageGenerator::new(Arc::new(CodexBackend::new(settings)))
}

fn done_report(reply: ServicesReply<ServicesReport>) -> ServicesReport {
    match reply {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, message } => panic!("refused: {error:?} {message}"),
    }
}

fn services_lines(running: &Running) -> Vec<String> {
    running
        .state
        .log()
        .newest(100)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("SERVICES"))
        .collect()
}

#[tokio::test]
async fn the_report_names_each_service_and_never_shows_a_key_or_a_home() {
    let running = start(&[]).await;
    let port = serve(&running, configured_text(), codex_generator()).await;
    let response = crate::client::get(port, path::SERVICES)
        .await
        .expect("answers");
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert!(!response.body.contains(SECRET_KEY), "{}", response.body);
    assert!(!response.body.contains("codex-home"), "{}", response.body);

    let report = done_report(fetch(port).await.expect("reads"));
    assert_eq!(report.text.backend, BackendKind::Responses);
    assert_eq!(report.text.profile, "responses:gpt-6-luna");
    assert_eq!(report.text.identity_model.as_deref(), Some("gpt-6-luna"));
    assert_eq!((report.text.configured, report.text.missing), (true, None));
    assert_eq!(report.imagegen.backend, "codex-cli");
    assert_eq!(report.imagegen.mode, ImageMode::Live);
    assert!(report.imagegen.codex_home_set);
    assert!(!report.imagegen.provenance_dir_set);
    assert_eq!(
        (report.imagegen.running, report.imagegen.last_outcome),
        (0, None)
    );
    assert_eq!(report.locate.endpoint_host, "queue.fal.run");
    assert_eq!(
        report.locate.model,
        df_imagery::oneshot_segment::DEFAULT_MODEL
    );
    assert_eq!(report.locate.min_confidence_permille, 600);
    assert!(services_lines(&running).is_empty(), "a read writes no line");
}

#[tokio::test]
async fn an_unconfigured_text_backend_names_the_variable_to_set() {
    let running = start(&[]).await;
    let responses = TextRuntime::new(BackendChoice::Responses(None), silent());
    let port = serve(
        &running,
        responses,
        ImageGenerator::new(Arc::new(SwitchedOff)),
    )
    .await;
    let report = done_report(fetch(port).await.expect("reads"));
    assert!(!report.text.configured);
    assert_eq!(report.text.missing.as_deref(), Some("OPENAI_API_KEY"));
    assert_eq!(report.imagegen.backend, "none");
    assert_eq!(report.imagegen.mode, ImageMode::Off);
    assert!(!report.imagegen.codex_home_set);

    let codex = TextRuntime::new(BackendChoice::Codex(None), silent());
    let port = serve(&running, codex, ImageGenerator::new(Arc::new(SwitchedOff))).await;
    let report = done_report(fetch(port).await.expect("reads"));
    assert_eq!(report.text.backend, BackendKind::Codex);
    assert_eq!(
        report.text.missing.as_deref(),
        Some("DF_TEXT_CODEX_EXECUTABLE")
    );
}

#[tokio::test]
async fn a_query_is_a_typed_refusal_and_one_audit_line() {
    let running = start(&[]).await;
    let port = serve(&running, configured_text(), codex_generator()).await;
    let response = crate::client::get(port, &format!("{}?verbose=1", path::SERVICES))
        .await
        .expect("answers");
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    let reply: ServicesReply<ServicesReport> =
        serde_json::from_str(&response.body).expect("a reply");
    let DevReply::Rejected { error, message } = reply else {
        panic!("refused: {}", response.body);
    };
    assert_eq!(error, ServicesError::InvalidQuery);
    assert_eq!(message, error.to_string());
    assert_eq!(
        services_lines(&running),
        ["SERVICES refused error=invalid_query"]
    );
}

#[tokio::test]
async fn the_router_also_serves_the_imagegen_and_locate_routes_over_the_same_services() {
    let running = start(&[]).await;
    let port = serve(&running, configured_text(), codex_generator()).await;
    let jobs = crate::client::get(port, crate::imagegen::routes::path::GENERATE)
        .await
        .expect("answers");
    assert_eq!(jobs.status, StatusCode::OK, "{}", jobs.body);
    let show = crate::client::get(
        port,
        &format!("{}?picture=x.png", crate::locate::routes::path::LOCATE_SHOW),
    )
    .await
    .expect("answers");
    assert_eq!(show.status, StatusCode::NOT_FOUND, "{}", show.body);
}

#[test]
fn the_verb_takes_no_words() {
    assert_eq!(parse(&[]), Ok(Services));
    assert_eq!(
        parse(&["text".to_owned()]),
        Err(CommandError::Usage(SERVICES_USAGE))
    );
    assert_eq!(SERVICES_USAGE, "services");
}

#[test]
fn the_verb_prints_one_line_of_keys_per_service() {
    let report = ServicesReport {
        text: TextReport {
            backend: BackendKind::Codex,
            profile: "codex".to_owned(),
            identity_model: None,
            configured: false,
            missing: Some("DF_TEXT_CODEX_EXECUTABLE".to_owned()),
            attempts_kept: 2,
            in_flight: 1,
        },
        imagegen: ImagegenReport {
            backend: "codex-cli".to_owned(),
            mode: ImageMode::Live,
            deadline_ms: 900_000,
            provenance_dir_set: true,
            codex_home_set: false,
            running: 1,
            last_outcome: Some("stored".to_owned()),
        },
        locate: LocateReport {
            model: "fal-ai/sam-3-1/image-rle".to_owned(),
            endpoint_host: "queue.fal.run".to_owned(),
            timeout_ms: 600_000,
            key_present: false,
            min_confidence_permille: 600,
        },
    };
    assert_eq!(
        format_services(&report),
        concat!(
            "text: backend=codex profile=codex model=- configured=false ",
            "missing=DF_TEXT_CODEX_EXECUTABLE attempts_kept=2 in_flight=1\n",
            "imagegen: backend=codex-cli mode=live deadline_ms=900000 provenance_dir=set ",
            "codex_home=unset running=1 last_outcome=stored\n",
            "locate: model=fal-ai/sam-3-1/image-rle endpoint_host=queue.fal.run ",
            "timeout_ms=600000 key=missing min_confidence_permille=600"
        )
    );
}

#[test]
fn every_closed_key_is_its_serde_name() {
    for mode in ImageMode::ALL {
        assert_eq!(serde_json::to_value(mode).unwrap(), mode.key());
        assert_eq!(ImageMode::from_key(mode.key()), Some(mode));
    }
    let error = serde_json::to_value(ServicesError::InvalidQuery).unwrap();
    assert_eq!(error["error"], ServicesError::InvalidQuery.key());
}

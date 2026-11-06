//! `GET /dev/services`, and the composition of the media and text routes over one text surface and
//! one picture generator.
//!
//! The route is merged into the host's one router before the layers, so it sits behind the same
//! guard as every route. It answers a `ServicesReply<ServicesReport>`; a successful read writes no
//! line, and a refusal is `SERVICES refused error=<key>`.

use std::sync::Arc;

use axum::Router;
use axum::extract::{RawQuery, State};
use axum::response::Response;
use axum::routing::get;
use df_imagery::oneshot_segment;

use super::request::{
    ImageMode, ImagegenReport, LocateReport, ServicesError, ServicesReport, TextReport,
};
use crate::dev_http::reply;
use crate::imagegen::{ImageBackend, ImageGenerator, SwitchedOff};
use crate::locate::{DEFAULT_MIN_CONFIDENCE_PERMILLE, FAL_KEY_VARIABLES, SCAN_TIMEOUT};
use crate::state::AppState;
use crate::text::TextRuntime;

/// The address of the services report.
pub mod path {
    pub const SERVICES: &str = "/dev/services";
}

struct ServicesRoutes {
    app: Arc<AppState>,
    text: Arc<TextRuntime>,
    generator: Arc<ImageGenerator>,
}

/// A span in whole milliseconds, as reports carry it.
fn whole_ms(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// A count as a report carries it.
fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// The host of an endpoint address: what comes after the scheme, up to the first `/`.
fn endpoint_host(endpoint: &str) -> &str {
    let rest = endpoint
        .split_once("://")
        .map_or(endpoint, |(_, rest)| rest);
    rest.split('/').next().unwrap_or(rest)
}

impl ServicesRoutes {
    fn report(&self) -> ServicesReport {
        let text = self.text.status();
        let generator = &self.generator;
        let backend = generator.backend_name();
        ServicesReport {
            text: TextReport {
                backend: text.backend,
                profile: text.profile,
                identity_model: text.identity_model,
                configured: text.configured,
                missing: text.missing.map(str::to_owned),
                attempts_kept: count(text.attempts_kept),
                in_flight: count(text.in_flight),
            },
            imagegen: ImagegenReport {
                backend: backend.to_owned(),
                mode: if backend == SwitchedOff.name() {
                    ImageMode::Off
                } else {
                    ImageMode::Live
                },
                deadline_ms: whole_ms(generator.deadline()),
                provenance_dir_set: generator.provenance_directory_set(),
                codex_home_set: generator.codex_home_set(),
                running: count(generator.running()),
                last_outcome: generator.last_outcome().map(str::to_owned),
            },
            locate: LocateReport {
                model: oneshot_segment::DEFAULT_MODEL.to_owned(),
                endpoint_host: endpoint_host(oneshot_segment::DEFAULT_ENDPOINT).to_owned(),
                timeout_ms: whole_ms(SCAN_TIMEOUT),
                // Read when asked, as a scan reads it when it starts; only its presence is shown.
                key_present: crate::environment::first_text(&FAL_KEY_VARIABLES).is_some(),
                min_confidence_permille: DEFAULT_MIN_CONFIDENCE_PERMILLE,
            },
        }
    }
}

async fn services(
    State(routes): State<Arc<ServicesRoutes>>,
    RawQuery(query): RawQuery,
) -> Response {
    if query.is_some_and(|query| !query.is_empty()) {
        let error = ServicesError::InvalidQuery;
        routes
            .app
            .record_line(format!("SERVICES refused error={}", error.key()));
        return reply::<ServicesReport, _>(Err(error));
    }
    reply::<_, ServicesError>(Ok(routes.report()))
}

/// The services route and the imagegen and locate routes, over `text` and `generator`.
pub fn router_with(
    app: Arc<AppState>,
    text: Arc<TextRuntime>,
    generator: Arc<ImageGenerator>,
) -> Router {
    let state = Arc::new(ServicesRoutes {
        app: Arc::clone(&app),
        text: Arc::clone(&text),
        generator: Arc::clone(&generator),
    });
    Router::new()
        .route(path::SERVICES, get(services))
        .with_state(state)
        .merge(crate::imagegen::router_with(Arc::clone(&app), generator))
        .merge(crate::locate::router_with(app, text))
}

/// The services, imagegen and locate routes over the text surface and the picture generator this
/// process's environment asks for, each built once.
pub fn router(app: Arc<AppState>) -> Router {
    let text = crate::locate::text_runtime(&app);
    let generator = Arc::new(ImageGenerator::from_environment(&app));
    router_with(app, text, generator)
}

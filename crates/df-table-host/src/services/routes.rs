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
use df_types::saturating_millis;

use super::request::{
    ImageMode, ImagegenReport, LocateReport, ServicesError, ServicesReport, SpeechReport,
    TextReport,
};
use crate::dev_http::reply;
use crate::imagegen::{ImageBackend, ImageGenerator, SwitchedOff};
use crate::locate::{DEFAULT_MIN_CONFIDENCE_PERMILLE, FAL_KEY_VARIABLES, SCAN_TIMEOUT};
use crate::speech::Speech;
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
    speech: Speech,
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
    /// The speech line. A voices file or media library that does not read leaves its count out
    /// and is recorded.
    async fn speech_report(&self) -> SpeechReport {
        match self.speech.status().await {
            Ok(status) => SpeechReport {
                configured: status.configured,
                missing: status.missing.map(|setting| setting.variable().to_owned()),
                tts_model: status.tts_model,
                stt_model: status.stt_model,
                voices: Some(count(status.voices.len())),
                clips_kept: Some(status.clips_kept),
            },
            Err(error) => {
                self.app
                    .record_line(format!("SERVICES speech_unreadable error={}", error.key()));
                let port = self.speech.port_status();
                SpeechReport {
                    configured: port.configured,
                    missing: port.missing.map(|setting| setting.variable().to_owned()),
                    tts_model: crate::speech::provider::TTS_MODEL.to_owned(),
                    stt_model: crate::speech::provider::STT_MODEL.to_owned(),
                    voices: None,
                    clips_kept: None,
                }
            }
        }
    }

    fn report(&self, speech: SpeechReport) -> ServicesReport {
        let text = self.text.status();
        let generator = &self.generator;
        let backend = generator.backend_name();
        ServicesReport {
            text: TextReport {
                backend: text.backend,
                profile: text.profile,
                identity_model: text.identity_model,
                persona_model: text.persona_model,
                persona_profile: text.persona_profile,
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
                deadline_ms: saturating_millis(generator.deadline()),
                provenance_dir_set: generator.provenance_directory_set(),
                codex_home_set: generator.codex_home_set(),
                running: count(generator.running()),
                last_outcome: generator.last_outcome().map(str::to_owned),
            },
            locate: LocateReport {
                model: oneshot_segment::DEFAULT_MODEL.to_owned(),
                endpoint_host: endpoint_host(oneshot_segment::DEFAULT_ENDPOINT).to_owned(),
                timeout_ms: saturating_millis(SCAN_TIMEOUT),
                // Read when asked, as a scan reads it when it starts; only its presence is shown.
                key_present: crate::environment::first_text(&FAL_KEY_VARIABLES).is_some(),
                min_confidence_permille: DEFAULT_MIN_CONFIDENCE_PERMILLE,
            },
            speech,
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
    let speech = routes.speech_report().await;
    reply::<_, ServicesError>(Ok(routes.report(speech)))
}

/// The services route and the imagegen, persona, speech and locate routes, over `text`,
/// `generator` and `speech`.
pub fn router_with(
    app: Arc<AppState>,
    text: Arc<TextRuntime>,
    generator: Arc<ImageGenerator>,
    speech: Speech,
) -> Router {
    let state = Arc::new(ServicesRoutes {
        app: Arc::clone(&app),
        text: Arc::clone(&text),
        generator: Arc::clone(&generator),
        speech: speech.clone(),
    });
    Router::new()
        .route(path::SERVICES, get(services))
        .with_state(state)
        .merge(crate::imagegen::router_with(Arc::clone(&app), generator))
        .merge(crate::persona::router_with(
            Arc::clone(&app),
            Arc::clone(&text),
            speech.clone(),
        ))
        .merge(crate::persona::address::router_with(
            Arc::clone(&app),
            Arc::clone(&text),
        ))
        .merge(crate::speech::router_with(speech))
        .merge(crate::movement::routes::router_with(
            Arc::clone(&app),
            Arc::clone(&text),
        ))
        .merge(crate::locate::router_with(app, text))
}

/// The services, imagegen, persona, speech and locate routes over the text surface, the picture
/// generator and the speech supplier this process's environment asks for, each built once.
pub fn router(app: Arc<AppState>) -> Router {
    let text = crate::locate::text_runtime(&app);
    let generator = Arc::new(ImageGenerator::from_environment(&app));
    let speech = Speech::from_environment(Arc::clone(&app));
    router_with(app, text, generator, speech)
}

//! The `/dev/media/generate` route: the developer API of the picture generator.
//!
//! It is merged into the host's one router before the layers, so it sits behind the same guard as
//! every route (loopback host names, JSON for any change, no foreign origin). A change is a POST
//! of a `GenerateRequest` and the status is a GET; every answer is a `GenerateReply`, and a
//! refusal carries its typed `GenerateError` and the status that goes with it (a mistake of the
//! caller is a 400, a backend that is not set up a 503, a deadline a 504). The route reads the
//! request, hands it to the generator and, when the caller asked to wait, waits for the job; it
//! never runs a backend on a runtime worker. A refusal that never became a job is recorded here,
//! once; a job records its own end.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;

use super::events::ImageGenEvent;
use super::model::ImageRequest;
use super::request::{GenerateError, GenerateRequest, JobView};
use super::service::ImageGenerator;
use crate::dev_http::{parse_body, reply};
use crate::state::AppState;

/// The address of the picture generator's dev API.
pub mod path {
    pub const GENERATE: &str = "/dev/media/generate";
}

struct GenerateRoutes {
    app: Arc<AppState>,
    generator: Arc<ImageGenerator>,
}

impl GenerateRoutes {
    /// Starts the job `body` asks for and, when it asks to wait, waits for the job to end.
    async fn generate(&self, body: GenerateRequest) -> Result<JobView, GenerateError> {
        let request = ImageRequest::new(&body.prompt, &body.name, body.purpose, body.aspect)?;
        let request = match &body.base {
            Some(base) => request.editing(base)?,
            None => request,
        };
        let started = self.generator.start(&self.app, request).await?;
        if !body.wait {
            return Ok(started.view);
        }
        let job = started.view.job;
        started
            .finished
            .await
            .map_err(|_| GenerateError::Stopped { job: Some(job) })?
    }

    /// Records a refusal that never became a job; a job records its own end.
    fn record_refusal(&self, name: Option<String>, error: &GenerateError) {
        if error.job().is_some() {
            return;
        }
        let event = ImageGenEvent::Refused {
            name,
            error: error.clone(),
            backend: self.generator.backend_name(),
        };
        self.app.record_line(event.line());
    }
}

async fn generate(State(routes): State<Arc<GenerateRoutes>>, bytes: Bytes) -> Response {
    let body = parse_body::<GenerateRequest>(&bytes);
    let name = body.as_ref().ok().map(|body| body.name.clone());
    let result = match body {
        Ok(body) => routes.generate(body).await,
        Err(error) => Err(GenerateError::from(error)),
    };
    if let Err(error) = &result {
        routes.record_refusal(name, error);
    }
    reply(result)
}

async fn status(State(routes): State<Arc<GenerateRoutes>>) -> Response {
    reply::<_, GenerateError>(Ok(routes.generator.views()))
}

/// The picture generator's routes over `generator`.
pub fn router_with(app: Arc<AppState>, generator: Arc<ImageGenerator>) -> Router {
    Router::new()
        .route(path::GENERATE, post(generate).get(status))
        .with_state(Arc::new(GenerateRoutes { app, generator }))
}

/// The picture generator's routes with the backend this process's environment asks for.
pub fn router(app: Arc<AppState>) -> Router {
    let generator = Arc::new(ImageGenerator::from_environment(&app));
    router_with(app, generator)
}

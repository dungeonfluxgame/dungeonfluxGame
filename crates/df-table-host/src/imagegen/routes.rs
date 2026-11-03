//! The `/dev/media/generate` route: the developer API of the picture generator.
//!
//! It is merged into the host's one router before the layers, so it sits behind the same guard as
//! every route (loopback host names, JSON for any change, no foreign origin). A change is a POST
//! of JSON and the status is a GET; every answer is a `GenerateReply`, and a refusal carries its
//! typed cause and the status that goes with it (a mistake of the caller is a 400, a backend that
//! is not set up a 503, a deadline a 504). The route reads the request, hands it to the generator
//! and, when the caller asked to wait, waits for the job; it never runs a backend on a runtime
//! worker.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};

use super::model::{GenerateBody, GenerateReply, ImageRequest, JobState, RefusalCause};
use super::service::{Context, ImageGenerator};
use crate::media_library::media_directory;
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
    fn context(&self) -> Context {
        let app = Arc::clone(&self.app);
        Context {
            media_directory: media_directory(self.app.playground().files()),
            record: Arc::new(move |line| {
                app.record_line(line);
                app.echo_new_lines();
            }),
        }
    }
}

fn answer(reply: GenerateReply) -> Response {
    let status = match &reply {
        GenerateReply::Refused { cause, .. } => cause.status(),
        GenerateReply::Finished { job } => match (job.state, job.cause) {
            (JobState::Done, _) => 200,
            (_, Some(cause)) => cause.status(),
            (_, None) => 500,
        },
        GenerateReply::Started { .. } | GenerateReply::Jobs { .. } => 200,
    };
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(reply)).into_response()
}

fn bad_request(message: String) -> Response {
    answer(GenerateReply::Refused {
        job: None,
        cause: RefusalCause::BadRequest,
        message,
    })
}

async fn generate(State(routes): State<Arc<GenerateRoutes>>, bytes: Bytes) -> Response {
    let body: GenerateBody = match serde_json::from_slice(&bytes) {
        Ok(body) => body,
        Err(error) => return bad_request(format!("the request body is not valid: {error}")),
    };
    let request = match ImageRequest::new(
        &body.prompt,
        &body.name,
        body.purpose.as_deref(),
        body.aspect.as_deref(),
    ) {
        Ok(request) => request,
        Err(problem) => return bad_request(problem.to_string()),
    };
    let context = routes.context();
    let started = match routes.generator.start(&context, request).await {
        Ok(started) => started,
        Err(refusal) => {
            return answer(GenerateReply::Refused {
                job: refusal.job,
                cause: refusal.cause,
                message: refusal.message,
            });
        }
    };
    if !body.wait {
        return answer(GenerateReply::Started {
            job: started.job,
            name: started.name,
            deadline_secs: started.deadline.as_secs(),
        });
    }
    match started.finished.await {
        Ok(job) => answer(GenerateReply::Finished { job: Box::new(job) }),
        Err(error) => answer(GenerateReply::Refused {
            job: Some(started.job),
            cause: RefusalCause::Failed,
            message: format!("the generation task stopped: {error}"),
        }),
    }
}

async fn status(State(routes): State<Arc<GenerateRoutes>>) -> Response {
    answer(GenerateReply::Jobs {
        jobs: routes.generator.views(),
    })
}

/// The picture generator's routes over `generator`.
pub fn router_with(app: Arc<AppState>, generator: Arc<ImageGenerator>) -> Router {
    Router::new()
        .route(path::GENERATE, post(generate).get(status))
        .with_state(Arc::new(GenerateRoutes { app, generator }))
}

/// The picture generator's routes with the backend this process's environment asks for.
pub fn router(app: Arc<AppState>) -> Router {
    router_with(app, Arc::new(ImageGenerator::from_environment()))
}

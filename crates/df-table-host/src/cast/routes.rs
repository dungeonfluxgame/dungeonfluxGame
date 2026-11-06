//! The cast routes. Each resolves the shown scene's current layout once ([`shown_layout`]), works
//! by character name, and answers a `CastReply`; a refusal is a typed [`CastError`] naming the
//! character and the names the layout does have. Cues and camera moves are applied through
//! `playground::apply_scene_change`, which records its own refusals; a cast route records only
//! the refusals it finds before that, so each refusal is one audit line.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, post};
use df_playground::lobby::change::ScreenEdit;
use df_playground::scene::SceneChange;
use df_scene_layout::{
    CharacterCue, SceneLayout, SceneRegion, Shot, cast, region_target, shot_zoom_percent,
};

use super::request::{CameraMoved, CameraRequest, CastError, CastList, Cued};
use crate::dev_http::{parse_body, reply};
use crate::locate::shown_backdrop;
use crate::state::AppState;

/// The addresses of the cast dev API.
pub mod path {
    /// The named characters of the shown scene (GET).
    pub const CAST: &str = "/dev/scene/cast";
    /// A cue about a character (POST a `CharacterCue`).
    pub const CUE: &str = "/dev/scene/cue";
    /// Frames a character (POST a `CameraRequest`).
    pub const CAMERA_ON: &str = "/dev/scene/camera-on";
}

/// Answers `result`, recording a refusal first as `CAST refused verb=.. error=<key>`. Only for
/// refusals the cast route found itself, never for one of [`apply`].
fn answer<T: serde::Serialize>(
    app: &AppState,
    verb: &str,
    result: Result<T, CastError>,
) -> Response {
    if let Err(error) = &result {
        app.record_line(format!("CAST refused verb={verb} error={}", error.key()));
    }
    reply(result)
}

/// The backdrop picture the page shows and its layout (`locate::shown_backdrop`).
async fn shown_layout(app: &AppState) -> Result<(String, SceneLayout), CastError> {
    let (fixture, _) = crate::playground::shown_fixture(app).await?;
    let backdrop = shown_backdrop(app, &fixture)
        .await?
        .ok_or(CastError::NoBackdrop)?;
    let picture = backdrop.picture;
    match backdrop.layout {
        Some(layout) => Ok((picture, layout)),
        None => Err(CastError::NoLayout { picture }),
    }
}

/// That `character` is a named character of `layout`, or the error naming the ones it has.
fn named<'a>(
    layout: &'a SceneLayout,
    picture: &str,
    character: &str,
) -> Result<&'a SceneRegion, CastError> {
    layout
        .region(character)
        .ok_or_else(|| CastError::UnknownCharacter {
            character: character.to_owned(),
            picture: picture.to_owned(),
            names: layout.names().into_iter().map(str::to_owned).collect(),
        })
}

/// Applies `change` and answers `applied` with its revision. A refusal is answered unrecorded:
/// `playground::apply_scene_change` wrote its audit line.
async fn apply<T: serde::Serialize>(
    app: &AppState,
    change: SceneChange,
    applied: impl FnOnce(u64) -> T,
) -> Response {
    let result = crate::playground::apply_scene_change(app, change)
        .await
        .map(|reply| applied(reply.revision))
        .map_err(CastError::from);
    reply(result)
}

async fn list(State(app): State<Arc<AppState>>) -> Response {
    let result = shown_layout(&app).await.map(|(picture, layout)| CastList {
        picture,
        members: cast(&layout),
    });
    answer(&app, "cast", result)
}

async fn cue(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let checked = async {
        let cue: CharacterCue = parse_body(&bytes)?;
        let (picture, layout) = shown_layout(&app).await?;
        named(&layout, &picture, &cue.character)?;
        Ok::<_, CastError>(cue)
    }
    .await;
    let cue = match checked {
        Ok(cue) => cue,
        Err(error) => return answer::<Cued>(&app, "cue", Err(error)),
    };
    let edit = ScreenEdit::Cue { cue: cue.clone() };
    apply(&app, SceneChange::Screen { edit }, |revision| Cued {
        cue,
        revision,
    })
    .await
}

async fn camera_on(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let checked = async {
        let request: CameraRequest = parse_body(&bytes)?;
        let shot = request.shot.unwrap_or(Shot::Upper);
        let (picture, layout) = shown_layout(&app).await?;
        let region = named(&layout, &picture, &request.character)?;
        let target = region_target(&request.character, Some(shot));
        let zoom_percent = shot_zoom_percent(region, shot);
        let duration_ms = request.over_ms.unwrap_or(crate::command::DEFAULT_CAMERA_MS);
        Ok::<_, CastError>((target, zoom_percent, duration_ms))
    }
    .await;
    let (target, zoom_percent, duration_ms) = match checked {
        Ok(checked) => checked,
        Err(error) => return answer::<CameraMoved>(&app, "camera", Err(error)),
    };
    let change = SceneChange::Camera {
        target: Some(target.clone()),
        zoom_percent,
        duration_ms,
    };
    apply(&app, change, |revision| CameraMoved {
        target,
        zoom_percent,
        duration_ms,
        revision,
    })
    .await
}

/// The cast routes.
pub fn router(app: Arc<AppState>) -> Router {
    Router::new()
        .route(path::CAST, get(list))
        .route(path::CUE, post(cue))
        .route(path::CAMERA_ON, post(camera_on))
        .with_state(app)
}

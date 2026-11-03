//! The playground's routes: what it shows, the live scene, and the fixture commands.
//!
//! The playground (`crates/df-playground`) is a browser page that draws the shared screen and a
//! phone from fixture files. This module owns the routes that change what it shows and tells the
//! pages when anything changed. Its files are served by `files`, what it shows is kept by
//! `controls_state`, the pages that are open by `clients`, and the audit lines are written by
//! `audit`. It never reads the canvas except for the names on the stage, and never interprets a
//! file but a fixture it is about to show or write.
//!
//! A GET never changes anything. When a fixture file is written or removed, and at startup, the
//! host reconciles what is shown with the files, as a recorded change that wakes the pages.

mod audit;
mod clients;
mod controls_state;
mod files;

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use df_playground::controls::{ControlsChange, ControlsReply};
use df_playground::fixture::{ElementSource, Fixture};
use df_playground::scene::SceneChange;
use df_table::event::RejectionCause;

pub(crate) use self::audit::playground_line;
use self::audit::{controls_pairs, reject_line, scene_line};
pub use self::clients::{ClientNameError, check_client_name};
pub use self::controls_state::{ControlsRefusal, PlaygroundState, SceneRefusal};
pub(crate) use self::files::problem_from;
use self::files::{FIXTURE_DIRECTORY, FIXTURE_EXTENSIONS, current_fixture_names};
pub use self::files::{
    FileProblem, PlaygroundFiles, content_type, fixture_names, is_plain_file_name, read_file,
    revision,
};
use crate::limits::MAXIMUM_FIXTURE_NAME_LENGTH;
use crate::media_library::{media_directory, media_entries, media_problem};
use crate::state::AppState;
use crate::wire::{ControlsBatch, FixtureRemove, FixtureWrite, Reply, route};

pub(crate) fn refused(status: StatusCode, message: String) -> Response {
    (
        status,
        Json(Reply::Rejected {
            cause: None,
            message,
            problems: Vec::new(),
        }),
    )
        .into_response()
}

pub(crate) fn done(state: &AppState) -> Response {
    Json(Reply::Done {
        revision: state.playground().revision(),
        notices: Vec::new(),
    })
    .into_response()
}

pub(crate) fn reject_malformed(state: &AppState, error: &serde_json::Error) -> Response {
    state.record_line(reject_line(RejectionCause::MalformedRequest, &[]));
    state.echo_new_lines();
    refused(
        StatusCode::BAD_REQUEST,
        format!("the request is not valid: {error}"),
    )
}

/// Why a fixture name is not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureNameError {
    name: String,
}

impl fmt::Display for FixtureNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "'{}' is not a fixture name: use letters, digits, '_', '-' and '.', up to {MAXIMUM_FIXTURE_NAME_LENGTH} characters",
            self.name
        )
    }
}

/// `tavern` and `tavern.json` both name the file `tavern.json`. Anything that could leave the
/// fixture directory, or is not a plain name, is refused.
pub fn fixture_file_name(name: &str) -> Result<String, FixtureNameError> {
    let file_name = if name.ends_with(".json") {
        name.to_owned()
    } else {
        format!("{name}.json")
    };
    let stem = file_name.trim_end_matches(".json");
    if stem.is_empty()
        || file_name.len() > MAXIMUM_FIXTURE_NAME_LENGTH
        || !is_plain_file_name(&file_name)
    {
        return Err(FixtureNameError {
            name: name.to_owned(),
        });
    }
    Ok(file_name)
}

/// Replaces what is shown with the first fixture when the chosen one is gone (or none was chosen
/// and one exists), and records the change as `PLAYGROUND reconcile fixture=.. revision=..`. The
/// caller announces it: every POST does.
pub(crate) fn reconcile_fixtures(state: &AppState) -> Result<Option<ControlsReply>, FileProblem> {
    let names = fixture_names(state.playground().files())?;
    // The line is written with the playground's lock held, so it is in revision order.
    let reconciled = state.playground().reconcile(&names, |reply| {
        state.record_line(playground_line(
            "reconcile",
            &[
                (
                    "fixture",
                    reply
                        .controls
                        .fixture
                        .clone()
                        .unwrap_or_else(|| "none".to_owned()),
                ),
                ("revision", reply.revision.to_string()),
            ],
        ));
    });
    Ok(reconciled)
}

/// `reconcile_fixtures` off the async threads. A directory that cannot be read is refused with
/// its reason, so the write that came before it still shows the problem.
async fn reconcile_after_files_changed(state: &Arc<AppState>) -> Result<(), FileProblem> {
    state.playground().fixture_files_changed();
    let worker = Arc::clone(state);
    tokio::task::spawn_blocking(move || reconcile_fixtures(&worker))
        .await
        .unwrap_or_else(|error| Err(FileProblem::Unreadable(error.to_string())))
        .map(|_| ())
}

/// What the playground shows now: the same snapshot the frame carries. A read changes nothing.
async fn controls_json(State(state): State<Arc<AppState>>) -> Json<ControlsReply> {
    Json(state.playground().snapshot())
}

/// Reads the fixture names in the changes as file names, so `tavern` means `tavern.json`.
fn with_fixture_files(
    changes: Vec<ControlsChange>,
) -> Result<Vec<ControlsChange>, FixtureNameError> {
    changes
        .into_iter()
        .map(|change| match change {
            ControlsChange::Fixture(name) => fixture_file_name(&name).map(ControlsChange::Fixture),
            other => Ok(other),
        })
        .collect()
}

/// Applies the changes together or not at all, and answers. A change that counts writes one
/// `PLAYGROUND set` line naming each part, with the playground's lock held so the log is in
/// revision order; one that alters nothing writes none; a refusal writes a reject line with its
/// reason.
async fn apply_controls(state: &Arc<AppState>, changes: Vec<ControlsChange>) -> Response {
    let changes = match with_fixture_files(changes) {
        Ok(changes) => changes,
        Err(error) => return refused(StatusCode::BAD_REQUEST, error.to_string()),
    };
    // The generation is read before the names: a fixture written or removed after this makes the
    // choice of a fixture refuse as changed instead of applying to a list that is out of date.
    let generation = state.playground().generation();
    let names = match current_fixture_names(state.playground().files().clone()).await {
        Ok(names) => names,
        Err(problem) => return refused(problem.status(), problem.message()),
    };
    let response =
        match state
            .playground()
            .change_all(changes.clone(), &names, generation, |reply| {
                let mut pairs = controls_pairs(&changes);
                pairs.push(("revision", reply.revision.to_string()));
                state.record_line(playground_line("set", &pairs));
            }) {
            Ok(reply) => Json(reply).into_response(),
            Err(refusal) => {
                state.record_line(reject_line(
                    RejectionCause::InvalidValue,
                    &[("reason", refusal.reason().to_owned())],
                ));
                refused(refusal.status(), refusal.to_string())
            }
        };
    state.echo_new_lines();
    response
}

async fn change_controls(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    match serde_json::from_slice::<ControlsChange>(&body) {
        Ok(change) => apply_controls(&state, vec![change]).await,
        Err(error) => reject_malformed(&state, &error),
    }
}

/// Several changes of the controls as one: all are applied, or none is.
async fn change_controls_batch(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    match serde_json::from_slice::<ControlsBatch>(&body) {
        Ok(batch) => apply_controls(&state, batch.changes).await,
        Err(error) => reject_malformed(&state, &error),
    }
}

/// Reads and checks the fixture `name` in `directory`, on the blocking pool.
async fn read_fixture(
    directory: std::path::PathBuf,
    name: String,
) -> Result<Fixture, (StatusCode, String)> {
    let read_name = name.clone();
    let loaded =
        tokio::task::spawn_blocking(move || read_file(&directory, &read_name, &FIXTURE_EXTENSIONS))
            .await
            .unwrap_or_else(|error| Err(FileProblem::Unreadable(error.to_string())));
    match loaded {
        Ok(bytes) => Fixture::parse(&String::from_utf8_lossy(&bytes)).map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                format!(
                    "{name}, line {}, column {}: {}",
                    error.line, error.column, error.message
                ),
            )
        }),
        Err(problem) => Err((problem.status(), problem.message())),
    }
}

/// The ids of the elements on the stage a scene change would be drawn on.
fn stage_ids(state: &AppState, base: &Fixture) -> Vec<String> {
    match &base.tv.elements {
        ElementSource::Inline(items) => items.iter().map(|item| item.id.clone()).collect(),
        ElementSource::Live(_) => state
            .canvas()
            .elements()
            .map(|element| element.id.to_string())
            .collect(),
    }
}

/// Why a scene change was not applied: the status a route answers with, and the words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SceneApplyError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
}

impl From<(StatusCode, String)> for SceneApplyError {
    fn from((status, message): (StatusCode, String)) -> Self {
        Self { status, message }
    }
}

/// Records the refusal of a scene change that is the caller's mistake.
fn refuse_scene_change(state: &AppState, reason: &'static str, message: String) -> SceneApplyError {
    state.record_line(reject_line(
        RejectionCause::InvalidValue,
        &[("reason", reason.to_owned())],
    ));
    SceneApplyError {
        status: StatusCode::BAD_REQUEST,
        message,
    }
}

/// Reads the fixture the live scene is drawn on, with the generation it was read at.
async fn shown_fixture(state: &AppState) -> Result<(Fixture, u64), SceneApplyError> {
    let Some((name, generation)) = state.playground().scene_target() else {
        return Err(SceneApplyError {
            status: StatusCode::BAD_REQUEST,
            message: "no fixture is shown; write one with `fixture write` first".to_owned(),
        });
    };
    let directory = state.playground().files().assets().join(FIXTURE_DIRECTORY);
    let base = read_fixture(directory, name).await?;
    Ok((base, generation))
}

/// The ids of the elements on the stage the live scene is drawn on now.
pub(crate) async fn current_stage_ids(state: &AppState) -> Result<Vec<String>, SceneApplyError> {
    let (base, _) = shown_fixture(state).await?;
    Ok(stage_ids(state, &base))
}

/// Applies one change to the live scene, checked against the media library and the stage, and
/// records it. This is the one path of `POST /dev/playground/scene` and of the Director, so a
/// dev verb and the engine make a camera move with the same checks and the same audit line.
pub(crate) async fn apply_scene_change(
    state: &AppState,
    change: SceneChange,
) -> Result<ControlsReply, SceneApplyError> {
    let (base, generation) = shown_fixture(state).await?;
    let media_directory = media_directory(state.playground().files());
    let library = tokio::task::spawn_blocking(move || media_entries(&media_directory))
        .await
        .unwrap_or_else(|error| Err(FileProblem::Unreadable(error.to_string())));
    let library = library.map_err(|problem| SceneApplyError {
        status: problem.status(),
        message: problem.message(),
    })?;
    if let Some(message) = media_problem(&change, &library) {
        return Err(refuse_scene_change(state, "no_such_media", message));
    }
    if let Some(message) = camera_problem(&change, &stage_ids(state, &base)) {
        return Err(refuse_scene_change(state, "no_such_element", message));
    }
    // The change is applied only if the fixture is still the one it was checked against.
    state
        .playground()
        .change_scene(change.clone(), &base, generation, |reply| {
            state.record_line(scene_line(&change, &reply.scene, reply.revision));
        })
        .map_err(|refusal| {
            state.record_line(reject_line(
                RejectionCause::InvalidValue,
                &[("reason", refusal.reason().to_owned())],
            ));
            SceneApplyError {
                status: refusal.status(),
                message: refusal.to_string(),
            }
        })
}

async fn change_scene(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let change: SceneChange = match serde_json::from_slice(&body) {
        Ok(change) => change,
        Err(error) => return reject_malformed(&state, &error),
    };
    let response = match apply_scene_change(&state, change).await {
        Ok(reply) => Json(reply).into_response(),
        Err(error) => refused(error.status, error.message),
    };
    state.echo_new_lines();
    response
}

/// Why a camera move cannot be made with the elements on the stage, or `None` when it can: the
/// camera looks at an element that is there, and the message names the ones that are.
pub fn camera_problem(change: &SceneChange, stage_ids: &[String]) -> Option<String> {
    let SceneChange::Camera {
        target: Some(target),
        ..
    } = change
    else {
        return None;
    };
    if stage_ids.contains(target) {
        return None;
    }
    Some(if stage_ids.is_empty() {
        format!("there is no element '{target}' on the stage: the stage is empty")
    } else {
        format!(
            "there is no element '{target}' on the stage; the elements are: {}",
            stage_ids.join(", ")
        )
    })
}

/// A name for a temporary file that no other write of this host shares, so two writes of the same
/// fixture at once each fill their own file and the last rename wins whole.
fn temporary_name(file_name: &str) -> String {
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let ordinal = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!(".{file_name}.{}.{ordinal}.tmp", std::process::id())
}

/// Pulls the camera back to the whole stage if it looks at one of the elements in `gone`
/// (despawned or cleared), as a recorded change: `PLAYGROUND camera_wide reason=target_gone
/// element=.. over=0 serial=.. revision=..`. The caller announces it.
pub(crate) fn release_camera_from(state: &AppState, gone: &[String]) {
    state
        .playground()
        .release_camera_from(gone, |reply, element| {
            let serial = reply.scene.camera_count;
            state.record_line(playground_line(
                "camera_wide",
                &[
                    ("reason", "target_gone".to_owned()),
                    ("element", element.to_owned()),
                    ("over", "0".to_owned()),
                    ("serial", serial.to_string()),
                    ("revision", reply.revision.to_string()),
                ],
            ));
        });
}

/// Writes `text` to `directory/file_name` through a temporary file of its own, so a reader never
/// sees half of it and a concurrent write of the same name never shares the file.
fn write_fixture_file(directory: &Path, file_name: &str, text: &str) -> Result<(), FileProblem> {
    std::fs::create_dir_all(directory).map_err(|error| problem_from(&error))?;
    let temporary = directory.join(temporary_name(file_name));
    std::fs::write(&temporary, text).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        problem_from(&error)
    })?;
    std::fs::rename(&temporary, directory.join(file_name)).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        problem_from(&error)
    })
}

async fn write_fixture(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let request: FixtureWrite = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => return reject_malformed(&state, &error),
    };
    let file_name = match fixture_file_name(&request.name) {
        Ok(file_name) => file_name,
        Err(error) => return refused(StatusCode::BAD_REQUEST, error.to_string()),
    };
    if let Err(error) = Fixture::parse(&request.text) {
        state.record_line(reject_line(
            RejectionCause::InvalidValue,
            &[("fixture", file_name.clone())],
        ));
        state.echo_new_lines();
        return refused(
            StatusCode::BAD_REQUEST,
            format!(
                "{file_name}, line {}, column {}: {}",
                error.line, error.column, error.message
            ),
        );
    }
    let directory = state.playground().files().assets().join(FIXTURE_DIRECTORY);
    let bytes = request.text.len();
    let name = file_name.clone();
    let written =
        tokio::task::spawn_blocking(move || write_fixture_file(&directory, &name, &request.text))
            .await
            .unwrap_or_else(|error| Err(FileProblem::Unreadable(error.to_string())));
    let response = match written {
        Ok(()) => {
            state.record_line(playground_line(
                "fixture_write",
                &[("name", file_name), ("bytes", bytes.to_string())],
            ));
            match reconcile_after_files_changed(&state).await {
                Ok(()) => done(&state),
                Err(problem) => refused(problem.status(), problem.message()),
            }
        }
        Err(problem) => refused(problem.status(), problem.message()),
    };
    state.echo_new_lines();
    response
}

async fn remove_fixture(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let request: FixtureRemove = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => return reject_malformed(&state, &error),
    };
    let file_name = match fixture_file_name(&request.name) {
        Ok(file_name) => file_name,
        Err(error) => return refused(StatusCode::BAD_REQUEST, error.to_string()),
    };
    let path = state
        .playground()
        .files()
        .assets()
        .join(FIXTURE_DIRECTORY)
        .join(&file_name);
    let removed = tokio::task::spawn_blocking(move || std::fs::remove_file(path))
        .await
        .unwrap_or_else(|error| Err(std::io::Error::other(error.to_string())));
    let response = match removed {
        Ok(()) => {
            state.record_line(playground_line("fixture_remove", &[("name", file_name)]));
            match reconcile_after_files_changed(&state).await {
                Ok(()) => done(&state),
                Err(problem) => refused(problem.status(), problem.message()),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => refused(
            StatusCode::BAD_REQUEST,
            format!("there is no fixture '{file_name}'"),
        ),
        Err(error) => {
            let problem = problem_from(&error);
            refused(problem.status(), problem.message())
        }
    };
    state.echo_new_lines();
    response
}

/// The playground's routes: its files, what it is showing, and the fixture commands. The page
/// reads the canvas from the host's own `/canvas.json`.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(route::PLAYGROUND, get(files::page))
        .route(route::PLAYGROUND_SLASH, get(files::page))
        .route(route::PLAYGROUND_THEME, get(files::theme))
        .route(route::PLAYGROUND_REVISION, get(files::revision_text))
        .route(route::PLAYGROUND_FIXTURE_LIST, get(files::fixture_list))
        .route(route::PLAYGROUND_FIXTURE, get(files::fixture))
        .route(route::PLAYGROUND_PACKAGE_FILE, get(files::package_file))
        .route(route::PLAYGROUND_CONTROLS, get(controls_json))
        .route(route::DEV_PLAYGROUND_CONTROLS, post(change_controls))
        .route(
            route::DEV_PLAYGROUND_CONTROLS_BATCH,
            post(change_controls_batch),
        )
        .route(route::DEV_PLAYGROUND_SCENE, post(change_scene))
        .route(route::PLAYGROUND_CLIENTS, get(clients::clients_json))
        .route(route::DEV_CLIENT_REPORT, post(clients::receive_report))
        .route(route::DEV_FIXTURE_WRITE, post(write_fixture))
        .route(route::DEV_FIXTURE_REMOVE, post(remove_fixture))
        .with_state(state)
}

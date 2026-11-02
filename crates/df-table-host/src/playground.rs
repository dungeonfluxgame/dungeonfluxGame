//! Serving the playground: its page, theme, fixtures and compiled package, straight from disk.
//!
//! The playground (`crates/df-playground`) is a browser page that draws the shared screen and a
//! phone from fixture files. This module only hands those files to the browser and tells it when
//! any of them changed, so a style or fixture edit shows without a rebuild. It never reads the
//! canvas and never interprets a file. Names that reach the disk are checked first, and every
//! answer says not to cache it.

use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Instant, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use df_playground::controls::{Controls, ControlsChange};
use df_playground::fixture::{ElementSource, Fixture};
use df_playground::media::{ClientReport, ClientReportPost};
use df_playground::scene::{SceneChange, SceneOverlay};
use df_table::event::RejectionCause;

use crate::media_library::{media_directory, media_entries, media_problem};
use crate::state::AppState;
use crate::wire::{ClientView, ControlsReply, FixtureRemove, FixtureWrite, Reply, route};

/// Largest file served, in bytes. The compiled package is the biggest file.
pub const MAXIMUM_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// Longest file name accepted from a request.
const MAXIMUM_NAME_LENGTH: usize = df_playground::media::MAXIMUM_NAME_LENGTH;
const INDEX_FILE: &str = "index.html";
const THEME_FILE: &str = "theme.css";
const FIXTURE_DIRECTORY: &str = "fixtures";
const FIXTURE_EXTENSIONS: [&str; 1] = ["json"];
const PACKAGE_EXTENSIONS: [&str; 2] = ["js", "wasm"];
const ASSET_EXTENSIONS: [&str; 2] = ["html", "css"];

/// Where the playground's files are: the hand-written assets and the compiled package.
#[derive(Debug, Clone)]
pub struct PlaygroundFiles {
    assets: PathBuf,
    package: PathBuf,
}

impl PlaygroundFiles {
    pub fn new(assets: PathBuf, package: PathBuf) -> Self {
        Self { assets, package }
    }

    pub fn assets(&self) -> &Path {
        &self.assets
    }

    pub fn package(&self) -> &Path {
        &self.package
    }
}

/// What the playground is showing, and how many changes it has had (so a reader can tell when to
/// look again).
#[derive(Debug)]
pub struct PlaygroundState {
    files: PlaygroundFiles,
    shown: Mutex<ControlsReply>,
    /// What each open page last said it was showing and playing, by the page's own name.
    clients: Mutex<BTreeMap<String, (Instant, ClientReport)>>,
}

/// Most pages remembered at once; a new page pushes out the one heard from longest ago.
pub const MAXIMUM_CLIENTS: usize = 8;
/// A page not heard from for this long is forgotten: it sends a sign of life every ten seconds,
/// so silence means it was closed.
pub const FORGET_CLIENT_AFTER: std::time::Duration = std::time::Duration::from_secs(45);
/// Longest name a page may give itself.
const MAXIMUM_CLIENT_NAME: usize = 40;

impl PlaygroundState {
    pub fn new(files: PlaygroundFiles) -> Self {
        Self {
            files,
            shown: Mutex::new(ControlsReply {
                revision: 0,
                controls: Controls::default(),
                scene: SceneOverlay::default(),
            }),
            clients: Mutex::new(BTreeMap::new()),
        }
    }

    fn lock_clients(&self) -> MutexGuard<'_, BTreeMap<String, (Instant, ClientReport)>> {
        self.clients.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Remembers what a page said. The report is bounded, and a ninth page replaces the page
    /// heard from longest ago.
    pub fn record_client(&self, client: String, report: ClientReport) {
        let mut clients = self.lock_clients();
        if !clients.contains_key(&client)
            && clients.len() >= MAXIMUM_CLIENTS
            && let Some(oldest) = clients
                .iter()
                .min_by_key(|(_, (heard, _))| *heard)
                .map(|(name, _)| name.clone())
        {
            clients.remove(&oldest);
        }
        clients.insert(client, (Instant::now(), report.bounded()));
    }

    /// The pages heard from, by name, with how long ago each last spoke.
    pub fn clients(&self) -> Vec<ClientView> {
        self.clients_at(Instant::now())
    }

    /// `clients` as of `now`, which tests move forward. A page silent for `FORGET_CLIENT_AFTER` is
    /// dropped from the answer and from memory.
    pub fn clients_at(&self, now: Instant) -> Vec<ClientView> {
        let mut clients = self.lock_clients();
        clients.retain(|_, (heard, _)| now.saturating_duration_since(*heard) < FORGET_CLIENT_AFTER);
        clients
            .iter()
            .map(|(client, (heard, report))| ClientView {
                client: client.clone(),
                age_ms: u64::try_from(now.saturating_duration_since(*heard).as_millis())
                    .unwrap_or(u64::MAX),
                report: report.clone(),
            })
            .collect()
    }

    pub fn files(&self) -> &PlaygroundFiles {
        &self.files
    }

    fn lock(&self) -> MutexGuard<'_, ControlsReply> {
        self.shown.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    /// What is shown now, as it is, without looking at the fixture files.
    pub fn snapshot(&self) -> ControlsReply {
        self.lock().clone()
    }

    /// The name of the fixture being shown, if one is.
    pub fn current_fixture(&self) -> Option<String> {
        self.lock().controls.fixture.clone()
    }

    /// Makes one change to the live scene on top of `base`, the fixture being shown. A change
    /// that alters nothing does not count.
    pub fn change_scene(
        &self,
        change: SceneChange,
        base: &Fixture,
    ) -> Result<ControlsReply, String> {
        let mut shown = self.lock();
        let mut next = shown.scene.clone();
        next.apply(change, base)
            .map_err(|error| error.to_string())?;
        if next != shown.scene {
            shown.scene = next;
            shown.revision += 1;
        }
        Ok(shown.clone())
    }

    /// What is shown now, with the fixture replaced by the first of `names` when the chosen one
    /// is missing from them. That replacement counts as a change.
    pub fn reconciled(&self, names: &[String]) -> ControlsReply {
        let mut shown = self.lock();
        let present = shown
            .controls
            .fixture
            .as_ref()
            .is_some_and(|chosen| names.contains(chosen));
        if !present && shown.controls.fixture != names.first().cloned() {
            shown.controls.fixture = names.first().cloned();
            shown.scene = shown.scene.cleared();
            shown.revision += 1;
        }
        shown.clone()
    }

    /// Makes one change. Choosing a fixture that does not exist is refused with the names that do.
    pub fn change(
        &self,
        change: ControlsChange,
        names: &[String],
    ) -> Result<ControlsReply, String> {
        if let ControlsChange::Fixture(name) = &change
            && !names.contains(name)
        {
            return Err(format!(
                "there is no fixture '{name}'; the fixtures are: {}",
                if names.is_empty() {
                    "none".to_owned()
                } else {
                    names.join(", ")
                }
            ));
        }
        let mut shown = self.lock();
        let mut next = shown.controls.clone();
        next.apply(change).map_err(|error| error.to_string())?;
        if next != shown.controls {
            // Another scenario starts from its own file, without the last one's live changes.
            if next.fixture != shown.controls.fixture {
                shown.scene = shown.scene.cleared();
            }
            shown.controls = next;
            shown.revision += 1;
        }
        Ok(shown.clone())
    }
}

/// Why a file was not served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileProblem {
    /// The name is not a plain file name of an allowed type.
    BadName,
    NotFound,
    TooLarge,
    Unreadable(String),
}

impl FileProblem {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadName | Self::NotFound => StatusCode::NOT_FOUND,
            Self::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::Unreadable(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::BadName => "no such playground file".to_owned(),
            Self::NotFound => {
                "that playground file is missing (is the directory right? see df-table serve --help)"
                    .to_owned()
            }
            Self::TooLarge => format!("the file is larger than {MAXIMUM_FILE_BYTES} bytes"),
            Self::Unreadable(message) => format!("the file could not be read: {message}"),
        }
    }
}

pub(crate) fn problem_from(error: &std::io::Error) -> FileProblem {
    match error.kind() {
        std::io::ErrorKind::NotFound => FileProblem::NotFound,
        _ => FileProblem::Unreadable(error.to_string()),
    }
}

pub use df_playground::media::is_plain_file_name;

fn extension_of(name: &str) -> Option<&str> {
    name.rsplit_once('.').map(|(_, extension)| extension)
}

fn has_extension(name: &str, extensions: &[&str]) -> bool {
    extension_of(name).is_some_and(|extension| extensions.contains(&extension))
}

/// The media type of a served file, by extension.
pub fn content_type(name: &str) -> &'static str {
    match extension_of(name) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("js") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// Reads `name` from `directory` if it is a plain file name with one of `extensions`.
pub fn read_file(
    directory: &Path,
    name: &str,
    extensions: &[&str],
) -> Result<Vec<u8>, FileProblem> {
    if !is_plain_file_name(name) || !has_extension(name, extensions) {
        return Err(FileProblem::BadName);
    }
    let path = directory.join(name);
    let metadata = std::fs::metadata(&path).map_err(|error| problem_from(&error))?;
    if !metadata.is_file() {
        return Err(FileProblem::NotFound);
    }
    if metadata.len() > MAXIMUM_FILE_BYTES {
        return Err(FileProblem::TooLarge);
    }
    std::fs::read(&path).map_err(|error| problem_from(&error))
}

/// The plain files in `directory` with one of `extensions`, sorted by name. A missing directory
/// has none.
fn plain_files(
    directory: &Path,
    extensions: &[&str],
) -> Result<Vec<(String, std::fs::Metadata)>, FileProblem> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(problem_from(&error)),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| problem_from(&error))?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !is_plain_file_name(&name) || !has_extension(&name, extensions) {
            continue;
        }
        let metadata = entry.metadata().map_err(|error| problem_from(&error))?;
        if metadata.is_file() {
            files.push((name, metadata));
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

/// The names of the fixture files, sorted.
pub fn fixture_names(files: &PlaygroundFiles) -> Result<Vec<String>, FileProblem> {
    let directory = files.assets.join(FIXTURE_DIRECTORY);
    Ok(plain_files(&directory, &FIXTURE_EXTENSIONS)?
        .into_iter()
        .map(|(name, _)| name)
        .collect())
}

fn stamp(
    directory: &Path,
    extensions: &[&str],
    hasher: &mut DefaultHasher,
) -> Result<(), FileProblem> {
    for (name, metadata) in plain_files(directory, extensions)? {
        name.hash(hasher);
        metadata.len().hash(hasher);
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |elapsed| elapsed.as_nanos());
        modified.hash(hasher);
    }
    Ok(())
}

/// A text that changes when any playground file does, as `<assets>.<package>`: the page reloads
/// its styles and fixtures when the first part changes and reloads itself when the second does.
pub fn revision(files: &PlaygroundFiles) -> Result<String, FileProblem> {
    let mut assets = DefaultHasher::new();
    stamp(&files.assets, &ASSET_EXTENSIONS, &mut assets)?;
    stamp(
        &files.assets.join(FIXTURE_DIRECTORY),
        &FIXTURE_EXTENSIONS,
        &mut assets,
    )?;
    let mut package = DefaultHasher::new();
    stamp(&files.package, &PACKAGE_EXTENSIONS, &mut package)?;
    Ok(format!(
        "{:016x}.{:016x}",
        assets.finish(),
        package.finish()
    ))
}

fn respond(result: Result<(Vec<u8>, &'static str), FileProblem>) -> Response {
    let (status, content_type, body) = match result {
        Ok((body, content_type)) => (StatusCode::OK, content_type, body),
        Err(problem) => (
            problem.status(),
            "text/plain; charset=utf-8",
            problem.message().into_bytes(),
        ),
    };
    let mut response = (status, body).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// Runs blocking file work off the async threads.
async fn blocking(
    work: impl FnOnce() -> Result<(Vec<u8>, &'static str), FileProblem> + Send + 'static,
) -> Response {
    respond(
        tokio::task::spawn_blocking(work)
            .await
            .unwrap_or_else(|error| Err(FileProblem::Unreadable(error.to_string()))),
    )
}

async fn asset(files: PlaygroundFiles, name: &'static str) -> Response {
    blocking(move || {
        read_file(&files.assets, name, &ASSET_EXTENSIONS).map(|bytes| (bytes, content_type(name)))
    })
    .await
}

async fn page(State(state): State<Arc<AppState>>) -> Response {
    asset(state.playground().files().clone(), INDEX_FILE).await
}

async fn theme(State(state): State<Arc<AppState>>) -> Response {
    asset(state.playground().files().clone(), THEME_FILE).await
}

async fn fixture(State(state): State<Arc<AppState>>, UrlPath(name): UrlPath<String>) -> Response {
    let files = state.playground().files().clone();
    blocking(move || {
        read_file(
            &files.assets.join(FIXTURE_DIRECTORY),
            &name,
            &FIXTURE_EXTENSIONS,
        )
        .map(|bytes| (bytes, content_type(&name)))
    })
    .await
}

async fn package_file(
    State(state): State<Arc<AppState>>,
    UrlPath(name): UrlPath<String>,
) -> Response {
    let files = state.playground().files().clone();
    blocking(move || {
        read_file(&files.package, &name, &PACKAGE_EXTENSIONS)
            .map(|bytes| (bytes, content_type(&name)))
    })
    .await
}

async fn fixture_list(State(state): State<Arc<AppState>>) -> Response {
    let files = state.playground().files().clone();
    blocking(move || {
        let names = fixture_names(&files)?;
        let text = serde_json::to_string(&names)
            .map_err(|error| FileProblem::Unreadable(error.to_string()))?;
        Ok((text.into_bytes(), content_type("list.json")))
    })
    .await
}

async fn revision_text(State(state): State<Arc<AppState>>) -> Response {
    let files = state.playground().files().clone();
    blocking(move || revision(&files).map(|text| (text.into_bytes(), "text/plain; charset=utf-8")))
        .await
}

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

/// The fixture names, read off the async threads.
async fn current_fixture_names(files: PlaygroundFiles) -> Result<Vec<String>, FileProblem> {
    tokio::task::spawn_blocking(move || fixture_names(&files))
        .await
        .unwrap_or_else(|error| Err(FileProblem::Unreadable(error.to_string())))
}

/// What the playground shows now. A fixture that is not chosen, or no longer exists, is replaced
/// by the first one, so the page always has something to draw while any fixture exists.
async fn controls_json(State(state): State<Arc<AppState>>) -> Response {
    let names = match current_fixture_names(state.playground().files().clone()).await {
        Ok(names) => names,
        Err(problem) => return refused(problem.status(), problem.message()),
    };
    Json(state.playground().reconciled(&names)).into_response()
}

/// A line for the audit log: the area, the verb and `key=value` pairs, like the canvas's lines.
pub(crate) fn playground_line(verb: &str, pairs: &[(&str, String)]) -> String {
    let mut line = format!("PLAYGROUND {verb}");
    for (key, value) in pairs {
        line.push_str(&format!(" {key}={value}"));
    }
    line
}

pub(crate) fn reject_malformed(state: &AppState, error: &serde_json::Error) -> Response {
    state.record_line(playground_line(
        "reject",
        &[("cause", RejectionCause::MalformedRequest.key().to_owned())],
    ));
    state.echo_new_lines();
    refused(
        StatusCode::BAD_REQUEST,
        format!("the request is not valid: {error}"),
    )
}

async fn change_controls(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let change: ControlsChange = match serde_json::from_slice(&body) {
        Ok(change) => change,
        Err(error) => return reject_malformed(&state, &error),
    };
    let change = match change {
        ControlsChange::Fixture(name) => match fixture_file_name(&name) {
            Ok(file_name) => ControlsChange::Fixture(file_name),
            Err(message) => return refused(StatusCode::BAD_REQUEST, message),
        },
        other => other,
    };
    let names = match current_fixture_names(state.playground().files().clone()).await {
        Ok(names) => names,
        Err(problem) => return refused(problem.status(), problem.message()),
    };
    let response = match state.playground().change(change.clone(), &names) {
        Ok(reply) => {
            let (key, value) = describe_change(&change);
            state.record_line(playground_line(
                "set",
                &[(&key, value), ("revision", reply.revision.to_string())],
            ));
            Json(reply).into_response()
        }
        Err(message) => {
            state.record_line(playground_line(
                "reject",
                &[("cause", RejectionCause::InvalidValue.key().to_owned())],
            ));
            refused(StatusCode::BAD_REQUEST, message)
        }
    };
    state.echo_new_lines();
    response
}

async fn change_scene(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let change: SceneChange = match serde_json::from_slice(&body) {
        Ok(change) => change,
        Err(error) => return reject_malformed(&state, &error),
    };
    let Some(name) = state.playground().current_fixture() else {
        return refused(
            StatusCode::BAD_REQUEST,
            "no fixture is shown; write one with `fixture write` first".to_owned(),
        );
    };
    let directory = state.playground().files().assets.join(FIXTURE_DIRECTORY);
    let read_name = name.clone();
    let loaded =
        tokio::task::spawn_blocking(move || read_file(&directory, &read_name, &FIXTURE_EXTENSIONS))
            .await
            .unwrap_or_else(|error| Err(FileProblem::Unreadable(error.to_string())));
    let base = match loaded {
        Ok(bytes) => match Fixture::parse(&String::from_utf8_lossy(&bytes)) {
            Ok(base) => base,
            Err(error) => {
                return refused(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "{name}, line {}, column {}: {}",
                        error.line, error.column, error.message
                    ),
                );
            }
        },
        Err(problem) => return refused(problem.status(), problem.message()),
    };
    let media_directory = media_directory(state.playground().files());
    let library = tokio::task::spawn_blocking(move || media_entries(&media_directory))
        .await
        .unwrap_or_else(|error| Err(FileProblem::Unreadable(error.to_string())));
    let library = match library {
        Ok(library) => library,
        Err(problem) => return refused(problem.status(), problem.message()),
    };
    if let Some(message) = media_problem(&change, &library) {
        state.record_line(playground_line(
            "reject",
            &[("cause", RejectionCause::InvalidValue.key().to_owned())],
        ));
        state.echo_new_lines();
        return refused(StatusCode::BAD_REQUEST, message);
    }
    let stage_ids: Vec<String> = match &base.tv.elements {
        ElementSource::Inline(items) => items.iter().map(|item| item.id.clone()).collect(),
        ElementSource::Live(_) => state
            .canvas()
            .elements()
            .map(|element| element.id.to_string())
            .collect(),
    };
    if let Some(message) = camera_problem(&change, &stage_ids) {
        state.record_line(playground_line(
            "reject",
            &[("cause", RejectionCause::InvalidValue.key().to_owned())],
        ));
        state.echo_new_lines();
        return refused(StatusCode::BAD_REQUEST, message);
    }
    let verb = scene_change_name(&change);
    let response = match state.playground().change_scene(change, &base) {
        Ok(reply) => {
            state.record_line(playground_line(
                "scene",
                &[("change", verb), ("revision", reply.revision.to_string())],
            ));
            Json(reply).into_response()
        }
        Err(message) => {
            state.record_line(playground_line(
                "reject",
                &[("cause", RejectionCause::InvalidValue.key().to_owned())],
            ));
            refused(StatusCode::BAD_REQUEST, message)
        }
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

/// The name of a scene change, for the log.
fn scene_change_name(change: &SceneChange) -> String {
    serde_json::to_value(change)
        .ok()
        .and_then(|value| {
            value
                .get("change")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

/// A change as `key` and `value`, for the log.
fn describe_change(change: &ControlsChange) -> (String, String) {
    let value = serde_json::to_value(change).unwrap_or(serde_json::Value::Null);
    let key = value
        .get("change")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let shown = match value.get("value") {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    };
    (key, shown)
}

/// `tavern` and `tavern.json` both name the file `tavern.json`. Anything that could leave the
/// fixture directory, or is not a plain name, is refused.
pub fn fixture_file_name(name: &str) -> Result<String, String> {
    let file_name = if name.ends_with(".json") {
        name.to_owned()
    } else {
        format!("{name}.json")
    };
    let stem = file_name.trim_end_matches(".json");
    if stem.is_empty() || !is_plain_file_name(&file_name) {
        return Err(format!(
            "'{name}' is not a fixture name: use letters, digits, '_', '-' and '.', up to {MAXIMUM_NAME_LENGTH} characters"
        ));
    }
    Ok(file_name)
}

/// Writes `text` to `directory/file_name` through a temporary file, so a reader never sees half
/// of it.
fn write_fixture_file(directory: &Path, file_name: &str, text: &str) -> Result<(), FileProblem> {
    std::fs::create_dir_all(directory).map_err(|error| problem_from(&error))?;
    let temporary = directory.join(format!(".{file_name}.tmp"));
    std::fs::write(&temporary, text).map_err(|error| problem_from(&error))?;
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
        Err(message) => return refused(StatusCode::BAD_REQUEST, message),
    };
    if let Err(error) = Fixture::parse(&request.text) {
        state.record_line(playground_line(
            "reject",
            &[
                ("cause", RejectionCause::InvalidValue.key().to_owned()),
                ("fixture", file_name.clone()),
            ],
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
    let directory = state.playground().files().assets.join(FIXTURE_DIRECTORY);
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
            done(&state)
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
        Err(message) => return refused(StatusCode::BAD_REQUEST, message),
    };
    let path = state
        .playground()
        .files()
        .assets
        .join(FIXTURE_DIRECTORY)
        .join(&file_name);
    let removed = tokio::task::spawn_blocking(move || std::fs::remove_file(path))
        .await
        .unwrap_or_else(|error| Err(std::io::Error::other(error.to_string())));
    let response = match removed {
        Ok(()) => {
            state.record_line(playground_line("fixture_remove", &[("name", file_name)]));
            done(&state)
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

pub(crate) fn done(state: &AppState) -> Response {
    Json(Reply::Done {
        revision: state.playground().revision(),
        notices: Vec::new(),
    })
    .into_response()
}

async fn clients_json(State(state): State<Arc<AppState>>) -> Response {
    Json(state.playground().clients()).into_response()
}

/// A page says what it is showing and playing. Nothing is recorded in the audit log and no other
/// page is woken: a report is a reading, not a change.
async fn receive_report(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let post: ClientReportPost = match serde_json::from_slice(&body) {
        Ok(post) => post,
        Err(error) => return reject_malformed(&state, &error),
    };
    let name_ok = !post.client.is_empty()
        && post.client.len() <= MAXIMUM_CLIENT_NAME
        && post
            .client
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character));
    if !name_ok {
        return refused(
            StatusCode::BAD_REQUEST,
            format!(
                "a client name is letters, digits, '_' and '-', up to {MAXIMUM_CLIENT_NAME} characters"
            ),
        );
    }
    state.playground().record_client(post.client, post.report);
    done(&state)
}

/// The playground's routes: its files, what it is showing, and the fixture commands. The page
/// reads the canvas from the host's own `/canvas.json`.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(route::PLAYGROUND, get(page))
        .route(route::PLAYGROUND_SLASH, get(page))
        .route(route::PLAYGROUND_THEME, get(theme))
        .route(route::PLAYGROUND_REVISION, get(revision_text))
        .route(route::PLAYGROUND_FIXTURE_LIST, get(fixture_list))
        .route(route::PLAYGROUND_FIXTURE, get(fixture))
        .route(route::PLAYGROUND_PACKAGE_FILE, get(package_file))
        .route(route::PLAYGROUND_CONTROLS, get(controls_json))
        .route(route::DEV_PLAYGROUND_CONTROLS, post(change_controls))
        .route(route::DEV_PLAYGROUND_SCENE, post(change_scene))
        .route(route::PLAYGROUND_CLIENTS, get(clients_json))
        .route(route::DEV_CLIENT_REPORT, post(receive_report))
        .route(route::DEV_FIXTURE_WRITE, post(write_fixture))
        .route(route::DEV_FIXTURE_REMOVE, post(remove_fixture))
        .with_state(state)
}

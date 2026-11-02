//! The playground's files: the page, theme, fixtures and compiled package, read from disk and
//! served with a never-cache answer.
//!
//! Names that reach the disk are checked first (a plain file name with an allowed extension), the
//! size is bounded, and blocking file work runs off the async threads. Nothing here reads the
//! canvas or interprets a file.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::limits::MAXIMUM_FILE_BYTES;
use crate::state::AppState;

const INDEX_FILE: &str = "index.html";
const THEME_FILE: &str = "theme.css";
pub(super) const FIXTURE_DIRECTORY: &str = "fixtures";
pub(super) const FIXTURE_EXTENSIONS: [&str; 1] = ["json"];
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

pub(super) async fn page(State(state): State<Arc<AppState>>) -> Response {
    asset(state.playground().files().clone(), INDEX_FILE).await
}

pub(super) async fn theme(State(state): State<Arc<AppState>>) -> Response {
    asset(state.playground().files().clone(), THEME_FILE).await
}

pub(super) async fn fixture(
    State(state): State<Arc<AppState>>,
    UrlPath(name): UrlPath<String>,
) -> Response {
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

pub(super) async fn package_file(
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

pub(super) async fn fixture_list(State(state): State<Arc<AppState>>) -> Response {
    let files = state.playground().files().clone();
    blocking(move || {
        let names = fixture_names(&files)?;
        let text = serde_json::to_string(&names)
            .map_err(|error| FileProblem::Unreadable(error.to_string()))?;
        Ok((text.into_bytes(), content_type("list.json")))
    })
    .await
}

pub(super) async fn revision_text(State(state): State<Arc<AppState>>) -> Response {
    let files = state.playground().files().clone();
    blocking(move || revision(&files).map(|text| (text.into_bytes(), "text/plain; charset=utf-8")))
        .await
}

/// The fixture names, read off the async threads.
pub(super) async fn current_fixture_names(
    files: PlaygroundFiles,
) -> Result<Vec<String>, FileProblem> {
    tokio::task::spawn_blocking(move || fixture_names(&files))
        .await
        .unwrap_or_else(|error| Err(FileProblem::Unreadable(error.to_string())))
}

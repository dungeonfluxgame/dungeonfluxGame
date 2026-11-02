//! The media library: the pictures and sounds an agent can put on the screens.
//!
//! Files live in `media/` under the playground's assets, are added and removed through the dev
//! API (so a command line on the same machine and a script can fill it), and are served to the
//! page by name. It owns the checks that keep the library safe to serve: only plain names of
//! known picture and sound types, no SVG, a size limit, a count limit, and a content type that
//! the browser may not guess at. It does not decide what is shown or played: the scene does.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path as UrlPath, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use df_playground::media::{
    AUDIO_EXTENSIONS, IMAGE_EXTENSIONS, MediaEntry, MediaKind, content_type, media_kind,
};
use df_playground::scene::SceneChange;

use crate::playground::{
    FileProblem, PlaygroundFiles, done, playground_line, problem_from, refused, reject_malformed,
};
use crate::state::AppState;
use crate::wire::{MediaAdd, MediaRemove, route};

/// Largest media file, in bytes.
pub const MAXIMUM_MEDIA_BYTES: usize = 16 * 1024 * 1024;
/// Most files in the library.
pub const MAXIMUM_MEDIA_FILES: usize = 128;
/// Largest request that adds a file: the file as base64, plus the envelope.
pub const MAXIMUM_UPLOAD_BODY_BYTES: usize = MAXIMUM_MEDIA_BYTES.div_ceil(3) * 4 + 1024;
const MEDIA_DIRECTORY: &str = "media";

/// Where the library is on disk.
pub fn media_directory(files: &PlaygroundFiles) -> PathBuf {
    files.assets().join(MEDIA_DIRECTORY)
}

/// The files in the library, sorted by name. A missing directory is an empty library.
pub fn media_entries(directory: &Path) -> Result<Vec<MediaEntry>, FileProblem> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(problem_from(&error)),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| problem_from(&error))?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Some(kind) = media_kind(&name) else {
            continue;
        };
        let metadata = entry.metadata().map_err(|error| problem_from(&error))?;
        if metadata.is_file() {
            found.push(MediaEntry {
                name,
                kind,
                bytes: metadata.len(),
            });
        }
    }
    found.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(found)
}

fn available(entries: &[MediaEntry], kind: MediaKind) -> String {
    let names: Vec<&str> = entries
        .iter()
        .filter(|entry| entry.kind == kind)
        .map(|entry| entry.name.as_str())
        .collect();
    if names.is_empty() {
        "none (add one with: media add FILE)".to_owned()
    } else {
        names.join(", ")
    }
}

fn check_media(name: &str, wanted: MediaKind, entries: &[MediaEntry]) -> Option<String> {
    let noun = match wanted {
        MediaKind::Image => "picture",
        MediaKind::Audio => "sound",
    };
    match entries.iter().find(|entry| entry.name == name) {
        None => Some(format!(
            "there is no {noun} '{name}'; the {noun}s are: {}",
            available(entries, wanted)
        )),
        Some(entry) if entry.kind != wanted => Some(format!(
            "'{name}' is a {}, not a {noun}",
            match entry.kind {
                MediaKind::Image => "picture",
                MediaKind::Audio => "sound",
            }
        )),
        Some(_) => None,
    }
}

/// Why a scene change cannot be made with the media in the library, or `None` when it can: a
/// picture slot needs a picture and music or a sound needs a sound, both present.
pub fn media_problem(change: &SceneChange, entries: &[MediaEntry]) -> Option<String> {
    match change {
        SceneChange::Image { media, .. } => check_media(media, MediaKind::Image, entries),
        SceneChange::Music { media, .. } | SceneChange::Sound { media, .. } => {
            check_media(media, MediaKind::Audio, entries)
        }
        _ => None,
    }
}

/// A value that changes when the file does, so a page can ask whether it has the current one.
fn entity_tag(metadata: &std::fs::Metadata) -> String {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!("\"{:x}-{:x}\"", metadata.len(), modified)
}

enum Served {
    NotModified(String),
    File { bytes: Vec<u8>, tag: String },
}

fn read_media(
    directory: &Path,
    name: &str,
    known_tag: Option<&str>,
) -> Result<Served, FileProblem> {
    if media_kind(name).is_none() {
        return Err(FileProblem::BadName);
    }
    let path = directory.join(name);
    let metadata = std::fs::metadata(&path).map_err(|error| problem_from(&error))?;
    if !metadata.is_file() {
        return Err(FileProblem::NotFound);
    }
    if metadata.len() > MAXIMUM_MEDIA_BYTES as u64 {
        return Err(FileProblem::TooLarge);
    }
    let tag = entity_tag(&metadata);
    if known_tag == Some(tag.as_str()) {
        return Ok(Served::NotModified(tag));
    }
    let bytes = std::fs::read(&path).map_err(|error| problem_from(&error))?;
    Ok(Served::File { bytes, tag })
}

fn media_headers(response: &mut Response, content: &'static str, tag: &str) {
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content));
    // The page checks with the tag each time; an unchanged file costs a short answer, not a file.
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if let Ok(value) = HeaderValue::from_str(tag) {
        headers.insert(header::ETAG, value);
    }
}

async fn media_file(
    State(state): State<Arc<AppState>>,
    UrlPath(name): UrlPath<String>,
    headers: HeaderMap,
) -> Response {
    let directory = media_directory(state.playground().files());
    let known_tag = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let served = {
        let name = name.clone();
        tokio::task::spawn_blocking(move || read_media(&directory, &name, known_tag.as_deref()))
            .await
            .unwrap_or_else(|error| Err(FileProblem::Unreadable(error.to_string())))
    };
    match served {
        Ok(Served::NotModified(tag)) => {
            let mut response = StatusCode::NOT_MODIFIED.into_response();
            media_headers(&mut response, content_type(&name), &tag);
            response
        }
        Ok(Served::File { bytes, tag }) => {
            let mut response = (StatusCode::OK, bytes).into_response();
            media_headers(&mut response, content_type(&name), &tag);
            response
        }
        Err(problem) => refused(problem.status(), problem.message()),
    }
}

async fn media_list(State(state): State<Arc<AppState>>) -> Response {
    let directory = media_directory(state.playground().files());
    let listed = tokio::task::spawn_blocking(move || media_entries(&directory))
        .await
        .unwrap_or_else(|error| Err(FileProblem::Unreadable(error.to_string())));
    match listed {
        Ok(entries) => Json(entries).into_response(),
        Err(problem) => refused(problem.status(), problem.message()),
    }
}

/// Writes `bytes` to `directory/name` through a temporary file, so a reader never sees half of it.
fn write_media_file(directory: &Path, name: &str, bytes: &[u8]) -> Result<(), FileProblem> {
    std::fs::create_dir_all(directory).map_err(|error| problem_from(&error))?;
    let temporary = directory.join(format!(".{name}.tmp"));
    std::fs::write(&temporary, bytes).map_err(|error| problem_from(&error))?;
    std::fs::rename(&temporary, directory.join(name)).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        problem_from(&error)
    })
}

fn kinds_help() -> String {
    format!(
        "pictures: {}; sounds: {}",
        IMAGE_EXTENSIONS.join(", "),
        AUDIO_EXTENSIONS.join(", ")
    )
}

async fn add_media(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let request: MediaAdd = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => return reject_malformed(&state, &error),
    };
    if media_kind(&request.name).is_none() {
        return refused(
            StatusCode::BAD_REQUEST,
            format!(
                "'{}' is not a media name: use a plain name ending in one of these ({})",
                request.name,
                kinds_help()
            ),
        );
    }
    let bytes = match STANDARD.decode(request.data_base64.as_bytes()) {
        Ok(bytes) if !bytes.is_empty() => bytes,
        Ok(_) => return refused(StatusCode::BAD_REQUEST, "the file is empty".to_owned()),
        Err(error) => {
            return refused(
                StatusCode::BAD_REQUEST,
                format!("the file data is not valid base64: {error}"),
            );
        }
    };
    if bytes.len() > MAXIMUM_MEDIA_BYTES {
        return refused(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("a media file is at most {MAXIMUM_MEDIA_BYTES} bytes"),
        );
    }
    let directory = media_directory(state.playground().files());
    let name = request.name.clone();
    let size = bytes.len();
    let written = tokio::task::spawn_blocking(move || {
        let existing = media_entries(&directory)?;
        if existing.len() >= MAXIMUM_MEDIA_FILES && !existing.iter().any(|entry| entry.name == name)
        {
            return Err(FileProblem::TooLarge);
        }
        write_media_file(&directory, &name, &bytes)
    })
    .await
    .unwrap_or_else(|error| Err(FileProblem::Unreadable(error.to_string())));
    let response = match written {
        Ok(()) => {
            state.record_line(playground_line(
                "media_add",
                &[("name", request.name), ("bytes", size.to_string())],
            ));
            done(&state)
        }
        Err(FileProblem::TooLarge) => refused(
            StatusCode::BAD_REQUEST,
            format!("the library holds at most {MAXIMUM_MEDIA_FILES} files; remove one first"),
        ),
        Err(problem) => refused(problem.status(), problem.message()),
    };
    state.echo_new_lines();
    response
}

async fn remove_media(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let request: MediaRemove = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => return reject_malformed(&state, &error),
    };
    if media_kind(&request.name).is_none() {
        return refused(
            StatusCode::BAD_REQUEST,
            format!("'{}' is not a media name", request.name),
        );
    }
    let path = media_directory(state.playground().files()).join(&request.name);
    let removed = tokio::task::spawn_blocking(move || std::fs::remove_file(path))
        .await
        .unwrap_or_else(|error| Err(std::io::Error::other(error.to_string())));
    let response = match removed {
        Ok(()) => {
            state.record_line(playground_line("media_remove", &[("name", request.name)]));
            done(&state)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => refused(
            StatusCode::BAD_REQUEST,
            format!("there is no media '{}'", request.name),
        ),
        Err(error) => {
            let problem = problem_from(&error);
            refused(problem.status(), problem.message())
        }
    };
    state.echo_new_lines();
    response
}

/// The media routes: list and fetch (GET), add and remove (POST).
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(route::PLAYGROUND_MEDIA_LIST, get(media_list))
        .route(route::PLAYGROUND_MEDIA_FILE, get(media_file))
        .route(
            route::DEV_MEDIA_ADD,
            post(add_media).layer(DefaultBodyLimit::max(MAXIMUM_UPLOAD_BODY_BYTES)),
        )
        .route(route::DEV_MEDIA_REMOVE, post(remove_media))
        .with_state(state)
}

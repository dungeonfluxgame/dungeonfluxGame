//! The media library: the pictures, sounds and videos an agent can put on the screens.
//!
//! Files live in `media/` under the playground's assets, are added and removed through the dev
//! API (so a command line on the same machine and a script can fill it), and are served to the
//! page by name, with `Range` answers so a video can start and seek. It owns the checks that
//! keep the library safe to serve: only plain names of known picture, sound and video types, no
//! SVG, size and count limits (`media_ingest`), and a content type that the browser may not
//! guess at. It does not decide what is shown or played: the scene does.
//!
//! A file is never held whole: an answer opens the file once, takes its length and entity tag from
//! that handle and streams the bytes from it in `SERVE_CHUNK_BYTES` chunks (a `HEAD` reads none),
//! and an add is decoded to a staged file as the request arrives (`media_upload`, `media_store`).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use df_playground::media::{MediaEntry, MediaKind, content_type, media_kind};
use df_playground::scene::SceneChange;
use df_table::event::RejectionCause;
use futures_util::{Stream, StreamExt};
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use crate::dev_http::parse_body;
use crate::limits::UPLOAD_PIECE_BYTES;
use crate::media_ingest::{IngestError, MediaLimits};
pub use crate::media_ingest::{
    MAXIMUM_MEDIA_BYTES, MAXIMUM_MEDIA_FILES, MAXIMUM_UPLOAD_BODY_BYTES, MAXIMUM_VIDEO_DURATION,
    video_limits_text,
};
use crate::media_range::{ByteSpan, RangeRequest, parse_range};
use crate::media_store::{AddError, Added, ingest_upload};
use crate::playground::{
    FileError, PlaygroundFiles, done, playground_line, refuse_file, refused, reject_body,
    run_file_task,
};
use crate::state::AppState;
use crate::wire::{MediaAdd, MediaRemove, path};

const MEDIA_DIRECTORY: &str = "media";
/// Bytes of a file read and sent at a time: the most of a file the host holds for one answer.
const SERVE_CHUNK_BYTES: u64 = 256 * 1024;
/// Pieces of an upload waiting for the thread that decodes them; a full queue holds the sender
/// back, so an upload in flight holds a few pieces, not the file.
const UPLOAD_QUEUE_PIECES: usize = 4;

/// Where the library is on disk.
pub fn media_directory(files: &PlaygroundFiles) -> PathBuf {
    files.assets().join(MEDIA_DIRECTORY)
}

/// The files in the library, sorted by name. A missing directory is an empty library.
pub fn media_entries(directory: &Path) -> Result<Vec<MediaEntry>, FileError> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Some(kind) = media_kind(&name) else {
            continue;
        };
        let metadata = entry.metadata()?;
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

fn noun(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "picture",
        MediaKind::Audio => "sound",
        MediaKind::Video => "video",
    }
}

fn check_media(name: &str, wanted: MediaKind, entries: &[MediaEntry]) -> Option<String> {
    let wanted_noun = noun(wanted);
    match entries.iter().find(|entry| entry.name == name) {
        None => Some(format!(
            "there is no {wanted_noun} '{name}'; the {wanted_noun}s are: {}",
            available(entries, wanted)
        )),
        Some(entry) if entry.kind != wanted => Some(format!(
            "'{name}' is a {}, not a {wanted_noun}",
            noun(entry.kind)
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
        SceneChange::SeatSet { portrait, .. } => portrait
            .set_value()
            .and_then(|media| check_media(media, MediaKind::Image, entries)),
        SceneChange::Screen { edit } => edit
            .media_names()
            .into_iter()
            .find_map(|name| check_media(name, MediaKind::Image, entries)),
        SceneChange::Home { edit } => edit
            .media_names()
            .into_iter()
            .find_map(|name| check_media(name, MediaKind::Image, entries)),
        _ => None,
    }
}

/// A value that changes when the file does, so a page can ask whether it has the current one. A
/// file whose modification time cannot be read has no such value, and the answer says so: a
/// made-up value would let a replaced file of the same size answer 304 with stale bytes.
fn entity_tag(metadata: &std::fs::Metadata) -> Result<String, FileError> {
    let modified = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| FileError::ModifiedBeforeEpoch)?
        .as_nanos();
    Ok(format!("\"{:x}-{:x}\"", metadata.len(), modified))
}

/// The request headers that decide how a media file is answered.
#[derive(Debug, Default)]
struct Conditions {
    if_none_match: Option<String>,
    range: Option<String>,
    if_range: Option<String>,
}

impl Conditions {
    fn from_headers(headers: &HeaderMap) -> Self {
        let text = |name| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        Self {
            if_none_match: text(header::IF_NONE_MATCH),
            range: text(header::RANGE),
            if_range: text(header::IF_RANGE),
        }
    }
}

/// The body of an answer: the bytes of a span of the open file, or only their count when the
/// request was a `HEAD`.
enum Payload {
    Withheld { length: u64 },
    Stream { file: File, length: u64 },
}

impl Payload {
    /// The `length` bytes of `file` from `start`, or just `length` for a `HEAD`. Nothing is read.
    fn of(mut file: File, start: u64, length: u64, head: bool) -> Result<Self, FileError> {
        if head {
            return Ok(Self::Withheld { length });
        }
        file.seek(SeekFrom::Start(start))?;
        Ok(Self::Stream { file, length })
    }

    fn into_response(self, status: StatusCode) -> Response {
        let (mut response, length) = match self {
            Self::Withheld { length } => (status.into_response(), length),
            Self::Stream { file, length } => {
                let mut response = Response::new(Body::from_stream(read_in_chunks(file, length)));
                *response.status_mut() = status;
                (response, length)
            }
        };
        response
            .headers_mut()
            .insert(header::CONTENT_LENGTH, HeaderValue::from(length));
        response
    }
}

/// Reads up to one chunk from the file's current position. A file that ends before the length it
/// was measured at (it was cut short in place) is an error, so the answer is cut, never padded.
fn read_chunk(mut file: File, remaining: u64) -> std::io::Result<(File, Vec<u8>)> {
    let mut chunk = Vec::new();
    (&mut file)
        .take(remaining.min(SERVE_CHUNK_BYTES))
        .read_to_end(&mut chunk)?;
    if chunk.is_empty() {
        return Err(std::io::ErrorKind::UnexpectedEof.into());
    }
    Ok((file, chunk))
}

/// The next `length` bytes of the file as a stream of chunks, each read on a blocking thread. The
/// stream owns the file; dropping the answer (the page went away) stops the reading.
pub(crate) fn read_in_chunks(
    file: File,
    length: u64,
) -> impl Stream<Item = std::io::Result<Bytes>> + Send + 'static {
    futures_util::stream::unfold(Some((file, length)), |state| async move {
        let (file, remaining) = state?;
        if remaining == 0 {
            return None;
        }
        match tokio::task::spawn_blocking(move || read_chunk(file, remaining)).await {
            Ok(Ok((file, chunk))) => {
                let read = u64::try_from(chunk.len()).unwrap_or(u64::MAX);
                let left = remaining.saturating_sub(read);
                Some((Ok(Bytes::from(chunk)), Some((file, left))))
            }
            Ok(Err(error)) => Some((Err(error), None)),
            Err(error) => Some((Err(std::io::Error::other(error.to_string())), None)),
        }
    })
}

enum Served {
    NotModified(String),
    File {
        payload: Payload,
        tag: String,
    },
    Part {
        payload: Payload,
        tag: String,
        span: ByteSpan,
        file_bytes: u64,
    },
    Unsatisfiable {
        tag: String,
        file_bytes: u64,
    },
}

/// The range to answer: the `Range` header, unless `If-Range` names a version that is not the
/// current one, in which case the page must have the whole file again.
fn requested_range(conditions: &Conditions, tag: &str, file_bytes: u64) -> RangeRequest {
    let Some(range) = conditions.range.as_deref() else {
        return RangeRequest::Whole;
    };
    if conditions
        .if_range
        .as_deref()
        .is_some_and(|version| version != tag)
    {
        return RangeRequest::Whole;
    }
    parse_range(range, file_bytes)
}

/// Answers a request from one open file: the length and the entity tag come from this handle's
/// metadata and the bytes from this handle, so a file replaced meanwhile cannot mix two versions.
/// A `HEAD` reads no bytes.
fn serve_file(
    file: File,
    conditions: &Conditions,
    head: bool,
    limits: &MediaLimits,
) -> Result<Served, FileError> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(FileError::NotFound);
    }
    let file_bytes = metadata.len();
    if file_bytes > limits.maximum_file_bytes {
        return Err(FileError::TooLarge {
            maximum_bytes: limits.maximum_file_bytes,
        });
    }
    let tag = entity_tag(&metadata)?;
    if conditions.if_none_match.as_deref() == Some(tag.as_str()) {
        return Ok(Served::NotModified(tag));
    }
    match requested_range(conditions, &tag, file_bytes) {
        RangeRequest::Whole => Ok(Served::File {
            payload: Payload::of(file, 0, file_bytes, head)?,
            tag,
        }),
        RangeRequest::Partial(span) => Ok(Served::Part {
            payload: Payload::of(file, span.start, span.length(), head)?,
            tag,
            span,
            file_bytes,
        }),
        RangeRequest::Unsatisfiable => Ok(Served::Unsatisfiable { tag, file_bytes }),
    }
}

fn open_media(
    directory: &Path,
    name: &str,
    conditions: &Conditions,
    head: bool,
) -> Result<Served, FileError> {
    if media_kind(name).is_none() {
        return Err(FileError::InvalidName);
    }
    let file = File::open(directory.join(name))?;
    serve_file(file, conditions, head, &MediaLimits::DEFAULT)
}

fn media_headers(response: &mut Response, content: &'static str, tag: &str) {
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content));
    // The page checks with the tag each time; an unchanged file costs a short answer, not a file.
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if let Ok(value) = HeaderValue::from_str(tag) {
        headers.insert(header::ETAG, value);
    }
}

fn set_content_range(response: &mut Response, text: &str) {
    if let Ok(value) = HeaderValue::from_str(text) {
        response.headers_mut().insert(header::CONTENT_RANGE, value);
    }
}

fn served_response(served: Served, content: &'static str) -> Response {
    let (mut response, tag) = match served {
        Served::NotModified(tag) => (StatusCode::NOT_MODIFIED.into_response(), tag),
        Served::File { payload, tag } => (payload.into_response(StatusCode::OK), tag),
        Served::Part {
            payload,
            tag,
            span,
            file_bytes,
        } => {
            let mut response = payload.into_response(StatusCode::PARTIAL_CONTENT);
            set_content_range(
                &mut response,
                &format!("bytes {}-{}/{file_bytes}", span.start, span.end_inclusive),
            );
            (response, tag)
        }
        Served::Unsatisfiable { tag, file_bytes } => {
            let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            set_content_range(&mut response, &format!("bytes */{file_bytes}"));
            (response, tag)
        }
    };
    media_headers(&mut response, content, &tag);
    response
}

async fn media_file(
    State(state): State<Arc<AppState>>,
    UrlPath(name): UrlPath<String>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    let directory = media_directory(state.playground().files());
    let conditions = Conditions::from_headers(&headers);
    let head = method == Method::HEAD;
    let served = {
        let name = name.clone();
        run_file_task(move || open_media(&directory, &name, &conditions, head)).await
    };
    match served {
        Ok(served) => served_response(served, content_type(&name)),
        Err(error) => {
            if error.status().is_server_error() {
                state.record_line(playground_line(
                    "media_unreadable",
                    &[("name", name.clone())],
                ));
            }
            refuse_file(error)
        }
    }
}

async fn media_list(State(state): State<Arc<AppState>>) -> Response {
    let directory = media_directory(state.playground().files());
    match run_file_task(move || media_entries(&directory)).await {
        Ok(entries) => Json(entries).into_response(),
        Err(error) => refuse_file(error),
    }
}

/// The length the request declares for its body, if it declares one.
fn declared_length(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(header::CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn record_malformed(state: &AppState) {
    state.record_line(playground_line(
        "reject",
        &[("cause", RejectionCause::MalformedRequest.key().to_owned())],
    ));
}

fn refuse_add(state: &AppState, error: &AddError) -> Response {
    if matches!(error, AddError::Ingest(IngestError::InvalidRequest(_))) {
        record_malformed(state);
    }
    refused(error.status(), error.to_string())
}

/// Reads the request body as it arrives and hands it to the thread that stages the file. After
/// the thread gives up (a refusal) the rest of the body is read and dropped, up to the limit, so
/// the page gets its answer rather than a closed connection.
async fn add_media(State(state): State<Arc<AppState>>, headers: HeaderMap, body: Body) -> Response {
    let limits = MediaLimits::DEFAULT;
    if declared_length(&headers).is_some_and(|length| length > limits.maximum_upload_body_bytes()) {
        let error = AddError::Ingest(IngestError::TooLarge {
            maximum_bytes: limits.maximum_file_bytes,
        });
        return refuse_add(&state, &error);
    }
    let directory = media_directory(state.playground().files());
    let (sender, worker) = start_staging(directory, limits);
    let mut pieces = body.into_data_stream();
    let mut received = 0_u64;
    let mut worker_listening = true;
    while let Some(Ok(piece)) = pieces.next().await {
        received = received.saturating_add(u64::try_from(piece.len()).unwrap_or(u64::MAX));
        if received > limits.maximum_upload_body_bytes() {
            break;
        }
        if worker_listening && sender.send(piece).await.is_err() {
            worker_listening = false;
        }
    }
    drop(sender);
    match finish_staging(worker).await {
        Ok(added) => {
            state.record_line(playground_line(
                "media_add",
                &[
                    ("name", added.name),
                    ("bytes", added.file_bytes.to_string()),
                ],
            ));
            done(&state)
        }
        Err(error) => refuse_add(&state, &error),
    }
}

/// Starts the thread that stages one add into `directory`; the add request is sent to it in
/// pieces through the returned sender, and dropping the sender ends the request.
fn start_staging(
    directory: PathBuf,
    limits: MediaLimits,
) -> (Sender<Bytes>, JoinHandle<Result<Added, AddError>>) {
    let (sender, receiver) = tokio::sync::mpsc::channel::<Bytes>(UPLOAD_QUEUE_PIECES);
    let worker = tokio::task::spawn_blocking(move || ingest_upload(receiver, &directory, &limits));
    (sender, worker)
}

/// The staging thread's answer; a thread that stopped without one is `FileError::Stopped`.
async fn finish_staging(worker: JoinHandle<Result<Added, AddError>>) -> Result<Added, AddError> {
    worker
        .await
        .unwrap_or(Err(AddError::File(FileError::Stopped)))
}

/// Stores `bytes` as `file_name` in the library at `directory` through the same staged path and
/// add envelope as `media add`, encoded and sent in pieces, so the picture is never held twice.
pub(crate) async fn store_bytes(
    directory: PathBuf,
    file_name: &str,
    bytes: &[u8],
) -> Result<Added, AddError> {
    // A name always writes as a JSON string; the mapping only keeps the path panic-free.
    let (prefix, suffix) = MediaAdd::streamed_ends(file_name)
        .map_err(|_| AddError::File(FileError::Io(std::io::ErrorKind::InvalidData)))?;
    let (sender, worker) = start_staging(directory, MediaLimits::DEFAULT);
    // A send fails once the staging thread has given up (a refusal); its answer says why, so the
    // sending simply stops.
    let mut open = sender.send(Bytes::from(prefix)).await.is_ok();
    for piece in bytes.chunks(UPLOAD_PIECE_BYTES) {
        if !open {
            break;
        }
        let encoded = base64::engine::general_purpose::STANDARD.encode(piece);
        open = sender.send(Bytes::from(encoded)).await.is_ok();
    }
    if open {
        // The result of the last send is read through the worker's answer below.
        let _ = sender.send(Bytes::from_static(suffix.as_bytes())).await;
    }
    drop(sender);
    finish_staging(worker).await
}

async fn remove_media(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let request: MediaRemove = match parse_body(&body) {
        Ok(request) => request,
        Err(error) => return reject_body(&state, &error),
    };
    if media_kind(&request.name).is_none() {
        return refused(
            StatusCode::BAD_REQUEST,
            format!("'{}' is not a media name", request.name),
        );
    }
    let media_path = media_directory(state.playground().files()).join(&request.name);
    let removed =
        run_file_task(move || std::fs::remove_file(media_path).map_err(FileError::from)).await;
    match removed {
        Ok(()) => {
            state.record_line(playground_line("media_remove", &[("name", request.name)]));
            done(&state)
        }
        Err(FileError::NotFound) => refused(
            StatusCode::BAD_REQUEST,
            format!("there is no media '{}'", request.name),
        ),
        Err(error) => refuse_file(error),
    }
}

/// The media routes: list and fetch (GET and HEAD), add and remove (POST). The add route reads
/// its own body, up to `MAXIMUM_UPLOAD_BODY_BYTES`; the router-wide `MAXIMUM_REQUEST_BYTES` limit
/// does not cover it.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(path::PLAYGROUND_MEDIA_LIST, get(media_list))
        .route(path::PLAYGROUND_MEDIA_FILE, get(media_file))
        .route(path::DEV_MEDIA_ADD, post(add_media))
        .route(path::DEV_MEDIA_REMOVE, post(remove_media))
        .with_state(state)
}

#[cfg(test)]
mod media_library_tests;

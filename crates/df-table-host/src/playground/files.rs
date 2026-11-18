//! The playground's files: the page, theme, fixtures and compiled package, read from disk. The
//! page, the fixtures and the revision are served with a never-cache answer; the theme and the
//! package carry an entity tag (`media_library::entity_tag`) and are checked each time, so a page
//! that reloads (a phone back from its lock screen) gets a bodiless 304 for the files it already
//! has instead of the whole package again.
//!
//! Names that reach the disk are checked first (a plain file name with an allowed extension), the
//! size is bounded, and blocking file work runs off the async threads. Nothing here reads the
//! canvas or interprets a file. A file that cannot be read (a 5xx), and a package file that is
//! missing, is recorded as `PLAYGROUND file_unreadable kind= name= status=`; a successful read
//! is not recorded.

use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use axum::body::Body;
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use super::audit::playground_line;
use crate::blocking::run_blocking;
use crate::limits::{FILES_WATCH_INTERVAL, MAXIMUM_ASSET_FILE_BYTES, MAXIMUM_PACKAGE_FILE_BYTES};
use crate::media_library::{entity_tag, read_in_chunks};
use crate::state::AppState;

const INDEX_FILE: &str = "index.html";
const THEME_FILE: &str = "theme.css";
pub(super) const FIXTURE_DIRECTORY: &str = "fixtures";
pub(super) const FIXTURE_EXTENSIONS: [&str; 1] = ["json"];
const DICE_DIRECTORY: &str = "dice";
const DICE_EXTENSIONS: [&str; 1] = ["png"];
const DICE_CACHE_CONTROL: &str = "max-age=600";
const PACKAGE_EXTENSIONS: [&str; 2] = ["js", "wasm"];
const ASSET_EXTENSIONS: [&str; 2] = ["html", "css"];
const LAYOUT_EXTENSIONS: [&str; 1] = ["json"];

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

/// Why a playground or library file was not served, read or written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileError {
    /// The name is not a plain file name of an allowed type.
    InvalidName,
    NotFound,
    /// The file is larger than `maximum_bytes`, the most this kind of file may be.
    TooLarge {
        maximum_bytes: u64,
    },
    /// The file system refused the read or write for another reason.
    Io(ErrorKind),
    /// The file's modification time is before 1970, so it has no entity tag.
    ModifiedBeforeEpoch,
    /// The blocking task stopped without an answer (it panicked or the runtime is stopping).
    Stopped,
}

impl FileError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::InvalidName | Self::NotFound => StatusCode::NOT_FOUND,
            Self::TooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::Io(_) | Self::ModifiedBeforeEpoch | Self::Stopped => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    /// A closed word for the audit line, so a log says why without carrying free text.
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidName => "invalid_name",
            Self::NotFound => "not_found",
            Self::TooLarge { .. } => "too_large",
            Self::Io(_) => "io",
            Self::ModifiedBeforeEpoch => "modified_before_epoch",
            Self::Stopped => "stopped",
        }
    }

    /// Transitional: the sentence for the error, kept while `serve.rs` still calls it; new code
    /// formats the error itself.
    pub fn message(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for FileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName => formatter.write_str("no such playground file"),
            Self::NotFound => formatter.write_str(concat!(
                "that playground file is missing ",
                "(is the directory right? see df-table serve --help)"
            )),
            Self::TooLarge { maximum_bytes } => {
                write!(formatter, "the file is larger than {maximum_bytes} bytes")
            }
            Self::Io(kind) => write!(formatter, "the file could not be read or written: {kind}"),
            Self::ModifiedBeforeEpoch => {
                formatter.write_str("the file's modification time is before 1970")
            }
            Self::Stopped => formatter.write_str("the file task stopped without an answer"),
        }
    }
}

impl std::error::Error for FileError {}

/// A missing file is `NotFound`; any other failure keeps its kind.
impl From<std::io::Error> for FileError {
    fn from(error: std::io::Error) -> Self {
        match error.kind() {
            ErrorKind::NotFound => Self::NotFound,
            kind => Self::Io(kind),
        }
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
        Some("png") => "image/png",
        _ => "application/octet-stream",
    }
}

/// Reads `name` from `directory` if it is a plain file name with one of `extensions`, and at most
/// `MAXIMUM_ASSET_FILE_BYTES` long (a page, a theme or a fixture; the package is streamed, not
/// read whole). A file that grows while it is read is refused the same way.
pub fn read_file(directory: &Path, name: &str, extensions: &[&str]) -> Result<Vec<u8>, FileError> {
    read_whole(open_asset_file(directory, name, extensions)?)
}

/// Opens `name` in `directory` if it is a plain file name with one of `extensions`, and at most
/// `MAXIMUM_ASSET_FILE_BYTES` long.
fn open_asset_file(
    directory: &Path,
    name: &str,
    extensions: &[&str],
) -> Result<std::fs::File, FileError> {
    if !is_plain_file_name(name) || !has_extension(name, extensions) {
        return Err(FileError::InvalidName);
    }
    open_plain_file(&directory.join(name), MAXIMUM_ASSET_FILE_BYTES)
}

/// The bytes of an asset file opened by `open_asset_file`.
fn read_whole(file: std::fs::File) -> Result<Vec<u8>, FileError> {
    let mut bytes = Vec::new();
    // One byte past the limit shows that the file outgrew it after it was measured.
    file.take(MAXIMUM_ASSET_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(FileError::from)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAXIMUM_ASSET_FILE_BYTES {
        return Err(FileError::TooLarge {
            maximum_bytes: MAXIMUM_ASSET_FILE_BYTES,
        });
    }
    Ok(bytes)
}

/// Opens `path` if it is a file of at most `maximum_bytes`.
fn open_plain_file(path: &Path, maximum_bytes: u64) -> Result<std::fs::File, FileError> {
    let metadata = std::fs::metadata(path).map_err(FileError::from)?;
    if !metadata.is_file() {
        return Err(FileError::NotFound);
    }
    if metadata.len() > maximum_bytes {
        return Err(FileError::TooLarge { maximum_bytes });
    }
    std::fs::File::open(path).map_err(FileError::from)
}

/// The plain files in `directory` with one of `extensions`, sorted by name. A missing directory
/// has none.
fn plain_files(
    directory: &Path,
    extensions: &[&str],
) -> Result<Vec<(String, std::fs::Metadata)>, FileError> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(FileError::from)?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !is_plain_file_name(&name) || !has_extension(&name, extensions) {
            continue;
        }
        let metadata = entry.metadata().map_err(FileError::from)?;
        if metadata.is_file() {
            files.push((name, metadata));
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

/// The names of the fixture files, sorted.
pub fn fixture_names(files: &PlaygroundFiles) -> Result<Vec<String>, FileError> {
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
) -> Result<(), FileError> {
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
pub fn revision(files: &PlaygroundFiles) -> Result<String, FileError> {
    let mut assets = DefaultHasher::new();
    stamp(&files.assets, &ASSET_EXTENSIONS, &mut assets)?;
    stamp(
        &files.assets.join(FIXTURE_DIRECTORY),
        &FIXTURE_EXTENSIONS,
        &mut assets,
    )?;
    // A picture's scanned layout (`media locate`) is attached to the fixtures that show it, so a
    // new scan or label is a new revision and the page reads its fixture again.
    stamp(
        &files.assets.join(crate::locate::LAYOUT_DIRECTORY),
        &LAYOUT_EXTENSIONS,
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

/// Reads the playground's files every `FILES_WATCH_INTERVAL` and announces a new revision as a
/// change (`AppState::refresh_files_revision`), so every page follows the files from its frames
/// instead of asking for them itself; on the same beat it reads the party's heroes again
/// (`AppState::refresh_party_heroes`), whose headshot is made in the background. A new revision
/// writes `PLAYGROUND files_changed revision=`; a read that fails writes
/// `PLAYGROUND files_unreadable reason=` once, until a read succeeds.
/// Abort the handle to stop it.
pub(crate) fn spawn_files_watch(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(FILES_WATCH_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut failing = false;
        loop {
            interval.tick().await;
            state.refresh_party_heroes().await;
            match state.refresh_files_revision().await {
                Ok(changed) => {
                    failing = false;
                    if let Some(revision) = changed {
                        state.record_line_and_echo(playground_line(
                            "files_changed",
                            &[("revision", revision)],
                        ));
                    }
                }
                Err(error) if !failing => {
                    failing = true;
                    state.record_line_and_echo(playground_line(
                        "files_unreadable",
                        &[("reason", error.key().to_owned())],
                    ));
                }
                Err(_) => {}
            }
        }
    })
}

/// Which playground file a read was for, in its audit line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServedFileKind {
    /// The page itself.
    Asset,
    Theme,
    Revision,
    FixtureList,
    Fixture,
    Package,
    /// One picture of the dice, which the roll card draws its dice from.
    Dice,
}

impl ServedFileKind {
    pub const ALL: [Self; 7] = [
        Self::Asset,
        Self::Theme,
        Self::Revision,
        Self::FixtureList,
        Self::Fixture,
        Self::Package,
        Self::Dice,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Asset => "asset",
            Self::Theme => "theme",
            Self::Revision => "revision",
            Self::FixtureList => "fixture_list",
            Self::Fixture => "fixture",
            Self::Package => "package",
            Self::Dice => "dice",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }
}

/// Records a read the page needed and did not get: any 5xx, and a missing package file (the page
/// cannot start without it). `name` is written only when it is a plain file name.
fn record_unreadable(state: &AppState, kind: ServedFileKind, name: Option<&str>, error: FileError) {
    let status = error.status();
    let missing_package = kind == ServedFileKind::Package && status == StatusCode::NOT_FOUND;
    if !status.is_server_error() && !missing_package {
        return;
    }
    let name = name.filter(|name| is_plain_file_name(name)).unwrap_or("-");
    state.record_line(playground_line(
        "file_unreadable",
        &[
            ("kind", kind.key().to_owned()),
            ("name", name.to_owned()),
            ("status", status.as_u16().to_string()),
        ],
    ));
}

/// Answers a read of a playground file: its bytes, or its refusal, recorded when it matters.
fn respond(
    state: &AppState,
    kind: ServedFileKind,
    name: Option<&str>,
    result: Result<(Vec<u8>, &'static str), FileError>,
) -> Response {
    let (status, content_type, body) = match result {
        Ok((body, content_type)) => (StatusCode::OK, content_type, body),
        Err(error) => {
            record_unreadable(state, kind, name, error);
            (
                error.status(),
                "text/plain; charset=utf-8",
                error.to_string().into_bytes(),
            )
        }
    };
    let mut response = (status, body).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// Runs blocking file work off the async threads.
async fn blocking(
    state: &AppState,
    kind: ServedFileKind,
    name: Option<&str>,
    work: impl FnOnce() -> Result<(Vec<u8>, &'static str), FileError> + Send + 'static,
) -> Response {
    respond(
        state,
        kind,
        name,
        run_blocking(work, FileError::Stopped).await,
    )
}

async fn asset(state: &AppState, kind: ServedFileKind, name: &'static str) -> Response {
    let files = state.playground().files().clone();
    blocking(state, kind, Some(name), move || {
        read_file(&files.assets, name, &ASSET_EXTENSIONS).map(|bytes| (bytes, content_type(name)))
    })
    .await
}

pub(super) async fn page(State(state): State<Arc<AppState>>) -> Response {
    asset(&state, ServedFileKind::Asset, INDEX_FILE).await
}

/// What a read of a file the page keeps found: the page's copy is current, or the current file
/// (`body`). Either way `tag` is the current file's entity tag.
enum TaggedRead<T> {
    Unchanged { tag: String },
    Changed { body: T, tag: String },
}

/// The entity tag of the copy the page already has (`If-None-Match`), when it names one.
fn held_tag(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

/// Lets the page keep the answer and check it each time by its entity tag `tag`.
fn checked_by_tag(response: &mut Response, tag: &str) {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    if let Ok(value) = HeaderValue::from_str(tag) {
        headers.insert(header::ETAG, value);
    }
}

/// The bodiless answer to a page that already has the file tagged `tag`.
fn not_modified(tag: &str) -> Response {
    let mut response = StatusCode::NOT_MODIFIED.into_response();
    checked_by_tag(&mut response, tag);
    response
}

/// The theme, unless `held` is its current entity tag. The tag and the bytes come from one open
/// handle, so a file replaced meanwhile cannot pair one version's tag with another's bytes.
fn read_theme(assets: &Path, held: Option<&str>) -> Result<TaggedRead<Vec<u8>>, FileError> {
    let file = open_asset_file(assets, THEME_FILE, &ASSET_EXTENSIONS)?;
    let tag = entity_tag(&file.metadata()?)?;
    if held == Some(tag.as_str()) {
        return Ok(TaggedRead::Unchanged { tag });
    }
    Ok(TaggedRead::Changed {
        body: read_whole(file)?,
        tag,
    })
}

pub(super) async fn theme(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let files = state.playground().files().clone();
    let held = held_tag(&headers);
    let read = run_blocking(
        move || read_theme(&files.assets, held.as_deref()),
        FileError::Stopped,
    )
    .await;
    match read {
        Ok(TaggedRead::Unchanged { tag }) => not_modified(&tag),
        Ok(TaggedRead::Changed { body, tag }) => {
            let mut response = respond(
                &state,
                ServedFileKind::Theme,
                Some(THEME_FILE),
                Ok((body, content_type(THEME_FILE))),
            );
            checked_by_tag(&mut response, &tag);
            response
        }
        Err(error) => respond(&state, ServedFileKind::Theme, Some(THEME_FILE), Err(error)),
    }
}

pub(super) async fn fixture(
    State(state): State<Arc<AppState>>,
    UrlPath(name): UrlPath<String>,
) -> Response {
    let files = state.playground().files().clone();
    let read_name = name.clone();
    blocking(&state, ServedFileKind::Fixture, Some(&name), move || {
        let name = read_name;
        read_file(
            &files.assets.join(FIXTURE_DIRECTORY),
            &name,
            &FIXTURE_EXTENSIONS,
        )
        .map(|bytes| {
            let layouts = files.assets.join(crate::locate::LAYOUT_DIRECTORY);
            let media = crate::media_library::media_directory(&files);
            (
                crate::locate::attach_layout(bytes, &media, &layouts),
                content_type(&name),
            )
        })
    })
    .await
}

pub(super) async fn dice(
    State(state): State<Arc<AppState>>,
    UrlPath(name): UrlPath<String>,
) -> Response {
    let files = state.playground().files().clone();
    let read_name = name.clone();
    let mut response = blocking(&state, ServedFileKind::Dice, Some(&name), move || {
        let name = read_name;
        read_file(&files.assets.join(DICE_DIRECTORY), &name, &DICE_EXTENSIONS)
            .map(|bytes| (bytes, content_type(&name)))
    })
    .await;
    if response.status() == StatusCode::OK {
        // The dice art is large and changes rarely: a card draws it from the browser's memory
        // after the page's first read, so the first roll does not wait for it.
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static(DICE_CACHE_CONTROL),
        );
    }
    response
}

pub(super) async fn package_file(
    State(state): State<Arc<AppState>>,
    UrlPath(name): UrlPath<String>,
    headers: HeaderMap,
) -> Response {
    let files = state.playground().files().clone();
    let held = held_tag(&headers);
    let opened = {
        let name = name.clone();
        run_blocking(
            move || open_package_file(&files.package, &name, held.as_deref()),
            FileError::Stopped,
        )
        .await
    };
    match opened {
        Ok(TaggedRead::Unchanged { tag }) => not_modified(&tag),
        Ok(TaggedRead::Changed {
            body: (file, length),
            tag,
        }) => {
            // The package is the biggest file there is: it is streamed in chunks, never held whole.
            let mut response = Response::new(Body::from_stream(read_in_chunks(file, length)));
            let headers = response.headers_mut();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static(content_type(&name)),
            );
            headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
            checked_by_tag(&mut response, &tag);
            response
        }
        Err(error) => respond(&state, ServedFileKind::Package, Some(&name), Err(error)),
    }
}

/// Opens the package file `name` and measures it, if it is a plain name with a package extension
/// and at most `MAXIMUM_PACKAGE_FILE_BYTES`, unless `held` is its current entity tag.
fn open_package_file(
    directory: &Path,
    name: &str,
    held: Option<&str>,
) -> Result<TaggedRead<(std::fs::File, u64)>, FileError> {
    if !is_plain_file_name(name) || !has_extension(name, &PACKAGE_EXTENSIONS) {
        return Err(FileError::InvalidName);
    }
    let file = open_plain_file(&directory.join(name), MAXIMUM_PACKAGE_FILE_BYTES)?;
    let metadata = file.metadata().map_err(FileError::from)?;
    let tag = entity_tag(&metadata)?;
    if held == Some(tag.as_str()) {
        return Ok(TaggedRead::Unchanged { tag });
    }
    Ok(TaggedRead::Changed {
        body: (file, metadata.len()),
        tag,
    })
}

pub(super) async fn fixture_list(State(state): State<Arc<AppState>>) -> Response {
    let files = state.playground().files().clone();
    blocking(&state, ServedFileKind::FixtureList, None, move || {
        let names = fixture_names(&files)?;
        // A list of strings always writes as JSON; the mapping only keeps the path panic-free.
        let text =
            serde_json::to_string(&names).map_err(|_| FileError::Io(ErrorKind::InvalidData))?;
        Ok((text.into_bytes(), content_type("list.json")))
    })
    .await
}

pub(super) async fn revision_text(State(state): State<Arc<AppState>>) -> Response {
    let files = state.playground().files().clone();
    blocking(&state, ServedFileKind::Revision, None, move || {
        revision(&files).map(|text| (text.into_bytes(), "text/plain; charset=utf-8"))
    })
    .await
}

/// The fixture names, read off the async threads.
pub(super) async fn current_fixture_names(
    files: PlaygroundFiles,
) -> Result<Vec<String>, FileError> {
    run_blocking(move || fixture_names(&files), FileError::Stopped).await
}

#[cfg(test)]
mod files_tests;

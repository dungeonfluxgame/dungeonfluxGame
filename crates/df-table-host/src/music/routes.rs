//! The `/dev/playground/scene/music` route and its flow: reuse the scene's named track or compose
//! it, then play it looping.
//!
//! The flow runs on a task of its own (`run_owned`) so a client that leaves cannot lose a track
//! already paid for. It records `MUSIC reused`, `MUSIC compose`, `MUSIC stored` or
//! `MUSIC compose_failed`, and the route records `MUSIC refused error=<key>` for any refusal. A
//! line holds names, sizes, milliseconds and the prompt's length, never the prompt. The music
//! change itself writes its `PLAYGROUND` line as `scene music` does.
//!
//! Requests for one name take turns ([`TrackLocks`]), so a track is paid for once: a second
//! request for a name being composed waits, then finds the stored track and reuses it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use df_playground::media::{MediaEntry, MediaKind};
use df_playground::scene::SceneChange;
use df_types::saturating_millis;
use tokio::sync::Mutex;

use super::backend::{Composer, ElevenLabsComposer};
use super::limits::{COMPOSE_DEADLINE, COMPOSED_EXTENSION};
use super::request::{MusicError, MusicPlayed, MusicProviderCause, MusicRequest, MusicSource};
use crate::blocking::{run_blocking, run_owned};
use crate::dev_http::{parse_body, reply};
use crate::io_cause::IoCause;
use crate::media_library::{media_directory, media_entries, store_bytes};
use crate::media_store::AddError;
use crate::playground::{FileError, PlaygroundChangeError, apply_scene_change};
use crate::state::AppState;

/// The address of the music engine's dev API.
pub mod path {
    /// Plays a scene's background music, composing it first when the library has none of that
    /// name (POST a `MusicRequest`).
    pub const SCENE_MUSIC: &str = "/dev/playground/scene/music";
}

/// The track a request plays: a sound the library holds under the name, or a file to compose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Track {
    Library { file: String, bytes: u64 },
    Compose { file: String },
}

/// The library's sound named `name` (its file name without the extension), the first by file name
/// when several are; otherwise `NAME.mp3`, to be composed.
pub(super) fn choose_track(name: &str, entries: &[MediaEntry]) -> Track {
    let held = entries.iter().find(|entry| {
        entry.kind == MediaKind::Audio
            && entry
                .name
                .rsplit_once('.')
                .is_some_and(|(stem, _)| stem == name)
    });
    match held {
        Some(entry) => Track::Library {
            file: entry.name.clone(),
            bytes: entry.bytes,
        },
        None => Track::Compose {
            file: format!("{name}.{COMPOSED_EXTENSION}"),
        },
    }
}

fn file_cause(error: FileError) -> IoCause {
    match error {
        FileError::Io(kind) => IoCause::from_kind(kind),
        _ => IoCause::Other,
    }
}

fn add_cause(error: AddError) -> IoCause {
    match error {
        AddError::File(error) => file_cause(error),
        AddError::Ingest(_) => IoCause::InvalidData,
    }
}

/// The names being played, each with the lock its requests take in turn. A name's lock leaves the
/// map when its last request is done, so the map holds only names in flight.
#[derive(Default)]
pub(super) struct TrackLocks {
    names: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl TrackLocks {
    /// The lock of `name`, made when no request for it is in flight.
    async fn of(&self, name: &str) -> Arc<Mutex<()>> {
        Arc::clone(self.names.lock().await.entry(name.to_owned()).or_default())
    }

    /// Forgets `name`'s lock once nobody but the map and this finished request holds it.
    async fn release(&self, name: &str, lock: Arc<Mutex<()>>) {
        let mut names = self.names.lock().await;
        if Arc::strong_count(&lock) == 2 {
            names.remove(name);
        }
    }
}

/// What a composition produced, for the reply.
struct Composed {
    bytes: u64,
    length_ms: u64,
    compose_ms: u64,
}

/// Composes `request` as `file` within `deadline` and stores it in the library.
async fn compose_and_store(
    app: &AppState,
    composer: &dyn Composer,
    request: &MusicRequest,
    file: &str,
    deadline: Duration,
) -> Result<Composed, MusicError> {
    let length_ms = request.length_seconds * 1_000;
    let (provider, model) = (composer.provider(), composer.model());
    app.record_line_and_echo(format!(
        concat!(
            "MUSIC compose name={} file={} length_ms={} prompt_chars={} deadline_ms={} ",
            "provider={} model={}"
        ),
        request.name,
        file,
        length_ms,
        request.prompt.chars().count(),
        saturating_millis(deadline),
        provider,
        model
    ));
    let started = tokio::time::Instant::now();
    let composed = tokio::time::timeout(
        deadline,
        composer.compose(&request.prompt, length_ms, deadline),
    )
    .await
    .unwrap_or(Err(MusicError::Provider {
        cause: MusicProviderCause::Deadline,
    }));
    let stored = match composed {
        Ok(audio) => {
            let directory = media_directory(app.playground().files());
            store_bytes(directory, file, &audio)
                .await
                .map(|_| u64::try_from(audio.len()).unwrap_or(u64::MAX))
                .map_err(|error| MusicError::Unstored {
                    cause: add_cause(error),
                })
        }
        Err(error) => Err(error),
    };
    let compose_ms = saturating_millis(started.elapsed());
    match stored {
        Ok(bytes) => {
            app.record_line_and_echo(format!(
                concat!(
                    "MUSIC stored name={} file={} bytes={} length_ms={} duration_ms={} ",
                    "provider={} model={}"
                ),
                request.name, file, bytes, length_ms, compose_ms, provider, model
            ));
            Ok(Composed {
                bytes,
                length_ms: u64::from(length_ms),
                compose_ms,
            })
        }
        Err(error) => {
            let cause = match &error {
                MusicError::Provider { cause } => cause.key(),
                MusicError::Unstored { cause } => cause.key(),
                _ => "none",
            };
            app.record_line_and_echo(format!(
                "MUSIC compose_failed name={} error={} cause={} duration_ms={} provider={}",
                request.name,
                error.key(),
                cause,
                compose_ms,
                provider
            ));
            Err(error)
        }
    }
}

/// Plays `request`'s track looping, composing and storing it first when the library has no sound
/// of its name; requests for one name take turns on `tracks`. Nothing on the scene changes unless
/// the music change itself is made.
pub(super) async fn play(
    app: &AppState,
    composer: &dyn Composer,
    tracks: &TrackLocks,
    request: MusicRequest,
    deadline: Duration,
) -> Result<MusicPlayed, MusicError> {
    request.check()?;
    let name = request.name.clone();
    let lock = tracks.of(&name).await;
    let played = {
        let _turn = lock.lock().await;
        play_in_turn(app, composer, request, deadline).await
    };
    tracks.release(&name, lock).await;
    played
}

/// A track the library holds under a request's name: found there or just made.
pub(crate) struct Ensured {
    pub(crate) file: String,
    pub(crate) source: MusicSource,
    pub(crate) bytes: u64,
    pub(crate) length_ms: Option<u64>,
    pub(crate) compose_ms: Option<u64>,
}

/// The library's sounds, for choosing a track.
async fn library_entries(app: &AppState) -> Result<Vec<MediaEntry>, MusicError> {
    let directory = media_directory(app.playground().files());
    run_blocking(move || media_entries(&directory), FileError::Stopped)
        .await
        .map_err(|error| MusicError::LibraryUnreadable {
            cause: file_cause(error),
        })
}

/// The track of `request`'s name, reused from the library or composed and stored, once the
/// request's turn for its name has come. The scene is not touched.
async fn ensure_in_turn(
    app: &AppState,
    composer: &dyn Composer,
    request: &MusicRequest,
    deadline: Duration,
) -> Result<Ensured, MusicError> {
    let entries = library_entries(app).await?;
    match choose_track(&request.name, &entries) {
        Track::Library { file, bytes } => {
            app.record_line(format!(
                "MUSIC reused name={} file={file} bytes={bytes}",
                request.name
            ));
            Ok(Ensured {
                file,
                source: MusicSource::Library,
                bytes,
                length_ms: None,
                compose_ms: None,
            })
        }
        Track::Compose { file } => {
            let composed = compose_and_store(app, composer, request, &file, deadline).await?;
            Ok(Ensured {
                file,
                source: MusicSource::Composed,
                bytes: composed.bytes,
                length_ms: Some(composed.length_ms),
                compose_ms: Some(composed.compose_ms),
            })
        }
    }
}

/// [`play`] once the request's turn for its name has come.
async fn play_in_turn(
    app: &AppState,
    composer: &dyn Composer,
    request: MusicRequest,
    deadline: Duration,
) -> Result<MusicPlayed, MusicError> {
    let Ensured {
        file,
        source,
        bytes,
        length_ms,
        compose_ms,
    } = ensure_in_turn(app, composer, &request, deadline).await?;
    let change = SceneChange::Music {
        media: file.clone(),
        target: request.target,
        volume_percent: request.volume_percent,
        looping: true,
    };
    let reply = apply_scene_change(app, change)
        .await
        .map_err(|error| MusicError::Scene(PlaygroundChangeError::from(error)))?;
    Ok(MusicPlayed {
        file,
        source,
        bytes,
        length_ms,
        compose_ms,
        target: request.target,
        volume_percent: request.volume_percent,
        revision: reply.revision,
    })
}

/// The music engine's one composer and its turns for track names, shared by every route and flow
/// that finds, makes or plays a track (the tempo director's `prepare` uses the same one).
#[derive(Clone)]
pub struct MusicService {
    composer: Arc<dyn Composer>,
    tracks: Arc<TrackLocks>,
}

impl MusicService {
    /// The service over ElevenLabs, with the speech key from the environment.
    pub fn from_environment() -> Self {
        Self::with_composer(Arc::new(ElevenLabsComposer::from_environment()))
    }

    pub(crate) fn with_composer(composer: Arc<dyn Composer>) -> Self {
        Self {
            composer,
            tracks: Arc::new(TrackLocks::default()),
        }
    }

    /// The track of `request`'s name: the library's when it holds one, else composed and stored.
    /// Requests for one name take turns, so a track is paid for once. The scene is not touched.
    pub(crate) async fn ensure(
        &self,
        app: &AppState,
        request: &MusicRequest,
    ) -> Result<Ensured, MusicError> {
        request.check()?;
        let lock = self.tracks.of(&request.name).await;
        let ensured = {
            let _turn = lock.lock().await;
            ensure_in_turn(app, self.composer.as_ref(), request, COMPOSE_DEADLINE).await
        };
        self.tracks.release(&request.name, lock).await;
        ensured
    }
}

/// The library file of each of `names` (a sound of that name), or none, in order. Nothing is
/// composed: this is how a flow learns which tracks it may play without waiting.
pub(crate) async fn held_files(
    app: &AppState,
    names: &[String],
) -> Result<Vec<Option<String>>, MusicError> {
    let entries = library_entries(app).await?;
    Ok(names
        .iter()
        .map(|name| match choose_track(name, &entries) {
            Track::Library { file, .. } => Some(file),
            Track::Compose { .. } => None,
        })
        .collect())
}

struct MusicRoutes {
    app: Arc<AppState>,
    service: MusicService,
}

async fn scene_music(State(routes): State<Arc<MusicRoutes>>, bytes: Bytes) -> Response {
    let result = match parse_body::<MusicRequest>(&bytes) {
        Ok(request) => {
            let (app, service) = (Arc::clone(&routes.app), routes.service.clone());
            run_owned(
                async move {
                    play(
                        &app,
                        service.composer.as_ref(),
                        &service.tracks,
                        request,
                        COMPOSE_DEADLINE,
                    )
                    .await
                },
                MusicError::Stopped,
            )
            .await
        }
        Err(error) => Err(MusicError::InvalidBody(error)),
    };
    if let Err(error) = &result {
        routes
            .app
            .record_line(format!("MUSIC refused error={}", error.key()));
    }
    reply(result)
}

/// The music route over `service`.
pub fn router_over(app: Arc<AppState>, service: MusicService) -> Router {
    Router::new()
        .route(path::SCENE_MUSIC, post(scene_music))
        .with_state(Arc::new(MusicRoutes { app, service }))
}

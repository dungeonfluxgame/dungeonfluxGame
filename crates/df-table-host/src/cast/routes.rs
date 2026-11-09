//! The cast routes. Each resolves the shown scene's current layout once ([`shown_cast`]), works
//! by character name, and answers a `CastReply`; a refusal is a typed [`CastError`] naming the
//! character and the names the layout does have. Cues and camera moves are applied through
//! `playground::apply_scene_change`, which records its own refusals; a cast route records only
//! the refusals it finds before that, so each refusal is one audit line.

use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, post};
use df_playground::fixture::PlateKind;
use df_playground::lobby::change::ScreenEdit;
use df_playground::scene::SceneChange;
use df_scene_layout::{
    CharacterCue, CueKind, REGION_TARGET_PREFIX, SceneLayout, Shot, cast, region_target,
    shot_zoom_percent,
};
use tokio::task::JoinHandle;

use super::request::{CameraMoved, CameraRequest, CastError, CastList, Cued};
use crate::dev_http::{parse_body, reply};
use crate::persona::summaries_of;
use crate::shown_scene::{ShownCast, named, shown_cast};
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

/// How long the camera takes to ease onto a speaker, and back out to the wide plate, in
/// milliseconds: slow enough to follow, with the page's ease-in-out curve.
const SPEAKER_CAMERA_MS: u64 = 2_000;
/// How long the camera holds on a speaker after the last line is cued before it eases back out to
/// the wide plate, plus [`REST_PER_CHARACTER_MS`] for each character of the line. It is long on
/// purpose: the next line of a talk arrives within it and must not make the camera swing out and
/// in again.
pub(crate) const SPEAKER_REST_MS: u64 = 15_000;
/// What a cued line's rest adds for each of its characters, so a longer line stays up long enough
/// to read across the room and hear out.
const REST_PER_CHARACTER_MS: u64 = 70;

/// The rest after a cued line of `characters` characters, in milliseconds: `base_ms` plus
/// [`REST_PER_CHARACTER_MS`] for each character. A count past u64 saturates, as the rest of the
/// host's counts do.
pub(crate) fn line_rest_ms(base_ms: u64, characters: usize) -> u64 {
    REST_PER_CHARACTER_MS
        .saturating_mul(u64::try_from(characters).unwrap_or(u64::MAX))
        .saturating_add(base_ms)
}

/// The host's one pending pull-back after the last spoken line. A new line cancels it
/// ([`Self::cancel`]) and the next one stored replaces it, so at most one timer exists; dropping
/// the host aborts it.
#[derive(Default)]
pub(crate) struct SpeakerRest {
    pending: Mutex<Option<JoinHandle<()>>>,
}

impl SpeakerRest {
    /// Aborts the pending pull-back, if any: a line was cued, so the talk goes on.
    pub(crate) fn cancel(&self) {
        let pending = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(task) = pending {
            task.abort();
        }
    }

    /// Keeps `task` as the pending pull-back, aborting any earlier one.
    pub(crate) fn keep(&self, task: JoinHandle<()>) {
        let earlier = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(task);
        if let Some(earlier) = earlier {
            earlier.abort();
        }
    }
}

impl Drop for SpeakerRest {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// What the camera does when a line is cued for a speaker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SpeakerMove {
    /// It stays as it is: the speaker is already framed, or the camera is held elsewhere on
    /// purpose.
    Hold,
    /// It eases to the gentle speaker shot of `target`.
    Push { target: String, zoom_percent: u16 },
    /// It eases back to the whole stage: the speaker has no region to frame.
    Wide,
}

/// The character a camera target frames, if it is one of the region targets of a layout
/// (`region:<name>` or `region:<name>:<shot>`).
fn framed_character(target: &str) -> Option<&str> {
    let rest = target.strip_prefix(REGION_TARGET_PREFIX)?;
    Some(rest.split_once(':').map_or(rest, |(name, _)| name))
}

/// What the camera does for a line of `speaker` on the backdrop with `layout` (`None` when it has
/// none, such as an item close-up) when it now looks at `current`. A camera held on something
/// other than a character is left alone.
pub(crate) fn speaker_move(
    layout: Option<&SceneLayout>,
    speaker: &str,
    current: Option<&str>,
) -> SpeakerMove {
    let framed = current.and_then(framed_character);
    match layout.and_then(|layout| layout.region(speaker)) {
        Some(region) => match (framed, current) {
            (Some(name), _) if name == speaker => SpeakerMove::Hold,
            (Some(_), _) | (None, None) => SpeakerMove::Push {
                target: region_target(speaker, Some(Shot::Speaker)),
                zoom_percent: shot_zoom_percent(region, Shot::Speaker),
            },
            (None, Some(_)) => SpeakerMove::Hold,
        },
        None if framed.is_some() => SpeakerMove::Wide,
        None => SpeakerMove::Hold,
    }
}

/// The camera's part in a spoken line: decided before the cue and applied in the same scene
/// change, then announced once the cue is applied.
pub(crate) struct SpeakerShot {
    character: String,
    movement: SpeakerMove,
    line_characters: usize,
}

impl SpeakerShot {
    /// The camera change to apply with the line's cue, if the camera moves.
    pub(crate) fn change(&self) -> Option<SceneChange> {
        match &self.movement {
            SpeakerMove::Hold => None,
            SpeakerMove::Push {
                target,
                zoom_percent,
            } => Some(SceneChange::Camera {
                target: Some(target.clone()),
                zoom_percent: *zoom_percent,
                duration_ms: SPEAKER_CAMERA_MS,
            }),
            SpeakerMove::Wide => Some(wide_change()),
        }
    }

    /// Records the move as `CAMERA speaker character=.. move=hold|push|wide target=.. zoom=..
    /// over=.. revision=..` once the cue is applied, and arms the pull-back after the last line.
    pub(crate) fn applied(self, app: &Arc<AppState>, revision: u64) {
        let (name, target, zoom_percent, over_ms) = match &self.movement {
            SpeakerMove::Hold => ("hold", "-", 0, 0),
            SpeakerMove::Push {
                target,
                zoom_percent,
            } => ("push", target.as_str(), *zoom_percent, SPEAKER_CAMERA_MS),
            SpeakerMove::Wide => ("wide", "-", 100, SPEAKER_CAMERA_MS),
        };
        app.record_line(format!(
            concat!(
                "CAMERA speaker character={} move={} target={} zoom={} over={} ",
                "revision={}"
            ),
            self.character, name, target, zoom_percent, over_ms, revision
        ));
        let rest_ms = line_rest_ms(SPEAKER_REST_MS, self.line_characters);
        // The task holds the host weakly, so the host that owns the task can be dropped.
        let host: Weak<AppState> = Arc::downgrade(app);
        let character = self.character;
        let task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(rest_ms)).await;
            if let Some(app) = host.upgrade() {
                pull_back(&app, &character, rest_ms).await;
            }
        });
        app.speaker_rest().keep(task);
    }
}

fn wide_change() -> SceneChange {
    SceneChange::Camera {
        target: None,
        zoom_percent: df_playground::limits::MINIMUM_ZOOM_PERCENT,
        duration_ms: SPEAKER_CAMERA_MS,
    }
}

/// The scene change that takes the bubble, and with it the outline, of `character` away.
fn quiet_change(character: &str) -> SceneChange {
    SceneChange::Screen {
        edit: ScreenEdit::Cue {
            cue: CharacterCue {
                character: character.to_owned(),
                kind: CueKind::Quiet,
            },
        },
    }
}

/// After `rest_ms` without a line, the last speaker's line leaves (bubble and outline) and the
/// camera eases back to the whole stage if it still frames a character:
/// `CAMERA speaker move=rest rest_ms=.. revision=..`.
async fn pull_back(app: &AppState, character: &str, rest_ms: u64) {
    let mut changes = vec![quiet_change(character)];
    if app
        .playground()
        .camera_target()
        .as_deref()
        .and_then(framed_character)
        .is_some()
    {
        changes.push(wide_change());
    }
    // A refusal is recorded by the scene path.
    if let Ok(applied) = crate::playground::apply_scene_changes(app, changes).await {
        app.record_line_and_echo(format!(
            concat!(
                "CAMERA speaker character={} move=rest rest_ms={} over={} ",
                "revision={}"
            ),
            character, rest_ms, SPEAKER_CAMERA_MS, applied.revision
        ));
    }
}

/// Decides the camera's part in a line of `speaker` that is about to be cued.
pub(crate) async fn speaker_shot(app: &AppState, speaker: &str, line: &str) -> SpeakerShot {
    app.speaker_rest().cancel();
    // Only a room plate shown with its own scan frames anyone: a close-up, or a room shown with
    // the cast before it, has nobody where that layout's figures stood.
    let layout = shown_cast(app)
        .await
        .ok()
        .filter(|shown| shown.plate == PlateKind::Room)
        .map(|shown| shown.layout);
    let current = app.playground().camera_target();
    SpeakerShot {
        character: speaker.to_owned(),
        movement: speaker_move(layout.as_ref(), speaker, current.as_deref()),
        line_characters: line.chars().count(),
    }
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
    let listed = async {
        let ShownCast {
            picture, layout, ..
        } = shown_cast(&app).await?;
        let members = cast(&layout);
        let names = members.iter().map(|member| member.name.clone()).collect();
        let personas = summaries_of(&app, names).await;
        Ok::<_, CastError>(CastList {
            picture,
            members,
            personas,
        })
    }
    .await;
    answer(&app, "cast", listed)
}

async fn cue(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let checked = async {
        let cue: CharacterCue = parse_body(&bytes)?;
        let ShownCast {
            picture, layout, ..
        } = shown_cast(&app).await?;
        named(&layout, &picture, &cue.character)?;
        Ok::<_, CastError>(cue)
    }
    .await;
    let cue = match checked {
        Ok(cue) => cue,
        Err(error) => return answer::<Cued>(&app, "cue", Err(error)),
    };
    let edit = ScreenEdit::Cue { cue: cue.clone() };
    let mut changes = vec![SceneChange::Screen { edit }];
    let shot = match &cue.kind {
        CueKind::Speaking { line } => {
            // One character at a time has the floor: only the speaker keeps a line and outline.
            if let Ok(shown) = shown_cast(&app).await {
                changes.extend(
                    shown
                        .layout
                        .names()
                        .into_iter()
                        .filter(|name| *name != cue.character)
                        .map(quiet_change),
                );
            }
            Some(speaker_shot(&app, &cue.character, line).await)
        }
        CueKind::Thinking | CueKind::Quiet => None,
    };
    changes.extend(shot.as_ref().and_then(SpeakerShot::change));
    let result = crate::playground::apply_scene_changes(&app, changes)
        .await
        .map(|reply| {
            if let Some(shot) = shot {
                shot.applied(&app, reply.revision);
            }
            Cued {
                cue,
                revision: reply.revision,
            }
        })
        .map_err(CastError::from);
    reply(result)
}

async fn camera_on(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let checked = async {
        let request: CameraRequest = parse_body(&bytes)?;
        let shot = request.shot.unwrap_or(Shot::Upper);
        let ShownCast {
            picture, layout, ..
        } = shown_cast(&app).await?;
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

//! The persona routes. Each runs its file work on the blocking pool and answers a `PersonaReply`;
//! a refusal is a typed [`PersonaError`] and one `PERSONA refused verb=.. [id=..] error=<key>`
//! audit line, a write is `PERSONA written id=<id>`, an edit `PERSONA edited id=<id> edits=<n>
//! changed=<bool>` and a removal `PERSONA removed id=<id>`. The questions to the text service
//! (`generate`, `generate-cast`, `say`, `ask`, `coverage`, `backfill`) are in `generate` and
//! `backfill` and write their own lines. A line never holds persona text, a hint, a question, a
//! situation or a spoken line.
//!
//! Every paid sequence (a generation, a coverage-backfill-line sequence, a line) runs on a task of
//! its own that the handler waits for, so a client that leaves cannot lose an answer already paid
//! for or stop `ask` between its coverage and its backfill (`blocking::run_owned`).

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use df_content::persona::{Persona, PersonaId};
use df_playground::lobby::change::ScreenEdit;
use df_playground::scene::SceneChange;
use df_scene_layout::{CharacterCue, CueKind};
use df_table::ports::Clock;

use super::backfill::{BackfillJob, backfill_persona, check_gaps, consider, coverage_of};
use super::generate::{
    Occasion, ShownScene, Writing, generate_cast, generate_character, load_persona, persona_route,
    say_line,
};
use super::request::{
    Answered, AskRequest, Audience, BackfillRecord, BackfillRequest, Backfilled, BackfillsQuery,
    CastRow, CoverageRequest, Covered, EditRequest, Edited, GenerateCastRequest, GenerateRequest,
    Generation, PersonaError, PersonaRemoved, PersonaStored, RemoveRequest, SaidLine, SayRequest,
    ShowQuery, Spoken, summary_of,
};
use super::storage::{
    PERSONA_WRITES, check_reserved, list_personas, persona_directory, persona_id, read_backfills,
    read_persona, remove_persona, write_persona,
};
use crate::blocking::{run_blocking, run_owned};
use crate::dev_http::{parse_body, reply};
use crate::observe::WallClock;
use crate::speech::{Speech, SpeechFeature, Voiced, sound_change};
use crate::state::AppState;
use crate::text::{TextRoute, TextRuntime};

/// The addresses of the persona dev API.
pub mod path {
    /// The stored personas (GET), or stores one, replacing a persona of the same id (POST a
    /// persona's JSON).
    pub const PERSONA: &str = df_playground::path::PERSONA_LIST;
    /// One persona (GET, with `?audience=public|owner`).
    pub const PERSONA_ONE: &str = "/dev/persona/{id}";
    /// Removes a persona (POST a `RemoveRequest`). Its backfill record stays.
    pub const PERSONA_REMOVE: &str = "/dev/persona/remove";
    /// Changes a persona with typed edits (POST an `EditRequest`).
    pub const PERSONA_EDIT: &str = "/dev/persona/edit";
    /// Has the text service write the persona of a character (POST a `GenerateRequest`).
    pub const PERSONA_GENERATE: &str = "/dev/persona/generate";
    /// Has it write the persona of every character of the scene (POST a `GenerateCastRequest`).
    pub const PERSONA_GENERATE_CAST: &str = "/dev/persona/generate-cast";
    /// Has it word a line from a character and cues it (POST a `SayRequest`). A dev tool.
    pub const PERSONA_SAY: &str = "/dev/persona/say";
    /// Puts a question to a character: coverage, backfill of gaps, then the line (POST an
    /// `AskRequest`). A dev tool.
    pub const PERSONA_ASK: &str = df_playground::path::PERSONA_ASK;
    /// Whether the persona holds enough to answer a question (POST a `CoverageRequest`).
    pub const PERSONA_COVERAGE: &str = "/dev/persona/coverage";
    /// Fills gaps of a persona by adding detail (POST a `BackfillRequest`).
    pub const PERSONA_BACKFILL: &str = "/dev/persona/backfill";
    /// The backfills committed to a persona (GET with `?id=`).
    pub const PERSONA_BACKFILLS: &str = "/dev/persona/backfills";
}

/// What every persona route works with: the host, the text surface the questions go to, and the
/// speech service that voices a line when asked.
struct PersonaRoutes {
    app: Arc<AppState>,
    text: Arc<TextRuntime>,
    speech: Speech,
}

/// `text` when it is a persona id, so it can go into an audit line.
fn known_id(text: &str) -> Option<&str> {
    PersonaId::new(text).ok().map(|_| text)
}

/// Answers `result`, recording a refusal first as `PERSONA refused verb=.. [id=..] error=<key>`.
fn answer<T: serde::Serialize>(
    app: &AppState,
    verb: &str,
    id: Option<&str>,
    result: Result<T, PersonaError>,
) -> Response {
    if let Err(error) = &result {
        let id = id.map_or_else(String::new, |id| format!(" id={id}"));
        app.record_line(format!(
            "PERSONA refused verb={verb}{id} error={}",
            error.key()
        ));
    }
    reply(result)
}

/// The stored persona of `id`, or `UnknownPersona`.
fn stored(directory: &std::path::Path, id: &PersonaId) -> Result<Persona, PersonaError> {
    read_persona(directory, id)?.ok_or_else(|| PersonaError::UnknownPersona { id: id.to_string() })
}

async fn list(State(routes): State<Arc<PersonaRoutes>>) -> Response {
    let directory = persona_directory(&routes.app);
    let result = run_blocking(move || list_personas(&directory), PersonaError::Stopped)
        .await
        .map(|personas| personas.iter().map(summary_of).collect::<Vec<_>>());
    answer(&routes.app, "list", None, result)
}

async fn show(
    State(routes): State<Arc<PersonaRoutes>>,
    Path(id): Path<String>,
    query: Result<Query<ShowQuery>, QueryRejection>,
) -> Response {
    let app = &routes.app;
    let known = known_id(&id);
    let checked = match query {
        Ok(Query(query)) => persona_id(&id).map(|id| (id, query.audience)),
        Err(rejection) => Err(PersonaError::InvalidQuery {
            detail: rejection.body_text(),
        }),
    };
    let (id, audience) = match checked {
        Ok(checked) => checked,
        Err(error) => return answer::<Persona>(app, "show", known, Err(error)),
    };
    let directory = persona_directory(app);
    let shown = id.to_string();
    let read = run_blocking(move || stored(&directory, &id), PersonaError::Stopped).await;
    match audience {
        Audience::Public => answer(
            app,
            "show",
            Some(&shown),
            read.map(|persona| persona.public_view()),
        ),
        Audience::Owner => answer(app, "show", Some(&shown), read),
    }
}

async fn write(State(routes): State<Arc<PersonaRoutes>>, bytes: Bytes) -> Response {
    let app = &routes.app;
    let written = async {
        let persona: Persona = parse_body(&bytes)?;
        check_reserved(persona.id())?;
        let directory = persona_directory(app);
        run_blocking(
            move || {
                let _writing = PERSONA_WRITES.lock();
                let replaced = write_persona(&directory, &persona)?;
                Ok(PersonaStored {
                    persona: summary_of(&persona),
                    replaced,
                })
            },
            PersonaError::Stopped,
        )
        .await
    }
    .await;
    if let Ok(stored) = &written {
        app.record_line(format!("PERSONA written id={}", stored.persona.id));
    }
    answer(app, "write", None, written)
}

async fn edit(State(routes): State<Arc<PersonaRoutes>>, bytes: Bytes) -> Response {
    let app = &routes.app;
    let request: EditRequest = match parse_body(&bytes) {
        Ok(request) => request,
        Err(error) => return answer::<Edited>(app, "edit", None, Err(error.into())),
    };
    let known = known_id(&request.id).map(str::to_owned);
    let edited = async {
        let id = persona_id(&request.id)?;
        let directory = persona_directory(app);
        let edits = request.edits.len();
        let edited = run_blocking(
            move || {
                let _writing = PERSONA_WRITES.lock();
                let before = stored(&directory, &id)?;
                let after = before
                    .edit(&request.edits)
                    .map_err(|cause| PersonaError::InvalidEdit { cause })?;
                let changed = after != before;
                if changed {
                    write_persona(&directory, &after)?;
                }
                Ok(Edited {
                    persona: after,
                    changed,
                })
            },
            PersonaError::Stopped,
        )
        .await?;
        app.record_line(format!(
            "PERSONA edited id={} edits={edits} changed={}",
            edited.persona.id(),
            edited.changed
        ));
        Ok::<_, PersonaError>(edited)
    }
    .await;
    answer(app, "edit", known.as_deref(), edited)
}

async fn remove(State(routes): State<Arc<PersonaRoutes>>, bytes: Bytes) -> Response {
    let app = &routes.app;
    let request: RemoveRequest = match parse_body(&bytes) {
        Ok(request) => request,
        Err(error) => return answer::<PersonaRemoved>(app, "remove", None, Err(error.into())),
    };
    let known = known_id(&request.id).map(str::to_owned);
    let removed = async {
        let id = persona_id(&request.id)?;
        let directory = persona_directory(app);
        run_blocking(
            move || {
                let _writing = PERSONA_WRITES.lock();
                if remove_persona(&directory, &id)? {
                    Ok(PersonaRemoved { id: id.to_string() })
                } else {
                    Err(PersonaError::UnknownPersona { id: id.to_string() })
                }
            },
            PersonaError::Stopped,
        )
        .await
    }
    .await;
    if let Ok(removed) = &removed {
        app.record_line(format!("PERSONA removed id={}", removed.id));
    }
    answer(app, "remove", known.as_deref(), removed)
}

async fn generate(State(routes): State<Arc<PersonaRoutes>>, bytes: Bytes) -> Response {
    let request: GenerateRequest = match parse_body(&bytes) {
        Ok(request) => request,
        Err(error) => {
            return answer::<Generation>(&routes.app, "generate", None, Err(error.into()));
        }
    };
    let known = known_id(&request.character).map(str::to_owned);
    let owned = Arc::clone(&routes);
    let result = run_owned(
        async move {
            let scene = ShownScene::read(&owned.app).await?;
            scene.check(&request.character)?;
            let writing = Writing {
                hint: request.hint.as_deref(),
                guidance: request.guidance.as_ref(),
                model: request.model.as_deref(),
                overwrite: request.overwrite,
            };
            // A failure of the generation itself is recorded where it happened.
            Ok(
                generate_character(&owned.app, &owned.text, &scene, &request.character, writing)
                    .await,
            )
        },
        PersonaError::Stopped,
    )
    .await;
    match result {
        Ok(generation) => reply(generation),
        Err(error) => answer::<Generation>(&routes.app, "generate", known.as_deref(), Err(error)),
    }
}

async fn generate_scene_cast(State(routes): State<Arc<PersonaRoutes>>, bytes: Bytes) -> Response {
    let request: GenerateCastRequest = match parse_body(&bytes) {
        Ok(request) => request,
        Err(error) => {
            return answer::<Vec<CastRow>>(&routes.app, "generate-cast", None, Err(error.into()));
        }
    };
    let owned = Arc::clone(&routes);
    let result = run_owned(
        async move {
            let scene = ShownScene::read(&owned.app).await?;
            let writing = Writing {
                hint: request.hint.as_deref(),
                guidance: request.guidance.as_ref(),
                model: request.model.as_deref(),
                overwrite: request.overwrite,
            };
            Ok(generate_cast(&owned.app, &owned.text, &scene, writing).await)
        },
        PersonaError::Stopped,
    )
    .await;
    answer(&routes.app, "generate-cast", None, result)
}

/// The scene change that shows `kind` over `character`.
fn cue_change(character: &str, kind: CueKind) -> SceneChange {
    SceneChange::Screen {
        edit: ScreenEdit::Cue {
            cue: CharacterCue {
                character: character.to_owned(),
                kind,
            },
        },
    }
}

/// The scene changes that take the bubbles of `others` away when a line is cued, so that one
/// character at a time has the floor: the last speaker's bubble stays up until this one arrives.
fn quiet_changes(others: &[String]) -> Vec<SceneChange> {
    others
        .iter()
        .map(|other| cue_change(other, CueKind::Quiet))
        .collect()
}

/// The named characters of `scene` other than `character`.
fn others_of(scene: &ShownScene, character: &str) -> Vec<String> {
    let mut names = scene.names();
    names.retain(|name| name != character);
    names
}

// TODO(DEV-12): a two-character host test that a thinking cue leaves the other's bubble up.
/// Shows the wordless thinking indicator over `character` while the model and the voice work. It
/// takes nothing else away: the last speaker's bubble stays up until the next line replaces it.
async fn cue_thinking(app: &AppState, character: &str) -> Result<(), PersonaError> {
    crate::playground::apply_scene_change(app, cue_change(character, CueKind::Thinking)).await?;
    Ok(())
}

/// Takes the thinking indicator away after a turn that failed.
async fn clear_thinking(app: &AppState, character: &str) {
    // A refusal is recorded by the scene path, and the failure being reported is the turn's own.
    let _ = crate::playground::apply_scene_change(app, cue_change(character, CueKind::Quiet)).await;
}

/// Applies `said` as the speaking cue of `character` through the one scene path `scene cue` takes,
/// which records its own refusals. When `speak` is asked the line is voiced first and its sound
/// is cued in the same scene change, so the bubble and the voice start together; a voice that
/// fails is reported and the bubble is cued without it.
async fn cue_line(
    routes: &PersonaRoutes,
    character: String,
    said: SaidLine,
    speak: bool,
    others: &[String],
) -> Result<Spoken, PersonaError> {
    let voice = if speak && routes.speech.is_on(SpeechFeature::Tts) {
        Some(routes.speech.voice_line(&character, &said.line).await)
    } else {
        None
    };
    let mut changes = quiet_changes(others);
    changes.push(cue_change(
        &character,
        CueKind::Speaking {
            line: said.line.clone(),
        },
    ));
    if let Some(Voiced::Played { clip, .. }) = &voice {
        changes.push(sound_change(clip));
    }
    let applied = crate::playground::apply_scene_changes(&routes.app, changes).await?;
    Ok(Spoken {
        character,
        said,
        revision: applied.revision,
        voice,
    })
}

/// The checked scene, route and stored persona a question about `character` is asked of.
async fn asked_of(
    routes: &PersonaRoutes,
    character: &str,
    model: Option<&str>,
) -> Result<(ShownScene, Arc<TextRoute>, Persona), PersonaError> {
    let scene = ShownScene::read(&routes.app).await?;
    scene.check(character)?;
    let own = load_persona(&routes.app, character).await?;
    let route = persona_route(&routes.text, model)?;
    Ok((scene, route, own))
}

async fn say(State(routes): State<Arc<PersonaRoutes>>, bytes: Bytes) -> Response {
    let request: SayRequest = match parse_body(&bytes) {
        Ok(request) => request,
        Err(error) => return answer::<Spoken>(&routes.app, "say", None, Err(error.into())),
    };
    let known = known_id(&request.character).map(str::to_owned);
    let owned = Arc::clone(&routes);
    let result = run_owned(
        async move {
            let (scene, route, own) =
                asked_of(&owned, &request.character, request.model.as_deref()).await?;
            let others = others_of(&scene, &request.character);
            cue_thinking(&owned.app, &request.character).await?;
            let occasion = Occasion {
                situation: &request.situation,
                question: None,
                guidance: None,
                not_established: &[],
            };
            let said = match say_line(&owned.app, &route, &scene, &own, occasion).await {
                Ok(said) => said,
                Err(error) => {
                    clear_thinking(&owned.app, &request.character).await;
                    return Err(error);
                }
            };
            // The cue goes through the scene path, which records its own refusals.
            let spoken = cue_line(
                &owned,
                request.character.clone(),
                said,
                request.speak,
                &others,
            )
            .await;
            if spoken.is_err() {
                clear_thinking(&owned.app, &request.character).await;
            }
            Ok(spoken)
        },
        PersonaError::Stopped,
    )
    .await;
    match result {
        Ok(spoken) => reply(spoken),
        Err(error) => answer::<Spoken>(&routes.app, "say", known.as_deref(), Err(error)),
    }
}

async fn ask(State(routes): State<Arc<PersonaRoutes>>, bytes: Bytes) -> Response {
    let request: AskRequest = match parse_body(&bytes) {
        Ok(request) => request,
        Err(error) => return answer::<Answered>(&routes.app, "ask", None, Err(error.into())),
    };
    let known = known_id(&request.character).map(str::to_owned);
    let owned = Arc::clone(&routes);
    let result = run_owned(
        async move {
            let (scene, route, _) =
                asked_of(&owned, &request.character, request.model.as_deref()).await?;
            let others = others_of(&scene, &request.character);
            cue_thinking(&owned.app, &request.character).await?;
            let at_ms = WallClock.now_ms();
            let considered = match consider(&owned.app, &route, &scene, &request, at_ms).await {
                Ok(considered) => considered,
                Err(error) => {
                    clear_thinking(&owned.app, &request.character).await;
                    return Err(error);
                }
            };
            let said = considered.said.clone();
            let spoken = cue_line(
                &owned,
                request.character.clone(),
                said,
                request.speak,
                &others,
            )
            .await;
            if spoken.is_err() {
                clear_thinking(&owned.app, &request.character).await;
            }
            Ok(spoken.map(|spoken| considered.answered(spoken)))
        },
        PersonaError::Stopped,
    )
    .await;
    match result {
        Ok(answered) => reply(answered),
        Err(error) => answer::<Answered>(&routes.app, "ask", known.as_deref(), Err(error)),
    }
}

async fn coverage(State(routes): State<Arc<PersonaRoutes>>, bytes: Bytes) -> Response {
    let request: CoverageRequest = match parse_body(&bytes) {
        Ok(request) => request,
        Err(error) => return answer::<Covered>(&routes.app, "coverage", None, Err(error.into())),
    };
    let known = known_id(&request.character).map(str::to_owned);
    let owned = Arc::clone(&routes);
    let result = run_owned(
        async move {
            let (_, route, own) =
                asked_of(&owned, &request.character, request.model.as_deref()).await?;
            let found = coverage_of(
                &owned.app,
                &route,
                &own,
                &request.question,
                request.guidance.as_ref(),
            )
            .await?;
            Ok(Covered {
                enough: found.enough,
                gaps: found.gaps,
                profile: route.profile().as_str().to_owned(),
            })
        },
        PersonaError::Stopped,
    )
    .await;
    answer(&routes.app, "coverage", known.as_deref(), result)
}

async fn backfill(State(routes): State<Arc<PersonaRoutes>>, bytes: Bytes) -> Response {
    let request: BackfillRequest = match parse_body(&bytes) {
        Ok(request) => request,
        Err(error) => {
            return answer::<Backfilled>(&routes.app, "backfill", None, Err(error.into()));
        }
    };
    let known = known_id(&request.character).map(str::to_owned);
    if let Err(error) = check_gaps(&request.gaps) {
        return answer::<Backfilled>(&routes.app, "backfill", known.as_deref(), Err(error));
    }
    let owned = Arc::clone(&routes);
    let result = run_owned(
        async move {
            let (scene, route, own) =
                asked_of(&owned, &request.character, request.model.as_deref()).await?;
            let job = BackfillJob {
                question: None,
                gaps: &request.gaps,
                guidance: request.guidance.as_ref(),
                at_ms: WallClock.now_ms(),
            };
            // A failure of the backfill itself is recorded where it happened.
            Ok(backfill_persona(&owned.app, &route, &scene, &own, job)
                .await
                .map(|committed| committed.done))
        },
        PersonaError::Stopped,
    )
    .await;
    match result {
        Ok(done) => reply(done),
        Err(error) => answer::<Backfilled>(&routes.app, "backfill", known.as_deref(), Err(error)),
    }
}

async fn backfills(
    State(routes): State<Arc<PersonaRoutes>>,
    query: Result<Query<BackfillsQuery>, QueryRejection>,
) -> Response {
    let app = &routes.app;
    let query = query.map(|Query(query)| query);
    let known = query
        .as_ref()
        .ok()
        .and_then(|query| known_id(&query.id))
        .map(str::to_owned);
    let read = async {
        let query = query.map_err(|rejection| PersonaError::InvalidQuery {
            detail: rejection.body_text(),
        })?;
        let id = persona_id(&query.id)?;
        let directory = persona_directory(app);
        run_blocking(
            move || read_backfills(&directory, &id),
            PersonaError::Stopped,
        )
        .await
    }
    .await;
    answer::<Vec<BackfillRecord>>(app, "backfills", known.as_deref(), read)
}

/// The persona routes, asking the text service of `text` and voicing lines with `speech`.
pub fn router_with(app: Arc<AppState>, text: Arc<TextRuntime>, speech: Speech) -> Router {
    Router::new()
        .route(path::PERSONA, get(list).post(write))
        .route(path::PERSONA_ONE, get(show))
        .route(path::PERSONA_REMOVE, post(remove))
        .route(path::PERSONA_EDIT, post(edit))
        .route(path::PERSONA_GENERATE, post(generate))
        .route(path::PERSONA_GENERATE_CAST, post(generate_scene_cast))
        .route(path::PERSONA_SAY, post(say))
        .route(path::PERSONA_ASK, post(ask))
        .route(path::PERSONA_COVERAGE, post(coverage))
        .route(path::PERSONA_BACKFILL, post(backfill))
        .route(path::PERSONA_BACKFILLS, get(backfills))
        .with_state(Arc::new(PersonaRoutes { app, text, speech }))
}

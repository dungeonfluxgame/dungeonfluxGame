//! The dev routes of the emotional layer, answering a `DevReply` whose error is an
//! [`EmotionError`]; a refusal is also one `PERSONA refused verb=.. [id=..] error=<key>` audit
//! line, like the other persona routes.
//!
//! - `GET /dev/persona/state?id=`: the live state of a stored persona and the emotional triggers
//!   whose numeric condition holds in it (a `StateView`).
//! - `POST /dev/persona/state/reset` (`StateTarget`): forgets the state, so the persona rests at
//!   its temperament again.
//! - `POST /dev/persona/state/set` (`SetEmotionRequest`): sets one emotion's level by hand.
//! - `POST /dev/persona/temperament` (`TemperamentRequest`): the text service writes the
//!   persona's temperament and emotional triggers; emotion ids are checked against the emotion
//!   list and behavior ids against the persona, never by matching words.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use df_content::persona::emotion_generation::{
    TEMPERAMENT_INSTRUCTION, TEMPERAMENT_OUTPUT_NAME, read_temperament, temperament_facts,
    temperament_schema,
};
use df_content::persona::{
    EmotionConfig, Persona, PersonaEdit, PersonaState, Temperament, set_emotion, set_known_name,
    set_rapport,
};
use tokio::time::Instant;

use super::emotions::{EmotionError, emotions_path, load_emotions, read_emotions};
use super::generate::{load_persona, operation_key, persona_route, text_request};
use super::request::{Edited, PersonaError};
use super::routes::apply_edits;
use super::state::{change_state, current_state, reset_state, state_directory, state_triggers};
use super::state_view::{
    SetEmotionRequest, SetKnownRequest, SetRapportRequest, StateTarget, StateView,
    TemperamentRequest,
};
use super::storage::{persona_directory, persona_id, read_persona};
use crate::blocking::{run_blocking, run_owned};
use crate::dev_http::{parse_body, reply};
use crate::state::AppState;
use crate::text::{TextRoute, TextRuntime};

/// The addresses of the emotion dev API.
pub mod path {
    /// The live state of a persona (GET with `?id=`).
    pub const STATE: &str = "/dev/persona/state";
    /// Forgets a persona's state (POST a `StateTarget`).
    pub const STATE_RESET: &str = "/dev/persona/state/reset";
    /// Sets one emotion of a persona by hand (POST a `SetEmotionRequest`).
    pub const STATE_SET: &str = "/dev/persona/state/set";
    /// Sets a persona's standing toward another character by hand (POST a `SetRapportRequest`).
    pub const STATE_RAPPORT: &str = "/dev/persona/state/rapport";
    /// Says whether a persona knows another character's name (POST a `SetKnownRequest`).
    pub const STATE_KNOWS: &str = "/dev/persona/state/knows";
    /// Has the text service write a persona's temperament and emotional triggers (POST a
    /// `TemperamentRequest`).
    pub const TEMPERAMENT: &str = "/dev/persona/temperament";
}

struct EmotionRoutes {
    app: Arc<AppState>,
    text: Arc<TextRuntime>,
}

/// Answers `result`, recording a refusal first as `PERSONA refused verb=.. [id=..] error=<key>`.
fn answer<T: serde::Serialize>(
    app: &AppState,
    verb: &str,
    id: Option<&str>,
    result: Result<T, EmotionError>,
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

/// The state of the stored persona `id` as a view: the persona must exist, the emotion list is
/// read now, and a persona with no state file is at its temperament's rest.
async fn view_of(app: &AppState, id: &str) -> Result<StateView, EmotionError> {
    let id = persona_id(id)?;
    let personas = persona_directory(app);
    let states = state_directory(app);
    let emotions = emotions_path(app);
    run_blocking(
        move || {
            let persona = read_persona(&personas, &id)?
                .ok_or_else(|| PersonaError::UnknownPersona { id: id.to_string() })?;
            let config = read_emotions(&emotions)?;
            let state = current_state(&states, id.as_str(), &config, persona.temperament())?;
            let holding = state_triggers(&persona, &state)
                .into_iter()
                .map(|trigger| trigger.id.clone())
                .collect();
            Ok(StateView { state, holding })
        },
        EmotionError::Stopped,
    )
    .await
}

async fn show(
    State(routes): State<Arc<EmotionRoutes>>,
    query: Result<Query<StateTarget>, QueryRejection>,
) -> Response {
    let app = &routes.app;
    let result = match query {
        Ok(Query(target)) => view_of(app, &target.id).await,
        Err(rejection) => Err(PersonaError::InvalidQuery {
            detail: rejection.body_text(),
        }
        .into()),
    };
    answer(app, "state", None, result)
}

async fn reset(State(routes): State<Arc<EmotionRoutes>>, bytes: Bytes) -> Response {
    let app = &routes.app;
    let request: StateTarget = match parse_body(&bytes) {
        Ok(request) => request,
        Err(error) => return answer::<StateView>(app, "state-reset", None, Err(error.into())),
    };
    let result = async {
        view_of(app, &request.id).await?;
        let states = state_directory(app);
        let id = request.id.clone();
        run_blocking(move || reset_state(&states, &id), EmotionError::Stopped).await?;
        view_of(app, &request.id).await
    }
    .await;
    if result.is_ok() {
        app.record_line(format!("PERSONA state_reset id={}", request.id));
    }
    answer(app, "state-reset", Some(&request.id), result)
}

async fn set(State(routes): State<Arc<EmotionRoutes>>, bytes: Bytes) -> Response {
    let app = &routes.app;
    let request: SetEmotionRequest = match parse_body(&bytes) {
        Ok(request) => request,
        Err(error) => return answer::<StateView>(app, "state-set", None, Err(error.into())),
    };
    let (emotion, level) = (request.emotion.clone(), request.level);
    let result = change_stored(app, &request.id, move |config, temperament, state| {
        if set_emotion(config, temperament, state, &emotion, level) {
            Ok(())
        } else {
            Err(EmotionError::UnknownEmotion { emotion })
        }
    })
    .await;
    if result.is_ok() {
        app.record_line(format!(
            "PERSONA state_set id={} emotion={} level={}",
            request.id, request.emotion, request.level
        ));
    }
    answer(app, "state-set", Some(&request.id), result)
}

async fn rapport(State(routes): State<Arc<EmotionRoutes>>, bytes: Bytes) -> Response {
    let app = &routes.app;
    let request: SetRapportRequest = match parse_body(&bytes) {
        Ok(request) => request,
        Err(error) => return answer::<StateView>(app, "state-rapport", None, Err(error.into())),
    };
    let (toward, rapport) = (request.toward.clone(), request.rapport);
    let result = change_stored(app, &request.id, move |config, _, state| {
        if set_rapport(config, state, &toward, rapport) {
            Ok(())
        } else {
            Err(EmotionError::InvalidStanding { toward })
        }
    })
    .await;
    if result.is_ok() {
        app.record_line(format!(
            "PERSONA state_rapport id={} toward={} rapport={}",
            request.id, request.toward, request.rapport
        ));
    }
    answer(app, "state-rapport", Some(&request.id), result)
}

async fn knows(State(routes): State<Arc<EmotionRoutes>>, bytes: Bytes) -> Response {
    let app = &routes.app;
    let request: SetKnownRequest = match parse_body(&bytes) {
        Ok(request) => request,
        Err(error) => return answer::<StateView>(app, "state-knows", None, Err(error.into())),
    };
    let (toward, known) = (request.toward.clone(), request.known);
    let result = change_stored(app, &request.id, move |_, _, state| {
        if set_known_name(state, &toward, known) {
            Ok(())
        } else {
            Err(EmotionError::InvalidStanding { toward })
        }
    })
    .await;
    if result.is_ok() {
        app.record_line(format!(
            "PERSONA state_knows id={} toward={} known={}",
            request.id, request.toward, request.known
        ));
    }
    answer(app, "state-knows", Some(&request.id), result)
}

/// Changes the stored state of the persona `id` by `change` under the write lock, with the emotion
/// list and the persona's temperament read now, and answers the new state's view.
async fn change_stored(
    app: &AppState,
    id: &str,
    change: impl FnOnce(&EmotionConfig, &Temperament, &mut PersonaState) -> Result<(), EmotionError>
    + Send
    + 'static,
) -> Result<StateView, EmotionError> {
    let persona = persona_id(id)?;
    let personas = persona_directory(app);
    let states = state_directory(app);
    let emotions = emotions_path(app);
    run_blocking(
        move || {
            let temperament = read_persona(&personas, &persona)?
                .ok_or_else(|| PersonaError::UnknownPersona {
                    id: persona.to_string(),
                })?
                .temperament()
                .clone();
            let config = read_emotions(&emotions)?;
            change_state(&states, persona.as_str(), &config, &temperament, |state| {
                change(&config, &temperament, state)
            })?;
            Ok(())
        },
        EmotionError::Stopped,
    )
    .await?;
    view_of(app, id).await
}

/// Asks the model for only the temperament and the emotional triggers of `own`, and gives the
/// edit that sets them; nothing else of the persona changes.
async fn temperament_edit(
    app: &AppState,
    route: &TextRoute,
    own: &Persona,
    config: &EmotionConfig,
) -> Result<PersonaEdit, EmotionError> {
    let request = text_request(
        route.profile(),
        TEMPERAMENT_INSTRUCTION,
        temperament_facts(own, config),
        TEMPERAMENT_OUTPUT_NAME,
        temperament_schema(config),
    );
    let key = operation_key("temperament")?;
    let started = Instant::now();
    let edit = route
        .ask(&key, &request, |answer| {
            read_temperament(answer, own, config)
        })
        .await
        .map_err(PersonaError::from)?
        .value;
    app.record_line(format!(
        "PERSONA temperament_generated id={} duration_ms={}",
        own.id(),
        started.elapsed().as_millis()
    ));
    Ok(edit)
}

async fn temperament(State(routes): State<Arc<EmotionRoutes>>, bytes: Bytes) -> Response {
    let request: TemperamentRequest = match parse_body(&bytes) {
        Ok(request) => request,
        Err(error) => {
            return answer::<Edited>(&routes.app, "temperament", None, Err(error.into()));
        }
    };
    let known = persona_id(&request.id).ok().map(|id| id.to_string());
    let owned = Arc::clone(&routes);
    let result = run_owned(
        async move {
            let route = persona_route(&owned.text, request.model.as_deref())?;
            let own = load_persona(&owned.app, &request.id).await?;
            let config = load_emotions(&owned.app)?;
            let edit = temperament_edit(&owned.app, &route, &own, &config).await?;
            Ok(apply_edits(&owned.app, &request.id, vec![edit]).await?)
        },
        EmotionError::Stopped,
    )
    .await;
    answer(&routes.app, "temperament", known.as_deref(), result)
}

/// The emotion routes, asking the text service of `text`.
pub(super) fn router(app: Arc<AppState>, text: Arc<TextRuntime>) -> Router {
    Router::new()
        .route(path::STATE, get(show))
        .route(path::STATE_RESET, post(reset))
        .route(path::STATE_SET, post(set))
        .route(path::STATE_RAPPORT, post(rapport))
        .route(path::STATE_KNOWS, post(knows))
        .route(path::TEMPERAMENT, post(temperament))
        .with_state(Arc::new(EmotionRoutes { app, text }))
}

//! The perception routes and what they do: one character looks at the picture and tells what they
//! see and think (`look`), a list of characters each look at the same picture (`perceive_scene`),
//! and the thoughts a character kept are read back.
//!
//! Each route answers a `PerceiveReply`. A refusal is one `PERSONA refused verb=perceive|
//! perceive_scene|thoughts [id=..] error=<key> cause=<key>` audit line; an answer is one `PERSONA
//! perceived who=.. vision=.. regions=.. attention=.. sees=.. feeling=.. viewpoint=..
//! picture=.. leak_suspect=.. retried=.. thought=.. duration_ms=..` line, and a scene call one
//! `PERSONA perceived_scene ..` line of counts. The lines hold ids and numbers, never a thought,
//! a sight or a line of the scene, which are words.
//!
//! Every character's call is its own: its facts are built from its own persona, state and
//! knowledge (`facts`), its answer is checked against its own lists and nothing of it reaches
//! another character's call. Every paid sequence runs on a task of its own that the handler waits
//! for, so a client that leaves cannot lose an answer already paid for (`blocking::run_owned`).

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use df_ai::text::TextImage;
use df_content::persona::{Appraisal, EmotionDelta, PersonaId};
use futures_util::future::join_all;
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::time::Instant;

use super::super::generate::{load_persona, personas_named, text_request};
use super::super::judge_route::{ask_request, judge_route};
use super::super::line::emotion_config;
use super::super::state::apply_appraisal;
use super::answer::{Dropped, OUTPUT_NAME, Proposed, instruction, read_answer, schema};
use super::facts::{Sight, briefing};
use super::request::{
    Attended, Mode, NewThing, PerceiveCause, PerceiveRequest, PerceiveRouteError,
    PerceiveSceneRequest, Perceived, SceneOutcome, ScenePerceived, SceneRow, Seen, Thoughts,
    ThoughtsQuery, Viewpoint, looker_id,
};
use super::screen::{Screen, read_screen};
use super::thoughts::{THOUGHT_KIND, keep_thought, thought_of};
use crate::blocking::run_owned;
use crate::dev_http::{parse_body, reply};
use crate::state::AppState;
use crate::text::{TextRoute, TextRuntime};

/// The addresses of the perception dev API.
pub mod path {
    /// What one character sees and thinks (POST a `PerceiveRequest`).
    pub const PERSONA_PERCEIVE: &str = "/dev/persona/perceive";
    /// What each of several characters sees and thinks of the same picture (POST a
    /// `PerceiveSceneRequest`).
    pub const PERSONA_PERCEIVE_SCENE: &str = "/dev/persona/perceive/scene";
    /// The newest thoughts of a character (GET with `?who=&limit=`).
    pub const PERSONA_THOUGHTS: &str = "/dev/persona/thoughts";
}

/// How many characters a scene call asks at once when it does not say.
pub const DEFAULT_PARALLEL: usize = 4;
/// The most characters a scene call asks at once.
pub const MAXIMUM_PARALLEL: usize = 8;
/// The most characters one scene call takes.
pub const MAXIMUM_CHARACTERS: usize = 64;
/// The thoughts `thoughts` answers when it is given no limit.
pub const DEFAULT_THOUGHTS: usize = 10;
/// The most thoughts one `thoughts` call answers.
pub const MAXIMUM_THOUGHTS: usize = 1_000;
/// What the picture is called to the model.
const PICTURE_LABEL: &str = "The screen now";

/// What every perception route works with: the host and the text surface the questions go to.
struct PerceiveRoutes {
    app: Arc<AppState>,
    text: Arc<TextRuntime>,
}

fn bad(cause: impl Into<PerceiveCause>) -> PerceiveRouteError {
    PerceiveRouteError::BadRequest {
        cause: cause.into(),
    }
}

/// What the characters of one call share.
struct Common<'a> {
    mode: Mode,
    recent: &'a [String],
    scene: Option<&'a str>,
    route: &'a TextRoute,
}

/// `who` as it can go into an audit line: a persona id, else `-`.
fn id_for_line(who: &str) -> &str {
    if PersonaId::new(who).is_ok() {
        who
    } else {
        "-"
    }
}

/// Records a refusal of `verb` (about `who`, when it is one character) as `PERSONA refused
/// verb=.. [id=..] error=<key> cause=<key>`.
fn record_refused(app: &AppState, verb: &str, who: Option<&str>, error: &PerceiveRouteError) {
    let cause = match error {
        PerceiveRouteError::BadRequest { cause }
        | PerceiveRouteError::AnswerRefused { cause, .. } => cause.key(),
        _ => "-",
    };
    let id = who.map_or_else(String::new, |who| format!(" id={}", id_for_line(who)));
    app.record_line(format!(
        "PERSONA refused verb={verb}{id} error={} cause={cause}",
        error.key()
    ));
}

/// Adds the things the answer proposed to the active place, so the next close look can cut to
/// them, and names each as the `item` of the `sees` entry it came from (and as the attention, when
/// it is attended to). A place that cannot take them (none active, or one of the bible's) is one
/// `PERSONA thing_unadded` line, and the proposals are reported as not added. A proposal the
/// answer check left out is reported the same way, one `PERSONA thing_unadded who=.. error=<key>`
/// line each; it never costs the rest of the answer.
async fn add_new_things(
    app: &AppState,
    who: &str,
    sees: &mut [Seen],
    attention: &mut Attended,
    (proposed, dropped): (Vec<Proposed>, Vec<Dropped>),
) -> Vec<NewThing> {
    let mut reported: Vec<NewThing> = Vec::new();
    for left_out in dropped {
        app.record_line(format!(
            "PERSONA thing_unadded who={who} error={}",
            left_out.cause.key()
        ));
        reported.push(NewThing {
            id: left_out.id,
            name: left_out.name,
            look: left_out.look,
            added: false,
            why_not: Some(left_out.cause.to_string()),
        });
    }
    if proposed.is_empty() {
        return reported;
    }
    let things = proposed.iter().map(|one| one.thing.clone()).collect();
    let added = crate::world::add_things(app, "perceive", things).await;
    if let Err(error) = &added {
        app.record_line(format!(
            "PERSONA thing_unadded who={who} error={}",
            error.key()
        ));
    }
    let why_not = added.as_ref().err().map(ToString::to_string);
    let added = added.is_ok();
    reported.extend(proposed.into_iter().map(|one| {
        if added {
            sees[one.see].item = Some(one.thing.id.clone());
            if one.attended {
                *attention = Attended::Item {
                    id: one.thing.id.clone(),
                };
            }
        } else {
            // The model may have named it already; it is no item of the place.
            sees[one.see].item = None;
            if *attention
                == (Attended::Item {
                    id: one.thing.id.clone(),
                })
            {
                *attention = Attended::None;
            }
        }
        NewThing {
            id: one.thing.id,
            name: one.thing.name,
            look: one.thing.look,
            added,
            why_not: why_not.clone(),
        }
    }));
    reported
}

/// What `who` sees of `screen` and thinks of it, asked of `common.route`. The picture is attached
/// when the screen has its bytes. An answer a check refuses is asked once more with the reason.
async fn look(
    app: &Arc<AppState>,
    who: &str,
    focus: Option<&str>,
    common: &Common<'_>,
    screen: &Screen,
) -> Result<Perceived, PerceiveRouteError> {
    let started = Instant::now();
    let id = looker_id(who).map_err(bad)?;
    let persona = load_persona(app, id.as_str()).await.map_err(bad)?;
    let config = emotion_config(app);
    let sight = Sight {
        mode: common.mode,
        focus,
        recent: common.recent,
        scene: common.scene,
    };
    let briefed = briefing(app, &persona, screen, &sight).await.map_err(bad)?;
    let offered = briefed.offered;
    let schema = schema(&offered, &config);
    let instruction = instruction();
    let mut facts = briefed.facts;
    let mut retried = false;
    let checked = loop {
        let mut question = text_request(
            common.route.profile(),
            &instruction,
            facts.clone(),
            OUTPUT_NAME,
            schema.clone(),
        );
        if let Some((media_type, bytes)) = &screen.image {
            question.images = vec![TextImage {
                label: PICTURE_LABEL.to_owned(),
                media_type: *media_type,
                bytes: bytes.clone(),
            }];
        }
        let read = |answer: &Value| read_answer(answer, &offered, &config);
        match ask_request(common.route, "perceive", &question, read).await {
            Err(PerceiveRouteError::AnswerRefused { cause, .. }) if !retried => {
                retried = true;
                facts["now"]["retry_reason"] = json!(cause.to_string());
            }
            asked => break asked?,
        }
    };
    // The feeling goes into the state only through the state path every line takes.
    if let Some(feeling) = &checked.feeling {
        apply_appraisal(
            app,
            &persona,
            &Appraisal {
                emotions: vec![EmotionDelta {
                    emotion: feeling.emotion.clone(),
                    delta: feeling.delta,
                }],
                ..Appraisal::default()
            },
        );
    }
    let mut sees = checked.sees;
    let mut attention = checked.attention;
    let new_things = add_new_things(
        app,
        id.as_str(),
        &mut sees,
        &mut attention,
        (checked.new_things, checked.dropped),
    )
    .await;
    let mut perceived = Perceived {
        who: id.to_string(),
        mode: common.mode,
        picture: screen.picture.clone(),
        vision: screen.image.is_some(),
        viewpoint: offered.viewpoint.clone(),
        regions: offered.regions.len(),
        sees,
        new_things,
        thinking_about: checked.thinking_about,
        attention,
        feeling: checked.feeling,
        wants: checked.wants,
        leak_suspect: checked.leak_suspect,
        thought: None,
        duration_ms: 0,
    };
    perceived.thought = keep_thought(app, &perceived, offered.place.as_ref()).await;
    perceived.duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    app.record_line(format!(
        concat!(
            "PERSONA perceived who={} vision={} regions={} attention={} sees={} new_things={} ",
            "feeling={} viewpoint={} picture={} leak_suspect={} retried={} thought={} ",
            "duration_ms={}"
        ),
        perceived.who,
        perceived.vision,
        perceived.regions,
        perceived.attention.label(),
        perceived.sees.len(),
        perceived
            .new_things
            .iter()
            .filter(|thing| thing.added)
            .count(),
        perceived.feeling.as_ref().map_or_else(
            || "-".to_owned(),
            |feeling| format!("{}:{:+}", feeling.emotion, feeling.delta)
        ),
        match perceived.viewpoint {
            Viewpoint::Pictured { .. } => "pictured",
            Viewpoint::Viewer => "viewer",
        },
        perceived.picture.as_deref().unwrap_or("-"),
        perceived.leak_suspect,
        retried,
        perceived.thought.as_deref().unwrap_or("-"),
        perceived.duration_ms
    ));
    Ok(perceived)
}

async fn perceive_one(
    routes: &PerceiveRoutes,
    request: &PerceiveRequest,
) -> Result<Perceived, PerceiveRouteError> {
    let route = judge_route(&routes.text, request.model.as_deref())?;
    let screen = read_screen(&routes.app, request.picture.as_deref(), request.vision)
        .await
        .map_err(bad)?;
    let common = Common {
        mode: request.mode,
        recent: &request.recent,
        scene: request.scene.as_deref(),
        route: &route,
    };
    look(
        &routes.app,
        &request.who,
        request.focus.as_deref(),
        &common,
        &screen,
    )
    .await
}

/// The characters of the picture's cast who have a persona, in the scan's order.
async fn pictured_with_personas(
    app: &AppState,
    screen: &Screen,
) -> Result<Vec<String>, PerceiveRouteError> {
    let names: Vec<String> = screen
        .layout
        .as_ref()
        .map(|layout| layout.names().into_iter().map(str::to_owned).collect())
        .unwrap_or_default();
    let personas = personas_named(app, names).await.map_err(bad)?;
    Ok(personas
        .iter()
        .map(|persona| persona.id().to_string())
        .collect())
}

/// What looking did for `name`, as a row of a scene call; a refusal is the row's and is recorded.
async fn scene_row(
    app: &Arc<AppState>,
    name: &str,
    common: &Common<'_>,
    screen: &Screen,
) -> SceneRow {
    let outcome = match look(app, name, None, common, screen).await {
        Ok(perceived) => SceneOutcome::Perceived {
            perceived: Box::new(perceived),
        },
        Err(error) => {
            record_refused(app, "perceive", Some(name), &error);
            SceneOutcome::Refused { error }
        }
    };
    SceneRow {
        who: name.to_owned(),
        outcome,
    }
}

async fn perceive_scene(
    routes: &PerceiveRoutes,
    request: &PerceiveSceneRequest,
) -> Result<ScenePerceived, PerceiveRouteError> {
    let started = Instant::now();
    let parallel = request.parallel.unwrap_or(DEFAULT_PARALLEL);
    if !(1..=MAXIMUM_PARALLEL).contains(&parallel) {
        return Err(bad(PerceiveCause::InvalidParallel {
            maximum: MAXIMUM_PARALLEL,
        }));
    }
    let route = judge_route(&routes.text, request.model.as_deref())?;
    let screen = read_screen(&routes.app, request.picture.as_deref(), request.vision)
        .await
        .map_err(bad)?;
    let mut who: Vec<String> = Vec::new();
    let named = if request.who.is_empty() {
        pictured_with_personas(&routes.app, &screen).await?
    } else {
        request.who.clone()
    };
    for name in named {
        if !who.contains(&name) {
            who.push(name);
        }
    }
    if who.is_empty() {
        return Err(bad(PerceiveCause::NoCharacters));
    }
    if who.len() > MAXIMUM_CHARACTERS {
        return Err(bad(PerceiveCause::TooManyCharacters {
            maximum: MAXIMUM_CHARACTERS,
        }));
    }
    let common = Common {
        mode: request.mode,
        recent: &request.recent,
        scene: request.scene.as_deref(),
        route: &route,
    };
    // At most `parallel` characters are asked at once; the rows keep the order of `who`.
    let room = Semaphore::new(parallel);
    let (app, room, common_ref, screen_ref) = (&routes.app, &room, &common, &screen);
    let results: Vec<SceneRow> = join_all(who.iter().map(|name| async move {
        let _turn = room.acquire().await;
        scene_row(app, name, common_ref, screen_ref).await
    }))
    .await;
    let perceived = results
        .iter()
        .filter(|row| matches!(row.outcome, SceneOutcome::Perceived { .. }))
        .count();
    routes.app.record_line(format!(
        concat!(
            "PERSONA perceived_scene characters={} perceived={} refused={} ",
            "parallel={} vision={} picture={} duration_ms={}"
        ),
        results.len(),
        perceived,
        results.len() - perceived,
        parallel,
        screen.image.is_some(),
        screen.picture.as_deref().unwrap_or("-"),
        started.elapsed().as_millis()
    ));
    Ok(ScenePerceived {
        picture: screen.picture,
        vision: screen.image.is_some(),
        results,
    })
}

async fn recall(
    routes: &PerceiveRoutes,
    query: &ThoughtsQuery,
) -> Result<Thoughts, PerceiveRouteError> {
    let id = looker_id(&query.who).map_err(bad)?;
    let limit = query.limit.unwrap_or(DEFAULT_THOUGHTS);
    if limit == 0 || limit > MAXIMUM_THOUGHTS {
        return Err(bad(PerceiveCause::InvalidLimit {
            maximum: MAXIMUM_THOUGHTS,
        }));
    }
    let memories = crate::world::memory::recent(&routes.app, id.as_str(), THOUGHT_KIND, limit)
        .await
        .map_err(|error| {
            bad(PerceiveCause::Thoughts {
                error: error.key().to_owned(),
            })
        })?;
    Ok(Thoughts {
        who: id.to_string(),
        thoughts: memories.into_iter().map(thought_of).collect(),
    })
}

async fn perceive(State(routes): State<Arc<PerceiveRoutes>>, bytes: Bytes) -> Response {
    let (who, result) = match parse_body::<PerceiveRequest>(&bytes) {
        Ok(request) => {
            let owned = Arc::clone(&routes);
            let who = request.who.clone();
            let result = run_owned(
                async move { perceive_one(&owned, &request).await },
                PerceiveRouteError::Stopped,
            )
            .await;
            (Some(who), result)
        }
        Err(error) => (None, Err(PerceiveRouteError::InvalidBody(error))),
    };
    if let Err(error) = &result {
        record_refused(&routes.app, "perceive", who.as_deref(), error);
    }
    reply::<_, PerceiveRouteError>(result)
}

async fn scene(State(routes): State<Arc<PerceiveRoutes>>, bytes: Bytes) -> Response {
    let result = match parse_body::<PerceiveSceneRequest>(&bytes) {
        Ok(request) => {
            let owned = Arc::clone(&routes);
            run_owned(
                async move { perceive_scene(&owned, &request).await },
                PerceiveRouteError::Stopped,
            )
            .await
        }
        Err(error) => Err(PerceiveRouteError::InvalidBody(error)),
    };
    if let Err(error) = &result {
        record_refused(&routes.app, "perceive_scene", None, error);
    }
    reply::<_, PerceiveRouteError>(result)
}

async fn thoughts(
    State(routes): State<Arc<PerceiveRoutes>>,
    query: Result<Query<ThoughtsQuery>, QueryRejection>,
) -> Response {
    let (who, result) = match query {
        Ok(Query(query)) => {
            let who = query.who.clone();
            (Some(who), recall(&routes, &query).await)
        }
        Err(rejection) => (
            None,
            Err(bad(PerceiveCause::Malformed {
                detail: rejection.body_text(),
            })),
        ),
    };
    if let Err(error) = &result {
        record_refused(&routes.app, "thoughts", who.as_deref(), error);
    }
    reply::<_, PerceiveRouteError>(result)
}

/// The perception routes, asking the text service of `text`.
pub(crate) fn router_with(app: Arc<AppState>, text: Arc<TextRuntime>) -> Router {
    Router::new()
        .route(path::PERSONA_PERCEIVE, post(perceive))
        .route(path::PERSONA_PERCEIVE_SCENE, post(scene))
        .route(path::PERSONA_THOUGHTS, get(thoughts))
        .with_state(Arc::new(PerceiveRoutes { app, text }))
}

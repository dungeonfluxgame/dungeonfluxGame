//! The movement routes: each is one model call against a closed schema. `POST /dev/move/interpret`
//! has Jev read one line a player said and answers a `DevReply` of an `Interpreted` reading, or a
//! typed [`InterpretRouteError`] with one `MOVE refused verb=interpret error=<key>` audit line. A
//! reading is one `MOVE interpreted reading=<key> unclear=<key|-> declared=<key>
//! second_reading=<bool> duration_ms=.. back_to=<id|->` line. `POST /dev/move/design`
//! designs the scene a move arrives in, residents included, and answers a `DevReply` of a
//! `SceneDesign` or a [`DesignRouteError`], with `MOVE designed ..` and `MOVE refused verb=design`
//! lines. A refused answer to the design call is asked for again, with every rule broken so far as
//! `refused_before` and one `MOVE retry verb=design attempt=<n> reason=<key> detail=<rule>` line
//! each time, until an answer stands or the design stage's deadline
//! ([`Deadlines::designing_ms`]) has passed; the refusal that ends it names its typed cause on its
//! `MOVE refused` line. A line never reaches an audit
//! line: it is data for the model.
//!
//! The question runs under the persona text scope and the default persona model, as the persona
//! questions do (`persona::generate`). The call runs on a task of its own that the handler waits
//! for, so a client that leaves cannot lose an answer already paid for.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::post;
use df_ai::text_attempts::OperationKey;
use df_content::world_bible::is_valid_id;
use serde::{Deserialize, Serialize};

use super::design::{
    DESIGN_INSTRUCTION, DesignError, MoverInventories, WalkNeeds, check_approach, check_coins,
    check_feature, check_move, check_shown_items, check_walk, design_facts, design_schema,
    parse_design_among, settle_coins, with_approach,
};
use super::interpret::{
    InterpretError, JEV_INSTRUCTION, jev_facts, jev_schema, parse_reading, require_text,
};
use super::transition::Deadlines;
use super::types::{
    ApproachAsk, Clarification, Declared, Distance, Interpreted, MoveIntent, PartyMember,
    SceneDesign, TownRole, WalkAsk,
};
use crate::blocking::run_owned;
use crate::dev_http::{BodyError, DevReply, RouteError, parse_body, reply};
use crate::persona::generate::{operation_key, text_request};
use crate::state::AppState;
use crate::text::{
    ModelCause, ServiceReason, TextAskError, TextRoute, TextRuntime, TextServiceCause,
};
use crate::world::{Town, Visited, end_town, seed_town};

/// The addresses of the movement dev API.
pub mod path {
    /// Reads one line as a move (POST an `InterpretRequest`). A dev tool; it asks the text service.
    pub const MOVE_INTERPRET: &str = "/dev/move/interpret";
    /// Designs the scene a move arrives in (POST a `DesignRequest`). A dev tool; it asks the text
    /// service.
    pub const MOVE_DESIGN: &str = "/dev/move/design";
    /// POST a `BackRequest`: a move back to a place the party has been to makes it the active one
    /// again, with no design and no plate.
    pub const MOVE_BACK: &str = "/dev/move/back";
    /// POST an `ApproachNote`: one moment of a step toward something on screen, as one `MOVE
    /// approach` audit line.
    pub const MOVE_APPROACH: &str = "/dev/move/approach";
}

/// The name of the answer object of a reading.
const OUTPUT_NAME: &str = "movement_reading";
/// The name of the answer object of a design.
const DESIGN_OUTPUT_NAME: &str = "scene_design";

/// The body of `POST /dev/move/interpret`: the line, the speaker's persona id when a character
/// said it, how the caller says the speaker meant it, the party that may move, and the model to
/// ask instead of the persona default. `refused_before` is Jev's own earlier clarification of this
/// line, sent on a second reading. The line is not derived for debugging, so it has no `Debug`.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterpretRequest {
    pub line: String,
    #[serde(default)]
    pub speaker: Option<String>,
    /// What the scene is and who has been talking; Jev reads "we" and "us" against it.
    #[serde(default)]
    pub context: Option<String>,
    /// `move` when the speaker said the line toward going somewhere; the default is `unknown`.
    #[serde(default)]
    pub declared: Declared,
    #[serde(default)]
    pub refused_before: Option<Clarification>,
    pub party: Vec<PartyMember>,
    #[serde(default)]
    pub model: Option<String>,
}

/// The answer of the movement route: its reading, or why it was refused.
pub type InterpretReply = DevReply<Interpreted, InterpretRouteError>;

/// Why a reading was refused. `key()` is the serde tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum InterpretRouteError {
    /// The request body is not the JSON the route reads.
    InvalidBody(BodyError),
    /// The line holds no words.
    EmptyLine,
    /// The text service is not set up; `missing` says what to set.
    TextServiceMissing { missing: String },
    /// The model name is not usable.
    InvalidModel { cause: ModelCause },
    /// The text service gave no usable answer.
    TextService {
        cause: TextServiceCause,
        reason: Option<ServiceReason>,
    },
    /// The reader refused the model's answer (`replayed` when it was a replay).
    AnswerRefused {
        replayed: bool,
        cause: InterpretError,
    },
    /// The text-service operation key is not valid.
    InvalidOperationKey,
    /// The blocking task stopped without an answer.
    Stopped,
}

impl InterpretRouteError {
    /// The error's key for the audit line: its serde tag.
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::EmptyLine => "empty_line",
            Self::TextServiceMissing { .. } => "text_service_missing",
            Self::InvalidModel { .. } => "invalid_model",
            Self::TextService { .. } => "text_service",
            Self::AnswerRefused { .. } => "answer_refused",
            Self::InvalidOperationKey => "invalid_operation_key",
            Self::Stopped => "stopped",
        }
    }
}

impl InterpretRouteError {
    /// The typed cause of a refused answer, for the audit line.
    fn cause(&self) -> Option<String> {
        match self {
            Self::AnswerRefused { cause, .. } => Some(format!("cause={}", cause.key())),
            _ => None,
        }
    }
}

impl std::fmt::Display for InterpretRouteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBody(_) => formatter.write_str("the request body is not the JSON to read"),
            Self::EmptyLine => formatter.write_str("the line is empty"),
            Self::TextServiceMissing { missing } => {
                write!(formatter, "the text service is not set up: {missing}")
            }
            Self::InvalidModel { .. } => formatter.write_str("the model name is not usable"),
            Self::TextService { .. } => formatter.write_str("the text service gave no answer"),
            Self::AnswerRefused { cause, .. } => {
                write!(formatter, "the model's answer was refused: {cause}")
            }
            Self::InvalidOperationKey => formatter.write_str("the operation key is not valid"),
            Self::Stopped => formatter.write_str("the reading stopped without an answer"),
        }
    }
}

impl std::error::Error for InterpretRouteError {}

impl RouteError for InterpretRouteError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_) | Self::EmptyLine | Self::InvalidModel { .. } => {
                StatusCode::BAD_REQUEST
            }
            Self::TextServiceMissing { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::TextService { .. } | Self::AnswerRefused { .. } => StatusCode::BAD_GATEWAY,
            Self::InvalidOperationKey | Self::Stopped => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<TextAskError<InterpretError>> for InterpretRouteError {
    fn from(error: TextAskError<InterpretError>) -> Self {
        match error {
            TextAskError::Service { cause, reason } => Self::TextService { cause, reason },
            TextAskError::AnswerRefused(cause) => Self::AnswerRefused {
                replayed: false,
                cause,
            },
            TextAskError::ReplayRefused(cause) => Self::AnswerRefused {
                replayed: true,
                cause,
            },
        }
    }
}

/// What the movement route works with: the host and the text surface the question goes to.
struct MoveRoutes {
    app: Arc<AppState>,
    text: Arc<TextRuntime>,
    /// How long the design call keeps asking again after refused answers.
    design_deadline: Duration,
}

/// Answers `result`, recording a refusal first as `MOVE refused verb=<verb> error=<key>`, with
/// `cause_of`'s words after it when the refusal has a typed cause (`cause=<key> detail=<rule>`,
/// never the model's text).
fn answer<T: Serialize, E: RouteError + Serialize>(
    app: &AppState,
    verb: &str,
    key_of: fn(&E) -> &'static str,
    cause_of: fn(&E) -> Option<String>,
    result: Result<T, E>,
) -> Response {
    if let Err(error) = &result {
        let cause = cause_of(error).map(|cause| format!(" {cause}"));
        app.record_line(format!(
            "MOVE refused verb={verb} error={}{}",
            key_of(error),
            cause.unwrap_or_default()
        ));
    }
    reply(result)
}

/// Why no route could be made for a question: the text service is not set up, or the model name is
/// not usable. Both route errors carry it.
enum RouteSetup {
    TextServiceMissing { missing: String },
    InvalidModel { cause: ModelCause },
}

impl From<RouteSetup> for InterpretRouteError {
    fn from(setup: RouteSetup) -> Self {
        match setup {
            RouteSetup::TextServiceMissing { missing } => Self::TextServiceMissing { missing },
            RouteSetup::InvalidModel { cause } => Self::InvalidModel { cause },
        }
    }
}

impl From<RouteSetup> for DesignRouteError {
    fn from(setup: RouteSetup) -> Self {
        match setup {
            RouteSetup::TextServiceMissing { missing } => Self::TextServiceMissing { missing },
            RouteSetup::InvalidModel { cause } => Self::InvalidModel { cause },
        }
    }
}

/// The route the question is asked of: the default persona model, or the one the request names.
fn route_for(text: &TextRuntime, model: Option<&str>) -> Result<Arc<TextRoute>, RouteSetup> {
    if let Some(missing) = text.missing() {
        return Err(RouteSetup::TextServiceMissing {
            missing: missing.to_owned(),
        });
    }
    text.persona_route(model)
        .map_err(|cause| RouteSetup::InvalidModel { cause })
}

/// The audit word of a reading.
fn reading_key(reading: &Interpreted) -> &'static str {
    match reading {
        Interpreted::Move { .. } => "move",
        Interpreted::NotAMove => "not_a_move",
        Interpreted::NeedsClarification(_) => "needs_clarification",
    }
}

/// The audit word of what a clarification leaves unclear; `-` for any other reading.
fn unclear_key(reading: &Interpreted) -> &'static str {
    match reading {
        Interpreted::NeedsClarification(clarification) => clarification.unclear.key(),
        Interpreted::Move { .. } | Interpreted::NotAMove => "-",
    }
}

/// Asks the model for the reading of `request.line` and checks its answer.
async fn interpret_line(
    routes: &MoveRoutes,
    request: &InterpretRequest,
) -> Result<Interpreted, InterpretRouteError> {
    require_text(&request.line).map_err(|_: InterpretError| InterpretRouteError::EmptyLine)?;
    let route = route_for(&routes.text, request.model.as_deref())?;
    let key = operation_key("interpret").map_err(|_| InterpretRouteError::InvalidOperationKey)?;
    // The places the party has been to, as the closed ids a move may go back to.
    let visited = crate::world::kept_places(&routes.app);
    let visited_ids: Vec<String> = visited.iter().map(|place| place.id.clone()).collect();
    let facts = jev_facts(
        &request.line,
        request.speaker.as_deref(),
        request.context.as_deref(),
        request.declared,
        request.refused_before.as_ref(),
        (&request.party, &visited),
    );
    let question = text_request(
        route.profile(),
        JEV_INSTRUCTION,
        facts,
        OUTPUT_NAME,
        jev_schema(&visited_ids),
    );
    let started = Instant::now();
    let reading = route
        .ask(&key, &question, |answer| {
            parse_reading(answer, &request.line, (&request.party, &visited_ids))
        })
        .await?
        .value;
    let back_to = match &reading {
        Interpreted::Move {
            back_to: Some(place),
            ..
        } => place.as_str(),
        _ => "-",
    };
    routes.app.record_line(format!(
        "MOVE interpreted reading={} unclear={} declared={} second_reading={} \
         duration_ms={} back_to={back_to}",
        reading_key(&reading),
        unclear_key(&reading),
        request.declared.key(),
        request.refused_before.is_some(),
        started.elapsed().as_millis()
    ));
    Ok(reading)
}

async fn interpret(State(routes): State<Arc<MoveRoutes>>, bytes: Bytes) -> Response {
    let request: InterpretRequest = match parse_body(&bytes) {
        Ok(request) => request,
        Err(error) => {
            return answer::<Interpreted, _>(
                &routes.app,
                "interpret",
                InterpretRouteError::key,
                InterpretRouteError::cause,
                Err(InterpretRouteError::InvalidBody(error)),
            );
        }
    };
    let owned = Arc::clone(&routes);
    let result = run_owned(
        async move { interpret_line(&owned, &request).await },
        InterpretRouteError::Stopped,
    )
    .await;
    answer(
        &routes.app,
        "interpret",
        InterpretRouteError::key,
        InterpretRouteError::cause,
        result,
    )
}

/// The body of `POST /dev/move/back`: the kept place a move goes back to, by id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackRequest {
    pub place: String,
}

/// `POST /dev/move/back`: a move whose reading goes back to a place the party has been to
/// (`Interpreted::Move::back_to`) makes that place the active one again as it was left, spending
/// no design and no plate. One `MOVE act back_to=<id> design=skipped` line; a place not kept is
/// `unknown_place`.
async fn back(State(routes): State<Arc<MoveRoutes>>, bytes: Bytes) -> Response {
    let result = async {
        let BackRequest { place } =
            parse_body(&bytes).map_err(crate::world::WorldError::InvalidBody)?;
        let app = Arc::clone(&routes.app);
        let id = place.clone();
        crate::blocking::run_blocking(
            move || crate::world::revisit_place(&app, &id),
            crate::world::WorldError::Stopped,
        )
        .await?
        .ok_or(crate::world::WorldError::UnknownPlace { id: place })
    }
    .await;
    match &result {
        Ok(kept) => routes
            .app
            .record_line(format!("MOVE act back_to={} design=skipped", kept.id)),
        Err(error) => routes
            .app
            .record_line(format!("MOVE refused verb=back error={}", error.key())),
    }
    reply(result)
}

/// What became of one step of an approach (`planning/explore.md`): reached (the target was within
/// reach, so the step is its close-up, no plate), made (its plate is in the library), shown (it
/// went up and its place is the active one), kept (made ahead and not taken: in the library,
/// never shown) or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApproachOutcome {
    Reached,
    Made,
    Shown,
    Kept,
    Failed,
}

impl ApproachOutcome {
    pub const ALL: [Self; 5] = [
        Self::Reached,
        Self::Made,
        Self::Shown,
        Self::Kept,
        Self::Failed,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Reached => "reached",
            Self::Made => "made",
            Self::Shown => "shown",
            Self::Kept => "kept",
            Self::Failed => "failed",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|outcome| outcome.key() == key)
    }
}

/// The body of `POST /dev/move/approach`: one moment of step `step` toward `target`, how far the
/// target was before it and is after it, the step's plate, whether it was made ahead, and the
/// milliseconds it took (the design and plate for `made`, the wait for `shown`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApproachNote {
    pub target: String,
    pub step: u32,
    pub before: Distance,
    #[serde(default)]
    pub after: Option<Distance>,
    #[serde(default)]
    pub plate: Option<String>,
    pub ahead: bool,
    pub outcome: ApproachOutcome,
    #[serde(default)]
    pub took_ms: Option<u64>,
}

/// `POST /dev/move/approach`: one `MOVE approach target= step= distance=<before>[-><after>]
/// plate= ahead= outcome= ms=` line on the host's clock; the answer is when it was recorded.
async fn approach_note(State(routes): State<Arc<MoveRoutes>>, bytes: Bytes) -> Response {
    let result = parse_body::<ApproachNote>(&bytes)
        .map_err(crate::world::WorldError::InvalidBody)
        .and_then(|note| {
            // The ids go into an audit line: each is a place id or the note is refused.
            let invalid = std::iter::once(&note.target)
                .chain(note.plate.as_ref())
                .find(|id| !is_valid_id(&id.to_ascii_lowercase()));
            match invalid {
                Some(id) => Err(crate::world::WorldError::InvalidWalkStage { id: id.clone() }),
                None => Ok(note),
            }
        })
        .map(|note| {
            routes.app.record_line(format!(
                "MOVE approach target={} step={} distance={}{} plate={} ahead={} outcome={} \
                 ms={}",
                note.target,
                note.step,
                note.before.key(),
                note.after
                    .map(|after| format!("->{}", after.key()))
                    .unwrap_or_default(),
                note.plate.as_deref().unwrap_or("-"),
                note.ahead,
                note.outcome.key(),
                note.took_ms
                    .map_or_else(|| "-".to_owned(), |took| took.to_string())
            ));
            crate::world::WalkNoted {
                at_ms: routes.app.playground().clock_ms(),
            }
        });
    if let Err(error) = &result {
        routes
            .app
            .record_line(format!("MOVE refused verb=approach error={}", error.key()));
    }
    reply(result)
}

/// The body of `POST /dev/move/design`: the move, the party that may move, what the scene was and
/// who has been talking, and the model to ask instead of the persona default. The move holds the
/// player's line, so it has no `Debug`.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignRequest {
    pub intent: MoveIntent,
    pub party: Vec<PartyMember>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// The sights wanted on the way and the move's role in its town, for a move that walks.
    #[serde(default)]
    pub walk: Option<WalkAsk>,
    /// Who the scenario says is at the destination: each must be one of the design's residents,
    /// with that id, so the same character is met there in every run (a guild master, a smith
    /// whose shop is theirs).
    #[serde(default)]
    pub residents: Vec<ExpectedResident>,
    /// The step of an approach this move is, when it is one: its design keeps the target in sight
    /// at a distance one step can leave it at (`check_approach`).
    #[serde(default)]
    pub approach: Option<ApproachAsk>,
}

/// Someone the scenario puts at a move's destination: their persona id and who they are, as the
/// place shows them (no name).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedResident {
    pub id: String,
    pub who: String,
}

/// The words the design is told of who is at the destination, before the move's own situation.
pub(super) fn residents_words(expected: &[ExpectedResident]) -> Option<String> {
    if expected.is_empty() {
        return None;
    }
    let people: Vec<String> = expected
        .iter()
        .map(|resident| format!("{} (resident id {})", resident.who.trim(), resident.id))
        .collect();
    Some(format!(
        "Who is at the destination, each a resident of the design with that id: {}.",
        people.join("; ")
    ))
}

/// Why `design` does not hold the residents the scenario expects there, or none: each expected id
/// is one of its residents.
pub(super) fn expected_miss(
    design: &SceneDesign,
    expected: &[ExpectedResident],
) -> Option<DesignError> {
    expected
        .iter()
        .find(|resident| {
            !design
                .residents
                .iter()
                .any(|placed| placed.id == resident.id)
        })
        .map(|missing| DesignError::Malformed {
            reason: format!(
                "the destination has {} as a resident with the id {}, and the design has no \
                 resident of that id",
                missing.who.trim(),
                missing.id
            ),
        })
}

/// The answer of the design route: the scene design, or why it was refused.
pub type DesignReply = DevReply<SceneDesign, DesignRouteError>;

/// Why a design was refused. `key()` is the serde tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum DesignRouteError {
    /// The request body is not the JSON the route reads.
    InvalidBody(BodyError),
    /// The move names nobody, a non-party mover or no destination.
    BadMove { cause: DesignError },
    /// The text service is not set up; `missing` says what to set.
    TextServiceMissing { missing: String },
    /// The model name is not usable.
    InvalidModel { cause: ModelCause },
    /// The text service gave no usable answer.
    TextService {
        cause: TextServiceCause,
        reason: Option<ServiceReason>,
    },
    /// The design check refused the model's answer (`replayed` when it was a replay).
    AnswerRefused { replayed: bool, cause: DesignError },
    /// The text-service operation key is not valid.
    InvalidOperationKey,
    /// The blocking task stopped without an answer.
    Stopped,
}

impl DesignRouteError {
    /// The error's key for the audit line: its serde tag.
    pub fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::BadMove { .. } => "bad_move",
            Self::TextServiceMissing { .. } => "text_service_missing",
            Self::InvalidModel { .. } => "invalid_model",
            Self::TextService { .. } => "text_service",
            Self::AnswerRefused { .. } => "answer_refused",
            Self::InvalidOperationKey => "invalid_operation_key",
            Self::Stopped => "stopped",
        }
    }
}

impl DesignRouteError {
    /// The typed cause of a refused move or answer, for the audit line: its key and the rule it
    /// broke, never the model's text.
    fn cause(&self) -> Option<String> {
        match self {
            Self::BadMove { cause } | Self::AnswerRefused { cause, .. } => {
                Some(format!("cause={} detail={cause}", cause.key()))
            }
            _ => None,
        }
    }
}

impl std::fmt::Display for DesignRouteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBody(_) => formatter.write_str("the request body is not the JSON to read"),
            Self::BadMove { cause } => write!(formatter, "the move cannot be designed: {cause}"),
            Self::TextServiceMissing { missing } => {
                write!(formatter, "the text service is not set up: {missing}")
            }
            Self::InvalidModel { .. } => formatter.write_str("the model name is not usable"),
            Self::TextService { .. } => formatter.write_str("the text service gave no answer"),
            Self::AnswerRefused { cause, .. } => {
                write!(formatter, "the model's design was refused: {cause}")
            }
            Self::InvalidOperationKey => formatter.write_str("the operation key is not valid"),
            Self::Stopped => formatter.write_str("the design stopped without an answer"),
        }
    }
}

impl std::error::Error for DesignRouteError {}

impl RouteError for DesignRouteError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_) | Self::BadMove { .. } | Self::InvalidModel { .. } => {
                StatusCode::BAD_REQUEST
            }
            Self::TextServiceMissing { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::TextService { .. } | Self::AnswerRefused { .. } => StatusCode::BAD_GATEWAY,
            Self::InvalidOperationKey | Self::Stopped => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<TextAskError<DesignError>> for DesignRouteError {
    fn from(error: TextAskError<DesignError>) -> Self {
        match error {
            TextAskError::Service { cause, reason } => Self::TextService { cause, reason },
            TextAskError::AnswerRefused(cause) => Self::AnswerRefused {
                replayed: false,
                cause,
            },
            TextAskError::ReplayRefused(cause) => Self::AnswerRefused {
                replayed: true,
                cause,
            },
        }
    }
}

/// Asks the model to design the scene `request.intent` arrives in and checks its answer. A move
/// that walks is also asked for the sights on the way, and the town it is in is kept or ended as
/// its role says.
async fn design_scene(
    routes: &MoveRoutes,
    request: &DesignRequest,
) -> Result<SceneDesign, DesignRouteError> {
    check_move(&request.intent, &request.party)
        .map_err(|cause| DesignRouteError::BadMove { cause })?;
    if request
        .walk
        .as_ref()
        .is_some_and(|ask| !is_valid_id(&ask.move_id))
    {
        return Err(DesignRouteError::BadMove {
            cause: DesignError::Malformed {
                reason: "the walk's move id is not a plain id".to_owned(),
            },
        });
    }
    let route = route_for(&routes.text, request.model.as_deref())?;
    let key = operation_key("design").map_err(|_| DesignRouteError::InvalidOperationKey)?;
    let inventories: MoverInventories = request
        .intent
        .who
        .iter()
        .map(|who| (who.clone(), crate::world::inventory_of(&routes.app, who)))
        .filter(|(_, items)| !items.is_empty())
        .collect();
    let town = crate::world::town_of(&routes.app);
    let visited = crate::world::visited_of(&routes.app);
    // After a crossing into another region every place is designed in it: the design is told the
    // region's look before the move's own situation.
    // Who the scenario puts at the destination is told before the situation too.
    let context = [
        crate::world::region_of(&routes.app).map(|region| region.design_words()),
        residents_words(&request.residents),
        request.context.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let context = (!context.is_empty()).then(|| context.join(" "));
    let needs = request
        .walk
        .as_ref()
        .map(|ask| WalkNeeds::of(ask, town.as_ref()));
    let started = Instant::now();
    let deadline = routes.design_deadline;
    let met = people_met(&routes.app, request).await;
    let asked = Asked {
        request,
        context: context.as_deref(),
        inventories: &inventories,
        needs: needs.as_ref(),
        visited: &visited,
        met: &met,
    };
    // No count of askings is capped: a refused answer is shown to the model as every rule broken
    // so far and asked for again, so invalid answers do not lose the move, until the stage's
    // deadline has passed (a service failure ends it at once).
    let mut refused: Vec<DesignError> = Vec::new();
    let mut first_key = Some(key);
    let design = loop {
        let key = match first_key.take() {
            Some(key) => key,
            None => operation_key("design").map_err(|_| DesignRouteError::InvalidOperationKey)?,
        };
        match ask_design(&route, &key, &asked, &refused).await {
            Ok(design) => break design,
            Err(TextAskError::AnswerRefused(cause)) if started.elapsed() < deadline => {
                routes.app.record_line(format!(
                    "MOVE retry verb=design attempt={} reason={} detail={cause}",
                    refused.len() + 2,
                    cause.key()
                ));
                refused.push(cause);
            }
            Err(error) => return Err(error.into()),
        }
    };
    let retried = !refused.is_empty();
    let kept = keep_town(routes, request.walk.as_ref(), town, &design).await;
    routes.app.record_line(format!(
        "MOVE designed placements={} residents={} stages={} town={kept} feature={} \
         retried={retried} duration_ms={}",
        design.placements.len(),
        design.residents.len(),
        design.stages.len(),
        design.feature,
        started.elapsed().as_millis()
    ));
    Ok(design)
}

/// What the town of `ask`'s role came to once the design is accepted, as the `MOVE designed`
/// line says it: `seeded` (kept from the design's look), `reused`, `left` (ended), or `none`. A
/// town that cannot be kept or ended is `unwritten`, and the move goes on.
async fn keep_town(
    routes: &MoveRoutes,
    ask: Option<&WalkAsk>,
    town: Option<Town>,
    design: &SceneDesign,
) -> &'static str {
    let Some(ask) = ask else {
        return "none";
    };
    let (kept, written) = match (ask.town, town, &design.town_look) {
        (TownRole::Leaves, Some(_), _) => ("left", end_town(&routes.app).await.map(|_| ())),
        (TownRole::Seeds, None, Some(look)) => {
            let seeded = Town {
                id: format!("town-{}", ask.move_id),
                look: look.clone(),
                created_at: ask.move_id.clone(),
            };
            ("seeded", seed_town(&routes.app, seeded).await)
        }
        (_, Some(_), _) => ("reused", Ok(())),
        _ => ("none", Ok(())),
    };
    match written {
        Ok(()) => kept,
        Err(error) => {
            routes
                .app
                .record_line(format!("WORLD refused verb=town error={}", error.key()));
            "unwritten"
        }
    }
}

/// The ids of the stored personas, the people already met, whose ids a new resident never takes;
/// but those the move may meet again under their own ids: whom the scenario puts at the
/// destination, who is at the place now (a step on toward them keeps them) and what an approach
/// walks toward. Empty when they cannot be read.
async fn people_met(app: &Arc<AppState>, request: &DesignRequest) -> Vec<String> {
    let here = crate::seat_pages::listener_ids(app).await;
    crate::persona::stored_ids(app)
        .await
        .into_iter()
        .filter(|id| !request.residents.iter().any(|resident| &resident.id == id))
        .filter(|id| !here.contains(id))
        .filter(|id| {
            request
                .approach
                .as_ref()
                .is_none_or(|ask| &ask.target != id)
        })
        .collect()
}

/// What one asking of the design call is about: the move, what the movers hold and what the move
/// asks of the walk.
struct Asked<'a> {
    request: &'a DesignRequest,
    /// The move's situation as the design is told it: the region's words before the request's.
    context: Option<&'a str>,
    inventories: &'a MoverInventories,
    needs: Option<&'a WalkNeeds>,
    visited: &'a [Visited],
    /// The people already met (stored personas) but those the scenario puts at the destination:
    /// a new resident never takes one of their ids.
    met: &'a [String],
}

/// One asking of the design call: the move, what the movers hold and, on a later asking, every
/// refusal of the earlier answers; the answer is checked against the move, the party, the movers'
/// inventories and the walk the move asked for.
async fn ask_design(
    route: &TextRoute,
    key: &OperationKey,
    asked: &Asked<'_>,
    refused_before: &[DesignError],
) -> Result<SceneDesign, TextAskError<DesignError>> {
    let Asked {
        request,
        context,
        inventories,
        needs,
        visited,
        met,
    } = asked;
    let facts = design_facts(
        &request.intent,
        &request.party,
        *context,
        inventories,
        refused_before,
        *needs,
        visited,
    );
    let facts = match &request.approach {
        Some(ask) => with_approach(facts, ask),
        None => facts,
    };
    let question = text_request(
        route.profile(),
        DESIGN_INSTRUCTION,
        facts,
        DESIGN_OUTPUT_NAME,
        design_schema(*needs),
    );
    let answered = route
        .ask(key, &question, |answer| {
            let mut design = parse_design_among(answer, &request.intent, &request.party, met)?;
            check_shown_items(&design, inventories)?;
            check_coins(&design, inventories)?;
            settle_coins(&mut design, inventories);
            check_feature(&design, visited)?;
            if let Some(miss) = expected_miss(&design, &request.residents) {
                return Err(miss);
            }
            check_walk(&design, *needs)?;
            if let Some(ask) = &request.approach {
                check_approach(&design, ask)?;
            }
            if let Some(needs) = needs {
                needs.settle(&mut design);
            }
            Ok(design)
        })
        .await?;
    Ok(answered.value)
}

async fn design(State(routes): State<Arc<MoveRoutes>>, bytes: Bytes) -> Response {
    let request: DesignRequest = match parse_body(&bytes) {
        Ok(request) => request,
        Err(error) => {
            return answer::<SceneDesign, _>(
                &routes.app,
                "design",
                DesignRouteError::key,
                DesignRouteError::cause,
                Err(DesignRouteError::InvalidBody(error)),
            );
        }
    };
    let owned = Arc::clone(&routes);
    let result = run_owned(
        async move { design_scene(&owned, &request).await },
        DesignRouteError::Stopped,
    )
    .await;
    answer(
        &routes.app,
        "design",
        DesignRouteError::key,
        DesignRouteError::cause,
        result,
    )
}

/// The movement routes, asking the text service of `text`.
// TODO(DEV-12): route tests on a fake text service: a reading, a design with a resident, a refused
// answer, an empty line, a bad move, a missing text service and the audit lines.
pub fn router_with(app: Arc<AppState>, text: Arc<TextRuntime>) -> Router {
    let designing = Deadlines::default().designing_ms;
    router_with_design_deadline(app, text, Duration::from_millis(designing))
}

/// [`router_with`] where the design call asks again after refused answers for `design_deadline`.
pub(crate) fn router_with_design_deadline(
    app: Arc<AppState>,
    text: Arc<TextRuntime>,
    design_deadline: Duration,
) -> Router {
    Router::new()
        .route(path::MOVE_INTERPRET, post(interpret))
        .route(path::MOVE_DESIGN, post(design))
        .route(path::MOVE_BACK, post(back))
        .route(path::MOVE_APPROACH, post(approach_note))
        .with_state(Arc::new(MoveRoutes {
            app,
            text,
            design_deadline,
        }))
}

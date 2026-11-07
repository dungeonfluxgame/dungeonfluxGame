//! The bodies and replies of the persona routes, and why they refuse.
//!
//! Every route answers a `PersonaReply<T>`: `{"outcome":"done","value":..}` or
//! `{"outcome":"rejected","error":{"error":"<key>",..},"message":".."}`.

use std::fmt;

use axum::http::StatusCode;
use df_content::persona::backfill::{BackfillError, Gap, NarrativeGuidance};
use df_content::persona::generation::GenerationError;
use df_content::persona::{
    EditError, Persona, PersonaEdit, PersonaError as ContentError, PersonaField,
};
use serde::{Deserialize, Serialize};

use crate::dev_http::{BodyError, DevReply, RouteError};
use crate::locate::IoCause;
use crate::playground::SceneApplyError;
use crate::shown_scene::ShownSceneError;
use crate::speech::Voiced;
use crate::text::{ModelCause, ServiceReason, TextAskError, TextServiceCause};

/// The answer of a persona route: its value, or why it was refused.
pub type PersonaReply<T> = DevReply<T, PersonaError>;

/// Whose view of a persona a `GET /dev/persona/{id}` answers. The dev API has no password and is
/// for the operator only, so `Owner` is not an access control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    /// `Persona::public_view`: what any listener may know. The default.
    #[default]
    Public,
    /// The whole persona, private drives and backstory included.
    Owner,
}

impl Audience {
    pub const ALL: [Self; 2] = [Self::Public, Self::Owner];

    pub fn key(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Owner => "owner",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|audience| audience.key() == key)
    }
}

/// The query of `GET /dev/persona/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowQuery {
    #[serde(default)]
    pub audience: Audience,
}

/// The body of `POST /dev/persona/remove`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveRequest {
    pub id: String,
}

/// A stored persona in a list: who, and what they do. Nothing private.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaSummary {
    pub id: String,
    pub display_name: String,
    pub job: String,
}

impl From<&Persona> for PersonaSummary {
    fn from(persona: &Persona) -> Self {
        Self {
            id: persona.id().as_str().to_owned(),
            display_name: persona.display_name().to_owned(),
            job: persona.job().to_owned(),
        }
    }
}

/// The body of `POST /dev/persona/generate`: write the persona of one named character of the shown
/// scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateRequest {
    pub character: String,
    /// What the persona should be like, when the caller has an idea. It is data for the model.
    #[serde(default)]
    pub hint: Option<String>,
    /// What the narrative engine wants the persona to serve.
    #[serde(default)]
    pub guidance: Option<NarrativeGuidance>,
    /// The model to ask instead of the persona default.
    #[serde(default)]
    pub model: Option<String>,
    /// Write a new persona over a stored one. Without it a stored persona is kept.
    #[serde(default)]
    pub overwrite: bool,
}

/// The body of `POST /dev/persona/generate-cast`: `GenerateRequest` for every named character of
/// the shown scene.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateCastRequest {
    #[serde(default)]
    pub hint: Option<String>,
    #[serde(default)]
    pub guidance: Option<NarrativeGuidance>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub overwrite: bool,
}

/// The body of `POST /dev/persona/say`: a line from a character in a situation. A dev tool; the
/// game feeds the same brief into the expression contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SayRequest {
    pub character: String,
    /// What is happening, as the character meets it. It is data for the model.
    pub situation: String,
    /// The model to ask instead of the persona default.
    #[serde(default)]
    pub model: Option<String>,
    /// Voice the line and play it with its bubble. Off unless it says otherwise.
    #[serde(default)]
    pub speak: bool,
}

fn yes() -> bool {
    true
}

/// The body of `POST /dev/persona/ask`: put a question to a character. The text service judges
/// whether the persona holds enough to answer, fills what it lacks (adding only), and the
/// character answers in character. A dev tool: the narrative engine calls the same steps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
// TODO(DEV-12): redact `question` in Debug, as the page's copy does.
pub struct AskRequest {
    pub character: String,
    pub question: String,
    #[serde(default)]
    pub guidance: Option<NarrativeGuidance>,
    #[serde(default)]
    pub model: Option<String>,
    /// Fill gaps before answering. On unless it says otherwise.
    #[serde(default = "yes")]
    pub backfill: bool,
    /// Judge the persona's coverage of the question first. On unless it says otherwise; off
    /// skips the coverage question and any backfill, so the turn is one model call.
    #[serde(default = "yes")]
    pub coverage: bool,
    /// Voice the line and play it with its bubble. Off unless it says otherwise.
    #[serde(default)]
    pub speak: bool,
    /// What is happening around the question; it is data for the model.
    #[serde(default)]
    pub situation: Option<String>,
}

/// The body of `POST /dev/persona/coverage`: does the persona hold enough to answer `question`?
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageRequest {
    pub character: String,
    pub question: String,
    #[serde(default)]
    pub guidance: Option<NarrativeGuidance>,
    #[serde(default)]
    pub model: Option<String>,
}

/// The body of `POST /dev/persona/backfill`: fill `gaps` of a persona, adding only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackfillRequest {
    pub character: String,
    pub gaps: Vec<Gap>,
    #[serde(default)]
    pub guidance: Option<NarrativeGuidance>,
    #[serde(default)]
    pub model: Option<String>,
}

/// The query of `GET /dev/persona/backfills`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackfillsQuery {
    pub id: String,
}

/// What the coverage question found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Covered {
    pub enough: bool,
    pub gaps: Vec<Gap>,
    /// The profile (backend and model) the question was asked under.
    pub profile: String,
}

/// A committed backfill: the gaps it filled and the edits that filled them, all additions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backfilled {
    pub gaps: Vec<Gap>,
    pub edits: Vec<PersonaEdit>,
    /// Its number in the persona's record of backfills.
    pub sequence: u64,
    /// The profile (backend and model) the backfill was asked under.
    pub profile: String,
}

/// What a backfill did for a question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum BackfillOutcome {
    /// The persona held enough.
    NotNeeded,
    /// The persona lacked something and the request said not to fill it.
    NotAsked,
    /// The coverage question failed, so what the persona lacks is unknown; the answer is from the
    /// persona as it was.
    CoverageFailed {
        error: PersonaError,
    },
    Backfilled {
        edits: usize,
        sequence: u64,
    },
    /// The backfill failed; the answer is from the persona as it was.
    BackfillFailed {
        error: PersonaError,
    },
    /// The request skipped the coverage question (`coverage: false`).
    CoverageSkipped,
}

/// A character's answer to a question, with what was filled in first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answered {
    pub covered: bool,
    pub gaps: Vec<Gap>,
    pub backfill: BackfillOutcome,
    /// The profile (backend and model) every question was asked under.
    pub profile: String,
    pub spoken: Spoken,
}

/// One committed backfill in a persona's `<id>.backfills.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackfillRecord {
    /// Counts up from 1 within the persona's record.
    pub sequence: u64,
    /// When it was committed, in milliseconds since the Unix epoch.
    pub at_ms: u64,
    /// The question that led to it; none for a direct backfill.
    pub question: Option<String>,
    pub gaps: Vec<Gap>,
    pub edits: Vec<PersonaEdit>,
    pub guidance: Option<NarrativeGuidance>,
    /// The profile (backend and model) that wrote it.
    pub profile: String,
}

/// What generating a persona did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Generation {
    /// A new persona was written and stored. `leak_suspect` is true when its public texts share a
    /// word with its private ones (a whole-word check, not a guarantee either way); nothing was
    /// refused.
    Generated {
        persona: PersonaSummary,
        leak_suspect: bool,
    },
    /// A persona was already stored and `overwrite` was not asked; no model was called.
    KeptExisting { persona: PersonaSummary },
}

/// What generating the persona of one character of a cast did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum CastOutcome {
    Generated {
        persona: PersonaSummary,
        leak_suspect: bool,
    },
    KeptExisting {
        persona: PersonaSummary,
    },
    Failed {
        error: PersonaError,
    },
}

impl From<Result<Generation, PersonaError>> for CastOutcome {
    fn from(result: Result<Generation, PersonaError>) -> Self {
        match result {
            Ok(Generation::Generated {
                persona,
                leak_suspect,
            }) => Self::Generated {
                persona,
                leak_suspect,
            },
            Ok(Generation::KeptExisting { persona }) => Self::KeptExisting { persona },
            Err(error) => Self::Failed { error },
        }
    }
}

/// One character of a cast and what generating its persona did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CastRow {
    pub character: String,
    pub outcome: CastOutcome,
}

/// The body of `POST /dev/persona/edit`: typed edits to the stored persona `id`, applied all
/// together or not at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditRequest {
    pub id: String,
    pub edits: Vec<PersonaEdit>,
}

/// The persona after an edit, whole (the dev API is for the operator), and whether the edits
/// changed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edited {
    pub persona: Persona,
    pub changed: bool,
}

/// A persona that was stored by `POST /dev/persona`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaStored {
    pub persona: PersonaSummary,
    /// Whether it replaced a stored persona of the same id (otherwise it created one).
    pub replaced: bool,
}

/// A line a character said, and the stance and slips the model reported with it. The model only
/// words and reports; the production engine, not the model, owns whether a secret is revealed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaidLine {
    pub line: String,
    /// The behavior id the character took, when the model says one.
    pub behavior: Option<String>,
    /// The trigger ids of this persona that the model judged fired.
    pub fired_triggers: Vec<String>,
    /// The secret ids of this persona that the line gave away.
    pub revealed_secrets: Vec<String>,
    /// True when the line has a whole word (4 letters or more) of the character's private side
    /// that the public persona, the question and the situation do not use. It blocks nothing:
    /// inflections and paraphrase pass the check, so it is a hint and not a guarantee.
    pub leak_suspect: bool,
}

/// A line applied to the live scene as a cue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spoken {
    pub character: String,
    pub said: SaidLine,
    pub revision: u64,
    /// What voicing the line came to; none when the request did not ask for a voice.
    #[serde(default)]
    pub voice: Option<Voiced>,
}

/// A persona that was removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaRemoved {
    pub id: String,
}

/// A line of an answer that is not a line of the persona it was asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum LineError {
    /// The answer is not the object the schema describes; `detail` is the reader's sentence.
    Malformed { detail: String },
    /// The line is blank.
    Blank,
    /// The answer names a behavior, trigger or secret the persona does not have.
    UnknownId { kind: LineIdKind, id: String },
}

/// What an id of a line's answer names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineIdKind {
    Behavior,
    Trigger,
    Secret,
}

impl LineIdKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Behavior => "behavior",
            Self::Trigger => "trigger",
            Self::Secret => "secret",
        }
    }
}

impl fmt::Display for LineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed { detail } => {
                write!(
                    formatter,
                    "the answer is not the line object asked for: {detail}"
                )
            }
            Self::Blank => formatter.write_str("the line is blank"),
            Self::UnknownId { kind, id } => write!(
                formatter,
                "the answer names the {} {id:?}, which they do not have",
                kind.key()
            ),
        }
    }
}

impl std::error::Error for LineError {}

/// The error of the reader that refused an answer of the text service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum AnswerError {
    Generation { error: GenerationError },
    Backfill { error: BackfillError },
    Line { error: LineError },
}

impl fmt::Display for AnswerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Generation { error } => write!(formatter, "{error}"),
            Self::Backfill { error } => write!(formatter, "{error}"),
            Self::Line { error } => write!(formatter, "{error}"),
        }
    }
}

impl From<GenerationError> for AnswerError {
    fn from(error: GenerationError) -> Self {
        Self::Generation { error }
    }
}

impl From<BackfillError> for AnswerError {
    fn from(error: BackfillError) -> Self {
        Self::Backfill { error }
    }
}

impl From<LineError> for AnswerError {
    fn from(error: LineError) -> Self {
        Self::Line { error }
    }
}

/// Why a persona request was refused. An error never carries persona text beyond the sentence of
/// a reader that refused a file or a body.
///
/// `key()` is the serde tag, except for the wrapper `Shown`, whose key is its cause's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum PersonaError {
    /// The request body is not the JSON the route reads, or its persona is not valid.
    InvalidBody(BodyError),
    /// The query is not one the route reads; `detail` is the reader's sentence.
    InvalidQuery { detail: String },
    /// A persona id is not valid.
    InvalidPersona { cause: ContentError },
    /// The id is a word the dev API or its command line uses (`remove`, `generate`, ...).
    ReservedId { id: String },
    /// The edits would leave the persona invalid, or name something it does not have.
    InvalidEdit { cause: EditError },
    /// The persona file given for `expected` holds the persona `found`. Found by the dev CLI
    /// before it sends anything.
    IdMismatch { expected: String, found: String },
    /// No persona is stored for `id`.
    UnknownPersona { id: String },
    /// The `personas` folder could not be listed.
    FolderUnreadable { cause: IoCause },
    /// The file of `id` could not be read.
    Unreadable { id: String, cause: IoCause },
    /// The file of `id` is not a valid persona; `detail` is the reader's sentence.
    StoredInvalid { id: String, detail: String },
    /// The file of `id` holds the persona `found`.
    StoredIdMismatch { id: String, found: String },
    /// The file of `id` could not be written or removed.
    Unwritable { id: String, cause: IoCause },
    /// The shown scene, its layout or the named character could not be had.
    Shown { cause: ShownSceneError },
    /// The text service is not set up; `missing` says what to set.
    TextServiceMissing { missing: String },
    /// The text service gave no usable answer.
    TextService {
        cause: TextServiceCause,
        reason: Option<ServiceReason>,
    },
    /// The reader refused the text service's answer (`replayed` when it was a replay).
    AnswerRefused { replayed: bool, cause: AnswerError },
    /// The text-service operation key is not valid.
    InvalidOperationKey,
    /// The model name is not usable.
    InvalidModel { cause: ModelCause },
    /// A gap names a field a backfill cannot fill.
    NotFillable { field: PersonaField },
    /// A backfill named no gap.
    NoGaps,
    /// A backfill has a word of the character's private side in a public field; nothing was
    /// written. The check is of whole words: inflections and paraphrase pass it, so it reduces
    /// accidents and is not a guarantee.
    #[serde(rename = "private_detail")]
    PrivateDetailInPublicField,
    /// The record of the backfills of `id` could not be read.
    BackfillsUnreadable { id: String, cause: IoCause },
    /// The record of the backfills of `id` is not valid and is left as it is; `detail` is the
    /// reader's sentence.
    BackfillsInvalid { id: String, detail: String },
    /// The record of the backfills of `id` could not be written. `persona_restored` says whether
    /// the persona file the backfill had already written was put back as it was.
    BackfillsUnwritable {
        id: String,
        cause: IoCause,
        persona_restored: bool,
    },
    /// The blocking task stopped without an answer.
    Stopped,
}

impl PersonaError {
    /// The error's key for the audit line: its serde tag, or for `Shown` its cause's key.
    pub(crate) fn key(&self) -> &'static str {
        match self {
            Self::InvalidBody(_) => "invalid_body",
            Self::InvalidQuery { .. } => "invalid_query",
            Self::InvalidPersona { .. } => "invalid_persona",
            Self::ReservedId { .. } => "reserved_id",
            Self::InvalidEdit { .. } => "invalid_edit",
            Self::IdMismatch { .. } => "id_mismatch",
            Self::UnknownPersona { .. } => "unknown_persona",
            Self::FolderUnreadable { .. } => "folder_unreadable",
            Self::Unreadable { .. } => "unreadable",
            Self::StoredInvalid { .. } => "stored_invalid",
            Self::StoredIdMismatch { .. } => "stored_id_mismatch",
            Self::Unwritable { .. } => "unwritable",
            Self::Shown { cause } => cause.key(),
            Self::TextServiceMissing { .. } => "text_service_missing",
            Self::TextService { .. } => "text_service",
            Self::AnswerRefused { .. } => "answer_refused",
            Self::InvalidOperationKey => "invalid_operation_key",
            Self::InvalidModel { .. } => "invalid_model",
            Self::NotFillable { .. } => "not_fillable",
            Self::NoGaps => "no_gaps",
            Self::PrivateDetailInPublicField => "private_detail",
            Self::BackfillsUnreadable { .. } => "backfills_unreadable",
            Self::BackfillsInvalid { .. } => "backfills_invalid",
            Self::BackfillsUnwritable { .. } => "backfills_unwritable",
            Self::Stopped => "stopped",
        }
    }
}

impl fmt::Display for PersonaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBody(error) => write!(formatter, "{error}"),
            Self::InvalidQuery { detail } => write!(formatter, "the query is not valid: {detail}"),
            Self::InvalidPersona { cause } => write!(formatter, "{cause}"),
            Self::ReservedId { id } => write!(
                formatter,
                "{id:?} is a word the persona commands use, so it cannot be a persona id"
            ),
            Self::InvalidEdit { cause } => write!(formatter, "{cause}"),
            Self::IdMismatch { expected, found } => write!(
                formatter,
                "the persona file holds {found:?}, not {expected:?}"
            ),
            Self::UnknownPersona { id } => write!(
                formatter,
                "no persona is stored for {id:?} (list them with: persona list)"
            ),
            Self::FolderUnreadable { cause } => {
                write!(
                    formatter,
                    "the personas folder could not be listed: {cause}"
                )
            }
            Self::Unreadable { id, cause } => write!(
                formatter,
                "the persona file of {id:?} could not be read: {cause}"
            ),
            Self::StoredInvalid { id, detail } => write!(
                formatter,
                "the stored persona {id:?} is not valid: {detail}"
            ),
            Self::StoredIdMismatch { id, found } => write!(
                formatter,
                "the persona file of {id:?} holds the persona {found:?}"
            ),
            Self::Unwritable { id, cause } => write!(
                formatter,
                "the persona file of {id:?} could not be changed: {cause}"
            ),
            Self::Shown { cause } => write!(formatter, "{cause}"),
            Self::TextServiceMissing { missing } => formatter.write_str(missing),
            Self::TextService {
                cause,
                reason: None,
            } => write!(formatter, "the text service {cause}"),
            Self::TextService {
                cause,
                reason: Some(reason),
            } => write!(formatter, "the text service {cause} ({})", reason.key()),
            Self::AnswerRefused { replayed, cause } => write!(
                formatter,
                "the text service gave {} answer the reader refused: {cause}",
                if *replayed { "a replayed" } else { "an" }
            ),
            Self::InvalidOperationKey => formatter.write_str("the operation key is not valid"),
            Self::InvalidModel { cause } => write!(formatter, "{cause}"),
            Self::NotFillable { field } => write!(
                formatter,
                "the persona's {} is not a field a backfill can fill",
                field.key()
            ),
            Self::NoGaps => formatter.write_str("a backfill needs at least one gap"),
            Self::PrivateDetailInPublicField => formatter.write_str(concat!(
                "the backfill has a word of the character's private side in a public field, ",
                "so nothing was written (a whole-word check: inflections and paraphrase pass it, ",
                "so it reduces accidents and is not a guarantee)"
            )),
            Self::BackfillsUnreadable { id, cause } => write!(
                formatter,
                "the backfill record of {id:?} could not be read: {cause}"
            ),
            Self::BackfillsInvalid { id, detail } => write!(
                formatter,
                "the backfill record of {id:?} is not valid and is left as it is: {detail}"
            ),
            Self::BackfillsUnwritable {
                id,
                cause,
                persona_restored,
            } => write!(
                formatter,
                "the backfill record of {id:?} could not be written: {cause}; the persona file {}",
                if *persona_restored {
                    "was put back as it was"
                } else {
                    "keeps the backfill (it could not be put back)"
                }
            ),
            Self::Stopped => formatter.write_str("the persona task stopped without an answer"),
        }
    }
}

impl std::error::Error for PersonaError {}

impl From<BodyError> for PersonaError {
    fn from(error: BodyError) -> Self {
        Self::InvalidBody(error)
    }
}

impl From<ShownSceneError> for PersonaError {
    fn from(cause: ShownSceneError) -> Self {
        Self::Shown { cause }
    }
}

impl From<SceneApplyError> for PersonaError {
    fn from(error: SceneApplyError) -> Self {
        Self::Shown {
            cause: ShownSceneError::from(error),
        }
    }
}

impl<E: fmt::Display + Into<AnswerError>> From<TextAskError<E>> for PersonaError {
    fn from(error: TextAskError<E>) -> Self {
        match error {
            TextAskError::Service { cause, reason } => Self::TextService { cause, reason },
            TextAskError::AnswerRefused(error) => Self::AnswerRefused {
                replayed: false,
                cause: error.into(),
            },
            TextAskError::ReplayRefused(error) => Self::AnswerRefused {
                replayed: true,
                cause: error.into(),
            },
        }
    }
}

impl RouteError for PersonaError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidBody(_)
            | Self::InvalidQuery { .. }
            | Self::InvalidPersona { .. }
            | Self::ReservedId { .. }
            | Self::InvalidModel { .. }
            | Self::NotFillable { .. }
            | Self::NoGaps
            | Self::IdMismatch { .. } => StatusCode::BAD_REQUEST,
            Self::PrivateDetailInPublicField | Self::InvalidEdit { .. } => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            Self::Shown { cause } => cause.status(),
            Self::UnknownPersona { .. } => StatusCode::NOT_FOUND,
            Self::TextServiceMissing { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::TextService { .. } | Self::AnswerRefused { .. } => StatusCode::BAD_GATEWAY,
            Self::FolderUnreadable { .. }
            | Self::Unreadable { .. }
            | Self::StoredInvalid { .. }
            | Self::StoredIdMismatch { .. }
            | Self::Unwritable { .. }
            | Self::BackfillsUnreadable { .. }
            | Self::BackfillsInvalid { .. }
            | Self::BackfillsUnwritable { .. }
            | Self::InvalidOperationKey
            | Self::Stopped => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

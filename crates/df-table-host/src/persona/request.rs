//! The bodies and replies of the persona routes, and why they refuse.
//!
//! Every route answers a `PersonaReply<T>`: `{"outcome":"done","value":..}` or
//! `{"outcome":"rejected","error":{"error":"<key>",..},"message":".."}`.

use std::fmt;

use axum::http::StatusCode;
use df_content::persona::backfill::{BackfillError, Gap, NarrativeGuidance};
use df_content::persona::generation::GenerationError;
use df_content::persona::privacy::PrivacyError;
use df_content::persona::{
    Appraisal, EditError, Persona, PersonaEdit, PersonaError as ContentError, PersonaField,
    PersonaState,
};
use serde::{Deserialize, Serialize};

use crate::dev_http::{BodyError, DevReply, RouteError};
use crate::io_cause::IoCause;
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

// The page reads the list, so the page owns the summary and the JSON is one.
pub use df_playground::voice_wire::PersonaSummary;

/// Who `persona` is and what they do. Nothing private.
pub(super) fn summary_of(persona: &Persona) -> PersonaSummary {
    PersonaSummary {
        id: persona.id().as_str().to_owned(),
        display_name: persona.display_name().to_owned(),
        job: persona.job().to_owned(),
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
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The last lines of the conversation, oldest first, so the line does not repeat them.
    #[serde(default)]
    pub recent: Vec<String>,
    /// Who the line is said to (a persona id or a name), so triggers are judged by what they do.
    #[serde(default)]
    pub asked_by: Option<String>,
    /// What the character sets out to do with this line ("ask her the price"), apart from the
    /// situation they are in, so a line meant to ask asks and never answers itself.
    #[serde(default)]
    pub intent: Option<String>,
}

fn yes() -> bool {
    true
}

// What a player says, and what a situation holds, is data for the model and never printed: the
// Debug of a request that carries one shows its lengths, as the page's own request does.
impl std::fmt::Debug for SayRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SayRequest")
            .field("character", &self.character)
            .field("situation_characters", &self.situation.chars().count())
            .field("model", &self.model)
            .field("speak", &self.speak)
            .field("recent_lines", &self.recent.len())
            .field("asked_by", &self.asked_by)
            .finish()
    }
}

impl std::fmt::Debug for AskRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AskRequest")
            .field("character", &self.character)
            .field("question_characters", &self.question.chars().count())
            .field("has_guidance", &self.guidance.is_some())
            .field("model", &self.model)
            .field("backfill", &self.backfill)
            .field("backfill_sync", &self.backfill_sync)
            .field("coverage", &self.coverage)
            .field("speak", &self.speak)
            .field("has_situation", &self.situation.is_some())
            .field("recent_lines", &self.recent.len())
            .field("asked_by", &self.asked_by)
            .finish()
    }
}

impl std::fmt::Debug for CoverageRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CoverageRequest")
            .field("character", &self.character)
            .field("question_characters", &self.question.chars().count())
            .field("has_guidance", &self.guidance.is_some())
            .field("model", &self.model)
            .finish()
    }
}

/// The body of `POST /dev/persona/ask`: put a question to a character. One model call words the
/// line and reports what the persona lacked (gaps) and how the exchange moved the character; the
/// gaps are filled afterwards (adding only), off the critical path. A dev tool: the narrative
/// engine calls the same steps.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskRequest {
    pub character: String,
    pub question: String,
    #[serde(default)]
    pub guidance: Option<NarrativeGuidance>,
    #[serde(default)]
    pub model: Option<String>,
    /// Fill the gaps the line reported, after it. On unless it says otherwise.
    #[serde(default = "yes")]
    pub backfill: bool,
    /// Wait for that backfill before answering (debug and tests). Off unless it says otherwise.
    #[serde(default)]
    pub backfill_sync: bool,
    /// Run the old separate coverage call, and a backfill before the line, instead of the gaps the
    /// line reports (debugging). Off unless it says otherwise.
    #[serde(default)]
    pub coverage: bool,
    /// Voice the line and play it with its bubble. Off unless it says otherwise.
    #[serde(default)]
    pub speak: bool,
    /// What is happening around the question; it is data for the model.
    #[serde(default)]
    pub situation: Option<String>,
    /// The last lines of the conversation, oldest first, so the answer does not repeat them.
    #[serde(default)]
    pub recent: Vec<String>,
    /// Who asks (a persona id or a name), so triggers are judged by what they do.
    #[serde(default)]
    pub asked_by: Option<String>,
}

/// The body of `POST /dev/persona/voice`: the text service writes only the voice, the expertise
/// and the voice packs of the stored persona `id`; everything else of it is untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceRequest {
    pub id: String,
    /// The model to ask instead of the persona default.
    #[serde(default)]
    pub model: Option<String>,
}

/// The body of `POST /dev/persona/coverage`: does the persona hold enough to answer `question`?
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The backfill runs after the line on a task of its own; `PERSONA backfill_done` records it
    /// and `persona backfills` lists it as record `record`.
    Running {
        record: u64,
    },
    /// A backfill of this persona is already running, so this line's gaps are not queued.
    Busy,
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
    /// A new persona was written and stored. `leak_suspect` is true when the model judged that its
    /// public texts give away its private side (the same call reports it); nothing was refused.
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

/// A line a character said, and the stance, slips, gaps and appraisal the model reported with it in
/// the same call. The model only words and reports; the production engine, not the model, owns
/// whether a secret is revealed, and code validates every id and number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaidLine {
    pub line: String,
    /// The behavior id the line took: a fired trigger's behavior wins over the model's own.
    pub behavior: Option<String>,
    /// The trigger ids of this persona that the model judged fired.
    pub fired_triggers: Vec<String>,
    /// The emotional trigger ids that fired by event (the model's judgement) or by state (code's).
    #[serde(default)]
    pub fired_emotional_triggers: Vec<String>,
    /// The emotional triggers whose state condition held before the line, checked by code.
    #[serde(default)]
    pub state_triggered: Vec<String>,
    /// Set when the model's behavior disagreed with a fired trigger's and code took the trigger's.
    #[serde(default)]
    pub behavior_override: Option<BehaviorOverride>,
    /// The secret ids of this persona that the line gave away.
    pub revealed_secrets: Vec<String>,
    /// The total in copper pieces the line quotes, held inside the stall's bounds when the bible
    /// knows the stall; the unchecked number otherwise.
    #[serde(default)]
    pub quoted_cp: Option<u64>,
    /// What the line quoted and what the bible allowed.
    #[serde(default)]
    pub quote: Option<QuoteReport>,
    /// True when the model judged, in the call that worded the line, that the line gives away the
    /// character's private side beyond what the question and the situation already say. It blocks
    /// nothing: it is the model's judgement, a hint and not a guarantee.
    pub leak_suspect: bool,
    /// What the question needed that the persona and the world facts do not hold.
    #[serde(default)]
    pub gaps: Vec<Gap>,
    /// How the model says the exchange moved the character, after code kept only what is valid.
    #[serde(default)]
    pub appraisal: Appraisal,
    /// What of the answer code dropped as not valid (an emotion, speaker or tone that is not on
    /// the lists), as `kind:id`.
    #[serde(default)]
    pub dropped: Vec<String>,
    /// The bubble tone id of the line: the appraisal's when valid, else the character's usual.
    #[serde(default)]
    pub tone: Option<String>,
    /// True when the line still reuses a phrase of `recent` after the one retry.
    #[serde(default)]
    pub repeats_recent: bool,
    /// True when the first answer repeated `recent` and the line is the second.
    #[serde(default)]
    pub repeat_retried: bool,
    /// The character's state after this line.
    #[serde(default)]
    pub state: PersonaState,
    /// The time the model calls took, in milliseconds.
    #[serde(default)]
    pub model_ms: u64,
    /// The time the whole line took, in milliseconds.
    #[serde(default)]
    pub total_ms: u64,
}

/// A fired trigger's behavior taken over a different one the model named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BehaviorOverride {
    /// The behavior the model named, when it named one.
    pub model: Option<String>,
    /// The fired trigger whose behavior the line took.
    pub trigger: String,
    pub took: String,
}

/// The bounds the bible gives a quote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteBounds {
    pub clamped_total_cp: u32,
    pub floor_total_cp: u32,
    pub ask_total_cp: u32,
}

/// A quote a line made: the lot and the total the model proposed, and the bounds when the bible
/// knows the speaker's stall and the item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteReport {
    pub item: String,
    pub qty: u32,
    pub proposed_total_cp: u64,
    pub bounds: Option<QuoteBounds>,
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
    /// The answer is not the object the schema describes. `detail` is the reader's own sentence
    /// (it names the field or type it stopped at), which has no closed set to enumerate.
    Malformed { detail: String },
    /// The line is blank.
    Blank,
    /// The answer names a behavior, trigger or secret the persona does not have.
    UnknownId { kind: LineIdKind, id: String },
    /// The answer's gaps are not gaps a backfill can fill.
    Gaps { cause: BackfillError },
}

/// What an id of a line's answer names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineIdKind {
    Behavior,
    Trigger,
    EmotionalTrigger,
    Secret,
}

impl LineIdKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Behavior => "behavior",
            Self::Trigger => "trigger",
            Self::EmotionalTrigger => "emotional trigger",
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
            Self::Gaps { cause } => write!(formatter, "the answer's gaps are not valid: {cause}"),
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
    Privacy { error: PrivacyError },
}

impl fmt::Display for AnswerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Generation { error } => write!(formatter, "{error}"),
            Self::Backfill { error } => write!(formatter, "{error}"),
            Self::Line { error } => write!(formatter, "{error}"),
            Self::Privacy { error } => write!(formatter, "{error}"),
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

impl From<PrivacyError> for AnswerError {
    fn from(error: PrivacyError) -> Self {
        Self::Privacy { error }
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
    /// The query is not one the route reads. `detail` is the query reader's own sentence, which
    /// has no closed set to enumerate.
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
    /// The file of `id` is not a valid persona; `cause` says where and why the reader stopped.
    StoredInvalid { id: String, cause: BodyError },
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
    /// The model judged that a backfill gives away the character's private side in a public
    /// field; nothing was written. The judgement is the model's, not a guarantee.
    #[serde(rename = "private_detail")]
    PrivateDetailInPublicField,
    /// The record of the backfills of `id` could not be read.
    BackfillsUnreadable { id: String, cause: IoCause },
    /// The record of the backfills of `id` is not valid and is left as it is; `cause` says where
    /// and why the reader stopped.
    BackfillsInvalid { id: String, cause: BodyError },
    /// The record of the backfills of `id` already holds the largest sequence number, so no
    /// further backfill can be recorded.
    BackfillSequenceExhausted { id: String },
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
            Self::BackfillSequenceExhausted { .. } => "backfill_sequence_exhausted",
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
            Self::StoredInvalid { id, cause } => write!(
                formatter,
                "the stored persona {id:?} is not valid: {}",
                cause.detail
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
                "the model judged that the backfill gives away the character's private side in a ",
                "public field, so nothing was written (a judgement of meaning, not a guarantee)"
            )),
            Self::BackfillsUnreadable { id, cause } => write!(
                formatter,
                "the backfill record of {id:?} could not be read: {cause}"
            ),
            Self::BackfillsInvalid { id, cause } => write!(
                formatter,
                "the backfill record of {id:?} is not valid and is left as it is: {}",
                cause.detail
            ),
            Self::BackfillSequenceExhausted { id } => write!(
                formatter,
                "the backfill record of {id:?} cannot count further, so nothing was written"
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
            | Self::BackfillSequenceExhausted { .. }
            | Self::BackfillsUnwritable { .. }
            | Self::InvalidOperationKey
            | Self::Stopped => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

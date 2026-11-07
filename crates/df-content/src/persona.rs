//! A character's persona: the authored, persistent description of who they are.
//!
//! This is the content a `NpcState`'s `role`, `personality`, `motivations`, `goals`, `needs` and
//! `fears` references resolve to (`planning/interaction-engine.md`). The interaction engine decides
//! what a character does; a model only words it, and never invents persistent personality, so the
//! persona is the one place that personality is written down.
//!
//! A persona has two audiences. What any listener, the screen or a prompt for another character
//! may know is the [`PublicPersona`] ([`Persona::public_view`]): who they are, their job, how they
//! come across, how they talk and the part of their backstory that is common knowledge. Their
//! drives (motivations, goals, needs, fears) and `private_backstory` are the character's own, kept
//! server-side, and are read only through their own accessors.
//!
//! A persona links to a character by id. The id has the same grammar as a SAM layout label
//! (`df_scene_layout::is_character_name`), so a persona and a labelled mask match by equal text.
//! Reading goes through [`Persona::new`], so a stored persona is checked like a new one.

use std::fmt;

use serde::{Deserialize, Serialize};

pub mod backfill;
pub mod edit;
pub mod generation;
pub mod policy;

pub use edit::{EditError, PersonaEdit};
pub use policy::{PersonaBehavior, PersonaSecret, PersonaTrigger, Sensitivity};

#[cfg(test)]
mod backfill_tests;
#[cfg(test)]
mod edit_tests;
#[cfg(test)]
mod generation_tests;
#[cfg(test)]
mod policy_tests;

/// The persona file format this reader understands.
pub const SCHEMA_VERSION: u32 = 1;
/// The most characters in a persona id, the same bound as a SAM layout label.
pub const MAXIMUM_ID_CHARS: usize = 256;
/// The most entries in a persona's list of traits or drives.
pub const MAXIMUM_LIST_ENTRIES: usize = 1_000;
/// The most characters in one name, job, speech style or list entry.
pub const MAXIMUM_ENTRY_CHARS: usize = 10_000;
/// The most characters in either backstory.
pub const MAXIMUM_BACKSTORY_CHARS: usize = 100_000;

/// A part of a persona, for naming where a check failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonaField {
    Id,
    DisplayName,
    Job,
    Personality,
    Motivations,
    Goals,
    Needs,
    Fears,
    SpeechStyle,
    Backstory,
    PrivateBackstory,
    Boundaries,
    Secrets,
    Behaviors,
    DefaultBehavior,
    Triggers,
}

impl PersonaField {
    /// Every field, in declaration order.
    pub const ALL: [Self; 16] = [
        Self::Id,
        Self::DisplayName,
        Self::Job,
        Self::Personality,
        Self::Motivations,
        Self::Goals,
        Self::Needs,
        Self::Fears,
        Self::SpeechStyle,
        Self::Backstory,
        Self::PrivateBackstory,
        Self::Boundaries,
        Self::Secrets,
        Self::Behaviors,
        Self::DefaultBehavior,
        Self::Triggers,
    ];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::DisplayName => "display_name",
            Self::Job => "job",
            Self::Personality => "personality",
            Self::Motivations => "motivations",
            Self::Goals => "goals",
            Self::Needs => "needs",
            Self::Fears => "fears",
            Self::SpeechStyle => "speech_style",
            Self::Backstory => "backstory",
            Self::PrivateBackstory => "private_backstory",
            Self::Boundaries => "boundaries",
            Self::Secrets => "secrets",
            Self::Behaviors => "behaviors",
            Self::DefaultBehavior => "default_behavior",
            Self::Triggers => "triggers",
        }
    }

    /// The field with serde name `key`.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|field| field.key() == key)
    }
}

/// Why a persona was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum PersonaError {
    /// The file's schema version is not [`SCHEMA_VERSION`].
    UnsupportedSchema { schema: u32 },
    /// The id is empty, too long or has a character other than a letter, a digit, `-` or `_`.
    InvalidId { id: String },
    /// A required field is empty or only blank.
    Blank { field: PersonaField },
    /// A list entry is empty or only blank.
    BlankEntry { field: PersonaField, index: usize },
    /// A text is longer than its bound.
    TooLong {
        field: PersonaField,
        characters: usize,
        maximum: usize,
    },
    /// A list has more entries than its bound.
    TooMany {
        field: PersonaField,
        entries: usize,
        maximum: usize,
    },
    /// A text has a control character other than a newline or a tab.
    ControlCharacter { field: PersonaField },
    /// An item's id, a trigger's event or a secret's `known_by` entry is empty, too long or has
    /// a character other than a letter, a digit, `-` or `_`.
    InvalidItemId { field: PersonaField, id: String },
    /// Two items of one list have the same id.
    DuplicateItemId { field: PersonaField, id: String },
    /// A trigger or the default names a behavior the persona does not have.
    UnknownBehavior { behavior: String },
}

impl fmt::Display for PersonaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema { schema } => write!(
                formatter,
                "the persona is in schema {schema}; this reader reads {SCHEMA_VERSION}"
            ),
            Self::InvalidId { id } => write!(
                formatter,
                "{id:?} is not a persona id: use 1 to {MAXIMUM_ID_CHARS} letters, digits, '-', '_'"
            ),
            Self::Blank { field } => write!(formatter, "the persona's {} is blank", field.key()),
            Self::BlankEntry { field, index } => write!(
                formatter,
                "entry {index} of the persona's {} is blank",
                field.key()
            ),
            Self::TooLong {
                field,
                characters,
                maximum,
            } => write!(
                formatter,
                "the persona's {} is {characters} characters; at most {maximum}",
                field.key()
            ),
            Self::TooMany {
                field,
                entries,
                maximum,
            } => write!(
                formatter,
                "the persona's {} has {entries} entries; at most {maximum}",
                field.key()
            ),
            Self::ControlCharacter { field } => write!(
                formatter,
                "the persona's {} has a control character",
                field.key()
            ),
            Self::InvalidItemId { field, id } => write!(
                formatter,
                "{id:?} is not an id in the persona's {}: use letters, digits, '-' or '_'",
                field.key()
            ),
            Self::DuplicateItemId { field, id } => write!(
                formatter,
                "the persona's {} has two items with the id {id:?}",
                field.key()
            ),
            Self::UnknownBehavior { behavior } => {
                write!(formatter, "the persona has no behavior {behavior:?}")
            }
        }
    }
}

impl std::error::Error for PersonaError {}

/// The id of a persona, which is also the name of the character it describes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PersonaId(String);

/// Whether `text` is a label: 1 to [`MAXIMUM_ID_CHARS`] letters, digits, `-` or `_`. The grammar
/// of a persona id and of the ids of a persona's items.
fn is_label(text: &str) -> bool {
    !text.is_empty()
        && text.chars().count() <= MAXIMUM_ID_CHARS
        && text
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

impl PersonaId {
    /// The id `text`: 1 to [`MAXIMUM_ID_CHARS`] letters, digits, `-` or `_`.
    pub fn new(text: &str) -> Result<Self, PersonaError> {
        if is_label(text) {
            Ok(Self(text.to_owned()))
        } else {
            Err(PersonaError::InvalidId {
                id: text.chars().take(MAXIMUM_ID_CHARS + 1).collect(),
            })
        }
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PersonaId {
    type Error = PersonaError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(&text)
    }
}

impl From<PersonaId> for String {
    fn from(id: PersonaId) -> Self {
        id.0
    }
}

impl fmt::Display for PersonaId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A persona as written, before it is checked ([`Persona::new`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PersonaDraft {
    /// The character's name as the layout labels it, such as `innkeeper`.
    pub id: String,
    /// What the story calls them, such as `Marta Venn`.
    pub display_name: String,
    /// What they do, such as `Keeper of the Ember Roads inn`.
    pub job: String,
    /// How they come across: one trait per entry, as a short phrase or sentence.
    pub personality: Vec<String>,
    /// What moves them day to day. Private.
    pub motivations: Vec<String>,
    /// What they are after. Private.
    pub goals: Vec<String>,
    /// What they lack. Private.
    pub needs: Vec<String>,
    /// What they dread. Private.
    pub fears: Vec<String>,
    /// How they talk: a short description of voice, pace and habits of speech.
    pub speech_style: String,
    /// The part of their past anyone in the scene could know.
    pub backstory: String,
    /// The rest of their past. Private.
    pub private_backstory: String,
    /// What they will never do, whatever is offered. Private.
    pub boundaries: Vec<String>,
    /// What they keep back. Private.
    pub secrets: Vec<PersonaSecret>,
    /// The stances they can take. Private.
    pub behaviors: Vec<PersonaBehavior>,
    /// The behavior they start in, by id. Private.
    pub default_behavior: Option<String>,
    /// What moves them between behaviors. Private.
    pub triggers: Vec<PersonaTrigger>,
}

/// The stored form of a [`Persona`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersonaRecord {
    schema: u32,
    id: PersonaId,
    display_name: String,
    job: String,
    #[serde(default)]
    personality: Vec<String>,
    #[serde(default)]
    motivations: Vec<String>,
    #[serde(default)]
    goals: Vec<String>,
    #[serde(default)]
    needs: Vec<String>,
    #[serde(default)]
    fears: Vec<String>,
    #[serde(default)]
    speech_style: String,
    #[serde(default)]
    backstory: String,
    #[serde(default)]
    private_backstory: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    boundaries: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    secrets: Vec<PersonaSecret>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    behaviors: Vec<PersonaBehavior>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_behavior: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    triggers: Vec<PersonaTrigger>,
}

/// A character's persona. Its parts are checked when it is made, so a persona in hand is valid.
/// Its `Debug` output names only the public identity, so a private secret never reaches a log by
/// accident.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PersonaRecord", into = "PersonaRecord")]
pub struct Persona {
    id: PersonaId,
    display_name: String,
    job: String,
    personality: Vec<String>,
    motivations: Vec<String>,
    goals: Vec<String>,
    needs: Vec<String>,
    fears: Vec<String>,
    speech_style: String,
    backstory: String,
    private_backstory: String,
    boundaries: Vec<String>,
    secrets: Vec<PersonaSecret>,
    behaviors: Vec<PersonaBehavior>,
    default_behavior: Option<String>,
    triggers: Vec<PersonaTrigger>,
}

/// What a listener, the screen or another character's prompt may know of a persona.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicPersona {
    pub id: String,
    pub display_name: String,
    pub job: String,
    pub personality: Vec<String>,
    pub speech_style: String,
    pub backstory: String,
}

/// Whose prompt a persona is being written into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BriefScope {
    /// Another character's prompt, a view or a log: only the public persona.
    Listener,
    /// The character's own prompt, when a model words what they say: the public persona and the
    /// character's private side.
    Own,
}

/// The character's private side: what they want, lack and fear, and the rest of their past.
/// Its `Debug` output shows nothing of it.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct PrivatePersona {
    pub motivations: Vec<String>,
    pub goals: Vec<String>,
    pub needs: Vec<String>,
    pub fears: Vec<String>,
    pub backstory: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub boundaries: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<PersonaSecret>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub behaviors: Vec<PersonaBehavior>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_behavior: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<PersonaTrigger>,
}

impl fmt::Debug for Persona {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Persona")
            .field("id", &self.id.as_str())
            .field("display_name", &self.display_name)
            .field("job", &self.job)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for PrivatePersona {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PrivatePersona { .. }")
    }
}

/// A persona as written into a prompt's facts for one audience ([`Persona::brief`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PersonaBrief {
    #[serde(flatten)]
    pub public: PublicPersona,
    /// Present only in the character's own brief.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private: Option<PrivatePersona>,
}

fn check_text(
    field: PersonaField,
    text: &str,
    maximum: usize,
    required: bool,
) -> Result<(), PersonaError> {
    if required && text.trim().is_empty() {
        return Err(PersonaError::Blank { field });
    }
    let characters = text.chars().count();
    if characters > maximum {
        return Err(PersonaError::TooLong {
            field,
            characters,
            maximum,
        });
    }
    if text
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Err(PersonaError::ControlCharacter { field });
    }
    Ok(())
}

/// A text that must stay on one line (a name, a job, a trait, a condition): [`check_text`] and no
/// line break or tab either, so it cannot break the structure of a prompt it is written into.
fn check_line(
    field: PersonaField,
    text: &str,
    maximum: usize,
    required: bool,
) -> Result<(), PersonaError> {
    if text
        .chars()
        .any(|character| matches!(character, '\n' | '\t'))
    {
        return Err(PersonaError::ControlCharacter { field });
    }
    check_text(field, text, maximum, required)
}

fn check_list(field: PersonaField, entries: &[String]) -> Result<(), PersonaError> {
    if entries.len() > MAXIMUM_LIST_ENTRIES {
        return Err(PersonaError::TooMany {
            field,
            entries: entries.len(),
            maximum: MAXIMUM_LIST_ENTRIES,
        });
    }
    for (index, entry) in entries.iter().enumerate() {
        if entry.trim().is_empty() {
            return Err(PersonaError::BlankEntry { field, index });
        }
        check_line(field, entry, MAXIMUM_ENTRY_CHARS, false)?;
    }
    Ok(())
}

impl Persona {
    /// The persona `draft` describes, when every part of it is valid.
    pub fn new(draft: PersonaDraft) -> Result<Self, PersonaError> {
        let id = PersonaId::new(&draft.id)?;
        check_line(
            PersonaField::DisplayName,
            &draft.display_name,
            MAXIMUM_ENTRY_CHARS,
            true,
        )?;
        check_line(PersonaField::Job, &draft.job, MAXIMUM_ENTRY_CHARS, true)?;
        check_list(PersonaField::Personality, &draft.personality)?;
        check_list(PersonaField::Motivations, &draft.motivations)?;
        check_list(PersonaField::Goals, &draft.goals)?;
        check_list(PersonaField::Needs, &draft.needs)?;
        check_list(PersonaField::Fears, &draft.fears)?;
        check_line(
            PersonaField::SpeechStyle,
            &draft.speech_style,
            MAXIMUM_ENTRY_CHARS,
            false,
        )?;
        check_text(
            PersonaField::Backstory,
            &draft.backstory,
            MAXIMUM_BACKSTORY_CHARS,
            false,
        )?;
        check_text(
            PersonaField::PrivateBackstory,
            &draft.private_backstory,
            MAXIMUM_BACKSTORY_CHARS,
            false,
        )?;
        check_list(PersonaField::Boundaries, &draft.boundaries)?;
        policy::check(
            &draft.secrets,
            &draft.behaviors,
            draft.default_behavior.as_deref(),
            &draft.triggers,
        )?;
        Ok(Self {
            id,
            display_name: draft.display_name,
            job: draft.job,
            personality: draft.personality,
            motivations: draft.motivations,
            goals: draft.goals,
            needs: draft.needs,
            fears: draft.fears,
            speech_style: draft.speech_style,
            backstory: draft.backstory,
            private_backstory: draft.private_backstory,
            boundaries: draft.boundaries,
            secrets: draft.secrets,
            behaviors: draft.behaviors,
            default_behavior: draft.default_behavior,
            triggers: draft.triggers,
        })
    }

    /// The character's name as the layout labels it.
    pub fn id(&self) -> &PersonaId {
        &self.id
    }

    /// What the story calls them.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// What they do.
    pub fn job(&self) -> &str {
        &self.job
    }

    /// How they come across, one trait per entry.
    pub fn personality(&self) -> &[String] {
        &self.personality
    }

    /// How they talk.
    pub fn speech_style(&self) -> &str {
        &self.speech_style
    }

    /// The part of their past anyone in the scene could know.
    pub fn backstory(&self) -> &str {
        &self.backstory
    }

    /// What moves them. The character's own: never for another listener or a view.
    pub fn private_motivations(&self) -> &[String] {
        &self.motivations
    }

    /// What they are after. The character's own.
    pub fn private_goals(&self) -> &[String] {
        &self.goals
    }

    /// What they lack. The character's own.
    pub fn private_needs(&self) -> &[String] {
        &self.needs
    }

    /// What they dread. The character's own.
    pub fn private_fears(&self) -> &[String] {
        &self.fears
    }

    /// The rest of their past. The character's own.
    pub fn private_backstory(&self) -> &str {
        &self.private_backstory
    }

    /// What they will never do. The character's own.
    pub fn private_boundaries(&self) -> &[String] {
        &self.boundaries
    }

    /// What they keep back. The character's own.
    pub fn private_secrets(&self) -> &[PersonaSecret] {
        &self.secrets
    }

    /// The stances they can take. The character's own.
    pub fn private_behaviors(&self) -> &[PersonaBehavior] {
        &self.behaviors
    }

    /// The behavior they start in, by id. The character's own.
    pub fn private_default_behavior(&self) -> Option<&str> {
        self.default_behavior.as_deref()
    }

    /// What moves them between behaviors. The character's own.
    pub fn private_triggers(&self) -> &[PersonaTrigger] {
        &self.triggers
    }

    /// What a listener, the screen or another character's prompt may know of them.
    pub fn public_view(&self) -> PublicPersona {
        PublicPersona {
            id: self.id.as_str().to_owned(),
            display_name: self.display_name.clone(),
            job: self.job.clone(),
            personality: self.personality.clone(),
            speech_style: self.speech_style.clone(),
            backstory: self.backstory.clone(),
        }
    }
}

impl Persona {
    /// The persona for `scope`'s prompt: the public persona for a [`BriefScope::Listener`], and
    /// that with the private side for the character's [`BriefScope::Own`] lines. It is data for a
    /// prompt's facts; the instructions around it say to treat it as data, never as instructions.
    pub fn brief(&self, scope: BriefScope) -> PersonaBrief {
        let private = matches!(scope, BriefScope::Own).then(|| PrivatePersona {
            motivations: self.motivations.clone(),
            goals: self.goals.clone(),
            needs: self.needs.clone(),
            fears: self.fears.clone(),
            backstory: self.private_backstory.clone(),
            boundaries: self.boundaries.clone(),
            secrets: self.secrets.clone(),
            behaviors: self.behaviors.clone(),
            default_behavior: self.default_behavior.clone(),
            triggers: self.triggers.clone(),
        });
        PersonaBrief {
            public: self.public_view(),
            private,
        }
    }
}

impl TryFrom<PersonaRecord> for Persona {
    type Error = PersonaError;

    fn try_from(record: PersonaRecord) -> Result<Self, Self::Error> {
        if record.schema != SCHEMA_VERSION {
            return Err(PersonaError::UnsupportedSchema {
                schema: record.schema,
            });
        }
        Self::new(PersonaDraft {
            id: record.id.into(),
            display_name: record.display_name,
            job: record.job,
            personality: record.personality,
            motivations: record.motivations,
            goals: record.goals,
            needs: record.needs,
            fears: record.fears,
            speech_style: record.speech_style,
            backstory: record.backstory,
            private_backstory: record.private_backstory,
            boundaries: record.boundaries,
            secrets: record.secrets,
            behaviors: record.behaviors,
            default_behavior: record.default_behavior,
            triggers: record.triggers,
        })
    }
}

impl From<Persona> for PersonaRecord {
    fn from(persona: Persona) -> Self {
        Self {
            schema: SCHEMA_VERSION,
            id: persona.id,
            display_name: persona.display_name,
            job: persona.job,
            personality: persona.personality,
            motivations: persona.motivations,
            goals: persona.goals,
            needs: persona.needs,
            fears: persona.fears,
            speech_style: persona.speech_style,
            backstory: persona.backstory,
            private_backstory: persona.private_backstory,
            boundaries: persona.boundaries,
            secrets: persona.secrets,
            behaviors: persona.behaviors,
            default_behavior: persona.default_behavior,
            triggers: persona.triggers,
        }
    }
}

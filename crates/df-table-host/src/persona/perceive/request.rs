//! The bodies and replies of the perception routes, and why they refuse.
//!
//! Every route answers a `PerceiveReply<T>`: `{"outcome":"done","value":..}` or
//! `{"outcome":"rejected","error":{"error":"<key>",..},"message":".."}`. The error is the judge
//! routes' shared one (`JudgeRouteError`) over this API's own [`PerceiveCause`], so a refusal of
//! the request and a refusal of the model's answer carry the same typed keys.

use std::fmt;

use df_content::persona::PersonaId;
use serde::{Deserialize, Serialize};

use super::super::judge_route::JudgeRouteError;
use super::super::request::PersonaError;
use crate::dev_http::DevReply;
use crate::locate::request::LocateError;
use crate::shown_scene::ShownSceneError;

/// Why a perception request, or the model's answer to it, was refused.
pub type PerceiveRouteError = JudgeRouteError<PerceiveCause>;

/// The answer of a perception route: its value, or why it was refused.
pub type PerceiveReply<T> = DevReply<T, PerceiveRouteError>;

/// How closely a character looks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// What catches the eye first: a few things, the most striking.
    #[default]
    Glance,
    /// A careful look: everything the character would pick out, what they attend to first.
    Study,
}

impl Mode {
    pub const ALL: [Self; 2] = [Self::Glance, Self::Study];

    pub fn key(self) -> &'static str {
        match self {
            Self::Glance => "glance",
            Self::Study => "study",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.key() == key)
    }
}

fn vision_by_default() -> bool {
    true
}

/// `POST /dev/persona/perceive`: what `who` sees on screen now, and what they are thinking about.
/// Every field but `who` may be left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerceiveRequest {
    /// The persona id of the character who looks.
    pub who: String,
    #[serde(default)]
    pub mode: Mode,
    /// The picture to look at, a media library name; by default the picture on screen now.
    #[serde(default)]
    pub picture: Option<String>,
    /// A region id of the picture's scan, or an item id of the place or of what the character
    /// carries, that the character is already attending to.
    #[serde(default)]
    pub focus: Option<String>,
    /// The last lines of the scene, oldest first, as `<speaker id>: <line>`.
    #[serde(default)]
    pub recent: Vec<String>,
    /// The words the picture was made from, for a character who cannot be given the picture
    /// itself (`vision: false`); never sent with the picture.
    #[serde(default)]
    pub scene: Option<String>,
    /// Whether the picture itself is sent to the model. False asks from the scan's layout and the
    /// scene's words alone.
    #[serde(default = "vision_by_default")]
    pub vision: bool,
    #[serde(default)]
    pub model: Option<String>,
}

/// `POST /dev/persona/perceive/scene`: what each of a list of characters sees of the same picture
/// from their own point of view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerceiveSceneRequest {
    /// The characters who look; none means everyone in the picture's cast who has a persona.
    #[serde(default)]
    pub who: Vec<String>,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub picture: Option<String>,
    #[serde(default)]
    pub recent: Vec<String>,
    #[serde(default)]
    pub scene: Option<String>,
    #[serde(default = "vision_by_default")]
    pub vision: bool,
    /// How many characters are asked at once, from 1 to the maximum; the default when left out.
    #[serde(default)]
    pub parallel: Option<usize>,
    #[serde(default)]
    pub model: Option<String>,
}

/// `GET /dev/persona/thoughts?who=&limit=`: the newest thoughts of a character.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThoughtsQuery {
    pub who: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Where the character looks from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "from", rename_all = "snake_case")]
pub enum Viewpoint {
    /// The character is in the picture, in the scan's region `region`.
    Pictured { region: String },
    /// The character is not in the picture: they see what the screen shows a viewer.
    Viewer,
}

/// What a sight is of. Only the place and the others in it are sights; the character's own body and
/// clothes and what they carry are known from their own facts, so they never draw attention.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Subject {
    /// The place or a thing in it.
    #[default]
    Place,
    /// Another person.
    Person,
    /// The character themselves: their body, their clothes.
    #[serde(rename = "self")]
    Own,
    /// Something the character holds.
    Carried,
}

impl Subject {
    pub fn key(self) -> &'static str {
        match self {
            Self::Place => "place",
            Self::Person => "person",
            Self::Own => "self",
            Self::Carried => "carried",
        }
    }

    /// Whether the sight is the character's own and so can never be what they attend to.
    pub fn is_own(self) -> bool {
        matches!(self, Self::Own | Self::Carried)
    }
}

/// One thing the character picks out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seen {
    /// What they see, in their own terms; a person they do not know by name is said by look.
    pub what: String,
    /// The scan region it is, one of the picture's.
    pub region: Option<String>,
    /// The item or thing of the place, or the one they carry, it is.
    pub item: Option<String>,
    /// How much it draws them, 0 to 100.
    pub salience: u8,
    /// What it is of: the place, another person, themselves or something they carry.
    #[serde(default)]
    pub subject: Subject,
}

/// A thing the character noticed that the place did not offer, proposed as a thing of the place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewThing {
    /// Its id as a thing of the place (the id grammar).
    pub id: String,
    pub name: String,
    /// What a camera brought up close would show of it, in neutral words.
    pub look: String,
    /// Whether it is now a thing of the active place, so the next close look can cut to it.
    pub added: bool,
    /// Why it was left out, and what to change, when it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why_not: Option<String>,
}

/// What the character now attends to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "on", rename_all = "snake_case")]
pub enum Attended {
    Region { id: String },
    Item { id: String },
    None,
}

impl Attended {
    /// As the audit line and the command line write it: `region:<id>`, `item:<id>` or `none`.
    pub fn label(&self) -> String {
        match self {
            Self::Region { id } => format!("region:{id}"),
            Self::Item { id } => format!("item:{id}"),
            Self::None => "none".to_owned(),
        }
    }
}

/// A feeling the sight stirs: an emotion of the campaign's list and its change in points.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feeling {
    pub emotion: String,
    pub delta: i8,
}

/// What one character sees and thinks, checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Perceived {
    pub who: String,
    pub mode: Mode,
    /// The picture looked at, when there is one.
    pub picture: Option<String>,
    /// Whether the model was given the picture itself. False: the answer is from the scan's layout
    /// and the scene's words alone.
    pub vision: bool,
    pub viewpoint: Viewpoint,
    /// How many scan regions the model was told of.
    pub regions: usize,
    pub sees: Vec<Seen>,
    /// The things among `sees` that the place did not offer and the character proposed as things of
    /// the place; each `sees` entry it came from now names it as its `item`.
    pub new_things: Vec<NewThing>,
    /// The character's inner thought, in their own voice.
    pub thinking_about: String,
    pub attention: Attended,
    /// The feeling the engine took into the character's state, when there was one.
    pub feeling: Option<Feeling>,
    /// What they now want to do, as short phrases.
    pub wants: Vec<String>,
    /// The model's report that the thought states the character's private side. A thought that
    /// does is never to be shown.
    pub leak_suspect: bool,
    /// The memory (`world memory`, kind `thought`) the answer was kept as, when it was kept.
    pub thought: Option<String>,
    pub duration_ms: u64,
}

/// What a character-by-character call did for one character.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SceneOutcome {
    Perceived { perceived: Box<Perceived> },
    Refused { error: PerceiveRouteError },
}

/// One character of `perceive scene` and what looking did for them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneRow {
    pub who: String,
    pub outcome: SceneOutcome,
}

/// What `perceive scene` answers: the picture they all looked at and a row for each character.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenePerceived {
    pub picture: Option<String>,
    pub vision: bool,
    pub results: Vec<SceneRow>,
}

/// One thought, as kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thought {
    /// The id of the memory it is kept as.
    pub id: String,
    /// Its place in the owner's memories: higher is later.
    pub seq: u64,
    /// The place the character was at, when the world had one.
    pub place: Option<String>,
    /// The picture they looked at, when it was a library picture.
    pub picture: Option<String>,
    /// The item or thing they attended to, when they attended to one.
    pub attended: Option<String>,
    /// The thought, then what they saw and wanted, as words.
    pub text: String,
}

/// What `GET /dev/persona/thoughts` answers: the newest thoughts, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thoughts {
    pub who: String,
    pub thoughts: Vec<Thought>,
}

/// Why a request, or the model's answer to it, cannot be used. `key()` is the serde tag (`kind`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PerceiveCause {
    /// The character's persona cannot be had.
    Persona { cause: Box<PersonaError> },
    /// The scene shows no picture, or what it shows cannot be read.
    Shown { cause: ShownSceneError },
    /// The picture the request names cannot be had: a name that is no library picture, a file that
    /// is missing or a layout that does not read.
    Picture { cause: LocateError },
    /// The focus is neither a region of the picture's scan nor an item the character can see.
    UnknownFocus { id: String },
    /// `perceive scene` was asked for no one: nobody in the cast has a persona.
    NoCharacters,
    /// `perceive scene` was asked for more characters than one call takes.
    TooManyCharacters { maximum: usize },
    /// `parallel` is not from 1 to the maximum.
    InvalidParallel { maximum: usize },
    /// The `thoughts` query names a limit that is not usable.
    InvalidLimit { maximum: usize },
    /// The thoughts of a character cannot be had.
    Thoughts { error: String },
    /// The answer is not the object the schema describes.
    Malformed { detail: String },
    /// The thought is blank.
    EmptyThought,
    /// A `sees` entry names a region the picture's scan does not have.
    UnknownRegion { id: String },
    /// A `sees` entry names an item the character cannot see or carry.
    UnknownItem { id: String },
    /// The feeling names an emotion that is not on the campaign's list.
    UnknownEmotion { emotion: String },
    /// The attention names both a region and an item, or one that is not offered.
    BadAttention { detail: String },
    /// A salience is outside 0 to 100.
    BadSalience { salience: i64 },
    /// A proposed new thing is not usable: it names a `sees` entry that is not there or is already
    /// another new thing, an id that is not in the id grammar or is already a thing or an item,
    /// words that say nothing, or it is attended to while the attention names something else.
    BadNewThing { detail: String },
    /// A proposed new thing is a vista (a distant town, open fields, the sky, a road), not an
    /// object a camera can be brought up close to, so it is not a thing of the place.
    Vista,
    /// A proposed new thing comes from a `sees` entry that has a region: the regions of the scan
    /// are people, and people are never things of the place.
    NewThingForPerson { see: usize },
    /// The answer calls someone by a handle (`person-N`) it was told of, though a handle has no
    /// meaning outside the call; say them by their look.
    HandleInText,
}

impl PerceiveCause {
    /// The cause's serde tag, for the audit line.
    pub fn key(&self) -> &'static str {
        match self {
            Self::Persona { .. } => "persona",
            Self::Shown { .. } => "shown",
            Self::Picture { .. } => "picture",
            Self::UnknownFocus { .. } => "unknown_focus",
            Self::NoCharacters => "no_characters",
            Self::TooManyCharacters { .. } => "too_many_characters",
            Self::InvalidParallel { .. } => "invalid_parallel",
            Self::InvalidLimit { .. } => "invalid_limit",
            Self::Thoughts { .. } => "thoughts",
            Self::Malformed { .. } => "malformed",
            Self::EmptyThought => "empty_thought",
            Self::UnknownRegion { .. } => "unknown_region",
            Self::UnknownItem { .. } => "unknown_item",
            Self::UnknownEmotion { .. } => "unknown_emotion",
            Self::BadAttention { .. } => "bad_attention",
            Self::BadSalience { .. } => "bad_salience",
            Self::BadNewThing { .. } => "bad_new_thing",
            Self::Vista => "vista",
            Self::NewThingForPerson { .. } => "new_thing_for_person",
            Self::HandleInText => "handle_in_text",
        }
    }
}

impl fmt::Display for PerceiveCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Persona { cause } => write!(formatter, "{cause}"),
            Self::Shown { cause } => write!(formatter, "{cause}"),
            Self::Picture { cause } => write!(formatter, "{cause}"),
            Self::UnknownFocus { id } => write!(
                formatter,
                "{id:?} is neither a region of the picture nor an item in sight or carried"
            ),
            Self::NoCharacters => {
                formatter.write_str("nobody in the picture's cast has a persona to look")
            }
            Self::TooManyCharacters { maximum } => {
                write!(formatter, "a scene call takes at most {maximum} characters")
            }
            Self::InvalidParallel { maximum } => {
                write!(formatter, "parallel is a whole number from 1 to {maximum}")
            }
            Self::InvalidLimit { maximum } => {
                write!(formatter, "limit is a whole number from 1 to {maximum}")
            }
            Self::Thoughts { error } => {
                write!(formatter, "the thoughts could not be had: {error}")
            }
            Self::Malformed { detail } => {
                write!(
                    formatter,
                    "the answer is not the object asked for: {detail}"
                )
            }
            Self::EmptyThought => formatter.write_str("thinking_about is blank"),
            Self::UnknownRegion { id } => {
                write!(formatter, "{id:?} is not a region of the picture's scan")
            }
            Self::UnknownItem { id } => write!(
                formatter,
                "{id:?} is not an item in sight or carried by the character"
            ),
            Self::UnknownEmotion { emotion } => {
                write!(formatter, "{emotion:?} is not on the emotion list")
            }
            Self::BadAttention { detail } => {
                write!(formatter, "the attention is not usable: {detail}")
            }
            Self::BadSalience { salience } => {
                write!(formatter, "salience {salience} is not from 0 to 100")
            }
            Self::BadNewThing { detail } => {
                write!(formatter, "a new thing is not usable: {detail}")
            }
            Self::Vista => formatter.write_str(
                "a vista is no thing of the place: a camera cannot go up close to it; remove its \
                 new_things entry",
            ),
            Self::NewThingForPerson { see } => write!(
                formatter,
                "sees entry {see} has a region, so it is a person, and a person is never a thing; \
                 remove its new_things entry"
            ),
            Self::HandleInText => formatter
                .write_str("the answer calls someone person-N; say them by how they look instead"),
        }
    }
}

impl std::error::Error for PerceiveCause {}

impl From<PersonaError> for PerceiveCause {
    fn from(cause: PersonaError) -> Self {
        Self::Persona {
            cause: Box::new(cause),
        }
    }
}

/// `text` as the id of a persona who looks, or the cause that says why it cannot be one.
pub(super) fn looker_id(text: &str) -> Result<PersonaId, PerceiveCause> {
    super::super::storage::persona_id(text).map_err(PerceiveCause::from)
}

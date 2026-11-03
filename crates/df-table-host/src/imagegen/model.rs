//! What crosses the picture generator: the request a verb makes, the picture and provenance the
//! backend returns, the typed reasons it refuses, and the answers of the dev route.
//!
//! The route and the command line read and write these types, so a rename breaks the build
//! instead of a running command. Nothing here holds a credential, and an audit line never holds
//! the prompt: it holds the prompt's length.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The limit for one generation unless the owner sets another: the owner wants large limits, and
/// a Codex picture takes minutes.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(15 * 60);

/// Extra time the host waits after the backend's own deadline for its blocking task to hand back
/// an answer, before it calls the attempt timed out itself.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(60);

/// The long side of the size asked of the backend, in pixels. The Codex image tool picks its own
/// size (about 1672 by 941 for a wide picture), so this is a request, not a promise.
pub const REQUESTED_LONG_SIDE_PX: u32 = 1672;

/// Most characters of prompt text one request may carry.
pub const MAXIMUM_PROMPT_CHARS: usize = 16_000;

/// Longest picture name, in characters.
pub const MAXIMUM_NAME_CHARS: usize = 64;

/// What the picture is for. It sets the default shape and adds one line of direction to the
/// prompt, so the owner's words stay the first thing the backend reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Plate,
    Background,
    Portrait,
}

impl Purpose {
    pub const ALL: [Self; 3] = [Self::Plate, Self::Background, Self::Portrait];

    pub fn key(self) -> &'static str {
        match self {
            Self::Plate => "plate",
            Self::Background => "background",
            Self::Portrait => "portrait",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|purpose| purpose.key() == text)
    }

    /// The shape a picture of this purpose has when none is asked for.
    pub fn default_aspect(self) -> Aspect {
        match self {
            Self::Plate | Self::Background => Aspect::WIDE,
            Self::Portrait => Aspect {
                width: 3,
                height: 4,
            },
        }
    }

    /// One line of direction added after the prompt.
    pub fn direction(self) -> &'static str {
        match self {
            Self::Plate => {
                "Intended use: a finished scene plate for a tabletop role-playing game shown on a television, one wide picture of the place."
            }
            Self::Background => {
                "Intended use: a full-bleed background picture behind a game interface; environment only, with a calm area in the middle for panels."
            }
            Self::Portrait => {
                "Intended use: a character portrait, one figure from the chest up, centred, in a plain setting."
            }
        }
    }
}

/// A picture shape as two whole numbers, such as 16:9.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Aspect {
    pub width: u32,
    pub height: u32,
}

impl Aspect {
    pub const WIDE: Self = Self {
        width: 16,
        height: 9,
    };

    /// The most either number may be.
    const MAXIMUM_PART: u32 = 64;
    /// The widest or tallest shape: this many to one.
    const MAXIMUM_RATIO: u32 = 4;

    /// `W:H` with both whole numbers from 1 to 64, and neither side more than four times the other.
    pub fn parse(text: &str) -> Option<Self> {
        let (width, height) = text.split_once(':')?;
        let width: u32 = width.parse().ok()?;
        let height: u32 = height.parse().ok()?;
        let in_range = |part: u32| (1..=Self::MAXIMUM_PART).contains(&part);
        if !in_range(width) || !in_range(height) {
            return None;
        }
        if width > height.saturating_mul(Self::MAXIMUM_RATIO)
            || height > width.saturating_mul(Self::MAXIMUM_RATIO)
        {
            return None;
        }
        Some(Self { width, height })
    }

    pub fn label(self) -> String {
        format!("{}:{}", self.width, self.height)
    }

    /// The size asked of the backend: the long side is `REQUESTED_LONG_SIDE_PX`.
    pub fn requested_size(self) -> (u32, u32) {
        let (long, short) = if self.width >= self.height {
            (self.width, self.height)
        } else {
            (self.height, self.width)
        };
        let short_px = (u64::from(REQUESTED_LONG_SIDE_PX) * u64::from(short) + u64::from(long) / 2)
            / u64::from(long);
        let short_px = u32::try_from(short_px).unwrap_or(REQUESTED_LONG_SIDE_PX);
        if self.width >= self.height {
            (REQUESTED_LONG_SIDE_PX, short_px)
        } else {
            (short_px, REQUESTED_LONG_SIDE_PX)
        }
    }
}

impl fmt::Display for Aspect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.width, self.height)
    }
}

/// Why a request is not one a generation can start from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestProblem {
    EmptyPrompt,
    PromptTooLong { chars: usize },
    BadName { name: String },
    BadPurpose { text: String },
    BadAspect { text: String },
}

impl fmt::Display for RequestProblem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPrompt => formatter.write_str("the prompt is empty"),
            Self::PromptTooLong { chars } => write!(
                formatter,
                "the prompt is {chars} characters; at most {MAXIMUM_PROMPT_CHARS}"
            ),
            Self::BadName { name } => write!(
                formatter,
                "'{name}' is not a picture name: use 1 to {MAXIMUM_NAME_CHARS} letters, digits, '-' or '_', with no extension"
            ),
            Self::BadPurpose { text } => {
                write!(
                    formatter,
                    "'{text}' is not a purpose; use plate, background or portrait"
                )
            }
            Self::BadAspect { text } => write!(
                formatter,
                "'{text}' is not a shape; use W:H such as 16:9, with neither side more than four times the other"
            ),
        }
    }
}

fn plain_name(name: &str) -> bool {
    let count = name.chars().count();
    (1..=MAXIMUM_NAME_CHARS).contains(&count)
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

/// One checked request: a prompt, the name the picture is stored under (without its extension),
/// what it is for and its shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRequest {
    pub prompt: String,
    pub name: String,
    pub purpose: Purpose,
    pub aspect: Aspect,
}

impl ImageRequest {
    /// Checks the words of a request. `purpose` and `aspect` are the text the caller gave, if any.
    pub fn new(
        prompt: &str,
        name: &str,
        purpose: Option<&str>,
        aspect: Option<&str>,
    ) -> Result<Self, RequestProblem> {
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return Err(RequestProblem::EmptyPrompt);
        }
        let chars = prompt.chars().count();
        if chars > MAXIMUM_PROMPT_CHARS {
            return Err(RequestProblem::PromptTooLong { chars });
        }
        if !plain_name(name) {
            return Err(RequestProblem::BadName {
                name: name.to_owned(),
            });
        }
        let purpose = match purpose {
            None => Purpose::Background,
            Some(text) => Purpose::parse(text).ok_or_else(|| RequestProblem::BadPurpose {
                text: text.to_owned(),
            })?,
        };
        let aspect = match aspect {
            None => purpose.default_aspect(),
            Some(text) => Aspect::parse(text).ok_or_else(|| RequestProblem::BadAspect {
                text: text.to_owned(),
            })?,
        };
        Ok(Self {
            prompt: prompt.to_owned(),
            name: name.to_owned(),
            purpose,
            aspect,
        })
    }

    /// The text the backend is given: the prompt, then the purpose's direction.
    pub fn sent_text(&self) -> String {
        format!("{}\n\n{}", self.prompt, self.purpose.direction())
    }
}

/// What a backend is asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendRequest {
    pub text: String,
    pub width_px: u32,
    pub height_px: u32,
}

/// A picture a backend delivered, with what it can say about how it was made.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedImage {
    pub bytes: Vec<u8>,
    pub mime: String,
    pub width_px: u32,
    pub height_px: u32,
    pub sha256_hex: String,
    pub duration: Duration,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// Why a backend gave no picture. Each case says what to do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendError {
    /// The backend is not switched on for this host.
    NotConfigured(String),
    /// The Codex executable cannot be found or run.
    CodexMissing(String),
    /// There is no signed-in Codex home to run with.
    NotSignedIn(String),
    /// The deadline ended the attempt.
    Timeout,
    /// The attempt was interrupted after work may have started.
    Interrupted,
    /// The account's usage limit stopped it.
    UsageLimited,
    /// The service refused or failed the request.
    Refused(String),
    /// Anything else, with a reason.
    Failed(String),
}

/// A closed key for each way a generation does not finish, for audit lines and typed answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalCause {
    BadRequest,
    NotConfigured,
    CodexMissing,
    NotSignedIn,
    NameBusy,
    Timeout,
    Interrupted,
    UsageLimited,
    Refused,
    BadImage,
    StoreFailed,
    Failed,
}

impl RefusalCause {
    pub fn key(self) -> &'static str {
        match self {
            Self::BadRequest => "bad_request",
            Self::NotConfigured => "not_configured",
            Self::CodexMissing => "codex_missing",
            Self::NotSignedIn => "not_signed_in",
            Self::NameBusy => "name_busy",
            Self::Timeout => "timeout",
            Self::Interrupted => "interrupted",
            Self::UsageLimited => "usage_limited",
            Self::Refused => "refused",
            Self::BadImage => "bad_image",
            Self::StoreFailed => "store_failed",
            Self::Failed => "failed",
        }
    }

    /// The status a refusal of this cause answers with.
    pub fn status(self) -> u16 {
        match self {
            Self::BadRequest => 400,
            Self::NameBusy => 409,
            Self::NotConfigured | Self::CodexMissing | Self::NotSignedIn | Self::UsageLimited => {
                503
            }
            Self::Timeout => 504,
            Self::Interrupted | Self::Refused | Self::BadImage => 502,
            Self::StoreFailed | Self::Failed => 500,
        }
    }
}

impl BackendError {
    pub fn cause(&self) -> RefusalCause {
        match self {
            Self::NotConfigured(_) => RefusalCause::NotConfigured,
            Self::CodexMissing(_) => RefusalCause::CodexMissing,
            Self::NotSignedIn(_) => RefusalCause::NotSignedIn,
            Self::Timeout => RefusalCause::Timeout,
            Self::Interrupted => RefusalCause::Interrupted,
            Self::UsageLimited => RefusalCause::UsageLimited,
            Self::Refused(_) => RefusalCause::Refused,
            Self::Failed(_) => RefusalCause::Failed,
        }
    }

    /// The message a person reads, with the step to take when there is one.
    pub fn message(&self) -> String {
        match self {
            Self::NotConfigured(reason) | Self::CodexMissing(reason) | Self::NotSignedIn(reason) => {
                reason.clone()
            }
            Self::Timeout => "the generation did not finish before its deadline; the provider may still have done the work, and nothing was retried".to_owned(),
            Self::Interrupted => "the generation was interrupted after work may have started; nothing was retried".to_owned(),
            Self::UsageLimited => "the Codex account's usage limit stopped the generation; try again when the limit resets".to_owned(),
            Self::Refused(detail) | Self::Failed(detail) => detail.clone(),
        }
    }
}

/// How a job stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Done,
    Refused,
}

/// What is known about how a stored picture was made. The prompt is here, in full, because this is
/// the record that goes next to the picture; it is never in an audit line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    pub backend: String,
    pub purpose: Purpose,
    pub aspect: String,
    pub requested_width_px: u32,
    pub requested_height_px: u32,
    pub prompt: String,
    pub sent_text: String,
    pub duration_ms: u64,
    pub deadline_secs: u64,
    pub width_px: u32,
    pub height_px: u32,
    pub mime: String,
    pub bytes: u64,
    pub sha256: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// The file in the media library.
    pub media_file: String,
    /// What the file is, when the host stored it as it came.
    pub note: String,
}

/// One job as the status answer and the final answer show it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobView {
    pub job: u64,
    pub name: String,
    pub state: JobState,
    pub elapsed_ms: u64,
    pub cause: Option<RefusalCause>,
    pub message: Option<String>,
    pub provenance: Option<Provenance>,
    /// Where the provenance file was written, when the host was told where.
    pub provenance_file: Option<String>,
}

/// The body of `POST /dev/media/generate`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerateBody {
    pub prompt: String,
    pub name: String,
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub aspect: Option<String>,
    #[serde(default)]
    pub wait: bool,
}

/// What every answer of the route is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum GenerateReply {
    /// The generation runs on; `media generate status` shows it.
    Started {
        job: u64,
        name: String,
        deadline_secs: u64,
    },
    /// The generation ended: the job says how.
    Finished {
        job: Box<JobView>,
    },
    Refused {
        job: Option<u64>,
        cause: RefusalCause,
        message: String,
    },
    Jobs {
        jobs: Vec<JobView>,
    },
}

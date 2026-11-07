//! What a request is for: the `PurposeSpec` of every text and image purpose (O.5, J.1).
//!
//! A purpose says how long a request may take (its deadline), how big its answer may be, how often
//! a failure is retried, which queue class it waits in, what one call costs and what the table does
//! when it fails (the fallback). Adding a purpose is one entry in `PurposeTable::built_in` and
//! nothing else: the broker's conformance test fails a purpose without a deadline, a class or a
//! fallback, and the type makes the last two impossible to leave out. It never queues or runs a
//! request: `scheduler` does.
//!
//! The deadlines are candidates from the plan's observed latencies (H.5: the text calls behind
//! today's play take 15 to 45 s through Codex; a plate takes 90 to 240 s) and are to be tuned from
//! measurement. Every built-in price is `Unpriced`: the lanes in use are record-only (no spend is
//! known), and a priced lane sets its own `Price`.

use std::collections::BTreeMap;
use std::fmt;

use df_table::element::NameError;
use df_table::ports::PurposeName;
use serde::{Deserialize, Serialize};

use super::limits::{
    DEFAULT_RETRY, MAXIMUM_IMAGE_ANSWER_EDGE_PIXELS, MAXIMUM_TEXT_ANSWER_CHARACTERS, SINGLE_RETRY,
    image_deadline_ms, text_deadline_ms,
};

/// Whether a purpose goes to the text lane or the image lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Text,
    Image,
}

impl ResourceKind {
    pub const ALL: &'static [Self] = &[Self::Text, Self::Image];

    pub fn key(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.key() == key)
    }
}

/// The priority class a request waits in. The order of `ALL` is the order they are served: a
/// higher class is always claimed first and may preempt a lower one (J.1, H.7).
///
/// `Live` joins the plan's `player_blocking` (interpret, narrate) and `scene_build` classes: both
/// block the table and neither may be dropped. `Plate` is the scene's plates, `Lookahead` the
/// builds of the next scene that SB-LA starts early, and `Background` the plan's `prefetch`
/// (portraits, item views).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueClass {
    Live,
    Plate,
    Lookahead,
    Background,
}

impl QueueClass {
    /// Highest priority first.
    pub const ALL: &'static [Self] = &[Self::Live, Self::Plate, Self::Lookahead, Self::Background];

    pub fn key(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Plate => "plate",
            Self::Lookahead => "lookahead",
            Self::Background => "background",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|class| class.key() == key)
    }

    /// 0 is the highest priority.
    pub fn rank(self) -> usize {
        match self {
            Self::Live => 0,
            Self::Plate => 1,
            Self::Lookahead => 2,
            Self::Background => 3,
        }
    }

    /// Whether a full queue of this class may drop its oldest waiting request. Live never does:
    /// it refuses the new request instead (H.7: a class that cannot drop surfaces as a status).
    pub fn drops_oldest_when_full(self) -> bool {
        !matches!(self, Self::Live)
    }
}

/// What the table does when a purpose fails, closed (H.4). The broker reports the failure; the
/// owner applies the fallback, and no failure withholds a reward or stalls a scene (pattern 11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fallback {
    /// Offer the legal choices as buttons: "I could not read that. Pick one:" (C.7).
    ButtonsFromLegalChoices,
    /// Show the structured readout of the committed outcome (H.15).
    StructuredReadout,
    /// Play the scene on its place card with the authored or last background (H.4).
    PlaceCard,
    /// Keep the last plate of the same scene on screen (H.4).
    HoldLastPlate,
    /// Play on the establishing card until the build lands (K.2).
    EstablishingCard,
    /// Show text where the picture would be (a portrait, an item view).
    TextInPlaceOfPicture,
    /// Nothing is shown for it: the work was an optimisation (look-ahead, prefetch).
    Nothing,
}

impl Fallback {
    pub const ALL: &'static [Self] = &[
        Self::ButtonsFromLegalChoices,
        Self::StructuredReadout,
        Self::PlaceCard,
        Self::HoldLastPlate,
        Self::EstablishingCard,
        Self::TextInPlaceOfPicture,
        Self::Nothing,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::ButtonsFromLegalChoices => "buttons_from_legal_choices",
            Self::StructuredReadout => "structured_readout",
            Self::PlaceCard => "place_card",
            Self::HoldLastPlate => "hold_last_plate",
            Self::EstablishingCard => "establishing_card",
            Self::TextInPlaceOfPicture => "text_in_place_of_picture",
            Self::Nothing => "nothing",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|fallback| fallback.key() == key)
    }
}

/// How large an answer may be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum SizeRule {
    TextCharacters { maximum: u32 },
    ImagePixels { width: u32, height: u32 },
}

impl SizeRule {
    fn kind(self) -> ResourceKind {
        match self {
            Self::TextCharacters { .. } => ResourceKind::Text,
            Self::ImagePixels { .. } => ResourceKind::Image,
        }
    }

    fn is_empty(self) -> bool {
        match self {
            Self::TextCharacters { maximum } => maximum == 0,
            Self::ImagePixels { width, height } => width == 0 || height == 0,
        }
    }
}

/// How a failed call is retried (H.8: two retries, with backoff and jitter, never for a refusal).
/// The backoff doubles from `initial_backoff_ms` up to `maximum_backoff_ms`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub maximum_retries: u8,
    pub initial_backoff_ms: u64,
    pub maximum_backoff_ms: u64,
}

impl RetryPolicy {
    /// The wait before retry number `retry` (the first retry is 1), before jitter. It doubles and
    /// stops at the maximum, and is zero for retry zero.
    pub fn backoff_ms(self, retry: u8) -> u64 {
        let Some(doublings) = retry.checked_sub(1) else {
            return 0;
        };
        let scaled = self
            .initial_backoff_ms
            .checked_shl(u32::from(doublings))
            .filter(|scaled| scaled >> u32::from(doublings) == self.initial_backoff_ms)
            .unwrap_or(u64::MAX);
        scaled.min(self.maximum_backoff_ms)
    }
}

/// What one provider call costs. A call that was sent is never refunded, and nothing here ever
/// refuses a request (coding style section 8, pattern 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "price", rename_all = "snake_case")]
pub enum Price {
    Priced {
        micro_dollars: u64,
    },
    /// The lane records no spend; the ledger still counts the calls (H.8, D18).
    Unpriced,
}

/// Everything the broker knows about one purpose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurposeSpec {
    name: PurposeName,
    kind: ResourceKind,
    deadline_ms: u64,
    size: SizeRule,
    retry: RetryPolicy,
    class: QueueClass,
    price: Price,
    fallback: Fallback,
}

/// Why a purpose could not be built or the table could not be assembled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PurposeError {
    InvalidName(NameError),
    /// A deadline of zero would fail every request before it started.
    ZeroDeadline {
        purpose: PurposeName,
    },
    /// The size rule is for the other kind of resource, or allows nothing.
    SizeRuleMismatch {
        purpose: PurposeName,
    },
    /// A retry backoff of zero, or an initial backoff above the maximum.
    InvalidBackoff {
        purpose: PurposeName,
    },
    DuplicatePurpose {
        purpose: PurposeName,
    },
}

impl fmt::Display for PurposeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(error) => write!(formatter, "a purpose name is invalid: {error}"),
            Self::ZeroDeadline { purpose } => {
                write!(formatter, "the purpose {purpose} has a deadline of zero")
            }
            Self::SizeRuleMismatch { purpose } => write!(
                formatter,
                "the size rule of {purpose} does not fit its kind or allows nothing"
            ),
            Self::InvalidBackoff { purpose } => {
                write!(formatter, "the retry backoff of {purpose} is invalid")
            }
            Self::DuplicatePurpose { purpose } => {
                write!(formatter, "the purpose {purpose} is listed twice")
            }
        }
    }
}

impl std::error::Error for PurposeError {}

/// The facts of a purpose besides its name, so a spec is built from one value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PurposeFacts {
    pub kind: ResourceKind,
    pub deadline_ms: u64,
    pub size: SizeRule,
    pub retry: RetryPolicy,
    pub class: QueueClass,
    pub price: Price,
    pub fallback: Fallback,
}

impl PurposeSpec {
    pub fn new(name: &str, facts: PurposeFacts) -> Result<Self, PurposeError> {
        let name = PurposeName::new(name).map_err(PurposeError::InvalidName)?;
        let PurposeFacts {
            kind,
            deadline_ms,
            size,
            retry,
            class,
            price,
            fallback,
        } = facts;
        if deadline_ms == 0 {
            return Err(PurposeError::ZeroDeadline { purpose: name });
        }
        if size.kind() != kind || size.is_empty() {
            return Err(PurposeError::SizeRuleMismatch { purpose: name });
        }
        if retry.initial_backoff_ms == 0 || retry.initial_backoff_ms > retry.maximum_backoff_ms {
            return Err(PurposeError::InvalidBackoff { purpose: name });
        }
        Ok(Self {
            name,
            kind,
            deadline_ms,
            size,
            retry,
            class,
            price,
            fallback,
        })
    }

    pub fn name(&self) -> &PurposeName {
        &self.name
    }

    pub fn kind(&self) -> ResourceKind {
        self.kind
    }

    pub fn deadline_ms(&self) -> u64 {
        self.deadline_ms
    }

    pub fn size(&self) -> SizeRule {
        self.size
    }

    pub fn retry(&self) -> RetryPolicy {
        self.retry
    }

    pub fn class(&self) -> QueueClass {
        self.class
    }

    pub fn price(&self) -> Price {
        self.price
    }

    pub fn fallback(&self) -> Fallback {
        self.fallback
    }
}

const TEXT_CHARACTERS: SizeRule = SizeRule::TextCharacters {
    maximum: MAXIMUM_TEXT_ANSWER_CHARACTERS,
};
const SHORT_TEXT_CHARACTERS: SizeRule = TEXT_CHARACTERS;
const SCENE_PICTURE: SizeRule = SizeRule::ImagePixels {
    width: MAXIMUM_IMAGE_ANSWER_EDGE_PIXELS,
    height: MAXIMUM_IMAGE_ANSWER_EDGE_PIXELS,
};
const PORTRAIT: SizeRule = SCENE_PICTURE;

/// The purposes the broker serves, by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurposeTable {
    purposes: BTreeMap<PurposeName, PurposeSpec>,
}

impl PurposeTable {
    pub fn new(specs: Vec<PurposeSpec>) -> Result<Self, PurposeError> {
        let mut purposes = BTreeMap::new();
        for spec in specs {
            let name = spec.name.clone();
            if purposes.insert(name.clone(), spec).is_some() {
                return Err(PurposeError::DuplicatePurpose { purpose: name });
            }
        }
        Ok(Self { purposes })
    }

    /// The purposes H.5 lists as model calls, plus the plan's image purposes, each with the class
    /// of H.7 and the fallback of H.4.
    pub fn built_in() -> Result<Self, PurposeError> {
        let text_deadline = text_deadline_ms();
        let image_deadline = image_deadline_ms();
        let text = |deadline_ms, size, retry, class, fallback| PurposeFacts {
            kind: ResourceKind::Text,
            deadline_ms,
            size,
            retry,
            class,
            price: Price::Unpriced,
            fallback,
        };
        let image = |deadline_ms, size, class, fallback| PurposeFacts {
            kind: ResourceKind::Image,
            deadline_ms,
            size,
            retry: DEFAULT_RETRY,
            class,
            price: Price::Unpriced,
            fallback,
        };
        let live_text = |fallback| {
            text(
                text_deadline,
                TEXT_CHARACTERS,
                DEFAULT_RETRY,
                QueueClass::Live,
                fallback,
            )
        };
        Self::new(vec![
            PurposeSpec::new(
                "interpret",
                text(
                    text_deadline,
                    SHORT_TEXT_CHARACTERS,
                    SINGLE_RETRY,
                    QueueClass::Live,
                    Fallback::ButtonsFromLegalChoices,
                ),
            )?,
            PurposeSpec::new("narrate", live_text(Fallback::StructuredReadout))?,
            PurposeSpec::new("describe", live_text(Fallback::PlaceCard))?,
            PurposeSpec::new("dress", live_text(Fallback::PlaceCard))?,
            PurposeSpec::new("staging_text", live_text(Fallback::PlaceCard))?,
            PurposeSpec::new(
                "ruling_suggestion",
                live_text(Fallback::ButtonsFromLegalChoices),
            )?,
            PurposeSpec::new(
                "background",
                image(
                    image_deadline,
                    SCENE_PICTURE,
                    QueueClass::Live,
                    Fallback::EstablishingCard,
                ),
            )?,
            PurposeSpec::new(
                "plate",
                image(
                    image_deadline,
                    SCENE_PICTURE,
                    QueueClass::Plate,
                    Fallback::HoldLastPlate,
                ),
            )?,
            PurposeSpec::new(
                "lookahead_describe",
                text(
                    text_deadline,
                    TEXT_CHARACTERS,
                    DEFAULT_RETRY,
                    QueueClass::Lookahead,
                    Fallback::Nothing,
                ),
            )?,
            PurposeSpec::new(
                "lookahead_background",
                image(
                    image_deadline,
                    SCENE_PICTURE,
                    QueueClass::Lookahead,
                    Fallback::Nothing,
                ),
            )?,
            PurposeSpec::new(
                "portrait",
                image(
                    image_deadline,
                    PORTRAIT,
                    QueueClass::Background,
                    Fallback::TextInPlaceOfPicture,
                ),
            )?,
            PurposeSpec::new(
                "item_view",
                image(
                    image_deadline,
                    PORTRAIT,
                    QueueClass::Background,
                    Fallback::TextInPlaceOfPicture,
                ),
            )?,
        ])
    }

    pub fn get(&self, name: &PurposeName) -> Option<&PurposeSpec> {
        self.purposes.get(name)
    }

    /// Every purpose, in name order.
    pub fn all(&self) -> impl Iterator<Item = &PurposeSpec> {
        self.purposes.values()
    }
}

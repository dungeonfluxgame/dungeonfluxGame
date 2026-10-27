//! The cost engine: what every billed provider call cost, in exact micro-dollars, who caused it,
//! and the totals of the run and of this server session.
//!
//! # The contract
//!
//! * **One choke point.** A paid call is metered through the hub's [`SpendLedger`]:
//!   [`SpendLedger::begin_call`] right before the request is sent returns a [`BilledCall`], which
//!   must be settled: [`BilledCall::settle`] with the usage the provider reported (or the
//!   quantity the server counted, flagged estimated), or [`BilledCall::not_billed`] when the
//!   provider refused before doing any billable work. A call dropped unsettled (a timeout, a
//!   transport failure, a cancelled future) is still recorded, with its estimate flagged
//!   `unknown`: a call whose outcome is unknown may have been billed.
//! * **Typed.** [`Provider`], [`Category`], [`Cause`] and [`UsageSource`] are closed enums; the
//!   cause is a required part of every [`CallSpec`]. The run and the session belong to the
//!   ledger (one per server run), so a call site cannot attach a call to the wrong run.
//! * **Exact money.** Every amount is whole micro-dollars (`u64`). One rounding rule: each
//!   priced line is `quantity * usd / per`, computed exactly (`df_commerce::tariff`), added to
//!   the line's carried remainder and rounded half up to a whole micro-dollar; what is left
//!   over is carried to the next call of the same provider, model and category
//!   ([`prices::price_carrying`]), so a run of sub-micro calls is off by under half a
//!   micro-dollar in all, not per call. An event's total is the sum of its rounded lines and
//!   every total is a sum of event lines, so each breakdown sums to its total exactly.
//! * **Prices are data.** `content/pricing.json` (schema `dungeonflux.pricing/1`), or the file
//!   `DF_GAMEPLAY_PRICING` names, holds one dated, sourced rate per provider, model, category
//!   and context tier. A malformed file is refused whole and loudly (the `COST pricing` startup line);
//!   a missing file or a missing entry leaves those lines unpriced. Pricing never blocks a call:
//!   there is no spend cap of any kind.
//! * **Counted once.** A [`BilledCall`] is consumed when it is settled, so a live call is
//!   recorded once. The book numbers events as it records them (`<session>-<n>`, increasing), so
//!   the run log holds each session's ids in increasing order; on replay a line whose number is
//!   not above its session's high-water mark is a repeat and is counted once. The dedupe state
//!   is one number per session.
//! * **Per run.** Events are appended to `development/runs/<database>/cost.jsonl` (one JSON
//!   object per line, [`CostEvent`]) by an owned writer thread, so the event path never blocks
//!   on disk; the run totals are rebuilt from that file when the server starts again. A log that
//!   cannot be read or written leaves the book open in memory, with the reason in `dev cost`.
//!   [`close_run`] drains the writer when the server shuts down.
//!
//! # The `COST` line
//!
//! Every event writes one audit line:
//!
//! ```text
//! COST call=<id> provider=<provider> model=<model> cause=<cause> detail=<detail>
//!   source=<reported|estimated|unknown> usd=<d.dddddd>
//!   lines=<category>:<quantity>@<d.dddddd|unpriced>,... run_usd=<d.dddddd> session_usd=<d.dddddd>
//! ```
//!
//! (one line; wrapped here). Words never contain a space.

use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use df_providers::{
    GeneratedImage, ImageBackground, ImageEdit, ImageEditModel, ImageQuality, ImageUsage,
    NativeHttpRefusal, ProviderFailureClass, SpeechAdapterReason, TextTokenUsage,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::imagery::{ImageFailure, ImageKind, ImagePriority, ImageRoute};
use super::resources::SpendLedger;
use crate::voice_line::{NARRATOR_ROLE, VoiceLine};

pub mod port;
mod prices;
mod provider_meter;
mod specs;

use port::{MeterObserver, MeterStart, MeteredCall, RECENT_LIMIT};
#[cfg(test)]
use prices::PricingError;
use prices::{Carry, price_carrying};
pub(in crate::gameplay) use prices::{PriceTable, PricingStatus, load_prices};
pub(in crate::gameplay) use provider_meter::LedgerMeter;
#[cfg(test)]
pub(in crate::gameplay) use provider_meter::test_meter;
pub(in crate::gameplay) use specs::{
    ambience_call, embedding_call, music_error_may_be_billed, music_layer_call, segment_call,
    segment_error_may_be_billed, settle_counted, settle_failure, stt_call, video_call, world_call,
    world_error_may_be_billed,
};

/// Events recorded before the book is opened are held up to this many; more are counted lost.
const PENDING_LIMIT: usize = 4096;
/// Lines queued for the writer thread; a full queue keeps the totals and counts a write failure.
const WRITE_QUEUE: usize = 1024;
/// How long a flush waits for the writer.
#[cfg(test)]
const FLUSH_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
/// The top spenders the report lists.
const TOP_SPENDERS: usize = 10;
/// The largest bill a `dev cost compare` accepts.
pub(in crate::gameplay) const BILL_LIMIT_BYTES: usize = 256 * 1024;

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(in crate::gameplay) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Who bills the call.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub(in crate::gameplay) enum Provider {
    #[serde(rename = "openai")]
    OpenAi,
    #[serde(rename = "codex_subscription")]
    CodexSubscription,
    #[serde(rename = "elevenlabs")]
    ElevenLabs,
    #[serde(rename = "fal")]
    Fal,
    #[serde(rename = "bfl")]
    Bfl,
    #[serde(rename = "worldlabs")]
    WorldLabs,
    #[serde(rename = "typesafe")]
    TypeSafe,
}

impl Provider {
    pub(in crate::gameplay) const ALL: [Self; 7] = [
        Self::OpenAi,
        Self::CodexSubscription,
        Self::ElevenLabs,
        Self::Fal,
        Self::Bfl,
        Self::WorldLabs,
        Self::TypeSafe,
    ];

    pub(in crate::gameplay) const fn label(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::CodexSubscription => "codex_subscription",
            Self::ElevenLabs => "elevenlabs",
            Self::Fal => "fal",
            Self::Bfl => "bfl",
            Self::WorldLabs => "worldlabs",
            Self::TypeSafe => "typesafe",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|provider| provider.label() == text)
    }
}

/// What one priced line counts.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::gameplay) enum Category {
    /// Uncached text input tokens.
    TextInput,
    TextCachedInput,
    /// Input tokens written to the prompt cache.
    TextCacheWrite,
    /// Output tokens, reasoning included.
    TextOutput,
    /// Image input tokens (reference images).
    ImageInput,
    ImageCachedInput,
    /// Image output tokens.
    ImageOutput,
    Characters,
    AudioSeconds,
    VideoSeconds,
    Images,
    Megapixels,
    /// Supplier credits (BFL, World Labs).
    Credits,
    Calls,
}

impl Category {
    pub(in crate::gameplay) const ALL: [Self; 14] = [
        Self::TextInput,
        Self::TextCachedInput,
        Self::TextCacheWrite,
        Self::TextOutput,
        Self::ImageInput,
        Self::ImageCachedInput,
        Self::ImageOutput,
        Self::Characters,
        Self::AudioSeconds,
        Self::VideoSeconds,
        Self::Images,
        Self::Megapixels,
        Self::Credits,
        Self::Calls,
    ];

    pub(in crate::gameplay) const fn label(self) -> &'static str {
        match self {
            Self::TextInput => "text_input",
            Self::TextCachedInput => "text_cached_input",
            Self::TextCacheWrite => "text_cache_write",
            Self::TextOutput => "text_output",
            Self::ImageInput => "image_input",
            Self::ImageCachedInput => "image_cached_input",
            Self::ImageOutput => "image_output",
            Self::Characters => "characters",
            Self::AudioSeconds => "audio_seconds",
            Self::VideoSeconds => "video_seconds",
            Self::Images => "images",
            Self::Megapixels => "megapixels",
            Self::Credits => "credits",
            Self::Calls => "calls",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.label() == text)
    }
}

/// What in the game asked for the call. The [`CallSpec::detail`] narrows it (the storyteller's
/// purpose, the speaking role, the picture's profile and size).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::gameplay) enum Cause {
    /// A composed film still of a story scene (the plate cadence).
    ScenePlate,
    /// An existing plate edited for a lasting change.
    PlateEdit,
    /// The backdrop re-framed and re-lit.
    SceneRefresh,
    /// A person's or creature's canonical picture.
    CharacterPortrait,
    /// A place's canonical empty-room picture.
    PlacePortrait,
    /// A cut-out with another expression.
    CastExpression,
    /// A close-up of one object.
    ItemCloseUp,
    /// A fighter's home-angle guard or fallen frame for the character clips.
    ClipFrame,
    /// A storyteller text call; the detail is its purpose (`npc_reply`, `place_creation`, ...).
    Storyteller,
    /// A line without a named speaker, spoken in the narrator's voice.
    NarrationVoice,
    /// A named speaker's line (an NPC reply, a hero's cry); the detail is the actor id, else
    /// the name.
    CharacterVoice,
    /// A player's recorded clip turned into words.
    Transcription,
    /// A story-memory text embedded for recall.
    Embedding,
    /// A Jev judgement of a moment, an investigation or a creator's appearance.
    Judgement,
    /// A person found in a scene plate or a figure mask cut.
    Segmentation,
    /// A vision check of a generated picture, or of who stands where in a plate.
    PictureJudgement,
    /// A speaker's head found in a scene plate.
    HeadLocation,
    /// One music layer of a place's set.
    PlaceMusic,
    /// A place's ambience bed.
    PlaceAmbience,
    /// A looping home video clip.
    HomeVideo,
    /// A fighter's character clip.
    CharacterClip,
    /// A place's explorable 3D world.
    SplatWorld,
}

impl Cause {
    pub(in crate::gameplay) const ALL: [Self; 22] = [
        Self::ScenePlate,
        Self::PlateEdit,
        Self::SceneRefresh,
        Self::CharacterPortrait,
        Self::PlacePortrait,
        Self::CastExpression,
        Self::ItemCloseUp,
        Self::ClipFrame,
        Self::Storyteller,
        Self::NarrationVoice,
        Self::CharacterVoice,
        Self::Transcription,
        Self::Embedding,
        Self::Judgement,
        Self::Segmentation,
        Self::PictureJudgement,
        Self::HeadLocation,
        Self::PlaceMusic,
        Self::PlaceAmbience,
        Self::HomeVideo,
        Self::CharacterClip,
        Self::SplatWorld,
    ];

    fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.label() == text)
    }

    pub(in crate::gameplay) const fn label(self) -> &'static str {
        match self {
            Self::ScenePlate => "scene_plate",
            Self::PlateEdit => "plate_edit",
            Self::SceneRefresh => "scene_refresh",
            Self::CharacterPortrait => "character_portrait",
            Self::PlacePortrait => "place_portrait",
            Self::CastExpression => "cast_expression",
            Self::ItemCloseUp => "item_close_up",
            Self::ClipFrame => "clip_frame",
            Self::Storyteller => "storyteller",
            Self::NarrationVoice => "narration_voice",
            Self::CharacterVoice => "character_voice",
            Self::Transcription => "transcription",
            Self::Embedding => "embedding",
            Self::Judgement => "judgement",
            Self::Segmentation => "segmentation",
            Self::PictureJudgement => "picture_judgement",
            Self::HeadLocation => "head_location",
            Self::PlaceMusic => "place_music",
            Self::PlaceAmbience => "place_ambience",
            Self::HomeVideo => "home_video",
            Self::CharacterClip => "character_clip",
            Self::SplatWorld => "splat_world",
        }
    }
}

/// Where a quantity came from.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::gameplay) enum UsageSource {
    /// The provider's own usage report.
    Reported,
    /// What the server counted or estimated (characters sent, one call).
    Estimated,
    /// The call was sent and its outcome is unknown; the quantities are the estimate.
    Unknown,
}

impl UsageSource {
    const fn label(self) -> &'static str {
        match self {
            Self::Reported => "reported",
            Self::Estimated => "estimated",
            Self::Unknown => "unknown",
        }
    }
}

/// The quantities one call used, and where they came from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct Usage {
    source: UsageSource,
    quantities: Vec<(Category, u64)>,
    /// Whether the call produced anything the game could use. A call billed for an answer that
    /// was no use (a reply that did not parse) is a loss, not a purchase; the money core keeps
    /// the customer out of it.
    usable: bool,
}

impl Usage {
    /// Quantities from the provider's own usage report.
    pub(in crate::gameplay) fn reported(quantities: Vec<(Category, u64)>) -> Self {
        Self {
            source: UsageSource::Reported,
            quantities,
            usable: true,
        }
    }

    /// Quantities the server counted (flagged estimated in every report).
    pub(in crate::gameplay) fn estimated(quantities: Vec<(Category, u64)>) -> Self {
        Self {
            source: UsageSource::Estimated,
            quantities,
            usable: true,
        }
    }

    /// The same usage for a call whose output was no use.
    #[must_use]
    pub(in crate::gameplay) fn unusable(mut self) -> Self {
        self.usable = false;
        self
    }
}

/// What a billed call is: who bills it, the model, and what in the game caused it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct CallSpec {
    pub(in crate::gameplay) provider: Provider,
    pub(in crate::gameplay) model: String,
    pub(in crate::gameplay) cause: Cause,
    /// A short label narrowing the cause; reduced to `[a-z0-9_.:/-]` when recorded.
    pub(in crate::gameplay) detail: String,
    /// The most the call can use, a conservative figure for every category its route declares.
    /// When set it is the estimate recorded for a call of unknown outcome, and the units a
    /// prepaid admission holds; empty, the estimate is the caller's own.
    pub(in crate::gameplay) maxima: Vec<(Category, u64)>,
    /// Calls made after this one succeeds, each recorded as an event of its own when this one
    /// settles (a picture's background removal). Not recorded for a call of unknown outcome.
    pub(in crate::gameplay) follow_ons: Vec<FollowOn>,
}

/// A call made after another one, with the units it used.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct FollowOn {
    pub(in crate::gameplay) spec: CallSpec,
    pub(in crate::gameplay) quantities: Vec<(Category, u64)>,
}

impl CallSpec {
    /// A call with no stated maxima.
    pub(in crate::gameplay) fn new(
        provider: Provider,
        model: impl Into<String>,
        cause: Cause,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            provider,
            model: model.into(),
            cause,
            detail: detail.into(),
            maxima: Vec::new(),
            follow_ons: Vec::new(),
        }
    }

    /// The same call with `maxima` as its conservative per-category figures.
    #[must_use]
    pub(in crate::gameplay) fn with_maxima(mut self, maxima: Vec<(Category, u64)>) -> Self {
        self.maxima = maxima;
        self
    }

    /// The same call made again for `reason` (a corrected regeneration): the detail says so.
    pub(in crate::gameplay) fn again(&self, reason: &str) -> Self {
        Self {
            detail: format!("{}/{reason}", self.detail),
            ..self.clone()
        }
    }
}

/// One line of a recorded event; `micro_usd` is `None` when the table has no price for it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::gameplay) struct CostLine {
    pub(in crate::gameplay) category: Category,
    pub(in crate::gameplay) quantity: u64,
    pub(in crate::gameplay) micro_usd: Option<u64>,
}

/// One billed call, as recorded and persisted (one line of `cost.jsonl`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::gameplay) struct CostEvent {
    /// The idempotency key: an id is counted once per run.
    pub(in crate::gameplay) call_id: String,
    pub(in crate::gameplay) at_ms: u64,
    pub(in crate::gameplay) run: String,
    pub(in crate::gameplay) session: String,
    pub(in crate::gameplay) provider: Provider,
    pub(in crate::gameplay) model: String,
    pub(in crate::gameplay) cause: Cause,
    pub(in crate::gameplay) detail: String,
    pub(in crate::gameplay) source: UsageSource,
    pub(in crate::gameplay) lines: Vec<CostLine>,
    /// The sum of the priced lines.
    pub(in crate::gameplay) micro_usd: u64,
    /// False for a call that was billed and produced nothing the game could use. Absent in the
    /// log (and read as true) for every other call.
    #[serde(default = "usable_by_default", skip_serializing_if = "is_usable")]
    pub(in crate::gameplay) usable: bool,
}

const fn usable_by_default() -> bool {
    true
}

const fn is_usable(usable: &bool) -> bool {
    *usable
}

impl CostEvent {
    fn unpriced(&self) -> bool {
        self.lines.iter().any(|line| line.micro_usd.is_none())
    }

    /// A stored event is accepted only when its total is the sum of its lines.
    fn is_consistent(&self) -> bool {
        !self.call_id.is_empty()
            && self.lines.iter().try_fold(0_u64, |sum, line| {
                sum.checked_add(line.micro_usd.unwrap_or(0))
            }) == Some(self.micro_usd)
    }
}

/// `micro` micro-dollars as dollars with six decimals, exactly.
pub(in crate::gameplay) fn usd(micro: u64) -> String {
    format!("{}.{:06}", micro / 1_000_000, micro % 1_000_000)
}

/// `text` reduced to the characters a `COST` word may hold.
fn word(text: &str) -> String {
    let word: String = text
        .chars()
        .take(120)
        .map(|character| {
            let character = character.to_ascii_lowercase();
            if character.is_ascii_alphanumeric() || "_.:/-".contains(character) {
                character
            } else {
                '-'
            }
        })
        .collect();
    if word.is_empty() {
        "-".to_owned()
    } else {
        word
    }
}

/// The one documented `COST` audit line of an event.
pub(in crate::gameplay) fn cost_line(
    event: &CostEvent,
    run_micro: u64,
    session_micro: u64,
) -> String {
    let lines: Vec<String> = event
        .lines
        .iter()
        .map(|line| {
            format!(
                "{}:{}@{}",
                line.category.label(),
                line.quantity,
                line.micro_usd.map_or_else(|| "unpriced".to_owned(), usd)
            )
        })
        .collect();
    format!(
        "COST call={} provider={} model={} cause={} detail={} source={}{} usd={} lines={} \
         run_usd={} session_usd={}",
        word(&event.call_id),
        event.provider.label(),
        word(&event.model),
        event.cause.label(),
        word(&event.detail),
        event.source.label(),
        if event.usable { "" } else { " usable=no" },
        usd(event.micro_usd),
        if lines.is_empty() {
            "-".to_owned()
        } else {
            lines.join(",")
        },
        usd(run_micro),
        usd(session_micro),
    )
}

// ------------------------------------------------------------------------------------------------
// Metering.

/// One billed provider call in flight. Settle it with [`Self::settle`] or [`Self::not_billed`];
/// dropped unsettled it is recorded with its estimate as of unknown outcome.
#[must_use = "a billed call must be settled with its usage, or marked not billed"]
pub(in crate::gameplay) struct BilledCall {
    ledger: Arc<SpendLedger>,
    spec: Option<CallSpec>,
    estimate: Vec<(Category, u64)>,
}

impl BilledCall {
    pub(in crate::gameplay) fn new(
        ledger: Arc<SpendLedger>,
        spec: CallSpec,
        estimate: &[(Category, u64)],
    ) -> Self {
        // A spec that states its maxima is estimated by them, whatever estimate the caller sent.
        let estimate = if spec.maxima.is_empty() {
            estimate.to_vec()
        } else {
            spec.maxima.clone()
        };
        ledger.costs().notify_started(&spec, &estimate);
        Self {
            ledger,
            spec: Some(spec),
            estimate,
        }
    }

    /// Records what the call used. A usage with no quantities (a call that reported none) stands
    /// for the call's own estimate, flagged as the usage's source says. The calls that follow it
    /// (see [`CallSpec::follow_ons`]) are recorded as events of their own, estimated.
    pub(in crate::gameplay) fn settle(self, usage: Usage) {
        self.settle_at(usage, now_ms());
    }

    /// As [`Self::settle`], recorded at `at_ms`: the caller's clock.
    pub(in crate::gameplay) fn settle_at(mut self, usage: Usage, at_ms: u64) {
        if let Some(mut spec) = self.spec.take() {
            let usage = if usage.quantities.is_empty() {
                Usage {
                    source: usage.source,
                    quantities: std::mem::take(&mut self.estimate),
                    usable: usage.usable,
                }
            } else {
                usage
            };
            let follow_ons = std::mem::take(&mut spec.follow_ons);
            self.ledger.costs().meter(spec, usage, at_ms);
            for follow_on in follow_ons {
                self.ledger.costs().meter(
                    follow_on.spec,
                    Usage::estimated(follow_on.quantities),
                    at_ms,
                );
            }
        }
    }

    /// The provider refused before doing billable work (a rate limit, a rejected request):
    /// nothing is recorded.
    pub(in crate::gameplay) fn not_billed(mut self) {
        self.spec = None;
    }
}

impl Drop for BilledCall {
    fn drop(&mut self) {
        if let Some(spec) = self.spec.take() {
            let usage = Usage {
                source: UsageSource::Unknown,
                quantities: std::mem::take(&mut self.estimate),
                usable: true,
            };
            self.ledger.costs().meter(spec, usage, now_ms());
        }
    }
}

/// The cause of a picture of `kind`.
pub(in crate::gameplay) const fn image_cause(kind: ImageKind) -> Cause {
    match kind {
        ImageKind::CanonicalCharacter => Cause::CharacterPortrait,
        ImageKind::CanonicalRoom => Cause::PlacePortrait,
        ImageKind::SceneRefresh => Cause::SceneRefresh,
        ImageKind::CastExpression => Cause::CastExpression,
        ImageKind::ArtifactCloseUp => Cause::ItemCloseUp,
        ImageKind::ScenePlate => Cause::ScenePlate,
        ImageKind::ScenePlateEdit => Cause::PlateEdit,
    }
}

const fn quality_label(quality: ImageQuality) -> &'static str {
    match quality {
        ImageQuality::Low => "low",
        ImageQuality::Medium => "medium",
        ImageQuality::High => "high",
    }
}

/// The provider and the billed model of an image route. A FLUX edit is billed under the fal
/// endpoint that made it: text to image without references, the multi-reference edit with them.
pub(in crate::gameplay) fn image_route_model(
    route: &ImageRoute,
    model: ImageEditModel,
    has_references: bool,
) -> (Provider, &'static str) {
    match route {
        ImageRoute::OpenAi(_) => (
            Provider::OpenAi,
            match model {
                ImageEditModel::Gpt25Flare => ImageEditModel::Gpt25Flare.model_id(),
                ImageEditModel::Gpt25Sunburst => ImageEditModel::Gpt25Sunburst.model_id(),
            },
        ),
        ImageRoute::Flux(_) if has_references => (Provider::Fal, df_providers::FLUX_KONTEXT_MULTI),
        ImageRoute::Flux(_) => (Provider::Fal, df_providers::FLUX_ULTRA),
        ImageRoute::Bfl(_) => (Provider::Bfl, df_providers::BFL_FLUX3_IMAGE),
    }
}

/// Which pool a picture waited in: `background` is look-ahead work.
const fn priority_label(priority: ImagePriority) -> &'static str {
    match priority {
        ImagePriority::Foreground => "foreground",
        ImagePriority::Presence => "presence",
        ImagePriority::Background => "background",
    }
}

/// Output tokens of a gpt-image picture per 1024 by 1024 pixels at each quality: the published
/// token table (272, 1056 and 4160 for a square), which scales with the drawn area.
const fn image_output_tokens_per_square(quality: ImageQuality) -> u64 {
    match quality {
        ImageQuality::Low => 272,
        ImageQuality::Medium => 1056,
        ImageQuality::High => 4160,
    }
}

/// The most input tokens one reference picture can add: the vision patch cap.
const IMAGE_INPUT_TOKENS_PER_REFERENCE: u64 = 1_536;

/// The most prompt tokens of a clip frame edit.
const CLIP_FRAME_PROMPT_TOKENS: u64 = 1_000;

/// The conservative per-category maxima of one gpt-image edit: the prompt at two bytes per token,
/// every reference at the cap, the output by area and quality.
fn openai_image_maxima(edit: &ImageEdit<'_>) -> Vec<(Category, u64)> {
    let area = u64::from(edit.width) * u64::from(edit.height);
    let output = (area * image_output_tokens_per_square(edit.quality)).div_ceil(1024 * 1024);
    let references = u64::try_from(edit.references.len()).unwrap_or(u64::MAX);
    vec![
        (
            Category::TextInput,
            u64::try_from(edit.prompt.len().div_ceil(2)).unwrap_or(u64::MAX),
        ),
        (
            Category::ImageInput,
            references.saturating_mul(IMAGE_INPUT_TOKENS_PER_REFERENCE),
        ),
        (Category::ImageOutput, output),
    ]
}

/// The spec of one picture call: the cause from its kind, and a detail naming the pool (look-ahead
/// work is `background`), the imagery profile, the quality, the drawn size and the reference
/// count (what drives its cost). The maxima are in the route's own priced categories, so a call
/// of unknown outcome is booked at what it could have cost, never at zero.
pub(in crate::gameplay) fn image_call(
    route: &ImageRoute,
    model: ImageEditModel,
    kind: ImageKind,
    priority: ImagePriority,
    profile: &str,
    edit: &ImageEdit<'_>,
) -> CallSpec {
    let (provider, billed_model) = image_route_model(route, model, !edit.references.is_empty());
    let (maxima, follow_ons) = match route {
        ImageRoute::OpenAi(_) => (openai_image_maxima(edit), Vec::new()),
        // The supplier reports the credits after the call: the table's per-call ceiling stands
        // until it does.
        ImageRoute::Bfl(_) => (vec![(Category::Calls, 1)], Vec::new()),
        ImageRoute::Flux(_) => (
            vec![(Category::Images, 1)],
            // A transparent picture is cut out by a second fal call once the first one is made.
            if edit.background == ImageBackground::Transparent {
                vec![FollowOn {
                    spec: CallSpec::new(
                        Provider::Fal,
                        df_providers::BACKGROUND_REMOVAL,
                        image_cause(kind),
                        "background_removal",
                    ),
                    quantities: vec![(Category::Images, 1)],
                }]
            } else {
                Vec::new()
            },
        ),
    };
    CallSpec {
        follow_ons,
        ..CallSpec::new(
            provider,
            billed_model,
            image_cause(kind),
            format!(
                "{}/{}/{profile}/{}/{}x{}/refs{}",
                kind.label(),
                priority_label(priority),
                quality_label(edit.quality),
                edit.width,
                edit.height,
                edit.references.len()
            ),
        )
        .with_maxima(maxima)
    }
}

/// Settles one picture call from its provider result: the reported tokens (OpenAI), the reported
/// credits (BFL) or one estimated image (FLUX); a rate limit, an exhausted quota or a rejected
/// request was not billed; any other failure is of unknown outcome.
pub(in crate::gameplay) fn settle_image_call(
    call: BilledCall,
    route: &ImageRoute,
    result: &Result<(GeneratedImage, Option<String>), ImageFailure>,
) {
    match result {
        Ok((generated, credits)) => call.settle(image_usage(route, generated, credits.as_deref())),
        // A quota 429 is refused outright by the supplier, like a rate limit: nothing is booked.
        Err(
            ImageFailure::RateLimited | ImageFailure::InvalidRequest | ImageFailure::QuotaExhausted,
        ) => call.not_billed(),
        Err(_) => drop(call),
    }
}

/// The tokens an OpenAI image call reported; a call that reported none is booked at the call's
/// own maxima, flagged estimated (an empty estimate stands for them, see [`BilledCall::settle`]).
pub(in crate::gameplay) fn image_token_usage(usage: ImageUsage) -> Usage {
    if usage == ImageUsage::default() {
        return Usage::estimated(Vec::new());
    }
    Usage::reported(vec![
        (Category::TextInput, usage.text_input_tokens),
        (Category::ImageInput, usage.image_input_tokens),
        (Category::ImageOutput, usage.output_tokens),
    ])
}

/// The spec of one character-clip frame edit (`fallen` or `guard`) by `model`. The frame is a
/// square picture at medium quality from one reference.
pub(in crate::gameplay) fn clip_frame_call(model: ImageEditModel, fallen: bool) -> CallSpec {
    CallSpec::new(
        Provider::OpenAi,
        model.model_id(),
        Cause::ClipFrame,
        if fallen { "fallen" } else { "guard" },
    )
    .with_maxima(vec![
        (Category::TextInput, CLIP_FRAME_PROMPT_TOKENS),
        (Category::ImageInput, IMAGE_INPUT_TOKENS_PER_REFERENCE),
        (
            Category::ImageOutput,
            image_output_tokens_per_square(ImageQuality::Medium),
        ),
    ])
}

fn image_usage(route: &ImageRoute, generated: &GeneratedImage, credits: Option<&str>) -> Usage {
    match route {
        ImageRoute::OpenAi(_) => image_token_usage(generated.usage),
        ImageRoute::Bfl(_) => {
            match credits.and_then(|credits| credits.trim().parse::<u64>().ok()) {
                Some(credits) => Usage::reported(vec![(Category::Credits, credits)]),
                None => Usage::estimated(Vec::new()),
            }
        }
        ImageRoute::Flux(_) => Usage::estimated(vec![(Category::Images, 1)]),
    }
}

/// The spec of one storyteller text call of `purpose`.
pub(in crate::gameplay) fn text_call(model: &str, purpose: &str) -> CallSpec {
    CallSpec::new(Provider::OpenAi, model, Cause::Storyteller, purpose)
}

/// A subscription call is counted separately, without inventing an API tariff.
pub(in crate::gameplay) fn codex_text_call(purpose: &str) -> CallSpec {
    CallSpec::new(
        Provider::CodexSubscription,
        "codex-cli",
        Cause::Storyteller,
        purpose,
    )
    .with_maxima(vec![(Category::Calls, 1)])
}

pub(in crate::gameplay) fn subscription_usage() -> Usage {
    Usage::reported(vec![(Category::Calls, 1)])
}

/// A text call's supplier-reported tokens. Cached and cache-written tokens are part of the
/// reported input, so the plain input line is what remains.
pub(in crate::gameplay) fn text_usage(usage: &TextTokenUsage) -> Usage {
    let tokens = |count: u128| u64::try_from(count).unwrap_or(u64::MAX);
    let written = usage.cache_write_tokens.unwrap_or(0);
    let plain = usage
        .input_tokens
        .saturating_sub(usage.cached_input_tokens)
        .saturating_sub(written);
    let mut quantities = vec![
        (Category::TextInput, tokens(plain)),
        (Category::TextCachedInput, tokens(usage.cached_input_tokens)),
    ];
    if usage.cache_write_tokens.is_some() {
        quantities.push((Category::TextCacheWrite, tokens(written)));
    }
    quantities.push((Category::TextOutput, tokens(usage.output_tokens)));
    Usage::reported(quantities)
}

/// The spec of one voice clip of `line`, by `model`: a named speaker's line is a character voice
/// (detailed by actor id, else name), any other line is spoken in the narrator's voice, as the
/// voice cast decides it.
pub(in crate::gameplay) fn voice_call(model: &str, line: &VoiceLine) -> CallSpec {
    let (cause, detail) = match (line.actor_id(), line.speaker()) {
        (Some(actor), _) => (Cause::CharacterVoice, actor),
        (None, Some(name)) => (Cause::CharacterVoice, name),
        (None, None) => (Cause::NarrationVoice, NARRATOR_ROLE),
    };
    // The characters of the line are known exactly: the caller's estimate is its maxima.
    CallSpec::new(Provider::ElevenLabs, model, cause, detail)
}

/// Whether a failed speech request may have been billed: a request refused before it was sent,
/// or one the supplier refused outright (invalid, denied, rate limited, unavailable), was not.
pub(in crate::gameplay) const fn speech_failure_may_be_billed(reason: SpeechAdapterReason) -> bool {
    match reason {
        SpeechAdapterReason::UnregisteredRoute
        | SpeechAdapterReason::InvalidCredential
        | SpeechAdapterReason::InvalidProfile
        | SpeechAdapterReason::InvalidInput
        | SpeechAdapterReason::UnsupportedLocale
        | SpeechAdapterReason::ModeDoesNotDispatch
        | SpeechAdapterReason::Owner(_)
        | SpeechAdapterReason::Transport(
            NativeHttpRefusal::InvalidLimits
            | NativeHttpRefusal::InvalidConfiguration
            | NativeHttpRefusal::InvalidEndpoint
            | NativeHttpRefusal::InvalidCredential
            | NativeHttpRefusal::ProviderDisabled
            | NativeHttpRefusal::RequestTooLarge,
        )
        | SpeechAdapterReason::Supplier(
            ProviderFailureClass::InvalidRequest
            | ProviderFailureClass::Denied
            | ProviderFailureClass::Capacity
            | ProviderFailureClass::Unavailable,
        ) => false,
        SpeechAdapterReason::Transport(_)
        | SpeechAdapterReason::Supplier(_)
        | SpeechAdapterReason::Json
        | SpeechAdapterReason::Transcript
        | SpeechAdapterReason::EmptyAudio => true,
    }
}

/// Settles a speech call that failed with `reason`: not billed, or of unknown outcome.
pub(in crate::gameplay) fn settle_speech_failure(call: BilledCall, reason: SpeechAdapterReason) {
    if speech_failure_may_be_billed(reason) {
        drop(call);
    } else {
        call.not_billed();
    }
}

// ------------------------------------------------------------------------------------------------
// The book: totals, the replay high-water marks, the run log.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct LineTotal {
    quantity: u64,
    micro_usd: u64,
    unpriced_quantity: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CauseTotal {
    events: u64,
    micro_usd: u64,
}

/// The totals of one scope (the run or this session).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Totals {
    events: u64,
    micro_usd: u64,
    estimated_events: u64,
    unknown_events: u64,
    unpriced_events: u64,
    /// Calls billed whose output was no use.
    unusable_events: u64,
    by_line: BTreeMap<(Provider, String, Category), LineTotal>,
    by_cause: BTreeMap<Cause, CauseTotal>,
    by_detail: BTreeMap<(Cause, String), CauseTotal>,
}

impl Totals {
    fn add(&mut self, event: &CostEvent) {
        self.events += 1;
        self.micro_usd = self.micro_usd.saturating_add(event.micro_usd);
        match event.source {
            UsageSource::Reported => {}
            UsageSource::Estimated => self.estimated_events += 1,
            UsageSource::Unknown => self.unknown_events += 1,
        }
        if event.unpriced() {
            self.unpriced_events += 1;
        }
        if !event.usable {
            self.unusable_events += 1;
        }
        for line in &event.lines {
            let total = self
                .by_line
                .entry((event.provider, event.model.clone(), line.category))
                .or_default();
            total.quantity = total.quantity.saturating_add(line.quantity);
            match line.micro_usd {
                Some(micro) => total.micro_usd = total.micro_usd.saturating_add(micro),
                None => {
                    total.unpriced_quantity = total.unpriced_quantity.saturating_add(line.quantity)
                }
            }
        }
        let count = |total: &mut CauseTotal| {
            total.events += 1;
            total.micro_usd = total.micro_usd.saturating_add(event.micro_usd);
        };
        count(self.by_cause.entry(event.cause).or_default());
        count(
            self.by_detail
                .entry((event.cause, event.detail.clone()))
                .or_default(),
        );
    }

    fn json(&self) -> Value {
        let mut causes: Vec<(&Cause, &CauseTotal)> = self.by_cause.iter().collect();
        causes.sort_by(|a, b| b.1.micro_usd.cmp(&a.1.micro_usd).then(a.0.cmp(b.0)));
        let mut details: Vec<(&(Cause, String), &CauseTotal)> = self.by_detail.iter().collect();
        details.sort_by(|a, b| b.1.micro_usd.cmp(&a.1.micro_usd).then(a.0.cmp(b.0)));
        json!({
            "usd": usd(self.micro_usd),
            "micro_usd": self.micro_usd,
            "events": self.events,
            "estimated_events": self.estimated_events,
            "unknown_events": self.unknown_events,
            "unpriced_events": self.unpriced_events,
            "unusable_events": self.unusable_events,
            "by_line": self.by_line.iter().map(|((provider, model, category), total)| json!({
                "provider": provider.label(),
                "model": model,
                "category": category.label(),
                "quantity": total.quantity,
                "unpriced_quantity": total.unpriced_quantity,
                "micro_usd": total.micro_usd,
                "usd": usd(total.micro_usd),
            })).collect::<Vec<_>>(),
            "by_cause": causes.iter().map(|(cause, total)| json!({
                "cause": cause.label(),
                "events": total.events,
                "micro_usd": total.micro_usd,
                "usd": usd(total.micro_usd),
            })).collect::<Vec<_>>(),
            "top_spenders": details.iter().take(TOP_SPENDERS).map(|((cause, detail), total)| json!({
                "cause": cause.label(),
                "detail": detail,
                "events": total.events,
                "micro_usd": total.micro_usd,
                "usd": usd(total.micro_usd),
            })).collect::<Vec<_>>(),
        })
    }
}

enum WriterMessage {
    Line(String),
    /// Answers once every earlier line is written (tests).
    #[cfg(test)]
    Flush(mpsc::SyncSender<()>),
}

/// The run's append-only log, written by one owned thread. Dropping it closes the queue and
/// joins the thread after it has written every queued line.
struct CostLog {
    path: PathBuf,
    sender: Option<mpsc::SyncSender<WriterMessage>>,
    worker: Option<thread::JoinHandle<()>>,
    write_failures: Arc<AtomicU64>,
}

impl CostLog {
    fn start(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        let (sender, receiver) = mpsc::sync_channel::<WriterMessage>(WRITE_QUEUE);
        let write_failures = Arc::new(AtomicU64::new(0));
        let failures = Arc::clone(&write_failures);
        let worker = thread::Builder::new()
            .name("cost-log".to_owned())
            .spawn(move || {
                for message in receiver {
                    match message {
                        WriterMessage::Line(line) => {
                            if file.write_all(line.as_bytes()).is_err() {
                                failures.fetch_add(1, Ordering::SeqCst);
                            }
                        }
                        #[cfg(test)]
                        WriterMessage::Flush(done) => {
                            if file.flush().is_err() {
                                failures.fetch_add(1, Ordering::SeqCst);
                            }
                            let _answered = done.send(());
                        }
                    }
                }
            })?;
        Ok(Self {
            path: path.to_owned(),
            sender: Some(sender),
            worker: Some(worker),
            write_failures,
        })
    }

    /// Queues one line without waiting; a full queue is a counted write failure.
    fn write(&self, line: String) {
        let queued = self
            .sender
            .as_ref()
            .is_some_and(|sender| sender.try_send(WriterMessage::Line(line)).is_ok());
        if !queued {
            self.write_failures.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl Drop for CostLog {
    fn drop(&mut self) {
        drop(self.sender.take());
        if let Some(worker) = self.worker.take() {
            let _joined = worker.join();
        }
    }
}

struct BookState {
    run: String,
    session: String,
    opened_at_ms: Option<u64>,
    next_call: u64,
    prices: PriceTable,
    /// The sub-micro-dollar remainder carried per line, see [`prices::price_carrying`]. One entry
    /// per provider, model and category the run has priced.
    carries: BTreeMap<(Provider, String, Category), Carry>,
    pricing: PricingStatus,
    /// The highest event number counted per session: the whole replay dedupe state.
    high_water: BTreeMap<String, u64>,
    run_totals: Totals,
    session_totals: Totals,
    log: Option<CostLog>,
    /// Why there is no log: `None` before open or without a path, else `closed` or the I/O
    /// failure (`read:<kind>`, `write:<kind>`).
    log_note: Option<String>,
    /// Calls metered before the book was opened, priced and written when it opens.
    pending: Vec<(CallSpec, Usage, u64)>,
    pending_lost: u64,
    malformed_lines: u64,
    duplicate_calls: u64,
    /// Who is told of every call, see [`port::MeterObserver`].
    observers: Vec<Arc<dyn MeterObserver>>,
    /// The last [`RECENT_LIMIT`] calls, oldest first.
    recent: VecDeque<MeteredCall>,
}

/// The cost book a [`SpendLedger`] keeps: prices, totals and the run's log.
pub(in crate::gameplay) struct CostBook {
    state: Mutex<BookState>,
}

impl Default for CostBook {
    fn default() -> Self {
        Self::with_session(&format!("s{}", now_ms()))
    }
}

/// What [`CostBook::open`] found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct Opened {
    pub(in crate::gameplay) replayed: u64,
    pub(in crate::gameplay) malformed: u64,
    pub(in crate::gameplay) run_micro_usd: u64,
    /// The log could not be read or opened: the book counts in memory only.
    pub(in crate::gameplay) log_error: Option<String>,
}

/// `<session>-<n>` as its session and number.
fn split_call_id(call_id: &str) -> Option<(&str, u64)> {
    let (session, number) = call_id.rsplit_once('-')?;
    let number = number.parse::<u64>().ok().filter(|number| *number > 0)?;
    (!session.is_empty()).then_some((session, number))
}

impl CostBook {
    /// A book whose call ids start with `session`.
    pub(in crate::gameplay) fn with_session(session: &str) -> Self {
        Self {
            state: Mutex::new(BookState {
                run: String::new(),
                session: session.to_owned(),
                opened_at_ms: None,
                next_call: 0,
                prices: PriceTable::default(),
                carries: BTreeMap::new(),
                pricing: PricingStatus::NotLoaded,
                high_water: BTreeMap::new(),
                run_totals: Totals::default(),
                session_totals: Totals::default(),
                log: None,
                log_note: None,
                pending: Vec::new(),
                pending_lost: 0,
                malformed_lines: 0,
                duplicate_calls: 0,
                observers: Vec::new(),
                recent: VecDeque::new(),
            }),
        }
    }

    /// Opens the book for `run`: replays the run's earlier events from `log_path` into the run
    /// totals, starts the writer, and prices and writes the calls metered before. A log that
    /// cannot be read or opened is reported in [`Opened::log_error`] and the book opens in memory;
    /// only opening twice is refused.
    pub(in crate::gameplay) fn open(
        &self,
        run: &str,
        log_path: Option<&Path>,
        prices: PriceTable,
        pricing: PricingStatus,
        at_ms: u64,
    ) -> io::Result<Opened> {
        if locked(&self.state).opened_at_ms.is_some() {
            return Err(io::Error::other("the cost book is already open"));
        }
        let mut replayed_events = Vec::new();
        let mut malformed = 0;
        let mut log_error = None;
        if let Some(path) = log_path {
            match fs::File::open(path) {
                Ok(file) => {
                    for line in io::BufReader::new(file).lines() {
                        let Ok(line) = line else {
                            malformed += 1;
                            continue;
                        };
                        if line.trim().is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<CostEvent>(&line) {
                            Ok(event)
                                if event.is_consistent()
                                    && split_call_id(&event.call_id).is_some() =>
                            {
                                replayed_events.push(event);
                            }
                            _ => malformed += 1,
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => log_error = Some(format!("read:{:?}", error.kind())),
            }
        }
        // A log that could not be read is not appended to either: its run totals would be wrong.
        let log = match log_path.filter(|_| log_error.is_none()).map(CostLog::start) {
            Some(Ok(log)) => Some(log),
            Some(Err(error)) => {
                log_error = Some(format!("write:{:?}", error.kind()));
                None
            }
            None => None,
        };
        let (lines, opened) = {
            let mut state = locked(&self.state);
            if state.opened_at_ms.is_some() {
                return Err(io::Error::other("the cost book is already open"));
            }
            state.run = run.to_owned();
            state.opened_at_ms = Some(at_ms);
            state.prices = prices;
            state.carries.clear();
            state.pricing = pricing;
            state.log = log;
            state.log_note = log_error.clone();
            state.malformed_lines = malformed;
            let mut replayed = 0;
            for event in replayed_events {
                let Some((session, number)) = split_call_id(&event.call_id) else {
                    continue;
                };
                let mark = state.high_water.entry(session.to_owned()).or_default();
                if number <= *mark {
                    state.duplicate_calls += 1;
                    continue;
                }
                *mark = number;
                state.run_totals.add(&event);
                replayed += 1;
            }
            // A restart within the same millisecond must not reuse this session's numbers.
            let own = state.high_water.get(&state.session).copied().unwrap_or(0);
            state.next_call = state.next_call.max(own);
            let pending = std::mem::take(&mut state.pending);
            let recorded: Vec<(String, MeteredCall)> = pending
                .into_iter()
                .map(|(spec, usage, at)| Self::record_locked(&mut state, spec, usage, at))
                .collect();
            let observers = state.observers.clone();
            let lines: Vec<String> = recorded
                .into_iter()
                .map(|(line, call)| {
                    notify_settled(&observers, &call);
                    line
                })
                .collect();
            let opened = Opened {
                replayed,
                malformed,
                run_micro_usd: state.run_totals.micro_usd,
                log_error,
            };
            (lines, opened)
        };
        for line in lines {
            super::actor::record_audit_line("gameplay.cost", line);
        }
        Ok(opened)
    }

    /// Records one settled call under the next event number; never refused.
    fn meter(&self, spec: CallSpec, usage: Usage, at_ms: u64) {
        let (line, call, observers) = {
            let mut state = locked(&self.state);
            if state.opened_at_ms.is_none() {
                if state.pending.len() < PENDING_LIMIT {
                    state.pending.push((spec, usage, at_ms));
                } else {
                    state.pending_lost += 1;
                }
                return;
            }
            let (line, call) = Self::record_locked(&mut state, spec, usage, at_ms);
            (line, call, state.observers.clone())
        };
        super::actor::record_audit_line("gameplay.cost", line);
        notify_settled(&observers, &call);
    }

    /// Registers an observer of every call recorded from now on.
    pub(in crate::gameplay) fn observe(&self, observer: Arc<dyn MeterObserver>) {
        locked(&self.state).observers.push(observer);
    }

    /// Tells the observers a call is about to be sent.
    fn notify_started(&self, spec: &CallSpec, estimate: &[(Category, u64)]) {
        let observers = locked(&self.state).observers.clone();
        if observers.is_empty() {
            return;
        }
        let start = MeterStart::of(spec, estimate);
        for observer in &observers {
            observer.started(&start);
        }
    }

    /// The last calls recorded, oldest first.
    pub(in crate::gameplay) fn recent(&self) -> Vec<MeteredCall> {
        locked(&self.state).recent.iter().cloned().collect()
    }

    /// The run's call count, price and calls with an unpriced line.
    pub(in crate::gameplay) fn totals(&self) -> port::MeterTotals {
        let state = locked(&self.state);
        port::MeterTotals {
            calls: state.run_totals.events,
            micro_usd: state.run_totals.micro_usd,
            unpriced_calls: state.run_totals.unpriced_events,
        }
    }

    /// Numbers, prices, counts and queues one event; returns its `COST` line and the call as the
    /// port shows it.
    fn record_locked(
        state: &mut BookState,
        spec: CallSpec,
        usage: Usage,
        at_ms: u64,
    ) -> (String, MeteredCall) {
        state.next_call += 1;
        let number = state.next_call;
        let call_id = format!("{}-{number}", state.session);
        state.high_water.insert(state.session.clone(), number);
        let (lines, micro_usd) = price_carrying(
            &state.prices,
            &mut state.carries,
            spec.provider,
            &spec.model,
            &usage.quantities,
        );
        let event = CostEvent {
            call_id,
            at_ms,
            run: state.run.clone(),
            session: state.session.clone(),
            provider: spec.provider,
            model: spec.model,
            cause: spec.cause,
            detail: word(&spec.detail),
            source: usage.source,
            lines,
            micro_usd,
            usable: usage.usable,
        };
        state.run_totals.add(&event);
        state.session_totals.add(&event);
        if let Some(log) = &state.log {
            match serde_json::to_string(&event) {
                Ok(mut json) => {
                    json.push('\n');
                    log.write(json);
                }
                Err(_) => {
                    log.write_failures.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
        let call = MeteredCall::of(&event);
        if state.recent.len() == RECENT_LIMIT {
            state.recent.pop_front();
        }
        state.recent.push_back(call.clone());
        let line = cost_line(
            &event,
            state.run_totals.micro_usd,
            state.session_totals.micro_usd,
        );
        (line, call)
    }

    /// Writes every queued line, stops the writer and keeps counting in memory: the server's
    /// shutdown step. Blocks until the writer has drained, so call it off the async workers.
    pub(in crate::gameplay) fn close(&self) {
        let (log, run_micro, session_micro) = {
            let mut state = locked(&self.state);
            let log = state.log.take();
            if log.is_some() {
                state.log_note = Some("closed".to_owned());
            }
            (
                log,
                state.run_totals.micro_usd,
                state.session_totals.micro_usd,
            )
        };
        let Some(log) = log else {
            return;
        };
        let path = log.path.display().to_string();
        let failures = Arc::clone(&log.write_failures);
        drop(log);
        super::actor::record_audit_line(
            "gameplay.cost",
            format!(
                "COST closed log={path} run_usd={} session_usd={} write_failures={}",
                usd(run_micro),
                usd(session_micro),
                failures.load(Ordering::SeqCst)
            ),
        );
    }

    /// Waits until every queued line is written.
    #[cfg(test)]
    pub(in crate::gameplay) fn flush(&self) -> bool {
        let (done, wait) = mpsc::sync_channel(1);
        let queued = {
            let state = locked(&self.state);
            state.log.as_ref().and_then(|log| log.sender.clone())
        }
        .is_some_and(|sender| sender.send(WriterMessage::Flush(done)).is_ok());
        queued && wait.recv_timeout(FLUSH_WAIT).is_ok()
    }

    /// The `dev cost` document: the run and the session, by provider, model and category, by
    /// cause, and the top spenders. `now_ms` dates the session's spend per wall-clock hour.
    pub(in crate::gameplay) fn report(&self, now_ms: u64) -> Value {
        let state = locked(&self.state);
        let elapsed = state
            .opened_at_ms
            .map(|opened| now_ms.saturating_sub(opened))
            .filter(|elapsed| *elapsed > 0);
        let per_hour = elapsed.map(|elapsed| {
            let micro =
                u128::from(state.session_totals.micro_usd) * 3_600_000 / u128::from(elapsed);
            usd(u64::try_from(micro).unwrap_or(u64::MAX))
        });
        let mut session = state.session_totals.json();
        session["id"] = json!(state.session);
        session["opened_at_ms"] = json!(state.opened_at_ms);
        session["usd_per_wall_hour"] = json!(per_hour);
        let mut run = state.run_totals.json();
        run["id"] = json!(state.run);
        json!({
            "pricing": state.pricing.json(),
            "prices": state.prices.json(),
            "routes": ROUTES.iter().map(BilledRoute::json).collect::<Vec<_>>(),
            "log": {
                "path": state.log.as_ref().map(|log| log.path.display().to_string()),
                "status": match (&state.log, &state.log_note) {
                    (Some(_), _) => "open".to_owned(),
                    (None, Some(note)) => note.clone(),
                    (None, None) => "off".to_owned(),
                },
                "write_failures": state
                    .log
                    .as_ref()
                    .map_or(0, |log| log.write_failures.load(Ordering::SeqCst)),
                "malformed_lines": state.malformed_lines,
                "duplicate_calls": state.duplicate_calls,
                "pending_lost": state.pending_lost,
            },
            "run": run,
            "session": session,
        })
    }

    /// The run's spend so far, by provider, model and category, in micro-dollars.
    fn run_lines(&self) -> BTreeMap<(Provider, String, Category), u64> {
        locked(&self.state)
            .run_totals
            .by_line
            .iter()
            .map(|(key, total)| (key.clone(), total.micro_usd))
            .collect()
    }

    /// The price of `quantities` of one model at the book's current rates, rounded once: what a
    /// call is expected to cost before it is made. `None` when any line has no rate.
    pub(in crate::gameplay) fn quote(
        &self,
        provider: Provider,
        model: &str,
        quantities: &[(Category, u64)],
    ) -> Option<u64> {
        locked(&self.state)
            .prices
            .quote(provider, model, quantities)
    }

    /// The price of one OpenAI picture call's reported tokens at the book's rates; `None` when
    /// the call reported none or the file does not price them.
    pub(in crate::gameplay) fn quote_image_usage(
        &self,
        model: &str,
        usage: ImageUsage,
    ) -> Option<u64> {
        let Usage { quantities, .. } = image_token_usage(usage);
        if quantities.is_empty() {
            return None;
        }
        self.quote(Provider::OpenAi, model, &quantities)
    }

    /// Compares the run's spend with a provider bill (see [`parse_bill`]).
    pub(in crate::gameplay) fn compare_bill(&self, csv: &str) -> Result<Value, String> {
        let bill = parse_bill(csv)?;
        let ours = self.run_lines();
        // Our lines of a provider the bill does not cover are not compared.
        let mut keys: Vec<&(Provider, String, Category)> = bill.keys().collect();
        keys.extend(
            ours.keys()
                .filter(|key| bill.keys().any(|billed| billed.0 == key.0)),
        );
        keys.sort();
        keys.dedup();
        let rows: Vec<Value> = keys
            .into_iter()
            .map(|key| {
                let ours = ours.get(key).copied().unwrap_or(0);
                let billed = bill.get(key).copied().unwrap_or(0);
                json!({
                    "provider": key.0.label(),
                    "model": key.1,
                    "category": key.2.label(),
                    "ours_usd": usd(ours),
                    "bill_usd": usd(billed),
                    "delta_usd": if ours >= billed {
                        usd(ours - billed)
                    } else {
                        format!("-{}", usd(billed - ours))
                    },
                })
            })
            .collect();
        Ok(json!({"rows": rows}))
    }
}

/// Tells each observer of a recorded call; run after the book's lock is released.
fn notify_settled(observers: &[Arc<dyn MeterObserver>], call: &MeteredCall) {
    for observer in observers {
        observer.settled(call);
    }
}

/// Parses a bill in the documented comparison form: a `provider,model,category,usd` header, then
/// one row per line item, amounts in dollars (at most six decimals; repeated rows add up).
///
/// TODO(C-B3): read the providers' own exports (the OpenAI organization costs export names line
/// items such as "gpt-image-2.5-flare, input image") into these rows, as item C-B3 of the "Cost
/// engine backlog" in `planning/issue-triage.md` says; today the owner writes the rows from the
/// billing page.
pub(in crate::gameplay) fn parse_bill(
    csv: &str,
) -> Result<BTreeMap<(Provider, String, Category), u64>, String> {
    let mut rows = BTreeMap::new();
    for ((provider, model, category), micro) in df_commerce::tariff::parse_bill(csv)? {
        let provider = Provider::parse(&provider)
            .ok_or_else(|| format!("bill names an unknown provider {}", word(&provider)))?;
        let category = Category::parse(&category)
            .ok_or_else(|| format!("bill names an unknown category {}", word(&category)))?;
        let total: &mut u64 = rows.entry((provider, model, category)).or_default();
        *total = total.saturating_add(micro);
    }
    Ok(rows)
}

// ------------------------------------------------------------------------------------------------
// Startup and the dev read.

static CURRENT: Mutex<Option<Arc<SpendLedger>>> = Mutex::new(None);

/// The ledger whose book `dev cost` reads.
pub(in crate::gameplay) fn current() -> Option<Arc<SpendLedger>> {
    locked(&CURRENT).clone()
}

/// The server's shutdown step: uninstalls the run's ledger and drains its log. Blocking.
pub(in crate::gameplay) fn close_run() {
    let installed = locked(&CURRENT).take();
    if let Some(ledger) = installed {
        ledger.costs().close();
    }
}

/// The price file: `DF_GAMEPLAY_PRICING`, else `content/pricing.json` under `root`.
fn pricing_path(root: &Path) -> PathBuf {
    match std::env::var("DF_GAMEPLAY_PRICING") {
        Ok(path) if !path.is_empty() => PathBuf::from(path),
        _ => root.join("content").join("pricing.json"),
    }
}

/// Opens the hub ledger's cost book for the run named by the game's database: loads the price
/// table, replays `development/runs/<database>/cost.jsonl` under `root`, and installs the ledger
/// for `dev cost`. Returns the startup lines. Nothing here can stop the game: a refused price
/// file or an unwritable log is reported and the calls are still counted in memory.
pub(in crate::gameplay) fn open_run(
    ledger: &Arc<SpendLedger>,
    root: &Path,
    database: &str,
) -> String {
    let (prices, pricing) = load_prices(&pricing_path(root));
    let pricing_line = match &pricing {
        PricingStatus::Loaded { path, entries } => {
            format!(
                "COST pricing loaded entries={entries} path={}",
                path.display()
            )
        }
        PricingStatus::Missing { path } => {
            format!(
                "COST pricing missing path={} (every line unpriced)",
                path.display()
            )
        }
        PricingStatus::Refused { path, reason } => format!(
            "COST pricing REFUSED reason={reason} path={} (every line unpriced; fix the file)",
            path.display()
        ),
        PricingStatus::NotLoaded => "COST pricing not_loaded".to_owned(),
    };
    let safe = !database.is_empty()
        && database.len() <= 63
        && database
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    let log_path = safe.then(|| {
        root.join("development")
            .join("runs")
            .join(database)
            .join("cost.jsonl")
    });
    let run = if safe { database } else { "unnamed" };
    let opened = ledger
        .costs()
        .open(run, log_path.as_deref(), prices, pricing, now_ms());
    *locked(&CURRENT) = Some(Arc::clone(ledger));
    let log_line = match (opened, &log_path) {
        (
            Ok(Opened {
                log_error: Some(error),
                ..
            }),
            _,
        ) => format!("COST on run={run} log=unavailable ({error}); counted in memory only"),
        (Ok(opened), Some(path)) => format!(
            "COST on run={run} log={} replayed={} malformed={} run_usd={}",
            path.display(),
            opened.replayed,
            opened.malformed,
            usd(opened.run_micro_usd)
        ),
        (Ok(_), None) => format!("COST on run={run} log=off (no database name)"),
        (Err(error), _) => format!("COST on run={run} not opened ({:?})", error.kind()),
    };
    format!("{pricing_line}\n{log_line}")
}

// ------------------------------------------------------------------------------------------------
// The coverage guard: every provider client the game can call, and how its cost is metered.

/// How a billed route's cost reaches the book.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum Metering {
    /// Metered and priced from the table.
    Priced,
}

/// One billed route: the `df_providers` client, the provider, the model and the categories its
/// events carry.
#[derive(Clone, Copy, Debug)]
pub(in crate::gameplay) struct BilledRoute {
    pub(in crate::gameplay) client: &'static str,
    pub(in crate::gameplay) provider: Provider,
    pub(in crate::gameplay) model: &'static str,
    pub(in crate::gameplay) categories: &'static [Category],
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "read by the cost-metering route tests; settlement reads categories"
        )
    )]
    pub(in crate::gameplay) metering: Metering,
}

const IMAGE_TOKENS: &[Category] = &[
    Category::TextInput,
    Category::ImageInput,
    Category::ImageOutput,
];
/// A vision judgement's declared categories: the instruction, the pictures and the answer.
const VISION_TOKENS: &[Category] = &[
    Category::TextInput,
    Category::ImageInput,
    Category::TextOutput,
];
const TEXT_TOKENS: &[Category] = &[
    Category::TextInput,
    Category::TextCachedInput,
    Category::TextCacheWrite,
    Category::TextOutput,
];

/// The categories the route of `provider` and `model` declares; none for a model no route names.
fn declared_categories(provider: Provider, model: &str) -> &'static [Category] {
    ROUTES
        .iter()
        .find(|route| route.provider == provider && route.model == model)
        .map_or(&[], |route| route.categories)
}

impl BilledRoute {
    fn json(&self) -> Value {
        json!({
            "client": self.client,
            "provider": self.provider.label(),
            "model": self.model,
            "categories": self
                .categories
                .iter()
                .map(|category| category.label())
                .collect::<Vec<_>>(),
            "metering": "priced",
            "reason": "",
        })
    }
}

/// Every route the game calls; a reason's `C-Bn` is an item of the cost engine backlog in
/// `planning/issue-triage.md`. The coverage tests check this against the `df_providers` clients
/// and the price file, so a new client cannot go untracked.
pub(in crate::gameplay) const ROUTES: &[BilledRoute] = &[
    BilledRoute {
        client: "NativeImageEditProvider",
        provider: Provider::OpenAi,
        model: "gpt-image-2.5-flare",
        categories: IMAGE_TOKENS,
        metering: Metering::Priced,
    },
    BilledRoute {
        client: "NativeImageEditProvider",
        provider: Provider::OpenAi,
        model: "gpt-image-2.5-sunburst",
        categories: IMAGE_TOKENS,
        metering: Metering::Priced,
    },
    BilledRoute {
        client: "NativeBflImageProvider",
        provider: Provider::Bfl,
        model: df_providers::BFL_FLUX3_IMAGE,
        categories: &[Category::Credits, Category::Calls],
        metering: Metering::Priced,
    },
    BilledRoute {
        client: "NativeFalImageProvider",
        provider: Provider::Fal,
        model: df_providers::FLUX_ULTRA,
        categories: &[Category::Images],
        metering: Metering::Priced,
    },
    BilledRoute {
        client: "NativeFalImageProvider",
        provider: Provider::Fal,
        model: df_providers::FLUX_KONTEXT_MULTI,
        categories: &[Category::Images],
        metering: Metering::Priced,
    },
    BilledRoute {
        client: "NativeFalImageProvider",
        provider: Provider::Fal,
        model: df_providers::BACKGROUND_REMOVAL,
        categories: &[Category::Images],
        metering: Metering::Priced,
    },
    BilledRoute {
        client: "NativeTextProvider",
        provider: Provider::OpenAi,
        model: "gpt-6-luna",
        categories: TEXT_TOKENS,
        metering: Metering::Priced,
    },
    BilledRoute {
        client: "NativeSpeechProvider",
        provider: Provider::ElevenLabs,
        model: df_providers::ELEVEN_TTS_MODEL,
        categories: &[Category::Characters],
        metering: Metering::Priced,
    },
    BilledRoute {
        client: "NativeSpeechProvider",
        provider: Provider::ElevenLabs,
        model: df_providers::ELEVEN_STT_MODEL,
        categories: &[Category::AudioSeconds],
        metering: Metering::Priced,
    },
    BilledRoute {
        client: "NativeEmbeddingProvider",
        provider: Provider::OpenAi,
        model: df_providers::EMBEDDING_MODEL_SMALL,
        categories: &[Category::TextInput],
        metering: Metering::Priced,
    },
    BilledRoute {
        client: "NativeFalSegmenter",
        provider: Provider::Fal,
        model: "fal-ai/sam-3-1/image-rle",
        categories: &[Category::Calls],
        metering: Metering::Priced,
    },
    BilledRoute {
        client: "NativeHeadLocator",
        provider: Provider::OpenAi,
        model: "gpt-5.5",
        categories: VISION_TOKENS,
        metering: Metering::Priced,
    },
    BilledRoute {
        client: "NativePlateJudge",
        provider: Provider::OpenAi,
        model: "gpt-5.5",
        categories: VISION_TOKENS,
        metering: Metering::Priced,
    },
    BilledRoute {
        client: "NativeElevenMusicProvider",
        provider: Provider::ElevenLabs,
        model: df_providers::MUSIC_MODEL,
        categories: &[Category::AudioSeconds],
        metering: Metering::Priced,
    },
    BilledRoute {
        client: "NativeElevenMusicProvider",
        provider: Provider::ElevenLabs,
        model: df_providers::SOUND_MODEL,
        categories: &[Category::AudioSeconds],
        metering: Metering::Priced,
    },
    BilledRoute {
        client: "NativeFalMediaProvider",
        provider: Provider::Fal,
        model: df_providers::KLING_O1_IMAGE_TO_VIDEO,
        categories: &[Category::VideoSeconds],
        metering: Metering::Priced,
    },
    BilledRoute {
        client: "NativeWorldLabs",
        provider: Provider::WorldLabs,
        model: "marble-1.1",
        categories: &[Category::Credits],
        metering: Metering::Priced,
    },
    BilledRoute {
        client: "SystemOneChoiceProvider",
        provider: Provider::TypeSafe,
        model: "jev-latest",
        categories: &[Category::TextInput],
        metering: Metering::Priced,
    },
];

#[cfg(test)]
#[path = "cost_tests.rs"]
mod tests;

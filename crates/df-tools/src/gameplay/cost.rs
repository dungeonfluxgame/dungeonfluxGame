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
//!   priced line is `quantity * usd / per`, computed exactly in nano-dollars with `u128`, then
//!   rounded half up to a whole micro-dollar ([`Price::micro_usd`]). An event's total is the sum
//!   of its rounded lines and every total is a sum of event lines, so each breakdown sums to its
//!   total exactly.
//! * **Prices are data.** `content/pricing.json` (schema `dungeonflux.pricing/1`), or the file
//!   `DF_GAMEPLAY_PRICING` names, holds one dated, sourced rate per provider, model and
//!   category. A malformed file is refused whole and loudly (the `COST pricing` startup line);
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
    collections::BTreeMap,
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
    GeneratedImage, ImageEdit, ImageEditModel, ImageQuality, ImageUsage, NativeHttpRefusal,
    ProviderFailureClass, SpeechAdapterReason, TextTokenUsage,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::imagery::{ImageFailure, ImageKind, ImagePriority, ImageRoute};
use super::resources::SpendLedger;
use crate::voice_line::{NARRATOR_ROLE, VoiceLine};

/// The price table's schema name; a file with any other is refused.
pub(in crate::gameplay) const PRICING_SCHEMA: &str = "dungeonflux.pricing/1";
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
    pub(in crate::gameplay) const ALL: [Self; 6] = [
        Self::OpenAi,
        Self::ElevenLabs,
        Self::Fal,
        Self::Bfl,
        Self::WorldLabs,
        Self::TypeSafe,
    ];

    pub(in crate::gameplay) const fn label(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
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
}

impl Cause {
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
}

impl Usage {
    /// Quantities from the provider's own usage report.
    pub(in crate::gameplay) fn reported(quantities: Vec<(Category, u64)>) -> Self {
        Self {
            source: UsageSource::Reported,
            quantities,
        }
    }

    /// Quantities the server counted (flagged estimated in every report).
    pub(in crate::gameplay) fn estimated(quantities: Vec<(Category, u64)>) -> Self {
        Self {
            source: UsageSource::Estimated,
            quantities,
        }
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
}

impl CallSpec {
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
        "COST call={} provider={} model={} cause={} detail={} source={} usd={} lines={} \
         run_usd={} session_usd={}",
        word(&event.call_id),
        event.provider.label(),
        word(&event.model),
        event.cause.label(),
        word(&event.detail),
        event.source.label(),
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
// The price table.

/// One dated, sourced rate: `usd` for every `per` units.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct Price {
    nano_usd: u64,
    per: u64,
    as_of: String,
    source: String,
}

impl Price {
    /// The price of `quantity` units in whole micro-dollars: `quantity * usd / per`, exact in
    /// nano-dollars, rounded half up. Saturates at `u64::MAX`.
    pub(in crate::gameplay) fn micro_usd(&self, quantity: u64) -> u64 {
        let nanos = u128::from(quantity) * u128::from(self.nano_usd);
        let divisor = u128::from(self.per) * 1_000;
        u64::try_from((nanos + divisor / 2) / divisor).unwrap_or(u64::MAX)
    }
}

/// Why a price file was refused. It names the entry, never the file's other contents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum PricingError {
    Unreadable,
    Json(String),
    Schema,
    Currency,
    Entry { index: usize, reason: &'static str },
    Duplicate { index: usize },
}

impl PricingError {
    pub(in crate::gameplay) fn reason(&self) -> String {
        match self {
            Self::Unreadable => "unreadable".to_owned(),
            Self::Json(detail) => format!("json:{}", word(detail)),
            Self::Schema => format!("schema_not_{PRICING_SCHEMA}"),
            Self::Currency => "currency_not_usd".to_owned(),
            Self::Entry { index, reason } => format!("entry_{index}_{reason}"),
            Self::Duplicate { index } => format!("entry_{index}_duplicate"),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PriceFile {
    schema: String,
    currency: String,
    prices: Vec<PriceEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PriceEntry {
    provider: String,
    model: String,
    category: String,
    usd: String,
    per: u64,
    as_of: String,
    source: String,
    #[serde(default)]
    #[allow(
        dead_code,
        reason = "a note for the owner who edits the file; never read"
    )]
    note: String,
}

/// `text` as whole nano-dollars: digits, optionally a point and at most nine decimals.
fn parse_nano_usd(text: &str) -> Option<u64> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty()
        || fraction.len() > 9
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || (text.contains('.') && fraction.is_empty())
    {
        return None;
    }
    let whole: u64 = whole.parse().ok()?;
    let fraction: u64 = if fraction.is_empty() {
        0
    } else {
        format!("{fraction:0<9}").parse().ok()?
    };
    whole.checked_mul(1_000_000_000)?.checked_add(fraction)
}

/// Whole nano-dollars as the shortest exact dollar text (`0.042`, `5`).
fn nano_usd_text(nanos: u64) -> String {
    let fraction = format!("{:09}", nanos % 1_000_000_000);
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        (nanos / 1_000_000_000).to_string()
    } else {
        format!("{}.{fraction}", nanos / 1_000_000_000)
    }
}

fn is_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
}

/// The dated prices, by provider, model and category. A lookup is a pure read.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::gameplay) struct PriceTable {
    entries: BTreeMap<(Provider, String, Category), Price>,
}

impl PriceTable {
    /// Parses and validates a whole price file; any malformed entry refuses the whole file.
    pub(in crate::gameplay) fn parse(text: &str) -> Result<Self, PricingError> {
        let file: PriceFile = serde_json::from_str(text).map_err(|error| {
            PricingError::Json(format!("line_{}_column_{}", error.line(), error.column()))
        })?;
        if file.schema != PRICING_SCHEMA {
            return Err(PricingError::Schema);
        }
        if file.currency != "USD" {
            return Err(PricingError::Currency);
        }
        let mut entries = BTreeMap::new();
        for (index, entry) in file.prices.into_iter().enumerate() {
            let refuse = |reason| PricingError::Entry { index, reason };
            let provider = Provider::parse(&entry.provider).ok_or(refuse("provider"))?;
            let category = Category::parse(&entry.category).ok_or(refuse("category"))?;
            if entry.model.is_empty()
                || entry.model.len() > 120
                || !entry.model.bytes().all(|byte| byte.is_ascii_graphic())
            {
                return Err(refuse("model"));
            }
            let nano_usd = parse_nano_usd(&entry.usd).ok_or(refuse("usd"))?;
            if entry.per == 0 {
                return Err(refuse("per"));
            }
            if !is_date(&entry.as_of) {
                return Err(refuse("as_of"));
            }
            if !entry.source.starts_with("https://") || entry.source.len() > 400 {
                return Err(refuse("source"));
            }
            let price = Price {
                nano_usd,
                per: entry.per,
                as_of: entry.as_of,
                source: entry.source,
            };
            if entries
                .insert((provider, entry.model, category), price)
                .is_some()
            {
                return Err(PricingError::Duplicate { index });
            }
        }
        Ok(Self { entries })
    }

    /// The rate of one line, if the table has it.
    pub(in crate::gameplay) fn lookup(
        &self,
        provider: Provider,
        model: &str,
        category: Category,
    ) -> Option<&Price> {
        self.entries.get(&(provider, model.to_owned(), category))
    }

    pub(in crate::gameplay) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Every rate with its date and source, for `dev cost`.
    fn json(&self) -> Value {
        self.entries
            .iter()
            .map(|((provider, model, category), price)| {
                json!({
                    "provider": provider.label(),
                    "model": model,
                    "category": category.label(),
                    "usd": nano_usd_text(price.nano_usd),
                    "per": price.per,
                    "as_of": price.as_of,
                    "source": price.source,
                })
            })
            .collect()
    }

    /// Prices `quantities` for one call: the lines of a [`CostEvent`] and their sum.
    pub(in crate::gameplay) fn price(
        &self,
        provider: Provider,
        model: &str,
        quantities: &[(Category, u64)],
    ) -> (Vec<CostLine>, u64) {
        let lines: Vec<CostLine> = quantities
            .iter()
            .map(|(category, quantity)| CostLine {
                category: *category,
                quantity: *quantity,
                micro_usd: self
                    .lookup(provider, model, *category)
                    .map(|price| price.micro_usd(*quantity)),
            })
            .collect();
        let total = lines.iter().fold(0_u64, |sum, line| {
            sum.saturating_add(line.micro_usd.unwrap_or(0))
        });
        (lines, total)
    }
}

/// How the price file was read at startup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum PricingStatus {
    /// No book has been opened yet.
    NotLoaded,
    Loaded {
        path: PathBuf,
        entries: usize,
    },
    /// No file: every line is unpriced.
    Missing {
        path: PathBuf,
    },
    /// A malformed file is refused whole: every line is unpriced.
    Refused {
        path: PathBuf,
        reason: String,
    },
}

impl PricingStatus {
    fn json(&self) -> Value {
        match self {
            Self::NotLoaded => json!({"status": "not_loaded"}),
            Self::Loaded { path, entries } => {
                json!({"status": "loaded", "path": path.display().to_string(), "entries": entries})
            }
            Self::Missing { path } => {
                json!({"status": "missing", "path": path.display().to_string()})
            }
            Self::Refused { path, reason } => json!({
                "status": "refused",
                "path": path.display().to_string(),
                "reason": reason,
            }),
        }
    }
}

/// Reads the price file at `path`. Never fails: a missing or refused file gives an empty table.
pub(in crate::gameplay) fn load_prices(path: &Path) -> (PriceTable, PricingStatus) {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return (
                PriceTable::default(),
                PricingStatus::Missing {
                    path: path.to_owned(),
                },
            );
        }
        Err(_) => {
            return (
                PriceTable::default(),
                PricingStatus::Refused {
                    path: path.to_owned(),
                    reason: PricingError::Unreadable.reason(),
                },
            );
        }
    };
    match PriceTable::parse(&text) {
        Ok(table) => {
            let entries = table.len();
            (
                table,
                PricingStatus::Loaded {
                    path: path.to_owned(),
                    entries,
                },
            )
        }
        Err(error) => (
            PriceTable::default(),
            PricingStatus::Refused {
                path: path.to_owned(),
                reason: error.reason(),
            },
        ),
    }
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
        Self {
            ledger,
            spec: Some(spec),
            estimate: estimate.to_vec(),
        }
    }

    /// Records what the call used.
    pub(in crate::gameplay) fn settle(mut self, usage: Usage) {
        if let Some(spec) = self.spec.take() {
            self.ledger.costs().meter(spec, usage, now_ms());
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

/// The provider and the billed model of an image route.
pub(in crate::gameplay) fn image_route_model(
    route: &ImageRoute,
    model: ImageEditModel,
) -> (Provider, &'static str) {
    match route {
        ImageRoute::OpenAi(_) => (
            Provider::OpenAi,
            match model {
                ImageEditModel::Gpt25Flare => ImageEditModel::Gpt25Flare.model_id(),
                ImageEditModel::Gpt25Sunburst => ImageEditModel::Gpt25Sunburst.model_id(),
            },
        ),
        ImageRoute::Flux(_) => (Provider::Fal, FLUX_COMPOSITE_ROUTE),
        ImageRoute::Bfl(_) => (Provider::Bfl, df_providers::BFL_FLUX3_IMAGE),
    }
}

/// The FLUX route's several fal calls per edit, recorded as one unpriced call until each sub-call
/// is metered (C-B4 in the cost engine backlog, `planning/issue-triage.md`).
const FLUX_COMPOSITE_ROUTE: &str = "flux-1.1-ultra-raw+kontext-max";

/// Which pool a picture waited in: `background` is look-ahead work.
const fn priority_label(priority: ImagePriority) -> &'static str {
    match priority {
        ImagePriority::Foreground => "foreground",
        ImagePriority::Presence => "presence",
        ImagePriority::Background => "background",
    }
}

/// The spec of one picture call: the cause from its kind, and a detail naming the pool (look-ahead
/// work is `background`), the imagery profile, the quality, the drawn size and the reference
/// count (what drives its cost).
pub(in crate::gameplay) fn image_call(
    route: &ImageRoute,
    model: ImageEditModel,
    kind: ImageKind,
    priority: ImagePriority,
    profile: &str,
    edit: &ImageEdit<'_>,
) -> CallSpec {
    let (provider, model) = image_route_model(route, model);
    CallSpec {
        provider,
        model: model.to_owned(),
        cause: image_cause(kind),
        detail: format!(
            "{}/{}/{profile}/{}/{}x{}/refs{}",
            kind.label(),
            priority_label(priority),
            quality_label(edit.quality),
            edit.width,
            edit.height,
            edit.references.len()
        ),
    }
}

/// Settles one picture call from its provider result: the reported tokens (OpenAI), the reported
/// credits (BFL) or one estimated call (FLUX); a rate limit or a rejected request was not billed;
/// any other failure is of unknown outcome.
pub(in crate::gameplay) fn settle_image_call(
    call: BilledCall,
    route: &ImageRoute,
    result: &Result<(GeneratedImage, Option<String>), ImageFailure>,
) {
    match result {
        Ok((generated, credits)) => call.settle(image_usage(route, generated, credits.as_deref())),
        Err(ImageFailure::RateLimited | ImageFailure::InvalidRequest) => call.not_billed(),
        Err(_) => drop(call),
    }
}

/// The tokens an OpenAI image call reported; a call that reported none is one estimated call.
pub(in crate::gameplay) fn image_token_usage(usage: ImageUsage) -> Usage {
    if usage == ImageUsage::default() {
        return Usage::estimated(vec![(Category::Calls, 1)]);
    }
    Usage::reported(vec![
        (Category::TextInput, usage.text_input_tokens),
        (Category::ImageInput, usage.image_input_tokens),
        (Category::ImageOutput, usage.output_tokens),
    ])
}

/// The spec of one character-clip frame edit (`fallen` or `guard`) by `model`.
pub(in crate::gameplay) fn clip_frame_call(model: ImageEditModel, fallen: bool) -> CallSpec {
    CallSpec {
        provider: Provider::OpenAi,
        model: model.model_id().to_owned(),
        cause: Cause::ClipFrame,
        detail: if fallen { "fallen" } else { "guard" }.to_owned(),
    }
}

fn image_usage(route: &ImageRoute, generated: &GeneratedImage, credits: Option<&str>) -> Usage {
    match route {
        ImageRoute::OpenAi(_) => image_token_usage(generated.usage),
        ImageRoute::Bfl(_) => {
            match credits.and_then(|credits| credits.trim().parse::<u64>().ok()) {
                Some(credits) => Usage::reported(vec![(Category::Credits, credits)]),
                None => Usage::estimated(vec![(Category::Calls, 1)]),
            }
        }
        ImageRoute::Flux(_) => Usage::estimated(vec![(Category::Calls, 1)]),
    }
}

/// The spec of one storyteller text call of `purpose`.
pub(in crate::gameplay) fn text_call(model: &str, purpose: &str) -> CallSpec {
    CallSpec {
        provider: Provider::OpenAi,
        model: model.to_owned(),
        cause: Cause::Storyteller,
        detail: purpose.to_owned(),
    }
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
    CallSpec {
        provider: Provider::ElevenLabs,
        model: model.to_owned(),
        cause,
        detail: detail.to_owned(),
    }
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
            | NativeHttpRefusal::InvalidEndpoint
            | NativeHttpRefusal::InvalidCredential
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
            let lines: Vec<String> = pending
                .into_iter()
                .map(|(spec, usage, at)| Self::record_locked(&mut state, spec, usage, at))
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
        let line = {
            let mut state = locked(&self.state);
            if state.opened_at_ms.is_none() {
                if state.pending.len() < PENDING_LIMIT {
                    state.pending.push((spec, usage, at_ms));
                } else {
                    state.pending_lost += 1;
                }
                return;
            }
            Self::record_locked(&mut state, spec, usage, at_ms)
        };
        super::actor::record_audit_line("gameplay.cost", line);
    }

    /// Numbers, prices, counts and queues one event; returns its `COST` line.
    fn record_locked(state: &mut BookState, spec: CallSpec, usage: Usage, at_ms: u64) -> String {
        state.next_call += 1;
        let number = state.next_call;
        let call_id = format!("{}-{number}", state.session);
        state.high_water.insert(state.session.clone(), number);
        let (lines, micro_usd) = state
            .prices
            .price(spec.provider, &spec.model, &usage.quantities);
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
        cost_line(
            &event,
            state.run_totals.micro_usd,
            state.session_totals.micro_usd,
        )
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
    let mut lines = csv.lines().filter(|line| !line.trim().is_empty());
    match lines.next().map(str::trim) {
        Some("provider,model,category,usd") => {}
        _ => return Err("the bill must start with the header provider,model,category,usd".into()),
    }
    let mut rows = BTreeMap::new();
    for (index, line) in lines.enumerate() {
        let row = index + 2;
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        let [provider, model, category, amount] = fields[..] else {
            return Err(format!("bill row {row} must have four fields"));
        };
        let provider =
            Provider::parse(provider).ok_or_else(|| format!("bill row {row}: unknown provider"))?;
        let category =
            Category::parse(category).ok_or_else(|| format!("bill row {row}: unknown category"))?;
        let nanos = parse_nano_usd(amount)
            .filter(|nanos| nanos % 1_000 == 0)
            .ok_or_else(|| {
                format!("bill row {row}: usd must be dollars with at most six decimals")
            })?;
        let total: &mut u64 = rows
            .entry((provider, model.to_owned(), category))
            .or_default();
        *total = total.saturating_add(nanos / 1_000);
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
    /// Metered; recorded unpriced for the stated reason.
    Unpriced(&'static str),
    /// Not metered yet; the reason names the backlog item.
    Pending(&'static str),
}

/// One billed route: the `df_providers` client, the provider, the model and the categories its
/// events carry.
#[derive(Clone, Copy, Debug)]
pub(in crate::gameplay) struct BilledRoute {
    pub(in crate::gameplay) client: &'static str,
    pub(in crate::gameplay) provider: Provider,
    pub(in crate::gameplay) model: &'static str,
    pub(in crate::gameplay) categories: &'static [Category],
    pub(in crate::gameplay) metering: Metering,
}

const IMAGE_TOKENS: &[Category] = &[
    Category::TextInput,
    Category::ImageInput,
    Category::ImageOutput,
];
const TEXT_TOKENS: &[Category] = &[
    Category::TextInput,
    Category::TextCachedInput,
    Category::TextCacheWrite,
    Category::TextOutput,
];

impl BilledRoute {
    fn json(&self) -> Value {
        let (metering, reason) = match self.metering {
            Metering::Priced => ("priced", ""),
            Metering::Unpriced(reason) => ("unpriced", reason),
            Metering::Pending(reason) => ("pending", reason),
        };
        json!({
            "client": self.client,
            "provider": self.provider.label(),
            "model": self.model,
            "categories": self
                .categories
                .iter()
                .map(|category| category.label())
                .collect::<Vec<_>>(),
            "metering": metering,
            "reason": reason,
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
        categories: &[Category::Credits],
        metering: Metering::Priced,
    },
    BilledRoute {
        client: "NativeFalImageProvider",
        provider: Provider::Fal,
        model: FLUX_COMPOSITE_ROUTE,
        categories: &[Category::Calls],
        metering: Metering::Unpriced("several fal calls per edit; C-B4 meters each"),
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
        metering: Metering::Pending("C-B1: speech_input.rs SttSecond charge"),
    },
    BilledRoute {
        client: "NativeEmbeddingProvider",
        provider: Provider::OpenAi,
        model: df_providers::EMBEDDING_MODEL_SMALL,
        categories: &[Category::TextInput],
        metering: Metering::Pending("C-B1: memory/embedder.rs embed_charged"),
    },
    BilledRoute {
        client: "NativeFalSegmenter",
        provider: Provider::Fal,
        model: "fal-ai/sam-3-1/image-rle",
        categories: &[Category::Calls],
        metering: Metering::Pending("C-B1: figure_masks.rs and plate_segments SegmentCall"),
    },
    BilledRoute {
        client: "NativeHeadLocator",
        provider: Provider::OpenAi,
        model: "gpt-5.5",
        categories: &[
            Category::TextInput,
            Category::ImageInput,
            Category::TextOutput,
        ],
        metering: Metering::Pending("C-B2: head locator calls are not charged at all today"),
    },
    BilledRoute {
        client: "NativePlateJudge",
        provider: Provider::OpenAi,
        model: "gpt-5.5",
        categories: &[
            Category::TextInput,
            Category::ImageInput,
            Category::TextOutput,
        ],
        metering: Metering::Pending("C-B2: photo checks are not charged at all today"),
    },
    BilledRoute {
        client: "NativeElevenMusicProvider",
        provider: Provider::ElevenLabs,
        model: "music_v2",
        categories: &[Category::AudioSeconds],
        metering: Metering::Pending("C-B1: place_music MusicCall"),
    },
    BilledRoute {
        client: "NativeFalMediaProvider",
        provider: Provider::Fal,
        model: df_providers::KLING_O1_IMAGE_TO_VIDEO,
        categories: &[Category::VideoSeconds],
        metering: Metering::Pending("C-B1: home_video and character_clips VideoClip"),
    },
    BilledRoute {
        client: "NativeWorldLabs",
        provider: Provider::WorldLabs,
        model: "marble-1.1",
        categories: &[Category::Credits],
        metering: Metering::Pending("C-B1: splat_world SplatCredit"),
    },
    BilledRoute {
        client: "SystemOneChoiceProvider",
        provider: Provider::TypeSafe,
        model: "jev-latest",
        categories: &[Category::TextInput],
        metering: Metering::Pending("C-B5: Jev calls report no usage; needs a df-providers hook"),
    },
];

#[cfg(test)]
#[path = "cost_tests.rs"]
mod tests;

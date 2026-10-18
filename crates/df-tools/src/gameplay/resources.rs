//! The resource generation hub: the one place the program asks for a generated resource (a
//! picture of a person, creature or place, a scene plate, a plate edit, an item close-up, a
//! splat-world place, and later a music set, a voice line or a video clip) and the one place that
//! remembers what was
//! asked for, what is in flight, what is ready, what failed and what it cost.
//!
//! # The contract
//!
//! * **A typed spec per kind.** A consumer describes what it needs with a [`ResourceSpec`]. The
//!   spec names its [`ResourceKind`], a per-kind [`ResourceSpec::CONTRACT_VERSION`], a canonical
//!   JSON description and a subject. The [`ResourceKey`] is the hash of the kind, the version and
//!   the canonical description, so a changed spec or a bumped version is a new key and a stale
//!   value cannot be served for it.
//! * **One API.** Every consumer calls [`Hub::get_or_request`] (queue a generation, idempotent per
//!   key) or [`Hub::render`] (run a generation now under the same bookkeeping). They read
//!   [`Hub::status`] and [`Hub::list`]; the dev tools call [`Hub::invalidate`] and
//!   [`Hub::regenerate`]. Nobody else keeps a cache index, an in-flight map, a failure memory, a
//!   retry loop, a spend record or a fallback; the bytes themselves stay in the owner's store (the
//!   imagery cache for pictures), which an entry's [`Generated::handle`] points at.
//! * **The hub owns** the registry by key, the one-generation-in-flight-per-key rule, the bounded
//!   timeout and attempt limit per kind ([`Limits`]), the failure memory (a permanent failure
//!   stays failed until `invalidate` or `regenerate`; a transient one is retried up to the attempt
//!   limit), the freshness check of a ready entry ([`Hub::set_validator`]), the shared
//!   [`SpendLedger`] (a record of what was spent; **generation has no spend cap of any kind**),
//!   the uniform `RESOURCE` and `SPEND` audit lines and the dev-CLI hooks.
//! * **Generation is on demand.** [`Hub::get_or_request`] is called by the server at the point a
//!   view first needs the resource; nothing is generated ahead of time, and no test or client
//!   render loop calls it. A failure keeps the kind's neutral chrome ([`Chrome`]) and is written
//!   to the audit log.
//! * **Who runs the work.** The hub does not spawn. A worker that owns a `JoinSet` calls
//!   [`Hub::take_jobs`] for as many jobs as it has room for, runs each [`Job`], and reports it
//!   with [`Hub::complete`]. That keeps every task owned, bounded and cancellable by the worker
//!   that already owns it (the living-scenes director).
//!
//! # Adding a resource kind (the exact API)
//!
//! 1. Implement [`ResourceSpec`] for the spec type: `KIND`, `CONTRACT_VERSION`, `canonical()`,
//!    `subject()`, `chrome()`. (Image kinds use the `image_spec!` types below.)
//! 2. Optionally tune the kind once at startup: [`Hub::set_limits`] (timeout at least the
//!    provider's own deadline, attempts), [`Hub::set_validator`] (is a ready entry's output still
//!    there?).
//! 3. Build a [`Maker`] (`Arc<dyn Fn() -> Generation>`), whose generation reserves from the ledger
//!    ([`SpendLedger::reserve`], which never refuses), calls the provider, then
//!    [`Reservation::commit`]s or drops/refunds it, and returns `Result<Generated, ResourceFailure>` with a permanent or transient failure.
//! 4. Call `hub.get_or_request(&spec, maker)` where a view first needs it, and a worker calls
//!    `hub.take_jobs(n)` / `job.execute().await` / `hub.complete(done)`. A kind whose request is
//!    built inline from live state (a scene plate) calls `hub.render(&spec, future)` instead.
//!
//! `MusicSet`, `VoiceLine` and `VideoClip` are reserved kinds with their [`Unit`]s: add the spec and
//! the maker, never a second cache or spend record.
//!
//! # Audit lines
//!
//! Every state change is one line through `actor::record_audit_line`:
//! `RESOURCE <request|ready|failed|invalidated|regenerate|stale> kind=<kind> key=<key>
//! subject=<subject> ms=<n> handle=<handle> error=<label>`, and every ledger movement
//! `SPEND <reserve|commit|refund|reconcile> unit=<unit> amount=<n> reserved=<n> committed=<n>`.

use std::{
    collections::BTreeMap,
    fmt,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

use df_render::art_contract::Neutral;
use sha2::{Digest, Sha256};

/// What a resource is. Each kind has its own contract version and limits.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::gameplay) enum ResourceKind {
    /// A person, creature, place, ground or prop picture (the art contract's depictions).
    Depiction,
    ScenePlate,
    PlateEdit,
    ItemCloseUp,
    /// The SAM 3.1 instances (masks, scores) of a scene plate for one noun phrase set: a hosted
    /// provider call, kept as a note beside the cached plate.
    PlateSegments,
    /// Who and where everyone is in a scene plate (identity, facing, layout): derived from the
    /// plate's segments and the vision judge, kept as a note beside the cached plate.
    PlateStaging,
    /// A generated 3D splat world for a place.
    SplatWorld,
    /// Reserved for the music worker.
    MusicSet,
    /// Reserved for the voice migration.
    VoiceLine,
    /// Reserved for the video migration.
    VideoClip,
}

impl ResourceKind {
    pub(in crate::gameplay) const fn label(self) -> &'static str {
        match self {
            Self::Depiction => "depiction",
            Self::ScenePlate => "scene_plate",
            Self::PlateEdit => "plate_edit",
            Self::ItemCloseUp => "item_close_up",
            Self::PlateSegments => "plate_segments",
            Self::PlateStaging => "plate_staging",
            Self::SplatWorld => "splat_world",
            Self::MusicSet => "music_set",
            Self::VoiceLine => "voice_line",
            Self::VideoClip => "video_clip",
        }
    }
}

/// The stable identity of a resource: a hash of its kind, contract version and canonical spec.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::gameplay) struct ResourceKey(String);

impl ResourceKey {
    pub(in crate::gameplay) fn of<S: ResourceSpec>(spec: &S) -> Self {
        let text = format!(
            "{}|{}|{}",
            S::KIND.label(),
            S::CONTRACT_VERSION,
            spec.canonical()
        );
        let digest = Sha256::digest(text.as_bytes());
        Self(digest.iter().take(16).map(|b| format!("{b:02x}")).collect())
    }

    pub(in crate::gameplay) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ResourceKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// What the display shows when there is no resource: the art contract's neutral chrome.
pub(in crate::gameplay) type Chrome = Neutral;

/// A typed description of one resource. Implement this for each spec type.
pub(in crate::gameplay) trait ResourceSpec {
    const KIND: ResourceKind;
    /// Bump when the way this kind is made changes: every old key stops being served.
    const CONTRACT_VERSION: u32;
    /// The complete, order-stable description of what is wanted, as compact JSON text.
    fn canonical(&self) -> String;
    /// What the resource is about, for `invalidate(subject)` and the audit lines (for example
    /// `character:ant-wolf`).
    fn subject(&self) -> String;
    /// What stands until the resource is ready, or when it cannot be made.
    fn chrome(&self) -> Chrome;
}

/// Why a generation did not produce a resource.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum ResourceFailure {
    /// The same request would fail again (a refusal, an invalid or unusable result): remembered
    /// as failed, with its reason, until `invalidate` or `regenerate`; never retried.
    Permanent(&'static str),
    /// A provider or transport failure that may pass: retried up to the kind's attempt limit.
    Transient(&'static str),
    /// The generation ran past its timeout (transient).
    TimedOut,
    /// The generation was dropped (cancelled or its worker ended); transient.
    Interrupted,
}

impl ResourceFailure {
    pub(in crate::gameplay) fn label(&self) -> &'static str {
        match self {
            Self::Permanent(label) | Self::Transient(label) => label,
            Self::TimedOut => "timed_out",
            Self::Interrupted => "interrupted",
        }
    }

    /// Whether asking again before `invalidate` or `regenerate` could succeed.
    pub(in crate::gameplay) fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Transient(_) | Self::TimedOut | Self::Interrupted
        )
    }
}

/// What a finished generation hands back: an opaque handle the consumer understands (for an
/// image, the imagery cache key) and a short human note for the dev tools.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct Generated {
    pub(in crate::gameplay) handle: String,
    pub(in crate::gameplay) note: String,
}

impl Generated {
    pub(in crate::gameplay) fn new(handle: impl Into<String>) -> Self {
        Self {
            handle: handle.into(),
            note: String::new(),
        }
    }
}

/// One generation: the future a worker runs.
pub(in crate::gameplay) type Generation =
    Pin<Box<dyn Future<Output = Result<Generated, ResourceFailure>> + Send>>;

/// Builds the generation again whenever it must run (first request, retry or regeneration). It
/// holds only what the generation needs and must not keep the owner of the hub alive.
pub(in crate::gameplay) type Maker = Arc<dyn Fn() -> Generation + Send + Sync>;

/// Whether a ready entry's output still exists (for an image, the cache file).
pub(in crate::gameplay) type Validator = Arc<dyn Fn(&Generated) -> bool + Send + Sync>;

/// The bounds one kind runs under.
#[derive(Clone, Copy, Debug)]
pub(in crate::gameplay) struct Limits {
    /// A generation running longer than this fails as `TimedOut`. Set it to at least the
    /// provider's own deadline (plus any wait for a free provider slot), or the hub fires first.
    pub(in crate::gameplay) timeout: Duration,
    /// Attempts per key (first try included) before a transient failure stays failed until
    /// `regenerate` or `invalidate`.
    pub(in crate::gameplay) attempts: u8,
}

impl Limits {
    /// The defaults per kind; production start-up tunes the image kinds to the provider deadline.
    pub(in crate::gameplay) const fn for_kind(kind: ResourceKind) -> Self {
        let seconds = match kind {
            ResourceKind::Depiction
            | ResourceKind::ScenePlate
            | ResourceKind::PlateEdit
            | ResourceKind::ItemCloseUp => 180,
            // The fal call has its own deadline; the staging runs two vision calls after it.
            ResourceKind::PlateSegments => 120,
            ResourceKind::PlateStaging => 240,
            // Submit, a 15-minute poll, export and two provider-bounded downloads.
            ResourceKind::SplatWorld => 1_620,
            ResourceKind::MusicSet => 600,
            ResourceKind::VoiceLine => 90,
            ResourceKind::VideoClip => 900,
        };
        Self {
            timeout: Duration::from_secs(seconds),
            attempts: 2,
        }
    }
}

/// What a consumer is told about a resource.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum ResourceStatus {
    /// Made and cached; `handle` points at the bytes.
    Ready(Generated),
    /// Requested or being made; the chrome stands meanwhile.
    Pending(Chrome),
    /// The last attempt failed; the chrome stands and the failure is in the audit log.
    Failed(ResourceFailure, Chrome),
    /// Nothing is known under this key.
    Fallback(Chrome),
}

/// What a spend is counted in. One ledger holds one account per unit.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::gameplay) enum Unit {
    /// One provider image call.
    ImageCall,
    VideoClip,
    VoiceCharacter,
    SttSecond,
    TextToken,
    MusicCall,
    /// One input token of a text-embedding call (the story memory).
    EmbeddingToken,
    /// One hosted segmentation call (SAM on fal); its price is the model's, in the audit line.
    SegmentCall,
    /// One World Labs Marble credit; a marble-1.1 world records 1,580.
    SplatCredit,
}

impl Unit {
    pub(in crate::gameplay) const fn label(self) -> &'static str {
        match self {
            Self::ImageCall => "image_call",
            Self::VideoClip => "video_clip",
            Self::VoiceCharacter => "voice_character",
            Self::SttSecond => "stt_second",
            Self::TextToken => "text_token",
            Self::MusicCall => "music_call",
            Self::EmbeddingToken => "embedding_token",
            Self::SegmentCall => "segment_call",
            Self::SplatCredit => "splat_credit",
        }
    }
}

/// One account of the ledger.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::gameplay) struct Spend {
    /// Held by reservations not yet settled.
    pub(in crate::gameplay) reserved: u64,
    /// Charged and kept.
    pub(in crate::gameplay) committed: u64,
}

/// The one spend ledger of the program: a record of what every provider call cost, per [`Unit`],
/// with reserve, commit, refund-if-not-sent and reconcile. **It never refuses.** Generation has
/// no spend cap of any kind (not per session, per run or per kind): a reservation always
/// succeeds and a resource never fails for want of budget. What the ledger gives is the truth
/// about spend, in the `SPEND` audit lines and in [`SpendLedger::report`] for the dev tools.
///
/// * [`SpendLedger::reserve`] records `amount` of a unit as held.
/// * [`Reservation::commit`] settles it as spent (the call was sent, whatever its outcome); the
///   actual amount may differ from the reservation (a provider-reported usage).
/// * Dropping a [`Reservation`], or [`Reservation::refund`], returns it (the call was not sent).
/// * [`SpendLedger::reconcile`] corrects the committed total later from usage evidence.
#[derive(Default)]
pub(in crate::gameplay) struct SpendLedger {
    accounts: Mutex<BTreeMap<Unit, Spend>>,
}

fn spend_audit(event: &str, unit: Unit, amount: u64, account: Spend) {
    super::actor::record_audit_line(
        "gameplay.spend",
        format!(
            "SPEND {event} unit={} amount={amount} reserved={} committed={}",
            unit.label(),
            account.reserved,
            account.committed
        ),
    );
}

impl SpendLedger {
    /// Records `amount` of `unit` as held by a call about to be made. Never refused.
    pub(in crate::gameplay) fn reserve(self: &Arc<Self>, unit: Unit, amount: u64) -> Reservation {
        let mut accounts = locked(&self.accounts);
        let account = accounts.entry(unit).or_default();
        account.reserved = account.reserved.saturating_add(amount);
        spend_audit("reserve", unit, amount, *account);
        Reservation {
            ledger: Arc::clone(self),
            unit,
            amount,
            settled: false,
        }
    }

    /// Corrects the committed total of a unit by `delta` (usage evidence that arrived later).
    pub(in crate::gameplay) fn reconcile(&self, unit: Unit, delta: i64) {
        let mut accounts = locked(&self.accounts);
        let account = accounts.entry(unit).or_default();
        account.committed = if delta >= 0 {
            account.committed.saturating_add(delta.unsigned_abs())
        } else {
            account.committed.saturating_sub(delta.unsigned_abs())
        };
        spend_audit("reconcile", unit, delta.unsigned_abs(), *account);
    }

    /// The account of one unit.
    pub(in crate::gameplay) fn snapshot(&self, unit: Unit) -> Spend {
        locked(&self.accounts)
            .get(&unit)
            .copied()
            .unwrap_or_default()
    }

    /// Every account that has seen a movement: the dev tools' read of the ledger.
    pub(in crate::gameplay) fn report(&self) -> Vec<(Unit, Spend)> {
        locked(&self.accounts)
            .iter()
            .map(|(unit, spend)| (*unit, *spend))
            .collect()
    }
}

/// A held amount of one unit. Settled by `commit` or `refund`; dropped unsettled it is refunded.
#[must_use = "a reservation is held until it is committed or refunded"]
pub(in crate::gameplay) struct Reservation {
    ledger: Arc<SpendLedger>,
    unit: Unit,
    amount: u64,
    settled: bool,
}

impl Reservation {
    fn release(&mut self, committed: u64) -> Spend {
        self.settled = true;
        let mut accounts = locked(&self.ledger.accounts);
        let account = accounts.entry(self.unit).or_default();
        account.reserved = account.reserved.saturating_sub(self.amount);
        account.committed = account.committed.saturating_add(committed);
        *account
    }

    /// The call was sent: keep `actual` as spent (whatever the outcome; a call of unknown outcome
    /// may have been billed).
    pub(in crate::gameplay) fn commit(mut self, actual: u64) {
        let account = self.release(actual);
        spend_audit("commit", self.unit, actual, account);
    }

    /// The call was not sent: give the reservation back.
    pub(in crate::gameplay) fn refund(mut self) {
        let account = self.release(0);
        spend_audit("refund", self.unit, self.amount, account);
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.settled {
            let account = self.release(0);
            spend_audit("refund", self.unit, self.amount, account);
        }
    }
}

#[derive(Clone)]
enum State {
    Queued,
    InFlight { since: Instant },
    Ready(Generated),
    Failed(ResourceFailure),
}

struct Entry {
    kind: ResourceKind,
    subject: String,
    chrome: Chrome,
    maker: Option<Maker>,
    state: State,
    attempts: u8,
    /// How long the last attempt took.
    elapsed: Option<Duration>,
}

#[derive(Default)]
struct Registry {
    entries: BTreeMap<ResourceKey, Entry>,
    limits: BTreeMap<ResourceKind, Limits>,
    validators: BTreeMap<ResourceKind, Validator>,
}

impl Registry {
    fn limits_of(&self, kind: ResourceKind) -> Limits {
        self.limits
            .get(&kind)
            .copied()
            .unwrap_or(Limits::for_kind(kind))
    }

    /// A ready entry whose output has gone is no longer ready.
    fn ready_is_stale(&self, entry: &Entry) -> bool {
        match (&entry.state, self.validators.get(&entry.kind)) {
            (State::Ready(generated), Some(validator)) => !validator(generated),
            _ => false,
        }
    }
}

/// A queued generation for a worker to run.
pub(in crate::gameplay) struct Job {
    pub(in crate::gameplay) key: ResourceKey,
    pub(in crate::gameplay) run: Generation,
    timeout: Duration,
}

impl Job {
    /// Runs the generation under its timeout. The worker then calls [`Hub::complete`].
    pub(in crate::gameplay) async fn execute(self) -> Completed {
        let result = match tokio::time::timeout(self.timeout, self.run).await {
            Ok(result) => result,
            Err(_) => Err(ResourceFailure::TimedOut),
        };
        Completed {
            key: self.key,
            result,
        }
    }
}

/// A finished job, for [`Hub::complete`].
pub(in crate::gameplay) struct Completed {
    pub(in crate::gameplay) key: ResourceKey,
    pub(in crate::gameplay) result: Result<Generated, ResourceFailure>,
}

/// One row of [`Hub::list`], for the dev tools.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct ResourceRow {
    pub(in crate::gameplay) key: ResourceKey,
    pub(in crate::gameplay) kind: ResourceKind,
    pub(in crate::gameplay) subject: String,
    /// `queued`, `in_flight`, `ready` or `failed`.
    pub(in crate::gameplay) state: &'static str,
    pub(in crate::gameplay) handle: String,
    pub(in crate::gameplay) last_error: String,
    pub(in crate::gameplay) attempts: u8,
}

/// What a subject's depiction resolves to, for the dev tools (see
/// `LivingScenes::describe_depiction`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct DepictionReport {
    pub(in crate::gameplay) subject: String,
    pub(in crate::gameplay) role: df_render::art_contract::Role,
    /// `live` (a generated image), `reviewed` (a library photograph) or `neutral` (chrome).
    pub(in crate::gameplay) source: &'static str,
    /// The live path or the served library path; empty for the neutral chrome.
    pub(in crate::gameplay) asset: String,
    /// `none`, `pending`, `ready` or `failed`.
    pub(in crate::gameplay) status: &'static str,
    pub(in crate::gameplay) last_error: String,
}

/// What [`Hub::invalidate`] drops.
pub(in crate::gameplay) enum Invalidate<'a> {
    Key(&'a ResourceKey),
    Subject(&'a str),
}

/// Declares an image-backed spec type: the subject it is about, the imagery key of its request
/// (so a changed request is a new resource) and the chrome that stands meanwhile.
macro_rules! image_spec {
    ($(#[$meta:meta])* $name:ident, $kind:expr, $version:expr) => {
        $(#[$meta])*
        #[derive(Clone, Debug)]
        pub(in crate::gameplay) struct $name {
            subject: String,
            request_key: String,
            chrome: Chrome,
        }

        impl $name {
            pub(in crate::gameplay) fn new(subject: &str, request_key: &str, chrome: Chrome) -> Self {
                Self {
                    subject: subject.to_owned(),
                    request_key: request_key.to_owned(),
                    chrome,
                }
            }
        }

        impl ResourceSpec for $name {
            const KIND: ResourceKind = $kind;
            const CONTRACT_VERSION: u32 = $version;

            fn canonical(&self) -> String {
                serde_json::json!({"subject": self.subject, "request": self.request_key})
                    .to_string()
            }

            fn subject(&self) -> String {
                self.subject.clone()
            }

            fn chrome(&self) -> Chrome {
                self.chrome
            }
        }
    };
}

image_spec!(
    /// A person, creature or place picture, including its expression, health and refresh variants.
    DepictionSpec,
    ResourceKind::Depiction,
    1
);
image_spec!(
    /// One composed scene plate.
    PlateSpec,
    ResourceKind::ScenePlate,
    1
);
image_spec!(
    /// An edit of a scene plate.
    PlateEditSpec,
    ResourceKind::PlateEdit,
    1
);
image_spec!(
    /// A close-up photograph of one object.
    ItemCloseUpSpec,
    ResourceKind::ItemCloseUp,
    1
);

/// The supported generated splat-world encodings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum SplatWorldFormat {
    Ply,
    Sog,
}

impl SplatWorldFormat {
    const fn label(self) -> &'static str {
        match self {
            Self::Ply => "ply",
            Self::Sog => "sog",
        }
    }
}

/// A place-specific request for a Gaussian-splat world.
#[derive(Clone, Debug)]
pub(in crate::gameplay) struct SplatWorldSpec {
    place_id: String,
    prompt_facts: Vec<String>,
    format: SplatWorldFormat,
}

impl SplatWorldSpec {
    pub(in crate::gameplay) fn new(
        place_id: impl Into<String>,
        prompt_facts: Vec<String>,
        format: SplatWorldFormat,
    ) -> Self {
        Self {
            place_id: place_id.into(),
            prompt_facts,
            format,
        }
    }

    /// The place this world is for.
    pub(in crate::gameplay) fn place_id(&self) -> &str {
        &self.place_id
    }

    /// The place facts the photographic prompt is built from.
    pub(in crate::gameplay) fn prompt_facts(&self) -> &[String] {
        &self.prompt_facts
    }

    /// The requested splat encoding.
    pub(in crate::gameplay) fn format(&self) -> SplatWorldFormat {
        self.format
    }
}

impl ResourceSpec for SplatWorldSpec {
    const KIND: ResourceKind = ResourceKind::SplatWorld;
    const CONTRACT_VERSION: u32 = 1;

    fn canonical(&self) -> String {
        serde_json::json!({
            "place_id": self.place_id,
            "prompt_facts": self.prompt_facts,
            "format": self.format.label(),
        })
        .to_string()
    }

    fn subject(&self) -> String {
        format!("place:{}", self.place_id)
    }

    fn chrome(&self) -> Chrome {
        Neutral::Nothing
    }
}

/// The hub handle: cheap to clone, shared by every consumer.
#[derive(Clone, Default)]
pub(in crate::gameplay) struct Hub {
    registry: Arc<Mutex<Registry>>,
    ledger: Arc<SpendLedger>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn audit(state: &str, key: &ResourceKey, entry: &Entry, ms: u128, handle: &str, error: &str) {
    super::actor::record_audit_line(
        "gameplay.resource",
        resource_audit_line(state, key, entry, ms, handle, error),
    );
}

fn resource_audit_line(
    state: &str,
    key: &ResourceKey,
    entry: &Entry,
    ms: u128,
    handle: &str,
    error: &str,
) -> String {
    format!(
        "RESOURCE {state} kind={} key={key} subject={} ms={ms} handle={handle} error={error}",
        entry.kind.label(),
        entry.subject
    )
}

impl Hub {
    pub(in crate::gameplay) fn new() -> Self {
        Self::default()
    }

    /// The one spend ledger. Providers reserve from it; the image budget is its `ImageCall`
    /// account.
    pub(in crate::gameplay) fn ledger(&self) -> &Arc<SpendLedger> {
        &self.ledger
    }

    /// Sets the bounds of one kind (the per-kind default applies otherwise).
    pub(in crate::gameplay) fn set_limits(&self, kind: ResourceKind, limits: Limits) {
        locked(&self.registry).limits.insert(kind, limits);
    }

    /// Sets how a ready entry of `kind` is checked for freshness: an entry whose output has gone
    /// (a deleted cache file) is generated again instead of served.
    pub(in crate::gameplay) fn set_validator(&self, kind: ResourceKind, validator: Validator) {
        locked(&self.registry).validators.insert(kind, validator);
    }

    /// The consumer's one question: give me this resource. A ready resource is returned; an
    /// unknown one is queued once (a second ask while it is queued, in flight or ready does not
    /// start another generation) and `Pending` is returned with the kind's chrome. A ready
    /// resource whose output has gone is queued again. A failed one stays failed until
    /// [`Hub::regenerate`] or [`Hub::invalidate`] (a transient failure is retried by the worker up
    /// to the attempt limit before that).
    pub(in crate::gameplay) fn get_or_request<S: ResourceSpec>(
        &self,
        spec: &S,
        maker: Maker,
    ) -> ResourceStatus {
        let key = ResourceKey::of(spec);
        let mut registry = locked(&self.registry);
        if let Some(entry) = registry.entries.get(&key) {
            if !registry.ready_is_stale(entry) {
                return Self::status_of(entry);
            }
            audit("stale", &key, entry, 0, "", "output_missing");
        }
        let entry = Entry {
            kind: S::KIND,
            subject: spec.subject(),
            chrome: spec.chrome(),
            maker: Some(maker),
            state: State::Queued,
            attempts: 0,
            elapsed: None,
        };
        audit("request", &key, &entry, 0, "", "");
        let status = Self::status_of(&entry);
        registry.entries.insert(key, entry);
        status
    }

    /// Runs a generation now under the hub's bookkeeping: the entry is `InFlight` while it runs,
    /// `Ready` or `Failed` after, and the audit lines are the same as for a queued generation. For
    /// a kind whose request is built inline from live state (a scene plate); the spec's canonical
    /// text must carry the request's own content key so a changed request is a new resource.
    ///
    /// A ready entry whose output still exists is returned without running; a permanent failure,
    /// or a transient one past the attempt limit, is returned without running (nothing is spent
    /// again). Two concurrent calls for one key both run; the provider layer coalesces the
    /// identical request, so it is still made once.
    pub(in crate::gameplay) async fn render<S: ResourceSpec>(
        &self,
        spec: &S,
        generation: Generation,
    ) -> Result<Generated, ResourceFailure> {
        let key = ResourceKey::of(spec);
        let limits;
        {
            let mut registry = locked(&self.registry);
            limits = registry.limits_of(S::KIND);
            let stale = registry
                .entries
                .get(&key)
                .is_some_and(|entry| registry.ready_is_stale(entry));
            if let Some(entry) = registry.entries.get(&key) {
                match &entry.state {
                    State::Ready(generated) if !stale => return Ok(generated.clone()),
                    State::Failed(failure)
                        if !failure.is_retryable() || entry.attempts >= limits.attempts =>
                    {
                        return Err(failure.clone());
                    }
                    _ => {}
                }
                if stale {
                    audit("stale", &key, entry, 0, "", "output_missing");
                }
            }
            let entry = registry
                .entries
                .entry(key.clone())
                .or_insert_with(|| Entry {
                    kind: S::KIND,
                    subject: spec.subject(),
                    chrome: spec.chrome(),
                    maker: None,
                    state: State::Queued,
                    attempts: 0,
                    elapsed: None,
                });
            if stale {
                entry.attempts = 0;
            }
            entry.state = State::InFlight {
                since: Instant::now(),
            };
            entry.attempts = entry.attempts.saturating_add(1);
            audit("request", &key, entry, 0, "", "");
        }
        let started = Instant::now();
        let result = match tokio::time::timeout(limits.timeout, generation).await {
            Ok(result) => result,
            Err(_) => Err(ResourceFailure::TimedOut),
        };
        self.finish(&key, result.clone(), started.elapsed());
        result
    }

    /// Hands out up to `available` queued generations, oldest key first, marking them in flight.
    pub(in crate::gameplay) fn take_jobs(&self, available: usize) -> Vec<Job> {
        let mut registry = locked(&self.registry);
        let limits = registry.limits.clone();
        let mut jobs = Vec::new();
        for (key, entry) in &mut registry.entries {
            if jobs.len() >= available {
                break;
            }
            if !matches!(entry.state, State::Queued) {
                continue;
            }
            let Some(maker) = entry.maker.clone() else {
                continue;
            };
            entry.state = State::InFlight {
                since: Instant::now(),
            };
            entry.attempts = entry.attempts.saturating_add(1);
            jobs.push(Job {
                key: key.clone(),
                run: maker(),
                timeout: limits
                    .get(&entry.kind)
                    .copied()
                    .unwrap_or(Limits::for_kind(entry.kind))
                    .timeout,
            });
        }
        jobs
    }

    /// Records a finished job. A retryable failure is re-queued below the kind's attempt limit;
    /// a permanent one, or one past the limit, stays failed until `regenerate` or `invalidate`.
    pub(in crate::gameplay) fn complete(&self, completed: Completed) {
        let elapsed = {
            let registry = locked(&self.registry);
            registry
                .entries
                .get(&completed.key)
                .and_then(|entry| match entry.state {
                    State::InFlight { since } => Some(since.elapsed()),
                    _ => None,
                })
                .unwrap_or_default()
        };
        self.finish(&completed.key, completed.result, elapsed);
    }

    fn finish(
        &self,
        key: &ResourceKey,
        result: Result<Generated, ResourceFailure>,
        took: Duration,
    ) {
        let mut registry = locked(&self.registry);
        let limits = registry.limits.clone();
        let Some(entry) = registry.entries.get_mut(key) else {
            // Invalidated while it ran: the result is stale and is dropped.
            return;
        };
        if !matches!(entry.state, State::InFlight { .. }) {
            return;
        }
        entry.elapsed = Some(took);
        let ms = took.as_millis();
        match result {
            Ok(generated) => {
                audit("ready", key, entry, ms, &generated.handle, "");
                entry.state = State::Ready(generated);
            }
            Err(failure) => {
                let attempts = limits
                    .get(&entry.kind)
                    .copied()
                    .unwrap_or(Limits::for_kind(entry.kind))
                    .attempts;
                audit("failed", key, entry, ms, "", failure.label());
                if failure.is_retryable() && entry.attempts < attempts && entry.maker.is_some() {
                    entry.state = State::Queued;
                } else {
                    entry.state = State::Failed(failure);
                }
            }
        }
    }

    fn status_of(entry: &Entry) -> ResourceStatus {
        match &entry.state {
            State::Queued | State::InFlight { .. } => ResourceStatus::Pending(entry.chrome),
            State::Ready(generated) => ResourceStatus::Ready(generated.clone()),
            State::Failed(failure) => ResourceStatus::Failed(failure.clone(), entry.chrome),
        }
    }

    /// The status of a key; `Fallback` (with the generic nothing chrome) when nothing is known. A
    /// ready entry whose output has gone reads as `Pending` (it is generated again on the next
    /// ask).
    pub(in crate::gameplay) fn status(&self, key: &ResourceKey) -> ResourceStatus {
        let registry = locked(&self.registry);
        registry
            .entries
            .get(key)
            .map_or(ResourceStatus::Fallback(Neutral::Nothing), |entry| {
                if registry.ready_is_stale(entry) {
                    ResourceStatus::Pending(entry.chrome)
                } else {
                    Self::status_of(entry)
                }
            })
    }

    /// The handle of a ready resource whose output still exists.
    pub(in crate::gameplay) fn ready(&self, key: &ResourceKey) -> Option<Generated> {
        match self.status(key) {
            ResourceStatus::Ready(generated) => Some(generated),
            _ => None,
        }
    }

    /// Every entry, optionally of one kind: in flight, cached and failed, for the dev tools.
    pub(in crate::gameplay) fn list(&self, kind: Option<ResourceKind>) -> Vec<ResourceRow> {
        locked(&self.registry)
            .entries
            .iter()
            .filter(|(_, entry)| kind.is_none_or(|kind| entry.kind == kind))
            .map(|(key, entry)| {
                let (state, handle, last_error) = match &entry.state {
                    State::Queued => ("queued", String::new(), String::new()),
                    State::InFlight { .. } => ("in_flight", String::new(), String::new()),
                    State::Ready(generated) => ("ready", generated.handle.clone(), String::new()),
                    State::Failed(failure) => ("failed", String::new(), failure.label().to_owned()),
                };
                ResourceRow {
                    key: key.clone(),
                    kind: entry.kind,
                    subject: entry.subject.clone(),
                    state,
                    handle,
                    last_error,
                    attempts: entry.attempts,
                }
            })
            .collect()
    }

    /// Every entry about one subject.
    pub(in crate::gameplay) fn for_subject(&self, subject: &str) -> Vec<ResourceRow> {
        self.list(None)
            .into_iter()
            .filter(|row| row.subject == subject)
            .collect()
    }

    /// Drops entries, failures included, so the next need asks again (an entry in flight finishes
    /// and is then dropped as stale). Returns how many were dropped. This is the only failure
    /// memory there is: nothing below the hub remembers a failure.
    pub(in crate::gameplay) fn invalidate(&self, target: Invalidate<'_>) -> usize {
        let mut registry = locked(&self.registry);
        let keys: Vec<ResourceKey> = registry
            .entries
            .iter()
            .filter(|(key, entry)| match &target {
                Invalidate::Key(wanted) => *wanted == *key,
                Invalidate::Subject(subject) => entry.subject == *subject,
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in &keys {
            if let Some(entry) = registry.entries.remove(key) {
                audit("invalidated", key, &entry, 0, "", "");
            }
        }
        keys.len()
    }

    /// Re-queues a resource now, clearing its failure and attempts (idempotent: one in-flight
    /// generation per key). Needs the entry's maker, so it works for queued kinds; an
    /// inline-rendered entry is invalidated instead and its consumer asks again. Returns whether
    /// a generation was queued.
    pub(in crate::gameplay) fn regenerate(&self, key: &ResourceKey) -> bool {
        let mut registry = locked(&self.registry);
        let Some(entry) = registry.entries.get_mut(key) else {
            return false;
        };
        if matches!(entry.state, State::Queued | State::InFlight { .. }) || entry.maker.is_none() {
            return false;
        }
        entry.state = State::Queued;
        entry.attempts = 0;
        audit("regenerate", key, entry, 0, "", "");
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    #[derive(Clone)]
    struct TestSpec {
        subject: &'static str,
        text: &'static str,
    }

    impl ResourceSpec for TestSpec {
        const KIND: ResourceKind = ResourceKind::Depiction;
        const CONTRACT_VERSION: u32 = 1;
        fn canonical(&self) -> String {
            format!("{{\"text\":\"{}\"}}", self.text)
        }
        fn subject(&self) -> String {
            format!("character:{}", self.subject)
        }
        fn chrome(&self) -> Chrome {
            Neutral::Silhouette
        }
    }

    struct VersionTwo(TestSpec);
    impl ResourceSpec for VersionTwo {
        const KIND: ResourceKind = ResourceKind::Depiction;
        const CONTRACT_VERSION: u32 = 2;
        fn canonical(&self) -> String {
            self.0.canonical()
        }
        fn subject(&self) -> String {
            self.0.subject()
        }
        fn chrome(&self) -> Chrome {
            Neutral::Silhouette
        }
    }

    fn ok(handle: &'static str) -> Maker {
        Arc::new(move || Box::pin(async move { Ok(Generated::new(handle)) }))
    }

    fn transient() -> Maker {
        Arc::new(|| Box::pin(async { Err(ResourceFailure::Transient("provider")) }))
    }

    fn wolf() -> TestSpec {
        TestSpec {
            subject: "ant-wolf",
            text: "wolf",
        }
    }

    async fn run_all(hub: &Hub) {
        for job in hub.take_jobs(8) {
            hub.complete(job.execute().await);
        }
    }

    #[test]
    fn a_changed_spec_or_contract_version_is_a_new_key() {
        let base = ResourceKey::of(&wolf());
        assert_eq!(base, ResourceKey::of(&wolf()));
        let changed = TestSpec {
            subject: "ant-wolf",
            text: "dire wolf",
        };
        assert_ne!(base, ResourceKey::of(&changed));
        assert_ne!(base, ResourceKey::of(&VersionTwo(wolf())));
    }

    #[test]
    fn splat_world_keys_are_stable_and_cover_place_facts_and_format() {
        let spec = SplatWorldSpec::new(
            "harbor",
            vec!["stone quay".to_owned(), "red lighthouse".to_owned()],
            SplatWorldFormat::Ply,
        );
        let key = ResourceKey::of(&spec);
        assert_eq!(key, ResourceKey::of(&spec));
        assert_ne!(
            key,
            ResourceKey::of(&SplatWorldSpec::new(
                "harbor",
                vec!["stone quay".to_owned(), "red lighthouse".to_owned()],
                SplatWorldFormat::Sog,
            ))
        );
        assert_ne!(
            key,
            ResourceKey::of(&SplatWorldSpec::new(
                "harbor",
                vec!["stone quay".to_owned()],
                SplatWorldFormat::Ply,
            ))
        );
        assert_ne!(
            key,
            ResourceKey::of(&SplatWorldSpec::new(
                "market",
                vec!["stone quay".to_owned(), "red lighthouse".to_owned()],
                SplatWorldFormat::Ply,
            ))
        );
    }

    #[test]
    fn splat_world_resource_audit_uses_the_hub_line_shape() {
        let spec = SplatWorldSpec::new("harbor", Vec::new(), SplatWorldFormat::Sog);
        let key = ResourceKey::of(&spec);
        let entry = Entry {
            kind: ResourceKind::SplatWorld,
            subject: spec.subject(),
            chrome: spec.chrome(),
            maker: None,
            state: State::Failed(ResourceFailure::Permanent("unavailable")),
            attempts: 1,
            elapsed: Some(Duration::ZERO),
        };

        assert_eq!(
            resource_audit_line("failed", &key, &entry, 0, "", "unavailable"),
            format!(
                "RESOURCE failed kind=splat_world key={key} subject=place:harbor ms=0 handle= error=unavailable"
            )
        );
    }

    #[tokio::test]
    async fn asking_twice_queues_one_generation_and_the_chrome_stands_until_ready() {
        let hub = Hub::new();
        assert_eq!(
            hub.get_or_request(&wolf(), ok("key-1")),
            ResourceStatus::Pending(Neutral::Silhouette)
        );
        assert_eq!(
            hub.get_or_request(&wolf(), ok("key-2")),
            ResourceStatus::Pending(Neutral::Silhouette)
        );
        let jobs = hub.take_jobs(8);
        assert_eq!(jobs.len(), 1, "one request in flight per key");
        assert!(
            hub.take_jobs(8).is_empty(),
            "an in-flight key is not taken again"
        );
        for job in jobs {
            hub.complete(job.execute().await);
        }
        let key = ResourceKey::of(&wolf());
        assert_eq!(hub.ready(&key), Some(Generated::new("key-1")));
        assert_eq!(
            hub.get_or_request(&wolf(), ok("key-3")),
            ResourceStatus::Ready(Generated::new("key-1"))
        );
    }

    #[tokio::test]
    async fn take_jobs_is_bounded_by_what_the_worker_has_room_for() {
        let hub = Hub::new();
        for (subject, text) in [("a", "a"), ("b", "b"), ("c", "c")] {
            hub.get_or_request(&TestSpec { subject, text }, ok("h"));
        }
        assert_eq!(hub.take_jobs(2).len(), 2);
        assert_eq!(hub.take_jobs(2).len(), 1);
        assert_eq!(hub.take_jobs(2).len(), 0);
    }

    #[tokio::test]
    async fn a_transient_failure_is_retried_to_the_attempt_limit_then_stays_failed() {
        let hub = Hub::new();
        hub.get_or_request(&wolf(), transient());
        run_all(&hub).await;
        let key = ResourceKey::of(&wolf());
        assert!(
            matches!(hub.status(&key), ResourceStatus::Pending(_)),
            "queued for a retry"
        );
        run_all(&hub).await;
        assert_eq!(
            hub.status(&key),
            ResourceStatus::Failed(ResourceFailure::Transient("provider"), Neutral::Silhouette)
        );
        assert!(hub.take_jobs(8).is_empty());
        let rows = hub.list(Some(ResourceKind::Depiction));
        assert_eq!(
            (rows[0].state, rows[0].last_error.as_str()),
            ("failed", "provider")
        );
        assert!(hub.regenerate(&key));
        assert!(!hub.regenerate(&key), "idempotent while queued");
    }

    #[tokio::test]
    async fn a_permanent_failure_is_never_retried_and_names_its_reason() {
        let hub = Hub::new();
        let refused: Maker =
            Arc::new(|| Box::pin(async { Err(ResourceFailure::Permanent("refused")) }));
        hub.get_or_request(&wolf(), refused);
        run_all(&hub).await;
        assert_eq!(
            hub.status(&ResourceKey::of(&wolf())),
            ResourceStatus::Failed(ResourceFailure::Permanent("refused"), Neutral::Silhouette)
        );
        assert!(
            hub.take_jobs(8).is_empty(),
            "no retry of a permanent failure"
        );
    }

    #[tokio::test]
    async fn render_spends_nothing_again_on_a_failed_key_until_it_is_invalidated() {
        let hub = Hub::new();
        let calls = Arc::new(AtomicU32::new(0));
        let attempt = |calls: Arc<AtomicU32>, failure: ResourceFailure| -> Generation {
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(failure)
            })
        };
        let spec = wolf();
        // Permanent: one call, then served from the failure memory.
        for _ in 0..3 {
            let result = hub
                .render(
                    &spec,
                    attempt(calls.clone(), ResourceFailure::Permanent("refused")),
                )
                .await;
            assert_eq!(result, Err(ResourceFailure::Permanent("refused")));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // Invalidation clears the memory: the next ask runs.
        assert_eq!(hub.invalidate(Invalidate::Subject("character:ant-wolf")), 1);
        let transient = ResourceFailure::Transient("provider");
        for _ in 0..4 {
            let _ = hub
                .render(&spec, attempt(calls.clone(), transient.clone()))
                .await;
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1 + 2,
            "a transient failure is tried at most the attempt limit (two)"
        );
    }

    #[tokio::test]
    async fn a_ready_entry_whose_output_has_gone_is_generated_again() {
        let hub = Hub::new();
        let present = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&present);
        hub.set_validator(
            ResourceKind::Depiction,
            Arc::new(move |_| flag.load(Ordering::SeqCst)),
        );
        let first = hub
            .render(&wolf(), Box::pin(async { Ok(Generated::new("one")) }))
            .await;
        assert_eq!(first, Ok(Generated::new("one")));
        let served = hub
            .render(&wolf(), Box::pin(async { Ok(Generated::new("two")) }))
            .await;
        assert_eq!(served, Ok(Generated::new("one")), "still there: served");
        present.store(false, Ordering::SeqCst);
        assert!(matches!(
            hub.status(&ResourceKey::of(&wolf())),
            ResourceStatus::Pending(_)
        ));
        let again = hub
            .render(&wolf(), Box::pin(async { Ok(Generated::new("two")) }))
            .await;
        assert_eq!(again, Ok(Generated::new("two")), "gone: made again");
    }

    #[tokio::test]
    async fn a_generation_past_its_timeout_fails_as_timed_out() {
        let hub = Hub::new();
        hub.set_limits(
            ResourceKind::Depiction,
            Limits {
                timeout: Duration::from_millis(10),
                attempts: 1,
            },
        );
        let slow: Maker = Arc::new(|| {
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok(Generated::new("late"))
            })
        });
        hub.get_or_request(&wolf(), slow);
        run_all(&hub).await;
        assert_eq!(
            hub.status(&ResourceKey::of(&wolf())),
            ResourceStatus::Failed(ResourceFailure::TimedOut, Neutral::Silhouette)
        );
    }

    #[test]
    fn the_default_timeout_of_every_kind_is_at_least_the_provider_deadline_floor() {
        for kind in [
            ResourceKind::Depiction,
            ResourceKind::ScenePlate,
            ResourceKind::PlateEdit,
            ResourceKind::ItemCloseUp,
        ] {
            assert!(Limits::for_kind(kind).timeout >= Duration::from_secs(120));
        }
        assert!(
            Limits::for_kind(ResourceKind::VideoClip).timeout
                > Limits::for_kind(ResourceKind::VoiceLine).timeout
        );
    }

    #[test]
    fn the_ledger_records_reserve_commit_refund_and_reconcile_and_never_refuses() {
        let ledger = Arc::new(SpendLedger::default());
        // No budget exists to run out of: any amount of any unit is reserved.
        let first = ledger.reserve(Unit::ImageCall, 2);
        let huge = ledger.reserve(Unit::ImageCall, u64::MAX / 2);
        assert_eq!(ledger.snapshot(Unit::ImageCall).reserved, 2 + u64::MAX / 2);
        first.commit(2);
        let spend = ledger.snapshot(Unit::ImageCall);
        assert_eq!((spend.reserved, spend.committed), (u64::MAX / 2, 2));
        // Refund when not sent, and an unsettled reservation is refunded on drop.
        huge.refund();
        ledger.reserve(Unit::ImageCall, 1).refund();
        drop(ledger.reserve(Unit::ImageCall, 1));
        assert_eq!(
            ledger.snapshot(Unit::ImageCall),
            Spend {
                reserved: 0,
                committed: 2
            }
        );
        // Later evidence corrects the committed total, both ways.
        ledger.reconcile(Unit::ImageCall, 1);
        assert_eq!(ledger.snapshot(Unit::ImageCall).committed, 3);
        ledger.reconcile(Unit::ImageCall, -2);
        assert_eq!(ledger.snapshot(Unit::ImageCall).committed, 1);
        // Units keep separate accounts and the dev tools can read them all.
        ledger.reserve(Unit::MusicCall, 1_000_000).commit(1_000_000);
        assert_eq!(ledger.snapshot(Unit::VideoClip), Spend::default());
        let report = ledger.report();
        assert_eq!(report.len(), 2);
        assert!(report.iter().any(|(unit, _)| *unit == Unit::MusicCall));
    }

    #[tokio::test]
    async fn invalidating_a_subject_makes_the_next_need_ask_again() {
        let hub = Hub::new();
        hub.get_or_request(&wolf(), ok("first"));
        run_all(&hub).await;
        let key = ResourceKey::of(&wolf());
        assert_eq!(hub.invalidate(Invalidate::Key(&key)), 1);
        assert_eq!(hub.status(&key), ResourceStatus::Fallback(Neutral::Nothing));
        hub.get_or_request(&wolf(), ok("second"));
        run_all(&hub).await;
        assert_eq!(hub.ready(&key), Some(Generated::new("second")));
        assert_eq!(hub.for_subject("character:ant-wolf").len(), 1);
    }
}

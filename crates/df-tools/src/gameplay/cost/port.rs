#![expect(
    dead_code,
    reason = "planned caller: durable commerce admission and read-only spend observers; see planning/commerce-service.md § Atomic hierarchical spend and planning/observability.md"
)]

//! The metering port: the narrow public surface of the cost engine, for the owners that must see a
//! billed call without reaching into the game (the acceptance tests, the money core's admission).
//!
//! It is read-only except for [`MeteringPort::start`], which opens a call on a route and hands back
//! a [`PortCall`] that can only be settled with units or dropped. [`BilledCall`] itself stays
//! unconstructible from outside: the port wraps it and adds nothing it cannot already do.
//!
//! Everything here is units: a category name and an integer quantity. The book also prices each
//! call (`micro_usd`), for reports; the money core prices units itself from its own price
//! version and must not rely on that figure.
//!
//! # Observers
//!
//! A [`MeterObserver`] sees every call twice: [`MeterStart`] when it is about to be sent, with
//! the maxima of every category its route declares (the figures a prepaid admission holds), and
//! [`MeteredCall`] once it is recorded, settled or dropped. Observers run on the calling thread
//! after the book's lock is released, and must not block.

use std::sync::Arc;

use super::{
    BilledCall, CallSpec, Category, Cause, CostEvent, PinnedCorrection, PinnedError, PinnedStart,
    PinnedUnsent, Provider, Usage, UsageSource,
};
use crate::gameplay::resources::SpendLedger;

/// The most recent calls the port keeps for [`MeteringPort::recent`].
pub(super) const RECENT_LIMIT: usize = 1024;

/// Where a call's units came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeteredSource {
    /// The supplier's own usage report.
    Reported,
    /// What the server counted or estimated.
    Estimated,
    /// The call was sent and its outcome is unknown; the units are the call's maxima.
    Unknown,
}

/// A quantity of one category (`text_input`, `audio_seconds`, `credits`, ...).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeteredUnits {
    pub category: String,
    pub quantity: u64,
}

/// A call about to be sent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeterStart {
    pub provider: String,
    pub model: String,
    pub cause: String,
    pub detail: String,
    /// The most the call can use, one entry per category its route declares.
    pub maxima: Vec<MeteredUnits>,
}

/// A recorded call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeteredCall {
    /// The idempotency key, counted once per run: `<session>-<n>` for an ordinary call, the
    /// stable dispatch-derived id for a pinned imagery call.
    pub call_id: String,
    /// True when this call replaced an earlier revision under the same id: an observer keyed
    /// by call id must replace its stored view, never count a second call.
    pub correction: bool,
    pub at_ms: u64,
    pub provider: String,
    pub model: String,
    pub cause: String,
    pub detail: String,
    pub source: MeteredSource,
    pub units: Vec<MeteredUnits>,
    /// The book's own price of the call in micro-dollars (lines with no rate count zero).
    pub micro_usd: u64,
    /// Whether any line has no rate in the price file.
    pub unpriced: bool,
    /// False for a call that was billed and produced nothing the game could use: its cost is the
    /// company's, not the customer's.
    pub usable: bool,
}

/// Sees every metered call. See the module note.
pub trait MeterObserver: Send + Sync {
    fn started(&self, start: &MeterStart);
    fn settled(&self, call: &MeteredCall);
}

fn units_of(quantities: &[(Category, u64)]) -> Vec<MeteredUnits> {
    quantities
        .iter()
        .map(|(category, quantity)| MeteredUnits {
            category: category.label().to_owned(),
            quantity: *quantity,
        })
        .collect()
}

impl MeterStart {
    /// The start of a call estimated at `estimate`. Every category its route declares has an
    /// entry: one the estimate leaves out (cached input, a cache write) is zero.
    pub(super) fn of(spec: &CallSpec, estimate: &[(Category, u64)]) -> Self {
        let mut maxima = units_of(estimate);
        for category in super::declared_categories(spec.provider, &spec.model) {
            if !estimate.iter().any(|(held, _)| held == category) {
                maxima.push(MeteredUnits {
                    category: category.label().to_owned(),
                    quantity: 0,
                });
            }
        }
        Self {
            provider: spec.provider.label().to_owned(),
            model: spec.model.clone(),
            cause: spec.cause.label().to_owned(),
            detail: spec.detail.clone(),
            maxima,
        }
    }
}

impl MeteredCall {
    pub(super) fn of(event: &CostEvent, correction: bool) -> Self {
        Self {
            call_id: event.call_id.clone(),
            correction,
            at_ms: event.at_ms,
            provider: event.provider.label().to_owned(),
            model: event.model.clone(),
            cause: event.cause.label().to_owned(),
            detail: event.detail.clone(),
            source: match event.source {
                UsageSource::Reported => MeteredSource::Reported,
                UsageSource::Estimated => MeteredSource::Estimated,
                UsageSource::Unknown => MeteredSource::Unknown,
            },
            units: event
                .lines
                .iter()
                .map(|line| MeteredUnits {
                    category: line.category.label().to_owned(),
                    quantity: line.quantity,
                })
                .collect(),
            micro_usd: event.micro_usd,
            unpriced: event.lines.iter().any(|line| line.micro_usd.is_none()),
            usable: event.usable,
        }
    }
}

/// Why the port refused a route or a price file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortError {
    UnknownProvider(String),
    UnknownCause(String),
    UnknownCategory(String),
    /// The price file was refused; the reason names the entry, never the file.
    Prices(String),
    /// The book is already open.
    AlreadyOpen,
}

/// The route of a call to start: names only, checked against the engine's closed vocabularies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortRoute {
    pub provider: String,
    pub model: String,
    pub cause: String,
    pub detail: String,
    /// The maxima of the call, one entry per declared category.
    pub maxima: Vec<MeteredUnits>,
}

/// Totals of the book, for the status a launcher or a test reads.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MeterTotals {
    pub calls: u64,
    pub micro_usd: u64,
    pub unpriced_calls: u64,
}

/// One call in flight through the port. Dropped unsettled it is recorded as of unknown outcome,
/// at its maxima.
#[must_use = "a metered call must be settled, or marked not billed"]
pub struct PortCall {
    billed: BilledCall,
}

impl PortCall {
    /// The call used `units`, from `source` (`Unknown` is not a source to settle with: drop the
    /// call instead).
    pub fn settle(self, source: MeteredSource, units: &[MeteredUnits]) -> Result<(), PortError> {
        self.settle_at(source, units, super::now_ms())
    }

    /// As [`Self::settle`], recorded at `at_ms`: the caller's clock.
    pub fn settle_at(
        self,
        source: MeteredSource,
        units: &[MeteredUnits],
        at_ms: u64,
    ) -> Result<(), PortError> {
        let quantities = quantities_of(units)?;
        let usage = match source {
            MeteredSource::Reported => Usage::reported(quantities),
            MeteredSource::Estimated | MeteredSource::Unknown => Usage::estimated(quantities),
        };
        self.billed.settle_at(usage, at_ms);
        Ok(())
    }

    /// As [`Self::settle`], for a call the supplier billed whose output was no use.
    pub fn settle_unusable(
        self,
        source: MeteredSource,
        units: &[MeteredUnits],
    ) -> Result<(), PortError> {
        let quantities = quantities_of(units)?;
        let usage = match source {
            MeteredSource::Reported => Usage::reported(quantities),
            MeteredSource::Estimated | MeteredSource::Unknown => Usage::estimated(quantities),
        };
        self.billed.settle_at(usage.unusable(), super::now_ms());
        Ok(())
    }

    /// The supplier refused before doing billable work: nothing is recorded.
    pub fn not_billed(self) {
        self.billed.not_billed();
    }
}

fn quantities_of(units: &[MeteredUnits]) -> Result<Vec<(Category, u64)>, PortError> {
    units
        .iter()
        .map(|unit| {
            Category::parse(&unit.category)
                .map(|category| (category, unit.quantity))
                .ok_or_else(|| PortError::UnknownCategory(unit.category.clone()))
        })
        .collect()
}

/// The port over one spend ledger and its cost book.
#[derive(Clone)]
pub struct MeteringPort {
    ledger: Arc<SpendLedger>,
}

impl Default for MeteringPort {
    fn default() -> Self {
        Self::new()
    }
}

impl MeteringPort {
    /// Reuses the composition root's actual run ledger and already opened Book.
    pub(in crate::gameplay) fn from_ledger(ledger: Arc<SpendLedger>) -> Self {
        Self { ledger }
    }

    /// A port over a ledger of its own and an unopened book: the composition root of a test, or
    /// a service that meters calls of its own.
    pub fn new() -> Self {
        Self {
            ledger: Arc::new(SpendLedger::default()),
        }
    }

    /// The port over the running game's ledger, once the server has opened its run.
    #[allow(
        dead_code,
        reason = "published by the gameplay re-export in artifacts/tmp/agents/C/cost-patch-w4.md"
    )]
    pub fn of_running_game() -> Option<Self> {
        super::current().map(|ledger| Self { ledger })
    }

    /// Opens the book for `run` with the price file `prices` (the text of `content/pricing.json`)
    /// and no log file.
    pub fn open(&self, run: &str, prices: &str) -> Result<(), PortError> {
        let table =
            super::PriceTable::parse(prices).map_err(|error| PortError::Prices(error.reason()))?;
        self.ledger
            .costs()
            .open(
                run,
                None,
                table,
                super::PricingStatus::NotLoaded,
                super::now_ms(),
            )
            .map(|_| ())
            .map_err(|_| PortError::AlreadyOpen)
    }

    /// Registers an observer of every call the book meters from now on.
    pub fn observe(&self, observer: Arc<dyn MeterObserver>) {
        self.ledger.costs().observe(observer);
    }

    /// The most recent calls, oldest first (at most [`RECENT_LIMIT`]).
    pub fn recent(&self) -> Vec<MeteredCall> {
        self.ledger.costs().recent()
    }

    /// The totals of the run so far.
    pub fn totals(&self) -> MeterTotals {
        self.ledger.costs().totals()
    }

    /// Starts a call on `route`. The route's names are checked against the engine's vocabularies.
    pub fn start(&self, route: &PortRoute) -> Result<PortCall, PortError> {
        let provider = Provider::parse(&route.provider)
            .ok_or_else(|| PortError::UnknownProvider(route.provider.clone()))?;
        let cause = Cause::parse(&route.cause)
            .ok_or_else(|| PortError::UnknownCause(route.cause.clone()))?;
        let spec = CallSpec::new(provider, route.model.clone(), cause, route.detail.clone())
            .with_maxima(quantities_of(&route.maxima)?);
        Ok(PortCall {
            billed: self.ledger.begin_call(spec, &[]),
        })
    }

    /// Opens the pinned imagery call for `call_id` (the stable dispatch-derived id) on
    /// `route`. Idempotent per id: an exact replay answers [`PinnedStart::ReplayedOpen`], and
    /// a replay after settle answers [`PinnedStart::AlreadySettled`] instead of opening a
    /// second charge identity.
    pub fn start_pinned(
        &self,
        call_id: &str,
        route: &PortRoute,
    ) -> Result<PinnedStart, PinnedError> {
        let provider = Provider::parse(&route.provider).ok_or(PinnedError::UnknownProvider)?;
        let cause = Cause::parse(&route.cause).ok_or(PinnedError::UnknownCause)?;
        let spec = CallSpec::new(provider, route.model.clone(), cause, route.detail.clone())
            .with_maxima(quantities_of(&route.maxima).map_err(|_| PinnedError::UnknownCategory)?);
        self.ledger.costs().start_pinned(call_id, spec)
    }

    /// Applies a usage report to the pinned call `call_id`: the first report settles it, a
    /// newer `revision` replaces the stored revision as a correction under the same id, an
    /// exact replay is a no-op, and a stale or conflicting revision is refused. An empty
    /// `units` stands for the call's maxima, flagged as `source` says. Unlike an ordinary
    /// port call, [`MeteredSource::Unknown`] stays unknown here: the call was sent and its
    /// outcome is unknown, booked at its maxima, never zeroed.
    pub fn correct_pinned(
        &self,
        call_id: &str,
        revision: u64,
        source: MeteredSource,
        units: &[MeteredUnits],
        usable: bool,
    ) -> Result<PinnedCorrection, PinnedError> {
        self.correct_pinned_at(call_id, revision, source, units, usable, super::now_ms())
    }

    /// As [`Self::correct_pinned`], recorded at `at_ms`: the caller's clock.
    pub fn correct_pinned_at(
        &self,
        call_id: &str,
        revision: u64,
        source: MeteredSource,
        units: &[MeteredUnits],
        usable: bool,
        at_ms: u64,
    ) -> Result<PinnedCorrection, PinnedError> {
        let quantities = quantities_of(units).map_err(|_| PinnedError::UnknownCategory)?;
        let mut usage = match source {
            MeteredSource::Reported => Usage::reported(quantities),
            MeteredSource::Estimated => Usage::estimated(quantities),
            MeteredSource::Unknown => Usage {
                source: UsageSource::Unknown,
                quantities,
                usable: true,
            },
        };
        if !usable {
            usage = usage.unusable();
        }
        self.ledger
            .costs()
            .correct_pinned(call_id, revision, usage, at_ms)
    }

    /// Coherent same-call Book snapshot; no history-window or separate-durability join.
    pub fn read_pinned_receipt(
        &self,
        call_id: &str,
    ) -> Result<Option<df_imagery::cost::PinnedCostReceipt>, PinnedError> {
        self.ledger.costs().read_pinned_receipt(call_id)
    }

    /// Atomic actual Book seal for a canonical call with protected no-delivery authority.
    pub fn seal_pinned_unsent(
        &self,
        call_id: &str,
    ) -> Result<df_imagery::cost::PinnedUnsentSeal, PinnedError> {
        self.ledger.costs().seal_pinned_unsent(call_id)
    }

    /// The existing cost log has synced this exact revision, or terminal unsent marker.
    pub fn pinned_durable(
        &self,
        call_id: &str,
        revision: Option<u64>,
    ) -> Result<bool, PinnedError> {
        self.ledger.costs().pinned_durable(call_id, revision)
    }

    /// Records the pinned call `call_id` as proven unsent (refused submit, cancelled before
    /// dispatch): nothing is recorded, and later usage for the id is refused.
    pub fn record_pinned_unsent(&self, call_id: &str) -> Result<PinnedUnsent, PinnedError> {
        self.ledger.costs().record_pinned_unsent(call_id)
    }
}

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

use super::{BilledCall, CallSpec, Category, Cause, CostEvent, Provider, Usage, UsageSource};
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
    /// The idempotency key: `<session>-<n>`, counted once per run.
    pub call_id: String,
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

/// A route the game calls but does not meter yet: hosted paid play refuses it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingRoute {
    pub client: String,
    pub provider: String,
    pub model: String,
    /// The backlog item and what is missing.
    pub reason: String,
}

/// The routes not metered yet, read from the engine's route table. An admission that refuses
/// unmetered routes (hosted paid play) reads this list instead of keeping one of its own.
pub fn pending_routes() -> Vec<PendingRoute> {
    super::ROUTES
        .iter()
        .filter_map(|route| match route.metering {
            super::Metering::Pending(reason) => Some(PendingRoute {
                client: route.client.to_owned(),
                provider: route.provider.label().to_owned(),
                model: route.model.to_owned(),
                reason: reason.to_owned(),
            }),
            super::Metering::Priced => None,
        })
        .collect()
}

impl MeteredCall {
    pub(super) fn of(event: &CostEvent) -> Self {
        Self {
            call_id: event.call_id.clone(),
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
}

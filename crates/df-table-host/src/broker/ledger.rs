//! The cost ledger: every request with its purpose and price, recorded and never blocking.
//!
//! It owns the entry of each request (what it was for, what class it waited in, how many provider
//! calls were sent and how it ended) and the running totals per purpose. A call that was sent is
//! spent and never refunded; a request that never reached a provider costs nothing. The ledger
//! decides nothing: there is no cap, no budget and no refusal anywhere in it (pattern 4), so a
//! hosted entitlement check, when it exists, reads the ledger and lives elsewhere. Entries are
//! kept up to `MAXIMUM_LEDGER_ENTRIES` and the oldest are forgotten first; the totals are never
//! forgotten, and their number is bounded by the number of purposes.

use std::collections::{BTreeMap, VecDeque};

use df_table::event::FailureCause;
use df_table::ports::{PurposeName, ResourceHandle};
use serde::{Deserialize, Serialize};

use super::limits::MAXIMUM_LEDGER_ENTRIES;
use super::purpose::{Price, QueueClass};

/// How a request ended, for the ledger's reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "end", rename_all = "snake_case")]
pub enum LedgerEnd {
    /// Not finished yet.
    Open,
    Ready,
    Failed {
        cause: FailureCause,
    },
    Cancelled,
    /// Its scene epoch passed, so a result, if one came, was rejected.
    Superseded,
    Dropped,
}

/// One request's line in the ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub handle: u64,
    pub purpose: PurposeName,
    pub class: QueueClass,
    pub price: Price,
    pub requested_ms: u64,
    /// Provider calls sent for this request: each is spent.
    pub calls_sent: u32,
    pub end: LedgerEnd,
}

/// What one purpose has cost so far.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurposeTotals {
    pub requests: u64,
    pub calls_sent: u64,
    /// Sum of the prices of the calls sent, in micro-dollars; unpriced calls add nothing.
    pub spent_micro_dollars: u64,
    pub unpriced_calls: u64,
}

/// The record of every request.
#[derive(Debug, Default)]
pub struct CostLedger {
    entries: VecDeque<LedgerEntry>,
    totals: BTreeMap<PurposeName, PurposeTotals>,
    forgotten_entries: u64,
}

impl CostLedger {
    /// Records a request when it is accepted. Handles only grow, so entries stay sorted by handle.
    pub fn record_request(
        &mut self,
        handle: ResourceHandle,
        purpose: &PurposeName,
        class: QueueClass,
        price: Price,
        requested_ms: u64,
    ) {
        self.totals.entry(purpose.clone()).or_default().requests += 1;
        self.entries.push_back(LedgerEntry {
            handle: handle.get(),
            purpose: purpose.clone(),
            class,
            price,
            requested_ms,
            calls_sent: 0,
            end: LedgerEnd::Open,
        });
        while self.entries.len() > MAXIMUM_LEDGER_ENTRIES {
            self.entries.pop_front();
            self.forgotten_entries += 1;
        }
    }

    /// Records that a provider call for this request was sent: its price is spent.
    pub fn record_call_sent(
        &mut self,
        handle: ResourceHandle,
        purpose: &PurposeName,
        price: Price,
    ) {
        let totals = self.totals.entry(purpose.clone()).or_default();
        totals.calls_sent += 1;
        match price {
            Price::Priced { micro_dollars } => {
                totals.spent_micro_dollars =
                    totals.spent_micro_dollars.saturating_add(micro_dollars);
            }
            Price::Unpriced => totals.unpriced_calls += 1,
        }
        if let Some(entry) = self.entry_mut(handle) {
            entry.calls_sent += 1;
        }
    }

    pub fn record_end(&mut self, handle: ResourceHandle, end: LedgerEnd) {
        if let Some(entry) = self.entry_mut(handle) {
            entry.end = end;
        }
    }

    fn entry_mut(&mut self, handle: ResourceHandle) -> Option<&mut LedgerEntry> {
        let index = self
            .entries
            .partition_point(|entry| entry.handle < handle.get());
        self.entries
            .get_mut(index)
            .filter(|entry| entry.handle == handle.get())
    }

    pub fn entry(&self, handle: ResourceHandle) -> Option<&LedgerEntry> {
        let index = self
            .entries
            .partition_point(|entry| entry.handle < handle.get());
        self.entries
            .get(index)
            .filter(|entry| entry.handle == handle.get())
    }

    /// The kept entries, oldest first.
    pub fn entries(&self) -> impl Iterator<Item = &LedgerEntry> {
        self.entries.iter()
    }

    /// Totals per purpose, in name order.
    pub fn totals(&self) -> impl Iterator<Item = (&PurposeName, &PurposeTotals)> {
        self.totals.iter()
    }

    pub fn totals_for(&self, purpose: &PurposeName) -> PurposeTotals {
        self.totals.get(purpose).copied().unwrap_or_default()
    }

    /// Entries forgotten to stay within `MAXIMUM_LEDGER_ENTRIES`.
    pub fn forgotten_entries(&self) -> u64 {
        self.forgotten_entries
    }
}

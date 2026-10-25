//! In-memory fakes of the five ports, faithful enough to drive the whole account app: operation
//! ids make a repeated request return the first result, balances never go below zero, a foreign
//! id is `not_found`, and credit appears only when the fake processor's webhook marks a checkout
//! paid. Compiled for tests and the `test-fakes` feature only; each fake carries the owner whose
//! real implementation replaces it.

mod assets;
mod campaigns;
mod campaigns_display;
mod campaigns_join;
mod campaigns_rules;
mod checkout;
mod extras;
mod finance;
mod finance_operator;
mod operators;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use df_customer::{Clock, SecretToken};
use df_types::{AccountId, OperationId};

pub use assets::FakeAssets;
pub use campaigns::FakeCampaigns;
pub use checkout::FakeCheckout;
pub use extras::FakeExtras;
pub use finance::FakeFinance;
pub use operators::{FakeOperators, FakeReview};

use super::ports::Ports;

/// Locks a fake's state; a panic in another test thread never hides this one's failure.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 32 random lowercase hexadecimal digits: an id of the shape every opaque id has.
fn fresh_hex() -> String {
    // The random source cannot fail in tests; an all-zero id would be refused on parse anyway.
    SecretToken::generate().map_or_else(|_| "1".repeat(32), |token| token.text()[..32].to_owned())
}

/// A short unique name for scratch directories in tests.
pub fn unique_name() -> String {
    fresh_hex()[..16].to_owned()
}

/// The first result of each (account, operation) pair, replayed on a repeat.
struct Receipts<T>(HashMap<(AccountId, OperationId), T>);

impl<T: Clone> Receipts<T> {
    fn replay(&self, account: AccountId, operation: OperationId) -> Option<T> {
        self.0.get(&(account, operation)).cloned()
    }

    fn record(&mut self, account: AccountId, operation: OperationId, result: T) {
        self.0.insert((account, operation), result);
    }
}

impl<T> Default for Receipts<T> {
    fn default() -> Self {
        Self(HashMap::new())
    }
}

/// All five fakes, wired together: the checkout fake's webhook credits the finance fake.
pub struct FakePorts {
    pub finance: FakeFinance,
    pub checkout: FakeCheckout,
    pub campaigns: FakeCampaigns,
    pub operators: FakeOperators,
    pub review: FakeReview,
    pub assets: FakeAssets,
    pub extras: FakeExtras,
}

impl FakePorts {
    /// `origin` is the configured public origin: checkout and invitation links are built on it.
    pub fn new(origin: &str, clock: Arc<dyn Clock>) -> Self {
        let finance = FakeFinance::new(Arc::clone(&clock));
        Self {
            checkout: FakeCheckout::new(finance.clone()),
            campaigns: FakeCampaigns::new(origin, Arc::clone(&clock)),
            operators: FakeOperators::new(Arc::clone(&clock)),
            review: FakeReview::new(Arc::clone(&clock)),
            assets: FakeAssets::new(Arc::clone(&clock)),
            extras: FakeExtras::new(clock),
            finance,
        }
    }
}

impl Ports for FakePorts {
    type Finance = FakeFinance;
    type Checkout = FakeCheckout;
    type Campaigns = FakeCampaigns;
    type Operators = FakeOperators;
    type Review = FakeReview;
    type Assets = FakeAssets;
    type Extras = FakeExtras;

    fn finance(&self) -> &FakeFinance {
        &self.finance
    }

    fn checkout(&self) -> &FakeCheckout {
        &self.checkout
    }

    fn campaigns(&self) -> &FakeCampaigns {
        &self.campaigns
    }

    fn operators(&self) -> &FakeOperators {
        &self.operators
    }

    fn review(&self) -> &FakeReview {
        &self.review
    }

    fn assets(&self) -> &FakeAssets {
        &self.assets
    }

    fn extras(&self) -> &FakeExtras {
        &self.extras
    }
}

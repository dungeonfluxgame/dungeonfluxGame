//! Fake processor checkout (TODO(B): replaced by B's Stripe test-mode port). Creating a checkout
//! credits nothing and neither does reading its status; only `deliver_webhook` does, as the
//! processor's signed webhook would (plan 17, PAY-01).

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use df_account_app::wire::{CheckoutCreated, CheckoutStatus, CheckoutView, CreditOffers};
use df_types::{AccountId, CheckoutId, OperationId};

use super::{Receipts, finance::FakeFinance, fresh_hex, lock};
use crate::app_api::{
    ids::hex,
    ports::{CheckoutPort, PortError, PortResult},
};

struct Record {
    account: AccountId,
    amount_cents: u32,
    status: CheckoutStatus,
}

#[derive(Default)]
struct State {
    checkouts: HashMap<CheckoutId, Record>,
    receipts: Receipts<CheckoutCreated>,
}

#[derive(Clone)]
pub struct FakeCheckout {
    finance: FakeFinance,
    state: Arc<Mutex<State>>,
}

impl FakeCheckout {
    pub fn new(finance: FakeFinance) -> Self {
        Self {
            finance,
            state: Arc::new(Mutex::new(State::default())),
        }
    }

    /// The processor's webhook for `checkout`: a paid one credits the account exactly once; a
    /// repeat, or a webhook for a checkout that is no longer pending, changes nothing.
    pub fn deliver_webhook(&self, checkout: CheckoutId, outcome: CheckoutStatus) -> bool {
        let mut state = lock(&self.state);
        let Some(record) = state.checkouts.get_mut(&checkout) else {
            return false;
        };
        if record.status != CheckoutStatus::Pending {
            return true;
        }
        record.status = outcome;
        if outcome == CheckoutStatus::Paid {
            let (account, cents) = (record.account, record.amount_cents);
            drop(state);
            self.finance
                .credit_purchase(account, cents, &hex(checkout.as_bytes()));
        }
        true
    }
}

impl CheckoutPort for FakeCheckout {
    fn offers(&self) -> CreditOffers {
        CreditOffers {
            is_test_mode: true,
            preset_amounts_cents: vec![1_000, 2_000, 5_000],
            custom_minimum_cents: 500,
            custom_maximum_cents: 50_000,
        }
    }

    async fn create(
        &self,
        account: AccountId,
        amount_cents: u32,
        operation: OperationId,
    ) -> PortResult<CheckoutCreated> {
        let mut state = lock(&self.state);
        if let Some(first) = state.receipts.replay(account, operation) {
            return Ok(first);
        }
        let id = CheckoutId::from_hex(&fresh_hex()).map_err(|_| PortError::Unavailable)?;
        let created = CheckoutCreated {
            checkout_id: hex(id.as_bytes()),
            // A path on this origin, like the processor's hosted page the real port returns.
            redirect_url: format!("/dev/checkout/{}", hex(id.as_bytes())),
        };
        state.checkouts.insert(
            id,
            Record {
                account,
                amount_cents,
                status: CheckoutStatus::Pending,
            },
        );
        state.receipts.record(account, operation, created.clone());
        Ok(created)
    }

    async fn status(&self, account: AccountId, checkout: CheckoutId) -> PortResult<CheckoutView> {
        let state = lock(&self.state);
        state
            .checkouts
            .get(&checkout)
            .filter(|record| record.account == account)
            .map(|record| CheckoutView {
                checkout_id: hex(checkout.as_bytes()),
                amount_cents: record.amount_cents,
                status: record.status,
            })
            .ok_or(PortError::NotFound)
    }
}
